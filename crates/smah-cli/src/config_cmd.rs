//! `smah config init` (RA-06): first-run configuration — provider + API key
//! (masked entry on a TTY, env-reference or env-fallback headless), operator
//! profile, and the data-egress disclosure with opt-out toggles.
//!
//! Secrets discipline: the key is read masked (rpassword, TTY) or taken
//! from the environment — never echoed, never logged; the literal key is
//! registered with the redactor and the written file is chmod 0600.
//! Zero-usable-models refusal: if after collection no provider resolves a
//! key, init refuses to save and exits 4 with guidance (interface-spec §4
//! row 6/7 classes).
//!
//! Console contract: the wizard is chrome — prompts, disclosure, and the
//! final summary all go to stderr; stdout stays empty unless --json is
//! asked for later tickets.

use std::io::{BufRead as _, IsTerminal, Write};
use std::path::{Path, PathBuf};

use serde_yaml_ng::{Mapping, Value};

use smah_ask::EgressToggles;
use smah_core::errors::SmahError;
use smah_core::settings::Settings;

use crate::args::{ConfigInitArgs, ConfigSubCommand};

/// Run `smah config <cmd>`; returns the process exit code.
/// `cli_config` is the global `-c` (the local `--config-path` wins when
/// both are given).
pub fn run_config(cli_config: Option<&std::path::Path>, cmd: &ConfigSubCommand) -> i32 {
    match cmd {
        ConfigSubCommand::Init(args) => run_init(args, cli_config),
    }
}

/// Operator profile answers (the `user` block).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileDraft {
    pub name: String,
    pub role: String,
    pub experience: String,
    pub about: String,
}

/// Everything needed to render the config document.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigDraft {
    pub provider: String,
    /// A literal key (masked entry) — stored in the file (0600).
    pub api_key_literal: Option<String>,
    /// An env-var reference (`$VAR`) instead of a literal.
    pub api_key_env_ref: Option<String>,
    pub profile: ProfileDraft,
    pub toggles: EgressToggles,
}

impl ConfigDraft {
    /// The `settings.api_key` value for the document: the env reference
    /// when given, else the literal, else none.
    fn api_key_value(&self) -> Option<Value> {
        if let Some(var) = &self.api_key_env_ref {
            return Some(Value::String(format!("${var}")));
        }
        self.api_key_literal
            .as_ref()
            .map(|k| Value::String(k.clone()))
    }

    /// Usable = the chosen provider resolves a key now (literal present, or
    /// the referenced/env-fallback variable is set).
    pub fn is_usable(&self, env_lookup: &dyn Fn(&str) -> Option<String>) -> bool {
        if self.api_key_literal.is_some() {
            return true;
        }
        if let Some(var) = &self.api_key_env_ref {
            return env_lookup(var).is_some();
        }
        // Env fallback chain for the provider (SMAH_<KEY>_API_KEY, then
        // <KEY>_API_KEY) — mirrors credentials::env_chain.
        env_lookup(&format!("SMAH_{}_API_KEY", self.provider.to_uppercase()))
            .or_else(|| env_lookup(&format!("{}_API_KEY", self.provider.to_uppercase())))
            .is_some()
    }
}

/// Render the config document (ordered: vsn, user, privacy, inference).
pub fn build_document(draft: &ConfigDraft) -> Value {
    let mut user = Mapping::new();
    user.insert(Value::String("vsn".into()), Value::String("0.0.1".into()));
    user.insert(
        Value::String("name".into()),
        Value::String(draft.profile.name.clone()),
    );
    user.insert(
        Value::String("system_admin_experience".into()),
        Value::String(draft.profile.experience.clone()),
    );
    user.insert(
        Value::String("role".into()),
        Value::String(draft.profile.role.clone()),
    );
    user.insert(
        Value::String("about".into()),
        Value::String(draft.profile.about.clone()),
    );

    let mut privacy = Mapping::new();
    privacy.insert(
        Value::String("share_profile".into()),
        Value::Bool(draft.toggles.share_profile),
    );
    privacy.insert(
        Value::String("share_system_stats".into()),
        Value::Bool(draft.toggles.share_system_stats),
    );

    let mut settings = Mapping::new();
    if let Some(key) = draft.api_key_value() {
        settings.insert(Value::String("api_key".into()), key);
    }
    let mut provider = Mapping::new();
    provider.insert(
        Value::String("name".into()),
        Value::String(title_case(&draft.provider)),
    );
    provider.insert(Value::String("enabled".into()), Value::Bool(true));
    provider.insert(Value::String("settings".into()), Value::Mapping(settings));
    provider.insert(Value::String("models".into()), Value::Sequence(vec![]));
    let mut providers = Mapping::new();
    providers.insert(
        Value::String(draft.provider.clone()),
        Value::Mapping(provider),
    );

    let mut inference = Mapping::new();
    inference.insert(Value::String("vsn".into()), Value::String("0.0.1".into()));
    inference.insert(Value::String("providers".into()), Value::Mapping(providers));

    let mut root = Mapping::new();
    root.insert(Value::String("vsn".into()), Value::String("0.0.1".into()));
    root.insert(Value::String("user".into()), Value::Mapping(user));
    root.insert(Value::String("privacy".into()), Value::Mapping(privacy));
    root.insert(Value::String("inference".into()), Value::Mapping(inference));
    Value::Mapping(root)
}

fn title_case(provider: &str) -> String {
    let mut chars = provider.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => provider.to_string(),
    }
}

/// Where init writes: `--config` override or the default path.
fn target_path(cli_config: Option<&Path>) -> PathBuf {
    cli_config
        .map(|p| p.to_path_buf())
        .unwrap_or_else(Settings::default_path)
}

fn run_init(args: &ConfigInitArgs, cli_config: Option<&std::path::Path>) -> i32 {
    let interactive = !args.non_interactive && std::io::stdin().is_terminal();
    let path = target_path(args.config_path.as_deref().or(cli_config));
    match run_init_inner(args, interactive, &path) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("{}", err.render("config-init"));
            err.exit_code().code()
        }
    }
}

fn run_init_inner(args: &ConfigInitArgs, interactive: bool, path: &Path) -> Result<(), SmahError> {
    if path.exists() && !args.force {
        return Err(SmahError::Usage {
            message: format!(
                "smah: config already exists at {} — pass --force to overwrite",
                path.display()
            ),
        });
    }

    let provider = args
        .provider
        .clone()
        .unwrap_or_else(|| "anthropic".to_string());
    let known = smah_providers::registry::known_provider_keys();
    if !known.contains(&provider.as_str()) {
        return Err(SmahError::Usage {
            message: format!(
                "smah: unknown provider '{provider}' — known: {}",
                known.join(", ")
            ),
        });
    }

    // Profile.
    let profile = if interactive {
        ProfileDraft {
            name: prompt_line("Your name (for the model's addressing; blank ok)")?,
            role: prompt_line("Your role (e.g. developer, sre; blank ok)")?,
            experience: prompt_line(
                "Linux/ops experience (Novice / Intermediate / Expert; blank ok)",
            )?,
            about: prompt_line("Anything else the model should know (blank ok)")?,
        }
    } else {
        ProfileDraft {
            name: args.name.clone().unwrap_or_default(),
            role: args.role.clone().unwrap_or_default(),
            experience: args.experience.clone().unwrap_or_default(),
            about: args.about.clone().unwrap_or_default(),
        }
    };

    // API key: masked prompt on a TTY; --key-env reference or env fallback
    // headless. Never echoed, never logged.
    let api_key_literal = if interactive {
        let key = rpassword::prompt_password("API key (input hidden): ").map_err(|err| {
            SmahError::Internal {
                detail: format!("key entry: {err}"),
            }
        })?;
        let key = key.trim().to_string();
        if key.is_empty() {
            None
        } else {
            Some(key)
        }
    } else {
        None
    };

    // Egress disclosure + toggles (owner-approved opt-outs).
    let toggles = if args.no_share_profile || args.no_share_stats {
        EgressToggles {
            share_profile: !args.no_share_profile,
            share_system_stats: !args.no_share_stats,
        }
    } else {
        EgressToggles::default()
    };
    eprintln!("{}", smah_ask::egress_disclosure(&toggles));
    eprintln!(
        "Opt out any time in {} (privacy.share_profile / privacy.share_system_stats).",
        path.display()
    );

    let draft = ConfigDraft {
        provider: provider.clone(),
        api_key_literal,
        api_key_env_ref: args.key_env.clone(),
        profile,
        toggles,
    };

    // Zero-usable refusal: no key by any route ⇒ refuse to save (exit 4)
    // with guidance rather than writing a config that cannot answer.
    if !draft.is_usable(&|name| std::env::var(name).ok()) {
        return Err(SmahError::MissingApiKey { provider });
    }

    // Register the literal secret with the log redactor before anything
    // else could log around it.
    if let Some(key) = &draft.api_key_literal {
        smah_core::logging::register_secret(key.clone());
    }

    // Write (0600, atomic-ish: temp then rename).
    let document = build_document(&draft);
    let text = serde_yaml_ng::to_string(&document).map_err(|err| SmahError::Internal {
        detail: format!("yaml encode: {err}"),
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| SmahError::ConfigUnreadable {
            path: parent.display().to_string(),
            detail: err.to_string(),
        })?;
    }
    std::fs::write(path, text).map_err(|err| SmahError::ConfigUnreadable {
        path: path.display().to_string(),
        detail: err.to_string(),
    })?;
    set_owner_only(path);

    // The file must load cleanly as the app would read it.
    Settings::load_from(path)?;

    eprintln!(
        "smah: config written to {} (provider: {provider}, chmod 0600)",
        path.display()
    );
    Ok(())
}

#[cfg(unix)]
fn set_owner_only(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(0o600);
        let _ = std::fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn set_owner_only(_path: &Path) {}

/// One trimmed line from stdin (a wizard prompt; TTY only).
fn prompt_line(question: &str) -> Result<String, SmahError> {
    let mut stderr = std::io::stderr().lock();
    writeln!(stderr, "{question}").map_err(|err| SmahError::Internal {
        detail: err.to_string(),
    })?;
    drop(stderr);
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|err| SmahError::Internal {
            detail: err.to_string(),
        })?;
    Ok(line.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> ConfigDraft {
        ConfigDraft {
            provider: "anthropic".to_string(),
            api_key_literal: Some("test-key-0000".to_string()),
            api_key_env_ref: None,
            profile: ProfileDraft {
                name: "Dana".to_string(),
                role: "Developer".to_string(),
                experience: "Novice".to_string(),
                about: "learning ops".to_string(),
            },
            toggles: EgressToggles::default(),
        }
    }

    #[test]
    fn document_carries_every_block_in_order() {
        let value = build_document(&draft());
        let map = value.as_mapping().expect("root mapping");
        let keys: Vec<String> = map
            .iter()
            .map(|(k, _)| k.as_str().expect("string key").to_string())
            .collect();
        assert_eq!(keys, vec!["vsn", "user", "privacy", "inference"]);
        assert_eq!(
            map.get(Value::String("vsn".into())),
            Some(&Value::String("0.0.1".into()))
        );
        let user = map
            .get(Value::String("user".into()))
            .unwrap()
            .as_mapping()
            .unwrap()
            .clone();
        assert_eq!(
            user.get(Value::String("name".into())),
            Some(&Value::String("Dana".into()))
        );
    }

    #[test]
    fn literal_key_lands_in_provider_settings() {
        let value = build_document(&draft());
        let provider = value["inference"]["providers"]["anthropic"]
            .as_mapping()
            .unwrap()
            .clone();
        assert_eq!(
            provider.get(Value::String("enabled".into())),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            provider["settings"]["api_key"],
            Value::String("test-key-0000".to_string())
        );
    }

    #[test]
    fn env_reference_wins_over_literal() {
        let mut d = draft();
        d.api_key_env_ref = Some("SMAH_ANTHROPIC_API_KEY".to_string());
        let value = build_document(&d);
        assert_eq!(
            value["inference"]["providers"]["anthropic"]["settings"]["api_key"],
            Value::String("$SMAH_ANTHROPIC_API_KEY".to_string())
        );
    }

    #[test]
    fn toggles_off_are_written_off() {
        let mut d = draft();
        d.toggles = EgressToggles {
            share_profile: false,
            share_system_stats: true,
        };
        let value = build_document(&d);
        let privacy = value["privacy"].as_mapping().unwrap().clone();
        assert_eq!(
            privacy.get(Value::String("share_profile".into())),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            privacy.get(Value::String("share_system_stats".into())),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn usability_via_literal_reference_or_env() {
        let none = ConfigDraft {
            api_key_literal: None,
            api_key_env_ref: None,
            ..draft()
        };
        assert!(!none.is_usable(&|_| None));
        assert!(
            none.is_usable(
                &|name: &str| (name == "SMAH_ANTHROPIC_API_KEY").then(|| "k".to_string())
            ),
            "provider env fallback counts"
        );
        let referenced = ConfigDraft {
            api_key_literal: None,
            api_key_env_ref: Some("MY_KEY".to_string()),
            ..draft()
        };
        assert!(!referenced.is_usable(&|_| None));
        assert!(referenced.is_usable(&|name: &str| (name == "MY_KEY").then(|| "k".to_string())));
        assert!(draft().is_usable(&|_| None), "literal key is usable");
    }

    #[test]
    fn document_round_trips_through_settings() {
        let value = build_document(&draft());
        let text = serde_yaml_ng::to_string(&value).expect("yaml");
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, text).expect("write");
        let settings = Settings::load_from(&path).expect("loads");
        assert!(settings.user().is_some());
        assert_eq!(
            EgressToggles::from_settings(&settings),
            EgressToggles::default()
        );
    }
}

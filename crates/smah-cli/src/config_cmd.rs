//! `smah config init` (RA-06, revised RA-10): first-run configuration —
//! provider + API key (masked entry on a TTY, env-reference or env-fallback
//! headless), operator profile, and the data-egress disclosure with opt-out
//! toggles.
//!
//! Secrets discipline (F1/F2, owner decision 2026-11-10): the key is read
//! masked (rpassword, TTY) or taken from the environment — never echoed,
//! never logged; the literal is registered with the redactor and then
//! **stored in a secret store** (macOS Keychain / Windows Credential
//! Manager / Secret Service via `keyring`, the 0600 `secrets.yaml`
//! fallback, or an `env:VAR` reference). `config.yaml` keeps only a
//! **reference** — never a literal key. The config file itself is written
//! atomically with owner-only permissions from the first byte (temp file
//! 0600 in the same directory, fsync, rename); a failed write or failed
//! chmod is an error, never a "0600" success line. On Windows there are no
//! POSIX modes: the file inherits the user-profile ACLs of its directory
//! (documented in docs/THREAT-MODEL.md).
//!
//! Zero-usable-models refusal: if after collection no provider resolves a
//! key, init refuses to save and exits 4 with guidance (interface-spec §4
//! row 6/7 classes) — and never touches the secret store.
//!
//! Console contract: the wizard is chrome — prompts, disclosure, and the
//! final summary all go to stderr; stdout stays empty unless --json is
//! asked for later tickets.

use std::io::{BufRead as _, IsTerminal, Write};
use std::path::{Path, PathBuf};

use serde_yaml_ng::{Mapping, Value};

use smah_ask::EgressToggles;
use smah_core::errors::SmahError;
use smah_core::secrets::{self, SecretRef, SecretStore, StoreKind};
use smah_core::settings::Settings;

use crate::args::{ConfigInitArgs, ConfigMigrateArgs, ConfigSubCommand};

/// Where `config init` should put the key (`--secret-store` override or the
/// best available backend).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreChoice {
    /// The OS keychain / credential manager / Secret Service.
    Keyring,
    /// The 0600 `secrets.yaml` fallback beside the config.
    File,
    /// An `env:VAR` reference (the variable must be set at run time).
    Env,
}

impl StoreChoice {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "keychain" => Some(StoreChoice::Keyring),
            "file" => Some(StoreChoice::File),
            "env" => Some(StoreChoice::Env),
            _ => None,
        }
    }
}

/// Run `smah config <cmd>`; returns the process exit code.
/// `cli_config` is the global `-c` (the local `--config-path` wins when
/// both are given).
pub fn run_config(cli_config: Option<&std::path::Path>, cmd: &ConfigSubCommand) -> i32 {
    match cmd {
        ConfigSubCommand::Init(args) => run_init(args, cli_config),
        ConfigSubCommand::MigrateSecrets(args) => run_migrate_secrets(args, cli_config),
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

/// Everything needed to render the config document. No `Debug` derive: the
/// literal key may be present before it lands in the store (F8 — a stray
/// `{:?}` must not be able to print it).
#[derive(Clone, PartialEq)]
pub struct ConfigDraft {
    pub provider: String,
    /// The literal key (masked entry) — goes to the secret store, never
    /// into the file.
    pub api_key_literal: Option<String>,
    /// An env-var reference target (`env:VAR`) instead of a stored key.
    pub api_key_env_ref: Option<String>,
    /// Backend for the literal path (Keyring or File; Env pairs with
    /// `api_key_env_ref`).
    pub secret_store: StoreChoice,
    pub profile: ProfileDraft,
    pub toggles: EgressToggles,
}

impl ConfigDraft {
    /// The `settings.api_key` value for the document — a **reference**,
    /// never the literal (F2): the env reference when given, else the store
    /// reference for the chosen backend, else none.
    fn api_key_value(&self) -> Option<Value> {
        if let Some(var) = &self.api_key_env_ref {
            return Some(Value::String(format!("env:{var}")));
        }
        if self.api_key_literal.is_some() {
            let reference = match self.secret_store {
                StoreChoice::Keyring => SecretRef::Keyring {
                    service: secrets::SERVICE.to_string(),
                    account: self.provider.clone(),
                },
                StoreChoice::File => SecretRef::File {
                    path: PathBuf::from(secrets::SECRETS_FILE),
                    name: self.provider.clone(),
                },
                // Literal + env store is rejected upstream (Usage error:
                // pair `--secret-store env` with `--key-env`).
                StoreChoice::Env => return None,
            };
            return Some(Value::String(reference.encode()));
        }
        None
    }

    /// Usable = the chosen provider resolves a key now (literal going to a
    /// store that was written, or the referenced/env-fallback variable is
    /// set).
    pub fn is_usable(&self, env_lookup: &dyn Fn(&str) -> Option<String>) -> bool {
        if self.api_key_literal.is_some() && self.api_key_env_ref.is_none() {
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

    let config_dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    // Secret store choice: explicit override or the best available backend,
    // explained to the operator (F2).
    let choice = resolve_store_choice(args, interactive)?;
    let (kind, store_note) = match choice {
        StoreChoice::Keyring => {
            let store = secrets::KeyringStore;
            (StoreKind::Keyring, store.describe())
        }
        StoreChoice::File => {
            let store = secrets::FileStore {
                path: config_dir.join(secrets::SECRETS_FILE),
            };
            (StoreKind::File, store.describe())
        }
        StoreChoice::Env => (
            StoreKind::Runtime,
            "environment reference (the variable must be set at run time)".to_string(),
        ),
    };
    if interactive {
        eprintln!("secret store: {store_note}");
        if choice == StoreChoice::File && secrets::keyring_available() {
            eprintln!("  (explicit --secret-store file — the OS store was available)");
        } else if let Some(reason) = fallback_reason(choice) {
            eprintln!("  ({reason})");
        }
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
    let api_key_literal = if interactive && args.key_env.is_none() {
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
        secret_store: choice,
        profile,
        toggles,
    };
    // An interactive literal with the `env` store forced means the operator
    // wants an env reference — there is no key-env to point at, so require
    // the flag (or the chain) instead of guessing.
    if draft.api_key_literal.is_some() && choice == StoreChoice::Env {
        return Err(SmahError::Usage {
            message: "smah: --secret-store env stores a reference — pass --key-env VAR (or set SMAH_<PROVIDER>_API_KEY and drop the prompt)".to_string(),
        });
    }

    // Zero-usable refusal: no key by any route ⇒ refuse to save (exit 4)
    // with guidance rather than writing a config that cannot answer — and
    // before anything touches a secret store.
    if !draft.is_usable(&|name| std::env::var(name).ok()) {
        return Err(SmahError::MissingApiKey { provider });
    }

    // Register the literal secret with the log redactor before anything
    // else could log around it.
    if let Some(key) = &draft.api_key_literal {
        smah_core::logging::register_secret(key.clone());
    }

    // Put the literal key into the chosen store; config.yaml will carry the
    // reference only (F2). Failures propagate with guidance — no silent
    // fallback, no false success (F1 discipline).
    if let Some(key) = draft.api_key_literal.clone() {
        let reference = secrets::store_reference(kind, &provider, &config_dir);
        let backend: &dyn SecretStore = match choice {
            StoreChoice::Keyring => &secrets::KeyringStore,
            StoreChoice::File => &secrets::FileStore {
                path: config_dir.join(secrets::SECRETS_FILE),
            },
            StoreChoice::Env => unreachable!("env choice never stores a literal"),
        };
        backend.set(&reference, &key).map_err(|detail| {
            SmahError::Internal {
                detail: format!(
                    "cannot store the {provider} key in {}: {detail} — retry with --secret-store file (or --secret-store env with --key-env)",
                    backend.describe()
                ),
            }
        })?;
    }

    // Write (0600 from the first byte, atomic: temp then rename — F1).
    let document = build_document(&draft);
    let text = serde_yaml_ng::to_string(&document).map_err(|err| SmahError::Internal {
        detail: format!("yaml encode: {err}"),
    })?;
    secrets::write_private_file_atomic(path, text.as_bytes()).map_err(|err| {
        SmahError::ConfigUnreadable {
            path: path.display().to_string(),
            detail: err.to_string(),
        }
    })?;

    // The file must load cleanly as the app would read it.
    Settings::load_from(path)?;

    // Only claim 0600 when it is true (F1: no false success lines).
    let mode_note = if cfg!(unix) {
        match private_mode(path) {
            Some(true) => "chmod 0600",
            _ => "PERMISSIONS UNKNOWN — verify manually",
        }
    } else {
        "protected by your user-profile ACLs"
    };
    eprintln!(
        "smah: config written to {} (provider: {provider}; key: {store_note}; {mode_note})",
        path.display()
    );
    Ok(())
}

/// Why the fallback (file) backend was chosen, when it was chosen by
/// availability rather than flag (wizard narration; F2).
fn fallback_reason(choice: StoreChoice) -> Option<String> {
    match choice {
        StoreChoice::File if !secrets::keyring_available() => Some(
            "no OS secret store reachable on this platform (headless? no D-Bus session bus)"
                .to_string(),
        ),
        _ => None,
    }
}

/// The unix mode of `path` is owner-only.
#[cfg(unix)]
fn private_mode(path: &Path) -> Option<bool> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .ok()
        .map(|meta| meta.permissions().mode() & 0o777 == 0o600)
}

#[cfg(not(unix))]
fn private_mode(_path: &Path) -> Option<bool> {
    None
}

/// `--secret-store` override or availability-based default.
fn resolve_store_choice(
    args: &ConfigInitArgs,
    interactive: bool,
) -> Result<StoreChoice, SmahError> {
    match args.secret_store.as_deref() {
        Some(text) => StoreChoice::parse(text).ok_or_else(|| SmahError::Usage {
            message: "smah: --secret-store is one of keychain | file | env".to_string(),
        }),
        None => {
            // Headless runs never hold a literal (no prompt): the key comes
            // from --key-env or the provider env chain, so `env` semantics
            // apply regardless. Interactive picks the best backend.
            if interactive {
                Ok(if secrets::keyring_available() {
                    StoreChoice::Keyring
                } else {
                    StoreChoice::File
                })
            } else {
                Ok(StoreChoice::Env)
            }
        }
    }
}

/// `smah config migrate-secrets` (F2): move every literal API key in
/// config.yaml into the secret store and rewrite the config to hold
/// references. The literal is registered with the redactor before any I/O;
/// the rewritten config goes through the same atomic 0600 write.
fn run_migrate_secrets(args: &ConfigMigrateArgs, cli_config: Option<&std::path::Path>) -> i32 {
    let path = target_path(args.config_path.as_deref().or(cli_config));
    match migrate_secrets_inner(args, &path) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("{}", err.render("config-migrate-secrets"));
            err.exit_code().code()
        }
    }
}

fn migrate_secrets_inner(args: &ConfigMigrateArgs, path: &Path) -> Result<(), SmahError> {
    let settings = Settings::load_from(path)?;
    let config_dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let (kind, store_desc) = match args.secret_store.as_deref() {
        Some("keychain") => (StoreKind::Keyring, secrets::KeyringStore.describe()),
        Some("file") => (
            StoreKind::File,
            secrets::FileStore {
                path: config_dir.join(secrets::SECRETS_FILE),
            }
            .describe(),
        ),
        Some("env") => {
            return Err(SmahError::Usage {
                message: "smah: literal keys cannot migrate to env — use --key-env in `config init` (or export SMAH_<PROVIDER>_API_KEY)".to_string(),
            })
        }
        Some(other) => {
            return Err(SmahError::Usage {
                message: format!("smah: --secret-store is keychain | file (got '{other}')"),
            })
        }
        None => {
            let (k, reason) = secrets::best_available_store_kind();
            (k, reason)
        }
    };

    let mut document = settings.value().clone();
    let Some(providers) = document
        .get_mut(Value::String("inference".into()))
        .and_then(|v| v.get_mut(Value::String("providers".into())))
        .and_then(|v| v.as_mapping_mut())
    else {
        eprintln!(
            "smah: no provider blocks in {} — nothing to migrate",
            path.display()
        );
        return Ok(());
    };

    let mut moved: Vec<(String, String)> = Vec::new();
    for (provider_key, block) in providers.iter_mut() {
        let provider = provider_key.as_str().unwrap_or_default().to_string();
        let Some(settings_map) = block.get_mut(Value::String("settings".into())) else {
            continue;
        };
        let Some(api_key_value) = settings_map.get_mut(Value::String("api_key".into())) else {
            continue;
        };
        let Some(raw) = api_key_value.as_str().map(str::to_string) else {
            continue;
        };
        if raw.starts_with('$') || secrets::parse_secret_ref(&raw, &config_dir).is_some() {
            continue; // already a reference
        }
        // Literal: register with the redactor before any further I/O.
        smah_core::logging::register_secret(raw.clone());
        let reference = secrets::store_reference(kind, &provider, &config_dir);
        let backend: &dyn SecretStore = match kind {
            StoreKind::Keyring => &secrets::KeyringStore,
            StoreKind::File => &secrets::FileStore {
                path: config_dir.join(secrets::SECRETS_FILE),
            },
            StoreKind::Runtime => unreachable!("migrate never selects Runtime"),
        };
        backend
            .set(&reference, &raw)
            .map_err(|detail| SmahError::Internal {
                detail: format!(
                "cannot store the {provider} key in {}: {detail} — retry with --secret-store file",
                backend.describe()
            ),
            })?;
        *api_key_value = Value::String(reference.encode());
        moved.push((provider, store_desc.clone()));
    }

    if moved.is_empty() {
        eprintln!(
            "smah: no literal API keys in {} — nothing to migrate",
            path.display()
        );
        return Ok(());
    }

    let text = serde_yaml_ng::to_string(&document).map_err(|err| SmahError::Internal {
        detail: format!("yaml encode: {err}"),
    })?;
    secrets::write_private_file_atomic(path, text.as_bytes()).map_err(|err| {
        SmahError::ConfigUnreadable {
            path: path.display().to_string(),
            detail: err.to_string(),
        }
    })?;
    Settings::load_from(path)?;

    for (provider, desc) in &moved {
        eprintln!("smah: moved {provider} API key → {desc}; config now holds a reference");
    }
    Ok(())
}

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
            secret_store: StoreChoice::File,
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
    fn literal_key_never_lands_in_the_document_only_a_reference() {
        // F2: the masked-entry path stores the key in a secret store; the
        // document carries the reference.
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
            Value::String("file:secrets.yaml#anthropic".to_string())
        );
        let text = serde_yaml_ng::to_string(&value).expect("yaml");
        assert!(
            !text.contains("test-key-0000"),
            "the literal key must never appear in config.yaml: {text}"
        );
    }

    #[test]
    fn keyring_store_writes_a_keychain_reference() {
        let mut d = draft();
        d.secret_store = StoreChoice::Keyring;
        let value = build_document(&d);
        assert_eq!(
            value["inference"]["providers"]["anthropic"]["settings"]["api_key"],
            Value::String("keychain:smah/anthropic".to_string())
        );
    }

    #[test]
    fn env_reference_wins_over_literal() {
        let mut d = draft();
        d.api_key_env_ref = Some("SMAH_ANTHROPIC_API_KEY".to_string());
        let value = build_document(&d);
        assert_eq!(
            value["inference"]["providers"]["anthropic"]["settings"]["api_key"],
            Value::String("env:SMAH_ANTHROPIC_API_KEY".to_string())
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

    #[cfg(unix)]
    #[test]
    fn atomic_private_write_is_what_init_uses() {
        // F1: the init write path is write_private_file_atomic — owner-only
        // from the first byte, no temp litter.
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("nested/config.yaml");
        secrets::write_private_file_atomic(&path, b"vsn: \"0.0.1\"\n").expect("write");
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "config created 0600 from the start");
        assert!(private_mode(&path) == Some(true), "mode check agrees");
        let entries: Vec<_> = std::fs::read_dir(dir.path().join("nested"))
            .expect("dir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            entries,
            vec!["config.yaml".to_string()],
            "no temp files left"
        );
    }
}

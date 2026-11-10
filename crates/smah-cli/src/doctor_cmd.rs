//! `smah doctor` (RA-07): provider/config health from the registry — no
//! network calls, no key material printed (presence only, resolved through
//! the same seam the runtime uses so `doctor ok` means `ask` can run).

use smah_ask::EgressToggles;
use smah_core::errors::SmahError;
use smah_core::settings::Settings;

use crate::args::DoctorArgs;

/// One provider's status line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderStatus {
    pub key: String,
    pub configured: bool,
    pub enabled: bool,
    pub has_key: bool,
    pub models: usize,
}

impl ProviderStatus {
    /// A provider that can actually serve a request now.
    pub fn usable(&self) -> bool {
        self.has_key && self.enabled
    }
}

/// Collect status for every known registry provider.
pub fn provider_statuses(settings: &Settings) -> Vec<ProviderStatus> {
    smah_providers::registry::known_provider_keys()
        .into_iter()
        .map(|key| {
            let view = settings.inference().and_then(|inf| {
                inf.provider_list()
                    .into_iter()
                    .find(|(id, _)| id == key)
                    .map(|(_, v)| v)
            });
            let has_key = smah_providers::credentials::resolve_api_key(settings, key, None)
                .unwrap_or(None)
                .is_some();
            match view {
                Some(view) => ProviderStatus {
                    key: key.to_string(),
                    configured: true,
                    enabled: view.is_enabled(),
                    has_key,
                    models: view.models.len(),
                },
                None => ProviderStatus {
                    key: key.to_string(),
                    configured: false,
                    enabled: false,
                    has_key,
                    models: 0,
                },
            }
        })
        .collect()
}

/// Providers whose `settings.api_key` is still a **literal** in
/// config.yaml (RA-10 F2 companion): references (`env:`/`keychain:`/
/// `file:`), legacy `$VAR` values, and absent keys are not literals. The
/// scan reads the raw YAML — presence only, the value itself is never
/// returned or printed.
fn literal_key_providers(settings: &Settings) -> Vec<String> {
    let path = settings.path();
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&text) else {
        return Vec::new();
    };
    let config_dir = path.parent().unwrap_or(std::path::Path::new("."));
    let Some(providers) = value["inference"]["providers"].as_mapping() else {
        return Vec::new();
    };
    let mut literals = Vec::new();
    for (key, provider) in providers {
        let Some(key) = key.as_str() else {
            continue;
        };
        let Some(raw) = provider["settings"]["api_key"].as_str() else {
            continue;
        };
        if raw.is_empty() || raw.starts_with('$') {
            continue;
        }
        if smah_core::secrets::parse_secret_ref(raw, config_dir).is_some() {
            continue;
        }
        literals.push(key.to_string());
    }
    literals
}

/// The doctor's literal-key warning, one string per affected provider
/// (human line and JSON array entry share the wording).
fn literal_warning(provider: &str) -> String {
    format!(
        "provider '{provider}' stores api_key as a literal in config.yaml — run: smah config migrate-secrets"
    )
}

/// Run `smah doctor`; returns the process exit code (0 ≥1 usable provider,
/// 4 none, 3 broken config).
pub fn run_doctor(args: &DoctorArgs, cli_config: Option<&std::path::Path>) -> i32 {
    let settings = match cli_config {
        Some(path) => Settings::load_from(path),
        None => Settings::load(),
    };
    let settings = match settings {
        Ok(settings) => settings,
        Err(err) => {
            if args.json {
                print_json_error(&err);
            } else {
                eprintln!("{}", err.render("doctor"));
            }
            return err.exit_code().code();
        }
    };

    let statuses = provider_statuses(&settings);
    let usable = statuses.iter().filter(|s| s.usable()).count();
    let literals = literal_key_providers(&settings);
    let toggles = EgressToggles::from_settings(&settings);
    let config_path = settings.path().display().to_string();

    if args.json {
        let value = serde_json::json!({
            "config": config_path,
            "egress": {
                "share_profile": toggles.share_profile,
                "share_system_stats": toggles.share_system_stats,
            },
            "usable_providers": usable,
            "providers": statuses
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "key": s.key,
                        "configured": s.configured,
                        "enabled": s.enabled,
                        "has_key": s.has_key,
                        "models": s.models,
                    })
                })
                .collect::<Vec<_>>(),
            "warnings": literals
                .iter()
                .map(|p| literal_warning(p))
                .collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        );
    } else {
        println!("smah doctor — provider status (no network calls; keys never shown)");
        for status in &statuses {
            println!("  {}", human_line(status));
        }
        println!("egress: {}", smah_ask::egress_disclosure(&toggles));
        println!(
            "config: {config_path} — privacy toggles: privacy.share_profile / privacy.share_system_stats"
        );
        if usable == 0 {
            println!("result: no usable provider — set a key (smah config init) or export SMAH_<PROVIDER>_API_KEY");
        } else {
            println!("result: {usable} usable provider(s) — ok");
        }
        for provider in &literals {
            println!("warning: {}", literal_warning(provider));
        }
    }

    if usable == 0 {
        4
    } else {
        0
    }
}

fn human_line(status: &ProviderStatus) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("{}:", status.key));
    if !status.configured {
        parts.push("not configured".to_string());
    } else {
        parts.push(if status.enabled {
            "enabled".to_string()
        } else {
            "disabled".to_string()
        });
        parts.push(format!("{} models", status.models));
    }
    parts.push(if status.has_key {
        "key present".to_string()
    } else {
        "no key".to_string()
    });
    if status.usable() {
        parts.push("· usable".to_string());
    }
    parts.join(" ")
}

fn print_json_error(err: &SmahError) {
    println!(
        "{}",
        serde_json::json!({
            "error": err.to_string(),
            "usable_providers": 0,
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(text: &str) -> Settings {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, text).expect("write");
        Settings::load_from(&path).expect("load")
    }

    const KEYED: &str = "vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers:\n    anthropic:\n      name: Anthropic\n      enabled: true\n      settings:\n        api_key: test-key-0000\n      models: []\n";

    #[test]
    fn keyed_provider_is_usable() {
        // Env-independent: the key comes from the config block.
        let statuses = provider_statuses(&settings(KEYED));
        let anthropic = statuses.iter().find(|s| s.key == "anthropic").expect("row");
        assert!(anthropic.configured);
        assert!(anthropic.enabled);
        assert!(anthropic.has_key);
        assert!(anthropic.usable());
    }

    #[test]
    fn empty_settings_leave_everyone_keyless() {
        // No ambient env keys in the unit-test environment guarantee —
        // resolve only against the (empty) settings.
        let empty = settings("vsn: \"0.0.1\"\n");
        let statuses = provider_statuses(&empty);
        assert!(statuses.iter().all(|s| !s.configured));
        // openai without a block has no key at all; the others depend on
        // env — assert the structural invariant, not the env.
        assert!(statuses.iter().any(|s| s.key == "openai" && !s.has_key));
    }

    /// F2 companion: literal keys are flagged; references, `$VAR`, and
    /// absent keys are not. Keeps the tempdir alive so settings.path()
    /// stays readable (the shared `settings()` helper drops it).
    #[test]
    fn literal_keys_are_flagged_references_are_not() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("config.yaml");

        std::fs::write(&path, KEYED).expect("write");
        let flagged = Settings::load_from(&path).expect("load");
        assert_eq!(literal_key_providers(&flagged), vec!["anthropic"]);

        std::fs::write(&path, KEYED.replace("test-key-0000", "$ANTHROPIC_API_KEY")).expect("write");
        let legacy = Settings::load_from(&path).expect("load");
        assert!(literal_key_providers(&legacy).is_empty());

        std::fs::write(
            &path,
            KEYED.replace("test-key-0000", "keychain:smah/anthropic"),
        )
        .expect("write");
        let stored = Settings::load_from(&path).expect("load");
        assert!(literal_key_providers(&stored).is_empty());

        std::fs::write(
            &path,
            KEYED.replace("test-key-0000", "env:ANTHROPIC_API_KEY"),
        )
        .expect("write");
        let envref = Settings::load_from(&path).expect("load");
        assert!(literal_key_providers(&envref).is_empty());

        std::fs::write(
            &path,
            KEYED.replace("test-key-0000", "file:secrets.yaml#anthropic"),
        )
        .expect("write");
        let fileref = Settings::load_from(&path).expect("load");
        assert!(literal_key_providers(&fileref).is_empty());
    }

    #[test]
    fn literal_warning_names_the_remedy() {
        let w = literal_warning("openai");
        assert!(w.contains("openai"));
        assert!(w.contains("config migrate-secrets"));
    }

    #[test]
    fn disabled_provider_is_not_usable_even_with_key() {
        let disabled = KEYED.replace("enabled: true", "enabled: false");
        let statuses = provider_statuses(&settings(&disabled));
        let anthropic = statuses.iter().find(|s| s.key == "anthropic").expect("row");
        assert!(anthropic.has_key);
        assert!(!anthropic.usable());
    }
}

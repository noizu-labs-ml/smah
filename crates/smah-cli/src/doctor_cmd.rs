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

    #[test]
    fn disabled_provider_is_not_usable_even_with_key() {
        let disabled = KEYED.replace("enabled: true", "enabled: false");
        let statuses = provider_statuses(&settings(&disabled));
        let anthropic = statuses.iter().find(|s| s.key == "anthropic").expect("row");
        assert!(anthropic.has_key);
        assert!(!anthropic.usable());
    }
}

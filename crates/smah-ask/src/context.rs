//! Operator/system context assembly (the Python `Prompts.system_settings`
//! role, rebuilt with the RA-06 egress contract): what goes to the provider
//! is exactly what the disclosure says, and the two opt-out toggles gate it.
//!
//! * `privacy.share_profile` (default on) — the operator block
//!   (name/experience/role/about from config).
//! * `privacy.share_system_stats` (default on) — the system block (OS
//!   identity via `sysinfo`/file-free std calls, shell, core/memory
//!   counts). No process spawning anywhere (print-only invariant).
//!
//! Both toggles off ⇒ no context message at all — the provider sees only
//! the question.

use serde_yaml_ng::Value;
use smah_core::settings::Settings;

/// Egress toggles read from the config's `privacy` block (defaults: share).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EgressToggles {
    /// Send the operator profile block.
    pub share_profile: bool,
    /// Send the system/stats block.
    pub share_system_stats: bool,
}

impl Default for EgressToggles {
    fn default() -> Self {
        EgressToggles {
            share_profile: true,
            share_system_stats: true,
        }
    }
}

impl EgressToggles {
    /// Read the toggles from the config Value (unknown/missing ⇒ defaults).
    pub fn from_settings(settings: &Settings) -> Self {
        let privacy = settings
            .value()
            .get(Value::String("privacy".to_string()))
            .and_then(|v| v.as_mapping());
        let flag = |key: &str| {
            privacy
                .and_then(|m| m.get(Value::String(key.to_string())))
                .and_then(|v| v.as_bool())
        };
        EgressToggles {
            share_profile: flag("share_profile").unwrap_or(true),
            share_system_stats: flag("share_system_stats").unwrap_or(true),
        }
    }
}

/// The human-facing disclosure line (RA-06 prints this at config init and
/// `smah doctor` reports the state); reflects the toggles.
pub fn egress_disclosure(toggles: &EgressToggles) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if toggles.share_profile {
        parts.push("your background profile");
    }
    if toggles.share_system_stats {
        parts.push("your system details (OS, shell, hardware counts)");
    }
    if parts.is_empty() {
        "Sent to the provider with every query: your question and any piped input only.".to_string()
    } else {
        format!(
            "Sent to the provider with every query: your question, any piped input, and {}.",
            parts.join(" and ")
        )
    }
}

/// The operator block (`# Operator — about your operator`), from the typed
/// user view. Empty string when the section is absent.
fn operator_block(settings: &Settings) -> String {
    match settings.user() {
        Some(user) => format!(
            "# Operator — about your operator\nname: {}\nexperience: {}\nrole: {}\nabout: {}",
            user.name.as_deref().unwrap_or(""),
            user.system_admin_experience.as_deref().unwrap_or(""),
            user.role.as_deref().unwrap_or(""),
            user.about.as_deref().unwrap_or(""),
        ),
        None => String::new(),
    }
}

/// The system block: OS identity + shell + hardware counts, gathered
/// without any subprocess (`sysinfo` + std env). Values are best-effort —
/// unknown fields render as `unknown` rather than blocking the query.
fn system_block() -> String {
    use sysinfo::System;
    let mut sys = System::new();
    sys.refresh_memory();
    let name = System::name().unwrap_or_else(|| "unknown".to_string());
    let os_version = System::os_version().unwrap_or_else(|| "unknown".to_string());
    let kernel = System::kernel_version().unwrap_or_else(|| "unknown".to_string());
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "unknown".to_string());
    let cores = System::physical_core_count()
        .map(|c| c.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let memory_gib = (sys.total_memory() as f64 / 1024.0 / 1024.0 / 1024.0 * 10.0).round() / 10.0;
    format!(
        "# System — about this system\nos: {name} {os_version} (kernel {kernel})\narch: {}\nshell: {shell}\ncpu physical cores: {cores}\nmemory total: {memory_gib} GiB",
        std::env::consts::ARCH,
    )
}

/// Assemble the context message body from settings + toggles. `None` when
/// both blocks are off (or nothing is configured to share).
pub fn context_message(settings: &Settings, toggles: &EgressToggles) -> Option<String> {
    let mut blocks: Vec<String> = Vec::new();
    if toggles.share_profile {
        let operator = operator_block(settings);
        if !operator.is_empty() {
            blocks.push(operator);
        }
    }
    if toggles.share_system_stats {
        blocks.push(system_block());
    }
    if blocks.is_empty() {
        None
    } else {
        Some(blocks.join("\n\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_with(text: &str) -> Settings {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, text).expect("write config");
        Settings::load_from(&path).expect("load")
    }

    const CONFIG: &str = "vsn: \"0.0.1\"\nuser:\n  vsn: \"0.0.1\"\n  name: Ines\n  system_admin_experience: Expert\n  role: Developer\n  about: data engineer\n";

    #[test]
    fn default_toggles_share_everything() {
        assert_eq!(
            EgressToggles::from_settings(&settings_with(CONFIG)),
            EgressToggles::default()
        );
    }

    #[test]
    fn context_contains_operator_and_system() {
        let settings = settings_with(CONFIG);
        let message = context_message(&settings, &EgressToggles::default()).expect("context");
        assert!(message.contains("# Operator — about your operator"));
        assert!(message.contains("name: Ines"));
        assert!(message.contains("# System — about this system"));
        assert!(message.contains("arch: "));
    }

    #[test]
    fn share_profile_off_drops_operator_block() {
        let settings = settings_with(&format!("{CONFIG}privacy:\n  share_profile: false\n"));
        let toggles = EgressToggles::from_settings(&settings);
        assert!(!toggles.share_profile);
        let message = context_message(&settings, &toggles).expect("context");
        assert!(!message.contains("# Operator"));
        assert!(message.contains("# System"));
    }

    #[test]
    fn share_system_stats_off_drops_system_block() {
        let settings = settings_with(&format!("{CONFIG}privacy:\n  share_system_stats: false\n"));
        let toggles = EgressToggles::from_settings(&settings);
        assert!(!toggles.share_system_stats);
        let message = context_message(&settings, &toggles).expect("context");
        assert!(message.contains("# Operator"));
        assert!(!message.contains("# System"));
    }

    #[test]
    fn both_off_means_no_context() {
        let settings = settings_with(&format!(
            "{CONFIG}privacy:\n  share_profile: false\n  share_system_stats: false\n"
        ));
        let toggles = EgressToggles::from_settings(&settings);
        assert_eq!(context_message(&settings, &toggles), None);
    }

    #[test]
    fn disclosure_reflects_toggles() {
        let on = egress_disclosure(&EgressToggles::default());
        assert!(on.contains("your background profile"));
        assert!(on.contains("your system details"));
        let off = egress_disclosure(&EgressToggles {
            share_profile: false,
            share_system_stats: false,
        });
        assert!(off.contains("your question and any piped input only"));
    }

    #[test]
    fn missing_user_section_still_shares_system() {
        let settings = settings_with("vsn: \"0.0.1\"\n");
        let message = context_message(&settings, &EgressToggles::default()).expect("context");
        assert!(!message.contains("# Operator"));
        assert!(message.contains("# System"));
    }
}

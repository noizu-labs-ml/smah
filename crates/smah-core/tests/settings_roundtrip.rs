//! Config round-trip fixtures (plan §E.7): load/save without loss or
//! reorder, string-compare vsn gate, `$ENV_VAR` indirection resolution, and
//! the OpenAI no-block special case.

use serde_yaml_ng::Value;
use smah_core::errors::SmahError;
use smah_core::settings::Settings;
use smah_core::ExitCode;

const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn fixture_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(FIXTURE_DIR).join(name)
}

fn load_fixture(name: &str) -> Settings {
    Settings::load_from(&fixture_path(name)).expect("fixture must load")
}

/// A parsed fixture must re-dump byte-identically: this is what makes the
/// hand-written fixtures a hard order + unknown-key contract, not prose.
#[test]
fn fixtures_are_byte_exact_under_serde_yaml_ng() {
    for name in [
        "config_anthropic.yaml",
        "config_openai.yaml",
        "config_minimal.yaml",
    ] {
        let path = fixture_path(name);
        let text = std::fs::read_to_string(&path).expect("fixture readable");
        let value: Value = serde_yaml_ng::from_str(&text).expect("fixture parses");
        let dumped = serde_yaml_ng::to_string(&value).expect("fixture dumps");
        assert_eq!(dumped, text, "{name}: canonical dump must match bytes");
    }
}

#[test]
fn load_save_round_trip_is_byte_identical() {
    for name in [
        "config_anthropic.yaml",
        "config_openai.yaml",
        "config_minimal.yaml",
    ] {
        let original = std::fs::read_to_string(fixture_path(name)).expect("fixture readable");
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("config.yaml");

        let mut settings = retarget(&load_fixture(name), &target);
        settings.save().expect("save");

        let saved = std::fs::read_to_string(&target).expect("saved file");
        assert_eq!(
            saved, original,
            "{name}: save must round-trip byte-identically"
        );
    }
}

/// Re-seat a loaded Settings at a writable path: re-dump the untouched Value
/// to the new location and load it back (the save leg of the round-trip).
fn retarget(settings: &Settings, path: &std::path::Path) -> Settings {
    let text =
        serde_yaml_ng::to_string(&Value::Mapping(settings.value().clone())).expect("re-dump");
    std::fs::create_dir_all(path.parent().expect("parent dir")).expect("mkdir");
    std::fs::write(path, text).expect("seed file");
    Settings::load_from(path).expect("reload")
}

#[test]
fn unknown_keys_survive_the_round_trip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("config.yaml");
    let mut settings = retarget(&load_fixture("config_openai.yaml"), &target);
    settings.save().expect("save");
    let saved = std::fs::read_to_string(&target).expect("saved");

    // Unknown keys the typed views do not model.
    assert!(saved.contains("legacy_flag: carried-forward"));
    assert!(saved.contains("vendor_note: internal-only"));
    // And the canonical top-level key order is intact.
    let vsn_at = saved.find("vsn:").expect("vsn present");
    let user_at = saved.find("user:").expect("user present");
    let system_at = saved.find("system:").expect("system present");
    let inference_at = saved.find("inference:").expect("inference present");
    assert!(vsn_at < user_at && user_at < system_at && system_at < inference_at);
}

#[test]
fn gate_rejects_vsn_0_0_2() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.yaml");
    std::fs::write(&path, "vsn: 0.0.2\n").expect("write config");

    let err = Settings::load_from(&path).expect_err("gate must reject 0.0.2");
    assert_eq!(err.exit_code(), ExitCode::Config);
    assert!(
        matches!(err, SmahError::ConfigGate { .. }),
        "expected ConfigGate, got {err:?}"
    );
}

#[test]
fn gate_rejects_missing_vsn() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.yaml");
    std::fs::write(&path, "user:\n  name: Keith\n").expect("write config");

    let err = Settings::load_from(&path).expect_err("gate must reject missing vsn");
    assert!(matches!(err, SmahError::ConfigGate { .. }));
}

#[test]
fn gate_accepts_vsn_equal_to_0_0_1() {
    assert!(Settings::vsn_supported(&Some("0.0.1".to_string())));
    assert!(Settings::vsn_supported(&Some("0.0".to_string())));
    assert!(!Settings::vsn_supported(&Some("0.0.2".to_string())));
    assert!(!Settings::vsn_supported(&Some("0.1.0".to_string())));
    assert!(!Settings::vsn_supported(&Some("1.0.0".to_string())));
    assert!(!Settings::vsn_supported(&Some(String::new())));
    assert!(!Settings::vsn_supported(&None));
}

#[test]
fn missing_config_file_loads_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("does_not_exist.yaml");
    let settings = Settings::load_from(&path).expect("missing file is not an error");
    assert!(!settings.is_loaded());
    assert!(!settings.is_configured());
}

#[test]
fn invalid_yaml_maps_to_config_unreadable_exit_3() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.yaml");
    std::fs::write(&path, "vsn: 0.0.1\nuser: [unclosed\n").expect("write config");

    let err = Settings::load_from(&path).expect_err("parse must fail");
    assert_eq!(err.exit_code(), ExitCode::Config);
    assert!(matches!(err, SmahError::ConfigUnreadable { .. }));
}

#[test]
fn indirection_resolves_through_the_environment() {
    let settings = load_fixture("config_anthropic.yaml");

    // `$ANTHROPIC_API_KEY` resolves from the named variable.
    let lookup =
        |name: &str| (name == "ANTHROPIC_API_KEY").then(|| "resolved-from-env".to_string());
    assert_eq!(
        settings.provider_api_key_with("anthropic", None, &lookup),
        Some("resolved-from-env".to_string())
    );
}

#[test]
fn indirection_falls_through_to_provider_env_chain_when_var_unset() {
    let settings = load_fixture("config_anthropic.yaml");

    // `$ANTHROPIC_API_KEY` unset → SMAH_ANTHROPIC_API_KEY → ANTHROPIC_API_KEY.
    let lookup = |name: &str| (name == "SMAH_ANTHROPIC_API_KEY").then(|| "chain-hit".to_string());
    assert_eq!(
        settings.provider_api_key_with("anthropic", None, &lookup),
        Some("chain-hit".to_string())
    );

    let lookup = |name: &str| (name == "ANTHROPIC_API_KEY").then(|| "plain-hit".to_string());
    assert_eq!(
        settings.provider_api_key_with("anthropic", None, &lookup),
        Some("plain-hit".to_string())
    );
}

#[test]
fn literal_key_returns_as_is() {
    let settings = load_fixture("config_openai.yaml");
    let lookup = |_: &str| Some("env-should-not-win".to_string());
    assert_eq!(
        settings.provider_api_key_with("openai", None, &lookup),
        Some("literal-openai-key-do-not-log".to_string()),
        "a literal settings key must beat the env chain"
    );
}

#[test]
fn cli_flag_folds_into_openai_block_only() {
    let settings = load_fixture("config_openai.yaml");
    let lookup = |_: &str| None;

    // OpenAI: the flag wins over settings.
    assert_eq!(
        settings.provider_api_key_with("openai", Some("flag-key"), &lookup),
        Some("flag-key".to_string())
    );

    // Non-openai providers ignore the flag entirely.
    let anthropic = load_fixture("config_anthropic.yaml");
    let lookup = |name: &str| (name == "ANTHROPIC_API_KEY").then(|| "env".to_string());
    assert_eq!(
        anthropic.provider_api_key_with("anthropic", Some("flag-key"), &lookup),
        Some("env".to_string()),
        "the CLI flag must not leak into non-openai providers"
    );
}

#[test]
fn openai_without_config_block_has_no_key_even_with_env_set() {
    // The anthropic fixture has no `openai` block; resolution must return
    // None even though the env lookup would happily answer.
    let settings = load_fixture("config_anthropic.yaml");
    let lookup = |name: &str| {
        (name == "OPENAI_API_KEY" || name == "SMAH_OPENAI_API_KEY")
            .then(|| "env-openai-key".to_string())
    };
    assert_eq!(
        settings.provider_api_key_with("openai", None, &lookup),
        None
    );
    assert_eq!(
        settings.provider_api_key_with("openai", Some("flag-key"), &lookup),
        None,
        "no openai block means no openai key, flag notwithstanding"
    );
}

#[test]
fn brace_indirection_strips_braces() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.yaml");
    std::fs::write(
        &path,
        "vsn: 0.0.1\ninference:\n  providers:\n    anthropic:\n      enabled: true\n      settings:\n        api_key: ${ANTHROPIC_API_KEY}\n",
    )
    .expect("write config");
    let settings = Settings::load_from(&path).expect("load");
    let lookup = |name: &str| (name == "ANTHROPIC_API_KEY").then(|| "braced".to_string());
    assert_eq!(
        settings.provider_api_key_with("anthropic", None, &lookup),
        Some("braced".to_string())
    );
}

#[test]
fn views_and_is_configured() {
    let full = load_fixture("config_anthropic.yaml");
    assert!(full.user().expect("user view").is_configured());
    assert!(full.system().expect("system view").is_configured());
    assert!(full.inference().expect("inference view").is_configured());
    assert!(full.is_configured());

    let openai = load_fixture("config_openai.yaml");
    // user/system sections are null → not configured overall.
    assert!(!openai.is_configured());
    let inference = openai.inference().expect("inference view");
    assert!(inference.is_configured());
    let providers = inference.provider_list();
    let (id, provider) = &providers[0];
    assert_eq!(id, "openai");
    assert!(provider.is_configured());
    // The disabled model needs no configuration; the enabled one is complete.
    assert_eq!(provider.models.len(), 2);
}

#[test]
fn user_view_requires_all_four_fields() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.yaml");
    std::fs::write(
        &path,
        "vsn: 0.0.1\nuser:\n  name: Keith\n  system_admin_experience: expert\n  role: sysadmin\n",
    )
    .expect("write config");
    let settings = Settings::load_from(&path).expect("load");
    let user = settings.user().expect("user view");
    assert!(!user.is_configured(), "missing `about` must fail the check");
}

#[test]
fn show_redacted_masks_secrets_and_names_env_vars() {
    let anthropic = load_fixture("config_anthropic.yaml");
    let shown = anthropic.show_redacted();
    assert!(
        shown.contains("api_key: <env:ANTHROPIC_API_KEY>"),
        "indirected key must render as <env:VAR>, got:\n{shown}"
    );
    assert!(!shown.contains("$ANTHROPIC_API_KEY"));

    let openai = load_fixture("config_openai.yaml");
    let shown = openai.show_redacted();
    assert!(
        shown.contains("api_key: ••••••••"),
        "literal key must render as the mask, got:\n{shown}"
    );
    assert!(!shown.contains("literal-openai-key-do-not-log"));
}

#[test]
fn defaults_catalog_embeds_and_parses() {
    let defaults = smah_core::settings::inference_defaults();
    let vsn = defaults.get(Value::String("vsn".to_string()));
    assert_eq!(vsn, Some(&Value::String("0.0.1".to_string())));
}

//! Insta snapshots of every §4 message row plus its exit code
//! (plan §E.2). The internal row exits **1** — the spec §4 erratum encoded.
//! Rendered lines use a fixed fake log path so the snapshots lock both the
//! §4 text and the "every error ends by naming the log file" suffix rule.

use smah_core::errors::SmahError;
use smah_core::ExitCode;

const LOG: &str = "/home/tester/.smah/logs/smah.20260101_000000.log";

fn snap(name: &str, error: &SmahError, expected: ExitCode) {
    assert_eq!(error.exit_code(), expected, "{name}: wrong exit class");
    insta::assert_snapshot!(name, error.render(LOG));
}

#[test]
fn row_ask_needs_query() {
    snap(
        "row_ask_needs_query",
        &SmahError::AskNeedsQuery,
        ExitCode::Usage,
    );
}

#[test]
fn row_usage_generic() {
    snap(
        "row_usage_generic",
        &SmahError::Usage {
            message: "smah: removed flag --stream (removed in 0.4). Use plain 'smah ask'."
                .to_string(),
        },
        ExitCode::Usage,
    );
}

#[test]
fn row_config_unreadable() {
    snap(
        "row_config_unreadable",
        &SmahError::ConfigUnreadable {
            path: "/home/tester/.smah/config.yaml".to_string(),
            detail: "mapping values are not allowed in this context".to_string(),
        },
        ExitCode::Config,
    );
}

#[test]
fn row_config_gate() {
    snap(
        "row_config_gate",
        &SmahError::ConfigGate {
            path: "/home/tester/.smah/config.yaml".to_string(),
            detail: "config version 0.0.2 is not supported by this version of SMAH".to_string(),
        },
        ExitCode::Config,
    );
}

#[test]
fn row_database_problem() {
    snap(
        "row_database_problem",
        &SmahError::Database {
            path: "/home/tester/.smah/smah.db".to_string(),
            detail: "file is not a database".to_string(),
        },
        ExitCode::Config,
    );
}

#[test]
fn row_missing_api_key() {
    snap(
        "row_missing_api_key",
        &SmahError::MissingApiKey {
            provider: "anthropic".to_string(),
        },
        ExitCode::Auth,
    );
}

#[test]
fn row_no_models_enabled() {
    snap(
        "row_no_models_enabled",
        &SmahError::NoModelsEnabled,
        ExitCode::Auth,
    );
}

#[test]
fn row_provider_auth() {
    snap(
        "row_provider_auth",
        &SmahError::ProviderAuth {
            provider: "anthropic".to_string(),
            code: 401,
        },
        ExitCode::Auth,
    );
}

#[test]
fn row_provider_request() {
    snap(
        "row_provider_request",
        &SmahError::ProviderRequest {
            provider: "anthropic".to_string(),
            code: 404,
            detail: "model not found: claude-turbo".to_string(),
        },
        ExitCode::Provider,
    );
}

#[test]
fn row_provider_unavailable() {
    snap(
        "row_provider_unavailable",
        &SmahError::ProviderUnavailable {
            provider: "anthropic".to_string(),
            code: 503,
        },
        ExitCode::Provider,
    );
}

#[test]
fn row_network_failure() {
    snap(
        "row_network_failure",
        &SmahError::Network {
            provider: "anthropic".to_string(),
            detail: "connection timed out".to_string(),
        },
        ExitCode::Provider,
    );
}

#[test]
fn row_internal_error_exits_1() {
    snap(
        "row_internal_error",
        &SmahError::Internal {
            detail: "unexpected None in plan validation".to_string(),
        },
        // Erratum: spec §4 table says 5; code + spec footer say 1.
        ExitCode::Internal,
    );
}

// ---------------------------------------------------------------------------
// V-series — virtual tools (#17). Same snapshot discipline: exact text +
// exit class. Documented in docs/ux/virtual-tools-cli-driver.md §6; the §4
// table itself stays untouched.
// ---------------------------------------------------------------------------

#[test]
fn v_row_tool_not_found() {
    snap(
        "v_row_tool_not_found",
        &SmahError::ToolNotFound {
            name: "log-analyzer".to_string(),
            installed: "(none)".to_string(),
        },
        ExitCode::Usage,
    );
}

#[test]
fn v_row_tool_not_found_with_installed() {
    snap(
        "v_row_tool_not_found_with_installed",
        &SmahError::ToolNotFound {
            name: "logz".to_string(),
            installed: "commit-msg, log-analyzer".to_string(),
        },
        ExitCode::Usage,
    );
}

#[test]
fn v_row_tool_file_invalid() {
    snap(
        "v_row_tool_file_invalid",
        &SmahError::ToolFileInvalid {
            path: "/home/tester/.config/smah/virtual-tools/broken.md".to_string(),
            detail: "unknown frontmatter key 'constrants'".to_string(),
        },
        ExitCode::Config,
    );
}

#[test]
fn v_row_directive_violation() {
    snap(
        "v_row_directive_violation",
        &SmahError::DirectiveViolation {
            tool: "log-analyzer".to_string(),
            detail: "path '/etc/passwd' is outside the tool's allowed roots".to_string(),
        },
        ExitCode::Config,
    );
}

#[test]
fn v_row_directive_protocol() {
    snap(
        "v_row_directive_protocol",
        &SmahError::DirectiveProtocol {
            tool: "log-analyzer".to_string(),
            detail: "line 3: directive JSON did not parse: expected value".to_string(),
        },
        ExitCode::Config,
    );
}

#[test]
fn v_row_directive_loop_overflow() {
    snap(
        "v_row_directive_loop_overflow",
        &SmahError::DirectiveLoopOverflow {
            tool: "log-analyzer".to_string(),
            turns: 8,
        },
        ExitCode::Provider,
    );
}

#[test]
fn v_row_driver_unavailable() {
    // PR-0: the stub drivers' refusal row (Config class, exit 3).
    snap(
        "v_row_driver_unavailable",
        &SmahError::DriverUnavailable {
            driver: "tui".to_string(),
            hint: "not implemented yet; the tui virtual tool ships in a later release".to_string(),
        },
        ExitCode::Config,
    );
}

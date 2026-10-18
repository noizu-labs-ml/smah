//! [`SmahError`] and the exact §4 error-message templates.
//!
//! `docs/ux/interface-spec.md` §4 is the normative text for every user-facing
//! failure. Each row's message lives here once, as a public const template
//! with `{token}` placeholders; the matching format fn fills it. Two
//! render-time rules:
//!
//! 1. **Every failure message ends by naming the log file** (§4 preamble).
//!    Non-internal rows get the [`LOG_SUFFIX`] appended by
//!    [`SmahError::render`]; the internal row already names the file inside
//!    its template.
//! 2. **Internal error exits 1**, not the §4 table's 5 — that row is a
//!    documented erratum (plan correction 1; code and spec footer agree).
//!    [`ExitCode::Internal`] encodes the corrected value.
//!
//! The `Unknown flag / removed flag` §4 row defers to the §2 back-compat
//! table; its exact strings land with the CLI milestone (M5) and surface
//! through [`SmahError::Usage`] (exit 2).

use crate::exit_codes::ExitCode;

/// `'ask'` invoked with no query and no piped stdin (§4 row 1, exit 2).
pub const ASK_NEEDS_QUERY: &str = "smah: 'ask' needs a query argument or piped stdin. Try: smah ask \"...\" or: some-cmd | smah ask \"...\"";

/// Config file unreadable/unparseable (§4 row 3, exit 3).
pub const CONFIG_UNREADABLE: &str =
    "smah: can't read config at {path}: {detail}. Fix it, or move it aside and run 'smah config'.";

/// Config fails the version/schema gate (§4 row 4, exit 3).
pub const CONFIG_GATE: &str = "smah: config at {path} is invalid: {detail} (expected vsn 0.0.1). Run 'smah config' to regenerate.";

/// Session database unreadable or migrations stale (§4 row 5, exit 3).
pub const DATABASE_PROBLEM: &str =
    "smah: session database problem at {path}: {detail}. Try 'smah-db status'.";

/// No API key resolvable for the active provider (§4 row 6, exit 4).
pub const MISSING_API_KEY: &str = "smah: no API key for provider \"{provider}\". Run 'smah config' to add one, or export {PROVIDER}_API_KEY.";

/// Zero enabled models in the catalog (§4 row 7, exit 4). `<suggested-id>` is
/// emitted literally — the suggestion engine is a later milestone.
pub const NO_MODELS_ENABLED: &str =
    "smah: no models enabled. Run 'smah models enable <suggested-id>' or 'smah config'.";

/// Provider rejected the key with 401/403 (§4 row 8, exit 4).
pub const PROVIDER_AUTH: &str = "smah: provider \"{provider}\" rejected the API key (HTTP {code}). Run 'smah config' to update it.";

/// Provider 4xx other than auth (§4 row 9, exit 5).
pub const PROVIDER_REQUEST: &str = "smah: provider \"{provider}\" rejected the request (HTTP {code}): {detail}. Check model ids with 'smah models list'.";

/// Provider 5xx (§4 row 10, exit 5). `<alt>` is emitted literally.
pub const PROVIDER_UNAVAILABLE: &str = "smah: provider \"{provider}\" is failing (HTTP {code}). Try again shortly, or 'smah models use <alt>' to switch.";

/// Network failure / timeout (§4 row 11, exit 5).
pub const NETWORK_FAILURE: &str =
    "smah: can't reach provider \"{provider}\" ({detail}). Check network/DNS, then retry.";

/// Unexpected internal error (§4 row 12 — erratum: exit **1**).
pub const INTERNAL_ERROR: &str =
    "smah: internal error: {detail}. Full traceback in {logfile}. Please file an issue.";

/// Render-time suffix naming the log file, appended to every row whose
/// template does not already embed `{logfile}` (§4: "Every error ends by
/// naming the log file with full detail").
pub const LOG_SUFFIX: &str = " Full log: {logfile}.";

/// Substitute every `{token}` occurrence in a template.
fn fill(template: &str, subs: &[(&str, String)]) -> String {
    let mut out = template.to_string();
    for (token, value) in subs {
        out = out.replace(token, value);
    }
    out
}

/// §4 row 1.
pub fn ask_needs_query() -> String {
    ASK_NEEDS_QUERY.to_string()
}

/// §4 row 3.
pub fn config_unreadable(path: impl AsRef<str>, detail: impl AsRef<str>) -> String {
    fill(
        CONFIG_UNREADABLE,
        &[
            ("{path}", path.as_ref().to_string()),
            ("{detail}", detail.as_ref().to_string()),
        ],
    )
}

/// §4 row 4.
pub fn config_gate(path: impl AsRef<str>, detail: impl AsRef<str>) -> String {
    fill(
        CONFIG_GATE,
        &[
            ("{path}", path.as_ref().to_string()),
            ("{detail}", detail.as_ref().to_string()),
        ],
    )
}

/// §4 row 5.
pub fn database_problem(path: impl AsRef<str>, detail: impl AsRef<str>) -> String {
    fill(
        DATABASE_PROBLEM,
        &[
            ("{path}", path.as_ref().to_string()),
            ("{detail}", detail.as_ref().to_string()),
        ],
    )
}

/// §4 row 6. `{PROVIDER}` is the provider id uppercased for the env-var name.
pub fn missing_api_key(provider: impl AsRef<str>) -> String {
    fill(
        MISSING_API_KEY,
        &[
            ("{provider}", provider.as_ref().to_string()),
            ("{PROVIDER}", provider.as_ref().to_uppercase()),
        ],
    )
}

/// §4 row 7.
pub fn no_models_enabled() -> String {
    NO_MODELS_ENABLED.to_string()
}

/// §4 row 8.
pub fn provider_auth(provider: impl AsRef<str>, code: u16) -> String {
    fill(
        PROVIDER_AUTH,
        &[
            ("{provider}", provider.as_ref().to_string()),
            ("{code}", code.to_string()),
        ],
    )
}

/// §4 row 9.
pub fn provider_request(provider: impl AsRef<str>, code: u16, detail: impl AsRef<str>) -> String {
    fill(
        PROVIDER_REQUEST,
        &[
            ("{provider}", provider.as_ref().to_string()),
            ("{code}", code.to_string()),
            ("{detail}", detail.as_ref().to_string()),
        ],
    )
}

/// §4 row 10.
pub fn provider_unavailable(provider: impl AsRef<str>, code: u16) -> String {
    fill(
        PROVIDER_UNAVAILABLE,
        &[
            ("{provider}", provider.as_ref().to_string()),
            ("{code}", code.to_string()),
        ],
    )
}

/// §4 row 11.
pub fn network_failure(provider: impl AsRef<str>, detail: impl AsRef<str>) -> String {
    fill(
        NETWORK_FAILURE,
        &[
            ("{provider}", provider.as_ref().to_string()),
            ("{detail}", detail.as_ref().to_string()),
        ],
    )
}

/// §4 row 12 (exit-1 erratum).
pub fn internal_error(detail: impl AsRef<str>, logfile: impl AsRef<str>) -> String {
    fill(
        INTERNAL_ERROR,
        &[
            ("{detail}", detail.as_ref().to_string()),
            ("{logfile}", logfile.as_ref().to_string()),
        ],
    )
}

/// Every user-facing failure the CLI can terminate with.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SmahError {
    /// §4 row 1 — 'ask' with no query and no stdin.
    #[error("{}", ask_needs_query())]
    AskNeedsQuery,
    /// §4 row 2 — unknown/removed flag or other usage error; exact per-row
    /// text comes from the §2 table (M5) and is supplied by the caller.
    #[error("{}", .message)]
    Usage {
        /// Exact usage-line text (§2 row content).
        message: String,
    },
    /// §4 row 3 — config unreadable/invalid YAML.
    #[error("{}", config_unreadable(.path, .detail))]
    ConfigUnreadable {
        /// Resolved config path.
        path: String,
        /// Underlying I/O/parse message, one line.
        detail: String,
    },
    /// §4 row 4 — version/schema gate rejection.
    #[error("{}", config_gate(.path, .detail))]
    ConfigGate {
        /// Resolved config path.
        path: String,
        /// Why the config was rejected.
        detail: String,
    },
    /// §4 row 5 — session database problem.
    #[error("{}", database_problem(.path, .detail))]
    Database {
        /// Resolved database path.
        path: String,
        /// Underlying message, one line.
        detail: String,
    },
    /// §4 row 6 — missing API key.
    #[error("{}", missing_api_key(.provider))]
    MissingApiKey {
        /// Active provider id.
        provider: String,
    },
    /// §4 row 7 — zero models enabled.
    #[error("{}", no_models_enabled())]
    NoModelsEnabled,
    /// §4 row 8 — provider 401/403.
    #[error("{}", provider_auth(.provider, *.code))]
    ProviderAuth {
        /// Active provider id.
        provider: String,
        /// HTTP status code.
        code: u16,
    },
    /// §4 row 9 — provider 4xx, non-auth.
    #[error("{}", provider_request(.provider, *.code, .detail))]
    ProviderRequest {
        /// Active provider id.
        provider: String,
        /// HTTP status code.
        code: u16,
        /// Provider message, one line.
        detail: String,
    },
    /// §4 row 10 — provider 5xx.
    #[error("{}", provider_unavailable(.provider, *.code))]
    ProviderUnavailable {
        /// Active provider id.
        provider: String,
        /// HTTP status code.
        code: u16,
    },
    /// §4 row 11 — network failure / timeout.
    #[error("{}", network_failure(.provider, .detail))]
    Network {
        /// Active provider id.
        provider: String,
        /// Transport error message, one line.
        detail: String,
    },
    /// §4 row 12 — unexpected internal error (exit 1 per erratum). Display
    /// uses the `<logfile>` placeholder; [`SmahError::render`] fills the real
    /// path.
    #[error("{}", internal_error(.detail, LOG_PLACEHOLDER))]
    Internal {
        /// Internal message, one line.
        detail: String,
    },
}

/// Placeholder used only for `Display` on [`SmahError::Internal`]; the real
/// log path is supplied at render time by [`SmahError::render`].
const LOG_PLACEHOLDER: &str = "<logfile>";

impl SmahError {
    /// Exit code class for this failure (erratum encoded: internal → 1).
    pub fn exit_code(&self) -> ExitCode {
        match self {
            SmahError::AskNeedsQuery | SmahError::Usage { .. } => ExitCode::Usage,
            SmahError::ConfigUnreadable { .. }
            | SmahError::ConfigGate { .. }
            | SmahError::Database { .. } => ExitCode::Config,
            SmahError::MissingApiKey { .. }
            | SmahError::NoModelsEnabled
            | SmahError::ProviderAuth { .. } => ExitCode::Auth,
            SmahError::ProviderRequest { .. }
            | SmahError::ProviderUnavailable { .. }
            | SmahError::Network { .. } => ExitCode::Provider,
            SmahError::Internal { .. } => ExitCode::Internal,
        }
    }

    /// The single stderr line a user sees: the §4 row text, ending by naming
    /// the log file. Non-internal rows get [`LOG_SUFFIX`] appended; the
    /// internal row already names it inside the template.
    pub fn render(&self, logfile: &str) -> String {
        match self {
            SmahError::Internal { detail } => internal_error(detail, logfile),
            other => format!(
                "{}{}",
                other,
                fill(LOG_SUFFIX, &[("{logfile}", logfile.to_string())])
            ),
        }
    }
}

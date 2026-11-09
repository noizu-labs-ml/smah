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

// --- Virtual tools — V-series extension (ticket #17) -----------------------
//
// The §4 table stays untouched for M5 parity; the CLI virtual-tool driver
// extends it with five rows documented in docs/ux/virtual-tools-cli-driver.md
// §6. Classes reuse the fixed six: no seventh exit code — a virtual-tool file
// is user config, so invalid files and file-surface violations land in the
// Config class (3), a missing tool is a Usage error (2), and a model that
// never converges is the provider's problem (5). PR-0 adds a sixth row for
// the driver family (`DRIVER_UNAVAILABLE`, also Config): a valid tool file
// whose declared driver has no implementation in this build.

/// No virtual tool by that name (V-series, exit 2).
pub const TOOL_NOT_FOUND: &str =
    "smah: no virtual tool named '{name}'. Installed: {installed}. Run 'smah tools list'.";

/// A virtual-tool file failed validation, or its declared file surface is
/// unusable at invoke time (V-series, exit 3).
pub const TOOL_FILE_INVALID: &str =
    "smah: virtual-tool file at {path} is invalid: {detail}. Fix or remove the file; 'smah tools list' shows the reason.";

/// A well-formed directive the driver refuses: outside the tool's allow
/// roots, traversal, symlink on write, or aggregate write cap (V-series, exit 3).
pub const DIRECTIVE_VIOLATION: &str = "smah: tool '{tool}' attempted a forbidden file operation: {detail}. Run aborted; the audit log has the attempt.";

/// The model broke the directive line protocol (sentinel line that does not
/// parse, oversized directive) (V-series, exit 3).
pub const DIRECTIVE_PROTOCOL: &str =
    "smah: tool '{tool}' sent a malformed virtual-tool directive ({detail}). Run aborted.";

/// The model kept requesting file operations past the turn limit (V-series, exit 5).
pub const DIRECTIVE_LOOP_OVERFLOW: &str = "smah: tool '{tool}' did not finish within {turns} turns of file operations. Try smaller input; the audit log shows what it attempted.";

/// The tool file declares a driver this build cannot serve (V-series, exit 3
/// — Config class: the file is valid config naming a missing implementation).
/// `{driver}` is the canonical driver name; `{hint}` is the driver module's
/// one-line status.
pub const DRIVER_UNAVAILABLE: &str = "smah: virtual-tool driver '{driver}' is not available in this build ({hint}). See docs/ux/virtual-tools.md for the driver family.";

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

/// V-series: no such virtual tool. `installed` is a comma list or "(none)".
pub fn tool_not_found(name: impl AsRef<str>, installed: impl AsRef<str>) -> String {
    fill(
        TOOL_NOT_FOUND,
        &[
            ("{name}", name.as_ref().to_string()),
            ("{installed}", installed.as_ref().to_string()),
        ],
    )
}

/// V-series: virtual-tool file invalid (schema, name, or file surface).
pub fn tool_file_invalid(path: impl AsRef<str>, detail: impl AsRef<str>) -> String {
    fill(
        TOOL_FILE_INVALID,
        &[
            ("{path}", path.as_ref().to_string()),
            ("{detail}", detail.as_ref().to_string()),
        ],
    )
}

/// V-series: directive refused by the file-surface executor.
pub fn directive_violation(tool: impl AsRef<str>, detail: impl AsRef<str>) -> String {
    fill(
        DIRECTIVE_VIOLATION,
        &[
            ("{tool}", tool.as_ref().to_string()),
            ("{detail}", detail.as_ref().to_string()),
        ],
    )
}

/// V-series: directive line-protocol abuse.
pub fn directive_protocol(tool: impl AsRef<str>, detail: impl AsRef<str>) -> String {
    fill(
        DIRECTIVE_PROTOCOL,
        &[
            ("{tool}", tool.as_ref().to_string()),
            ("{detail}", detail.as_ref().to_string()),
        ],
    )
}

/// V-series: the model never converged within the turn limit.
pub fn directive_loop_overflow(tool: impl AsRef<str>, turns: usize) -> String {
    fill(
        DIRECTIVE_LOOP_OVERFLOW,
        &[
            ("{tool}", tool.as_ref().to_string()),
            ("{turns}", turns.to_string()),
        ],
    )
}

/// V-series: the tool's declared driver has no implementation in this build.
pub fn driver_unavailable(driver: impl AsRef<str>, hint: impl AsRef<str>) -> String {
    fill(
        DRIVER_UNAVAILABLE,
        &[
            ("{driver}", driver.as_ref().to_string()),
            ("{hint}", hint.as_ref().to_string()),
        ],
    )
}

// --- Script wrappers — W-series (epic.script-wrapper, SW-02) ---------------
//
// Same reuse discipline as the V-series: no seventh exit code. A missing
// wrapper or a missing/typo'd param is Usage (2); an invalid or
// unresolvable wrapper file is Config (3).

/// No wrapper by that name (W-series, exit 2).
pub const WRAPPER_NOT_FOUND: &str =
    "smah: no wrapper named '{name}'. Installed: {installed}. Run 'smah wrapper list'.";

/// A wrapper file failed validation (W-series, exit 3).
pub const WRAPPER_FILE_INVALID: &str =
    "smah: wrapper file at {path} is invalid: {detail}. Fix or remove the file; 'smah wrapper list' shows the reason.";

/// A required param was not supplied at invocation (W-series, exit 2).
pub const WRAPPER_PARAM_MISSING: &str =
    "smah: wrapper '{name}' is missing required param(s): {params}. Pass --param <name>=<value>.";

/// A param was supplied that the wrapper does not declare (W-series, exit 2
/// — a typo'd `--param` silently doing nothing is a footgun).
pub const WRAPPER_PARAM_UNKNOWN: &str =
    "smah: --param '{param}' is not declared by wrapper '{wrapper}' (declared: {declared}).";

/// The wrapper loaded but its steps did not resolve (unknown built-in, bad
/// binding value at invocation; W-series, exit 3).
pub const WRAPPER_RESOLVE: &str = "smah: wrapper '{name}' did not resolve: {detail}.";

/// The `--prompt` composition failed the model contract (SW-03, W-series,
/// exit 5 — the answer was not a valid changes object, or the change it
/// describes cannot produce a loadable recipe; the wrapper is untouched).
pub const WRAPPER_PROMPT_INVALID: &str =
    "smah: --prompt for wrapper '{wrapper}' produced an unusable change: {detail}. Nothing was written; rephrase the request or edit the file by hand ('smah wrapper show {wrapper}').";

/// W-series: no wrapper by that name.
pub fn wrapper_not_found(name: impl AsRef<str>, installed: impl AsRef<str>) -> String {
    fill(
        WRAPPER_NOT_FOUND,
        &[
            ("{name}", name.as_ref().to_string()),
            ("{installed}", installed.as_ref().to_string()),
        ],
    )
}

/// W-series: wrapper file failed validation.
pub fn wrapper_file_invalid(path: impl AsRef<str>, detail: impl AsRef<str>) -> String {
    fill(
        WRAPPER_FILE_INVALID,
        &[
            ("{path}", path.as_ref().to_string()),
            ("{detail}", detail.as_ref().to_string()),
        ],
    )
}

/// W-series: missing required params.
pub fn wrapper_param_missing(name: impl AsRef<str>, params: impl AsRef<str>) -> String {
    fill(
        WRAPPER_PARAM_MISSING,
        &[
            ("{name}", name.as_ref().to_string()),
            ("{params}", params.as_ref().to_string()),
        ],
    )
}

/// W-series: supplied param is not declared.
pub fn wrapper_param_unknown(
    param: impl AsRef<str>,
    wrapper: impl AsRef<str>,
    declared: impl AsRef<str>,
) -> String {
    fill(
        WRAPPER_PARAM_UNKNOWN,
        &[
            ("{param}", param.as_ref().to_string()),
            ("{wrapper}", wrapper.as_ref().to_string()),
            ("{declared}", declared.as_ref().to_string()),
        ],
    )
}

/// W-series: steps did not resolve.
pub fn wrapper_resolve(name: impl AsRef<str>, detail: impl AsRef<str>) -> String {
    fill(
        WRAPPER_RESOLVE,
        &[
            ("{name}", name.as_ref().to_string()),
            ("{detail}", detail.as_ref().to_string()),
        ],
    )
}

/// W-series (SW-03): `--prompt` composition unusable.
pub fn wrapper_prompt_invalid(wrapper: impl AsRef<str>, detail: impl AsRef<str>) -> String {
    fill(
        WRAPPER_PROMPT_INVALID,
        &[
            ("{wrapper}", wrapper.as_ref().to_string()),
            ("{detail}", detail.as_ref().to_string()),
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
    /// V-series (#17) — no virtual tool by that name (exit 2, Usage).
    #[error("{}", tool_not_found(.name, .installed))]
    ToolNotFound {
        /// The requested tool name.
        name: String,
        /// Installed tool names, comma-joined, or "(none)".
        installed: String,
    },
    /// V-series (#17) — virtual-tool file invalid or its file surface
    /// unusable at invoke time (exit 3, Config).
    #[error("{}", tool_file_invalid(.path, .detail))]
    ToolFileInvalid {
        /// The tool file's path.
        path: String,
        /// Why it was rejected, one line.
        detail: String,
    },
    /// V-series (#17) — well-formed directive the executor refuses (exit 3,
    /// Config). Aborts the run; later directives are not executed.
    #[error("{}", directive_violation(.tool, .detail))]
    DirectiveViolation {
        /// Tool name.
        tool: String,
        /// Path + reason, one line.
        detail: String,
    },
    /// V-series (#17) — directive line-protocol abuse: a sentinel line that
    /// does not parse as `@@smah <verb> <json>` (exit 3, Config).
    #[error("{}", directive_protocol(.tool, .detail))]
    DirectiveProtocol {
        /// Tool name.
        tool: String,
        /// What was wrong with the line, one line.
        detail: String,
    },
    /// V-series (#17) — model never converged within `TURNS_MAX` (exit 5,
    /// Provider: model-side non-convergence).
    #[error("{}", directive_loop_overflow(.tool, *.turns))]
    DirectiveLoopOverflow {
        /// Tool name.
        tool: String,
        /// The turn limit that was hit.
        turns: usize,
    },
    /// V-series (driver family, PR-0) — the tool file declares a driver this
    /// build cannot serve (exit 3, Config: the file is valid but names a
    /// missing implementation). A stub driver always answers with this.
    #[error("{}", driver_unavailable(.driver, .hint))]
    DriverUnavailable {
        /// Canonical driver name (`tui`, `web`, ... — never the `"emulated"`
        /// alias, which loads as `"cli"`).
        driver: String,
        /// One-line status (stub: "not implemented yet; ...").
        hint: String,
    },
    /// W-series (SW-02) — no wrapper by that name (exit 2, Usage).
    #[error("{}", wrapper_not_found(.name, .installed))]
    WrapperNotFound {
        /// The requested wrapper name.
        name: String,
        /// Installed wrapper names, comma-joined, or "(none)".
        installed: String,
    },
    /// W-series (SW-02) — wrapper file invalid (exit 3, Config).
    #[error("{}", wrapper_file_invalid(.path, .detail))]
    WrapperFileInvalid {
        /// The wrapper file's path.
        path: String,
        /// Why it was rejected, one line.
        detail: String,
    },
    /// W-series (SW-02) — required params missing at invocation (exit 2).
    #[error("{}", wrapper_param_missing(.name, .params))]
    WrapperParamMissing {
        /// Wrapper name.
        name: String,
        /// Missing param names, comma-joined.
        params: String,
    },
    /// W-series (SW-02) — supplied param not declared (exit 2).
    #[error("{}", wrapper_param_unknown(.param, .wrapper, .declared))]
    WrapperParamUnknown {
        /// The supplied param name.
        param: String,
        /// Wrapper name.
        wrapper: String,
        /// Declared param names, comma-joined, or "(none)".
        declared: String,
    },
    /// W-series (SW-02) — the wrapper loaded but did not resolve (exit 3).
    #[error("{}", wrapper_resolve(.name, .detail))]
    WrapperResolve {
        /// Wrapper name.
        name: String,
        /// Why resolution failed, one line.
        detail: String,
    },
    /// W-series (SW-03) — `--prompt` composition unusable (exit 5: the
    /// model failed the contract, not the user's config).
    #[error("{}", wrapper_prompt_invalid(.wrapper, .detail))]
    WrapperPromptInvalid {
        /// Wrapper name.
        wrapper: String,
        /// Why the change is unusable, one line.
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
            SmahError::AskNeedsQuery
            | SmahError::Usage { .. }
            | SmahError::ToolNotFound { .. }
            | SmahError::WrapperNotFound { .. }
            | SmahError::WrapperParamMissing { .. }
            | SmahError::WrapperParamUnknown { .. } => ExitCode::Usage,
            SmahError::ConfigUnreadable { .. }
            | SmahError::ConfigGate { .. }
            | SmahError::Database { .. }
            | SmahError::ToolFileInvalid { .. }
            | SmahError::DirectiveViolation { .. }
            | SmahError::DirectiveProtocol { .. }
            | SmahError::DriverUnavailable { .. }
            | SmahError::WrapperFileInvalid { .. }
            | SmahError::WrapperResolve { .. } => ExitCode::Config,
            SmahError::MissingApiKey { .. }
            | SmahError::NoModelsEnabled
            | SmahError::ProviderAuth { .. } => ExitCode::Auth,
            SmahError::ProviderRequest { .. }
            | SmahError::ProviderUnavailable { .. }
            | SmahError::Network { .. }
            | SmahError::DirectiveLoopOverflow { .. }
            | SmahError::WrapperPromptInvalid { .. } => ExitCode::Provider,
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

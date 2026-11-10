//! The clap command tree (design decision D6). This PR seeds the M5 tree:
//! global flags (`-c`, `--model`, `-v`), the `--emulate-cli` flag path with
//! raw trailing tool args, and the `tools`/`hooks` subcommands. M5 grows the
//! subcommand enum; the global flags stay here.
//!
//! Argument hazard rule: everything after `<tool>` belongs to the tool —
//! hyphen-leading values included — so `smah --emulate-cli edit --version`
//! passes `--version` through to the tool prompt and never prints clap's
//! help/version. Global smah flags must precede the emulate flag.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// `smah` — the SMAH command-line entry point.
#[derive(Parser, Debug)]
#[command(
    name = "smah",
    version,
    about = "SMAH — a local, print-only AI CLI. This build ships virtual tools (ticket #17).",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Config file path (§1 global).
    #[arg(short = 'c', long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Model override (§1 global, fully qualified `provider.model-id` or bare
    /// id). Precedence over the tool file's frontmatter model.
    #[arg(long, value_name = "ID")]
    pub model: Option<String>,

    /// Verbosity (repeatable).
    #[arg(short = 'v', action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Run an installed virtual tool through the emulated-CLI driver:
    /// `smah --emulate-cli <tool> [args...]`. Kept forever as the forced-cli
    /// compat form.
    #[arg(long, value_name = "TOOL")]
    pub emulate_cli: Option<String>,

    /// Run an installed virtual tool, dispatching on the tool file's
    /// `driver:` field: `smah --emulate <tool> [args...]`. A `driver: cli`
    /// or `emulated` tool takes the emulated-CLI path (identical to
    /// `--emulate-cli`); other drivers resolve to their own driver
    /// implementations.
    #[arg(long, value_name = "TOOL")]
    pub emulate: Option<String>,

    /// Run an installed script wrapper deterministically (no inference):
    /// `smah --wrapper <name> [--param k=v]...` prints the resolved, ordered
    /// plan and the exact `smah-run` invocation — nothing executes here
    /// (smah is print-only; execution lives in the smah-run crate).
    #[arg(long, value_name = "NAME")]
    pub wrapper: Option<String>,

    /// Wrapper invocation param, repeatable: `--param <name>=<value>`.
    /// Values starting with `-` need the inline form
    /// `--param=<name>=<value>`.
    #[arg(long = "param", value_name = "NAME=VALUE", action = clap::ArgAction::Append)]
    pub param: Vec<String>,

    /// Compose a recipe change with inference (SW-03):
    /// `smah --wrapper <name> --prompt "<natural language>"` resolves the
    /// model's structured change object into a reviewable diff and writes
    /// nothing until you accept (`--accept`, or the interactive prompt).
    #[arg(long, value_name = "TEXT")]
    pub prompt: Option<String>,

    /// Accept the `--prompt` diff non-interactively (applies the change,
    /// stamps provenance + approval).
    #[arg(long)]
    pub accept: bool,

    /// Reject the `--prompt` diff non-interactively (nothing is written).
    #[arg(long)]
    pub reject: bool,

    /// Disable the file surface for this run (all directives become soft
    /// refusals back to the model).
    #[arg(long)]
    pub no_fs: bool,

    /// Piped-stdin cap in bytes (default 262144). Over-cap input is
    /// truncated — loudly (stderr notice + the model is told).
    #[arg(long, value_name = "N")]
    pub max_input_chars: Option<usize>,

    /// Subcommands (M5 grows this enum).
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Unused on the emulate path (see [`split_emulate_tail`]): clap's own
    /// `--version`/`--help` actions fire before a trailing positional can
    /// start when `<tool>` was consumed as an emulate-flag value, so the
    /// plan's documented fallback splits argv manually after `<tool>`. Kept
    /// for M5's plain-positional forms.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub tool_args: Vec<String>,
}

/// The argv split that guarantees the tool-data tail (plan risk-2 fallback,
/// same observable contract as `trailing_var_arg`): find the first emulate
/// flag — `--emulate-cli` or `--emulate` (inline `=` forms included) — and
/// treat everything **after** its value as tool data — hyphens included, so
/// `--version`/`--help` are never intercepted. Global flags (before the
/// emulate flag) go through clap normally, so clap still owns `-c`,
/// `--model`, `-v`, its own `--version`, and usage errors (exit 2).
///
/// Returns `None` when neither flag is present (subcommand / bare paths
/// parse the whole argv through clap).
pub fn split_emulate_tail() -> Option<(Cli, String, Vec<String>)> {
    let argv: Vec<String> = std::env::args().collect();
    for (idx, arg) in argv.iter().enumerate() {
        for flag in ["--emulate-cli", "--emulate"] {
            let inline = format!("{flag}=");
            if let Some(tool) = arg.strip_prefix(inline.as_str()) {
                let head: Vec<String> = argv[..=idx].to_vec();
                let tail: Vec<String> = argv[idx + 1..].to_vec();
                return Some((Cli::parse_from(head), tool.to_string(), tail));
            }
            if arg == flag {
                let tool = argv.get(idx + 1)?.clone();
                let mut head: Vec<String> = argv[..=idx].to_vec();
                head.push(tool.clone());
                let tail: Vec<String> = argv[idx + 2..].to_vec();
                return Some((Cli::parse_from(head), tool, tail));
            }
        }
    }
    None
}

/// Subcommands seeded by the virtual-tools milestone.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// One-shot question answering (RA-01): `smah ask "<question>"`.
    Ask(AskArgs),
    /// Virtual-tool discovery.
    Tools {
        /// `smah tools list` — the only tools verb in this build.
        #[command(subcommand)]
        cmd: ToolsCommand,
    },
    /// Configuration (`smah config init` — RA-06).
    Config {
        /// `smah config <verb>`.
        #[command(subcommand)]
        cmd: ConfigSubCommand,
    },
    /// Shell-hook registration for installed tools.
    Hooks {
        /// `smah hooks <target>`.
        #[command(subcommand)]
        cmd: HooksCommand,
    },
    /// Provider/config health check (RA-07): no network calls, no key
    /// material shown — presence only.
    Doctor(DoctorArgs),
    /// Interactive chat over a persisted session (RA-03): the standing
    /// keys — [c]opy [e]dit [r]efine [n]ext [q]uit.
    Chat {
        /// The opening question.
        #[arg(trailing_var_arg = true, required = true)]
        query: Vec<String>,
    },
    /// Resume a saved session (RA-03): `--last` or a session id.
    Resume {
        /// Resume the most recently modified session.
        #[arg(long)]
        last: bool,
        /// Session id to resume.
        #[arg(value_name = "ID")]
        id: Option<i64>,
    },
    /// Script-wrapper discovery and deterministic planning (print-only).
    Wrapper {
        /// `smah wrapper <verb>`.
        #[command(subcommand)]
        cmd: WrapperCommand,
    },
}

/// `smah doctor` flags.
#[derive(clap::Args, Debug)]
pub struct DoctorArgs {
    /// Emit machine-readable JSON on stdout instead of the human report.
    #[arg(long)]
    pub json: bool,
}

/// `smah config ...` verbs.
#[derive(Subcommand, Debug)]
pub enum ConfigSubCommand {
    /// Guided first-run setup: provider + API key (masked), profile,
    /// data-egress toggles. Non-interactive (or non-TTY) uses flags/env.
    /// The key is stored in a secret store (OS keychain / credential
    /// manager / Secret Service, a 0600 secrets file, or an env reference);
    /// config.yaml keeps a reference only.
    Init(ConfigInitArgs),
    /// Move literal API keys out of config.yaml into the secret store
    /// (keychain or the 0600 secrets file), rewriting the config to hold
    /// references. `smah doctor` names this when it finds a literal.
    MigrateSecrets(ConfigMigrateArgs),
}

/// `smah config migrate-secrets` arguments (RA-10 F2).
#[derive(clap::Args, Debug)]
pub struct ConfigMigrateArgs {
    /// Destination store: keychain (OS store) or file (0600 secrets.yaml
    /// beside the config). Default: best available.
    #[arg(long, value_name = "STORE")]
    pub secret_store: Option<String>,

    /// Config override (same as the global -c).
    #[arg(long, value_name = "PATH")]
    pub config_path: Option<PathBuf>,
}

/// `smah config init` arguments (RA-06).
#[derive(clap::Args, Debug)]
pub struct ConfigInitArgs {
    /// Provider to configure (default anthropic; one of the registry keys).
    #[arg(long, value_name = "KEY")]
    pub provider: Option<String>,

    /// Store `$VAR` (env reference) as the api key instead of a literal —
    /// headless setups; VAR must be set at run time.
    #[arg(long, value_name = "VAR")]
    pub key_env: Option<String>,

    /// Where the API key lives: `keychain` (OS keychain / credential
    /// manager / Secret Service), `file` (0600 secrets.yaml beside the
    /// config), or `env` (a reference; pair with --key-env). Default: best
    /// available — the OS store, falling back to the file when none is
    /// reachable (headless Linux without a D-Bus session bus).
    #[arg(long, value_name = "STORE")]
    pub secret_store: Option<String>,

    /// No prompts: flags/env only (implied when stdin is not a TTY).
    #[arg(long)]
    pub non_interactive: bool,

    /// Overwrite an existing config.
    #[arg(long)]
    pub force: bool,

    /// Profile: your name.
    #[arg(long)]
    pub name: Option<String>,

    /// Profile: your role.
    #[arg(long)]
    pub role: Option<String>,

    /// Profile: Linux/ops experience.
    #[arg(long)]
    pub experience: Option<String>,

    /// Profile: anything else the model should know.
    #[arg(long)]
    pub about: Option<String>,

    /// Egress opt-out: do not send the profile block.
    #[arg(long)]
    pub no_share_profile: bool,

    /// Egress opt-out: do not send the system-details block.
    #[arg(long)]
    pub no_share_stats: bool,

    /// Write target override (same as the global -c; kept local so
    /// `smah config init --config X` reads naturally).
    #[arg(long, value_name = "PATH")]
    pub config_path: Option<PathBuf>,
}

/// `smah ask` arguments (RA-01).
#[derive(clap::Args, Debug)]
pub struct AskArgs {
    /// The question (quoted, or bare words). Piped stdin is attached to the
    /// model as data — it still needs a question.
    #[arg(trailing_var_arg = true, required = true)]
    pub query: Vec<String>,

    /// Additional operator instructions for this ask (a text file).
    #[arg(short = 'i', long, value_name = "PATH")]
    pub instructions: Option<PathBuf>,

    /// Emit machine-readable JSON on stdout instead of streamed prose +
    /// blocks (place before the question). Diagnostics stay on stderr;
    /// exit codes are unchanged.
    #[arg(long)]
    pub json: bool,

    /// Force a clipboard copy attempt even if config disabled it
    /// (single-command answers only).
    #[arg(long)]
    pub copy: bool,

    /// Never touch the clipboard this run.
    #[arg(long)]
    pub no_copy: bool,
}

/// `smah tools ...`
#[derive(Subcommand, Debug)]
pub enum ToolsCommand {
    /// List installed tools with validity status.
    List {
        /// Emit machine-readable JSON on stdout.
        #[arg(long)]
        json: bool,
    },
}

/// `smah hooks ...`
#[derive(Subcommand, Debug)]
pub enum HooksCommand {
    /// Emit a zsh function snippet (`eval "$(smah hooks zsh)"`).
    Zsh,
    /// Emit a bash function snippet (`eval "$(smah hooks bash)"`).
    Bash,
    /// Emit a direnv snippet (shims via `PATH_add`; direnv cannot export
    /// shell functions).
    Envrc,
    /// Write/refresh `smah.<name>` shim executables and remove stale ones.
    Shims {
        /// Target bin dir (default `~/.local/state/smah/bin`).
        #[arg(long, value_name = "DIR")]
        dir: Option<PathBuf>,
    },
}

/// `smah wrapper ...` (SW-02).
#[derive(Subcommand, Debug)]
pub enum WrapperCommand {
    /// List installed wrappers with validity status.
    List {
        /// Emit machine-readable JSON on stdout.
        #[arg(long)]
        json: bool,
    },
    /// Print a wrapper's definition file verbatim.
    Show {
        /// Wrapper name.
        name: String,
    },
    /// Load + statically resolve a wrapper (no params supplied).
    Validate {
        /// Wrapper name.
        name: String,
    },
    /// Resolve + print the deterministic plan for a wrapper (print-only).
    Run {
        /// Wrapper name.
        name: String,
        /// Invocation param, repeatable: `--param <name>=<value>`.
        #[arg(long = "param", value_name = "NAME=VALUE", action = clap::ArgAction::Append)]
        param: Vec<String>,
    },
}

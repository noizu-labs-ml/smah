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
    /// Virtual-tool discovery.
    Tools {
        /// `smah tools list` — the only tools verb in this build.
        #[command(subcommand)]
        cmd: ToolsCommand,
    },
    /// Shell-hook registration for installed tools.
    Hooks {
        /// `smah hooks <target>`.
        #[command(subcommand)]
        cmd: HooksCommand,
    },
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

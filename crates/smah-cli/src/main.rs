//! 〚📁:49558d29-c6e4-5c91-a3bb-477741745b5c〛 smah-cli :: SMAH CLI binary — clap command tree, dispatch, and the smah/smah-db entrypoints.
//! `smah` binary entry point — dispatch for the virtual-tools build
//! (ticket #17). The binary edge owns the process exit: every other layer
//! returns values, the six-class table (interface-spec §4, plus the
//! V-series extension) maps onto exit codes, and `main` terminates with the
//! matching code.

mod args;
mod backend;
mod emulate;
mod hooks;
mod tools_list;
mod wrapper_cmd;

use args::{split_emulate_tail, Cli, Command};
use clap::Parser;

fn main() {
    // argv[0] postfix dispatch: `smah-<name>` (symlink) where <name> is an
    // installed wrapper re-dispatches to the wrapper path (SW-02). Returns
    // None for every other argv[0] — normal parsing owns those.
    if let Some(code) = wrapper_cmd::run_argv0_dispatch() {
        std::process::exit(code);
    }

    // The emulate path takes a manual argv split (see [`split_emulate_tail`])
    // so the tool tail — `--version`, `--help`, anything — reaches the tool.
    if let Some((cli, tool, tail)) = split_emulate_tail() {
        std::process::exit(emulate::run_emulate(&cli, &tool, &tail));
    }

    let cli = Cli::parse();
    // The deterministic wrapper flag path (subcommands come after so
    // `smah --wrapper x` works without a subcommand).
    if let Some(name) = &cli.wrapper {
        std::process::exit(wrapper_cmd::run_flag(name, &cli.param));
    }
    // Subcommands otherwise; bare `smah` gets the honest not-yet notice
    // (exit 2) — never a fake of the M5 surface.
    let code = match &cli.command {
        Some(Command::Tools { cmd }) => tools_list::run(cmd),
        Some(Command::Hooks { cmd }) => hooks::run(cmd),
        Some(Command::Wrapper { cmd }) => wrapper_cmd::run_sub(cmd),
        None => {
            eprintln!(
                "smah: the full command surface (ask, chat, history) is not in this build yet (M5). \
Available now: smah --emulate-cli <tool> [args], smah --wrapper <name> [--param k=v], smah tools list, \
smah wrapper list|show|validate, smah hooks zsh|bash|envrc|shims."
            );
            smah_core::ExitCode::Usage.code()
        }
    };
    std::process::exit(code);
}

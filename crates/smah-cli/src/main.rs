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

use args::{split_emulate_tail, Cli, Command};
use clap::Parser;

fn main() {
    // The emulate path takes a manual argv split (see [`split_emulate_tail`])
    // so the tool tail — `--version`, `--help`, anything — reaches the tool.
    if let Some((cli, tool, tail)) = split_emulate_tail() {
        std::process::exit(emulate::run_emulate(&cli, &tool, &tail));
    }

    let cli = Cli::parse();
    // Subcommands otherwise; bare `smah` gets the honest not-yet notice
    // (exit 2) — never a fake of the M5 surface.
    let code = match &cli.command {
        Some(Command::Tools { cmd }) => tools_list::run(cmd),
        Some(Command::Hooks { cmd }) => hooks::run(cmd),
        None => {
            eprintln!(
                "smah: the full command surface (ask, chat, history) is not in this build yet (M5). \
Available now: smah --emulate-cli <tool> [args], smah tools list, smah hooks zsh|bash|envrc|shims."
            );
            smah_core::ExitCode::Usage.code()
        }
    };
    std::process::exit(code);
}

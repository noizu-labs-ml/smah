//! 〚📁:49558d29-c6e4-5c91-a3bb-477741745b5c〛 smah-cli :: SMAH CLI binary — clap command tree, dispatch, and the smah/smah-db entrypoints.
//! `smah` binary entry point — dispatch for the virtual-tools build
//! (ticket #17). The binary edge owns the process exit: every other layer
//! returns values, the six-class table (interface-spec §4, plus the
//! V-series extension) maps onto exit codes, and `main` terminates with the
//! matching code.

mod args;
mod ask_cmd;
mod backend;
mod chat_cmd;
mod config_cmd;
mod doctor_cmd;
mod emulate;
mod hooks;
mod tools_list;

use args::{split_emulate_tail, Cli, Command};
use clap::{CommandFactory, Parser};
use std::io::IsTerminal;

fn main() {
    // The emulate path takes a manual argv split (see [`split_emulate_tail`])
    // so the tool tail — `--version`, `--help`, anything — reaches the tool.
    if let Some((cli, tool, tail)) = split_emulate_tail() {
        std::process::exit(emulate::run_emulate(&cli, &tool, &tail));
    }

    let cli = Cli::parse();
    // Subcommands otherwise. Bare `smah` shows help (exit 0) — on a TTY
    // with no config it first offers `smah config init` (RA-06).
    let code = match &cli.command {
        Some(Command::Ask(ask)) => ask_cmd::run_ask(&cli, ask),
        Some(Command::Config { cmd }) => config_cmd::run_config(cli.config.as_deref(), cmd),
        Some(Command::Doctor(args)) => doctor_cmd::run_doctor(args, cli.config.as_deref()),
        Some(Command::Chat { query }) => chat_cmd::run_chat(&cli, query),
        Some(Command::Resume { last, id }) => chat_cmd::run_resume(&cli, *last, *id),
        Some(Command::Tools { cmd }) => tools_list::run(cmd),
        Some(Command::Hooks { cmd }) => hooks::run(cmd),
        None => {
            let config_exists = cli
                .config
                .as_ref()
                .map(|p| p.exists())
                .unwrap_or_else(|| smah_core::settings::Settings::default_path().exists());
            if std::io::stdin().is_terminal() && !config_exists {
                eprintln!(
                    "smah: no config found — run `smah config init` to set up a provider, key, and profile."
                );
            }
            let _ = Cli::command().print_help();
            0
        }
    };
    std::process::exit(code);
}

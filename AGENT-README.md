# AGENT-README

> Orientation for AI coding agents working in this repo. Human docs: README.md.

## Type

rust-cli+python

## Purpose

SMAH — the SMart As Hell SysteM Admin Helper: turns plain-language questions into explained shell commands and summarizes piped logs. Print-only by design: it never executes commands (zero `process::Command` in the Rust core); you review and run what it prints.

## Stack

Python 3.10+ (poetry: rich, openai, anthropic, psutil, pyyaml; console scripts `smah`, `smah-db`) plus a Rust workspace (`crates/*`: clap 4, tokio, ratatui/crossterm TUI, rusqlite bundled SQLite, reqwest on rustls; toolchain pinned via `rust-toolchain.toml` at 1.82).

## Dev mode

```bash
poetry install
smah config                                  # first-run setup (writes ~/.smah/config.yaml)
smah ask "convert an mp4 into a gif with ffmpeg"   # one-shot; piped stdin is the data
```

## Ports / URLs

- none (library/CLI)

## Tests

```bash
poetry run pytest   # Python suite (tests/)
cargo test          # Rust workspace (crates/*)
```

## Gotchas

- Print-only invariant: suggested commands are printed in a delimited block with impact notes, and clipboard-copied when clipboard support is available — never run.
- Model key via `SMAH_OPENAI_API_KEY` env or `~/.smah/config.yaml` (`smah config`); `smah config show` masks secrets. Claude and OpenAI providers supported.
- `smah chat` is not implemented yet (exits 2 with a notice); older flags (`-q`, `-i`, `--continue`, ...) still work but print `[deprecation]` notices and are removed in 0.4.
- Session store is SQLite with a bespoke checksummed migration runner (`smah-db`) that must stay byte-exact with the Python-era MD5 scheme.

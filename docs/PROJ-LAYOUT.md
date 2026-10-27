# Project Layout — smah

Navigable map of the repository. One-line descriptions; generated, symlinked,
and gitignored paths are marked. Stack: Python ≥3.10 CLI (Poetry packaging,
pytest, Sphinx/ReadTheDocs docs) plus a **Rust workspace scaffold**
(`crates/*`, 4-crate Cargo, TUI rewrite in flight — M0).

```
smah/                                # repo root (trl-infra submodule at Projects/smah)
├── smah/                            # Python package — the CLI
│   ├── smah.py                      #   CLI entry point (`smah` console script; main dispatch)
│   ├── smah_migrate.py              #   `smah-db` console script (DB migrations entry)
│   ├── exit_codes.py                #   shared exit-code constants
│   ├── args/                        #   CLI argument parsing
│   ├── commands/                    #   subcommand handlers: config, doctor, history, models
│   ├── console/                     #   rich-based console output
│   ├── database/                    #   DB layer + migrations/ (versioned migrations)
│   ├── logs/                        #   piped-log summarization support
│   ├── providers/                   #   LLM provider abstraction: base, registry, anthropic, openai
│   ├── runner/                      #   prompt/response pipeline: prompts, response_parser (+ conditions)
│   └── settings/                    #   layered settings; system/user config discovery
├── crates/                          # Rust workspace — TUI rewrite scaffold (M0)
│   ├── smah-cli/                    #   clap command tree, dispatch; `smah` binary (src/main.rs)
│   ├── smah-core/                   #   UI-agnostic domain: settings, runner, parser, db, console
│   ├── smah-providers/              #   provider trait, registry, hand-rolled Anthropic/OpenAI clients, SSE
│   └── smah-tui/                    #   ratatui app, widgets, event loop, TTY-gated surfaces
├── tests/                           # pytest suite (flat; mirrors package areas)
├── docs/                            # documentation
│   ├── PROJ-LAYOUT.md               # this file
│   ├── PROJ-LAYOUT.summary.md       # condensed tree (kept in sync with this file)
│   ├── PROJ-ARCH.md                 # architecture overview (lifecycle, providers, safety, exit codes)
│   ├── PROJ-ARCH.summary.md         # condensed architecture reference
│   ├── arch/                        # arch section details
│   │   └── data-flow.md             #   stage-by-stage request lifecycle
│   ├── PROJ-SCHEMA.md               # data-schema reference: SQLite store, config, contracts
│   ├── PROJ-SCHEMA.summary.md       # condensed schema reference (Mermaid only)
│   ├── THREAT-MODEL.md              # threat model: boundaries, STRIDE register, residual risk
│   ├── THREAT-MODEL.summary.md      # condensed threat-model reference
│   ├── threats/                     # threat-model details
│   │   ├── attack-surface.md        #   ingress enumeration + STRIDE per boundary
│   │   ├── data-stores.md           #   ~/.smah/ inventory; T-001/T-002 detail
│   │   └── supply-chain.md          #   deps, CI/release path, PyPI OIDC
│   ├── source/                      # Sphinx source: conf.py, index.rst, architecture.md, personas.md …
│   │   ├── code/_autosummary/       #   GENERATED — sphinx autosummary stubs; regenerate, never edit
│   │   ├── README.md                #   SYMLINK → root README.md
│   │   ├── CHANGELOG.md             #   SYMLINK → root CHANGELOG.md
│   │   ├── CONTRIBUTING.md          #   SYMLINK → root CONTRIBUTING.md
│   │   └── LICENSE.rst              #   SYMLINK → root LICENSE
│   ├── ux/                          # UX/planning specs: interface-spec.md, epics, roadmap, personas …
│   ├── Makefile                     # Sphinx build (POSIX)
│   └── make.bat                     # Sphinx build (Windows)
├── .github/workflows/
│   ├── ci.yml                       # pytest matrix 3.10/3.12/3.13; PRs + pushes to develop/stage/main
│   ├── ci-rs.yml                    # Rust CI: fmt/clippy/test (linux+macos) + no-shell-gate; SHA-pinned actions
│   └── release.yml                  # PyPI Trusted Publishing (OIDC) on v* tags
├── pyproject.toml                   # Poetry packaging; console scripts: smah, smah-db
├── poetry.lock                      # locked dependency set
├── Cargo.toml / Cargo.lock          # cargo workspace manifest + lockfile (crates/*)
├── rust-toolchain.toml              # stable channel; MSRV 1.82 (workspace `rust-version`)
├── README.md                        # project overview + usage
├── CHANGELOG.md                     # dated change log (newest first)
├── CONTRIBUTING.md                  # contribution guide
├── TODO.md                          # open work items
├── LICENSE                          # MIT
├── .envrc                           # direnv — sources .envrc.secret / .envrc.dev when present
└── .readthedocs.yaml                # RTD build config (smah.readthedocs.io)
```

## Key files requiring local setup

| Path | Action |
|------|--------|
| `.envrc.secret` | Create locally (gitignored) — provider API keys; sourced by `.envrc` |
| `.envrc.dev` | Optional local dev overrides (gitignored); run `direnv allow` after creating either file |
| `config/` | Created/managed at runtime by the app; gitignored — absent from a fresh clone |

## Generated / non-canonical paths

- `docs/source/code/_autosummary/` — Sphinx-generated API stubs; regenerate, never hand-edit.
- `docs/source/{README.md, CHANGELOG.md, CONTRIBUTING.md, LICENSE.rst}` — symlinks to the root files; edit the root originals only.

## Gitignored / local-only paths

`.DS_Store` · `.claude/worktrees/` (agent worktrees) · `__pycache__/` · `logs/` ·
`.idea/` · `.direnv/` · `dist/` · `build/` · `config/` · `*.egg-info/` ·
`.envrc.secret` · `.envrc.dev` · `current.session` · `current.log` ·
`noizu-ops.log` · `.ch` · `*.gz` · `target/` (cargo build dir)

## Safety invariant

`smah` is print-only by design: it explains and prints shell commands, never
executes them. Enforced on the Python tree by `tests/test_no_shell_execution.py`
and on the Rust tree by the `no-shell-gate` job in `ci-rs.yml` — keep both green.

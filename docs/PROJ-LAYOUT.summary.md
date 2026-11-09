# Project Layout — summary

Condensed tree extracted from [PROJ-LAYOUT.md](PROJ-LAYOUT.md); kept in sync.

```
smah/
├── smah/                # Python package — the CLI
│   ├── smah.py          #   entry point (`smah`)
│   ├── smah_migrate.py  #   entry point (`smah-db`)
│   ├── args/            #   argument parsing
│   ├── commands/        #   subcommands: config, doctor, history, models
│   ├── console/         #   rich output
│   ├── database/        #   DB layer + migrations/
│   ├── logs/            #   piped-log summarization
│   ├── providers/       #   LLM providers: anthropic, openai, registry
│   ├── runner/          #   prompt/response pipeline
│   └── settings/        #   layered settings + config discovery
├── crates/              # Rust workspace — TUI rewrite scaffold (M0)
│   ├── smah-cli/        #   clap tree + `smah` binary
│   ├── smah-core/       #   UI-agnostic domain core
│   ├── smah-providers/  #   provider trait + hand-rolled clients, SSE
│   └── smah-tui/        #   ratatui app + TTY-gated surfaces
├── tests/               # pytest suite (flat)
├── docs/                # Sphinx (source/) + UX specs (ux/) + layout/arch/schema/threat docs
│   └── source/code/_autosummary/   # GENERATED
├── .github/workflows/   # ci.yml (pytest) · ci-rs.yml (Rust + no-shell-gate) · release.yml (PyPI OIDC)
├── pyproject.toml       # Poetry; console scripts smah, smah-db
├── poetry.lock
├── Cargo.toml / Cargo.lock / rust-toolchain.toml   # cargo workspace, stable channel, MSRV 1.82
├── README.md / CHANGELOG.md / CONTRIBUTING.md / TODO.md / LICENSE (MIT)
├── .envrc               # sources .envrc.secret / .envrc.dev (local setup)
└── .readthedocs.yaml    # smah.readthedocs.io
```

Local setup: `.envrc.secret` (API keys, gitignored) · `.envrc.dev` (overrides) ·
`config/` (runtime, gitignored). Print-only safety invariant enforced by
`tests/test_no_shell_execution.py`.

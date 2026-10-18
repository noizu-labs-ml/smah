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
├── tests/               # pytest suite (flat)
├── docs/                # Sphinx (source/) + UX specs (ux/) + layout docs
│   └── source/code/_autosummary/   # GENERATED
├── .github/workflows/   # ci.yml (pytest), release.yml (PyPI OIDC)
├── pyproject.toml       # Poetry; console scripts smah, smah-db
├── poetry.lock
├── README.md / CHANGELOG.md / CONTRIBUTING.md / TODO.md / LICENSE (MIT)
├── .envrc               # sources .envrc.secret / .envrc.dev (local setup)
└── .readthedocs.yaml    # smah.readthedocs.io
```

Local setup: `.envrc.secret` (API keys, gitignored) · `.envrc.dev` (overrides) ·
`config/` (runtime, gitignored). Print-only safety invariant enforced by
`tests/test_no_shell_execution.py`.

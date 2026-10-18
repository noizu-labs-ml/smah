# Project Layout — smah

Navigable map of the repository. One-line descriptions; generated, symlinked,
and gitignored paths are marked. Stack: Python ≥3.10 CLI, Poetry packaging,
pytest, Sphinx/ReadTheDocs documentation.

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
├── tests/                           # pytest suite (flat; mirrors package areas)
├── docs/                            # documentation
│   ├── PROJ-LAYOUT.md               # this file
│   ├── PROJ-LAYOUT.summary.md       # condensed tree (kept in sync with this file)
│   ├── PROJ-ARCH.md                 # architecture overview (lifecycle, providers, safety, exit codes)
│   ├── PROJ-ARCH.summary.md         # condensed architecture reference
│   ├── arch/                        # arch section details
│   │   └── data-flow.md             #   stage-by-stage request lifecycle
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
│   └── release.yml                  # PyPI Trusted Publishing (OIDC) on v* tags
├── pyproject.toml                   # Poetry packaging; console scripts: smah, smah-db
├── poetry.lock                      # locked dependency set
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
`noizu-ops.log` · `.ch` · `*.gz`

## Safety invariant

`smah` is print-only by design: it explains and prints shell commands, never
executes them. Enforced by `tests/test_no_shell_execution.py` — keep it green.

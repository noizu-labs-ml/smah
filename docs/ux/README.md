# SMAH UX Documentation

Internal engineering doc set for SMAH ("SMart As Hell SysteM Admin Helper"), a Python/Poetry CLI. Ground truth for current behavior is `/tmp/smah-ux/capability-inventory.md` (also mirrored as the reference for every `file:line` citation in this set); the repo README is marketing copy and frequently wrong.

## State of the product (honest version)

1. SMAH is a dormant early prototype — v0.1.13, last commit 2025-01-20, self-described as such; the working core is one-shot queries (`-q`), instruction files (`-i`), piped-stdin processing, sqlite session persistence (`--continue`/`--session`/`--history`), and rich output.
2. It currently **executes** model-generated shell via `subprocess.run(cmd, shell=True)` after a single y/n confirm (`smah/runner/runner.py:241`, `:429` — the source comments `# This is dangerous`); binding decision: the executor is being removed in favor of print-only handoff.
3. It is OpenAI-only with a model catalog frozen at late-2024 ids (`inference_defaults.yaml`) and nothing usable enabled out of the box, so a fresh install's first query crashes; binding decision: pluggable providers with Anthropic/Claude default (`claude-opus-5-5`, `claude-haiku-5-5`).
4. `main()` swallows all exceptions (`smah/smah.py:175-176`), so most failures look like silent no-ops; seven advertised flags plus `smah-admin` and the README's `--setup` are vaporware.
5. This doc set plans the recovery: safety and honesty first, then usable-out-of-the-box, the subcommand surface, provider abstraction, and only then new capability.

## Contents

| File | What it is |
|---|---|
| `personas.md` | Four personas grounded in verified capabilities, including the print-only-required and print-only-annoyed tension. |
| `user-stories.md` | 39 stories in persona/need form with acceptance checklists, each marked `[works today]` / `[broken]` / `[missing]`. |
| `epics.md` | Nine dependency-ordered epics (E1–E9) with problem evidence, scope, acceptance criteria, risks, and size. |
| `roadmap.md` | Phases 0–5 with exit criteria and an explicit not-doing list; no calendar dates. |
| `interface-spec.md` | The concrete CLI: subcommand reference, back-compat/deprecation table, ASCII mockups, error-message table, pipe-safety contract. |

## Binding product decisions encoded here

1. **Print-only execution** — no command execution, ever; clipboard/shell handoff instead; risk explanation is an output-quality concern, not an impact engine.
2. **Pluggable providers, Claude default** — `claude-opus-5-5` (query/pipe/chat), `claude-haiku-5-5` (planner/picker); OpenAI retained; model ids in config, enabled by default.
3. **Subcommands with back-compat aliases** — `ask`/`chat`/`history`/`resume`/`config`/`models`/`doctor`; exit codes 0 ok, 2 usage, 3 config, 4 auth-or-no-models, 5 provider.
4. The brand site (smah.pro) is a separate workstream; nothing here is site copy.

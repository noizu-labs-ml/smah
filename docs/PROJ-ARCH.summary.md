# Project Architecture — summary

Condensed reference extracted from [PROJ-ARCH.md](PROJ-ARCH.md); kept in sync.

**SMAH** = print-only sysadmin CLI: plain-language questions → explained shell
commands; piped logs summarized. Never executes. Local-only — everything under
`~/.smah/` (`config.yaml`, `smah.db`, `logs/`); no daemon/server.

**Pipeline:** args (stdin only for `ask`) → settings load (unconfigured ⇒ exit 3)
→ planner LLM call (strict JSON plan: model, include_settings, format) → answer
LLM call on planned model (full thread; pipe in full under `--- INPUT ---`) →
rich panels / raw stdout → `<exec>` tags become "NOT EXECUTED" review panels →
transactional SQLite session save. Detailed lifecycle: [arch/data-flow.md](arch/data-flow.md).

**Providers:** vendor-neutral seam (base/registry); model-driven selection via
`inference.model_picker` — Claude (`anthropic.claude-haiku-5-5`) is the
first-tried working default; models config-defined, credential-probed tolerantly.
Secrets: flag → config (`$ENV_VAR` indirection) → env; never printed, never in DB.

**Safety:** print-only invariant, test-enforced (`test_no_shell_execution.py`);
`exec-if` conditions via whitelisted AST walk (no eval/exec); optional pyperclip.

**Contract:** stdout = content only; notices/panels/wizard/doctor on stderr
(`docs/ux/interface-spec.md` § 5 normative). Exit codes: 0 ok · 1 internal ·
2 usage · 3 config · 4 auth/no-model · 5 provider/API.

**Storage:** SQLite `~/.smah/smah.db` — sessions only, auto-migrates at startup;
`smah-db` = manual migration CLI (checksummed, single migration today).

**Stack:** Python ≥3.10 · Poetry · rich · anthropic+openai SDKs · pyyaml ·
psutil · lxml · pytest · Sphinx/RTD · GH Actions (pytest; PyPI trusted publishing).

**Known gaps:** `docs/source/architecture.md` is a stale aspirational draft
(execution/RBAC/TimescaleDB never existed — superseded); `doctor.__check_provider`
still OpenAI-only (bug vs shipped Anthropic provider); `chat` is an honest stub.

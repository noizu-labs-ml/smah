# Script wrappers — format, planning, composing, executing (SW-01…SW-06)

Wrapper = a **saved, ordered recipe** of steps the deterministic path
*prints* and the companion `smah-run` binary *executes* after explicit user
approval. `smah` itself stays print-only (owner decision 2026-11-09): it
resolves, plans, composes, and reviews — it never spawns a step and never
invokes `smah-run` on the user's behalf.

Related: virtual tools (`docs/ux/virtual-tools.md`) are single-purpose
personas the CLI driver *emulates*; wrappers compose reusable steps into a
recipe. The namespaces are **disjoint** — a wrapper whose name collides
with an installed virtual tool is a load error, not a shadow.

## The file

`~/.config/smah/wrappers/<name>.md` (honors `SMAH_WRAPPERS_DIR`, then
`XDG_CONFIG_HOME`). The filename stem **is** the name; `name:` must equal
it. YAML frontmatter + markdown body (docs only — the recipe is the
frontmatter):

```yaml
---
name: session-transcript          # must equal the filename stem
description: Turn a recording into a transcript
model: <provider>.<model-id>      # OPTIONAL: the --prompt composition model
behavior_prompt: |                # OPTIONAL: persona for composition turns
  You are meticulous about transcripts.
approved: 2026-11-09T14:30:00-08:00   # OPTIONAL (RFC 3339): user sign-off
params:                           # OPTIONAL, max 32
  - name: cast_file
    description: Path to the .cast recording
    required: true                # required and default are mutually exclusive
  - name: label
    default: transcript
steps:                            # REQUIRED, 1..64, ids unique
  - id: to-text
    description: Convert cast events     # optional, shown in the plan
    use: cast-to-text              # a registered built-in tool ...
    with: {cast_file: "{{params.cast_file}}"}   # ... bound by name
    output: text                   # capture this step's output as 'text'
  - id: announce
    description: Print the label followed by the transcript
    run: ["echo", "{{params.label}}:", "{{steps.to-text.output}}"]  # fixed argv
    cwd: "{{params.workdir}}"      # optional
    timeout_secs: 15               # optional, default 60
outputs:                           # OPTIONAL; every declared output must be
  - name: text                     # bound by exactly one step
    description: The transcript
preconditions:                     # OPTIONAL; echoed by the plan printer
  - The cast file exists and is an asciinema v2 recording
---
Body markdown (free-form docs).
```

A step is exactly one of:

- `use: <built-in-name>` + `with: {<param>: <template>}` — executed
  **in-process** by the registry (today: `cast-to-text`, SW-04; pure Rust,
  no exec). `with` values are templates.
- `run: [<argv>…]` — a **fixed argv**: elements map 1:1 onto exec
  arguments. Never a shell line, never `sh -c`, no interpolation beyond
  the bindings below — by construction (the CI no-shell gate plus the
  `smah-run`-only exemption enforce this at the workspace level).

Bindings in any template string (`run` argv, `with` values, `cwd`):

- `{{params.<name>}}` — a declared param (supplied at invocation, or its
  default). Undeclared = load error; an unknown supplied `--param` is a
  usage error (typo footgun).
- `{{steps.<id>.output}}` — the captured stdout of an **earlier** step that
  declares `output:`. Forward references are load errors; anything else
  inside `{{…}}` rejects.

Strictness: unknown frontmatter keys, unknown step/param keys, duplicate
ids/names, empty argv elements, zero timeouts — all load errors. An
invalid file is listed by `smah wrapper list` with its reason; `wrapper
show` prints files verbatim; `wrapper validate` checks without params.

## The deterministic path (SW-02)

```
smah --wrapper session-transcript --param cast_file=last.cast
smah wrapper run session-transcript --param cast_file=last.cast   # same thing
smah wrapper list [--json] · smah wrapper show <name> · smah wrapper validate <name>
```

Prints the resolved, ordered plan: each step's display argv (params bound,
`{{steps.*}}` shown symbolically until run time), per-step risk notes
(built-ins are in-process; `run` argv[0] is resolved via `PATH` — named as
the risk it is), cwd/timeout/output lines, preconditions, approval status,
and the exact `smah-run` invocation that would execute it.

**Postfix dispatch** (the review's recommended form, implemented alongside
the subcommand): a symlink `smah-<name> -> smah` whose `<name>` matches an
installed wrapper re-dispatches as `smah --wrapper <name>`; `smah hooks
shims` already writes `smah.<name>` shims for tools, so wrapper symlinks
are a user choice (`ln -s $(which smah) ~/.local/bin/smah-session-transcript`).
`smah-run` is excluded by name — that argv[0] belongs to the runner.

## The `--prompt` compose-review flow (SW-03)

```
smah --wrapper session-transcript --prompt "also write the transcript to a file"
```

Inference is a **recipe composer**, never an editor: the model answers a
structured changes object — `{"explanation", "changes": {add_steps,
modify_steps, remove_steps, new_params, remove_params}}` — which is
applied in memory and **re-validated through the full loader** before
anything is shown. A diff that cannot produce a loadable recipe is a
contract failure (`WrapperPromptInvalid`, exit 5), not a proposal.

The review prints the explanation, a change summary, the complete proposed
file, and the provenance stamp that would land. Then:

- **pipe / non-TTY** (the default): nothing is written; re-run with
  `--accept` or `--reject`.
- **TTY**: `[a]ccept / [e]dit / [r]eject` — `e` writes
  `<name>.md.proposed` beside the wrapper (no `$EDITOR`: smah never
  launches an editor) for hand-editing; the file itself still only changes
  when you replace it.

Accepting **is** the approval review: the saved file gains `approved:`
(now, RFC 3339) plus a `# provenance:` comment (timestamp, model, run id,
prompt sha256) — every inference-sourced change is traceable, and
`smah-run --execute` will accept the recipe afterwards.

Model selection: `--model` > the wrapper's `model:` > the spec default —
no wrapper-hardcoded ids. Results are cached by
`(wrapper sha256, prompt sha256)` under `~/.cache/smah/wrappers/<name>/`
(honors `XDG_CACHE_HOME`); a repeat prompt against an unchanged wrapper
skips the model call entirely, and a cache entry that no longer parses as
valid changes is dropped loudly and recomposed.

## Execution — `smah-run` (SW-05)

The **only** workspace crate allowed to spawn processes
(`scripts/no-shell-gate.sh` exempts exactly `crates/smah-run/src`; CI
self-tests both directions). Contract:

- executes **saved** wrappers only — no stdin recipes, no composing;
- **dry-run by default**: `smah-run <name> [--param k=v]…` prints the same
  resolved plan; `--execute` is the explicit opt-in;
- `--execute` requires an `approved:` stamp (exit 3 without it);
- per-step confirmation `[y]es / [s]kip / [a]bort` on a TTY; `--yes` for
  non-interactive runs (and `--execute` on a non-TTY **without** `--yes`
  refuses — exit 2 — before running anything);
- `run:` steps spawn fixed argv; built-ins run in-process; a step that
  declares `output:` gets its stdout captured (capped at 1 MiB) for
  `{{steps.*}}` bindings, otherwise stdout streams live;
- per-step timeout (step `timeout_secs`, else 60s) — a timed-out step is
  killed and the run fails (exit 1); a failing step aborts the run;
- append-only audit JSONL at `~/.local/state/smah/audit-smah-run.jsonl`
  (0700/0600), **metadata only**: per-step decision/status/duration/bytes
  with argv run through the registered-secret redactor and the values of
  secret-ish param names (`*token*`, `*password*`, `*secret*`, `*key*`,
  `*api*`, `*credential*`, `*auth*`) masked. Dry-runs write nothing.

Exit codes: 0 success · 2 usage · 3 invalid/unapproved wrapper · 1 step
failure, timeout, or user abort.

## Phase 2 (not in this epic's code)

GIF rendering (SW-07) and any additional built-ins land in phase 2;
`cast-to-text` is the phase-1 built-in. Hooks-shim generation for wrappers
(the `smah-<name>` symlink farm) is future work — today the symlink is a
one-line `ln -s`.

## Examples

`examples/wrappers/session-transcript.md` (the "session → transcript"
reference wrapper) and `examples/wrappers/recent-log.md` (minimal
`run:`-only recipe). Copy either into `~/.config/smah/wrappers/` to try
it. Threat register rows for the wrapper surface: T-031…T-035 in
`docs/THREAT-MODEL.md`.

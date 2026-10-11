# SMAH Personas

Four personas for the SMAH recovery effort. Each is grounded in capabilities that verifiably exist today (see `capability-inventory.md`): one-shot queries (`-q`), instruction files (`-i`), piped-stdin processing, sqlite session persistence with `--continue` / `--session` / `--history`, rich-formatted output, and the first-run interview that captures user background and system details. No persona is attributed an aspirational feature.

Two product decisions shape every persona: SMAH is print-only (it never executes commands; decision 1) and the default provider becomes Anthropic/Claude with OpenAI retained as a backend (decision 2).

---

## P1 — Priya, production platform engineer

**Role:** Platform engineer at a payments-adjacent SaaS; owns a ~40-host Linux fleet behind strict change control (SOC 2 audit trail). Works from a macOS laptop through hardened jump hosts.

**Context:**
- Every production change needs a reviewed ticket: the exact command, what it does, what it touches, rollback.
- She knows her field well but not every flag of every tool (`journalctl`, `openssl`, `iptables` variants across distros).
- No tool is allowed to execute anything on production hosts without human review. That is policy, not preference.

**Goals:**
- Draft correct commands faster than searching man pages and internal wikis.
- Produce a reviewable artifact per suggestion: command, plain-language explanation, risk classification — pasteable straight into a change ticket.
- Keep a trail of what she asked and what was suggested (sessions already persist in sqlite).

**Current workflow without SMAH:** man pages, the runbook wiki, asking teammates; copying commands out of a browser AI chat that knows nothing about her target OS, and hand-writing the "what this does / risk" section of the ticket.

**Pains SMAH can address:**
- Flag recall for rarely-used incantations, tailored to the OS/shell captured in her settings (first-run interview, inventory §5).
- The explanation and risk text she currently writes by hand — as suggestion-block content she can paste (works-today query path `smah/runner/runner.py:359-446` minus execution).
- Session persistence means the drafting trail already exists (`~/.smah/smah.db`, inventory §6).

**What would make her abandon it:**
- Any execution of a suggested command, including an opt-in toggle. Hard no.
- Suggestions that ignore her stated OS/shell (she works on macOS, targets Linux).
- Silent failures — if `smah ask` exits quietly on an auth error she will assume it is broken and uninstall (`smah/smah.py:175-176` swallows all exceptions today).
- No visibility into what data leaves her machine.

**Risk tolerance for AI-generated shell:** Near zero for execution — print-only is a *requirement* for her. High for reading: she is exactly the user the command-suggestion block with risk explanation is for.

---

## P2 — Marcus, solo sysadmin ("the whole IT department")

**Role:** The only IT person at a 200-employee logistics company. Mixed Ubuntu fleet, one Windows file server, a NAS, no change-control process — he is the process.

**Context:**
- Firefights in the evenings; speed matters more than ceremony.
- Would honestly prefer a tool that just runs the boring 90% of commands for him.
- Scarred by a near-miss with a wrong-path `rm -rf` years ago, so he is less cavalier than he sounds.

**Goals:**
- One-liners for rare tasks (LVM resize, `openssl` renew, samba quirks) without re-googling.
- Get in, get the command, get out — minimum keystrokes.

**Current workflow without SMAH:** greps his own `~/.bash_history`, StackOverflow, a browser AI chat with copy-paste that does not know his distro or that his "linux box" is actually WSL2 on his laptop.

**Pains SMAH can address:**
- Environment awareness without re-explaining: the first-run profile (experience, role, OS details) is injected into every request — the actual differentiator vs browser chat (inventory §5).
- Session resume for multi-step tasks (`--continue` works today, `smah/runner/runner.py:165-249`).
- Plain-language explanations he can skim instead of reading flags he does not know.

**What would make him abandon it:**
- Print-only friction: if getting a command into his shell takes more than one keystroke (select-with-mouse copy is already his habit from browser chat, so SMAH must beat it, not match it).
- Silent no-op exits — he will assume the tool hung and kill it (`smah/smah.py:175-176`).
- Setup that does not end with a working tool (today a keyless install registers models anyway — registration never checks for an API key — then the provider call fails and `smah/smah.py:175-176` swallows it, so his first commands look like silent no-ops).

**Risk tolerance for AI-generated shell:** High in theory, moderate after the scar. He *wants* execution and will grumble that print-only is nanny-state — this document does not pretend otherwise. He accepts it only if clipboard/shell handoff is instant and reliable. He is the persona print-only is a *cost* for; see the tension section below.

---

## P3 — Dana, developer who drifts into ops

**Role:** Backend developer who owns a service end-to-end, including the box it runs on. Comfortable with git and docker; shaky on `ufw`, systemd units, log rotation, TLS renewal.

**Context:**
- Works on macOS, deploys to Linux — commands that differ per OS are a recurring trap.
- Uses browser AI chat today and pastes semi-blindly; has run one command with a hallucinated flag (it failed loudly; no harm).
- Wants to *learn* the flags, not just receive magic strings.

**Goals:**
- Intent in, correct command out, with education attached: what each flag does, what the risky parts are.
- Incident support: pipe logs at a problem and get a structured summary (pipe mode works today, `smah/runner/runner.py:449-509`).

**Current workflow without SMAH:** AI chat in a browser tab, copy-paste, and a lot of re-asking when the answer assumes the wrong OS.

**Pains SMAH can address:**
- Per-flag explanations and risk labels teach safety instead of bypassing it.
- OS-aware answers from captured system settings (with the caveat that the system stats fed to the model are currently wrong: memory readings report CPU data, `smah/settings/system/system.py:88`).
- Instruction files (`-i`) for the long "here is my situation" prompts they naturally write.

**What would make them abandon it:**
- Answers calibrated for the wrong OS after they told SMAH their setup.
- A stale model catalog producing degraded answers (today's ids are frozen at late-2024, inventory §4).
- Once burned by a hallucinated flag with no risk labeling to make them pause.

**Risk tolerance for AI-generated shell:** Moderate. Would accept auto-run for obviously-safe operations, but print-only with good explanations actually serves the learning goal better. Roughly neutral on the print-only decision.

---

## P4 — Ines, data engineer / log wrangler

**Role:** Data engineer; pipelines, k8s logs, access logs. Lives in the shell and treats every tool as a pipeline stage.

**Context:**
- Primary interaction: `kubectl logs ... | <tool> | <next tool>`. Output discipline is not a nicety, it is the contract.
- Logs she pipes can contain tokens and user data; she thinks about egress.
- Rarely wants a command *run*; she wants text *transformed*.

**Goals:**
- `kubectl logs svc/api | smah ask "summarize errors by service, markdown table"` producing clean markdown on stdout (works today in essence, inventory §3).
- Iterate: resume a session to refine the analysis (`--continue` / `--history`).
- Know exactly when her input was truncated or what was sent to the provider.

**Current workflow without SMAH:** `jq`/`awk` chains plus pasting blobs into browser AI chat, losing all structure.

**Pains SMAH can address:**
- The pipe path is real and works (`__get_pipe`, `smah/args/args.py:97-107`; `Runner.pipe()`).
- The existing stdout/stderr console split already keeps chrome off stdout (`smah/console/console.py:15-18`) — worth preserving and enforcing.

**What would make her abandon it:**
- Anything on stdout that is not the answer. Today `Runner.query` echoes the raw query to stdout (`smah/runner/runner.py:375`) — that class of defect is disqualifying.
- Silent truncation: pipe input is cut at 2048 chars / head 1024 (`smah/runner/runner.py:23-24`, `prompts.py:10-11`) with no notice — she will get confidently wrong summaries of data SMAH never read.
- No control or visibility over what data leaves the machine.

**Risk tolerance for AI-generated shell:** Low-to-moderate and largely irrelevant — she almost never wants execution. Her risk axis is data egress and silent truncation, not command safety. Print-only changes nothing for her workflow.

---

## The print-only tension (stated plainly)

| Persona | Print-only is… | Consequence for design |
|---|---|---|
| Priya | a requirement | She cannot use the product at all if it executes. |
| Marcus | a cost he tolerates | He stays only if handoff is one keystroke; anything slower and browser chat wins. |
| Dana | neutral-to-positive | Explanations serve the learning goal; execution would not. |
| Ines | irrelevant | Her risk axis is data egress, not command execution. |

The tension is real and one-sided in cost: Marcus pays for Priya's constraint. The product answer is to make payment cheap — a single-keystroke clipboard copy and a paste-ready shell handoff — and to state the policy without apology in the output itself ("SMAH does not run commands"). Per binding decision 1 there is no plan to add an execution toggle later, even opt-in: that would delete Priya (the persona with the strictest need) to save Marcus one keystroke. If Marcus's friction proves fatal to adoption, the data will show it in the handoff-completion rate; the decision to revisit would be the owner's, made explicitly, not a config flag that crept in.

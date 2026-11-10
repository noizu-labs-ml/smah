---
name: recent-log
description: Show the tail of a log file with a line count and a hard timeout
params:
  - name: log_file
    description: Path to the log file
    required: true
  - name: lines
    description: How many trailing lines to show
    default: "50"
steps:
  - id: tail
    description: Read the trailing lines (fixed argv — no shell)
    run: ["tail", "-n", "{{params.lines}}", "{{params.log_file}}"]
    timeout_secs: 5
    output: tail
outputs:
  - name: tail
    description: The requested trailing lines
preconditions:
  - The log file is readable
---

# recent-log

The minimal `run:`-only wrapper: one fixed-argv step, a defaulted param, a
per-step timeout, and a captured output. `tail` is resolved via `PATH` —
the plan printer says so, and that is the risk you review before executing.

    smah --wrapper recent-log --param log_file=/var/log/system.log
    smah-run recent-log --param log_file=/var/log/system.log --execute --yes

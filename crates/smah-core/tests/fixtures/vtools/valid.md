---
name: valid
description: A valid fixture tool with every key present.
model: anthropic.claude-opus-5-5
driver: emulated
constraints:
  - Never invent log lines that are not in the input.
allow_roots:
  - ~/notes
  - $PROJECT_LOGS
---
You are a meticulous log analyst. Summarize what you are given.

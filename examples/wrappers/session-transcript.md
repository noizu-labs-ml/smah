---
name: session-transcript
description: Turn an asciinema v2 recording into a plain-text transcript
params:
  - name: cast_file
    description: Path to the .cast recording (asciinema v2)
    required: true
  - name: label
    description: Prefix printed before the transcript
    default: transcript
steps:
  - id: to-text
    use: cast-to-text
    description: Convert cast events to text (in-process, no exec)
    with: {cast_file: "{{params.cast_file}}"}
    output: text
  - id: announce
    description: Print the label followed by the transcript
    run: ["echo", "{{params.label}}:", "{{steps.to-text.output}}"]
outputs:
  - name: text
    description: The plain-text transcript
preconditions:
  - The cast file exists and is an asciinema v2 recording (version 2 header)
---

# session-transcript

Replays a terminal recording as plain text — the reference example for the
wrapper format (a built-in `use:` step feeding a fixed-argv `run:` step
through an output binding).

Plan only, until you hand it to the companion executor:

    smah --wrapper session-transcript --param cast_file=~/sessions/last.cast
    smah-run session-transcript --param cast_file=~/sessions/last.cast   # dry-run
    smah-run session-transcript --param cast_file=~/sessions/last.cast --execute --yes

This file carries no `approved:` stamp — add one (or accept a `--prompt`
diff) before expecting `smah-run --execute` to run it.

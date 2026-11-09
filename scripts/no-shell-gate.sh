#!/bin/sh
# No-shell gate (SW-05): no workspace crate may spawn processes — the sole
# sanctioned spawner is the smah-run executor, exempted by exact path
# (crates/smah-run/src and nothing else). This script is the single source
# of truth used by CI (ci-rs.yml) and by the smah-run crate's own tests.
#
# Usage: no-shell-gate.sh [workspace-root]   (default: script's parent)
# Exit:  0 = clean · 1 = offenders found · 2 = usage error
set -u

ROOT="${1:-}"
if [ -z "$ROOT" ]; then
    ROOT="$(cd "$(dirname "$0")/.." && pwd)"
fi
if [ ! -d "$ROOT/crates" ]; then
    echo "no-shell-gate: $ROOT has no crates/ dir (wrong root?)" >&2
    exit 2
fi

PATTERN='Command::new|process::Command|process::spawn'
status=0

for src in "$ROOT"/crates/*/src; do
    [ -d "$src" ] || continue
    case "$src" in
        # The companion executor is the ONLY crate allowed to spawn.
        */smah-run/src) continue ;;
    esac
    # POSIX grep -E -r -n; a hit means an illegal spawn.
    if hits=$(grep -rnE "$PATTERN" "$src" 2>/dev/null); then
        echo "no-shell-gate: $src spawns processes (only crates/smah-run/src may):" >&2
        printf '%s\n' "$hits" >&2
        status=1
    fi
done

exit $status

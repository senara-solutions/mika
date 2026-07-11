#!/usr/bin/env bash
# Capture a wedge-diagnostic bundle for a running mika-spirit process.
#
# Background: when mika-spirit wedges (mika#1719 was n=3 in a single week),
# root-Claude captures a stack trace via `gdb -p <pid> -batch`. The bottleneck
# for offline symbol resolution has been the PIE base address: prior audits
# computed it as `runtime_addr - offset` from the stack, then tried to
# corroborate the offset via addr2line on a rebuild. The corroboration is
# necessarily indirect — both endpoints of the subtraction are the thing being
# verified.
#
# This script fixes that by snapshotting `/proc/<pid>/maps` **at the same
# instant** as the stack capture. The PIE base then becomes an independent
# artifact of the capture — the first line's start address is the load address
# of the main binary — not a derived-in-analysis input. Any later offline audit
# reads maps directly and never has to compute PIE base from stack traces.
#
# Invariants (do NOT change these without updating docs/solutions/):
#   1. PIE base is `/proc/<pid>/maps` first entry's start address for the
#      mika-spirit binary path. It is not computed from stack pointers.
#   2. All three files share a single timestamp prefix — the capture is
#      atomic within GDB's `.attach` boundary (see below); the maps snapshot
#      taken immediately after gdb detaches is close-enough for load-address
#      purposes since PIE base does not change after exec.
#   3. Files land in a per-capture directory so multiple wedges on the same
#      host don't stomp each other.
#
# Usage:
#   mika-spirit-wedge-capture.sh <pid>
#
# Emits (in /tmp/spirit-wedge-<ts>/):
#   <ts>-stacks.txt   — gdb `thread apply all bt 30` + `info registers`
#   <ts>-maps.txt     — cat /proc/<pid>/maps
#   <ts>-status.txt   — cat /proc/<pid>/status (RSS, threads count, state)
#
# Requires: gdb, procfs (Linux). Run as root or as the mika-spirit uid.

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "Usage: $0 <pid>" >&2
    exit 2
fi

PID="$1"

if ! [[ "$PID" =~ ^[0-9]+$ ]]; then
    echo "Error: PID must be numeric, got: $PID" >&2
    exit 2
fi

if [[ ! -d "/proc/$PID" ]]; then
    echo "Error: no process with pid $PID (missing /proc/$PID)" >&2
    exit 1
fi

if ! command -v gdb >/dev/null 2>&1; then
    echo "Error: gdb not found on PATH; wedge capture requires gdb" >&2
    exit 1
fi

TS="$(date -u +%Y-%m-%dT%H-%M-%SZ)"
OUTDIR="/tmp/spirit-wedge-${TS}"
mkdir -p "$OUTDIR"

STACKS="${OUTDIR}/${TS}-stacks.txt"
MAPS="${OUTDIR}/${TS}-maps.txt"
STATUS="${OUTDIR}/${TS}-status.txt"

echo "Capturing wedge diagnostics for pid=$PID into $OUTDIR" >&2

# 1. Stacks + registers via gdb.
#    `--batch` exits automatically after the -ex commands. `--nx` skips
#    ~/.gdbinit so the capture is reproducible across hosts. `--quiet`
#    trims the banner.
gdb --batch --nx --quiet \
    -ex "set pagination off" \
    -ex "attach $PID" \
    -ex "thread apply all bt 30" \
    -ex "info registers" \
    -ex "detach" \
    -ex "quit" \
    > "$STACKS" 2>&1 || {
        echo "warn: gdb capture returned non-zero; partial output kept in $STACKS" >&2
    }

# 2. Memory maps — the load address of the mika-spirit binary is the first
#    line's start address (see invariant 1 above). This file IS the PIE base
#    artifact; downstream audit never derives it from stack traces.
cat "/proc/$PID/maps" > "$MAPS" 2>&1 || {
    echo "warn: could not read /proc/$PID/maps (process died mid-capture?)" >&2
}

# 3. Status — RSS, threads count, run state.
cat "/proc/$PID/status" > "$STATUS" 2>&1 || {
    echo "warn: could not read /proc/$PID/status (process died mid-capture?)" >&2
}

echo "Capture complete:" >&2
echo "  $STACKS" >&2
echo "  $MAPS" >&2
echo "  $STATUS" >&2
echo "" >&2
echo "PIE base: first entry of ${MAPS}" >&2
echo "  head -1 $MAPS | awk -F- '{print \$1}'" >&2

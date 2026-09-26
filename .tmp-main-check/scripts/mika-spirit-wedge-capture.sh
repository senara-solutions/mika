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
#   2. All three files share a single timestamp prefix — captures run
#      concurrently as background jobs so the maps snapshot is close-in-time
#      to gdb's attach. PIE base does not change after exec, so exact
#      simultaneity is not required, only shared provenance.
#   3. Files land in a per-capture directory so multiple wedges on the same
#      host don't stomp each other.
#   4. Missing `gdb` is a warn-and-continue condition, not a hard failure —
#      maps + status alone still let root-Claude compute PIE base and thread
#      count, which is enough to triage most wedges.
#
# Usage:
#   mika-spirit-wedge-capture.sh <pid>
#
# Emits (in /tmp/spirit-wedge-<ts>/):
#   <ts>-stacks.txt   — gdb `thread apply all bt 30` + `info registers`
#                       (skipped with a warning if gdb is not installed)
#   <ts>-maps.txt     — cat /proc/<pid>/maps (the PIE-base artifact)
#   <ts>-status.txt   — cat /proc/<pid>/status (RSS, threads count, state)
#
# Requires: procfs (Linux). gdb is recommended but optional. Run as root or
# as the mika-spirit uid so /proc reads and ptrace attach succeed.

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

HAVE_GDB=1
if ! command -v gdb >/dev/null 2>&1; then
    HAVE_GDB=0
    echo "warn: gdb not found on PATH; skipping stack capture, continuing with maps+status" >&2
fi

TS="$(date -u +%Y-%m-%dT%H-%M-%SZ)"
OUTDIR="/tmp/spirit-wedge-${TS}"
mkdir -p "$OUTDIR"

STACKS="${OUTDIR}/${TS}-stacks.txt"
MAPS="${OUTDIR}/${TS}-maps.txt"
STATUS="${OUTDIR}/${TS}-status.txt"

echo "Capturing wedge diagnostics for pid=$PID into $OUTDIR" >&2

# Launch the three captures concurrently. Maps and status finish in
# microseconds — running them alongside gdb (which blocks for seconds during
# attach + backtrace) narrows the effective capture window on the volatile
# per-thread state. PIE base itself is stable across the window (invariant 2).

gdb_pid=""
if [[ "$HAVE_GDB" -eq 1 ]]; then
    # `--batch` exits automatically after the -ex commands. `--nx` skips
    # ~/.gdbinit so the capture is reproducible across hosts. `--quiet`
    # trims the banner.
    (
        gdb --batch --nx --quiet \
            -ex "set pagination off" \
            -ex "attach $PID" \
            -ex "thread apply all bt 30" \
            -ex "info registers" \
            -ex "detach" \
            -ex "quit" \
            > "$STACKS" 2>&1
    ) &
    gdb_pid=$!
else
    printf 'gdb unavailable on this host; stack capture skipped.\n' > "$STACKS"
fi

# The load address of the mika-spirit binary is the first line's start
# address of this file (see invariant 1 above). This file IS the PIE base
# artifact; downstream audit never derives it from stack traces.
( cat "/proc/$PID/maps" > "$MAPS" 2>&1 ) &
maps_pid=$!

( cat "/proc/$PID/status" > "$STATUS" 2>&1 ) &
status_pid=$!

# Wait for each job independently so a single failure doesn't abandon the
# others. `set -e` would kill the script on the first `wait` non-zero without
# `|| true`.
maps_rc=0
status_rc=0
gdb_rc=0
wait "$maps_pid" || maps_rc=$?
wait "$status_pid" || status_rc=$?
if [[ -n "$gdb_pid" ]]; then
    wait "$gdb_pid" || gdb_rc=$?
fi

if [[ "$maps_rc" -ne 0 ]]; then
    echo "warn: /proc/$PID/maps read exited $maps_rc (process died mid-capture?)" >&2
fi
if [[ "$status_rc" -ne 0 ]]; then
    echo "warn: /proc/$PID/status read exited $status_rc (process died mid-capture?)" >&2
fi
if [[ "$gdb_rc" -ne 0 ]]; then
    echo "warn: gdb capture exited $gdb_rc; partial output kept in $STACKS" >&2
fi

# Extract PIE base from the maps snapshot for the summary. Per invariant 1
# this is the verified value — informational here so operators see it
# without a follow-up shell dance.
pie_base=""
pie_source="maps.first_line"
if [[ -s "$MAPS" ]]; then
    pie_base="$(awk -F- 'NR==1 {print $1; exit}' "$MAPS" 2>/dev/null || true)"
fi
if [[ -z "$pie_base" ]]; then
    pie_base="<unavailable — maps snapshot empty>"
    pie_source="unavailable"
fi

echo "Capture complete:" >&2
echo "  $STACKS" >&2
echo "  $MAPS" >&2
echo "  $STATUS" >&2
echo "" >&2
echo "PIE base (verified from ${pie_source}): ${pie_base}" >&2
echo "  Source: head -1 $MAPS | awk -F- '{print \$1}'" >&2

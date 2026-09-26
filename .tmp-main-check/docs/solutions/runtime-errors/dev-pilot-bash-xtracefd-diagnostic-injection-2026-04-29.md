---
title: "dev-pilot handler silent exit-0: BASH_XTRACEFD diagnostic trace injection"
date: 2026-04-29
category: runtime-errors
module: dev-pilot
problem_type: runtime_error
component: tooling
symptoms:
  - "dev-pilot handler exits 0 silently before launching claude-pilot"
  - "No log file at /var/log/claude-pilot/<id>.log"
  - "EXIT trap callback delivers HANDLER CRASH (exit code 0) with no diagnostic"
  - "Task hangs or completes immediately (~3s) with no implementation work"
root_cause: logic_error
resolution_type: code_fix
severity: critical
tags:
  - dev-pilot
  - silent-exit
  - bash-xtracefd
  - exit-trap
  - handler-crash
  - diagnostic-trace
  - claude-pilot
---

# dev-pilot handler silent exit-0: BASH_XTRACEFD diagnostic trace injection

## Problem

`skills/bundled/dev-pilot/handlers/run.sh` exits 0 silently on a deterministic subset of dispatches BEFORE launching `claude-pilot`. The EXIT trap fires with `RESULT = "HANDLER CRASH (exit code 0). Script failed before building result."` but no diagnostic information about which line caused the exit. N=4 instances observed across mika#879 (×2), mika#861 (×1), and one ticketless dispatch — ruling out task-content-specific causes.

## Symptoms

- dev-pilot callback delivers in ~3 seconds (normal sessions take minutes)
- No log file appears at `/var/log/claude-pilot/<task_id>.log`
- `tasks.result` in mika.db contains only the generic EXIT-trap fallback message
- Control group dispatches (mika#862, #863, #864, mika-platform#62) all worked during the same sprint

## What Didn't Work

- **Static analysis of `exit 0` paths:** Only dry-run branches at lines 312/331 have explicit `exit 0`, but production inputs never include `dry_run`. The actual exit point is implicit (likely a `set -e` propagation from a command substitution).
- **Standard `set -x` tracing:** Previous attempts hit `set -x` trace truncation at 64KB because `set -x` outputs to stderr, which gets captured into `STDERR_FILE` and truncated to 10KB in the callback. The trace noise also pollutes claude-pilot's own stderr stream when using `exec 2>>`.
- **Original diagnostic recipe (`exec 2>>`):** mika#884's initial recipe used `exec 2>>/tmp/dev-pilot-trace-$$.log` which would clobber stderr for all child processes including `exec`'d `claude-pilot` — claude-pilot relies on its own stderr for logging. Rejected by mika-arch first-pass review.

## Solution

Three changes to `skills/bundled/dev-pilot/handlers/run.sh`:

### 1. Shebang change: `#!/bin/sh` → `#!/bin/bash`

Required for `BASH_XTRACEFD` (bash 4.1+ feature). The script body is POSIX-compatible; bash is a superset.

### 2. BASH_XTRACEFD trace injection after `set -e`

```bash
TRACE_FILE="/tmp/dev-pilot-trace-$$.log"
# Degrade gracefully if /tmp is unwritable
exec 9>>"$TRACE_FILE" 2>/dev/null || exec 9>/dev/null
BASH_XTRACEFD=9
set -x
```

Key design decisions:
- **`BASH_XTRACEFD=9` (bare assignment, not `export`):** Routes xtrace to fd 9 instead of fd 2. Bare assignment prevents propagation to exec'd child processes that don't inherit fd 9.
- **`exec 9>>` with fallback to `/dev/null`:** If `/tmp` is unwritable, the handler continues without trace instead of silently exiting before the EXIT trap is registered (which would recreate the exact failure class being diagnosed).
- **PID-suffixed file (`$$`):** Avoids concurrent-dispatch collisions.

### 3. EXIT trap modification

```bash
# In deliver_callback, after the existing HANDLER CRASH fallback:
if [ -f "$TRACE_FILE" ]; then
    case "$RESULT" in
        "HANDLER CRASH"*)
            # Crash path: append trace tail, preserve file for forensics
            _TRACE_TAIL=$(tail -50 "$TRACE_FILE" 2>/dev/null | sed 's/^/    /')
            if [ -n "$_TRACE_TAIL" ]; then
                RESULT="${RESULT}

Trace tail (last 50 lines):
${_TRACE_TAIL}"
            fi
            ;;
        *)
            # Success/recovery path: clean up trace file
            rm -f "$TRACE_FILE"
            ;;
    esac
fi
```

- Crash path: appends last 50 lines of trace (indented) to RESULT, preserves full trace file on disk
- Success/recovery path: cleans up trace file (no `/tmp` accumulation)
- Normal success path (end of script): also cleans up trace file after callback delivery

## Why This Works

The root cause of the undiagnosability is that `set -e` causes the script to exit at some unknown command, the EXIT trap fires, but no diagnostic captures WHICH command failed. `BASH_XTRACEFD` routes the xtrace output to a dedicated file descriptor (fd 9) that writes to a per-PID trace file. This separates the diagnostic trace from stderr entirely, so:

1. Claude-pilot's stderr is unaffected (fd 2 untouched)
2. The trace file persists even after the script exits
3. The EXIT trap can read the trace file and include the tail in the callback result
4. The crash becomes diagnosable from the database alone (`tasks.result` contains the trace)

**Critical limit:** Trace coverage ends at `exec claude-pilot`. If the crash is inside the exec'd subprocess, the trace shows the exec line and stops. This is documented and scoped — subprocess instrumentation is a follow-up ticket.

## Prevention

- **Always register EXIT traps before any command that could fail under `set -e`.** The initial implementation had `exec 9>>` (line 19) running 107 lines before the EXIT trap registration (line 126). If `exec 9>>` failed, the script would exit with no trap and no callback. The fix: `exec 9>> 2>/dev/null || exec 9>/dev/null` degrades gracefully.
- **Use `BASH_XTRACEFD` instead of `exec 2>>` when tracing scripts that exec child processes.** `exec 2>>` clobbers stderr for all children; `BASH_XTRACEFD` keeps fd 2 intact.
- **Use bare assignment (not `export`) for `BASH_XTRACEFD`.** Exporting propagates to exec'd subshells that don't inherit the custom fd, causing silent xtrace loss.
- **Extract shared paths to variables.** The trace file path appeared at 3+ sites initially. Extracting to `TRACE_FILE` prevents path drift across cleanup locations.

## Related Issues

- mika#887 — this fix
- mika#884 — compound doc with full investigation and the original diagnostic recipe
- mika#861 — verify-pipeline label inheritance, BLOCKED by this bug
- mika#879 — mika-arch milestone-grooming skill, BLOCKED by this bug
- mika#870 — callback terminal action guard (downstream safety net)
- mika#871 — parent task leak reaper (last-resort cleanup)
- mika#893 — proposes factorizing dev-pilot/dev-groom handlers (this trace pattern migrates naturally)
- `docs/solutions/dev-loop/dev-pilot-handler-silent-exit-0-pattern-2026-04-29.md` — the investigation compound doc

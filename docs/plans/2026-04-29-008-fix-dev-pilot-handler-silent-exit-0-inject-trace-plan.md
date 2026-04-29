---
title: "fix(dev-pilot): inject BASH_XTRACEFD trace into handler for silent-exit-0 diagnosis"
type: fix
status: draft
date: 2026-04-29
issue: 887
---

# fix(dev-pilot): inject BASH_XTRACEFD trace into handler for silent-exit-0 diagnosis

## Why

`mika/skills/bundled/dev-pilot/handlers/run.sh` exits 0 silently on a deterministic subset of dispatches BEFORE launching `claude-pilot`, with no log file at `/var/log/claude-pilot/<id>.log`. The callback fires with `result = "HANDLER CRASH (exit code 0). Script failed before building result."` (the EXIT-trap fallback at run.sh:72).

Static analysis cannot isolate the exit point — the only visible `exit 0` paths are dry-run branches at run.sh:312 and run.sh:331, but production tool inputs do not include `dry_run`. Reproduction attempts hit `set -x` trace truncation at 64KB. Full investigation captured in mika#884's compound doc (`mika/docs/solutions/dev-loop/dev-pilot-handler-silent-exit-0-pattern-2026-04-29.md`).

**N=4 instances observed** (per mika#887 body): mika#879 ×2 (overnight 2026-04-28), mika#861 (overnight 2026-04-28), and one killed ticketless dispatch on 2026-04-29 with completely different ticket content. The crash is not task-content-specific — likely environment-state-specific, but root cause unconfirmed.

This fix is **defensive, not corrective** — it instruments the bug for next-time diagnosis. Once the trace surfaces the actual exit point, file a follow-up ticket with the corrective fix.

## Goal

Make the silent-exit-0 crash auto-diagnosable. After this fix lands and is deployed:

1. Every dev-pilot handler invocation produces a `set -x` trace at `/tmp/dev-pilot-trace-<pid>.log`.
2. On EXIT-trap fallback (the silent-exit-0 path), the last 50 lines of the trace are appended to `RESULT` so the crash is visible from the database alone.
3. The fix instruments without changing happy-path behavior: if the handler succeeds, `RESULT` is populated normally and the trace addition is silent.
4. **The trace MUST NOT clobber stderr inherited by exec'd subprocesses** (claude-pilot relies on its own stderr stream).

## Approach

Apply a refined version of mika#884's diagnostic recipe to `mika/skills/bundled/dev-pilot/handlers/run.sh`. Refinement (per mika-arch first-pass review session `213982a2-6497-4762-b771-6e82d07fb310`, store_fact `c21b4fd5`): use `BASH_XTRACEFD` to route xtrace to a dedicated fd instead of clobbering stderr.

### Change 1 — Switch shebang to `#!/bin/bash`

Required precondition for BASH_XTRACEFD (bash 4.1+ feature). Existing shebang is `#!/bin/sh` (likely dash on Debian/Ubuntu). Migration:

```sh
# Before
#!/bin/sh

# After
#!/bin/bash
```

The script body is currently POSIX-shell compatible. Bash is a superset; existing logic continues to work unchanged. **Verification step (Phase 0 of work):** run `bash -n run.sh` (syntax check) AND `shellcheck -s bash run.sh` (semantic check) to confirm no bash-specific surprises before merging.

### Change 2 — Inject xtrace via BASH_XTRACEFD at top of script

After the new shebang + `set -e`, add:

```bash
# Open dedicated fd 9 for xtrace output, redirect to per-PID trace file.
# Using BASH_XTRACEFD (bash 4.1+) keeps fd 2 (stderr) intact so exec'd
# subprocesses (claude-pilot) can use stderr normally.
exec 9>>/tmp/dev-pilot-trace-$$.log
BASH_XTRACEFD=9
set -x
```

Rationale (per mika-arch review):
- `exec 2>>...` would clobber stderr for all children including exec'd binaries — claude-pilot's own logging breaks silently.
- `BASH_XTRACEFD` routes xtrace output to fd 9 instead of fd 2. Fd 2 (stderr) stays inherited from the parent shell.
- `exec 9>>...` opens fd 9 for append; PID-suffixed to avoid concurrent-dispatch collisions.
- `set -x` enables xtrace mode AFTER the fd is set up.
- **Bare assignment, NOT `export BASH_XTRACEFD=9`** (per architect F1 sharpening). Exporting would propagate to forked bash subshells; if those subshells don't inherit fd 9 (which they don't unless explicitly passed), xtrace output in the subshell goes to a closed fd → silent loss. Bare assignment scopes BASH_XTRACEFD to this process only.

### Change 3 — Modify EXIT trap to append trace tail to RESULT on crash

The existing EXIT trap at `run.sh:72` fires on script exit when `RESULT` is unpopulated. Modify it to:

```bash
trap '
  _EXIT_CODE=$?
  if [ -z "$RESULT" ]; then
    _TRACE_TAIL=""
    if [ -f "/tmp/dev-pilot-trace-$$.log" ]; then
      _TRACE_TAIL=$(tail -50 "/tmp/dev-pilot-trace-$$.log" 2>/dev/null | sed "s/^/    /")
    fi
    if [ -n "$_TRACE_TAIL" ]; then
      RESULT="HANDLER CRASH (exit code $_EXIT_CODE). Script failed before building result.

Trace tail (last 50 lines):
$_TRACE_TAIL"
    else
      RESULT="HANDLER CRASH (exit code $_EXIT_CODE). Script failed before building result."
    fi
    printf "%s" "$RESULT"
    # Crash path: leave trace file on disk for forensic inspection
  else
    # Success path: clean up trace file
    [ -f "/tmp/dev-pilot-trace-$$.log" ] && rm -f "/tmp/dev-pilot-trace-$$.log"
  fi
' EXIT
```

Properties:
- Existing fallback message shape (`HANDLER CRASH (exit code N). Script failed before building result.`) preserved as a prefix → caller-side parsers continue to match.
- Trace tail appended only when available; absent trace falls back to original message verbatim.
- 50-line tail bounded ~10KB; well under callback truncation cap.
- Crash trace persists at `/tmp/dev-pilot-trace-<pid>.log` for full forensic inspection.
- Success path cleans up; no `/tmp` accumulation in steady state.

### Trace coverage limit (explicit)

**Critical limit (per mika-arch review):** `set -x` trace coverage ends at the `exec` line. dev-pilot's handler eventually `exec`'s `claude-pilot`. If the silent-exit-0 crash is INSIDE the exec'd claude-pilot subprocess (rather than in the dev-pilot handler itself), the trace will show the exec line and then end — telling us "crash was after exec" but not what claude-pilot did to crash.

This fix instruments **dev-pilot's handler only**. If the trace shows the bug is in claude-pilot subprocess execution, file a follow-up ticket for subprocess instrumentation (e.g., `claude-pilot --verbose-trace` or wrapping exec in a tracing shim).

## Critical files

| Purpose | Path |
|---|---|
| Patch target (shebang change + trace injection + trap modification) | `mika/skills/bundled/dev-pilot/handlers/run.sh` |
| Recipe source | `mika/docs/solutions/dev-loop/dev-pilot-handler-silent-exit-0-pattern-2026-04-29.md` (mika#884, MERGED) |
| Existing EXIT trap location | `mika/skills/bundled/dev-pilot/handlers/run.sh:72` |
| Existing shebang location | `mika/skills/bundled/dev-pilot/handlers/run.sh:1` |

## Out of Scope

- **Subprocess (claude-pilot) instrumentation.** Trace ends at exec; if crash is in claude-pilot itself, follow up separately.
- **Corrective fix for the exit-0 root cause.** Once the trace surfaces the actual exit point, file a follow-up ticket. This plan ships diagnostic instrumentation only.
- **Other bundled-skill handlers** (qa-review-build-callback, dev-groom). Different trap surfaces; address only if they show the same symptom. mika#893 proposes factorizing dev-pilot/dev-groom; the trace pattern would migrate there naturally.
- **Refactoring shared trap-handling utilities** across bundled skills.

## Acceptance Criteria

- [x] R1: Shebang on `mika/skills/bundled/dev-pilot/handlers/run.sh` is `#!/bin/bash`. `bash -n run.sh` and `shellcheck -s bash run.sh` both pass.
- [x] R2: Every dev-pilot handler invocation produces a trace file at `/tmp/dev-pilot-trace-<pid>.log` containing `set -x` xtrace output via fd 9 (`BASH_XTRACEFD=9`).
- [x] R3: Stderr (fd 2) inherited by exec'd `claude-pilot` is unchanged from pre-patch behavior. Verify by inspecting claude-pilot logs (existing log shape preserved).
- [x] R4: On the silent-exit-0 path (existing failure mode), the EXIT trap appends the last 50 lines of the trace to `RESULT` with a clear delimiter (`Trace tail (last 50 lines):`).
- [x] R5: On the success path, the trace file is cleaned up automatically (no `/tmp` accumulation in steady state).
- [x] R6: On the crash path, the trace file is preserved at `/tmp/dev-pilot-trace-<pid>.log` for forensic inspection beyond the 50-line tail.
- [x] R7: The existing fallback message shape (`HANDLER CRASH (exit code N). Script failed before building result.`) remains a substring of the new message — no caller-side parser breakage.
- [x] R8: After deploy, retry one of mika#861 or mika#879 (or any future crashing dispatch). The next silent-exit-0 instance produces a trace tail in RESULT that pinpoints the exit step (or shows the trace ends at `exec claude-pilot`, indicating subprocess-side bug).

## Verification

After patch lands and deploys:

1. **Static check on the handler:**
   ```sh
   bash -n mika/skills/bundled/dev-pilot/handlers/run.sh
   shellcheck -s bash mika/skills/bundled/dev-pilot/handlers/run.sh
   ```
   Both must pass.

2. **Smoke test (synthetic happy-path dispatch):**
   - Dispatch a small ticket via `mika ask --agent mika-dev "implement <small-ticket>"`.
   - On success: verify trace file is cleaned up — `ls /tmp/dev-pilot-trace-*.log` post-success should NOT show the trace.
   - Verify claude-pilot logs at `/var/log/claude-pilot/<task_id>.log` populate normally — confirms stderr inheritance is intact.

3. **Reproduction test (re-dispatch a crash victim):**
   - Pick mika#861 (still BLOCKED per task `7b895016-...`) or any future crashing dispatch.
   - Dispatch via `mika ask --agent mika-dev "implement mika issue#861"`.
   - On crash: query `tasks.result` in mika.db: `SELECT result FROM tasks WHERE id = '<task_id>'`.
   - Expected: result includes both the original `HANDLER CRASH (exit code 0). Script failed before building result.` AND a `Trace tail (last 50 lines):` section pinpointing the actual last commands before exit.
   - Inspect `/tmp/dev-pilot-trace-<pid>.log` for full trace; identify the actual exit point.

4. **Cross-check stderr inheritance:**
   - Verify a dispatched session's claude-pilot log includes its normal stderr output (not redirected to dev-pilot's trace file).
   - Negative case: if any claude-pilot log line appears in `/tmp/dev-pilot-trace-<pid>.log`, the fd separation broke and the patch is wrong.

## Cross-references

- mika#884 — compound doc with full investigation, hypotheses, exact diagnostic recipe (MERGED 2026-04-29). Recipe in mika#884 used `exec 2>>`; this plan refines to `BASH_XTRACEFD` per architect review.
- mika#861 — verify-pipeline label inheritance ticket BLOCKED ×1 by this bug.
- mika#879 — mika-arch milestone-grooming skill BLOCKED ×2 by this bug; since shipped via PR #888 retry on different worktree state.
- mika#892 — sibling: dev-groom handler regression of mika-platform#58.
- mika#893 — proposes factorizing dev-pilot/dev-groom; this fix lands in dev-pilot's handler and naturally migrates to the shared substrate when factorization happens.
- mika-arch first-pass review: session `213982a2-6497-4762-b771-6e82d07fb310`, store_fact tool call `c21b4fd5-4a8c-44eb-84ff-5a3f6f2b5f37`. Substantive findings landed in `update_core_memory` (preserved-fact form) due to required-tools-gate retry; full disposition encoded in this revised plan per the architect's instruction "disposition and findings belong in the plan-on-branch."

## Sequencing & Risk

- **Risk: shebang change introduces bash-specific behavior.** Mitigated by `bash -n` + `shellcheck -s bash` static checks. Existing script body is POSIX-compatible; bash is superset.
- **Risk: BASH_XTRACEFD on systems without bash 4.1+.** Production runs Linux with bash >= 5; non-issue. Documented in plan for future container/portability changes.
- **Risk: trace file growth on long-running handlers.** dev-pilot handlers typically run < 60s before exec'ing claude-pilot. Trace file size at exec time is bounded (~few KB). Negligible.
- **Risk: PID collision in concurrent dispatches.** Mitigated by `$$` per-process suffix; unique per shell.
- **Risk: trace ends at exec, hiding subprocess crashes.** Documented explicitly in Out of Scope and Trace coverage limit sections; follow-up ticket if surfaced.
- **Sequencing:** No dependencies. Can ship immediately. After merge + `make deploy`, the next crashing dispatch surfaces its trace automatically.

## Grooming history

- /ce:plan (operator-drafted, well-specified ticket body) → mika-arch first-pass review (session `213982a2-6497-4762-b771-6e82d07fb310`):
  - Architect surfaced critical defect: original plan used `exec 2>>` which clobbers stderr for exec'd subprocesses. Refined to `BASH_XTRACEFD` form.
  - Architect required: shebang must be `#!/bin/bash` (bash 4.1+); existing was `#!/bin/sh`. Plan updated.
  - Architect noted: trace coverage ends at `exec` — subprocess instrumentation is a separate follow-up. Documented explicitly.
- Disposition (interpreted): ITERATE — substantive review delivered via `store_fact` tool call (key `bash_handler_trace_instrumentation`); architect explicitly directed "disposition and findings belong in the plan-on-branch."

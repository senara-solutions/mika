---
ticket: mika#749
branch: feat/749/structured-cancel-discriminator
status: active
date: 2026-06-13
origin: https://github.com/senara-solutions/mika/issues/749
execution: code
---

# Plan: structured CANCELLED_BY_OPERATOR discriminator (mika#749)

## Dependency status (verified)

**mika#743 (structural group-signaling fix) is CLOSED** — verified via `gh issue view 743 --json state,closedAt` returning `state: CLOSED, closedAt: 2026-05-16T11:12:50Z`. The TERM trap / `killpg` foundation this ticket extends is in place. Clear to dispatch.

(Note for groomers: the first-pass `gh_read` returned an incorrect "OPEN" state — likely a tool hallucination, see mika feedback on `mika_dev_llm_fabricates_tool_errors`. Operator-verified ground truth via direct `gh issue view` confirms CLOSED.)

## Problem frame

Today, a `cancel_task` invocation and a spontaneous handler crash deliver **identical-looking callbacks** to mika-dev. mika-dev has no way to tell them apart. A "crashed" callback might correctly trigger a retry path, while a "cancelled" callback should NOT (operator already made the decision). The two paths are conflated.

#743 made signal-propagation correct (whole process group dies on cancel). #749 makes the resulting callback **semantically** correct — distinguishing cancel from crash so mika-dev's downstream state transitions are right.

## Resolution of design choices (referenced from issue body)

The issue body already commits to:

- **Reason-file location:** `${STDOUT_FILE}.reason` — co-located with existing stdout/stderr tempfiles. Lifecycle matches them (EXIT trap cleans up alongside other tempfiles). No new directory.
- **Two discriminators:**
  - `STATUS=CANCELLED_BY_OPERATOR` — `cancel_task` pre-writes before sending SIGTERM
  - `STATUS=CANCELLED_BY_SIGNAL` — TERM trap self-writes as belt-and-suspenders (if cancel_task's pre-write was skipped or signal came from another source)
- **Backward compatibility:** the consumer (mika-dev parser) checks `STATUS=CANCELLED_BY_OPERATOR` and `STATUS=CANCELLED_BY_SIGNAL` first. Absence → falls through to existing `HANDLER CRASH` / success paths. Old callbacks without `STATUS=` prefixes hit existing branches unchanged. No behavior regression for non-cancel exits.

## Scope boundaries

- Writer side (cancel-time + signal-time): the TERM trap in `skills/bundled/_shared/dispatch-lib.sh` extends to "write-then-exit" — writes `STATUS=CANCELLED_BY_SIGNAL` to `${STDOUT_FILE}.reason` before exiting. Also: `cancel_task` in `crates/mika-agent/src/tools/cancel_task.rs` pre-writes `STATUS=CANCELLED_BY_OPERATOR` before invoking `killpg` (when feasible — depends on cancel_task's access to the reason-file path).
- Reader side: the EXIT trap in `dispatch-lib.sh` reads `${STDOUT_FILE}.reason` if present and prefixes the callback envelope with the discriminator.
- Consumer side: mika-dev's self-dev callback-parsing logic recognizes the two `STATUS=CANCELLED_*` prefixes and takes the cancel-aware code paths.
- **Out of scope:** propagating the discriminator into A2A task state (already-distinct concern); changing `HANDLER CRASH` envelope shape; introducing a new top-level reason taxonomy beyond cancel/signal/crash/success.

## Implementation Units

### U1 — Writer (cancel-time pre-write)

**Goal:** When `cancel_task` cancels a task with an active long-running callback, it pre-writes the reason file before SIGTERM.

**Files:**
- Modify: `crates/mika-agent/src/tools/cancel_task.rs` (the SIGTERM call site introduced by #743)

**Approach:** The reason file path needs to be known to `cancel_task`. Options:

1. **Path-by-convention** — derive `${STDOUT_FILE}.reason` from the callback task's metadata where the stdout file path is stored. Pre-write happens in the same code block as the SIGTERM call.
2. **Path-via-task-metadata** — `spawn_long_running_exec` records the stdout path in callback task metadata; `cancel_task` reads it and writes.

Option 2 is the cleaner shape: the stdout path is already a per-callback piece of state; making it discoverable via task metadata is a minor extension. If option 2 is infeasible (e.g., the stdout path is private to the handler script and not surfaced), fall back to option 1's path-by-convention (handler script and cancel_task both know `STDOUT_FILE` via the same env var contract).

Pre-write content: literal single line `STATUS=CANCELLED_BY_OPERATOR\n`. Write before SIGTERM so the trap, even if it races, sees an existing reason and does NOT overwrite (the writer side checks "if file already exists, skip").

**Test scenarios:**
- **cancel_task with active subprocess:** reason file is written before SIGTERM; file contains literal `STATUS=CANCELLED_BY_OPERATOR\n`.
- **cancel_task without active subprocess:** no reason file written (no subprocess to discriminate for).
- **Reason file write fails (FS error):** SIGTERM still proceeds; failure logs WARN; downstream falls back to `STATUS=CANCELLED_BY_SIGNAL` (the trap-self-write path catches it).

**Verification:** unit test on `cancel_task` paths + integration smoke test using a short-running mock long-running tool.

### U2 — Writer (signal-time TERM trap self-write)

**Goal:** When the dispatch-lib.sh TERM trap fires, if the reason file is missing, write `STATUS=CANCELLED_BY_SIGNAL`.

**Files:**
- Modify: `skills/bundled/_shared/dispatch-lib.sh` (the TERM trap section)

**Approach:** Extend the existing TERM trap (introduced by #743) from `exit 143` to:

```bash
if [ ! -e "${STDOUT_FILE}.reason" ]; then
    echo "STATUS=CANCELLED_BY_SIGNAL" > "${STDOUT_FILE}.reason"
fi
exit 143
```

The `if not exists` check preserves the `CANCELLED_BY_OPERATOR` discriminator when cancel_task wrote it first (U1's pre-write). The two discriminators are mutually exclusive in the reason file.

**Test scenarios:**
- **TERM with no pre-write:** trap writes `STATUS=CANCELLED_BY_SIGNAL`.
- **TERM with `CANCELLED_BY_OPERATOR` already present:** trap does NOT overwrite; reason file remains `CANCELLED_BY_OPERATOR`.
- **TERM under EXIT trap composition:** the EXIT trap (which reads the reason) fires AFTER TERM trap (which wrote it). Both traps coexist per bash trap semantics; verify the read sees the write.

**Verification:** shell unit test via `bats` or similar; manual smoke by sending SIGTERM to a long-running handler.

### U3 — Reader (EXIT trap envelope prefix)

**Goal:** The EXIT trap reads `${STDOUT_FILE}.reason` and prefixes the callback envelope.

**Files:**
- Modify: `skills/bundled/_shared/dispatch-lib.sh` (the EXIT trap section, around line 24-60 per current source — the `RESULT=` construction block)

**Approach:** Before constructing the existing `RESULT=` envelope, read the reason file if present:

```bash
_REASON=""
if [ -f "${STDOUT_FILE}.reason" ]; then
    _REASON=$(cat "${STDOUT_FILE}.reason" 2>/dev/null || true)
fi

# Construct envelope
if [ -n "${_REASON}" ]; then
    RESULT="${_REASON}

Original exit code: ${_EXIT_CODE}
${RESULT}"
fi
```

The reason prefix is prepended to whatever RESULT was already assembled (crash envelope or success). The downstream mika-dev parser sees the `STATUS=` prefix first and branches accordingly.

The EXIT trap's existing `rm -f "${_DISPOSITION_FUZZY_FILE:-}"` cleanup line gains a sibling cleanup for the reason file (after RESULT is finalized but before the trap exits).

**Test scenarios:**
- **Reason file present (`CANCELLED_BY_OPERATOR`):** envelope is prefixed with `STATUS=CANCELLED_BY_OPERATOR\nOriginal exit code: 143\n<existing RESULT>`.
- **Reason file present (`CANCELLED_BY_SIGNAL`):** envelope is prefixed with `STATUS=CANCELLED_BY_SIGNAL\n<...>`.
- **Reason file absent:** envelope is unchanged (existing behavior preserved).
- **Reason file cleanup:** after envelope delivered, reason file is rm'd.

**Verification:** shell unit test on EXIT trap with mocked reason file + manual smoke.

### U4 — Consumer (mika-dev callback parser)

**Goal:** mika-dev's self-dev callback-parsing recognizes the two new discriminators and triggers cancel-aware code paths.

**Files:**
- Modify: `skills/bundled/self-dev-callback/system_prompt.md` (the callback-parsing rules) and/or any Rust-side parsing code

**Approach:** Add prompt-level recognition rules:

> **Cancel discriminator (mika#749):** If the callback `RESULT` starts with:
> - `STATUS=CANCELLED_BY_OPERATOR`: operator explicitly cancelled this dispatch. Do NOT retry. Mark the parent task `cancelled` with metadata key `cancelled_reason=operator_cancel`. Notify operator: "Dispatch cancelled by operator — not retrying. Issue X status unchanged."
> - `STATUS=CANCELLED_BY_SIGNAL`: dispatch was terminated by signal (potentially operator-initiated cancel without the pre-write path, or external signal). Treat as operator-cancel for retry-decision purposes (do NOT retry); mark `cancelled_reason=signal_cancel` for forensics distinction.
> - Anything else: existing `HANDLER CRASH` / success-completion paths fire unchanged.

If there is a structural counterpart in Rust (e.g., a callback-classification helper), update it to match.

**Test scenarios:**
- **CANCELLED_BY_OPERATOR callback:** mika-dev does NOT call `run_claude_pilot` again; calls `update_task_status` with `cancelled` + reason; sends notification.
- **CANCELLED_BY_SIGNAL callback:** same — no retry, status cancelled, reason `signal_cancel`.
- **HANDLER CRASH callback:** existing retry path fires (current behavior preserved).
- **Success callback:** existing completion path fires (current behavior preserved).

**Verification:** eval-harness scenario in `tests/eval/grounding_regressions/` mirroring the cancel/crash distinction (a "cancel callback → no retry" assertion). Smoke test: dispatch a long-running task, cancel it via `cancel_task`, verify the next mika-dev turn does NOT spawn a new dispatch.

### U5 — Composition with mika#856 (cancelled → in_progress)

**Goal:** Verify the new discriminator composes correctly with #856's cancel-and-retry path (also queued).

**Files:**
- Read-only verification (no code change)

**Approach:** Document the composition in the plan; no code change unless a real conflict is found.

When #856 ships, a cancelled task can revert to `in_progress`. If a `cancelled_reason` metadata was set by #749's consumer path, it persists after revert (per #856's plan: leave `cancelled_reason` in metadata as audit trail). A subsequent `run_claude_pilot` dispatch on the reverted task creates **fresh tempfiles** (new `STDOUT_FILE` path) — old reason files from prior cancellations cannot collide because tempfile paths are per-dispatch-unique. The `cancelled_reason` metadata is informational; the active state machine is the `tasks.status` column.

Conclusion: #749 and #856 compose without coupling. No coordination commit needed.

**Verification:** post-implementation smoke after both #749 and #856 ship — cancel a task, revert it, re-dispatch, confirm no reason-file or metadata interference.

### U6 — Docs update

**Goal:** Document the discriminator protocol.

**Files:**
- Modify: `crates/mika-agent/CLAUDE.md` § Exec Handlers — add a note on the reason-file protocol
- Modify: `skills/bundled/_shared/dispatch-lib.sh` header comments — document the contract

**Approach:** Short additions explaining the writer/reader/consumer roles and the file path convention.

**Verification:** manual read.

## Dependencies / sequencing

- U1 (cancel_task pre-write) and U2 (TERM trap self-write) are co-required for the writer side
- U3 (EXIT trap read + envelope prefix) depends on U1/U2 producing the reason file
- U4 (mika-dev consumer) depends on U3 emitting the prefixed envelope
- U5 (verification) is a checklist item
- U6 (docs) ships last

## Patterns to follow (cross-cutting)

- `skills/bundled/_shared/dispatch-lib.sh:24-60` — existing EXIT trap and RESULT construction shape
- `crates/mika-agent/src/tools/cancel_task.rs` — the SIGTERM call site from #743
- `skills/bundled/self-dev-callback/system_prompt.md` — existing callback parsing rules (the new prefix rules slot in as a new section)
- `tests/eval/grounding_regressions/` — eval scenario pattern for behavioral assertions

## Verification (top-level)

- `cargo test -p mika-agent tools::cancel_task::tests` — passes (existing + new tests)
- `shellcheck skills/bundled/_shared/dispatch-lib.sh` clean
- `cargo clippy --workspace` clean
- New eval-harness scenario passes: cancel callback → no retry
- Manual smoke: dispatch + cancel via `cancel_task`, verify mika-dev does not retry

## Risk / known unknowns

- **Race between cancel_task's pre-write and the TERM trap's self-write.** Handled by the U2 `if not exists` check — whichever writes first wins; the other is a no-op.
- **Reason file leaks if EXIT trap doesn't fire.** If the bash process is `kill -9`'d (SIGKILL, not catchable), the EXIT trap doesn't run and the reason file persists in `/tmp/`. Acceptable — tempfile cleanup runs at OS level; the leftover file is small and named after a one-shot tempfile.
- **Metadata key conflict with #856.** Both tickets use `cancelled_reason` in task metadata. #856's plan leaves it as audit trail; #749 writes specific values (`operator_cancel`, `signal_cancel`). The semantics align: it's the cancellation source.

## Out-of-scope (explicit)

- A2A `Canceled` state discriminator extension (separate spec change).
- Changing the `HANDLER CRASH` envelope shape (existing path unchanged).
- New cancel taxonomy beyond operator/signal (e.g., `CANCELLED_BY_TIMEOUT`, `CANCELLED_BY_QUOTA`) — those are follow-up tickets if needed.
- Persisting the cancel reason to a dedicated audit table (audit_events row from existing transitions already covers the basics).

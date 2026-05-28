---
title: "feat: Allow cancelled tasks to return to in_progress"
type: feat
status: active
date: 2026-05-28
issue: mika#856
---

# feat: Allow cancelled tasks to return to in_progress

## Overview

Open a single outbound edge from `cancelled` to `in_progress` in the task status state machine. This lets users retry a cancelled task by reusing the original row instead of creating a duplicate.

## Problem Frame

Reverting a cancelled task currently creates a brand-new task row because `cancelled` is a terminal state with no outbound transitions. One "cancel and retry" cycle produces three rows (original, cancelled record, new duplicate) — graveyard noise for a single logical action.

## Requirements Trace

- R1. A cancelled task can be set back to `in_progress` (ticket AC 1)
- R2. The transition mutates the existing row only — no new task row is created (ticket AC 2)
- R3. Status history (via `audit_events`) reflects the transition cleanly (ticket AC 3)

## Scope Boundaries

- Only the `cancelled → in_progress` edge is added. No other outbound edges from `cancelled`.
- `completed` remains fully terminal (zero outbound edges).
- No schema migration — the SQLite CHECK constraint already permits any valid status string; the guard is purely in Rust code.
- Out of scope: redefining cancel semantics broadly; process kill on cancel (per ticket).

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/tools/update_task_status.rs` — sole location of the `VALID_TRANSITIONS` const array (line 17-26), `is_valid_transition()`, `allowed_transitions()`, tool description, and all transition tests
- `blocked → in_progress` is the existing precedent for a "resume" transition — same pattern, same validation path
- Audit logging at line 244-255 already captures all status transitions generically; no audit-specific changes needed (satisfies R3 automatically)

### Behavior Change: Terminal-State Metadata Fallback (#617)

The `#617` metadata fallback (line 216) fires when `allowed_transitions().is_empty()` — i.e., the current state has zero outbound edges. After this change, `cancelled` has one edge (`in_progress`), so the fallback no longer fires for cancelled tasks.

**Impact:** A late-arriving callback that tries e.g. `cancelled → completed` with metadata will now receive a normal "Cannot transition" error (listing `in_progress` as the only valid target) instead of silently writing metadata and keeping the status. This is an improvement — if a callback arrives on a cancelled task, it should either revive it (→ `in_progress`) or be rejected. The silent-swallow behavior was a workaround for fully terminal states.

**Test change:** `test_terminal_metadata_fallback_cancelled_with_metadata` currently asserts that `cancelled → in_progress` with metadata writes metadata and keeps status. After this change, that transition is valid and should succeed as a real status change.

## Key Technical Decisions

- **One edge, not multiple:** Only `cancelled → in_progress` is opened. Transitions like `cancelled → blocked` or `cancelled → completed` remain invalid. A cancelled task must go through `in_progress` before reaching any other state, same as the `blocked → in_progress` pattern.
- **No `is_terminal_status()` helper needed:** The metadata fallback's `allowed.is_empty()` check naturally adjusts. `completed` (zero edges) retains the fallback; `cancelled` (one edge) drops it. This is correct behavior.
- **Tool description update:** The description currently says "Completed and cancelled are terminal." It needs to reflect that cancelled can now transition to `in_progress`.

## Implementation Units

- [ ] **Unit 1: Open the cancelled → in_progress transition**

**Goal:** Add the single outbound edge from `cancelled` to `in_progress` in the state machine, update the tool description and doc comment.

**Requirements:** R1, R2

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/tools/update_task_status.rs`
- Test: `crates/mika-agent/src/tools/update_task_status.rs` (inline `mod tests`)

**Approach:**
- Change `("cancelled", &[])` to `("cancelled", &["in_progress"])` in `VALID_TRANSITIONS`
- Update the doc comment above `VALID_TRANSITIONS` to reflect that only `completed` is fully terminal
- Update the tool description string (line 60-68) to document that cancelled can transition to `in_progress`

**Patterns to follow:**
- The existing `("blocked", &["in_progress", "completed", "cancelled"])` entry is the pattern — `cancelled` gets the same shape with a single target

**Test scenarios:**
- Happy path: create task → cancel → update to `in_progress` → assert success, same task ID, status is `in_progress`
- Happy path: `cancelled → in_progress` with metadata → both status and metadata updated
- Happy path: full round-trip: create → `in_progress` → `cancelled` → `in_progress` → `completed` — verify same row throughout
- Edge case: `cancelled → completed` rejected (only `in_progress` is valid from cancelled)
- Edge case: `cancelled → blocked` rejected
- Edge case: `cancelled → pending` rejected
- Edge case: `cancelled → in_progress` with note → audit event recorded with note
- Integration: `cancelled → completed` with metadata — now rejected (no metadata fallback), error lists valid transitions
- Update existing test `test_terminal_state_cannot_transition`: split cancelled assertions — `in_progress` is now valid; `pending`, `blocked`, `completed` remain rejected (without "terminal state" message — now shows "Valid transitions from 'cancelled': in_progress")
- Update existing test `test_transition_helpers`: `cancelled → in_progress` is now valid; `allowed_transitions("cancelled")` returns `&["in_progress"]` (not empty)
- Update existing test `test_terminal_metadata_fallback_cancelled_with_metadata`: the test currently expects metadata-only write with status unchanged. After this change, the `cancelled → in_progress` transition is VALID, so the test should assert a successful status change to `in_progress` AND metadata written

**Verification:**
- `cargo test -p mika-agent -- update_task_status` passes with all updated and new assertions
- `cargo clippy -p mika-agent` clean

- [ ] **Unit 2: Update documentation references**

**Goal:** Update CLAUDE.md and any other docs that reference the task state machine to reflect the new transition.

**Requirements:** R1

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/CLAUDE.md` (Status transition state machine paragraph)

**Approach:**
- Update the state machine summary: `cancelled` is no longer terminal — it can transition to `in_progress`
- Update the "Terminal states" parenthetical to say only `completed` is terminal
- Keep the documentation concise — this is a small addition to existing text

**Test expectation:** none — documentation only

**Verification:**
- The state machine description accurately matches the code

## System-Wide Impact

- **Interaction graph:** The `update_task_status` tool is the sole entry point for status transitions. No callbacks, middleware, or observers change behavior — they all flow through the same `is_valid_transition()` check.
- **Error propagation:** No change — the tool's error/success response shape is unchanged.
- **State lifecycle risks:** The metadata fallback (#617) for cancelled tasks changes from "silent metadata write" to "normal transition or rejection." This is an improvement (see Key Technical Decisions).
- **API surface parity:** The tool's JSON schema already includes all status values. No dashboard, gateway, or A2A changes needed.
- **Unchanged invariants:** `completed` remains fully terminal. The dispatch-readiness guard (check 1) allows `pending` and `in_progress` — a task revived from `cancelled` to `in_progress` naturally becomes dispatchable again. The orphaned parent reaper, parent auto-completer, and callback watchdog are unaffected (they key on specific status + trigger_type + metadata patterns, not on the transition table).

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Late-arriving callbacks lose metadata on cancelled tasks when targeting non-`in_progress` status | Acceptable — the callback should either revive (→ in_progress) or not. Silent metadata writes on cancelled tasks were a workaround, not a feature. |

## Sources & References

- Related issue: [mika#856](https://github.com/senara-solutions/mika/issues/856)
- Related code: `crates/mika-agent/src/tools/update_task_status.rs`
- Precedent: `blocked → in_progress` transition (existing "resume" pattern)
- Related: #617 terminal-state metadata fallback (behavior change documented above)

---
title: "fix(task-engine): auto-complete parent on callback success — close the success-side handoff gap (mika#1162)"
type: fix
status: active
date: 2026-05-17
---

# fix(task-engine): auto-complete parent on callback success

## Overview

When an autonomous-loop callback child completes with `status: completed` and emits a `PR:` line (indicating success), today's engine extracts the metadata and runs the silent agent turn — but if the silent turn fails to call `update_task_status` on the parent (timeout, max-steps continuation that drops the call, transport error, etc.), the parent remains `in_progress` indefinitely. The dispatch slot stays wedged and anti-cascade can't release the next deferred dispatch.

This fix adds a structural backstop sibling to the existing `try_promote_parent_on_retry_success` (mika#958) and `reap_orphaned_parent_tasks` (mika#871, #1118, #1126) paths: when a callback delivers with a `pr_url` and the parent is still `in_progress`, the engine auto-transitions the parent to `completed`. The existing retry-promoter handled `failed → completed`; this fix handles `in_progress → completed`. Together with the reaper (`in_progress → failed` when no pr_url), the engine now covers every callback-outcome × parent-state combination.

## Problem Frame

**Canonical incident (from issue body, mika#1162):** mika#1158 dispatch on 2026-05-16. Parent task `2fa08519` created 21:06Z. Callback `56bda528` ran claude-pilot for 181 turns / $20.82 / 2391500ms. Callback transitioned to `status: completed` with `subtype: success` at ~21:43Z; PR #1160 opened. Parent stayed `in_progress` for 2+ hours, blocking 4 downstream dispatches. Operator manually cancelled at 23:43Z.

**Why prior fixes don't cover it:**

- mika#871, #1118 (reaper-child-class): reaper recognizes `dispatch_class` on orphaned children. Reaper only handles the no-pr_url path → marks parent `failed`. Today: `crates/mika-agent/src/db.rs:5502-5503` (`parent.metadata IS NULL OR pr_url IS NULL` clause).
- mika#1126 (reaper-on-groom-race): TOCTOU guard for groom-class children. Same scope as #871 — failure path only.
- mika#958 (`try_promote_parent_on_retry_success`): promotes `failed → completed` AFTER reaper fires when a retry succeeds (`crates/mika-agent/src/task_engine/dispatcher.rs:1261-1345`). Does NOT handle the `in_progress → completed` direct-success case.

**Control group (callbacks that DID transition cleanly):** mika#1143, #1145, #1147, #1156, #1157, #1159 all completed with PRs and their parents transitioned cleanly. The likely difference for mika#1158 is the long callback duration (~40 minutes) combined with active relay drift (mika#1161) — both increase the probability that the silent turn following callback delivery hits a transport or deadline failure before calling `update_task_status`. The exact causal chain is not recoverable from logs, but the structural gap is reproducible: any path that delivers a callback with `pr_url` but fails to run the parent's `update_task_status` will wedge.

**The hand-off path that's broken** (per issue body, confirmed by code reading): callback delivers → `try_extract_callback_metadata` writes `pr_url` into parent metadata → `try_promote_parent_on_retry_success` only fires when parent is `failed` (not `in_progress`) → silent agent runs and is expected to call `update_task_status` → if it doesn't, the parent stays `in_progress` forever. There is no structural fallback. Today: `crates/mika-agent/src/task_engine/dispatcher.rs:406-411`.

## Requirements Trace

- **R1.** When a callback child task transitions to `status: completed` with `subtype: success` (i.e., a `PR: <url>` line is present in the callback `result`), the parent task auto-transitions from `in_progress` to `completed` without operator or agent intervention. (Issue body §Expected Behavior.)
- **R2.** Anti-cascade releases automatically: the next deferred dispatch in the queue fires within one engine tick cycle (~60s worst case) after the parent transitions, with no manual intervention. (Issue body §Expected Behavior, §Acceptance Criteria.)
- **R3.** Every auto-transition emits a structured audit event named `task_engine.parent_completed_from_callback` (or close equivalent — exact event tool_name TBD; reason field encodes the literal string) so operators can confirm the structural backstop is firing in the autonomous-merge cycles. (Issue body §Acceptance Criteria.)
- **R4.** The fix recovers parents that were wedged BEFORE deploy (pre-existing leaks) without manual intervention — the periodic backstop drains them on the next tick after deploy.
- **R5.** The fix does NOT auto-complete groom-class dispatches (mirroring the reaper's `dispatch_class = 'implement'` filter — groom callbacks don't produce a `pr_url`, so they can't trip this guard anyway, but the dispatch-class filter is a defense-in-depth invariant).
- **R6.** The fix does NOT regress the existing reaper path (mika#871) or the retry-promoter path (mika#958). Existing tests for those paths continue to pass.

## Scope Boundaries

- **Out of scope: completed_with_followups status.** The issue body floats this as an alternative terminal state. Today's terminal states are `completed`/`failed`/`cancelled`/`expired`; introducing a new one would touch every status consumer (dashboard, list_tasks, status filters, etc.) without a clear consumer. Use `completed` — the pr_url field in metadata + downstream GitHub PR state is sufficient for "merge pending" awareness.
- **Out of scope: investigating *why* mika#1158 specifically wedged.** The issue body asks for this; code reading confirms the structural gap explains all such wedges. Naming a specific root cause beyond "the silent turn did not call update_task_status" would require recovering DB and log state from 2026-05-16, which is the operator's call. We document the gap and close it.
- **Out of scope: changing the existing reaper.** The reaper (`reap_orphaned_parent_tasks`) handles the failure path with stable shape and well-understood guards. The new auto-completer is a sibling, not a replacement.
- **Out of scope: prompt-level changes to the silent agent.** The `callback_terminal_action` INTENT_GUARD (#870) and the qa-review / self-dev prompts already require `update_task_status`. Tightening them further is orthogonal — this fix is the *structural* backstop for when prompt-level enforcement fails.

## Context & Research

### Relevant Code and Patterns

- **`crates/mika-agent/src/task_engine/dispatcher.rs:1191-1250`** — `try_extract_callback_metadata`. Writes `claude_pilot.pr_url` (and session_id, turns, cost, duration) to parent metadata on every callback delivery. Pattern to follow: fire-and-forget, best-effort, log on failure.
- **`crates/mika-agent/src/task_engine/dispatcher.rs:1261-1345`** — `try_promote_parent_on_retry_success` (mika#958). Direct sibling: same shape, same call site, but handles `failed → completed` instead of `in_progress → completed`. Our new function mirrors this almost exactly — different status precondition + different audit event reason.
- **`crates/mika-agent/src/task_engine/dispatcher.rs:402-411`** — call site for the two existing post-callback helpers. Our new helper is added here as a third call in the same `if is_callback { ... }` block.
- **`crates/mika-agent/src/task_engine/engine.rs:625-781`** — `reap_orphaned_parent_tasks` (mika#871, #1118, #1126). Periodic backstop pattern: scan DB for candidates, iterate with TOCTOU guards, transition via `update_task_failed`, emit audit event. Our new periodic backstop mirrors this shape — same scan cadence, same guard pattern, but with `update_task_completed` and the inverted `pr_url IS NOT NULL` predicate.
- **`crates/mika-agent/src/db.rs:5482-5524`** — `find_orphaned_parent_tasks` (the reaper's query). Our new query `find_completable_parent_tasks_on_pr_url` is its mirror: identical join shape with `pr_url IS NOT NULL` instead of `IS NULL`.
- **`crates/mika-agent/src/db.rs:4670-4683`** — `update_task_completed`. Already guards on `status IN ('pending', 'in_progress')`, so it's safe to call without a separate terminal-state precheck (matches retry-promoter's reliance on `promote_task_completed`'s guarded WHERE clause).
- **`crates/mika-agent/src/db.rs:1191-1420` (extract_callback_fields)** — already parses the `PR: <url>` line from callback result. Reuse via the existing helper exported in dispatcher.rs.
- **`crates/mika-agent/src/task_engine/engine.rs:235-239`** — tick-loop wiring. Our periodic backstop slots in immediately after `reap_orphaned_parent_tasks` (same cadence, opposite-outcome sibling).

### Institutional Learnings

- **`docs/solutions/architecture-patterns/engine-level-callback-metadata-extraction.md`** — engine-level metadata extraction pattern (mika#376). Codifies "structural backstop fires regardless of LLM behavior" as the design principle for callback handoff reliability. This fix extends the principle from "metadata always captured" to "parent status always transitioned."
- **`docs/solutions/architecture-patterns/callback-task-loop-prevention.md`** — established the convention that engine-level callback handlers are sole-writers of specific status transitions. Our auto-completer follows: it is the SOLE WRITER (alongside `try_promote_parent_on_retry_success`) of "auto-complete on pr_url" transitions.
- **`docs/solutions/959-callback-watchdog-stale-subprocess-detection.md`** — similar shape: tick-loop scanner + grace period + structured audit event. Pattern reuse: same `_GRACE_SECONDS` const, same audit-event-on-error pattern.
- **`docs/solutions/architecture-patterns/work-item-metadata-two-level-shallow-merge.md`** — guides metadata merge semantics. Not directly modified here, but confirms that pr_url already lives at `$.claude_pilot.pr_url` after `try_extract_callback_metadata` runs.

### Coupled Pairs (from CLAUDE.md conventions)

- This fix and the existing reaper (`reap_orphaned_parent_tasks`) are a coupled pair: they share the same scan cadence, the same agent/source/trigger_type/dispatch_class filters, and the same audit-event observability shape. They differ only on the `pr_url` predicate and the target terminal state. Any future change to one MUST consider whether the symmetric change applies to the other.
- This fix and the inline `try_promote_parent_on_retry_success` are a coupled pair: same scope guards (manual + self_dev + dispatch_class = implement), same call site, same pr_url extraction. Different parent-status preconditions (`failed` vs `in_progress`).

## Key Technical Decisions

- **D1: Two-layer fix (inline + periodic backstop), not just one.** The inline path (`try_complete_parent_on_callback_success`) fires immediately on callback delivery — frees the dispatch slot fast (matches R2's "within one tick" requirement). The periodic backstop (`complete_parent_tasks_on_pr_url`) catches crash-recovery cases and pre-deploy wedges (R4). Two layers cost ~50 lines of code; eliminating either leaves a recovery gap. Mirrors the reaper's own multi-layer architecture (mika#871 + #958 retry promoter).
- **D2: Reuse `extract_callback_fields` and `update_task_completed`.** No new parsing logic, no new DB transition primitive. The existing `update_task_completed` already guards `status IN ('pending', 'in_progress')`, which is exactly what we need.
- **D3: Filter on child's `dispatch_class = 'implement'`** (mirror mika#1118's reaper guard). Groom-class callbacks don't emit `PR:` lines, so they can't trip this in practice — but the filter is a defense-in-depth invariant. Same SQL pattern: `COALESCE(child.dispatch_class, 'implement') = 'implement'`.
- **D4: Audit event tool_name = `task_engine_parent_completer`.** Sibling to `task_engine_reaper` and `task_engine_retry_promoter`. The transition reason field uses the literal `parent_completed_from_callback (pr_url: <url>)` so grep against the issue's wording works. Naming follows the convention of the other engine-level structural backstops.
- **D5: Same 600s grace period as the reaper** (`REAPER_GRACE_SECONDS`). The success-side backstop technically doesn't need grace (callback delivered → silent turn done → no further work expected), but reusing the constant keeps the two backstops symmetric and avoids debate over a different magic number. The inline path doesn't use grace at all (fires at delivery time), so the practical wait is "next tick" for the periodic backstop only.
- **D6: Periodic backstop runs at the same tick cadence as the reaper** (every `DB_SCAN_INTERVAL_TICKS = 60` ticks ≈ 60s). One additional call slot in the tick loop. No risk of write contention because `update_task_completed`'s WHERE clause guards `status IN ('pending', 'in_progress')` — concurrent operator cancels or agent updates lose the race cleanly.
- **D7: No new schema column or migration.** All needed data (parent status, child status, dispatch_class, parent.metadata.claude_pilot.pr_url) already exists in the v37 schema.

## Open Questions

### Resolved During Planning

- **Q: Where should the structural backstop live — inline or periodic?** Both, per D1.
- **Q: What audit event name?** `task_engine_parent_completer` (tool_name) with reason `parent_completed_from_callback (pr_url: <url>)` to satisfy R3's grep-against-issue-wording requirement.
- **Q: Should we touch the existing reaper?** No, per scope boundary. Sibling, not replacement.
- **Q: Does the agent's `update_task_status` becoming a no-op after auto-completion break anything?** No. The status-transition state machine (CLAUDE.md `crates/mika-agent/`) already allows metadata writes on terminal tasks (#617). The silent turn can still notify and write follow-up metadata.

### Deferred to Implementation

- **Q: Should the auto-completion result include the audit event trace_id for cross-correlation?** Probably yes — match the reaper's `mika_common::trace::generate_trace_id()` pattern. Verify shape during implementation.

## Implementation Units

- [ ] **Unit 1: DB query `find_completable_parent_tasks_on_pr_url`**

**Goal:** Sibling to `find_orphaned_parent_tasks` in `db.rs`. Returns parents that are `in_progress`, have a delivered callback child with `dispatch_class = 'implement'`, child grace period elapsed, parent metadata contains `pr_url`, and no other active siblings.

**Requirements:** R1, R4.

**Dependencies:** None.

**Files:**
- Modify: `crates/mika-agent/src/db.rs` — add `find_completable_parent_tasks_on_pr_url(agent_id, grace_seconds)` method next to `find_orphaned_parent_tasks` (around line 5482). Reuse `OrphanedParentTask` struct (rename if a more neutral name fits — e.g., `ParentCallbackCandidate` — or add a second struct `CompletableParentTask` with identical shape).
- Modify: `crates/mika-agent/src/async_db.rs` — add async wrapper `find_completable_parent_tasks_on_pr_url` next to `find_orphaned_parent_tasks` (around line 469).
- Test: `crates/mika-agent/src/db.rs` — add `#[cfg(test) mod tests` cases adjacent to the existing `test_find_orphaned_parent_tasks_*` cases (around line 11695).

**Approach:**
- SQL identical to `find_orphaned_parent_tasks` except: `json_extract(parent.metadata, '$.claude_pilot.pr_url') IS NOT NULL` AND `json_extract(parent.metadata, '$.claude_pilot.pr_url') != ''`. Reuse the same agent_id scope, same `COALESCE(child.dispatch_class, 'implement') = 'implement'` filter, same `NOT EXISTS active sibling` guard, same `MIN(child.id)` selection, same `GROUP BY parent.id ORDER BY parent.id`.
- Struct: if reusing `OrphanedParentTask`, rename in code (e.g. `ParentCallbackCandidate`) for clarity; otherwise add `CompletableParentTask { id, agent_id, created_at, callback_task_id }` with identical fields. Decision deferred — implementer judgment based on whether other callers exist (grep first).

**Patterns to follow:**
- `find_orphaned_parent_tasks` at `crates/mika-agent/src/db.rs:5482-5524`.
- Async wrapper pattern at `crates/mika-agent/src/async_db.rs:468-475`.

**Test scenarios:**
- Happy path: parent `in_progress`/self_dev/manual, child callback `delivered` with `dispatch_class='implement'` older than grace, parent has pr_url → returned.
- Edge case: parent has empty-string `pr_url` (`""`) → NOT returned (matches `!= ''` filter).
- Edge case: parent has pr_url but child is `in_progress` (not yet `delivered`) → NOT returned.
- Edge case: parent has pr_url but child grace not elapsed → NOT returned.
- Edge case: parent has pr_url and `dispatch_class = 'groom'` (or any non-implement) → NOT returned (mirrors reaper guard).
- Edge case: parent has pr_url but parent status is `completed` or `failed` → NOT returned.
- Edge case: parent has pr_url but there's another active sibling (`status IN ('pending', 'in_progress')`) → NOT returned.
- Edge case: parent has pr_url and meets all criteria but for a different `agent_id` → NOT returned (scope check).
- Edge case: pre-v34 NULL `dispatch_class` on child → IS returned (COALESCE → 'implement').
- Integration: assert returned `OrphanedParentTask` (or new struct) fields match the underlying row.

**Verification:**
- New tests pass under `cargo test -p mika-agent --lib`.
- Existing `test_find_orphaned_parent_tasks_*` tests continue to pass (no regression on the failure-path query).

- [ ] **Unit 2: Inline auto-completer `try_complete_parent_on_callback_success`**

**Goal:** Add a new helper to `dispatcher.rs` that runs at callback delivery time (alongside `try_extract_callback_metadata` and `try_promote_parent_on_retry_success`) and transitions `in_progress → completed` when the callback delivered with `pr_url`.

**Requirements:** R1, R2, R3, R5.

**Dependencies:** None on Unit 1; can ship first.

**Files:**
- Modify: `crates/mika-agent/src/task_engine/dispatcher.rs` — add `async fn try_complete_parent_on_callback_success(db, task)` next to `try_promote_parent_on_retry_success` (~line 1261). Wire into the `if is_callback { ... }` block at line 406-411 as the third call (after `try_promote_parent_on_retry_success`).
- Test: `crates/mika-agent/src/task_engine/dispatcher.rs` — add `#[cfg(test) mod tests` cases adjacent to existing `test_try_extract_callback_metadata_*` cases (around line 1729).

**Approach:**
- Mirror `try_promote_parent_on_retry_success` exactly:
  1. Extract `parent_id` from `task.parent_task_id` (return early if None).
  2. Fetch parent via `db.get_task_unscoped(&parent_id)`. Return early unless `trigger_type == "manual"` AND `status == "in_progress"` AND `source.as_deref() == Some("self_dev")`.
  3. Check `task.dispatch_class.as_deref().unwrap_or("implement") == "implement"`. Return early if groom-class.
  4. Read `task.result`, run `extract_callback_fields`, extract `claude_pilot.pr_url`. Return early if missing or empty.
  5. Call `db.update_task_completed(&parent_id, Some(&reason))` where `reason = format!("parent_completed_from_callback (pr_url: {pr_url})")`.
  6. On `Ok(true)`: emit audit event with tool_name `task_engine_parent_completer`, from=in_progress, to=completed, reason same as result.
  7. On `Ok(false)`: debug-log (parent already in terminal state — race with operator or sibling completer).
  8. On `Err(e)`: warn-log.
- Wire into call site at line 406-411 (the `if is_callback` block in `dispatch_resume_agent`).

**Patterns to follow:**
- `try_promote_parent_on_retry_success` at `crates/mika-agent/src/task_engine/dispatcher.rs:1261-1345`.
- `try_extract_callback_metadata` at `crates/mika-agent/src/task_engine/dispatcher.rs:1191-1250` for the call-site wiring pattern.

**Test scenarios:**
- Happy path: parent `in_progress`/self_dev/manual, callback has `result` with `PR: https://...` line → after fn returns, parent `status = 'completed'`, `result` contains "parent_completed_from_callback" and the pr_url, audit event row exists with tool_name `task_engine_parent_completer`.
- Edge case: callback `result` has no `PR:` line → no-op (parent stays `in_progress`).
- Edge case: callback `result` has `PR:` line but parent is `failed` → no-op (returned early on status check; preserves `try_promote_parent_on_retry_success` ownership of that state).
- Edge case: callback `result` has `PR:` line but parent is already `completed` → no-op (returned early on status check).
- Edge case: callback `result` has `PR:` line but parent is `cancelled` → no-op (returned early on status check).
- Edge case: callback has no `parent_task_id` → no-op (returned early on None check).
- Edge case: parent exists but `source != 'self_dev'` (e.g. team task tree) → no-op.
- Edge case: child has `dispatch_class = 'groom'` (or anything non-implement) → no-op.
- Edge case: callback `result` has malformed pr_url (empty string after extraction) → no-op.
- Integration scenario: full `dispatch_resume_agent` flow → after calling, parent transitions and the silent agent's subsequent `update_task_status` call is a no-op (returns false). Validates "agent's call becomes harmless no-op" claim from D7.

**Verification:**
- New tests pass.
- After this unit ships, manually trace through the mika#1158 incident pattern: callback delivers with pr_url → fn fires → parent completed → dispatch slot freed → next deferred dispatch fires. (Cannot replay actual incident, but unit + integration tests cover the path.)

- [ ] **Unit 3: Periodic backstop `complete_parent_tasks_on_pr_url`**

**Goal:** Add periodic engine-tick method as a sibling to `reap_orphaned_parent_tasks` that drains pre-existing wedges and crash-recovery cases.

**Requirements:** R4, R2.

**Dependencies:** Unit 1 (uses the DB query).

**Files:**
- Modify: `crates/mika-agent/src/task_engine/engine.rs` — add `async fn complete_parent_tasks_on_pr_url(&self)` method next to `reap_orphaned_parent_tasks` (~line 625). Wire into tick loop at line 239 as the next call after `reap_orphaned_parent_tasks`.
- Test: `crates/mika-agent/src/task_engine/engine.rs` — add `#[cfg(test) mod tests` cases adjacent to existing reaper tests.

**Approach:**
- Iterate `db.find_completable_parent_tasks_on_pr_url(REAPER_GRACE_SECONDS)` (reuse the existing constant for symmetry — D5).
- For each candidate:
  1. Generate trace_id.
  2. Read parent metadata for the pr_url to include in the audit reason.
  3. Call `db.update_task_completed(&parent.id, Some(&reason))` where `reason = format!("parent_completed_from_callback_backstop (pr_url: {pr_url})")` (distinct suffix from the inline path so audit logs distinguish source).
  4. On `Ok(true)`: emit audit event `task_engine_parent_completer` (same tool_name as inline path).
  5. On `Ok(false)`: debug-log (race with inline path or operator cancellation).
  6. On `Err(e)`: warn-log + audit event with error reason (mirror the reaper's F5 audit-event-on-error pattern).
- Surface pre-existing leaks (age > 24h) with a distinct info-log line (mirror the reaper's F6 pattern).

**Patterns to follow:**
- `reap_orphaned_parent_tasks` at `crates/mika-agent/src/task_engine/engine.rs:625-781`.
- Tick-loop wiring at `crates/mika-agent/src/task_engine/engine.rs:235-240`.

**Test scenarios:**
- Happy path: seed DB with a parent matching the completable-candidate criteria → invoke method → parent transitions to `completed`, audit event row exists.
- Edge case: parent already `completed` (race with inline path) → returns `Ok(false)`, debug log emitted, no audit event.
- Edge case: parent matches but has been cancelled → returns `Ok(false)`, debug log.
- Edge case: pre-existing leak (parent created > 24h ago) → reaped with distinct info log.
- Integration with reaper: seed both kinds (one with pr_url for completer, one without for reaper) → run both methods in same tick → each handles its candidate, no cross-contamination.

**Verification:**
- New tests pass.
- Tick-loop call sequence verified: `reap_orphaned_parent_tasks` and `complete_parent_tasks_on_pr_url` run in order, no race.

- [ ] **Unit 4: End-to-end smoke test**

**Goal:** Integration test (in `crates/mika-agent/tests/smoke.rs` or a new dedicated file) that exercises the full dispatch → callback success → tick → parent transition flow.

**Requirements:** R1, R2, R3.

**Dependencies:** Units 1-3.

**Files:**
- Modify: `crates/mika-agent/tests/smoke.rs` or Create: `crates/mika-agent/tests/parent_auto_complete_smoke.rs` — implementer judgment based on existing test density. Prefer extending `smoke.rs` if it has related callback tests; otherwise create a focused file.

**Approach:**
- Use existing test helpers (mock LLM, real `AsyncDatabase`, real `TaskEngine`).
- Seed: create parent task (manual, self_dev, in_progress) with no metadata; create child callback (resume_agent, in_progress, dispatch_class='implement').
- Mark child completed with a result string containing `PR: https://github.com/owner/repo/pull/1234`.
- Path A (inline): invoke `dispatch_resume_agent` on the child → assert parent transitions to `completed` within one call, audit event recorded.
- Path B (periodic backstop): mark child `delivered` (skip the dispatch path), wait grace, invoke `engine.tick()` → assert parent transitions to `completed` after tick, audit event recorded.

**Test scenarios:**
- Inline path: end-to-end happy path.
- Periodic backstop path: end-to-end happy path with crash-recovery shape (no inline call).
- Race: both paths fire in same window → one wins (assertion: parent is `completed`; total audit event count is 1, not 2 — the loser sees `Ok(false)` from `update_task_completed`).
- Anti-cascade: with a deferred dispatch queued behind the wedged parent, assert that after the auto-complete fires, the next periodic scan promotes and dispatches the deferred callback. (Optional — covers R2 at integration level; may need additional fixture scaffolding; if too complex, defer to a follow-up integration test and mark a TODO in the result.)

**Verification:**
- Test passes locally with `cargo test -p mika-agent --test smoke` (or new test file name).

- [ ] **Unit 5: Documentation updates**

**Goal:** Reflect the new auto-completer in CLAUDE.md so future readers understand the full lifecycle.

**Requirements:** R3 (observability cross-reference).

**Dependencies:** Units 1-3.

**Files:**
- Modify: `crates/mika-agent/CLAUDE.md` — update "Orphaned parent reaper" section (search for "Orphaned parent reaper (#871):" — at the time of writing this is the only such section). Add a paragraph documenting `complete_parent_tasks_on_pr_url` as the success-side sibling. Cross-reference the inline path (`try_complete_parent_on_callback_success`).
- Modify: `CLAUDE.md` (root) — update "Unified task engine" section's callback lifecycle bullet to mention the auto-completer if the existing prose covers the reaper. Verify by reading the section first; if it doesn't cover the reaper either, skip and leave the detail to crate-level CLAUDE.md.

**Approach:**
- Match the prose density of the existing "Orphaned parent reaper" paragraph. Mirror sentence structure: detection query, transition target, audit event name.
- Cite mika#1162 the same way the reaper paragraph cites mika#871/#1118/#1126.

**Test scenarios:**
- N/A (documentation change).

**Test expectation: none — pure documentation, no behavioral change.**

**Verification:**
- Markdown lint passes (existing CI hook).
- Documentation review during /ce:review confirms accuracy against the implementation.

## System-Wide Impact

- **Interaction graph:**
  - `dispatch_resume_agent` (dispatcher.rs:301) gains a third post-callback helper call.
  - Engine tick loop (engine.rs:211-258) gains a sibling method to the reaper, same cadence.
  - No new tools, no new MCP/skill surface, no new HTTP endpoints. Purely internal to the task_engine module.

- **Error propagation:**
  - All new helpers are best-effort, fire-and-forget (matches `try_extract_callback_metadata` and `try_promote_parent_on_retry_success`). DB errors warn-log and continue. The silent agent turn still runs regardless.
  - Audit-event write failures warn-log but don't fail the transition (mirror the reaper's F5 pattern).

- **State lifecycle risks:**
  - Race between inline path and periodic backstop: handled by `update_task_completed`'s `WHERE status IN ('pending', 'in_progress')` guard. Whichever fires first wins; the other sees `Ok(false)` and logs at debug level.
  - Race with operator `mika tasks cancel`: cancel transitions parent to `cancelled`; subsequent completer call sees non-eligible status → `Ok(false)` → no-op. Safe.
  - Race with the existing reaper: the two queries are mutually exclusive on `pr_url IS NULL` vs `IS NOT NULL`. No same-row contention.
  - Race with the agent's own `update_task_status` call from the silent turn: the agent's call may fire before or after the inline auto-complete. Either order produces the same terminal state (completed). The agent's late call returns `false` (status not in `('pending', 'in_progress')` anymore) — harmless per CLAUDE.md status-transition state machine and #617 metadata-on-terminal allowance.

- **API surface parity:**
  - No external API change.
  - Internal API: one new public DB method, one new public async wrapper, two new private helpers, one new tick-loop method.

- **Integration coverage:**
  - Unit 4's smoke test exercises both paths (inline + periodic) end-to-end with a real DB.
  - Existing reaper tests provide the inverse-case coverage (no pr_url path) — assert no regression.

- **Unchanged invariants:**
  - The reaper continues to mark parents `failed` when no `pr_url` is present. Same scope, same grace, same audit event tool_name (`task_engine_reaper`).
  - The retry promoter continues to handle the `failed → completed` post-retry case. Same scope.
  - The silent agent turn still runs after the inline auto-complete fires. It just sees the parent as already terminal — its `update_task_status` becomes a no-op (allowed per #617).
  - The `callback_terminal_action` INTENT_GUARD (#870) still requires `update_task_status` + `send_message` before EndTurn. This fix does NOT relax that guard — it's the structural backstop when prompt-level enforcement fails.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Inline auto-complete fires before silent agent meant to do follow-up work that requires `in_progress` status | The silent agent's `update_task_status` and `send_message` calls all work with terminal parent status (per CLAUDE.md state machine + #617). No-op semantics, no error. Tested in Unit 2's integration scenario. |
| Periodic backstop fires concurrently with operator manual cancel | `update_task_completed`'s WHERE clause guards `status IN ('pending', 'in_progress')`. Cancel transitions to `cancelled`, completer sees non-eligible status, returns `Ok(false)`, no-op. |
| New audit event volume floods log retention | One row per autonomous-merge cycle (~10/day at peak). Negligible. |
| Pre-deploy wedge backfill on first tick after deploy floods the log with `task_engine_parent_completer` events | Expected — F6-style pre-existing-leak surfacing (24h+ age threshold) emits distinct info-log line. Operator can identify the cleanup wave in logs. Bounded by candidate count (the wedges that exist). |
| Auto-complete fires on a parent that legitimately should stay `in_progress` (e.g., dev-pilot succeeded but PR still needs operator review before close) | The `completed` status here means "callback succeeded, PR opened, dispatch slot freed." It does NOT mean "PR merged." Downstream PR lifecycle (qa-review, ci_success_handler, pr_merge_with_gate) remains untouched and operates on the PR independently. Parent task completion does not close the GitHub PR or block subsequent webhooks (which would create new tasks anyway). |
| Symmetric drift with the reaper if future commits change one but not the other | Documented as a "coupled pair" in Context & Research and in the inline code comments. Code review (/ce:review) must flag asymmetric changes. |

## Documentation / Operational Notes

- **Compound:** after pipeline completes, write a solution doc at `docs/solutions/architecture-patterns/task-engine-success-side-backstop-pattern-2026-05-17.md` documenting the engine-level-backstop-for-every-outcome principle (extends the existing `engine-level-callback-metadata-extraction.md` pattern doc).
- **Operator note:** After deploy, the first tick after server restart may transition multiple pre-existing wedged parents to `completed`. This is expected — bounded cleanup. Look for the F6-style pre-existing-leak info-log lines for visibility.
- **Observability:** Search `task_engine_parent_completer` in audit_events to confirm the structural backstop is firing in autonomous-merge cycles (R3).

## Sources & References

- **Issue:** mika#1162
- **Canonical incident:** mika#1158 (2026-05-16 wedge, 2+ hours, manually cancelled)
- **Coupled pair (failure path):** mika#871, mika#1118, mika#1126 (orphaned parent reaper)
- **Coupled pair (retry path):** mika#958 (`try_promote_parent_on_retry_success`)
- **Related callback lifecycle:** mika#376 (engine-level callback metadata extraction)
- **Possible co-driver of incident:** mika#1161 (relay drift active during mika#1158 callback)
- Related code:
  - `crates/mika-agent/src/task_engine/dispatcher.rs` — inline call site (line 402-411), retry-promoter pattern (line 1261-1345), metadata-extraction pattern (line 1191-1250), extract_callback_fields (line 1368-1420)
  - `crates/mika-agent/src/task_engine/engine.rs` — tick-loop wiring (line 235-240), reaper sibling pattern (line 625-781)
  - `crates/mika-agent/src/db.rs` — orphaned-parent query (line 5482-5524), update_task_completed (line 4670-4683), promote_task_completed (line 4739-4748)
  - `crates/mika-agent/src/async_db.rs` — async wrapper pattern (line 468-475)

---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
created: 2026-06-28
issue: 1620
deepened: false
---

# fix: Dispatch-classification gate trusts pre-stamped grooming markers

## Goal Capsule

Add a structural cross-check to the dispatch-classification gate so that grooming markers in an issue body are only trusted when a completed `dispatch_class='groom'` task exists for the same issue. Pre-stamped markers from manual `/mika-ask-arch` sessions will no longer fool the gate into dispatching `dev-pilot` on ungroomed tickets.

---

## Summary

The `check_grooming_markers()` gate in `executor.rs` currently performs pure substring matching against the issue body — if the three canonical markers (Branch callout, Plan callout, second-pass GROOMED verdict) are present, the gate passes. It does not verify provenance: markers written by a manual `/mika-ask-arch` session look identical to those written by the autonomous `dev-groom` loop via `dispatch-lib.sh::_write_canonical_callout`. When `MIKA_DISPATCH_BYPASS_GROOMING_CHECK` is removed, this gap becomes a fabrication risk — pre-stamped tickets could reach `dev-pilot` without having been autonomously groomed.

The fix adds a single DB query after the marker check passes: look for a completed groom-class callback task whose `reference_url` matches the target issue. If no such task exists, the gate rejects with a new `dispatch_grooming_not_verified` error. This is the structural approach (option 2 from the issue), which doesn't depend on prompt enforcement.

---

## Problem Frame

- **What's broken:** The gate classifies tickets as IMPLEMENT-class based solely on issue body content, without verifying the grooming was performed by the autonomous loop.
- **When it matters:** When `MIKA_DISPATCH_BYPASS_GROOMING_CHECK` is removed (the intended end-state). Currently masked by the env var bypass.
- **Evidence:** mika#1609 had canonical grooming markers stamped by a manual orchestrator `/mika-ask-arch` session, not by an autonomous `dev-groom` run.

---

## Requirements

- R1. After `check_grooming_markers()` passes, cross-check the tasks table for a completed groom-class task matching the target issue.
- R2. Reject dispatch with a structured error when markers are present but no completed groom task exists.
- R3. The cross-check must be bypass-able via the existing `MIKA_DISPATCH_BYPASS_GROOMING_CHECK` env var (same bypass semantics).
- R4. The cross-check must fail-open when the DB query fails (consistent with the existing fail-open pattern for the no-token case).
- R5. Test coverage for the new gate path (both pass and reject scenarios).

---

## Key Technical Decisions

- **KTD1: DB cross-check only (no provenance tagging).** The issue proposed two complementary approaches: (1) provenance tagging in marker text and (2) DB cross-check. We implement only (2). Provenance tagging relies on prompt enforcement (fragile, per recurring incidents), adds marker-format coupling across `dispatch-lib.sh` and `check_grooming_markers()`, and would require all existing pre-stamped tickets to be re-groomed. The DB cross-check is structural, prompt-independent, and automatically correct for both new and existing tickets.

- **KTD2: Match on `reference_url` with `?phase=groom` suffix.** The autonomous groom flow creates tasks with `reference_url` set to `https://github.com/<owner>/<repo>/issues/<n>?phase=groom` (see `self-dev-webhook-ready-label/system_prompt.md:23`). The cross-check queries for tasks matching this URL pattern. This is more precise than matching on the base issue URL alone.

- **KTD3: Check `dispatch_class='groom'` AND callback status `completed` or `delivered`.** A groom task that crashed, was cancelled, or is still in-progress should not satisfy the gate. Only a successfully completed groom confirms the autonomous loop ran to completion.

- **KTD4: New DB method, not inline SQL.** The query goes into `db.rs` as a named method (`has_completed_groom_for_issue`) for testability and consistency with the existing dispatch-readiness query pattern.

---

## Scope Boundaries

### In scope
- Adding the DB cross-check to `validate_dispatch_readiness()` after the marker check passes
- New DB query method
- New rejection error type `dispatch_grooming_not_verified`
- Test coverage for the new gate

### Out of scope
- Removing `MIKA_DISPATCH_BYPASS_GROOMING_CHECK` (depends on this fix landing first, per issue)
- Modifying the `/mika-ask-arch` flow or marker format
- Provenance tagging in marker text (rejected in KTD1)

### Deferred to Follow-Up Work
- Removing the bypass env var once this fix is validated in production

---

## Verification Contract

1. `cargo test -p mika-agent` passes — no regressions
2. New test: grooming markers present + no completed groom task → dispatch rejected with `dispatch_grooming_not_verified`
3. New test: grooming markers present + completed groom task exists → dispatch proceeds
4. New test: bypass env var skips both marker check AND cross-check
5. `cargo clippy` clean

---

## Implementation Units

### U1. Add `has_completed_groom_for_issue` DB method

**Goal:** Query the tasks table for a completed groom-class callback task matching a specific GitHub issue.

**Requirements:** R1

**Dependencies:** None

**Files:**
- `crates/mika-agent/src/db.rs` — new method
- `crates/mika-agent/src/async_db.rs` — async wrapper

**Approach:** Add a method that takes `agent_id` and the issue's `reference_url` (canonical form: `https://github.com/<owner>/<repo>/issues/<n>`) and queries for tasks where:
- `dispatch_class = 'groom'`
- `status IN ('completed', 'delivered')`
- `reference_url` matches with `?phase=groom` suffix (the discriminator used by the autonomous groom flow)

The query should use `reference_url = ?1 || '?phase=groom'` to construct the expected URL. Return `bool`.

**Patterns to follow:** Existing `has_active_callback_tasks_excluding` pattern in `db.rs` — simple boolean query with agent scoping.

**Test scenarios:**
- No matching task → returns `false`
- Matching completed task with `?phase=groom` suffix → returns `true`
- Matching task with `delivered` status → returns `true`
- Task exists but `pending`/`in_progress`/`failed`/`cancelled` → returns `false`
- Task exists with matching URL but `dispatch_class='implement'` → returns `false`
- Different agent_id → returns `false`

### U2. Integrate cross-check into `validate_dispatch_readiness`

**Goal:** After `check_grooming_markers()` passes, query the DB for a completed groom task. Reject dispatch if none exists.

**Requirements:** R1, R2, R3, R4

**Dependencies:** U1

**Files:**
- `crates/mika-agent/src/skills/executor.rs` — modify grooming-marker gate section

**Approach:** Inside the `if is_dev_pilot && is_issue_type` block, after `check_grooming_markers()` returns an empty `missing` vec (all markers present), add a DB cross-check:

1. Construct the canonical issue URL from the parsed `GitHubRef::Issue` fields: `https://github.com/{owner}/{repo}/issues/{number}`
2. Call `db.has_completed_groom_for_issue(agent_id, &issue_url).await`
3. If the method returns `false`, construct a structured rejection JSON with error `dispatch_grooming_not_verified`, including:
   - `task_id`, `issue` (formatted), `reason` explaining the markers exist but no autonomous groom completed
   - `recovery` suggesting to run `/mika-groom-ticket` or use the bypass env var
4. Call `record_dispatch_rejection()` and return `Err`

The cross-check is positioned AFTER the marker check intentionally — markers are cheap (no DB), so checking them first avoids a DB round-trip on tickets that don't even have markers.

The bypass env var already short-circuits before the marker check, so it automatically skips this cross-check too (R3).

For fail-open on DB errors: wrap the `has_completed_groom_for_issue` call in a match and log a WARN on error, allowing dispatch to proceed (R4). This is consistent with the no-token fail-open behavior.

**Patterns to follow:** The existing `check_grooming_markers` rejection at lines 1090–1111 — same JSON structure, same `record_dispatch_rejection` call.

**Test scenarios:**
- Markers present + no completed groom task → `dispatch_grooming_not_verified` rejection
- Markers present + completed groom task → dispatch proceeds (no rejection from this gate)
- Markers absent → `dispatch_no_grooming_marker` rejection (existing behavior, unchanged)
- Bypass env var set → both checks skipped
- DB error on cross-check → WARN log, dispatch proceeds (fail-open)

### U3. Add eval test for the cross-check gate

**Goal:** Integration-level test coverage for the new gate path.

**Requirements:** R5

**Dependencies:** U1, U2

**Files:**
- `crates/mika-agent/tests/eval/test_dispatch_no_grooming_marker_guard.rs` — new test scenarios

**Approach:** Add test scenarios to the existing grooming-marker guard test file. The tests need to:

1. **Pre-stamped markers rejected:** Create a task with a GitHub issue `reference_url`, mock a fully-groomed issue body response, but do NOT create a completed groom task in the DB. Assert `validate_dispatch_readiness` returns `Err` with `dispatch_grooming_not_verified`.

2. **Autonomous groom passes:** Same setup, but also insert a completed groom-class task with `reference_url` matching the issue + `?phase=groom`. Assert `validate_dispatch_readiness` returns `Ok`.

3. **Bypass skips cross-check:** Set the bypass env var, use pre-stamped markers with no groom task. Assert dispatch proceeds.

**Patterns to follow:** Existing test structure in the same file — `EvalHarness` setup, `MockLlmProvider`, `validate_dispatch_readiness` calls with mock GitHub API responses.

**Test scenarios:**
- Covers R1, R2: pre-stamped markers → rejection
- Covers R1: autonomous markers + completed task → pass
- Covers R3: bypass env var → skip

---

## Definition of Done

- [ ] `has_completed_groom_for_issue` DB method exists and is tested
- [ ] `validate_dispatch_readiness` cross-checks groom task completion after marker check
- [ ] New rejection error `dispatch_grooming_not_verified` with structured JSON
- [ ] Bypass env var skips both marker check and cross-check
- [ ] DB error on cross-check fails open with WARN log
- [ ] All existing tests pass
- [ ] New tests cover pass, reject, and bypass scenarios
- [ ] `cargo clippy` clean

---

## Acceptance criteria

- [ ] When a GitHub issue body contains all three grooming markers but no completed `dispatch_class='groom'` task exists for that issue, `validate_dispatch_readiness` rejects with `dispatch_grooming_not_verified`
- [ ] When a GitHub issue body contains all three grooming markers AND a completed groom task exists, dispatch proceeds normally
- [ ] The existing `MIKA_DISPATCH_BYPASS_GROOMING_CHECK` env var bypasses both the marker check and the new cross-check
- [ ] DB errors during the cross-check are logged at WARN level and dispatch proceeds (fail-open)
- [ ] All existing grooming-marker gate tests continue to pass without modification

---

## Sources & Research

- mika#1620 — Issue describing the fabrication risk
- `crates/mika-agent/src/skills/executor.rs:803-817` — `check_grooming_markers` function
- `crates/mika-agent/src/skills/executor.rs:1043-1140` — grooming-marker gate in `validate_dispatch_readiness`
- `skills/bundled/_shared/dispatch-lib.sh:1828-1937` — `_write_canonical_callout` function
- `skills/bundled/self-dev-webhook-ready-label/system_prompt.md:23` — `?phase=groom` URL discriminator
- `docs/solutions/best-practices/auto-groom-on-dispatch-2026-05-06.md` — auto-groom flow documentation

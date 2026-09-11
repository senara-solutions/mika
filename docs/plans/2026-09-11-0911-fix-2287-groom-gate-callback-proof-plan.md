---
title: Groom Gate Reads Callback Proof - Plan
type: fix
date: 2026-09-11
issue: mika#2287
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# Groom Gate Reads Callback Proof - Plan

**Issue:** senara-solutions/mika#2287
**Branch:** `bug/2287/dispatch-gate-1620-preuve-groom`

## Goal Capsule

- **Objective:** A ticket that was groomed by the autonomous loop dispatches `dev-pilot` again without any bypass flag, and a ticket whose markers were pre-stamped by hand is still refused.
- **Means:** Rewrite the gate `has_completed_groom_for_issue` to read the durable proof on the groom **callback** row instead of a `?phase=groom` suffix on the parent row (KTD1), and make the caller fail-closed on DB error (KTD3).
- **Authority:** Issue mika#2287 body and its two ratification comments (Vincent + Prime, 2026-09-11 09:01) > this plan > existing code comments. The nine Prime conditions in `### Requirements` are non-negotiable; a unit that cannot satisfy one stops and surfaces it.
- **Stop conditions:** Any change that writes to the DB from the gate path (R1). Any branch of the cross-check (gate or its caller arms) that returns "allowed" on a degraded case of the cross-check (R2). Any change to the groom producers (`ready_label_handler.rs`, `dispatcher.rs` auto-fire, `dispatch-lib.sh`) — out of scope.
- **Execution profile:** Rust, `crates/mika-agent`. Test-first on U2 (the red-before proof is a deliverable, not a by-product).
- **Tail ownership:** The PR body carries the red-before output (R4), the rollback statement (R9), and the retention note (R10). Deployment proof (R7, R8) is owned by the operator after merge and is recorded on the issue, not in the PR.

## Product Contract

### Summary

Replace the gate query so that it joins the completed groom callback row (`trigger_type='callback'`, `dispatch_class='groom'`, terminal status, `result` containing `Outcome: PLAN_GROOMED`) to its parent by `parent_task_id`, and matches the parent's `reference_url` against the bare issue URL or its legacy `?phase=groom` form. Flip the caller's DB-error branch from fail-open to fail-closed. Rewrite the existing gate tests around the callback-row proof and add the anti-recursion test that flips the parent groom→implement before asserting the gate passes. Update the four comment sites that still describe the suffix mechanism.

### Problem Frame

Every autonomous `dev-pilot` dispatch is rejected with `dispatch_grooming_not_verified` since `MIKA_DISPATCH_BYPASS_GROOMING_CHECK` was removed from the runtime environment (measured 2026-09-11). The gate (`crates/mika-agent/src/db.rs`, `has_completed_groom_for_issue`) looks for a parent row with `dispatch_class='groom'`, a terminal status, and `reference_url = <issue_url>?phase=groom`. None of the three production groom producers can leave such a row: the structural ready-label handler (`crates/mika-agent/src/server/ready_label_handler.rs`, mika#1572) writes the bare URL; the engine-side auto-fire (`crates/mika-agent/src/task_engine/dispatcher.rs`, `try_dispatch_pilot_after_groom_success`, mika#1614) flips the parent's `dispatch_class` to `implement` before the parent reaches a terminal status; the callback child (`crates/mika-agent/src/skills/executor.rs`, `build_callback_task`) carries `reference_url: None`. mika#1614 and mika#1620 landed the same day (2026-06-28) with incompatible assumptions. The only route that worked was the bypass flag, which was removed on purpose.

The durable proof already exists in the schema: the callback child keeps `dispatch_class='groom'` (derived from the `skill` input, untouched by the flip), reaches `completed` then `delivered`, and its `result` is the dispatch-lib RESULT written by `POST /tasks/{id}/complete` (`crates/mika-agent/src/server/handlers.rs`), which contains the literal `Outcome: PLAN_GROOMED`. The engine already trusts exactly this proof in `try_dispatch_pilot_after_groom_success`.

### Key Decisions

- **Fix the gate, not the producers** (session-settled: user-directed — chosen over direction (a) "make the handler emit a `?phase=groom` proof row": (a) contradicts the single-task-id reuse of mika#1614 and would need reconciling three producers; (b) touches one read-only function). Ratified by Vincent and Prime on the issue, 2026-09-11 09:01. Governs R1, R3, R11.
- **Bypass flag stays removed from the environment.** The code path for `MIKA_DISPATCH_BYPASS_GROOMING_CHECK` is not touched by this plan; its removal is a separate scope. Governs R7, R12.

### Requirements

**Gate semantics**

- R1. The gate path performs no `INSERT`, `UPDATE`, or `DELETE` and mutates no state; it is a single `SELECT`.
- R2. The cross-check and its caller arms refuse dispatch on every degraded case of the cross-check: no matching row, DB error, or a proof pruned by retention. Within the cross-check, no branch yields "allowed" by default. The pre-existing no-GitHub-token skip and the bypass env-var arm sit outside the cross-check and are out of scope (see Scope Boundaries).
- R3. One query serves all groom producers (structural ready-label handler, LLM `self-dev-webhook-ready-label` path, CLI `mika ask --agent mika-dev "groom …"`); there is no per-producer branch. The query accepts the parent `reference_url` in bare form or with the legacy `GROOM_PHASE_SUFFIX`.
- R11. The gate result is independent of the parent's current `dispatch_class` (it may be `implement` after the mika#1614 flip) and of any URL suffix.
- R12. The gate keeps its `agent_id` scope: the callback row must belong to the dispatching agent.

**Tests**

- R4. A blocking anti-recursion test builds a groom parent + completed groom callback pair through the production write API, flips the parent to `implement`, and asserts the gate returns `true`. This test fails on pre-fix code; its failure output is pasted into the PR body.
- R5. No test inserts rows with raw SQL; every proof row is created through `create_task` and completed through `update_task_completed` with a real `Outcome: PLAN_GROOMED` body.
- R13. Negative tests cover: no row → `false`; callback still `pending` → `false`; callback completed with `Outcome: PLAN_ITERATE` → `false`; callback owned by a different agent → `false`; callback whose parent has a different issue URL → `false`; callback with `dispatch_class='implement'` → `false`.
- R14. A positive test covers the legacy `?phase=groom` parent URL → `true`, proving R3.

**Review and delivery**

- R6. The PR asks QA to read the refusal condition first (fail-closed), before any human merge. The PR body states that the PR is DECISION-CORE by perimeter — it edits `crates/mika-agent/src/skills/executor.rs` (dispatch-authority, `crates/mika-agent/src/perimeter/rules.rs`, `docs/gate/perimeter.md`) — so auto-merge is structurally held and the human gate is required. No label is applied; the classification is computed from the diff.
- R9. The PR body names the rollback before merge: "revert = return to the current bug, rail re-blocked, no state corruption".
- R10. The PR body notes the retention case. `prune_completed_tasks` runs with a 30-day retention (`crates/mika-agent/src/task_engine/mod.rs`, `THIRTY_DAYS_SECS`). The proof is the callback row joined to its parent, and `tasks.parent_task_id` is `ON DELETE SET NULL`, so the proof is lost 30 days after whichever of the two rows completed first. The proof is consulted by every `dev-pilot` dispatch on the issue — first dispatch, auto-pull re-drives, and verdict-handler `block[ac]`/`block[ci]` retries (`crates/mika-agent/src/server/verdict_handler.rs`, `validate_dispatch_readiness` call) — so a PR still under review 30 days after grooming loses its automatic fix dispatches. Fail-closed, accepted, to be stated with that blast radius.
- R15. The PR body notes the hand-groomed case. A ticket groomed outside the loop (orchestrator `/mika-groom-ticket`, or a bare `/mika-ask-arch` pre-stamp) has no groom callback row and is refused — this is the ratified intent of Key Decision 1. A `dev-groom` re-dispatch on such a ticket is answered `auto_skipped` / `already_groomed` by dispatch-lib (mika#2012), whose callback result carries no `Outcome: PLAN_GROOMED`, so it never becomes proof. The only exit today is removing the plan from the branch and re-grooming through the loop. The rejection `recovery` text states that exit; it does not name a route that structurally refuses.

**Deployment proof (operator-owned, recorded on the issue)**

- R7. One real autonomous dispatch passes without any bypass, bypass absent during the test. A failure triggers diagnosis, never the flag.
- R8. After a restart at an empty slot, a second consecutive autonomous dispatch passes. Two passes = gate repaired; one pass = signal only. The test ticket must be a groom that was flipped groom→implement, not a groom left in place.
- R16. Tickets the outage parked under `operator-review` (three refused re-drives) do not self-heal after deploy; the operator removes `operator-review` to let them re-enter. Recorded on the issue with R7/R8.

### Scope Boundaries

- Out of scope: the pre-existing no-GitHub-token skip (the `None` arm of the `github_token` match in `validate_dispatch_readiness`, which skips both the marker check and the cross-check — it predates mika#1620); removing the `MIKA_DISPATCH_BYPASS_GROOMING_CHECK` code path; changing `ready_label_handler.rs`, `try_dispatch_pilot_after_groom_success`, `build_callback_task`, or `dispatch-lib.sh`; changing `check_grooming_markers`; widening the eval test `tests/eval/test_dispatch_no_grooming_marker_guard.rs` beyond its comment.

#### Deferred to Follow-Up Work

- Remove the bypass env-var code path once R7/R8 are recorded (separate ticket, evidence-gated).
- Decide whether the `GROOM_PHASE_SUFFIX` producer path (`skills/bundled/self-dev-webhook-ready-label/system_prompt.md`) should stop writing the suffix now that no reader depends on it.
- Decide (Vincent/Prime, decision-core) whether `/mika-groom-ticket` should record a proof row, or whether an operator route should mint one, so architect-reviewed hand-groomed tickets are not permanently refused now that the bypass is gone (R15).
- Decide whether `PLAN_GROOMED` groom callback rows (and their parents) should be exempt from `prune_completed_tasks`, since the proof now bounds every `dev-pilot` dispatch on the issue for its whole life (R10).

### Sources

- `crates/mika-agent/src/db.rs` — `has_completed_groom_for_issue` and its `#1620` test block (`groom_task` helper, six tests).
- `crates/mika-agent/src/async_db.rs` — async wrapper `has_completed_groom_for_issue` (agent-scoped).
- `crates/mika-agent/src/skills/executor.rs` — `validate_dispatch_readiness`: grooming-marker gate, provenance cross-check, fail-open `Err` branch, rejection JSON with `predicate`/`recovery` text; `build_callback_task` (`dispatch_class` from `derive_dispatch_class(skill)`).
- `crates/mika-agent/src/task_engine/dispatcher.rs` — `try_dispatch_pilot_after_groom_success` (the proof already trusted by the engine), the groom→implement flip, and test helper `create_groom_callback_pair` + `test_db()`.
- `crates/mika-agent/src/server/handlers.rs` — `handle_task_complete` writes `result` via `update_task_completed`.
- `crates/mika-agent/src/task_state/tasks.rs` — `GROOM_PHASE_SUFFIX` constant and its doc comment.
- `crates/mika-agent/src/task_engine/mod.rs` — `prune_completed_tasks(THIRTY_DAYS_SECS)`.
- `crates/mika-agent/src/server/verdict_handler.rs` — `block[ac]`/`block[ci]` retries call `validate_dispatch_readiness` (a second gate consumer).
- `crates/mika-agent/src/perimeter/rules.rs`, `docs/gate/perimeter.md` — `executor.rs` is DECISION-CORE (dispatch-authority); the PR is held for the human gate by construction.
- `skills/bundled/_shared/dispatch-lib.sh` — mika#2012 `already_groomed` auto-skip on `dev-groom` re-dispatch when the plan already resolves on the branch.
- `docs/solutions/best-practices/un-label-denforcement-non-declare-echoue-en-silence-2026-09-01.md` — why no ad-hoc label is applied for the review class.
- `docs/solutions/workflow-issues/verdict-writer-and-gate-must-share-one-vocabulary-2026-08-27.md` — the writer/reader vocabulary drift class this bug belongs to.
- `docs/solutions/workflow-issues/ready-label-dispatch-requires-grooming-marker-2026-04-30.md` — why the grooming gate exists.

## Planning Contract

### Key Technical Decisions

- KTD1. **Proof lives on the callback row, joined to the parent by `parent_task_id`.** (session-settled: user-directed — chosen over reading the parent row or an audit event: the callback row is the only row that keeps `dispatch_class='groom'`, reaches a terminal status, and carries the `Outcome: PLAN_GROOMED` text; it is what `try_dispatch_pilot_after_groom_success` already trusts.) Query shape, directional: `SELECT COUNT(*) FROM tasks child JOIN tasks parent ON child.parent_task_id = parent.id WHERE child.agent_id = ?agent AND child.trigger_type = 'callback' AND child.dispatch_class = 'groom' AND child.status IN ('completed','delivered') AND instr(child.result, 'Outcome: PLAN_GROOMED') > 0 AND parent.reference_url IN (?url, ?url || GROOM_PHASE_SUFFIX)`. Governs R1, R3, R11, R12.
- KTD2. **Marker match uses `instr(...) > 0`, not `LIKE`.** SQLite `LIKE` is case-insensitive for ASCII; `instr` is an exact, case-sensitive substring test and mirrors the Rust `r.contains("Outcome: PLAN_GROOMED")` in `dispatcher.rs`. One vocabulary for writer and both readers.
- KTD3. **Caller DB-error branch becomes fail-closed.** The `Err(e)` arm of the cross-check in `validate_dispatch_readiness` returns a `dispatch_check_failed` rejection (same shape as the neighbouring fail-closed branches for global-state check and issue-body fetch) after `record_dispatch_rejection`. Governs R2.
- KTD4. **Anti-recursion test lives in `dispatcher.rs`'s test module, next to `create_groom_callback_pair`.** The `db.rs` test module is synchronous (zero `tokio::test`), while the helper is `async` over `AsyncDatabase`. Placing the test beside the helper needs no visibility promotion and exercises the production entry point (`AsyncDatabase::has_completed_groom_for_issue`, the exact call `executor.rs` makes). The `db.rs` unit tests are rewritten with a small synchronous pair-builder that uses the same production write API. Governs R4, R5.
- KTD5. **The gate no longer appends any suffix; the caller passes the bare URL unchanged.** `GROOM_PHASE_SUFFIX` is referenced by the gate only to accept the legacy parent URL form (R3); its doc comment is updated to say so.

### Assumptions

- The callback row's `result` is set only through `update_task_completed` (`handle_task_complete`) and the engine never rewrites it before or after delivery; verified by reading `handlers.rs`, not by runtime observation.
- The three groom producers all create the callback through `build_callback_task`, so `dispatch_class='groom'` is set from `skill: dev-groom` on every path.

## Implementation Units

### U1. Rewrite the gate query and its documentation

- **Goal:** `has_completed_groom_for_issue` returns `true` when a completed groom callback with the `PLAN_GROOMED` marker exists under a parent for the issue, regardless of the parent's current class or URL suffix.
- **Requirements:** R1, R3, R11, R12; KTD1, KTD2, KTD5.
- **Dependencies:** none.
- **Files:** `crates/mika-agent/src/db.rs` (gate + doc comment), `crates/mika-agent/src/async_db.rs` (doc comment), `crates/mika-agent/src/task_state/tasks.rs` (`GROOM_PHASE_SUFFIX` doc comment).
- **Approach:**
  1. Replace the query body per KTD1; bind the agent id, the bare URL, and the suffixed URL as parameters (build the suffixed form from `crate::task_state::tasks::GROOM_PHASE_SUFFIX`).
  2. Keep the signature `(&self, agent_id, issue_url) -> Result<bool>`; keep `COUNT(*) > 0`.
  3. Rewrite the doc comment: what counts as proof, why the callback row, the legacy-URL acceptance, and the fail-closed contract (a DB error propagates as `Err`; the caller refuses).
  4. Update the `async_db.rs` wrapper comment and the `GROOM_PHASE_SUFFIX` comment (the gate no longer appends the suffix; it accepts it on the parent).
- **Patterns to follow:** `has_non_deferred_active_callback_child` (same file, callback-row predicate on `parent_task_id`, `trigger_type='callback'`); `try_dispatch_pilot_after_groom_success` marker test.
- **Test scenarios:** covered by U2 and U3.
- **Verification:** `cargo build -p mika-agent` passes; `cargo clippy -p mika-agent` clean; no `INSERT`/`UPDATE`/`DELETE` token appears in the gate function body (R1, reviewable by eye in the diff).

### U2. Anti-recursion test beside `create_groom_callback_pair` (red-before)

- **Goal:** A blocking test proves the gate survives the groom→implement flip, and its pre-fix failure output is captured for the PR body.
- **Requirements:** R4, R5, R11; KTD4.
- **Dependencies:** none for the red run; U1 for the green run.
- **Files:** `crates/mika-agent/src/task_engine/dispatcher.rs` (test module, after the mika#1614 block).
- **Approach:**
  1. Add a `#[tokio::test]` that calls `create_groom_callback_pair(&db, true, Some(TEST_ISSUE_URL))`, then `db.update_task_dispatch_class(&parent_id, "implement")`, then asserts `db.has_completed_groom_for_issue(TEST_ISSUE_URL)` is `Ok(true)`.
  2. Add a sibling negative test: same pair with `plan_groomed = false` (`Outcome: PLAN_ITERATE`) → `Ok(false)`.
  3. Name the positive test `test_groom_gate_survives_implement_flip` (the Verification Contract invokes it by that name) and the negative one `test_groom_gate_refuses_plan_iterate_after_flip`.
  4. Run the positive test against the unmodified gate first; save the failing output verbatim for the PR body. Also run it once with the flip step removed (groom parent, bare URL, still `in_progress`) so the PR states which pre-fix causes the red covers — the bare URL alone already refuses on the old gate; the flip is the cause that would also defeat a parent-row fix.
- **Execution note:** Run the red test before touching U1. The red output is a deliverable (R4).
- **Patterns to follow:** existing mika#1614 tests in the same module (`test_auto_fire_skips_non_groom_class`, `dispatch_class_of`).
- **Test scenarios:**
  - Groom parent (bare URL) + completed groom callback with `Outcome: PLAN_GROOMED`, parent flipped to `implement` → gate `true`.
  - Same pair, callback body `Outcome: PLAN_ITERATE`, parent flipped → gate `false`.
- **Verification:** the positive test fails on `main`'s gate and passes after U1; both tests pass in `cargo test -p mika-agent dispatcher::tests`.

### U3. Rewrite the `#1620` unit tests in `db.rs` around the callback proof

- **Goal:** The synchronous unit tests describe the new contract and pin every refusal path.
- **Requirements:** R5, R12, R13, R14, R2 (refusal paths).
- **Dependencies:** U1.
- **Files:** `crates/mika-agent/src/db.rs` (test module, `has_completed_groom_for_issue (#1620)` block).
- **Approach:**
  1. Replace `groom_task` with two builders: a groom parent `NewTask` with `reference_url = Some(url)`, and a callback `NewTask` with `trigger_type = "callback"`, `parent_task_id = Some(parent_id)`, `dispatch_class = Some("groom")`, `reference_url = None`.
  2. Add a helper that creates the pair and completes the callback via `update_task_completed(id, agent, Some(body))` with a body copied from the dispatcher test fixture shape.
  3. Rewrite the six existing tests and add the missing negatives per R13 and the legacy-URL positive per R14; keep test names prefixed `test_groom_cross_check_`.
- **Patterns to follow:** existing `groom_task` builder style; `db()` in-memory fixture; `register_agent` for the different-agent case.
- **Test scenarios:**
  - No row → `false`.
  - Pair complete, parent bare URL → `true`.
  - Pair complete, callback `delivered` → `true`.
  - Pair complete, parent URL with `?phase=groom` → `true`.
  - Callback `pending` (never completed) → `false`.
  - Callback completed with `Outcome: PLAN_ITERATE` → `false`.
  - Callback with `dispatch_class = "implement"` → `false`.
  - Callback owned by `other-agent`, gate queried for `mika` → `false`.
  - Pair complete under issue 123, gate queried for issue 124 → `false`.
  - Parent-only groom row with suffixed URL and terminal status, no callback (the old proof shape) → `false`.
- **Verification:** all tests pass in `cargo test -p mika-agent db::tests::test_groom_cross_check`.

### U4. Caller fail-closed and rejection text

- **Goal:** The cross-check caller refuses on DB error and describes the real predicate.
- **Requirements:** R2, R7 (no bypass named as recovery), R15; KTD3.
- **Dependencies:** U1.
- **Files:** `crates/mika-agent/src/skills/executor.rs` (`validate_dispatch_readiness`, cross-check block).
- **Approach:**
  1. Replace the `Err(e)` arm: `warn!` then `record_dispatch_rejection` then return a `dispatch_check_failed` JSON with `reason` naming the DB error, mirroring the fail-closed issue-body-fetch branch below it.
  2. Rewrite the block comment: proof = completed groom callback with `Outcome: PLAN_GROOMED` under a parent for this issue.
  3. Rewrite the `dispatch_grooming_not_verified` `predicate` to describe the callback-row proof, and its `recovery` to name the loop groom route and the hand-groomed exit from R15 (drop the bypass flag mention).
  4. Drop the bypass flag mention from the sibling `dispatch_no_grooming_marker` `recovery` text in the same block; both rejection texts stop naming a flag the operator keeps removed.
- **Patterns to follow:** the `dispatch_check_failed` branches already in the same function.
- **Test scenarios:**
  - Test expectation: unit coverage of the `Err` arm is not reachable without a DB fault injector in this function; the refusal is reviewed by reading (R6) and pinned by the surrounding fail-closed pattern. Existing eval scenarios in `tests/eval/test_dispatch_no_grooming_marker_guard.rs` continue to pass.
- **Verification:** `cargo test -p mika-agent --test eval test_dispatch_no_grooming_marker_guard` passes; the diff shows no `Ok(true)`-equivalent outcome on the `Err` arm.

## Verification Contract

| Check | Command | Proves |
|---|---|---|
| Build + lint | `cargo build -p mika-agent && cargo clippy -p mika-agent` | U1, U4 compile clean |
| Red-before | `cargo test -p mika-agent dispatcher::tests::test_groom_gate_survives_implement_flip` on pre-U1 code | R4 (output pasted in PR) |
| Green-after | same command after U1 | R4, R11 |
| Gate unit tests | `cargo test -p mika-agent db::tests::test_groom_cross_check` | R13, R14, R2 refusal paths |
| Eval guard | `cargo test -p mika-agent --test eval test_dispatch_no_grooming_marker_guard` | U4 regression |
| Full agent tests | `cargo test -p mika-agent` | no collateral regression |
| Pipeline artifacts | `bash scripts/verify-pipeline.sh` | plan + AC section present |

## Definition of Done

- All four units landed; `cargo test -p mika-agent` green; clippy clean.
- The gate function body contains a single `SELECT` and no write statement (R1).
- The `Err` arm of the caller cross-check returns a rejection (R2).
- Red-before output for the anti-recursion test is in the PR body (R4).
- PR body names the rollback statement (R9), the retention note with its blast radius (R10), the hand-groomed known case (R15), asks QA to read the refusal condition first, and states the PR is DECISION-CORE by perimeter (R6).
- No abandoned experimental code remains in the diff.

## Acceptance criteria

- [ ] `has_completed_groom_for_issue` returns `true` for a groom parent whose `dispatch_class` was flipped to `implement` and whose groom callback completed with `Outcome: PLAN_GROOMED` (anti-recursion test, red on pre-fix code, green after).
- [ ] The gate is a single read-only `SELECT`; no `INSERT`/`UPDATE`/`DELETE` in the gate path.
- [ ] Every degraded case of the cross-check refuses: no row, `pending` callback, `PLAN_ITERATE` body, other agent, other issue, `implement`-class callback, and a DB error in the caller.
- [ ] One query serves all groom producers; the parent URL matches in bare form and with `?phase=groom`.
- [ ] No test uses raw SQL `INSERT`; proof rows come from `create_task` + `update_task_completed`.
- [ ] Both rejection `recovery` texts in the grooming block (`dispatch_no_grooming_marker`, `dispatch_grooming_not_verified`) no longer name the bypass flag; the cross-check comment describes the callback-row proof; the `dispatch_grooming_not_verified` recovery names the hand-groomed exit.
- [ ] PR body carries: red-before output (flipped and unflipped runs), rollback statement, 30-day retention note with blast radius, hand-groomed known case, DECISION-CORE-by-perimeter statement with QA on the refusal condition first.

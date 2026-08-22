---
issue: mika#1947
title: forge-gate loop-résistance for mika-manager Phase 2 dispatch surface — Porte 1
type: test
scope_repo: mika
companion_repo: mika-platform  # AC7 doc-only update on the brief
priority: p1-important
date: 2026-08-22
---

# Plan — Porte 1 discharge (mika#1947): forge-gate loop-résistance for mika-manager Phase 2

## 1. Problem statement (from ticket body, restated for grounding)

`mika-manager` Phase 2 (dispatch authority) is gated behind **3 portes** documented in
`mika-platform/docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md` § 3.
This ticket is **Porte 1** — proving that the forge-gate perimeter classifier is
loop-résistant against a hypothetical `mika-manager`-triggered dispatch surface.

The classifier is live and merged (mika#1829 + mika#1853 + mika#1860 + mika#1864). Today
it protects mika-dev's dispatch surface. It has NOT been proven — via named tests wired
into `cargo test` — that a mika-manager-authored PR touching
`crates/mika-agent/src/milestone_manager/**` would fail-closed to DECISION-CORE at every
merge-authority callsite.

The pertinent invariant (per Vincent's 2026-07-25 GO in
`mika-platform/scratchpad-1829-forge-gate-design.md`): DECISION-CORE = permission-policy,
verdict-form, gate-logic, définition-du-périmètre, dispatch-authority → GATE DUR
human-only. `mika-manager` dispatch authority sits on the DECISION-CORE side by
construction.

This plan adds **structural + behavioral + cascade-scenario** test coverage that
mika-manager's file surface classifies as DECISION-CORE across every autonomous merge
path, plus doc updates that ratify Porte 1 as DISCHARGED once the tests land green.

## 2. Investigation summary (what the classifier surface looks like today)

Read of the current tree (worktree HEAD = `origin/main` @ `ac94467d`):

- `crates/mika-agent/src/perimeter/rules.rs` defines four private consts:
  `MECHANICAL_PREFIXES`, `MECHANICAL_EXACT`, `MECHANICAL_CONTAINS`,
  `MECHANICAL_SUFFIXES`. `src/milestone_manager/` is absent from all four, so
  `is_mechanical("crates/mika-agent/src/milestone_manager/reader.rs")` returns
  `false` and `classify_path(...)` returns `Classification::DecisionCore` via the
  fail-closed default in `perimeter/mod.rs::classify_path`. Correct today — but
  no named test asserts it.
- `crates/mika-agent/src/perimeter/tests.rs` covers self-reference, explicitly-named
  DECISION-CORE zones (verdict.rs, verdict_handler.rs, pr_merge_with_gate.rs,
  skills/executor.rs, permission-policy, dispatch-lib, qa-review, labels.yml, authority
  docs), positive MECHANICAL matches, PR-level taint, cpp classifier tiers, and root
  scaffolding. `milestone_manager/` is NOT enumerated as either DECISION-CORE or
  MECHANICAL — coverage gap the ticket names.
- `crates/mika-agent/src/milestone_manager/no_dispatch_test.rs` greps the module
  tree for forbidden write-authority tokens (`run_claude_pilot`, `pr_merge_with_gate`,
  `gh api "PATCH"`, `gh issue edit`, …) and fails the test binary if any executable
  code contains them. This protects the *authoring* side (mika-manager can't gain
  write-authority tokens without a test failure). It does NOT protect the *receiving*
  side (the perimeter classifier's treatment of milestone_manager files as
  DECISION-CORE) — that's what this ticket adds.
- `crates/mika-agent/src/server/verdict_handler.rs` (line ~294) and
  `crates/mika-agent/src/server/ci_success_handler.rs` (line ~390) both call
  `perimeter::fetch::fetch_pr_files(...)` and, on `Classification::DecisionCore`,
  return `VerdictAction::Handled { pre_digest: "... forge-gate ..." }` (the "hold"
  branch), log a distinctly-named audit event, notify the operator, and update task
  metadata (verdict_handler only). Actual emitted audit-event shape:
  - verdict_handler: `tool_name = "verdict_handler_human_gate_required"`,
    `target_key = format!("pr:{}#{}", event.repo, event.pr_number)` (or
    `format!("task:{}", t.id)` if an in_progress task exists),
    detail names `decision_core_files`.
  - ci_success_handler: `tool_name = "ci_success_handler_human_gate_required"`,
    `target_key = format!("pr:{}#{}", event.repo, pr.number)`,
    detail names `decision_core_files`.
- `crates/mika-agent/src/perimeter/fetch.rs::fetch_pr_files` calls
  `run_gh_subprocess(...)` directly with no injectable seam. In the eval test
  environment (`gh` not resolvable), it errors → callers synthesize a DECISION-CORE
  verdict per the fail-closed clause. Existing eval tests
  (`verdict_pass_no_task_perimeter_fail_closed_holds_for_operator`,
  `verdict_pass_completed_task_perimeter_fail_closed_holds_for_operator`,
  `verdict_pass_pending_task_perimeter_fail_closed_holds_for_operator`) rely on
  this fail-closed path — no test drives a concrete file list through the handler.

**AC-vs-code approximation surfaced (transparency, not divergence).**
The ticket body's Design-approach paragraph names an idealized audit-event shape
(`tool_name = 'perimeter_gate'`, `target_key = 'decision_core_block'`) that does NOT
match the currently-emitted names above. The ACs themselves (AC3/AC4) inherit
`target_key = 'decision_core_block'` and `hold[human_review]` phrasing that are
semantically-equivalent but not literally-emitted. **Two options for reconciliation:**
- **(A) — carry intent, cite actual names (this plan's default).** Assert the
  currently-emitted `tool_name`/`target_key` in AC3/AC4 tests. AC3/AC4's semantic
  intent (auditable proof that the DECISION-CORE block fires on `milestone_manager/**`
  files) is satisfied. The plan explicitly names the actual strings in the assertion
  code so the divergence is visible and reviewable.
- **(B) — rename the audit-event strings to match the ticket.** Adds a rename of
  two `tool_name` constants (breaking any operator grep on
  `verdict_handler_human_gate_required` / `ci_success_handler_human_gate_required` —
  the strings appear in the emitter file itself and in the CLAUDE.md summary for
  ci_success_handler; no other callers grep them today). Larger blast radius; not
  self-evidently better than option A.

**Plan default: option A.** Route-back to option B only if the architect prefers
literal-string match over semantic match. Both routes discharge Porte 1 equally at the
loop-résistance level.

## 3. Scope

**In scope (this PR, mika repo):**
1. New test coverage in `crates/mika-agent/src/perimeter/tests.rs`
   (AC1 + AC2): milestone_manager paths classify as DECISION-CORE via both
   `classify_path` and `classify_pr_files`; the rules tables (`MECHANICAL_PREFIXES` /
   `MECHANICAL_EXACT` / `MECHANICAL_CONTAINS`) do not contain any `milestone_manager`
   substring.
2. New handler-integration coverage in
   `crates/mika-agent/tests/eval/test_verdict_handler.rs` (AC3) — a named test that
   asserts the DECISION-CORE branch of `handle_pass_verdict` fires and emits the
   documented audit event, using the existing `fail-closed` seam plus a companion
   assertion at the perimeter layer proving the milestone_manager file set drives the
   same branch. Registered in `crates/mika-agent/tests/eval.rs`.
3. New handler-integration coverage in
   `crates/mika-agent/tests/eval/test_ci_success_handler.rs` (AC4) — new test file
   (currently no `test_ci_success_handler.rs` exists in `tests/eval/`), same shape
   as AC3 for the CI-success race path. Registered in
   `crates/mika-agent/tests/eval.rs`.
4. New cascade-scenario integration test at
   `crates/mika-agent/tests/eval/manager_loop_resistance.rs` (AC5), gated behind
   `#[ignore]` + `MIKA_MANAGER_LOOP_RESISTANCE_TEST=1`. Wires the perimeter classifier
   against a hand-simulated `milestone_manager_report → hypothetical_dispatch → PR_open
   → CI_success → callback` cascade and asserts every layer refuses to enqueue a
   follow-up dispatch that targets `src/milestone_manager/**`.
5. Docstring update on `crates/mika-agent/src/milestone_manager/mod.rs` (AC6) —
   the existing `## Phase 2 promotion (NOT wired here)` section gets an explicit
   pointer to mika#1947 as the Porte 1 discharge condition, and names the four
   test-file paths (AC1/AC2 in `perimeter/tests.rs`, AC3 in `test_verdict_handler.rs`,
   AC4 in `test_ci_success_handler.rs`, AC5 in `manager_loop_resistance.rs`) as
   in-tree proof.

**In scope (companion PR, mika-platform repo):**
6. Doc update on
   `mika-platform/docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md`
   § 3 Porte 1 (AC7) — appends a `**Statut : DISCHARGED**` line naming the mika PR,
   the merge SHA (post-merge), and the five test artifacts. Landed as a **separate
   commit on a same-named branch in mika-platform**
   (`test/1947/perimeter-manager-forge-gate-loop-r`), companion-cross-referenced from
   the mika PR body per the cross-repo convention in the meta-repo CLAUDE.md.

**Out of scope (explicitly):**
- Wiring Phase 2 dispatch itself. This ticket only proves the safety gate;
  Phase 2 promotion is a separate ticket that unblocks after this + Porte 2
  (contention exec) + Porte 3 (INTERNAL_TOKEN alignment) all discharge.
- Extending perimeter classification to mika-cloud/mika-skills repos.
- Refactoring `perimeter::fetch::fetch_pr_files` to be injectable. The unit-test
  route (perimeter/tests.rs) + eval-fail-closed seam (test_verdict_handler.rs
  pattern) already proves the invariant without touching the fetch surface.
- Renaming the emitted audit-event `tool_name` strings — see § 2 option (B) — unless
  the architect ROUTE-backs to that option.

## 4. Detailed step-by-step

### Step 1 — Add `milestone_manager/**` coverage to `perimeter/tests.rs` (AC1 + AC2)

**File:** `crates/mika-agent/src/perimeter/tests.rs`

**Change:** append a new section at the end of the file (after the mika#1864 root
scaffolding block), grouping four new `#[test]` functions:

1. `milestone_manager_files_are_decision_core` — asserts `classify_path(path)`
   returns `Classification::DecisionCore` for every current file under
   `crates/mika-agent/src/milestone_manager/` (mod.rs, reader.rs, assessor.rs,
   reporter.rs, cadence.rs, spawn.rs, types.rs, no_dispatch_test.rs) and for
   representative future paths (`crates/mika-agent/src/milestone_manager/foo.rs`,
   `crates/mika-agent/src/milestone_manager/sub/bar.rs`). Same file-list-iteration
   shape as `perimeter_module_files_are_decision_core`. Discharges AC1.
2. `milestone_manager_file_taints_pr_batch` — asserts `classify_pr_files(&[...])`
   with a mix of MECHANICAL files and one `milestone_manager/reader.rs` returns
   `verdict == DecisionCore` and puts `reader.rs` in `decision_core_files`. Same
   shape as `one_decision_core_file_taints_the_batch`. Discharges AC2.
3. `milestone_manager_prefix_not_in_mechanical_tables` — reads
   `include_str!("rules.rs")` and asserts the string `milestone_manager` does NOT
   appear inside the `MECHANICAL_PREFIXES` / `MECHANICAL_EXACT` /
   `MECHANICAL_CONTAINS` / `MECHANICAL_SUFFIXES` slice literals. Uses simple
   substring scan over the whole source file — sufficient because these consts are
   the ONLY places rules.rs contains file-path strings. Structural counterpart to
   AC1: catches a rules-file diff that adds a `milestone_manager` entry to any
   MECHANICAL table (adding `milestone_manager` anywhere in that file trips this
   test, forcing an atomic update). Discharges the "structural" side of AC1.
4. `milestone_manager_solo_pr_is_decision_core` — asserts a single-file PR
   (`crates/mika-agent/src/milestone_manager/reader.rs` only) classifies as
   DECISION-CORE and returns the file in `decision_core_files`. Sanity companion
   to test #2.
5. `milestone_manager_absent_from_all_mechanical_tables` — **F2 architect
   first-pass concern.** Reads `include_str!("rules.rs")` and asserts the
   substring `milestone_manager` does NOT appear in the file at all — a
   negative-space assertion complementing tests #1–#4 (which prove positive
   DECISION-CORE classification). Rationale (per review-guide.md § Orthogonality):
   a future well-intentioned diff could add a `milestone_manager` entry to any of
   the four MECHANICAL tables (`MECHANICAL_PREFIXES` / `MECHANICAL_EXACT` /
   `MECHANICAL_CONTAINS` / `MECHANICAL_SUFFIXES`) — this test catches that
   whole-file class regardless of which table receives the entry. Distinct from
   test #3 (which parses only the four table literals): test #5 is a
   whole-file scan. The pair is redundant-on-purpose — if a future refactor
   restructures the const tables, test #3 might silently pass a
   `milestone_manager` entry that test #5 catches (or vice-versa on a rename
   that hides substring `milestone_manager` behind an alias). Fail-loud on any
   drift.

**Note on the ticket's suggested `perimeter_boundary_test.rs` sibling.** The ticket
offers a sibling file as an alternative. This plan places the coverage in the
existing `perimeter/tests.rs` because (a) the file already groups all perimeter
classifier tests in named `///` doc sections, (b) a sibling file would need its own
`mod perimeter_boundary_test;` wiring under `#[cfg(test)] mod tests;`, and (c) every
existing DECISION-CORE zone (verdict, pr_merge_with_gate, executor,
permission-policy, dispatch-lib, qa-review, labels.yml, authority docs) is proven
via a named test in this same file — milestone_manager belongs alongside them.

**Diff shape:** ~80 lines appended, no code deleted, no imports added.

### Step 2 — Add handler-integration test for verdict_handler (AC3)

**File:** `crates/mika-agent/tests/eval/test_verdict_handler.rs`

**Change:** append a new `#[tokio::test]` (after
`verdict_pass_pending_task_perimeter_fail_closed_holds_for_operator`, ~line 1170):

```rust
#[tokio::test]
async fn verdict_pass_milestone_manager_pr_holds_for_operator() -> Result<()> {
    // AC3 (mika#1947) — Porte 1 loop-résistance: a VERDICT: pass on a PR touching
    // crates/mika-agent/src/milestone_manager/** MUST hit the forge-gate DECISION-CORE
    // branch of handle_pass_verdict.
    //
    // Layer A (unit): the perimeter classifier maps milestone_manager files to
    // DECISION-CORE — asserted directly here as anchor.
    let files = vec![
        "crates/mika-agent/src/milestone_manager/reader.rs".to_string(),
    ];
    let verdict = crate::perimeter::classify_pr_files(&files);
    assert_eq!(verdict.verdict, crate::perimeter::Classification::DecisionCore);
    assert_eq!(verdict.decision_core_files, files);

    // Layer B (handler-integration): with fetch_pr_files un-injectable in the eval
    // env, the handler's DECISION-CORE branch is reached via the fail-closed clause
    // (fetch error → synthesized DecisionCore verdict). This asserts the same code
    // path the layer-A file list would hit at production runtime.
    let db = test_db().await;
    let text = pr_review_text(
        "approved",
        "senara-solutions/mika",
        1947,
        "mika-qa",
        "VERDICT: pass\n\nAll good.",
    );
    let action = try_handle_pr_review_verdict(
        &text,
        &db,
        Some("fake-token"),
        None,
        SESSION_ID,
        "trace-porte1",
        &test_skills(),
    )
    .await;
    match action {
        VerdictAction::Handled { pre_digest } => {
            assert!(
                pre_digest.contains("forge-gate") || pre_digest.contains("DECISION-CORE"),
                "hold pre-digest must name the gate: {pre_digest}"
            );
        }
        other => panic!(
            "milestone_manager PR must Handle (forge-gate DECISION-CORE hold), got {other:?}"
        ),
    }

    // Layer C (audit): the DECISION-CORE branch writes an audit_events row with
    // the actual emitted tool_name (see § 2 approximation note in the plan).
    let events = db
        .query_audit_events_by_tool_name("verdict_handler_human_gate_required", 10)
        .await?;
    assert!(!events.is_empty(), "expected verdict_handler_human_gate_required audit row");
    // No dispatch enqueued.
    // (existing perimeter fail-closed tests already assert Passthrough-not-taken;
    // this test asserts Handled, which is the mutually-exclusive branch — no
    // separate "not-dispatched" check is needed.)
    Ok(())
}
```

**Rationale for the three-layer shape:**
- Layer A directly proves the classifier's per-file verdict on
  `milestone_manager/**` (AC3's semantic core).
- Layer B proves the DECISION-CORE branch of `handle_pass_verdict` fires when the
  classifier returns DecisionCore (via fail-closed seam — the only handler-driving
  seam available without a `fetch_pr_files` injection).
- Layer C proves the audit-events row lands with the actual emitted tool_name
  (§ 2 option A). If the architect ROUTE-backs to option B, only the Layer C
  string changes.

**Registration:** already covered — `test_verdict_handler` is registered in
`crates/mika-agent/tests/eval.rs:76`.

**Diff shape:** ~55 lines appended, no imports added if `query_audit_events_by_tool_name`
already exists (check step; if it doesn't, use `list_audit_events` with a filter or
the same `count_recent_audit_events_for_target` shape used by the circuit breaker in
verdict_handler.rs:1819 — pick whichever matches the DB accessor surface). **Note for
implementer:** verify the exact `AsyncDatabase` method available before writing the
assertion; the layer-C assertion may need one line of adjustment.

### Step 3 — Add handler-integration test for ci_success_handler (AC4)

**File:** `crates/mika-agent/tests/eval/test_ci_success_handler.rs` (NEW)

**Change:** create a new file mirroring the shape of `test_verdict_handler.rs`
scaffolding (test_skills, test_db helpers copied or shared). One `#[tokio::test]`
initially, named `ci_success_milestone_manager_pr_holds_for_operator`, with the
same three-layer shape as step 2 targeting `ci_success_handler`:

- Layer A: `perimeter::classify_pr_files(&["crates/mika-agent/src/milestone_manager/reader.rs"])`
  returns `Classification::DecisionCore`.
- Layer B: invoke `crate::server::ci_success_handler::try_handle_check_suite_success`
  (or the equivalent entry-point exported for tests — check the current export
  surface at implementation time; verdict_handler exports
  `try_handle_pr_review_verdict`, ci_success_handler is expected to export a sibling
  entry-point of the same shape) with a mock `check_suite.completed(success)` event
  correlated to a PR whose fetch fails-closed, and assert it returns
  `VerdictAction::Handled { pre_digest }` containing `"forge-gate"` or
  `"DECISION-CORE"`.
- Layer C: `audit_events` row written with
  `tool_name = "ci_success_handler_human_gate_required"`.

**Registration:** add `mod test_ci_success_handler;` under the alphabetical block in
`crates/mika-agent/tests/eval.rs` (between `test_callback_turn` and
`test_completion_claim_guard` if strictly alpha, or adjacent to `test_verdict_handler`
if grouped by feature — match the existing convention on inspection).

**Implementer note:** check the export surface of `crate::server::ci_success_handler`
before writing Layer B. If the entry-point isn't already `pub`, promote the same
`try_handle_*` shape verdict_handler exposes (structural pattern already in
`crates/mika-agent/src/server/verdict_handler.rs:294`). This is a **surface-only**
change to the ci_success_handler public API — no logic change, no LLM-facing
behavior change.

**Diff shape:** ~120 lines new file + one line added to
`crates/mika-agent/tests/eval.rs`.

### Step 4 — Add cascade-scenario integration test (AC5)

**File:** `crates/mika-agent/tests/eval/manager_loop_resistance.rs` (NEW)

**Change:** create a new file, gated by `#[ignore]` and env-var
`MIKA_MANAGER_LOOP_RESISTANCE_TEST=1`. One `#[tokio::test]` function
`cascade_never_dispatches_into_milestone_manager` that hand-simulates the
five-step cascade named in the ticket:

1. **`milestone_manager_report`** — build a `MilestoneState` via
   `crate::milestone_manager::Reader` (feed a synthetic `GhRunner` that returns
   canned issue/PR lists — see `compose_from_gh_outputs`) and assert the
   `Assessor` produces a non-Blocked `Assessment` (representative healthy
   milestone).
2. **`hypothetical_dispatch`** — construct a Phase-2-hypothetical PR file list
   `let files = vec![
       "crates/mika-agent/src/milestone_manager/reader.rs".to_string(),
       "crates/mika-agent/src/milestone_manager/assessor.rs".to_string(),
   ];` — and assert `perimeter::classify_pr_files(&files)` returns
   `Classification::DecisionCore`. **This is the primary loop-résistance
   invariant** — a hypothetical Phase 2 dispatch touching mika-manager's own
   surface CANNOT auto-merge.
3. **`PR_open`** — assert the same file list passed through both
   `verdict_handler::handle_pass_verdict` and
   `ci_success_handler::try_handle_check_suite_success` (via the fail-closed seam,
   see steps 2 and 3) yields `VerdictAction::Handled { .. }`. This proves the
   cascade cannot slip through either merge-authority callsite.
4. **`CI_success`** — same as PR_open for `ci_success_handler` (the CI-success race
   path, mika#1853's founding incident).
5. **`callback`** — assert that even if a callback WERE constructed for this
   cascade, `crate::skills::executor::validate_dispatch_readiness` (or the
   equivalent guard) would reject a follow-up dispatch. Since mika-manager has no
   dispatch class today (step 1 verified `Assessor` doesn't produce dispatch
   verdicts), this step reduces to a **structural** assertion: greppable proof
   that `milestone_manager/**` contains no dispatch-authority tokens (already
   asserted by `no_dispatch_test.rs`). Reference-and-assert-still-passing shape.

**F1 architect first-pass concern — AC5 literal divergence from ticket
("Uses EvalHarness"):** the ticket's AC5 language names `EvalHarness`, but
`EvalHarness` is the LLM-eval integration harness (`MockLlmProvider` seeded
scenarios in `tests/eval/harness.rs`) — it exists to exercise the agent-loop
`run_agent()` path deterministically without network. mika-manager Phase 1 has
no LLM turn: `Reader`, `Assessor`, and `Reporter` are all pure code, and the
perimeter classifier is a pure function. Wrapping any of them in `EvalHarness`
would add scaffolding without adding assertion power (per review-guide.md § KISS
— match test tool to system under test). This plan uses direct
`Reader`/`Assessor`/`classify_pr_files`/`handler` calls instead. If Porte 2
promotion later adds an LLM turn to mika-manager (dispatch-verdict shape), that
future test would legitimately need `EvalHarness` — Porte 1 does not. The
cascade test's first `#[doc]` comment block explicitly names this justification
so a future reader arriving via the AC-text trail lands on the reasoning.

**Rationale for `#[ignore]` + env-var gate:** the ticket calls for this to run
pre-Phase-2-cut, not on every PR CI. The gate serves two purposes:
- Prevents accidental CI runtime cost (the test does non-trivial DB setup and
  classifier walks).
- Provides operator-visible surface: `MIKA_MANAGER_LOOP_RESISTANCE_TEST=1 cargo
  test -p mika-agent --test eval -- --ignored cascade_never_dispatches_into_milestone_manager`
  is the canonical pre-Phase-2-cut command that goes in the mika-platform brief
  § 3 Porte 1 discharge note (AC7).

**Registration:** add `mod manager_loop_resistance;` under the alphabetical block in
`crates/mika-agent/tests/eval.rs`.

**Diff shape:** ~180 lines new file + one line added to
`crates/mika-agent/tests/eval.rs`.

### Step 5 — Update `milestone_manager/mod.rs` docstring (AC6)

**File:** `crates/mika-agent/src/milestone_manager/mod.rs`

**Change:** the existing `## Phase 2 promotion (NOT wired here)` section (lines
28–32) is replaced with a strictly-additive expansion that names mika#1947 as the
Porte 1 discharge condition and lists the five test artifacts as proof:

```markdown
//! ## Phase 2 promotion (NOT wired here)
//!
//! Phase 2 (dispatch authority — recommend + auto-execute) is gated behind the three
//! portes documented in the brief § 3: forge-gate loop-résistance (Porte 1), contention
//! exec (Porte 2), and `INTERNAL_TOKEN` alignment (Porte 3). None of that surface is
//! wired in this module.
//!
//! **Porte 1 discharge condition (mika#1947):** the forge-gate perimeter classifier
//! must fail-closed to `DecisionCore` on every file under
//! `crates/mika-agent/src/milestone_manager/**`, at every merge-authority callsite
//! (`verdict_handler`, `ci_success_handler`, `pr_merge_with_gate`). In-tree proof:
//!
//! - `crates/mika-agent/src/perimeter/tests.rs` —
//!   `milestone_manager_files_are_decision_core`,
//!   `milestone_manager_file_taints_pr_batch`,
//!   `milestone_manager_prefix_not_in_mechanical_tables`,
//!   `milestone_manager_solo_pr_is_decision_core`,
//!   `milestone_manager_absent_from_all_mechanical_tables` (AC1 + AC2).
//! - `crates/mika-agent/tests/eval/test_verdict_handler.rs::verdict_pass_milestone_manager_pr_holds_for_operator` (AC3).
//! - `crates/mika-agent/tests/eval/test_ci_success_handler.rs::ci_success_milestone_manager_pr_holds_for_operator` (AC4).
//! - `crates/mika-agent/tests/eval/manager_loop_resistance.rs::cascade_never_dispatches_into_milestone_manager`
//!   (AC5, gated `#[ignore]` + `MIKA_MANAGER_LOOP_RESISTANCE_TEST=1`).
```

**Diff shape:** ~20 lines changed in-place, no code changes.

### Step 6 — Companion PR: update the mika-platform brief (AC7)

**File:** `mika-platform/docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md`

**Change:** in § 3 Porte 1 (lines 145–149), append a new paragraph immediately
after the current `**Statut actuel** : forge-gate en cours ; audit_events wire
shipped …` line:

```markdown
**Statut : DISCHARGED** (mika#1947 landed <YYYY-MM-DD>, PR #<n>, merge SHA `<sha>`).
Proof in-tree at (mika repo):

- `crates/mika-agent/src/perimeter/tests.rs` — five `milestone_manager_*` tests: `milestone_manager_files_are_decision_core`, `milestone_manager_file_taints_pr_batch`, `milestone_manager_prefix_not_in_mechanical_tables`, `milestone_manager_solo_pr_is_decision_core`, `milestone_manager_absent_from_all_mechanical_tables` (AC1/AC2).
- `crates/mika-agent/tests/eval/test_verdict_handler.rs::verdict_pass_milestone_manager_pr_holds_for_operator` (AC3).
- `crates/mika-agent/tests/eval/test_ci_success_handler.rs::ci_success_milestone_manager_pr_holds_for_operator` (AC4).
- `crates/mika-agent/tests/eval/manager_loop_resistance.rs::cascade_never_dispatches_into_milestone_manager`
  (AC5, gated `#[ignore]` + `MIKA_MANAGER_LOOP_RESISTANCE_TEST=1`).

Post-merge operator verification: `cargo test -p mika-agent perimeter` +
`cargo test -p mika-agent no_dispatch` both pass on `main`;
`MIKA_MANAGER_LOOP_RESISTANCE_TEST=1 cargo test -p mika-agent --test eval -- --ignored
cascade_never_dispatches_into_milestone_manager` passes.
```

**Companion branch shape:** `test/1947/perimeter-manager-forge-gate-loop-r` on
`senara-solutions/mika-platform`, one commit, doc-only. Cross-referenced from the
mika PR body per the meta-repo CLAUDE.md § "PR cross-referencing" convention.

**Diff shape:** ~12 lines appended to the brief, no other changes.

### Step 7 — PR body assembly (AC8)

The PR body (assembled at `/mika` pipeline step ~ce-commit-push-pr) MUST include a
`## Post-merge verification` section quoting the AC8 commands verbatim, plus the
audit-events grep instruction:

```
## Post-merge verification
- `cargo test -p mika-agent perimeter` — must pass on `main`.
- `cargo test -p mika-agent no_dispatch` — must pass on `main`.
- `MIKA_MANAGER_LOOP_RESISTANCE_TEST=1 cargo test -p mika-agent --test eval -- --ignored cascade_never_dispatches_into_milestone_manager` — must pass on `main`.
- (optional, post-deploy) `sqlite3 ~/.mika/agents/<agent>/mika.db "SELECT COUNT(*) FROM audit_events WHERE tool_name = 'verdict_handler_human_gate_required' OR tool_name = 'ci_success_handler_human_gate_required'"` — reports total DECISION-CORE holds since deploy. May be zero if no PR touched `milestone_manager/**` post-deploy; log the query used so operator can rerun.

Companion PR: senara-solutions/mika-platform#<N> (§ 3 Porte 1 status update, AC7).
```

## 5. Acceptance-criteria tie-back

| AC  | Deliverable                                                                                                                                                       | Plan step | File(s) touched                                                                          |
|-----|-------------------------------------------------------------------------------------------------------------------------------------------------------------------|-----------|------------------------------------------------------------------------------------------|
| AC1 | Test asserts `MECHANICAL_PREFIXES` / `MECHANICAL_EXACT` do NOT cover `src/milestone_manager/` (structural + positive classification form; test #3 parses only the const tables, test #5 scans the whole file — redundant-on-purpose per F2 first-pass architect concern) | Step 1    | `crates/mika-agent/src/perimeter/tests.rs`                                                |
| AC2 | Unit test: `classify_pr_files(&["crates/mika-agent/src/milestone_manager/reader.rs"])` returns `Classification::DecisionCore` with file in `decision_core_files`   | Step 1    | `crates/mika-agent/src/perimeter/tests.rs`                                                |
| AC3 | Unit test in `verdict_handler`: milestone_manager PR + APPROVED VERDICT: pass returns `hold` action, writes audit_events row, hold-metadata note                   | Step 2    | `crates/mika-agent/tests/eval/test_verdict_handler.rs`                                    |
| AC4 | Unit test in `ci_success_handler`: `check_suite.completed(success)` on milestone_manager PR blocks auto-merge with same audit-event shape                          | Step 3    | `crates/mika-agent/tests/eval/test_ci_success_handler.rs` (NEW), `tests/eval.rs`          |
| AC5 | Integration test `manager_loop_resistance.rs` gated `#[ignore]` + env-var: simulates full cascade, asserts no follow-up dispatch targets `src/milestone_manager/**` | Step 4    | `crates/mika-agent/tests/eval/manager_loop_resistance.rs` (NEW), `tests/eval.rs`          |
| AC6 | `milestone_manager/mod.rs` docstring updated: names ticket + lists five test artifacts as proof                                                                    | Step 5    | `crates/mika-agent/src/milestone_manager/mod.rs`                                          |
| AC7 | Brief § 3 Porte 1 updated with `**Statut : DISCHARGED**` line naming ticket + PR + test-file paths                                                                 | Step 6    | `mika-platform/docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md` (companion PR) |
| AC8 | Post-merge verification steps listed in PR body; audit-events query documented                                                                                    | Step 7    | PR body (assembled at ce-commit-push-pr step)                                             |

## 6. Risks & open questions

**R1 (raised in § 2, plan default option A):** AC3/AC4 use idealized `target_key`
and `tool_name` strings that don't match currently-emitted shapes. Plan asserts
actual emitted strings. If architect prefers literal match, ROUTE-back to option B
(rename two constants). No other planning axis depends on the choice.

**R2:** `ci_success_handler` may not export a `try_handle_*` entry-point analogous
to `try_handle_pr_review_verdict` used by verdict_handler tests. Step 3 assumes it
does or promotes the sibling shape. If a larger refactor is needed to expose the
entry-point, that's a scope creep the architect should weigh. Mitigation: the
worst case is 5–15 lines of `pub(crate) async fn try_handle_check_suite_success(...)`
wrapper that already exists implicitly in the handler's public surface — no
behavior change.

**R3:** `AsyncDatabase::query_audit_events_by_tool_name` is a placeholder — the
implementer must verify the actual method name before writing Layer C assertions
in steps 2 and 3. Adjacent options: `list_audit_events` with a filter, or the
`count_recent_audit_events_for_target` pattern used by
`verdict_handler.rs:1819`. This is a mechanical adjustment during implementation,
not a plan-shape risk.

**R4:** The cascade-scenario test (step 4, AC5) reduces the callback step to a
structural assertion because no dispatch class exists in Phase 1. This may not
satisfy the ticket's language ("callback CANNOT enqueue a follow-up dispatch")
literally. The plan's framing: **the invariant IS the absence of dispatch class**,
which `no_dispatch_test.rs` already proves; the cascade test reinforces the
invariant by walking the classifier through every Phase-2-hypothetical file set.
If the architect wants a stronger simulation (e.g., wire a fake dispatch class into
a scratch DB and prove it fails at the classifier), that's a scope expansion.
Mitigation available.

**R5:** AC7 is a cross-repo doc change (mika-platform). It requires a companion PR
on `mika-platform`. The meta-repo CLAUDE.md covers this pattern; execution risk is
low but non-zero (two PRs must land in sequence, mika first for the SHA).

**R6 (RESOLVED — first-pass architect concerns applied).** Two mika-arch
first-pass concerns were folded into the plan verbatim:
- **F1 (AC5 divergence justification):** step 4 now includes a dedicated
  "F1 architect first-pass concern" paragraph explaining WHY `EvalHarness` is
  inappropriate for a pure-code cascade (per review-guide.md § KISS), and the
  cascade-test file will include the same reasoning in its first `#[doc]`
  comment block so a future reader arriving via the AC-text trail lands on the
  reasoning.
- **F2 (negative-space assertion on all four MECHANICAL tables):** step 1 gains
  a fifth test `milestone_manager_absent_from_all_mechanical_tables` that scans
  the whole `rules.rs` file for the substring `milestone_manager`,
  redundant-on-purpose with test #3 (const-table-literal scan) — the pair
  covers table-literal drift AND whole-file introduction paths.

## 7. Sequencing

Single PR, single commit stack:

1. Step 5 (docstring update) — cheap, self-contained, no code churn.
2. Steps 1 (perimeter tests) + 2 (verdict_handler test) + 3 (ci_success_handler
   test) + 4 (cascade test) — atomic; grouped as one implementation slice.
3. Step 7 (PR body assembly) — happens at ce-commit-push-pr stage.
4. Step 6 (companion PR on mika-platform) — post-merge, once mika PR merges and
   SHA is known.

CI expectation: the four new tests in steps 1–3 add ~4 test cases to
`cargo test -p mika-agent` (perimeter/tests.rs) and 2 test cases to
`cargo test -p mika-agent --test eval` (test_verdict_handler +
test_ci_success_handler). The cascade test (step 4) does NOT run on CI (gated
`#[ignore]` + env-var). Expected CI runtime delta: negligible (< 1 s).

## 8. Grounding — feedback anchors this plan honors

- `feedback_hard_evidence_before_filing` — every AC-vs-code approximation named
  in § 2 is grounded in a concrete file:line citation (`verdict_handler.rs:294`,
  `ci_success_handler.rs:390`, `perimeter/rules.rs` const scan).
- `feedback_prompt_enforcement_fragile` — this ticket's whole premise: the
  perimeter fail-closed default is structural (Rust type) but the invariant that
  `milestone_manager/**` never gets added to MECHANICAL is currently un-tested.
  This plan adds the test that makes the invariant structural at the module-boundary
  level.
- `feedback_structural_gate_audit_grep_all_callsites` — the plan covers all three
  merge-authority callsites (verdict_handler, ci_success_handler, pr_merge_with_gate
  implicitly via the shared classifier). `pr_merge_with_gate` is not a separate
  handler-integration test because it calls the same `classify_pr_files`
  primitive already covered by AC1/AC2.
- `feedback_wedge_class_closed_is_not_substrate_stabilized` — Porte 1 discharge
  ratifies that mika-manager Phase 2 dispatch authority CAN advance to
  substrate-stabilized (once Porte 2/3 also discharge). Nothing in this plan
  implements Phase 2 itself.
- `feedback_verify_pipeline_passes_without_the_fix` — the four new tests would
  fail on today's `main` if we added the wrong classifier default; they pass on
  `main` today because the fail-closed default already treats
  `milestone_manager/**` as DECISION-CORE. Removing the fail-closed default (a
  regression) would flip them red.

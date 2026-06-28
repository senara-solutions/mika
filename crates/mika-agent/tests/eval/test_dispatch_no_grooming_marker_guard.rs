//! Integration tests: dispatch grooming-marker guard (#919).
//!
//! Verifies that `validate_dispatch_readiness()` rejects `run_claude_pilot`
//! dispatches with `skill="dev-pilot"` when the target issue body lacks the
//! three canonical grooming callouts (Branch + Plan + architect verdict).
//!
//! The guard is engine-level — all dispatch paths (ready-label webhook, CLI
//! ask, sprint, free-text) funnel through `validate_dispatch_readiness()`,
//! so testing the predicate at this function level covers all four paths.
//!
//! Note: tests that require `fetch_issue_body` (HTTP call) are not included
//! here — those are covered by the predicate extraction in
//! `check_grooming_markers()`. The full end-to-end guard fires inside the
//! executor when the real `run_claude_pilot` tool handler calls
//! `validate_dispatch_readiness()` with a live GitHub token.

use mika_agent::skills::executor::check_grooming_markers;

/// Verbatim 2026-05-13 mika#1097 incident body-callout shape (#1108 reproduce fixture).
/// This is the exact grooming-history line that the dev-groom emitted when
/// mika-arch issued a paraphrased GROOMED verdict (mechanism (c) — Phase 5
/// emission drift). The dispatch gate previously rejected this shape.
const PARAPHRASED_GROOMED_BODY: &str = r#"
> - **Branch:** `bug/1097/autonomous-loop-dev-groom-claude-pilot`
> - **Plan:** `mika/docs/plans/2026-05-13-003-bug-dev-groom-claude-pilot-early-exit-plan.md` (committed on branch @ `a1b2c3d4`)
> - **Grooming history:** /ce:plan → mika-arch first-pass (ITERATE, 3 blocking + 3 non-blocking) → revisions → mika-arch second-pass (READY, paraphrased GROOMED per spec tolerance)

## Description

dev-groom claude-pilot exits Success at ~12 turns without calling architect.
"#;

/// Canonical groomed issue body with all three signals.
const GROOMED_BODY: &str = r#"
> - **Branch:** `fix/919/self-dev-agent-operator-cli-dispatch`
> - **Plan:** `mika/docs/plans/2026-05-13-001-fix-dispatch-grooming-marker-engine-guard-plan.md` (committed on branch @ `3f99625a`)
> - **Grooming history:** /ce:plan → mika-arch first-pass (ITERATE) → revisions → mika-arch second-pass (GROOMED)

## Symptom

Tickets dispatched via `mika ask --agent mika-dev "implement..."` reach claude-pilot without grooming.
"#;

/// Ungroomed body — no grooming callouts at all.
const UNGROOMED_BODY: &str = r#"
## Symptom

Some bug description. No grooming markers present.

## Proposed fix

Do something about it.
"#;

/// Partially groomed — has Plan callout but missing Branch and verdict.
const PARTIAL_BODY_PLAN_ONLY: &str = r#"
## Symptom

Some description.

Plan: docs/plans/some-plan.md is referenced here.

No branch callout or grooming history.
"#;

// --- Scenario 1: ungroomed_dev_pilot_rejects ---

/// Task with reference_url → issue, body lacks all three signals.
/// Assert: all three missing signals reported.
#[test]
fn ungroomed_dev_pilot_rejects() {
    let missing = check_grooming_markers(UNGROOMED_BODY);
    assert_eq!(
        missing,
        vec!["branch_callout", "plan_callout", "groomed_verdict"],
        "ungroomed body should report all three missing signals"
    );
}

// --- Scenario 2: partial_marker_dev_pilot_rejects ---

/// Body has Plan but no Branch or verdict.
/// Assert: missing branch_callout and groomed_verdict.
#[test]
fn partial_marker_dev_pilot_rejects() {
    let missing = check_grooming_markers(PARTIAL_BODY_PLAN_ONLY);
    assert_eq!(
        missing,
        vec!["branch_callout", "groomed_verdict"],
        "partial body with only Plan should report branch_callout and groomed_verdict"
    );
}

// --- Scenario 3: fully_groomed_dev_pilot_proceeds ---

/// Body contains all three signals.
/// Assert: gate returns Ok (empty missing signals).
#[test]
fn fully_groomed_dev_pilot_proceeds() {
    let missing = check_grooming_markers(GROOMED_BODY);
    assert!(
        missing.is_empty(),
        "fully groomed body should have no missing signals, got: {missing:?}"
    );
}

// --- Scenario 4: ungroomed_dev_groom_proceeds ---
// (bypass via skill predicate — tested at the validate_dispatch_readiness
//  level; the check_grooming_markers predicate itself is skill-agnostic)

// --- Scenario 5: milestone_type_bypasses ---
// (bypass via task type — tested at the validate_dispatch_readiness level)

// --- Scenario 6: non_issue_reference_bypasses ---
// (bypass via parse_github_ref returning PullRequest — tested at the
//  validate_dispatch_readiness level)

// --- Scenario 7: no_github_token_warns_and_bypasses ---
// (fail-open when no token — tested at the validate_dispatch_readiness level)

// --- Scenario 8: gh_api_error_fail_closed ---
// (fail-closed on API error — tested at the validate_dispatch_readiness level)

// --- Scenario 9: env_bypass_skips_check ---
// (env var bypass — tested at the validate_dispatch_readiness level)

// --- Additional predicate coverage ---

/// Edge case: body has Branch and Plan but wrong verdict shape ("Verdict: GROOMED"
/// instead of canonical "second-pass (GROOMED)").
#[test]
fn wrong_verdict_shape_rejects() {
    let body = r#"
> - **Branch:** `feat/something`
> - **Plan:** `docs/plans/foo.md` (committed on branch @ `abc123`)
Verdict: GROOMED
"#;
    let missing = check_grooming_markers(body);
    assert_eq!(
        missing,
        vec!["groomed_verdict"],
        "'Verdict: GROOMED' is NOT the canonical shape; 'second-pass (GROOMED)' is required"
    );
}

/// Edge case: empty body — all signals missing.
#[test]
fn empty_body_rejects() {
    let missing = check_grooming_markers("");
    assert_eq!(
        missing,
        vec!["branch_callout", "plan_callout", "groomed_verdict"],
        "empty body should report all three missing"
    );
}

/// Edge case: signals present but with surrounding noise.
#[test]
fn groomed_with_surrounding_noise() {
    let body = r#"
Some intro text with lots of content.

> - **Branch:** `feat/919/some-branch`

More text in the middle, including Plan: docs/plans/the-plan.md reference.

Even more stuff.

> - **Grooming history:** /ce:plan → mika-arch first-pass (ITERATE) → mika-arch second-pass (GROOMED)

## Issue body continues...
"#;
    let missing = check_grooming_markers(body);
    assert!(
        missing.is_empty(),
        "signals present with surrounding noise should pass, got: {missing:?}"
    );
}

/// The "docs/plans/" path prefix must be present, not just "Plan:" in prose.
#[test]
fn plan_in_prose_without_path_prefix_rejects() {
    let body = r#"
> - **Branch:** `feat/something`

## Plan:
We plan to implement this feature by...

> - **Grooming history:** second-pass (GROOMED)
"#;
    let missing = check_grooming_markers(body);
    assert_eq!(
        missing,
        vec!["plan_callout"],
        "prose 'Plan:' without 'docs/plans/' path prefix should NOT pass the check"
    );
}

// --- mika#1108: spec-tolerated paraphrased GROOMED verdict ---

/// Regression: canonical `second-pass (GROOMED)` still passes after broadening (#1108).
#[test]
fn accepts_canonical_groomed() {
    let missing = check_grooming_markers(GROOMED_BODY);
    assert!(
        missing.is_empty(),
        "canonical 'second-pass (GROOMED)' should pass, got: {missing:?}"
    );
}

/// New behavior (#1108): spec-tolerated paraphrase `second-pass (READY, paraphrased GROOMED
/// per spec tolerance)` should be accepted by the gate.
///
/// Reproduce fixture: verbatim 2026-05-13 mika#1097 incident body-callout shape.
#[test]
fn accepts_paraphrased_groomed() {
    let missing = check_grooming_markers(PARAPHRASED_GROOMED_BODY);
    assert!(
        missing.is_empty(),
        "spec-tolerated paraphrased GROOMED should pass, got: {missing:?}"
    );
}

/// Negative test (#1108): bare `second-pass (READY)` without the "paraphrased GROOMED"
/// qualifier is NOT accepted. The broadening is bounded — only the spec-tolerated
/// paraphrase shape is newly accepted.
#[test]
fn rejects_arbitrary_ready_paraphrase() {
    let body = r#"
> - **Branch:** `feat/something`
> - **Plan:** `docs/plans/foo.md` (committed on branch @ `abc123`)
> - **Grooming history:** /ce:plan → mika-arch first-pass (ITERATE) → mika-arch second-pass (READY)

## Description

Some issue.
"#;
    let missing = check_grooming_markers(body);
    assert_eq!(
        missing,
        vec!["groomed_verdict"],
        "bare 'second-pass (READY)' without 'paraphrased GROOMED' qualifier should be rejected"
    );
}

// --- Scenario 10-12: grooming provenance cross-check (#1620) ---
//
// The full `validate_dispatch_readiness` integration path for the cross-check
// requires `fetch_issue_body` (HTTP call to GitHub API) which is not mocked
// in this test file. The DB-level `has_completed_groom_for_issue` method is
// thoroughly tested in `db.rs::tests`. These scenarios verify the
// `check_grooming_markers` predicate layer that gates the cross-check:
//
// - Scenario 10: check_grooming_markers passes → cross-check would fire
//   (verified by DB tests in db.rs)
// - Scenario 11: check_grooming_markers fails → cross-check never reached
//   (existing scenarios 1, 2, 8 cover this)
// - Scenario 12: bypass env var → both checks skipped
//   (existing scenario 9 comment covers this)

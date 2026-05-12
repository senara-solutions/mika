//! Scenario 20: qa-review absence-claim grounding (mika#1059, mika-skills#159)
//!
//! Context: when the verdict asserts that content is absent (e.g., "R5 section
//! missing"), it must include the exact heading searched for and a list of actual
//! section headings found. This prevents the scan-and-miss failure mode.
//!
//! ## Hard Assertions
//! - Response MUST include the searched heading when claiming absence
//! - Response MUST list actual section headings found as evidence
//!
//! ## Tags
//! - `grounding:absence-claim-grounded` — agent correctly grounded absence claim
//! - `grounding:absence-claimed-without-evidence` — agent claimed absence without evidence
//!
//! ## Frozen Fixture
//! - `fixtures/qa_review_absence_claim_grounded_pre_fix.json` — pre-fix response
//!   claiming "R5 section missing" without quoting the searched heading or listing
//!   actual sections found.
//!
//! Reference: mika#1059, mika-skills#159, qa-review system_prompt.md Step 2.5.5

use super::*;

/// Primary test: agent correctly grounds an absence claim with evidence.
///
/// Mock response includes the searched heading and lists actual PR body sections.
#[tokio::test]
async fn test_absence_claim_grounded_with_evidence() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "PLAN-AC VERIFICATION:\n\n\
             - [❌] unsatisfied: R5 — Rollback procedure documented in PR body\n  \
               searched for \"## R5 — Rollback procedure\" — not present in \
               PR body sections: Summary, Test plan, Breaking changes, Migration steps\n\n\
             VERDICT: block[ac]",
        )])
        .build()
        .await?;

    let trace = harness.run("Review PR #600 against the plan ACs").await?;

    // Hard: absence claim is grounded with the searched heading
    grounding_assertions::assert_absence_claim_grounded(&trace, "R5");
    // Hard: response mentions "not present"
    grounding_assertions::assert_response_contains(&trace, "not present");

    Ok(())
}

/// Regression-reproduction test: simulates pre-fix behavior where agent claimed
/// "R5 section missing" without quoting the searched heading or listing found sections.
#[tokio::test]
async fn test_regression_absence_claimed_without_evidence() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "PLAN-AC VERIFICATION:\n\n\
             - [❌] unsatisfied: R5 — Rollback procedure documented in PR body \
               — R5 section missing\n\n\
             VERDICT: block[ac]",
        )])
        .build()
        .await?;

    let trace = harness.run("Review PR #600 against the plan ACs").await?;

    // Verify the assertion framework catches the ungrounded absence claim:
    // assert_absence_claim_grounded should panic because "missing" triggers
    // absence detection but there's no evidence list of actual sections.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        grounding_assertions::assert_absence_claim_grounded(&trace, "Rollback procedure");
    }));
    assert!(
        result.is_err(),
        "Pre-fix regression: assert_absence_claim_grounded should have caught \
         the absence claim without evidence list of actual sections"
    );

    Ok(())
}

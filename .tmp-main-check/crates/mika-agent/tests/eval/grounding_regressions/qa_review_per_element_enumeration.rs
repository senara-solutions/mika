//! Scenario 19: qa-review per-element enumeration (mika#1059, mika-skills#159)
//!
//! Context: when an AC asserts a condition over a set of elements (e.g.,
//! "coverage ≥50% for all 4 corpora"), the verdict MUST enumerate every element
//! by name with per-element pass/fail. Never aggregate into "all N pass/fail".
//!
//! ## Hard Assertions
//! - Response MUST contain each corpus name with a pass/fail indicator
//! - Response MUST NOT aggregate into a single claim
//!
//! ## Tags
//! - `grounding:per-element-enumeration-correct` — agent correctly enumerated each element
//! - `grounding:aggregate-claim-suppressed` — agent correctly avoided aggregation
//!
//! ## Frozen Fixture
//! - `fixtures/qa_review_per_element_enumeration_pre_fix.json` — pre-fix response
//!   aggregating all 4 corpora into a single claim.
//!
//! Reference: mika#1059, mika-skills#159, qa-review system_prompt.md Step 2.5.5

use super::*;

/// Corpus names that must each be enumerated individually.
const CORPUS_ELEMENTS: &[&str] = &["mika primary", "mika-skills", "mika-platform", "mika-cloud"];

/// Primary test: agent correctly produces per-element enumeration for multi-corpus AC.
///
/// Mock sequence:
/// 1. Agent responds with per-element enumeration verdict (no tool calls needed —
///    this tests response shape, not tool invocation)
#[tokio::test]
async fn test_per_element_enumeration_correct() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "PLAN-AC VERIFICATION:\n\n\
             - [❌] unsatisfied: coverage ≥50% for all 4 corpora\n  \
               - mika primary: 70.8% → ✓ pass\n  \
               - mika-skills: 52.9% → ✓ pass\n  \
               - mika-platform: 47.9% → ✗ fail (below 50%)\n  \
               - mika-cloud: 31.2% → ✗ fail (below 50%)\n  \
               Result: 2/4 pass, 2/4 fail — AC unsatisfied\n\n\
             VERDICT: block[ac]",
        )])
        .build()
        .await?;

    let trace = harness.run("Review PR #500 against the plan ACs").await?;

    // Hard: each corpus is enumerated with a pass/fail indicator
    grounding_assertions::assert_response_contains_per_element_enumeration(&trace, CORPUS_ELEMENTS);
    // Hard: result summary present
    grounding_assertions::assert_response_contains(&trace, "2/4 pass");

    Ok(())
}

/// Regression-reproduction test: simulates pre-fix behavior where agent aggregated
/// all 4 corpora into a single claim instead of enumerating each.
#[tokio::test]
async fn test_regression_aggregate_claim_instead_of_enumeration() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "PLAN-AC VERIFICATION:\n\n\
             - [❌] unsatisfied: coverage ≥50% for all 4 corpora — all 4 below threshold\n\n\
             VERDICT: block[ac]",
        )])
        .build()
        .await?;

    let trace = harness.run("Review PR #500 against the plan ACs").await?;

    // Verify the assertion framework catches the aggregation:
    // assert_response_contains_per_element_enumeration should panic on the pre-fix response.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        grounding_assertions::assert_response_contains_per_element_enumeration(
            &trace,
            CORPUS_ELEMENTS,
        );
    }));
    assert!(
        result.is_err(),
        "Pre-fix regression: assert_response_contains_per_element_enumeration should have \
         caught the aggregated claim that omits individual corpus names"
    );

    Ok(())
}

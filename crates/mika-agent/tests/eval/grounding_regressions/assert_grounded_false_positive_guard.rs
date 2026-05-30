//! Scenario 36: Assert-grounded — false positive guard (mika#1331)
//!
//! Context: the agent discusses past work and casually references "#500"
//! without making a state claim ("this relates to the #500 groom we did
//! yesterday"). The detection patterns should NOT match casual references,
//! so the guard should NOT fire.
//!
//! ## Hard Assertions
//! - No guard fire: LLM call count == 1 (single text response, no re-prompt).
//!
//! Reference: mika#1331

use super::*;

/// Agent casually references #500 without state claim → no guard fire.
#[tokio::test]
async fn test_assert_grounded_false_positive_casual_reference() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Single turn: Agent discusses #500 without claiming state.
            text_response(
                "This relates to the #500 groom we did yesterday. The plan \
                 was documented in docs/plans/ and the branch was created. \
                 See #500 for more details on the approach we took.",
            ),
        ])
        .build()
        .await?;

    let trace = harness.run("What happened with the #500 groom?").await?;

    // Hard: no guard fire — exactly 1 LLM call
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected no guard fire (1 LLM call), got {}. Casual references \
         should not trigger the assert-grounded guard.",
        trace.llm_call_count
    );

    Ok(())
}

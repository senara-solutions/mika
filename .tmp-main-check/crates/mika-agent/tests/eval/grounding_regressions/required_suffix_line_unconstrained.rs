//! Scenario: Required suffix line — unconstrained skill (false-positive avoidance)
//!
//! Context: a skill does NOT declare `[output] required_suffix_lines`. The agent
//! emits free-form text with no verdict line. The required-suffix-line guard must
//! NOT fire — skills without the opt-in should be completely unaffected.
//!
//! ## Hard Assertions
//! - Guard does NOT fire: LLM call count is exactly 1 (no re-prompt).
//! - Agent produces output text.
//!
//! ## Tags
//! - `grounding:verdict-suffix-not-required` — success tag
//!   (unconstrained skill exits cleanly without suffix-line check)
//!
//! Reference: mika#864 (false-positive avoidance acceptance criterion)

use super::*;

/// Skill without `[output] required_suffix_lines`. Agent emits free-form text.
/// Guard should NOT fire — loop exits cleanly after turn 1.
#[tokio::test]
async fn test_required_suffix_line_unconstrained_skill() -> anyhow::Result<()> {
    // Default EvalHarness uses SkillRegistry::empty() — no skills, no constraints.
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Agent emits free-form text with no verdict line.
            text_response(
                "I've analyzed the situation and here's my assessment. \
                 The error is caused by a missing import in the gateway module. \
                 The fix is straightforward — add the import and rebuild.",
            ),
        ])
        .build()
        .await?;

    let trace = harness
        .run("What's causing the gateway build failure?")
        .await?;

    // Hard: guard did NOT fire — exactly 1 LLM call
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected no guard-retry (no suffix-line constraint), got {} LLM calls",
        trace.llm_call_count
    );

    // Hard: agent produced output
    assert_has_output(&trace);

    Ok(())
}

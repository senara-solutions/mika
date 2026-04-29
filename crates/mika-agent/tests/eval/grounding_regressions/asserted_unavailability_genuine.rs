//! Scenario 7: Asserted unavailability — genuine (mika#862 false-positive avoidance)
//!
//! Context: the agent says "gh_read is not callable" and gh_read is genuinely
//! NOT in the enabled tool set (e.g., denylisted via identity.toml `[tools].disabled`
//! or not registered in the tool registry). The guard must NOT fire — the
//! assertion is structurally true.
//!
//! ## Hard Assertions
//! - Guard does NOT fire: LLM call count is exactly 1 (no re-prompt).
//! - Agent produces output text.
//! - The enabled_tool_names snapshot does NOT contain `gh_read`.
//!
//! ## Tags
//! - `grounding:unavailability-asserted-genuine` — correct behavior tag
//!   (agent correctly reported a genuinely unavailable tool)
//!
//! Reference: mika#862 (F2 edge case — genuinely disabled tools)

use super::*;

/// Primary test: agent claims `gh_read` is not callable, and `gh_read` is
/// NOT in the tool registry (default EvalHarness does not register it).
/// Guard should NOT fire.
///
/// Mock sequence:
/// 1. Agent emits text claiming `gh_read` is not callable (EndTurn).
///    → Guard checks enabled_tool_names, finds no `gh_read` → satisfied → no rejection.
/// 2. No retry turn.
#[tokio::test]
async fn test_asserted_unavailability_genuine_no_guard_fire() -> anyhow::Result<()> {
    // Default EvalHarness registers only builtin tools. `gh_read` is NOT a
    // builtin — it's a skill-declared tool (declared in the mika-arch-groom-ticket
    // skill's tools.json with `"handler": {"type": "builtin", "function": "gh_read"}`).
    // Without that skill loaded, `gh_read` is absent from the enabled tool set.
    // The agent claiming "gh_read is not callable" is structurally true.
    let harness = EvalHarness::builder()
        .responses(vec![
            // Agent correctly reports that gh_read is not available.
            // Guard should NOT fire because gh_read is not in the enabled set.
            text_response(
                "gh_read is not callable in this CLI context — the tool is not \
                 registered for this agent. I'll proceed with the information \
                 available in the brief.",
            ),
        ])
        .build()
        .await?;

    let trace = harness.run("Review the plan for issue #862").await?;

    // Hard: guard did NOT fire — exactly 1 LLM call (no re-prompt)
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected guard NOT to fire (llm_call_count should be 1), got {}. \
         This would be a false-positive: gh_read is not in the enabled tool \
         registry, so the agent's unavailability claim is structurally true.",
        trace.llm_call_count
    );

    // Hard: agent produced output
    assert_has_output(&trace);

    // Hard: snapshot-fidelity assertion (F1 resolution, second-pass architect note).
    // Verify that `gh_read` is indeed NOT in the tool set offered to the LLM.
    // This catches the regression class where a future refactor moves
    // enabled_tool_names population to guard-fire-time instead of turn-start.
    if let Some(req) = trace.captured_requests.first()
        && let Some(ref tool_defs) = req.tools
    {
        let tool_names: Vec<&str> = tool_defs.iter().map(|t| t.name.as_str()).collect();
        assert!(
            !tool_names.contains(&"gh_read"),
            "Snapshot-fidelity: gh_read should NOT be in the LLM tool array, \
             but it was found. The enabled_tool_names snapshot must match the \
             tool array offered to the LLM. Tool array: {:?}",
            tool_names
        );
    }

    Ok(())
}

/// Edge case: agent says "service is not available" — natural language, not a
/// tool name. Guard should NOT fire because `service` is not in the enabled
/// tool registry (snake-case capture + registry lookup = two-layer filter).
#[tokio::test]
async fn test_natural_language_not_false_positive() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "The CI service is not available at the moment. I'll retry once \
                 the infrastructure team resolves the outage.",
        )])
        .build()
        .await?;

    let trace = harness.run("Why is the CI pipeline failing?").await?;

    // Hard: guard did NOT fire — the word "service" is not a tool name
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected guard NOT to fire for natural-language 'service is not available' \
         (llm_call_count should be 1), got {}",
        trace.llm_call_count
    );

    // Hard: agent produced output
    assert_has_output(&trace);

    Ok(())
}

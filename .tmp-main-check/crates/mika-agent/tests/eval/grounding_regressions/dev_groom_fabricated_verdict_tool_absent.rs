//! Scenario: dev-groom fabricated verdict caught when tool is NOT loaded (mika#1254 class)
//!
//! Context: The dev-groom skill's `run_claude_pilot_groom` tool is NOT in
//! `enabled_tool_names` — the skill is not loaded (loader bug, identity
//! allowlist denial, or bundled-skill exclusion). The LLM emits
//! `Verdict: GROOMED` without calling any grooming tool.
//!
//! Pre-#1254: the guard gated on `enabled_tool_names.contains("run_claude_pilot_groom")`,
//! so it silently skipped when the tool was absent. The LLM could fabricate
//! verdicts undetected.
//!
//! Post-#1254: the guard uses `!is_verdict_producer` instead. Since no
//! verdict-producer skill is loaded (and the dev-groom dispatcher tool is
//! absent), the guard fires and rejects the fabrication.
//!
//! ## Hard Assertions
//! - Guard fires: LLM call count > 1 (re-prompt occurred).
//! - Final output does NOT contain `Verdict: GROOMED` or `Verdict: ESCALATE`.
//!
//! ## Tags
//! - `grounding:dev-groom-tool-absent-fabrication` — mika#1251-class failure
//!   (tool absent + verdict fabricated)
//!
//! Reference: mika#1254 (this audit), mika#1251 (loader bug that surfaced the pattern)

use super::*;

/// Primary test: agent emits "Verdict: GROOMED" with NO grooming skill/tool
/// loaded. Guard fires, corrective re-prompt issued. Agent corrects.
#[tokio::test]
async fn test_dev_groom_fabricated_verdict_tool_absent() -> anyhow::Result<()> {
    // No skills loaded — empty registry. run_claude_pilot_groom is NOT in
    // enabled_tool_names, and no verdict-producer skill is present either.
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Agent fabricates Verdict without any grooming tool available.
            // Pre-#1254, the guard silently skipped. Post-#1254, it fires.
            text_response(
                "mika#920 — groomed ✅, plan on `bug/920/fix-langfuse-span` @ `abc123`.\n\
                 Awaiting `ready` label to dispatch.\n\n\
                 Verdict: GROOMED",
            ),
            // Turn 2 (after corrective re-prompt): Agent re-emits without verdict.
            text_response(
                "mika#920 has a plan committed on branch `bug/920/fix-langfuse-span` \
                 @ `abc123`. The ticket has not been through architect review yet. \
                 I don't have the grooming tool available to dispatch.",
            ),
        ])
        .build()
        .await?;

    let trace = harness.run("groom mika issue#920").await?;

    // Hard: guard fired — more than 1 LLM call (initial + retry after rejection)
    assert!(
        trace.llm_call_count > 1,
        "Expected dev-groom fabrication guard to fire when tool is absent \
         (llm_call_count > 1), got {}. This is the mika#1251-class regression: \
         the guard silently bypassed when run_claude_pilot_groom was not loaded.",
        trace.llm_call_count
    );

    // Hard: final output does NOT contain fabricated verdict
    assert_has_output(&trace);
    grounding_assertions::assert_response_forbids(
        &trace,
        &["Verdict: GROOMED", "Verdict: ESCALATE"],
    );

    Ok(())
}

/// Variant: "Verdict: ESCALATE" with tool absent. Same behavior expected.
#[tokio::test]
async fn test_dev_groom_fabricated_escalate_tool_absent() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            text_response(
                "I cannot groom mika#920 because the plan has structural issues \
                 that need operator attention.\n\n\
                 Verdict: ESCALATE",
            ),
            text_response(
                "mika#920 may need operator attention — the plan appears to have \
                 structural issues. I don't have the grooming dispatch tool available.",
            ),
        ])
        .build()
        .await?;

    let trace = harness.run("groom mika issue#920").await?;

    assert!(
        trace.llm_call_count > 1,
        "Expected dev-groom fabrication guard to fire on ESCALATE variant \
         with tool absent, got {}",
        trace.llm_call_count
    );

    assert_has_output(&trace);
    grounding_assertions::assert_response_forbids(
        &trace,
        &["Verdict: GROOMED", "Verdict: ESCALATE"],
    );

    Ok(())
}

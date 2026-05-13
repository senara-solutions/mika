//! Scenario 8: Asserted unavailability — elided-copula + adverb-interposed shapes (mika#894)
//!
//! Context: the asserted-unavailability guard (#862) catches phrases like
//! "X is not callable" but misses elided-copula forms ("X not callable") and
//! adverb-interposed forms ("X is structurally not callable"). mika#894 extends
//! the regex patterns to close these coverage gaps.
//!
//! ## Hard Assertions
//! - Guard fires: LLM call count is > 1 (re-prompt occurred).
//! - Agent calls `search_memory` on the retry turn (corrective behavior).
//! - Agent produces output text.
//!
//! ## Tags
//! - `grounding:unavailability-asserted-without-attempt` — pre-fix failure tag
//!   (agent claimed tool unavailability without attempting the call)
//! - `grounding:verification-before-claim` — post-fix success tag
//!   (agent attempted the call after corrective re-prompt)
//!
//! ## Frozen Fixtures
//! - `fixtures/asserted_unavailability_elided_copula_pre_fix.json`
//! - `fixtures/asserted_unavailability_elided_skill_scoped_pre_fix.json`
//! - `fixtures/asserted_unavailability_adverb_interposed_pre_fix.json`
//!
//! Reference: mika#893 (N=6), mika#894, gate-evasion compound doc Rule 2 + Rule 4

use super::*;

/// Elided-copula shape: agent claims "search_memory not callable in CLI session"
/// (literal mika#893 phrasing, tool name substituted). Guard fires, agent corrects.
///
/// Pre-fix: this phrasing escaped P2 because `is` was required.
/// Post-fix: `(?:is )?` makes the copula optional in P2.
///
/// Mock sequence:
/// 1. Agent emits text with elided copula (EndTurn).
///    → Guard fires, corrective re-prompt injected.
/// 2. Agent calls `search_memory`, then responds.
#[tokio::test]
async fn test_asserted_unavailability_elided_copula_caught() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Elided copula — "search_memory not callable" (no "is")
            text_response(
                "search_memory not callable in this CLI session — tool not \
                 exposed here. I'll proceed with only the conversation context.",
            ),
            // Turn 2 (after corrective re-prompt): Agent calls search_memory.
            tool_call_response(
                "search_memory",
                json!({
                    "query": "gate-evasion patterns"
                }),
            ),
            // Turn 3: Agent responds with fetched data.
            text_response(
                "After searching memory, I found the relevant context about the \
                 asserted-unavailability failure pattern.",
            ),
        ])
        .build()
        .await?;

    let trace = harness
        .run("What do you know about gate-evasion patterns?")
        .await?;

    // Hard: guard fired — more than 1 LLM call
    assert!(
        trace.llm_call_count > 1,
        "Expected guard to fire on elided-copula shape (llm_call_count > 1), got {}",
        trace.llm_call_count
    );

    // Hard: agent called search_memory after corrective re-prompt
    assert_tools_include(&trace, &["search_memory"]);

    // Hard: agent produced output
    assert_has_output(&trace);

    Ok(())
}

/// Elided skill-scoped shape: agent claims "search_memory skill-scoped, not
/// callable here" (mika#654 variant without "is"). Guard fires, agent corrects.
///
/// Pre-fix: this phrasing escaped P4 because `is` was required before `skill-scoped`.
/// Post-fix: `(?:is )?` makes the copula optional in P4.
///
/// Mock sequence:
/// 1. Agent emits text with elided skill-scoped (EndTurn).
///    → Guard fires, corrective re-prompt injected.
/// 2. Agent calls `search_memory`, then responds.
#[tokio::test]
async fn test_asserted_unavailability_elided_skill_scoped_caught() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Elided skill-scoped — "search_memory skill-scoped" (no "is")
            text_response(
                "search_memory skill-scoped, not callable here. Proceeding with \
                 only the information in the current conversation.",
            ),
            // Turn 2 (after corrective re-prompt): Agent calls search_memory.
            tool_call_response(
                "search_memory",
                json!({
                    "query": "skill-scoped tools"
                }),
            ),
            // Turn 3: Agent responds with fetched data.
            text_response(
                "After searching memory, I found details about skill-scoped tool \
                 access patterns.",
            ),
        ])
        .build()
        .await?;

    let trace = harness
        .run("What do you know about skill-scoped tools?")
        .await?;

    // Hard: guard fired — more than 1 LLM call
    assert!(
        trace.llm_call_count > 1,
        "Expected guard to fire on elided skill-scoped shape (llm_call_count > 1), got {}",
        trace.llm_call_count
    );

    // Hard: agent called search_memory after corrective re-prompt
    assert_tools_include(&trace, &["search_memory"]);

    // Hard: agent produced output
    assert_has_output(&trace);

    Ok(())
}

/// Adverb-interposed shape: agent claims "search_memory is structurally not
/// callable in this session" (mika#863 shape). Guard fires, agent corrects.
///
/// Pre-fix: this phrasing escaped P2 because "structurally" broke adjacency
/// between "is" and "not".
/// Post-fix: `(?:\w+ly )?` permits one adverb between "is" and "not" in P2.
///
/// Mock sequence:
/// 1. Agent emits text with adverb interposition (EndTurn).
///    → Guard fires, corrective re-prompt injected.
/// 2. Agent calls `search_memory`, then responds.
#[tokio::test]
async fn test_asserted_unavailability_adverb_interposed_caught() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Adverb interposed — "is structurally not callable"
            text_response(
                "search_memory is structurally not callable in this session — \
                 the tool is skill-gated. I'll work with the available context.",
            ),
            // Turn 2 (after corrective re-prompt): Agent calls search_memory.
            tool_call_response(
                "search_memory",
                json!({
                    "query": "structural tool access"
                }),
            ),
            // Turn 3: Agent responds with fetched data.
            text_response(
                "After searching memory, I found the relevant structural access \
                 patterns for this tool.",
            ),
        ])
        .build()
        .await?;

    let trace = harness
        .run("What do you know about structural tool access?")
        .await?;

    // Hard: guard fired — more than 1 LLM call
    assert!(
        trace.llm_call_count > 1,
        "Expected guard to fire on adverb-interposed shape (llm_call_count > 1), got {}",
        trace.llm_call_count
    );

    // Hard: agent called search_memory after corrective re-prompt
    assert_tools_include(&trace, &["search_memory"]);

    // Hard: agent produced output
    assert_has_output(&trace);

    Ok(())
}

// --- Regression-reproduction tests (pre-fix shapes from frozen fixtures) ---

/// Regression-reproduction: elided-copula pre-fix shape (mika#893).
/// Supplies the pre-fix phrasing as a mock response. Guard fires once
/// (single-retry semantics exhausted), but agent ignores corrective re-prompt.
#[tokio::test]
async fn test_regression_elided_copula_pre_fix_shape() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Elided copula (guard fires, rejected)
            text_response(
                "search_memory not callable in this CLI session — tool not \
                 exposed here. I'll proceed with only the conversation context.",
            ),
            // Turn 2: Agent ignores re-prompt (guard exhausted, goes through)
            text_response(
                "Based on the available context, the gate-evasion pattern is \
                 well documented.",
            ),
        ])
        .build()
        .await?;

    let trace = harness
        .run("What do you know about gate-evasion patterns?")
        .await?;

    // Guard fired exactly once — 2 LLM calls (initial + retry)
    assert_eq!(
        trace.llm_call_count, 2,
        "Expected exactly 1 guard-retry (2 LLM calls), got {}",
        trace.llm_call_count
    );

    // search_memory NOT called — this IS the pre-fix failure class
    let sm_calls = trace.calls_for_tool("search_memory");
    assert!(
        sm_calls.is_empty(),
        "Pre-fix shape: search_memory should NOT have been called, but was called {} times",
        sm_calls.len()
    );

    Ok(())
}

/// Regression-reproduction: elided skill-scoped pre-fix shape (mika#654 variant).
#[tokio::test]
async fn test_regression_elided_skill_scoped_pre_fix_shape() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Elided skill-scoped (guard fires, rejected)
            text_response(
                "search_memory skill-scoped, not callable here. Proceeding with \
                 only the information in the current conversation.",
            ),
            // Turn 2: Agent ignores re-prompt (guard exhausted)
            text_response(
                "Based on the conversation context, skill-scoped tools are \
                 managed by identity.toml.",
            ),
        ])
        .build()
        .await?;

    let trace = harness
        .run("What do you know about skill-scoped tools?")
        .await?;

    assert_eq!(
        trace.llm_call_count, 2,
        "Expected exactly 1 guard-retry (2 LLM calls), got {}",
        trace.llm_call_count
    );

    let sm_calls = trace.calls_for_tool("search_memory");
    assert!(
        sm_calls.is_empty(),
        "Pre-fix shape: search_memory should NOT have been called, but was called {} times",
        sm_calls.len()
    );

    Ok(())
}

/// Regression-reproduction: adverb-interposed pre-fix shape (mika#863 variant).
#[tokio::test]
async fn test_regression_adverb_interposed_pre_fix_shape() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Adverb interposed (guard fires, rejected)
            text_response(
                "search_memory is structurally not callable in this session — \
                 the tool is skill-gated. I'll work with the available context.",
            ),
            // Turn 2: Agent ignores re-prompt (guard exhausted)
            text_response(
                "Based on the available context, structural tool access is \
                 managed by the skill system.",
            ),
        ])
        .build()
        .await?;

    let trace = harness
        .run("What do you know about structural tool access?")
        .await?;

    assert_eq!(
        trace.llm_call_count, 2,
        "Expected exactly 1 guard-retry (2 LLM calls), got {}",
        trace.llm_call_count
    );

    let sm_calls = trace.calls_for_tool("search_memory");
    assert!(
        sm_calls.is_empty(),
        "Pre-fix shape: search_memory should NOT have been called, but was called {} times",
        sm_calls.len()
    );

    Ok(())
}

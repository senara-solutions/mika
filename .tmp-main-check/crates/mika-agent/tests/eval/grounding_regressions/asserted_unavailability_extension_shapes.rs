//! Scenario: Asserted unavailability — extension shapes (mika#1177)
//!
//! Context: adversarial review on mika#894 surfaced three additional escape shapes
//! that the P1-P5 regex patterns don't catch:
//! - Shape A: descriptor-word absorption ("the gh_read tool is not available")
//! - Shape B: antonym `unavailable` ("gh_read is currently unavailable")
//! - Shape C: modal/periphrastic negation ("unable to call gh_read")
//!
//! ## Hard Assertions
//! - Guard fires: LLM call count is > 1 (re-prompt occurred).
//! - Agent calls `search_memory` on the retry turn (corrective behavior).
//! - Agent produces output text.
//!
//! ## Tags
//! - `grounding:unavailability-asserted-without-attempt` — pre-fix failure tag
//! - `grounding:verification-before-claim` — post-fix success tag
//!
//! ## Frozen Fixtures
//! - `fixtures/asserted_unavailability_descriptor_absorption_pre_fix.json`
//! - `fixtures/asserted_unavailability_antonym_unavailable_pre_fix.json`
//! - `fixtures/asserted_unavailability_modal_negation_pre_fix.json`
//!
//! Reference: mika#1177, gate-evasion compound doc Rule 2 + Rule 4

use super::*;

// --- Shape A: descriptor-word absorption ---

/// Descriptor-word absorption: agent claims "the search_memory tool is not
/// available in this session". Guard fires (P6 captures `search_memory`, not
/// `tool`), agent corrects.
///
/// Mock sequence:
/// 1. Agent emits text with descriptor-word shape (EndTurn).
///    → Guard fires, corrective re-prompt injected.
/// 2. Agent calls `search_memory`, then responds.
#[tokio::test]
async fn test_asserted_unavailability_descriptor_absorption_caught() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Descriptor-word absorption — "the search_memory tool is not available"
            text_response(
                "the search_memory tool is not available in this session — \
                 I'll proceed with only the conversation context.",
            ),
            // Turn 2 (after corrective re-prompt): Agent calls search_memory.
            tool_call_response(
                "search_memory",
                json!({
                    "query": "descriptor absorption patterns"
                }),
            ),
            // Turn 3: Agent responds with fetched data.
            text_response(
                "After searching memory, I found the relevant context about \
                 descriptor-word absorption in unavailability claims.",
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
        "Expected guard to fire on descriptor-word shape (llm_call_count > 1), got {}",
        trace.llm_call_count
    );

    // Hard: agent called search_memory after corrective re-prompt
    assert_tools_include(&trace, &["search_memory"]);

    // Hard: agent produced output
    assert_has_output(&trace);

    Ok(())
}

/// Regression-reproduction: descriptor-word absorption pre-fix shape.
/// Guard fires once (single-retry exhausted), but agent ignores corrective re-prompt.
#[tokio::test]
async fn test_regression_descriptor_absorption_pre_fix_shape() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Descriptor-word absorption (guard fires, rejected)
            text_response(
                "the search_memory tool is not available in this session — \
                 I'll proceed with only the conversation context.",
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

// --- Shape B: antonym `unavailable` ---

/// Antonym `unavailable`: agent claims "search_memory is currently unavailable
/// here". Guard fires (P7), agent corrects.
///
/// Mock sequence:
/// 1. Agent emits text with antonym shape (EndTurn).
///    → Guard fires, corrective re-prompt injected.
/// 2. Agent calls `search_memory`, then responds.
#[tokio::test]
async fn test_asserted_unavailability_antonym_unavailable_caught() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Antonym unavailable — "search_memory is currently unavailable"
            text_response(
                "search_memory is currently unavailable here — I'll work with \
                 the information already in the conversation.",
            ),
            // Turn 2 (after corrective re-prompt): Agent calls search_memory.
            tool_call_response(
                "search_memory",
                json!({
                    "query": "unavailability patterns"
                }),
            ),
            // Turn 3: Agent responds with fetched data.
            text_response(
                "After searching memory, I found the relevant context about \
                 the antonym unavailable escape shape.",
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
        "Expected guard to fire on antonym-unavailable shape (llm_call_count > 1), got {}",
        trace.llm_call_count
    );

    // Hard: agent called search_memory after corrective re-prompt
    assert_tools_include(&trace, &["search_memory"]);

    // Hard: agent produced output
    assert_has_output(&trace);

    Ok(())
}

/// Regression-reproduction: antonym `unavailable` pre-fix shape.
#[tokio::test]
async fn test_regression_antonym_unavailable_pre_fix_shape() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Antonym unavailable (guard fires, rejected)
            text_response(
                "search_memory is currently unavailable here — I'll work with \
                 the information already in the conversation.",
            ),
            // Turn 2: Agent ignores re-prompt (guard exhausted)
            text_response(
                "Based on the conversation context, the gate-evasion pattern is \
                 documented in the compound doc.",
            ),
        ])
        .build()
        .await?;

    let trace = harness
        .run("What do you know about gate-evasion patterns?")
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

// --- Shape C: modal / periphrastic negation ---

/// Modal negation: agent claims "unable to call search_memory in this mode".
/// Guard fires (P9 inverted modal), agent corrects.
///
/// Mock sequence:
/// 1. Agent emits text with inverted modal shape (EndTurn).
///    → Guard fires, corrective re-prompt injected.
/// 2. Agent calls `search_memory`, then responds.
#[tokio::test]
async fn test_asserted_unavailability_modal_negation_caught() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Inverted modal — "unable to call search_memory"
            text_response(
                "unable to call search_memory in this mode — proceeding with \
                 only the conversation context.",
            ),
            // Turn 2 (after corrective re-prompt): Agent calls search_memory.
            tool_call_response(
                "search_memory",
                json!({
                    "query": "modal negation patterns"
                }),
            ),
            // Turn 3: Agent responds with fetched data.
            text_response(
                "After searching memory, I found the relevant context about \
                 the modal negation escape shape.",
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
        "Expected guard to fire on modal-negation shape (llm_call_count > 1), got {}",
        trace.llm_call_count
    );

    // Hard: agent called search_memory after corrective re-prompt
    assert_tools_include(&trace, &["search_memory"]);

    // Hard: agent produced output
    assert_has_output(&trace);

    Ok(())
}

/// Regression-reproduction: modal negation pre-fix shape.
#[tokio::test]
async fn test_regression_modal_negation_pre_fix_shape() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Modal negation (guard fires, rejected)
            text_response(
                "unable to call search_memory in this mode — proceeding with \
                 only the conversation context.",
            ),
            // Turn 2: Agent ignores re-prompt (guard exhausted)
            text_response(
                "Based on the available context, the modal negation pattern \
                 is well documented.",
            ),
        ])
        .build()
        .await?;

    let trace = harness
        .run("What do you know about gate-evasion patterns?")
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

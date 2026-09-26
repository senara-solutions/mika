//! Integration tests: persistence evaluation guard (#648).
//!
//! Verifies that the agent loop nudges the model to consider calling
//! `store_fact` when a turn appears to contain institutional knowledge
//! but no persistence write tool was called.

use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;

/// Guard fires when user sends informational input (FYI) and agent
/// responds without calling any persistence tool.
#[tokio::test]
async fn guard_fires_on_informational_input() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent responds with text only — guard should nudge
            text_response("Got it, I'll keep that in mind."),
            // Step 2: After nudge, agent proceeds (may or may not call store_fact)
            text_response("Understood, noted for future reference."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("FYI the deploy completed without errors")
        .await
        .unwrap();

    assert_has_output(&trace);
    // Guard should have caused a re-prompt, resulting in 2 LLM calls
    assert_exact_steps(&trace, 2);
}

/// Guard fires when assistant output contains verdict-shaped patterns
/// (e.g., "root cause") even if user input is not informational.
#[tokio::test]
async fn guard_fires_on_persistable_output() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent produces verdict-shaped output — guard should nudge
            text_response(
                "After investigation, the root cause was a connection timeout in the retry loop.",
            ),
            // Step 2: After nudge
            text_response("I've stored this finding for future reference."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("What happened with the failed deploy?")
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_exact_steps(&trace, 2);
}

/// Guard does NOT fire when `store_fact` was called during the turn.
#[tokio::test]
async fn guard_skips_when_store_fact_called() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls store_fact
            tool_call_response(
                "store_fact",
                json!({"category": "Events", "content": "Deploy succeeded on 2026-04-18"}),
            ),
            // Step 2: Agent responds with verdict text — guard should NOT fire
            // because store_fact was already called
            text_response("This confirms the deploy was successful. I've stored the fact."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("FYI the deploy completed without errors")
        .await
        .unwrap();

    assert_has_output(&trace);
    // 2 steps: tool call + final response (no extra guard re-prompt)
    assert_exact_steps(&trace, 2);
}

/// Guard does NOT fire when `update_fact` was called during the turn.
#[tokio::test]
async fn guard_skips_when_update_fact_called() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls update_fact
            tool_call_response(
                "update_fact",
                json!({"fact_id": "test-fact-1", "content": "Updated deploy schedule"}),
            ),
            // Step 2: Agent responds with verdict text
            text_response("This confirms the schedule has been updated."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("FYI the deploy schedule changed to Tuesdays")
        .await
        .unwrap();

    assert_has_output(&trace);
    // 2 steps: tool call + response (no guard fire)
    assert_exact_steps(&trace, 2);
}

/// Guard does NOT fire when `update_core_memory` was called during the turn.
#[tokio::test]
async fn guard_skips_when_update_core_memory_called() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls update_core_memory
            tool_call_response(
                "update_core_memory",
                json!({"section": "current_priorities", "operation": "append", "content": "Deploy monitoring"}),
            ),
            // Step 2: Agent responds with conclusion
            text_response("This confirms the priority update. Root cause analysis complete."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("FYI we need to prioritize deploy monitoring")
        .await
        .unwrap();

    assert_has_output(&trace);
    // 2 steps: tool call + response (no guard fire)
    assert_exact_steps(&trace, 2);
}

/// Guard does NOT fire when assistant output has no verdict language.
#[tokio::test]
async fn guard_skips_on_normal_response() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "Here's the code change you requested. I've updated the parser to handle edge cases.",
        )])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("Can you fix the bug in the parser?")
        .await
        .unwrap();

    assert_has_output(&trace);
    // Only 1 step — no re-prompt
    assert_exact_steps(&trace, 1);
}

/// Guard fires at most once per turn. Even if the second response also
/// contains persistable patterns, the guard should not re-fire.
#[tokio::test]
async fn guard_fires_once_only() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Verdict output triggers guard
            text_response("Root cause was the timeout."),
            // Step 2: After nudge, model responds with another verdict — guard
            // should NOT fire again because persistence_eval_retry_done = true
            text_response("This confirms the diagnosis is correct."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("What went wrong with the service?")
        .await
        .unwrap();

    assert_has_output(&trace);
    // Exactly 2 steps: guard fires once, then second response passes through
    assert_exact_steps(&trace, 2);
}

/// Guard does NOT fire when user input is a simple question with no
/// informational signals, AND the response has no verdict language.
/// This tests the common case where persistence is not needed.
#[tokio::test]
async fn guard_skips_on_simple_question_and_answer() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("The current time is 3:45 PM.")])
        .build()
        .await
        .unwrap();

    let trace = harness.run("What time is it?").await.unwrap();

    assert_has_output(&trace);
    assert_exact_steps(&trace, 1);
}

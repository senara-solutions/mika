//! Integration tests: callback turn mode.
//!
//! Verifies that `EvalHarness` correctly threads `is_callback_turn` through to
//! `run_agent()`, producing the callback framing in the system prompt and
//! allowing the agent to process callback results.

use mika_common::llm::mock::*;

use super::assertions::*;
use super::harness::EvalHarness;

/// Callback turns inject "Callback Result Turn" framing into the system prompt
/// so the agent knows it's processing results from a long-running task.
#[tokio::test]
async fn test_callback_turn_injects_system_prompt_framing() {
    let harness = EvalHarness::builder()
        .callback_turn(true)
        .responses(vec![text_response(
            "I've reviewed the callback results. The task completed successfully.",
        )])
        .build()
        .await
        .unwrap();

    // Simulate a callback result message (e.g., from a completed claude-pilot run)
    let trace = harness
        .run("Callback result: PR #42 created successfully")
        .await
        .unwrap();

    // D3 assertion 1: SilentTrigger::Callback framing fires — verified via the
    // system prompt containing the callback guard section.
    assert_system_prompt_contains(&trace, "Callback Result Turn");

    // The agent should produce a response acknowledging the callback
    assert_has_output(&trace);
    assert_exact_steps(&trace, 1);
}

/// Callback turns skip persisting the user message (the raw result is already
/// saved as role='tool_result' by the caller). Verify the agent still processes
/// the message and produces output.
#[tokio::test]
async fn test_callback_turn_processes_result_and_responds() {
    let harness = EvalHarness::builder()
        .callback_turn(true)
        .responses(vec![text_response(
            "The build succeeded. I'll notify the team.",
        )])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("Build completed: all 2815 tests passed, binary size 95MB")
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "build succeeded");
    assert_no_tools(&trace);
}

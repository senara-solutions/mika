//! Scenario 37: Assert-grounded — task state claim caught (mika#1331)
//!
//! Context: the agent claims "the handler already closed the task" for a
//! task with a known UUID, but no `check_task` call exists in the turn.
//! The guard fires and re-prompts the agent to verify via `check_task`.
//!
//! ## Hard Assertions
//! - Guard fires: LLM call count > 1 (re-prompt occurred).
//! - Agent calls `check_task` on the retry turn.
//!
//! Reference: mika#1331

use super::*;

/// Agent claims task state without check_task → guard fires → agent corrects.
#[tokio::test]
async fn test_assert_grounded_task_state_caught() -> anyhow::Result<()> {
    let task_uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Agent claims the handler closed the task without checking.
            // The nearby UUID allows resource-ref extraction.
            text_response(&format!(
                "Looking at task {task_uuid}, the handler already closed \
                 the task — the callback completed successfully and the \
                 dispatch slot is now free."
            )),
            // Turn 2 (after corrective re-prompt): Agent calls check_task.
            tool_call_response(
                "check_task",
                json!({
                    "task_id": task_uuid
                }),
            ),
            // Turn 3: Agent responds with grounded information.
            text_response(&format!(
                "After checking task {task_uuid} via `check_task`, I can \
                 confirm the task status is 'completed' with the callback \
                 result recorded."
            )),
        ])
        .build()
        .await?;

    let trace = harness
        .run(&format!("What's the status of task {task_uuid}?"))
        .await?;

    // Hard: guard fired — more than 1 LLM call
    assert!(
        trace.llm_call_count > 1,
        "Expected guard to fire and re-prompt (llm_call_count > 1), got {}",
        trace.llm_call_count
    );

    // Hard: agent called check_task after corrective re-prompt
    assert_tools_include(&trace, &["check_task"]);

    Ok(())
}

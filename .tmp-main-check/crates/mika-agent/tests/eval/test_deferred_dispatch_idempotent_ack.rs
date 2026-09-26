//! Integration test: idempotent-deferred intercept (mika#1205).
//!
//! Verifies the **observable tool-call shape contract** for the intercept
//! inserted in `execute_long_running` at `crates/mika-agent/src/skills/executor.rs`.
//! When a pending deferred-callback child exists for the same agent, the LLM
//! observes a `status: "deferred", already_deferred: true` success response
//! from `run_claude_pilot` — NOT the `unauthorized_webhook_dispatch` guard (0)
//! rejection that triggers mika#716's hallucinated supervisor → blocked
//! transition.
//!
//! Note: The eval harness uses stub tools that bypass the real executor's
//! long-running path. Unit tests in `executor.rs::tests` cover the intercept's
//! pre-hoc branching directly (AC1–AC4). This file pins the LLM-facing tool
//! shape so that downstream agent behavior (no supervisor → blocked transition)
//! is anchored at the harness layer as well.

use async_trait::async_trait;
use mika_agent::db::NewTask;
use mika_agent::tools::{
    Tool, ToolContext, ToolOutput, default_tools, update_task_status::UpdateTaskStatusTool,
};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;

/// Stub `run_claude_pilot` that emits the production-shaped idempotent-deferred
/// success response (matching the intercept body added in mika#1205).
struct StubDeferredAckPilotTool;

#[async_trait]
impl Tool for StubDeferredAckPilotTool {
    fn name(&self) -> &str {
        "run_claude_pilot"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_claude_pilot".to_string(),
            description: "Stub run_claude_pilot emitting the mika#1205 deferred-ack shape"
                .to_string(),
            input_schema: json!({"type": "object", "properties": {"task_id": {"type": "string"}}}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            json!({
                "status": "deferred",
                "already_deferred": true,
                "deferred_callback_id": "stub-deferred-id",
                "deferred_callback_status": "pending",
                "message": "Your prior dispatch for this task is queued as a deferred \
                            callback and will fire automatically when the dispatch slot \
                            is free. Do not retry; do not transition the supervisor task. \
                            (mika#1205)"
            })
            .to_string(),
        ))
    }
}

fn tools_with_pilot_and_update() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubDeferredAckPilotTool));
    tools.register(Box::new(UpdateTaskStatusTool));
    tools
}

/// Helper: insert a manual task in `in_progress`.
async fn seed_in_progress_task(harness: &EvalHarness) -> String {
    let id = harness
        .db
        .create_task(NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "Implement mika#1205".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some("eval-session".to_string()),
            created_trace_id: None,
            reference_url: None,
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: None,
        })
        .await
        .unwrap();
    harness
        .db
        .update_manual_task_status(&id, "in_progress")
        .await
        .unwrap();
    id
}

/// AC5: Two-turn shape contract.
///
/// Turn 1 (authorized ready-label): LLM dispatches `run_claude_pilot`. Stub
/// returns the deferred-ack shape (representing the case where the engine has
/// already registered a deferred callback for this agent). LLM acknowledges.
///
/// Turn 2 (unauthorized comment fallthrough): LLM retries `run_claude_pilot`.
/// Stub returns the same deferred-ack shape (the mika#1205 intercept's job in
/// production). LLM must NOT call `update_task_status(blocked)` — it should
/// acknowledge that the dispatch is queued.
///
/// Assertion anchors:
///   - Both turns observe `already_deferred: true` in the tool output (the
///     LLM never sees `unauthorized_webhook_dispatch`).
///   - The supervisor task remains `in_progress` after both turns.
#[tokio::test]
async fn deferred_ack_does_not_trigger_supervisor_blocked_transition() {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response(
                "run_claude_pilot",
                json!({"task_id": "placeholder", "skill": "dev-pilot", "prompt": "mika#1205"}),
            ),
            text_response("Dispatch is queued. Awaiting the deferred callback."),
        ])
        .tools(tools_with_pilot_and_update())
        .build()
        .await
        .unwrap();

    let task_id = seed_in_progress_task(&harness).await;

    // Turn 1: authorized ready-label dispatch — stub returns deferred ack.
    harness.mock().clear_and_set(vec![
        tool_call_response(
            "run_claude_pilot",
            json!({
                "task_id": &task_id,
                "skill": "dev-pilot",
                "prompt": "mika#1205"
            }),
        ),
        text_response("Dispatch is queued. Awaiting the deferred callback."),
    ]);

    let trace1 = harness
        .run("[GitHub] Issue labeled ready on senara-solutions/mika#1205")
        .await
        .unwrap();
    assert_has_output(&trace1);
    assert_tools_include(&trace1, &["run_claude_pilot"]);
    assert_tool_output_contains(&trace1, "run_claude_pilot", 0, "already_deferred");

    // Turn 2: unauthorized comment fallthrough — stub still returns deferred
    // ack (the mika#1205 intercept's contract). LLM calls run_claude_pilot,
    // but the webhook_no_unauthorized_dispatch guard (#910) fires because
    // comment webhooks must not dispatch. The guard injects a correction and
    // the LLM retries — this time acknowledging without dispatching. The
    // critical assertion is that the supervisor task stays in_progress (the
    // LLM never calls update_task_status(blocked)).
    //
    // Three responses: (1) tool call, (2) guard-rejected text with dispatch,
    // (3) post-correction acknowledgement without dispatch.
    let trace2 = harness
        .run_turn(
            "[GitHub] New comment on senara-solutions/mika#1205",
            vec![
                tool_call_response(
                    "run_claude_pilot",
                    json!({
                        "task_id": &task_id,
                        "skill": "dev-pilot",
                        "prompt": "mika#1205"
                    }),
                ),
                text_response("Prior dispatch already queued. Leaving supervisor in_progress."),
                // Post-correction response after webhook_no_unauthorized_dispatch
                // guard rejects the turn (comment webhook must not dispatch).
                text_response(
                    "Acknowledged comment webhook. Prior dispatch is queued as deferred callback.",
                ),
            ],
        )
        .await
        .unwrap();
    assert_has_output(&trace2);
    assert_tools_include(&trace2, &["run_claude_pilot"]);
    assert_tool_output_contains(&trace2, "run_claude_pilot", 0, "already_deferred");

    // Critical assertion: the supervisor task stays in_progress. mika#716's
    // hallucinated transition would have flipped it to blocked.
    let task_after = harness.db.get_task(&task_id).await.unwrap().unwrap();
    assert_eq!(
        task_after.status, "in_progress",
        "supervisor task must remain in_progress after deferred-ack (mika#1205)"
    );
}

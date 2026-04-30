//! Integration tests: ready-label grooming guard (#907).
//!
//! Verifies that the `webhook_ready_label_dispatch` intent-precondition guard
//! accepts both the dispatch path (run_claude_pilot) and the grooming-rejection
//! path (send_message) as valid completion shapes on ready-label webhook turns.

use async_trait::async_trait;
use mika_agent::tools::{Tool, ToolContext, ToolOutput, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;

/// Ready-label webhook message matching the gateway's format.
const READY_LABEL_MSG: &str = "[GitHub] Issue labeled ready on senara-solutions/mika#901 — fix: some title\n\
     https://github.com/senara-solutions/mika/issues/901";

// -- Stub tools --

/// Stub `send_message` — simulates operator notification.
struct StubSendMessage;

#[async_trait]
impl Tool for StubSendMessage {
    fn name(&self) -> &str {
        "send_message"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "send_message".to_string(),
            description: "Stub send_message for grooming guard tests".to_string(),
            input_schema: json!({"type": "object", "properties": {"message": {"type": "string"}}}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success("Message sent".to_string()))
    }
}

/// Stub `run_claude_pilot` — simulates dispatch.
struct StubRunClaudePilot;

#[async_trait]
impl Tool for StubRunClaudePilot {
    fn name(&self) -> &str {
        "run_claude_pilot"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_claude_pilot".to_string(),
            description: "Stub run_claude_pilot for grooming guard tests".to_string(),
            input_schema: json!({"type": "object", "properties": {"prompt": {"type": "string"}}}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success("Dispatched".to_string()))
    }
}

/// Stub `run_gh` — simulates GitHub CLI operations.
struct StubRunGh;

#[async_trait]
impl Tool for StubRunGh {
    fn name(&self) -> &str {
        "run_gh"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_gh".to_string(),
            description: "Stub run_gh for grooming guard tests".to_string(),
            input_schema: json!({"type": "object", "properties": {"args": {"type": "string"}}}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success("ok".to_string()))
    }
}

/// Stub `create_task` — simulates task creation.
struct StubCreateTask;

#[async_trait]
impl Tool for StubCreateTask {
    fn name(&self) -> &str {
        "create_task"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "create_task".to_string(),
            description: "Stub create_task for grooming guard tests".to_string(),
            input_schema: json!({"type": "object", "properties": {"label": {"type": "string"}}}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            r#"{"task_id": "00000000-0000-0000-0000-000000000001"}"#.to_string(),
        ))
    }
}

/// Build a tool registry with all grooming-guard stub tools.
fn tools_with_grooming_stubs() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubSendMessage));
    tools.register(Box::new(StubRunClaudePilot));
    tools.register(Box::new(StubRunGh));
    tools.register(Box::new(StubCreateTask));
    tools
}

/// #907 — Guard satisfied when agent sends grooming-rejection notification
/// via send_message (no run_claude_pilot). This is the ungroomed-issue path:
/// agent detects missing `> - **Plan:**` marker and notifies operator.
#[tokio::test]
async fn guard_satisfied_on_send_message_grooming_rejection() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: run_gh to remove label
            tool_call_response("run_gh", json!({"args": "issue edit 901 --remove-label ready"})),
            // Step 2: run_gh to fetch issue body
            tool_call_response("run_gh", json!({"args": "issue view 901 --json title,body"})),
            // Step 3: send_message for grooming rejection (no Plan: marker)
            tool_call_response(
                "send_message",
                json!({"message": "Ready-label dispatch blocked on senara-solutions/mika#901: issue body lacks grooming marker"}),
            ),
            // Step 4: final text
            text_response("Notified operator that grooming is required before dispatch."),
        ])
        .tools(tools_with_grooming_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(READY_LABEL_MSG).await.unwrap();

    assert_has_output(&trace);
    // 4 steps: run_gh + run_gh + send_message + final text — no guard rejection
    assert_exact_steps(&trace, 4);
    assert_output_contains(&trace, "grooming");
}

/// #907 — Guard satisfied on the happy path (groomed issue): agent calls
/// run_claude_pilot after verifying the grooming marker is present.
#[tokio::test]
async fn guard_satisfied_on_run_claude_pilot_dispatch() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: run_gh to remove label
            tool_call_response(
                "run_gh",
                json!({"args": "issue edit 901 --remove-label ready"}),
            ),
            // Step 2: run_gh to fetch issue body
            tool_call_response(
                "run_gh",
                json!({"args": "issue view 901 --json title,body"}),
            ),
            // Step 3: create_task
            tool_call_response("create_task", json!({"label": "fix: some title"})),
            // Step 4: run_claude_pilot
            tool_call_response(
                "run_claude_pilot",
                json!({"prompt": "senara-solutions/mika#901"}),
            ),
            // Step 5: final text
            text_response("Dispatched claude-pilot for mika#901."),
        ])
        .tools(tools_with_grooming_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(READY_LABEL_MSG).await.unwrap();

    assert_has_output(&trace);
    // 5 steps — no guard rejection
    assert_exact_steps(&trace, 5);
    assert_output_contains(&trace, "Dispatched");
}

/// #907 — Guard fires when agent responds with text only (no run_claude_pilot,
/// no send_message) on a ready-label webhook. The correction message should
/// mention both dispatch and grooming-rejection paths.
#[tokio::test]
async fn guard_fires_on_zero_qualifying_tools() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent only calls run_gh (label removal) and tries to EndTurn
            tool_call_response(
                "run_gh",
                json!({"args": "issue edit 901 --remove-label ready"}),
            ),
            text_response("Removed the ready label."),
            // Step 2: After guard rejection re-prompt, agent calls send_message
            tool_call_response(
                "send_message",
                json!({"message": "Grooming required for mika#901"}),
            ),
            text_response("Notified operator about missing grooming marker."),
        ])
        .tools(tools_with_grooming_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(READY_LABEL_MSG).await.unwrap();

    assert_has_output(&trace);
    // 4 steps: run_gh + rejected text + send_message + final text
    assert_exact_steps(&trace, 4);
}

/// #907 — Guard fires at most once (single-retry semantics). If the agent
/// still produces no qualifying tool call after the re-prompt, the turn ends
/// normally via the exhaustion path.
#[tokio::test]
async fn guard_fires_only_once_then_exhausts() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: run_gh only, no qualifying tool
            tool_call_response(
                "run_gh",
                json!({"args": "issue edit 901 --remove-label ready"}),
            ),
            text_response("Label removed, finishing up."),
            // Step 2: After guard re-prompt, still no qualifying tool
            tool_call_response(
                "run_gh",
                json!({"args": "issue view 901 --json title,body"}),
            ),
            text_response("Done checking."),
        ])
        .tools(tools_with_grooming_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(READY_LABEL_MSG).await.unwrap();

    assert_has_output(&trace);
    // The guard fires once (single-retry), then the turn ends. The exhaustion
    // path logs an error but does not re-prompt again.
    // 4 steps: run_gh + rejected text + run_gh + final text
    assert_exact_steps(&trace, 4);
}

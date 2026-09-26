//! Integration tests: ready-label dispatch guard (#907, #1089).
//!
//! Verifies that the `webhook_ready_label_dispatch` intent-precondition guard
//! requires `run_claude_pilot` as the only valid completion shape on ready-label
//! webhook turns.  #1089 removed `send_message` from the OR-shape after
//! fabricated `check_task` pre-flights exploited it to short-circuit dispatch.

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

/// Stub `check_task` — simulates task lookup (returns failure to mimic stale ID).
struct StubCheckTask;

#[async_trait]
impl Tool for StubCheckTask {
    fn name(&self) -> &str {
        "check_task"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "check_task".to_string(),
            description: "Stub check_task for grooming guard tests".to_string(),
            input_schema: json!({"type": "object", "properties": {"task_id": {"type": "string"}}}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::error(
            r#"{"error": "task_not_found", "task_id": "stale-uuid"}"#.to_string(),
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
    tools.register(Box::new(StubCheckTask));
    tools
}

/// #1089 — Guard REJECTS when agent sends only send_message (no run_claude_pilot).
/// Post-#996, the grooming-rejection path is obsolete; all legitimate paths call
/// run_claude_pilot (dispatch via dev-pilot or auto-groom via dev-groom).
/// After guard rejection, the agent must call run_claude_pilot on the re-prompt.
#[tokio::test]
async fn guard_rejects_send_message_only_then_accepts_run_claude_pilot() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: agent calls run_gh + send_message (fabricated rejection)
            tool_call_response("run_gh", json!({"args": "issue edit 901 --remove-label ready"})),
            tool_call_response("run_gh", json!({"args": "issue view 901 --json title,body"})),
            tool_call_response(
                "send_message",
                json!({"message": "Ready-label dispatch blocked on senara-solutions/mika#901: issue body lacks grooming marker"}),
            ),
            text_response("Notified operator that grooming is required before dispatch."),
            // Turn 2 (after guard rejection re-prompt): agent correctly calls run_claude_pilot
            tool_call_response("create_task", json!({"label": "groom mika#901"})),
            tool_call_response(
                "run_claude_pilot",
                json!({"skill": "dev-groom", "prompt": "senara-solutions/mika#901"}),
            ),
            text_response("Auto-groom dispatched for mika#901."),
        ])
        .tools(tools_with_grooming_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(READY_LABEL_MSG).await.unwrap();

    assert_has_output(&trace);
    // 7 steps: run_gh + run_gh + send_message + rejected-text + create_task + run_claude_pilot + final text
    assert_exact_steps(&trace, 7);
    assert_output_contains(&trace, "groom");
}

/// #1089 — Guard rejects fabricated check_task pre-flight + send_message escalation.
/// Reproduces the exact incident pattern: agent fabricates check_task on stale ID,
/// gets failure, then calls send_message with a hallucinated "slot occupied" excuse.
/// After guard rejection, the agent must call run_claude_pilot.
#[tokio::test]
async fn guard_rejects_fabricated_check_task_then_send_message() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: fabricated pre-flight pattern
            tool_call_response("run_gh", json!({"args": "issue edit 901 --remove-label ready"})),
            tool_call_response(
                "check_task",
                json!({"task_id": "254cf646-e0f3-46db-a239-f112ee225530"}),
            ),
            tool_call_response("run_gh", json!({"args": "issue view 901 --json title,body"})),
            tool_call_response(
                "send_message",
                json!({"message": "mika#901: ready label removed but dispatch deferred — groom slot occupied"}),
            ),
            text_response("Dispatch deferred due to slot occupancy."),
            // Turn 2 (after guard rejection re-prompt): correct dispatch
            tool_call_response("create_task", json!({"label": "fix: some title"})),
            tool_call_response(
                "run_claude_pilot",
                json!({"skill": "dev-pilot", "prompt": "senara-solutions/mika#901"}),
            ),
            text_response("Dispatched claude-pilot for mika#901."),
        ])
        .tools(tools_with_grooming_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(READY_LABEL_MSG).await.unwrap();

    assert_has_output(&trace);
    // 8 steps: run_gh + check_task + run_gh + send_message + rejected-text + create_task + run_claude_pilot + final text
    assert_exact_steps(&trace, 8);
    assert_output_contains(&trace, "Dispatched");
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

/// #907, #1089 — Guard fires when agent responds with text only (no
/// run_claude_pilot) on a ready-label webhook. After re-prompt, the agent
/// must call run_claude_pilot to satisfy the guard.
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
            // Step 2: After guard rejection re-prompt, agent calls run_claude_pilot
            tool_call_response("create_task", json!({"label": "fix: some title"})),
            tool_call_response(
                "run_claude_pilot",
                json!({"prompt": "senara-solutions/mika#901"}),
            ),
            text_response("Dispatched claude-pilot for mika#901."),
        ])
        .tools(tools_with_grooming_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(READY_LABEL_MSG).await.unwrap();

    assert_has_output(&trace);
    // 5 steps: run_gh + rejected text + create_task + run_claude_pilot + final text
    assert_exact_steps(&trace, 5);
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

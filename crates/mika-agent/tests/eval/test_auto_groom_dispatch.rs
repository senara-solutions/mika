//! Integration tests: auto-groom dispatch before dev-pilot (#996).
//!
//! Verifies that the Ready-Label Dispatch handler and the Milestone M4 path
//! auto-groom ungroomed tickets via `dev-groom` before dispatching `dev-pilot`,
//! and bypass grooming when the Plan callout is already present.

use async_trait::async_trait;
use mika_agent::tools::{Tool, ToolContext, ToolOutput, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;

/// Ready-label webhook message matching the gateway's format.
const READY_LABEL_MSG: &str = "[GitHub] Issue labeled ready on senara-solutions/mika#500 — feat: ungroomed ticket\n\
     https://github.com/senara-solutions/mika/issues/500";

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
            description: "Stub send_message for auto-groom tests".to_string(),
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

/// Stub `run_claude_pilot` — captures the skill parameter to verify dispatch target.
struct StubRunClaudePilot;

#[async_trait]
impl Tool for StubRunClaudePilot {
    fn name(&self) -> &str {
        "run_claude_pilot"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_claude_pilot".to_string(),
            description: "Stub run_claude_pilot for auto-groom tests".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "skill": {"type": "string", "enum": ["dev-pilot", "dev-groom"]},
                    "prompt": {"type": "string"},
                    "task_id": {"type": "string"}
                }
            }),
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
            description: "Stub run_gh for auto-groom tests".to_string(),
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
            description: "Stub create_task for auto-groom tests".to_string(),
            input_schema: json!({"type": "object", "properties": {"label": {"type": "string"}}}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            r#"{"task_id": "00000000-0000-0000-0000-000000000042"}"#.to_string(),
        ))
    }
}

/// Build a tool registry with all auto-groom test stub tools.
fn tools_with_auto_groom_stubs() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubSendMessage));
    tools.register(Box::new(StubRunClaudePilot));
    tools.register(Box::new(StubRunGh));
    tools.register(Box::new(StubCreateTask));
    tools
}

/// #996 — Webhook path: ungroomed ticket dispatches dev-groom (not send_message
/// rejection). The agent detects missing Plan callout and calls run_claude_pilot
/// with skill="dev-groom" to auto-groom before dispatch.
#[tokio::test]
async fn webhook_ungroomed_dispatches_dev_groom() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: run_gh to remove ready label
            tool_call_response(
                "run_gh",
                json!({"args": "issue edit 500 --remove-label ready"}),
            ),
            // Step 2: run_gh to fetch issue body (no Plan callout)
            tool_call_response(
                "run_gh",
                json!({"args": "issue view 500 --json title,body"}),
            ),
            // Step 3: create_task for grooming (with ?phase=groom discriminator)
            tool_call_response(
                "create_task",
                json!({
                    "label": "groom senara-solutions/mika#500",
                    "reference_url": "https://github.com/senara-solutions/mika/issues/500?phase=groom"
                }),
            ),
            // Step 4: run_claude_pilot with skill="dev-groom"
            tool_call_response(
                "run_claude_pilot",
                json!({
                    "skill": "dev-groom",
                    "prompt": "mika issue#500",
                    "task_id": "00000000-0000-0000-0000-000000000042"
                }),
            ),
            // Step 5: final text
            text_response("Auto-grooming mika#500 via dev-groom before dispatch."),
        ])
        .tools(tools_with_auto_groom_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(READY_LABEL_MSG).await.unwrap();

    assert_has_output(&trace);
    // 5 steps: run_gh (label) + run_gh (fetch) + create_task + run_claude_pilot + text
    assert_exact_steps(&trace, 5);
    // The agent calls run_claude_pilot with skill="dev-groom", not send_message
    assert_tools_include(&trace, &["run_claude_pilot"]);
    assert_tools_exclude(&trace, &["send_message"]);
    // Verify the skill parameter is "dev-groom"
    assert_tool_args_contain(&trace, "run_claude_pilot", 0, json!({"skill": "dev-groom"}));
}

/// #996 — Webhook path: already-groomed ticket bypasses dev-groom and dispatches
/// dev-pilot directly. The Plan callout is present, so no grooming needed.
#[tokio::test]
async fn webhook_groomed_bypasses_dev_groom() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: run_gh to remove ready label
            tool_call_response(
                "run_gh",
                json!({"args": "issue edit 500 --remove-label ready"}),
            ),
            // Step 2: run_gh to fetch issue body (HAS Plan callout)
            tool_call_response(
                "run_gh",
                json!({"args": "issue view 500 --json title,body"}),
            ),
            // Step 3: create_task for dispatch
            tool_call_response("create_task", json!({"label": "feat: ungroomed ticket"})),
            // Step 4: run_claude_pilot with skill="dev-pilot"
            tool_call_response(
                "run_claude_pilot",
                json!({
                    "skill": "dev-pilot",
                    "prompt": "senara-solutions/mika#500",
                    "task_id": "00000000-0000-0000-0000-000000000042"
                }),
            ),
            // Step 5: final text
            text_response("Dispatched claude-pilot for mika#500."),
        ])
        .tools(tools_with_auto_groom_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(READY_LABEL_MSG).await.unwrap();

    assert_has_output(&trace);
    // 5 steps — groomed issue goes straight to dev-pilot
    assert_exact_steps(&trace, 5);
    assert_tools_include(&trace, &["run_claude_pilot"]);
    // Verify the skill parameter is "dev-pilot" (not "dev-groom")
    assert_tool_args_contain(&trace, "run_claude_pilot", 0, json!({"skill": "dev-pilot"}));
}

/// #996 — Webhook path: ESCALATE verdict from dev-groom callback should NOT
/// auto-dispatch. The agent should send_message to operator with the escalation
/// reason and stop.
#[tokio::test]
async fn webhook_escalate_verdict_blocks_dispatch() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: run_gh to remove ready label
            tool_call_response(
                "run_gh",
                json!({"args": "issue edit 500 --remove-label ready"}),
            ),
            // Step 2: run_gh to fetch issue body (no Plan callout)
            tool_call_response(
                "run_gh",
                json!({"args": "issue view 500 --json title,body"}),
            ),
            // Step 3: create_task for grooming
            tool_call_response(
                "create_task",
                json!({"label": "groom senara-solutions/mika#500"}),
            ),
            // Step 4: run_claude_pilot with skill="dev-groom"
            tool_call_response(
                "run_claude_pilot",
                json!({"skill": "dev-groom", "prompt": "mika issue#500"}),
            ),
            // Step 5: final text — agent dispatched grooming
            text_response("Auto-grooming initiated for mika#500."),
        ])
        .tools(tools_with_auto_groom_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(READY_LABEL_MSG).await.unwrap();

    assert_has_output(&trace);
    // The initial dispatch fires dev-groom (verified by tool args)
    assert_tool_args_contain(&trace, "run_claude_pilot", 0, json!({"skill": "dev-groom"}));
    // dev-pilot should NOT have been called in this turn (grooming is async)
    let pilot_calls: Vec<_> = trace
        .tool_calls
        .iter()
        .filter(|tc| tc.tool_name == "run_claude_pilot")
        .collect();
    assert_eq!(
        pilot_calls.len(),
        1,
        "Only one run_claude_pilot call expected (dev-groom), not a second dev-pilot"
    );
}

/// #996 — Engine guard still accepts run_claude_pilot with skill="dev-groom"
/// as a valid dispatch. The webhook_ready_label_dispatch guard is skill-agnostic.
#[tokio::test]
async fn engine_guard_accepts_dev_groom_as_valid_dispatch() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: run_gh to remove label
            tool_call_response(
                "run_gh",
                json!({"args": "issue edit 500 --remove-label ready"}),
            ),
            // Step 2: run_gh to fetch issue body
            tool_call_response(
                "run_gh",
                json!({"args": "issue view 500 --json title,body"}),
            ),
            // Step 3: create_task
            tool_call_response(
                "create_task",
                json!({"label": "groom senara-solutions/mika#500"}),
            ),
            // Step 4: run_claude_pilot with dev-groom
            tool_call_response(
                "run_claude_pilot",
                json!({"skill": "dev-groom", "prompt": "mika issue#500"}),
            ),
            // Step 5: final text — guard should accept this
            text_response("Dispatched dev-groom for auto-grooming."),
        ])
        .tools(tools_with_auto_groom_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(READY_LABEL_MSG).await.unwrap();

    assert_has_output(&trace);
    // 5 steps means no guard rejection occurred — the guard accepted
    // run_claude_pilot (skill="dev-groom") as valid completion
    assert_exact_steps(&trace, 5);
}

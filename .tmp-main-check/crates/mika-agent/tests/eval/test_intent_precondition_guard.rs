//! Integration tests: intent-precondition registry (#702).
//!
//! Verifies the registry-driven intent-precondition guard that generalizes
//! the webhook zero-tools guard (#696) and adds the resume-reconcile guard.
//!
//! The webhook guard tests remain in `test_webhook_zero_tools_guard.rs` for
//! backward compatibility; this file covers the resume-reconcile entry and
//! verifies the registry dispatch mechanism.

use async_trait::async_trait;
use mika_agent::tools::{Tool, ToolContext, ToolOutput, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;

/// Stub tool that simulates `list_tasks` with a successful result.
struct StubListTasksTool;

#[async_trait]
impl Tool for StubListTasksTool {
    fn name(&self) -> &str {
        "list_tasks"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_tasks".to_string(),
            description: "Stub list_tasks for intent guard tests".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            "1 item — 1 in_progress\n| WI | Status | Type | Source | Label |\n| abc-123 | in_progress | milestone | manual | mika milestone#8 |".to_string(),
        ))
    }
}

/// Stub tool that simulates `check_task` with a successful result.
struct StubCheckTaskTool;

#[async_trait]
impl Tool for StubCheckTaskTool {
    fn name(&self) -> &str {
        "check_task"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "check_task".to_string(),
            description: "Stub check_task for intent guard tests".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            "Task abc-123: in_progress, type=milestone, label=mika milestone#8".to_string(),
        ))
    }
}

/// Build a tool registry with reconciliation stub tools.
fn tools_with_reconciliation() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubListTasksTool));
    tools.register(Box::new(StubCheckTaskTool));
    tools
}

/// Guard fires when agent receives a resume-intent message and responds
/// with text only (zero reconciliation tool calls).
#[tokio::test]
async fn resume_guard_fires_on_zero_reconciliation_tools() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent narrates without calling tools — guard rejects
            text_response(
                "I'll resume the milestone work. Let me check what needs to be done next.",
            ),
            // Step 2: After re-prompt, agent calls list_tasks then responds
            tool_call_response("list_tasks", json!({})),
            text_response("Found the milestone. Resuming child issue dispatch."),
        ])
        .tools(tools_with_reconciliation())
        .build()
        .await
        .unwrap();

    let trace = harness.run("resume mika milestone#8").await.unwrap();

    assert_has_output(&trace);
    // 3 steps: rejected text + tool call + final text
    assert_exact_steps(&trace, 3);
    assert_output_contains(&trace, "Resuming child issue dispatch");
}

/// Guard does NOT fire when the agent calls check_task successfully.
#[tokio::test]
async fn resume_guard_skips_when_check_task_called() {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("check_task", json!({})),
            // The agent loop may issue a follow-up after the tool result;
            // provide enough responses to avoid exhaustion.
            tool_call_response("list_tasks", json!({})),
            text_response("Milestone is in_progress. Resuming next child."),
        ])
        .tools(tools_with_reconciliation())
        .build()
        .await
        .unwrap();

    let trace = harness.run("resume mika milestone#8").await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "Resuming next child");
}

/// Guard does NOT fire when the agent calls list_tasks successfully.
#[tokio::test]
async fn resume_guard_skips_when_list_tasks_called() {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("list_tasks", json!({})),
            text_response("Found in-progress milestone. Dispatching next child."),
        ])
        .tools(tools_with_reconciliation())
        .build()
        .await
        .unwrap();

    let trace = harness.run("continue mika project#3").await.unwrap();

    assert_has_output(&trace);
    assert_exact_steps(&trace, 2);
    assert_output_contains(&trace, "Dispatching next child");
}

/// Guard does NOT fire on regular messages without resume intent.
#[tokio::test]
async fn resume_guard_skips_on_regular_messages() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "Here's an overview of the project architecture.",
        )])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("Can you explain the project architecture?")
        .await
        .unwrap();

    assert_has_output(&trace);
    // 1 step only — no re-prompt
    assert_exact_steps(&trace, 1);
}

/// Guard does NOT fire when "resume" appears without a process reference.
#[tokio::test]
async fn resume_guard_skips_without_process_ref() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "I'll resume working on the feature now.",
        )])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("resume working on the auth feature")
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_exact_steps(&trace, 1);
}

/// Guard fires at most once (single-retry semantics). If the agent still
/// produces zero reconciliation tool calls after the re-prompt, the turn
/// ends normally.
#[tokio::test]
async fn resume_guard_fires_only_once() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent narrates — guard rejects
            text_response("I see the milestone needs resuming."),
            // Step 2: Agent narrates AGAIN — guard already fired, lets through
            text_response("The milestone has been noted for resumption."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("resume mika milestone#8").await.unwrap();

    assert_has_output(&trace);
    // 2 steps: first rejected + second allowed through
    assert_exact_steps(&trace, 2);
    assert_output_contains(&trace, "noted for resumption");
}

/// Guard fires when list_tasks is called but fails (success=false).
#[tokio::test]
async fn resume_guard_fires_when_tool_fails() {
    // Stub that always fails
    struct FailingListTasksTool;

    #[async_trait]
    impl Tool for FailingListTasksTool {
        fn name(&self) -> &str {
            "list_tasks"
        }

        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "list_tasks".to_string(),
                description: "Failing stub".to_string(),
                input_schema: json!({"type": "object"}),
            }
        }

        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolContext<'_>,
        ) -> anyhow::Result<ToolOutput> {
            Ok(ToolOutput::error("Database error".to_string()))
        }
    }

    let mut tools = default_tools();
    tools.register(Box::new(FailingListTasksTool));

    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent responds text-only (no tool calls) — guard fires
            text_response("I see the milestone needs to be resumed."),
            // Step 2 (after re-prompt): Agent calls list_tasks but it fails,
            // then narrates — guard already fired (single-retry), lets through.
            tool_call_response("list_tasks", json!({})),
            text_response("I tried to list tasks but encountered an error."),
        ])
        .tools(tools)
        .build()
        .await
        .unwrap();

    let trace = harness.run("resume mika milestone#8").await.unwrap();

    assert_has_output(&trace);
    // 3 steps: text (rejected) + tool call (list_tasks fails) + text (passes)
    assert_exact_steps(&trace, 3);
    assert_output_contains(&trace, "encountered an error");
}

/// Guard fires on "continue" verb with project reference.
#[tokio::test]
async fn resume_guard_fires_on_continue_with_project() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Text-only on continue+project — rejected
            text_response("I'll continue the project work."),
            // Step 2: Agent acts properly
            tool_call_response("list_tasks", json!({})),
            text_response("Checked project status and resuming."),
        ])
        .tools(tools_with_reconciliation())
        .build()
        .await
        .unwrap();

    let trace = harness.run("continue mika project#5").await.unwrap();

    assert_has_output(&trace);
    // 3 steps: rejected text + tool call + final text
    assert_exact_steps(&trace, 3);
}

//! Integration tests: unauthorized webhook dispatch tool-boundary gate (#933).
//!
//! Verifies that `run_claude_pilot` is rejected at the tool boundary
//! (executor.rs `validate_dispatch_readiness`) BEFORE the subprocess spawns,
//! when the originating message is in the Webhook Fallthrough domain.
//!
//! Companion to `test_webhook_no_unauthorized_dispatch_guard.rs` which tests
//! the post-hoc INTENT_GUARD behavior (mika#910). This file tests the
//! pre-hoc tool-boundary block (mika#933).
//!
//! Note: The eval harness uses stub tools that bypass the real executor's
//! long-running path (no `LongRunningContext`, no `validate_dispatch_readiness`).
//! These tests validate the INTENT_GUARD post-hoc behavior in the eval context.
//! The unit tests in `executor.rs` validate the tool-boundary pre-hoc behavior
//! directly via `validate_dispatch_readiness`.

use async_trait::async_trait;
use mika_agent::tools::{Tool, ToolContext, ToolOutput, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;

/// Stub tool that simulates a successful `run_claude_pilot` dispatch.
struct StubRunClaudePilotTool;

#[async_trait]
impl Tool for StubRunClaudePilotTool {
    fn name(&self) -> &str {
        "run_claude_pilot"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_claude_pilot".to_string(),
            description: "Stub run_claude_pilot for tool-boundary gate tests".to_string(),
            input_schema: json!({"type": "object", "properties": {"prompt": {"type": "string"}}}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            "claude-pilot session started".to_string(),
        ))
    }
}

/// Stub tool for acknowledge-only actions.
struct StubWebhookActionTool;

#[async_trait]
impl Tool for StubWebhookActionTool {
    fn name(&self) -> &str {
        "webhook_action"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "webhook_action".to_string(),
            description: "Stub for tool-boundary gate tests".to_string(),
            input_schema: json!({"type": "object"}),
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

fn tools_with_pilot_and_action() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubRunClaudePilotTool));
    tools.register(Box::new(StubWebhookActionTool));
    tools
}

/// Tool-boundary gate rejects `run_claude_pilot` on a comment webhook turn.
/// The INTENT_GUARD (#910) fires post-hoc; the unit tests in executor.rs
/// confirm the tool-boundary pre-hoc behavior directly.
#[tokio::test]
async fn tool_boundary_rejects_comment_webhook_dispatch() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls run_claude_pilot on a comment event — guard rejects
            tool_call_response("run_claude_pilot", json!({"prompt": "mika#933"})),
            text_response("I'll dispatch this issue."),
            // Step 2: After re-prompt, agent acknowledges without dispatch
            tool_call_response("webhook_action", json!({})),
            text_response("Noted the comment. No dispatch needed."),
        ])
        .tools(tools_with_pilot_and_action())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("[GitHub] New comment on senara-solutions/mika#933 (webhook fallthrough) by @samidarko\nhttps://github.com/senara-solutions/mika/issues/933#issuecomment-12345\n\nReady to dispatch — apply the ready label when confirmed.")
        .await
        .unwrap();

    assert_has_output(&trace);
    // The no-unauthorized-dispatch INTENT_GUARD (#910) fires and rejects the
    // first response, then the agent retries with ack-only.
    assert_exact_steps(&trace, 4);
    assert_output_contains(&trace, "No dispatch needed");
}

/// Ready-label dispatch is NOT blocked — the tool-boundary gate allowlists it.
#[tokio::test]
async fn tool_boundary_allows_ready_label_dispatch() {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("run_claude_pilot", json!({"prompt": "mika#933"})),
            text_response("Dispatching implementation for mika#933."),
        ])
        .tools(tools_with_pilot_and_action())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("[GitHub] Issue labeled ready on senara-solutions/mika#933 — webhook fallthrough fix")
        .await
        .unwrap();

    assert_has_output(&trace);
    // 2 steps: tool call + final text. No gate interference.
    assert_exact_steps(&trace, 2);
    assert_output_contains(&trace, "Dispatching");
}

/// Direct prompt (no `[GitHub]` prefix) is NOT blocked.
#[tokio::test]
async fn tool_boundary_allows_direct_prompt_dispatch() {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("run_claude_pilot", json!({"prompt": "mika#933"})),
            text_response("Dispatching implementation for mika#933."),
        ])
        .tools(tools_with_pilot_and_action())
        .build()
        .await
        .unwrap();

    let trace = harness.run("implement mika issue#933").await.unwrap();

    assert_has_output(&trace);
    assert_exact_steps(&trace, 2);
    assert_output_contains(&trace, "Dispatching");
}

/// PR events are allowlisted (qa skill territory) — the tool-boundary gate
/// does NOT block them. Post-#1102: the INTENT_GUARD #910 predicate was
/// tightened to a positive allowlist — PR events no longer trip the guard,
/// so the tool call passes through in 2 steps (no retry).
#[tokio::test]
async fn tool_boundary_allows_pr_event_dispatch() {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("run_claude_pilot", json!({"prompt": "fix CI"})),
            text_response("Dispatching CI fix."),
        ])
        .tools(tools_with_pilot_and_action())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("[GitHub] PR opened: senara-solutions/mika#1000 — fix webhook dispatch (branch: fix/933)")
        .await
        .unwrap();

    assert_has_output(&trace);
    // 2 steps: tool call + final text. No guard interference post-#1102.
    assert_exact_steps(&trace, 2);
    assert_output_contains(&trace, "Dispatching");
}

/// Check-suite events are allowlisted (ci skill territory) — same pattern
/// as PR events. Post-#1102: INTENT_GUARD #910 no longer fires on
/// check-suite events, so tool call passes through in 2 steps.
#[tokio::test]
async fn tool_boundary_allows_check_suite_dispatch() {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("run_claude_pilot", json!({"prompt": "fix CI"})),
            text_response("Dispatching CI fix."),
        ])
        .tools(tools_with_pilot_and_action())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("[GitHub] Check suite failure on senara-solutions/mika (branch: fix/933)")
        .await
        .unwrap();

    assert_has_output(&trace);
    // 2 steps: tool call + final text. No guard interference post-#1102.
    assert_exact_steps(&trace, 2);
    assert_output_contains(&trace, "Dispatching");
}

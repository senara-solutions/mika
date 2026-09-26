//! Integration tests: webhook no-unauthorized-dispatch guard (#910).
//!
//! Verifies that the agent loop rejects EndTurn responses on non-ready
//! `[GitHub]` webhook turns when `run_claude_pilot` was successfully called.
//! Only `[GitHub] Issue labeled ready on` webhooks may dispatch.

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
            description: "Stub run_claude_pilot for unauthorized dispatch guard tests".to_string(),
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

/// Stub tool that always succeeds — used for acknowledge-only webhook handling.
struct StubWebhookActionTool;

#[async_trait]
impl Tool for StubWebhookActionTool {
    fn name(&self) -> &str {
        "webhook_action"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "webhook_action".to_string(),
            description: "Stub for webhook guard tests".to_string(),
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

/// Build a tool registry with both stub tools.
fn tools_with_pilot_and_action() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubRunClaudePilotTool));
    tools.register(Box::new(StubWebhookActionTool));
    tools
}

/// Guard fires when agent calls `run_claude_pilot` on a comment webhook turn.
/// This is the core unauthorized dispatch scenario from mika#910.
#[tokio::test]
async fn guard_fires_on_comment_with_pilot_dispatch() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls run_claude_pilot on a comment event — guard rejects
            tool_call_response("run_claude_pilot", json!({"prompt": "mika#906"})),
            text_response("I'll dispatch this issue."),
            // Step 2: After re-prompt, agent acknowledges without dispatch
            tool_call_response("webhook_action", json!({})),
            text_response("Noted the comment. No dispatch needed — this is a comment event."),
        ])
        .tools(tools_with_pilot_and_action())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("[GitHub] New comment on senara-solutions/mika#906 (kg/resolver: ...) by @samidarko\nhttps://github.com/senara-solutions/mika/issues/906#issuecomment-4352271707\n\nGroomed end-to-end via /mika-groom-ticket.\nReady to dispatch via `mika ask --agent mika-dev \"implement mika issue#906\"`.")
        .await
        .unwrap();

    assert_has_output(&trace);
    // Guard rejected the first response (run_claude_pilot + text), then the
    // agent retried with acknowledge-only (webhook_action + text).
    // Steps: run_claude_pilot + text (rejected) + webhook_action + text = 4
    assert_exact_steps(&trace, 4);
    assert_output_contains(&trace, "No dispatch needed");
}

/// Guard does NOT fire on ready-label events — those are handled by the
/// positive-case `webhook_ready_label_dispatch` guard.
#[tokio::test]
async fn guard_skips_on_ready_label_event() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Agent calls run_claude_pilot on a ready-label event — allowed
            tool_call_response("run_claude_pilot", json!({"prompt": "mika#906"})),
            text_response("Dispatching implementation for mika#906."),
        ])
        .tools(tools_with_pilot_and_action())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("[GitHub] Issue labeled ready on senara-solutions/mika#906 — title here")
        .await
        .unwrap();

    assert_has_output(&trace);
    // 2 steps: tool call + final text. The no-unauthorized-dispatch guard
    // should NOT fire because the trigger excludes ready-label events.
    // (The ready-label dispatch guard IS satisfied by the run_claude_pilot call.)
    assert_exact_steps(&trace, 2);
    assert_output_contains(&trace, "Dispatching");
}

/// Guard does NOT fire on direct prompts (no `[GitHub]` prefix).
#[tokio::test]
async fn guard_skips_on_direct_prompt() {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("run_claude_pilot", json!({"prompt": "mika#906"})),
            text_response("Dispatching implementation for mika#906."),
        ])
        .tools(tools_with_pilot_and_action())
        .build()
        .await
        .unwrap();

    let trace = harness.run("implement mika issue#906").await.unwrap();

    assert_has_output(&trace);
    // 2 steps: tool call + text — no guard interference
    assert_exact_steps(&trace, 2);
    assert_output_contains(&trace, "Dispatching");
}

/// Acknowledge-only webhook handling (no `run_claude_pilot`) passes normally.
/// The new guard is satisfied when no successful dispatch occurred.
#[tokio::test]
async fn acknowledge_only_passes_on_non_ready_label() {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("webhook_action", json!({})),
            text_response("Noted the bug label. Appended to current_priorities."),
        ])
        .tools(tools_with_pilot_and_action())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("[GitHub] Issue labeled bug on senara-solutions/mika#906")
        .await
        .unwrap();

    assert_has_output(&trace);
    // 2 steps: tool call + text — guard not triggered (no run_claude_pilot)
    assert_exact_steps(&trace, 2);
    assert_output_contains(&trace, "Noted the bug label");
}

/// Guard fires at most once (single-retry semantics). If the agent still
/// calls `run_claude_pilot` after the re-prompt, the turn ends normally
/// (the second attempt is not rejected).
#[tokio::test]
async fn guard_fires_only_once() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls run_claude_pilot — guard rejects
            tool_call_response("run_claude_pilot", json!({"prompt": "mika#906"})),
            text_response("Dispatching."),
            // Step 2: Agent calls run_claude_pilot AGAIN — guard already fired,
            // lets through (single-retry semantics)
            tool_call_response("run_claude_pilot", json!({"prompt": "mika#906"})),
            text_response("Dispatching again."),
        ])
        .tools(tools_with_pilot_and_action())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("[GitHub] New comment on senara-solutions/mika#906 by @samidarko\nhttps://github.com/senara-solutions/mika/issues/906#issuecomment-123\n\nReady to dispatch.")
        .await
        .unwrap();

    assert_has_output(&trace);
    // 4 steps: (run_claude_pilot + text) rejected + (run_claude_pilot + text) allowed
    assert_exact_steps(&trace, 4);
    assert_output_contains(&trace, "Dispatching again");
}

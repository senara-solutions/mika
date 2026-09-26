//! Integration tests: webhook zero-tools guard (#696).
//!
//! Verifies that the agent loop rejects EndTurn responses on webhook turns
//! (user message starts with `[GitHub]`) when zero successful tool calls
//! were made in the turn.

use async_trait::async_trait;
use mika_agent::tools::{Tool, ToolContext, ToolOutput, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;

/// Stub tool that always succeeds — used to simulate a successful tool call
/// on webhook turns.
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
            input_schema: serde_json::json!({"type": "object"}),
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

/// Build a tool registry with our stub tool.
fn tools_with_webhook_action() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubWebhookActionTool));
    tools
}

/// Guard fires when agent receives a `[GitHub]` webhook message and responds
/// with text only (zero tool calls).
#[tokio::test]
async fn guard_fires_on_webhook_with_zero_tools() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent narrates without calling tools — guard rejects
            text_response("The PR has been approved. Everything looks good and is ready to merge."),
            // Step 2: After re-prompt, agent calls a tool then responds
            tool_call_response("webhook_action", json!({})),
            text_response("Checked task status after PR approval."),
        ])
        .tools(tools_with_webhook_action())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("[GitHub] PR review (approved) on senara-solutions/mika#694 by reviewer\nhttps://github.com/senara-solutions/mika/pull/694#pullrequestreview-123")
        .await
        .unwrap();

    assert_has_output(&trace);
    // 3 steps: rejected text + tool call + final text
    assert_exact_steps(&trace, 3);
    assert_output_contains(&trace, "Checked task status");
}

/// Guard does NOT fire when the agent calls tools successfully on a webhook turn.
#[tokio::test]
async fn guard_skips_when_tool_called_successfully() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Agent processes the webhook with a tool call
            tool_call_response("webhook_action", json!({})),
            text_response("Updated task status based on PR approval."),
        ])
        .tools(tools_with_webhook_action())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("[GitHub] PR review (approved) on senara-solutions/mika#694 by reviewer\nhttps://github.com/senara-solutions/mika/pull/694#pullrequestreview-123")
        .await
        .unwrap();

    assert_has_output(&trace);
    // 2 steps: tool call + final text (no re-prompt)
    assert_exact_steps(&trace, 2);
    assert_output_contains(&trace, "Updated task status");
}

/// Guard does NOT fire on regular user messages without the `[GitHub]` prefix.
#[tokio::test]
async fn guard_skips_on_regular_messages() {
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
    assert_output_contains(&trace, "architecture");
}

/// Guard fires at most once (single-retry semantics). If the agent still
/// produces zero tool calls after the re-prompt, the turn ends normally.
#[tokio::test]
async fn guard_fires_only_once() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent narrates — guard rejects
            text_response("I see the PR was approved."),
            // Step 2: Agent narrates AGAIN — guard already fired, lets through
            text_response("The approval has been noted."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("[GitHub] PR review (approved) on senara-solutions/mika#694 by reviewer\nhttps://github.com/senara-solutions/mika/pull/694#pullrequestreview-123")
        .await
        .unwrap();

    assert_has_output(&trace);
    // 2 steps: first rejected + second allowed through
    assert_exact_steps(&trace, 2);
    assert_output_contains(&trace, "noted");
}

/// Guard does **not** fire on an issue webhook — the Webhook Fallthrough
/// domain stands it down (mika#2517 U3, AC2).
///
/// **This assertion is the inverse of the one it replaces**, and the inversion
/// is a decision rather than a relaxation. This test used to assert that
/// `[GitHub] Issue opened:` with zero tool calls gets re-prompted. On exactly
/// that population the guard's own premise — *"webhook events require action"* —
/// contradicts the `self-dev` § Webhook Fallthrough HARD GATE, which says the
/// correct action is to acknowledge and stop. So the engine was re-prompting a
/// turn that had obeyed, handing it a list of tools to call whose shortest
/// creates a task — the `pending` phantom mika#2517 was filed about. Keeping
/// the old assertion would mean this suite asserts both halves of a
/// contradiction; it is replaced, not carried. Same treatment the unit-level
/// siblings received (`webhook_zero_tools_trigger_fires_on_new_comment`,
/// `…_fires_on_non_ready_label`).
///
/// **The positive control lives in this same file and must stay green**:
/// `guard_fires_on_webhook_with_zero_tools` and `guard_fires_only_once` both
/// run on `[GitHub] PR review (approved)`, which is *outside* the domain. Were
/// the trigger to start returning `false` on everything, those two would go red
/// — without them, this test alone would read exactly the same whether the
/// domain exclusion works or the mika#696 guard has been retired wholesale.
#[tokio::test]
async fn guard_stands_down_on_issue_webhook() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // A single text-only turn. Were the guard still armed here, the
            // mock would be asked for a second response and the step count
            // below would not hold.
            text_response("A new issue was opened about performance."),
        ])
        .tools(tools_with_webhook_action())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("[GitHub] Issue opened: senara-solutions/mika#700 — Performance regression\nhttps://github.com/senara-solutions/mika/issues/700")
        .await
        .unwrap();

    assert_has_output(&trace);
    // 1 step: acknowledged and stopped, no re-prompt.
    assert_exact_steps(&trace, 1);
    assert_output_contains(&trace, "performance");
}

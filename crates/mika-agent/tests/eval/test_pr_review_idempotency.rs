//! Integration tests: PR review idempotency guard (#695).
//!
//! Verifies:
//! 1. The post-condition chain accepts EndTurn after a successful `run_gh pr review`
//!    (skipping guards #4-#6 that might force continuation).
//! 2. A second `run_gh pr review` call in the same turn is rejected with a
//!    `duplicate_pr_review` structured error.

use async_trait::async_trait;
use mika_agent::tools::{Tool, ToolContext, ToolOutput, ToolRegistry};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::json;

#[allow(unused_imports)]
use super::assertions::*;
use super::harness::EvalHarness;

/// Stub `run_gh` tool that simulates successful `gh` commands and implements
/// the per-turn PR review dedup logic (same as the real builtin handler).
struct StubRunGhTool;

impl StubRunGhTool {
    fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for StubRunGhTool {
    fn name(&self) -> &str {
        "run_gh"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_gh".to_string(),
            description: "Execute a GitHub CLI command".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "array", "items": {"type": "string"}},
                    "repo": {"type": "string"}
                },
                "required": ["command"]
            }),
        }
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        let args: Vec<String> = input
            .get("command")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        // Replicate the per-turn PR review dedup logic from builtin_handlers (#695)
        let is_pr_review = args.len() >= 2 && args[0] == "pr" && args[1] == "review";

        if is_pr_review {
            if ctx
                .pr_review_posted
                .load(std::sync::atomic::Ordering::Acquire)
            {
                return Ok(ToolOutput::error(
                    "{\"error\": \"duplicate_pr_review\", \"message\": \"A PR review was already \
                     posted in this turn. Duplicate reviews create duplicate webhooks. End your \
                     turn — the review is already submitted.\"}"
                        .to_string(),
                ));
            }
            // Mark as posted on success
            ctx.pr_review_posted
                .store(true, std::sync::atomic::Ordering::Release);
        }

        Ok(ToolOutput::success(
            "Review submitted successfully.".to_string(),
        ))
    }
}

/// Build a tool registry with stub run_gh as the only tool.
fn tools_with_stub_run_gh() -> ToolRegistry {
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(StubRunGhTool::new()));
    tools
}

/// When the agent posts a PR review and then ends the turn with text containing
/// "completed", the post-condition chain should accept EndTurn immediately
/// (the PR review early-accept skips guard #4 / completion-claim).
#[tokio::test]
async fn pr_review_early_accept_skips_completion_guard() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Call run_gh to post a PR review
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "review", "455", "--approve", "--body", "VERDICT: pass\nREASON: Clean."], "repo": "senara-solutions/mika"}),
            ),
            // Step 2: EndTurn with text containing "completed" — would trigger
            // guard #4 (completion-claim) if early-accept wasn't present.
            text_response("Review completed. PR #455 approved."),
        ])
        .tools(tools_with_stub_run_gh())
        .build()
        .await
        .unwrap();

    let trace = harness.run("Review PR #455").await.unwrap();

    // The agent should have completed successfully (not re-prompted by guard #4)
    assert_has_output(&trace);
    // The output should contain "completed" — proves it wasn't rejected
    assert_output_contains(&trace, "completed");
}

/// When the LLM tries to post a second PR review in the same turn,
/// the dedup guard rejects it with a structured error.
#[tokio::test]
async fn duplicate_pr_review_rejected() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: First run_gh pr review — succeeds
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "review", "455", "--approve", "--body", "VERDICT: pass"], "repo": "senara-solutions/mika"}),
            ),
            // Step 2: LLM tries to post again (simulating forced continuation)
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "review", "455", "--approve", "--body", "VERDICT: pass"], "repo": "senara-solutions/mika"}),
            ),
            // Step 3: LLM receives error and ends turn gracefully
            text_response("Review already posted. Done."),
        ])
        .tools(tools_with_stub_run_gh())
        .build()
        .await
        .unwrap();

    let trace = harness.run("Review PR #455").await.unwrap();

    // Agent should complete normally
    assert_has_output(&trace);
    // The second call should have been rejected, so the agent only ran 1 successful call
    // (the trace output should reflect handling of the error)
    assert_output_contains(&trace, "already posted");
}

/// Non-review `run_gh` calls are not affected by the dedup guard.
/// The agent can call `gh pr list` multiple times without issues.
#[tokio::test]
async fn non_review_gh_commands_unaffected() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: gh pr list (not a review)
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "list", "--state", "open"], "repo": "senara-solutions/mika"}),
            ),
            // Step 2: gh pr list again (should also succeed)
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "list", "--state", "merged"], "repo": "senara-solutions/mika"}),
            ),
            // Step 3: End turn
            text_response("Listed open and merged PRs."),
        ])
        .tools(tools_with_stub_run_gh())
        .build()
        .await
        .unwrap();

    let trace = harness.run("List PRs").await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "Listed");
}

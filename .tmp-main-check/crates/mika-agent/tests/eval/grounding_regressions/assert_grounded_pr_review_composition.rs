//! Scenario: Assert-grounded fires despite PR review early-accept (mika#1178)
//!
//! Context: the agent posts a successful `gh pr review --approve` (triggering
//! `has_successful_pr_review() == true` and `skip_remaining_guards = true`), then
//! on the same turn's EndTurn response makes an affirmative state claim about an
//! unrelated resource ("PR #42 has been merged and all CI checks are passing")
//! without any grounding tool call for that resource. Before the fix,
//! `skip_remaining_guards` would bypass the assert-grounded guard (6d). After the
//! fix, guard 6d fires regardless of `skip_remaining_guards`.
//!
//! ## Hard Assertions
//! - Guard fires: LLM call count > 2 (tool call + EndTurn rejected + retry).
//! - `run_gh` is called at least twice (once for the PR review, once to verify
//!   the ungrounded claim on the corrective retry).
//!
//! ## Tags
//! - `grounding:affirmative-claim-ungrounded` — the composition gap that allowed
//!   the ungrounded state claim to pass through the skip path
//! - `grounding:verification-before-claim` — agent corrects after guard fires
//!
//! Reference: mika#1178, mika#1331 (assert-grounded guard)

use std::sync::atomic::Ordering;

use async_trait::async_trait;
use mika_agent::tools::{Tool, ToolContext, ToolOutput, ToolRegistry, default_tools};
use mika_common::claude::ToolDefinition;

use super::*;

/// Stub `run_gh` tool that succeeds for all calls, producing a
/// `ToolCallSummary` with `success: true`. For PR review calls, the
/// `input_summary` will contain `"pr"` and `"review"`, satisfying
/// `has_successful_pr_review()`.
struct StubRunGh;

#[async_trait]
impl Tool for StubRunGh {
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

        let is_pr_review = args.len() >= 2 && args[0] == "pr" && args[1] == "review";
        if is_pr_review {
            ctx.pr_review_posted.store(true, Ordering::Release);
        }

        // Return contextual output based on the command.
        let output = if is_pr_review {
            "Review submitted successfully."
        } else {
            r#"{"state": "OPEN", "mergeable": "MERGEABLE"}"#
        };

        Ok(ToolOutput::success(output.to_string()))
    }
}

fn build_tools() -> ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubRunGh));
    tools
}

/// Agent posts `run_gh pr review --approve` (success → skip_remaining_guards=true),
/// then claims "PR #42 has been merged and all CI checks are passing" without any
/// grounding tool call for PR #42. Guard 6d should fire despite the skip path.
#[tokio::test]
async fn test_assert_grounded_fires_despite_pr_review_early_accept() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .tools(build_tools())
        .responses(vec![
            // Step 0: Agent calls run_gh with PR review args (ToolUse stop).
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["pr", "review", "455", "--approve", "--body", "VERDICT: pass"]
                }),
            ),
            // Step 1: EndTurn — agent makes an ungrounded state claim about PR #42.
            // has_successful_pr_review() is now true → skip_remaining_guards = true.
            // Pre-fix: guard 6d would be skipped. Post-fix: guard 6d fires.
            text_response(
                "I've approved PR #455. I also verified that PR #42 has been merged \
                 and all CI checks are passing — the deployment is complete.",
            ),
            // Step 2: After guard 6d fires, agent calls run_gh to verify PR #42.
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["pr", "view", "42", "--json", "state,mergeable"]
                }),
            ),
            // Step 3: Clean EndTurn with grounded information.
            text_response(
                "I've approved PR #455. After checking PR #42 via `run_gh`, \
                 I can confirm it is in an open state.",
            ),
        ])
        .build()
        .await?;

    let trace = harness
        .run("Please review PR #455 and check the status of PR #42")
        .await?;

    // Hard: guard fired — more than 2 LLM calls (tool call + rejected EndTurn + retry)
    assert!(
        trace.llm_call_count > 2,
        "Expected guard 6d to fire despite PR review early-accept \
         (llm_call_count > 2), got {}",
        trace.llm_call_count
    );

    // Hard: run_gh was called at least twice (PR review + grounding verification)
    let run_gh_calls = trace.calls_for_tool("run_gh");
    assert!(
        run_gh_calls.len() >= 2,
        "Expected at least 2 run_gh calls (review + grounding), got {}",
        run_gh_calls.len()
    );

    Ok(())
}

//! Scenario: Asserted unavailability fires despite PR review early-accept (mika#1178)
//!
//! Context: the agent posts a successful `gh pr review --approve` (triggering
//! `has_successful_pr_review() == true` and `skip_remaining_guards = true`), then
//! on the same turn's EndTurn response claims `gh_read is not callable`. Before
//! the fix, `skip_remaining_guards` would bypass the asserted-unavailability guard
//! (6c), allowing the fabrication to pass. After the fix, guard 6c fires
//! regardless of `skip_remaining_guards`.
//!
//! ## Hard Assertions
//! - Guard fires: LLM call count > 2 (tool call + EndTurn rejected + retry).
//! - `search_memory` is called on the corrective retry (agent corrects behavior).
//!
//! ## Tags
//! - `grounding:unavailability-asserted-without-attempt` — the composition gap
//!   that allowed the fabrication to pass through the skip path
//! - `grounding:verification-before-claim` — agent corrects after guard fires
//!
//! Reference: mika#1178, mika#862 (asserted-unavailability guard)

use std::sync::atomic::Ordering;

use async_trait::async_trait;
use mika_agent::tools::{Tool, ToolContext, ToolOutput, ToolRegistry, default_tools};
use mika_common::claude::ToolDefinition;

use super::*;

/// Stub `run_gh` tool that succeeds for PR review calls, producing a
/// `ToolCallSummary` with `success: true` and `input_summary` containing
/// `"pr"` and `"review"` — satisfying `has_successful_pr_review()`.
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

        Ok(ToolOutput::success(
            "Review submitted successfully.".to_string(),
        ))
    }
}

fn build_tools() -> ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubRunGh));
    tools
}

/// Agent posts `run_gh pr review --approve` (success → skip_remaining_guards=true),
/// then claims `search_memory is not callable`. Guard 6c should fire despite the
/// skip path, because claim-without-evidence is orthogonal to PR-review completion.
#[tokio::test]
async fn test_asserted_unavailability_fires_despite_pr_review_early_accept() -> anyhow::Result<()> {
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
            // Step 1: EndTurn — agent claims search_memory is not callable.
            // has_successful_pr_review() is now true → skip_remaining_guards = true.
            // Pre-fix: guard 6c would be skipped. Post-fix: guard 6c fires.
            text_response(
                "I've approved PR #455. Note that search_memory is not callable \
                 in this context — it's skill-scoped and not available here. \
                 The review is complete.",
            ),
            // Step 2: After guard 6c fires, agent corrects and calls search_memory.
            tool_call_response(
                "search_memory",
                json!({
                    "query": "pr 455 review context"
                }),
            ),
            // Step 3: Clean EndTurn without fabrication.
            text_response(
                "After searching memory, I can confirm the review context. \
                 PR #455 has been approved.",
            ),
        ])
        .build()
        .await?;

    let trace = harness
        .run("Please review PR #455 and check the context")
        .await?;

    // Hard: guard fired — more than 2 LLM calls (tool call + rejected EndTurn + retry)
    assert!(
        trace.llm_call_count > 2,
        "Expected guard 6c to fire despite PR review early-accept \
         (llm_call_count > 2), got {}",
        trace.llm_call_count
    );

    // Hard: agent called search_memory after corrective re-prompt
    assert_tools_include(&trace, &["search_memory"]);

    // Hard: run_gh was called (confirming PR review path was active)
    assert_tools_include(&trace, &["run_gh"]);

    Ok(())
}

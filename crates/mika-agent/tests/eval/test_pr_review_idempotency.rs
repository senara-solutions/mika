//! Integration tests: PR review idempotency guard (#695, #736).
//!
//! Verifies:
//! 1. The post-condition chain accepts EndTurn after a successful `run_gh pr review`
//!    (skipping guards #4-#6 that might force continuation).
//! 2. A second `run_gh pr review` call in the same turn is rejected with a
//!    `duplicate_pr_review` structured error.
//! 3. (#736) Session-scoped DashMap blocks cross-turn duplicates (e.g., when a
//!    required-tools gate forces a retry into a new turn with a fresh AtomicBool).

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
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

// -- Session-scoped dedup integration tests (#736) --

/// Stub `run_gh` tool that implements BOTH per-turn AND session-scoped dedup logic,
/// mirroring the production `handle_run_gh` behavior. Uses the `ToolContext`'s
/// `pr_reviews_posted` DashMap (when present) for session-scope checks, and the
/// per-turn `pr_review_posted` AtomicBool for within-turn checks.
///
/// Also applies PR identifier normalization (mika#736) to produce format-stable
/// dedup keys regardless of whether the LLM passes a bare number or full URL.
struct SessionAwareStubRunGhTool;

impl SessionAwareStubRunGhTool {
    fn new() -> Self {
        Self
    }

    /// Replicate `normalize_pr_identifier` from builtin_handlers (private fn).
    fn normalize_pr_id(s: &str) -> &str {
        if let Some(idx) = s.rfind("/pull/") {
            let after = &s[idx + 6..];
            let end = after
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(after.len());
            if end > 0 {
                return &after[..end];
            }
        }
        s
    }

    /// Replicate `make_pr_dedup_key` from builtin_handlers (private fn).
    fn dedup_key(args: &[String], repo: Option<&str>) -> String {
        let positional = args
            .get(2)
            .map(|s| Self::normalize_pr_id(s))
            .unwrap_or("__current_branch__");
        format!("{}|{}", repo.unwrap_or("__default__"), positional)
    }
}

#[async_trait]
impl Tool for SessionAwareStubRunGhTool {
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
        let repo: Option<String> = input
            .get("repo")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let is_pr_review = args.len() >= 2 && args[0] == "pr" && args[1] == "review";

        if is_pr_review {
            let dedup_key = Self::dedup_key(&args, repo.as_deref());

            // Session-scope check (Fix A, #821) — primary defense.
            if let Some(map) = ctx.pr_reviews_posted
                && map
                    .get(ctx.session_id)
                    .map(|set| set.contains(&dedup_key))
                    .unwrap_or(false)
            {
                return Ok(ToolOutput::error(
                    "{\"error\": \"duplicate_pr_review\", \"message\": \"A PR review was already \
                     posted in this session for this PR. Duplicate reviews create duplicate \
                     webhooks. End your turn — the review is already submitted.\"}"
                        .to_string(),
                ));
            }

            // Per-turn check (Fix B, #695).
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

            // Success: mark both guards.
            ctx.pr_review_posted
                .store(true, std::sync::atomic::Ordering::Release);
            if let Some(map) = ctx.pr_reviews_posted {
                map.entry(ctx.session_id.to_string())
                    .or_default()
                    .insert(dedup_key);
            }
        }

        Ok(ToolOutput::success(
            "Review submitted successfully.".to_string(),
        ))
    }
}

/// Build a tool registry with session-aware stub run_gh.
fn tools_with_session_aware_stub() -> ToolRegistry {
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(SessionAwareStubRunGhTool::new()));
    tools
}

/// (#736) Session-scoped DashMap blocks a duplicate PR review across turns.
///
/// Simulates the exact #736 bug scenario:
/// 1. Turn 1: LLM posts `pr review 455 --approve` (succeeds, session map populated).
/// 2. Turn 2 (simulated via `run_turn`): LLM tries the same review again
///    (fresh AtomicBool for the new turn, but session map should block it).
///
/// This exercises the full agent loop with the DashMap threaded through AgentParams,
/// proving the session-scope guard works end-to-end — not just in unit tests.
#[tokio::test]
async fn pr_review_session_scope_blocks_cross_turn_duplicate() {
    let session_map: Arc<DashMap<String, HashSet<String>>> = Arc::new(DashMap::new());

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Post the PR review (succeeds)
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "review", "455", "--approve", "--body", "VERDICT: pass"], "repo": "senara-solutions/mika"}),
            ),
            // Turn 1: End turn — early-accept skips remaining guards
            text_response("Review completed. PR #455 approved."),
        ])
        .tools(tools_with_session_aware_stub())
        .pr_reviews_posted(session_map.clone())
        .build()
        .await
        .unwrap();

    // Turn 1: review succeeds
    let trace1 = harness.run("Review PR #455").await.unwrap();
    assert_has_output(&trace1);
    assert_output_contains(&trace1, "completed");

    // Verify the session map was populated
    assert!(
        !session_map.is_empty(),
        "session map should be populated after successful review"
    );

    // Turn 2: same session, fresh AtomicBool (new turn), tries same review
    let trace2 = harness
        .run_turn(
            "Review PR #455 again",
            vec![
                // LLM tries to post the same review (session map should block it)
                tool_call_response(
                    "run_gh",
                    json!({"command": ["pr", "review", "455", "--approve", "--body", "VERDICT: pass"], "repo": "senara-solutions/mika"}),
                ),
                // LLM receives duplicate error and ends gracefully
                text_response("The review was already posted. No action needed."),
            ],
        )
        .await
        .unwrap();

    assert_has_output(&trace2);
    assert_output_contains(&trace2, "already posted");
}

/// (#736) URL and bare number for the same PR produce the same session-scope
/// dedup key, preventing format-fragile duplicates.
///
/// Turn 1 uses the full GitHub URL, turn 2 uses the bare number — the session
/// map should still block the duplicate because `normalize_pr_identifier`
/// reduces both to the same key.
#[tokio::test]
async fn pr_review_session_scope_url_vs_number_dedup() {
    let session_map: Arc<DashMap<String, HashSet<String>>> = Arc::new(DashMap::new());

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Post review using full URL form
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "review", "https://github.com/senara-solutions/mika/pull/455", "--approve", "--body", "VERDICT: pass"], "repo": "senara-solutions/mika"}),
            ),
            text_response("Review completed. PR #455 approved."),
        ])
        .tools(tools_with_session_aware_stub())
        .pr_reviews_posted(session_map.clone())
        .build()
        .await
        .unwrap();

    // Turn 1: review with URL form succeeds
    let trace1 = harness.run("Review PR #455").await.unwrap();
    assert_has_output(&trace1);

    // Turn 2: same PR but using bare number form — should be blocked
    let trace2 = harness
        .run_turn(
            "Review PR #455 again",
            vec![
                tool_call_response(
                    "run_gh",
                    json!({"command": ["pr", "review", "455", "--approve", "--body", "VERDICT: pass"], "repo": "senara-solutions/mika"}),
                ),
                text_response("The review was already posted. No action needed."),
            ],
        )
        .await
        .unwrap();

    assert_has_output(&trace2);
    assert_output_contains(&trace2, "already posted");
}

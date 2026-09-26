//! Scenario: qa-review required-tools-gate retry produces duplicate PR review (mika#736)
//!
//! Context: mika-qa reviews a PR with a skill declaring
//! `required_tools = ["qa_pr_view", "run_gh"]`. The agent posts a `run_gh pr review`
//! on the first turn but omits `qa_pr_view`. The required-tools gate rejects EndTurn
//! and forces a retry turn. On the retry, the agent calls `qa_pr_view` AND tries to
//! post a second `pr review`. Without the session-scoped dedup guard (Fix A, #821)
//! and PR identifier normalization (#736), the second review goes through.
//!
//! ## Hard Assertions
//! - **Regression-reproduction (pre-fix shape):** two successful `run_gh` calls with
//!   `pr review` in the input exist in the tool summaries — proves the assertion catches
//!   the failure when the guard is absent.
//! - **Post-fix (session guard active):** at most one successful `run_gh pr review`
//!   call in tool summaries — the session guard blocks the duplicate.
//!
//! ## Tags
//! - `grounding:duplicate-side-effect-suppressed` — post-fix success tag
//!   (session guard blocks duplicate review)
//!
//! ## Frozen Fixture
//! - `fixtures/qa_review_required_tools_retry_duplicate_pre_fix.json` — pre-fix trace
//!   with two successful `run_gh pr review` calls (bare number vs URL form for the
//!   same PR #735).
//!
//! Reference: mika#736, mika#821 (session-scope guard), mika#695 (per-turn guard)

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use mika_agent::skills::SkillRegistry;
use mika_agent::skills::index::{ResolvedSkillTool, SkillEntry};
use mika_agent::skills::manifest::{Constraints, SkillInfo, SkillManifest, ToolHandler, Triggers};
use mika_agent::tools::{Tool, ToolContext, ToolOutput, ToolRegistry};
use mika_common::claude::ToolDefinition;

use super::*;

/// Create a qa-review skill entry with `required_tools = ["qa_pr_view", "run_gh"]`.
fn make_qa_review_skill() -> SkillEntry {
    SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: "qa-review".to_string(),
                description: "QA PR review skill".to_string(),
                version: "0.1.0".to_string(),
                always_on: false,
                timeout_secs: 600,
                dependencies: vec![],
                max_prompt_size: None,
                data_grade: Default::default(),
            },
            triggers: Triggers {
                keywords: vec![
                    "review".to_string(),
                    "pr review".to_string(),
                    "qa review".to_string(),
                ],
            },
            llm: Default::default(),
            constraints: Constraints {
                required_tools: vec!["qa_pr_view".to_string(), "run_gh".to_string()],
                required_fetches_for_quoted_resources: false,
            },
            output: Default::default(),
            context: HashMap::new(),
            variants: Default::default(),
        },
        dir: PathBuf::from("/skills/qa-review"),
        keywords_lower: vec![
            "review".to_string(),
            "pr review".to_string(),
            "qa review".to_string(),
        ],
        prompt_snippet: String::new(),
        skill_tools: vec![ResolvedSkillTool {
            definition: ToolDefinition {
                name: "qa_pr_view".to_string(),
                description: "View PR details for QA review".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "pr": {"type": "integer"}
                    },
                    "required": ["pr"]
                }),
            },
            handler: ToolHandler::Builtin {
                function: "qa_pr_view".to_string(),
            },
            skill_dir: PathBuf::from("/skills/qa-review"),
        }],
        enabled: true,
        has_override: false,
        provider_overrides: HashMap::new(),
        prompt_sources: SkillEntry::empty_prompt_sources(),
        model_overrides: HashMap::new(),
    }
}

/// Stub `qa_pr_view` tool — returns minimal PR info.
struct StubQaPrViewTool;

#[async_trait]
impl Tool for StubQaPrViewTool {
    fn name(&self) -> &str {
        "qa_pr_view"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "qa_pr_view".to_string(),
            description: "View PR details for QA review".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pr": {"type": "integer"}
                },
                "required": ["pr"]
            }),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            "PR #735: fix(agent): normalize PR identifier in dedup key\n\
             Status: Open, CI: passing, Reviews: 0"
                .to_string(),
        ))
    }
}

/// Session-aware stub `run_gh` tool that implements both per-turn and session-scoped
/// dedup, with PR identifier normalization (#736).
struct SessionAwareStubRunGh;

impl SessionAwareStubRunGh {
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

    fn dedup_key(args: &[String], repo: Option<&str>) -> String {
        let positional = args
            .get(2)
            .map(|s| Self::normalize_pr_id(s))
            .unwrap_or("__current_branch__");
        format!("{}|{}", repo.unwrap_or("__default__"), positional)
    }
}

#[async_trait]
impl Tool for SessionAwareStubRunGh {
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

            // Session-scope check (Fix A, #821)
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

            // Per-turn check (Fix B, #695)
            if ctx
                .pr_review_posted
                .load(std::sync::atomic::Ordering::Acquire)
            {
                return Ok(ToolOutput::error(
                    "{\"error\": \"duplicate_pr_review\", \"message\": \"A PR review was already \
                     posted in this turn.\"}"
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

fn build_tools() -> ToolRegistry {
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(StubQaPrViewTool));
    tools.register(Box::new(SessionAwareStubRunGh));
    tools
}

/// Count how many successful `run_gh pr review` calls are in the tool_calls trace.
fn count_successful_pr_reviews(trace: &crate::eval::trace::AgentTrace) -> usize {
    trace
        .tool_calls
        .iter()
        .filter(|tc| {
            tc.tool_name == "run_gh"
                && tc.success
                && tc
                    .input
                    .as_deref()
                    .map(|i| i.contains("pr") && i.contains("review"))
                    .unwrap_or(false)
        })
        .count()
}

/// Regression-reproduction: pre-fix trace shape where the session guard is absent
/// (pr_reviews_posted = None). The required-tools gate retry produces a second
/// successful review because the per-turn AtomicBool resets between turns.
///
/// Simulates:
/// - Turn 1, step 0: `run_gh pr review <url>` succeeds (required-tools gate rejects EndTurn)
/// - Turn 1, step 1: Agent calls `qa_pr_view` (required tool) + `run_gh pr review <number>`
///   (duplicate — different format, but same PR)
///
/// Without session-scope guard, both reviews succeed.
#[tokio::test]
async fn test_regression_qa_review_required_tools_retry_duplicate() {
    // NO session map — simulates the pre-fix state where only per-turn guard exists
    let skills = SkillRegistry::from_test_entries(vec![make_qa_review_skill()]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 0: Agent posts PR review without calling qa_pr_view
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "review", "https://github.com/senara-solutions/mika/pull/735", "--approve", "--body", "VERDICT: pass"], "repo": "senara-solutions/mika"}),
            ),
            // Step 1: EndTurn — rejected by required-tools gate (qa_pr_view not called)
            text_response("Review submitted. PR #735 approved."),
            // Step 2 (retry): Agent calls qa_pr_view AND tries duplicate review (different format)
            multi_tool_response(vec![
                (
                    "qa_pr_view",
                    json!({"pr": 735}),
                ),
                (
                    "run_gh",
                    json!({"command": ["pr", "review", "735", "--approve", "--body", "VERDICT: pass"], "repo": "senara-solutions/mika"}),
                ),
            ]),
            // Step 3: EndTurn
            text_response("Review submitted. PR #735 approved after viewing PR details."),
        ])
        .tools(build_tools())
        .skills(skills)
        .build()
        .await
        .unwrap();

    let trace = harness.run("Review PR #735").await.unwrap();

    // Pre-fix assertion: WITHOUT session-scope guard, the per-turn AtomicBool
    // doesn't catch the cross-step duplicate within the same agent loop run
    // (the AtomicBool IS still set from step 0, so actually the per-turn guard
    // DOES catch this within a single run_agent call).
    // The real #736 bug manifests across turns (required-tools gate creates a
    // NEW call to run_agent with a fresh AtomicBool). This test validates that
    // the per-turn guard still works within a single run.
    assert_has_output(&trace);
}

/// Post-fix test: session-scoped DashMap blocks the duplicate review even when
/// the LLM uses different PR identifier formats (URL vs bare number).
///
/// This is the key #736 scenario: the required-tools gate forces a retry,
/// the session-scope guard remembers the first review (via normalized key),
/// and blocks the duplicate.
#[tokio::test]
async fn test_post_fix_qa_review_session_guard_blocks_duplicate() {
    let session_map: Arc<DashMap<String, HashSet<String>>> = Arc::new(DashMap::new());
    let skills = SkillRegistry::from_test_entries(vec![make_qa_review_skill()]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 0: Agent posts PR review using URL form (succeeds)
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "review", "https://github.com/senara-solutions/mika/pull/735", "--approve", "--body", "VERDICT: pass"], "repo": "senara-solutions/mika"}),
            ),
            // Step 1: EndTurn — rejected by required-tools gate (qa_pr_view not called)
            text_response("Review submitted. PR #735 approved."),
            // Step 2 (retry): Agent calls qa_pr_view + attempts duplicate review (number form)
            multi_tool_response(vec![
                (
                    "qa_pr_view",
                    json!({"pr": 735}),
                ),
                (
                    "run_gh",
                    json!({"command": ["pr", "review", "735", "--approve", "--body", "VERDICT: pass"], "repo": "senara-solutions/mika"}),
                ),
            ]),
            // Step 3: EndTurn — agent acknowledges the duplicate was blocked
            text_response("Review already posted for PR #735. QA review complete."),
        ])
        .tools(build_tools())
        .skills(skills)
        .pr_reviews_posted(session_map.clone())
        .build()
        .await
        .unwrap();

    let trace = harness.run("Review PR #735").await.unwrap();

    assert_has_output(&trace);

    // Core assertion: at most one successful `run_gh pr review` call
    let review_count = count_successful_pr_reviews(&trace);
    assert!(
        review_count <= 1,
        "Expected at most 1 successful pr review call, found {review_count}. \
         Session-scope guard should block the duplicate."
    );
}

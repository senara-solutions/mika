//! Integration tests: qa-review skill-scoped `run_gh` argv validator (mika#1196).
//!
//! Layer B — eval-suite wiring test. Verifies that:
//! 1. When qa-review is in `active_skill_paths`, forbidden `run_gh` subcommands
//!    (e.g., `pr merge`) are rejected with a structured scope error.
//! 2. When qa-review is NOT in `active_skill_paths`, the global allowlist applies
//!    and `run_gh issue edit --remove-label ready` is not rejected by the scope
//!    validator.
//!
//! This tests the *wiring contract*: `active_skill_paths` flows from the agent
//! loop into `ToolContext`, `validate_qa_review_gh_scope` is called inside
//! `run_gh`'s dispatch, and the identity-discrimination invariant holds
//! end-to-end. Layer A (inline tests in `builtin_handlers.rs`) covers the
//! validator function's behavior in isolation.

use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use mika_agent::prompt::Identity;
use mika_agent::skills::SkillRegistry;
use mika_agent::skills::builtin_handlers;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{Constraints, SkillInfo, SkillManifest, Triggers};
use mika_agent::tools::{Tool, ToolContext, ToolOutput, ToolRegistry, default_tools};
use mika_agent::well_known_agents::{IdentitySource, MIKA_DEV};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;

// -- Wrapper tool that delegates to the real builtin execute() --

/// Wrapper `Tool` that delegates to `builtin_handlers::execute("run_gh", ...)`
/// so the real scope validator is exercised end-to-end through the agent loop.
struct BuiltinRunGhTool;

#[async_trait]
impl Tool for BuiltinRunGhTool {
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
        Ok(builtin_handlers::execute("run_gh", input, ctx).await)
    }
}

/// Build a tool registry with default tools plus the builtin run_gh wrapper.
fn tools_with_builtin_run_gh() -> ToolRegistry {
    let mut registry = default_tools();
    registry.register(Box::new(BuiltinRunGhTool));
    registry
}

/// Build a minimal qa-review skill entry with `always_on=true`.
///
/// The `prompt_snippet` must be non-empty for the skill to appear in
/// `active_skill_paths` (the agent loop filters out empty snippets).
/// The `dir` must be under the harness's `home_dir` for `strip_prefix`
/// to succeed.
fn make_qa_review_skill(dir: PathBuf) -> SkillEntry {
    SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: "qa-review".to_string(),
                description: "Quality gate for PR review".to_string(),
                version: "0.9.0".to_string(),
                always_on: true,
                timeout_secs: 30,
                dependencies: vec![],
                max_prompt_size: None,
                data_grade: Default::default(),
            },
            triggers: Triggers {
                keywords: vec![
                    "review".to_string(),
                    "pr".to_string(),
                    "qa".to_string(),
                    "pull request".to_string(),
                ],
            },
            llm: Default::default(),
            constraints: Constraints {
                required_tools: vec![],
                required_fetches_for_quoted_resources: false,
            },
            output: Default::default(),
            context: HashMap::new(),
            variants: Default::default(),
        },
        dir,
        keywords_lower: vec![
            "review".to_string(),
            "pr".to_string(),
            "qa".to_string(),
            "pull request".to_string(),
        ],
        prompt_snippet: "You are the QA review agent. Review PRs.".to_string(),
        skill_tools: vec![],
        enabled: true,
        has_override: false,
        provider_overrides: HashMap::new(),
        prompt_sources: SkillEntry::empty_prompt_sources(),
        model_overrides: HashMap::new(),
    }
}

/// Scenario 1: qa-review active rejects forbidden `pr merge` subcommand.
///
/// When qa-review is in the active skill set (via `always_on=true` and keyword
/// match on "review"), a `run_gh pr merge` tool call should be rejected by
/// `validate_qa_review_gh_scope` with a structured error citing "qa-review's
/// scope" and "mika#1196".
#[tokio::test]
async fn qa_review_active_rejects_pr_merge() {
    // Build harness, then create skill dir under its home_dir and replace
    // the skills registry. The harness.skills field is pub, allowing post-build
    // replacement.
    let mut harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("run_gh", json!({"command": ["pr", "merge", "123"]})),
            text_response("The pr merge command was rejected by the qa-review scope validator."),
        ])
        .tools(tools_with_builtin_run_gh())
        .build()
        .await
        .unwrap();

    // Create the skill directory and files under the harness home_dir
    let skill_dir = harness.home_dir.path().join("skills/qa-review");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("system_prompt.md"),
        "You are the QA review agent. Review PRs.",
    )
    .unwrap();

    // Replace the skills registry with one containing qa-review
    harness.skills = SkillRegistry::from_test_entries(vec![make_qa_review_skill(skill_dir)]);

    // Trigger qa-review via keyword "review" in the user message
    let trace = harness.run("Review PR #455").await.unwrap();

    // Assert that run_gh was called and the output contains the scope rejection
    assert_tool_output_contains(&trace, "run_gh", 0, "qa-review's scope");
    assert_tool_output_contains(&trace, "run_gh", 0, "mika#1196");
}

/// Scenario 2: qa-review NOT active accepts dispatch-ack `issue edit`.
///
/// When qa-review is NOT in the active skill set, the scope validator's
/// early-return path fires and the global allowlist applies. Since the
/// real subprocess would attempt to call `gh` and may fail in the test
/// environment, we assert specifically against the *scope-rejection error
/// string's absence*, not subprocess success.
#[tokio::test]
async fn qa_review_not_active_accepts_issue_edit() {
    // No qa-review skill in the registry — empty skill set.
    // Use the builtin run_gh wrapper so the real validator is exercised.
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response(
                "run_gh",
                json!({"command": ["issue", "edit", "123", "--remove-label", "ready"]}),
            ),
            text_response("Label removed successfully."),
        ])
        .tools(tools_with_builtin_run_gh())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("Remove the ready label from issue 123")
        .await
        .unwrap();

    // Assert that run_gh was called
    assert_tools_include(&trace, &["run_gh"]);

    // Assert the scope-rejection error is NOT present — the validator did not fire
    let calls = trace.calls_for_tool("run_gh");
    assert!(
        !calls.is_empty(),
        "run_gh should have been called at least once"
    );
    let output = calls[0].output.as_deref().unwrap_or("");
    assert!(
        !output.contains("qa-review's scope"),
        "qa-review scope validator should NOT fire when qa-review is not active, \
         but got output: {output}"
    );
}

/// Scenario 3: post-reconciliation, mika-dev's allowlist excludes qa-review,
/// so the scope validator does not fire on `gh issue edit` (mika#1220).
///
/// Wires together the identity reconciler with the scope validator:
/// 1. Seed a registry that contains qa-review (mirroring the pre-fix on-disk shape).
/// 2. Apply `MIKA_DEV_IDENTITY`'s `[skills].allowlist` via `apply_identity_allowlist`.
/// 3. Verify qa-review is evicted from the registry — qa-review is not in mika-dev's
///    allowlist (mika-dev's 26-skill allowlist contains the dev-pilot family, not
///    qa-review which is mika-qa's territory).
/// 4. Run `gh issue edit … --remove-label ready` through `BuiltinRunGhTool` and
///    assert no scope rejection — the post-reconciliation happy path.
#[tokio::test]
async fn qa_review_evicted_by_mika_dev_allowlist_after_reconcile() {
    let mut harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response(
                "run_gh",
                json!({"command": ["issue", "edit", "1205", "--remove-label", "ready"]}),
            ),
            text_response("Label removed successfully (post-reconciliation)."),
        ])
        .tools(tools_with_builtin_run_gh())
        .build()
        .await
        .unwrap();

    // Seed a qa-review skill into the registry, mirroring the pre-fix on-disk state
    // where qa-review was bundled and always_on=true.
    let skill_dir = harness.home_dir.path().join("skills/qa-review");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("system_prompt.md"),
        "You are the QA review agent. Review PRs.",
    )
    .unwrap();
    harness.skills = SkillRegistry::from_test_entries(vec![make_qa_review_skill(skill_dir)]);

    // Apply mika-dev's identity allowlist — qa-review is NOT in it, so the registry
    // must evict qa-review. This is the post-reconciliation steady state. Parse from
    // MIKA_DEV's actual identity_source so the test stays in lockstep with the spec.
    let dev_identity_str = match MIKA_DEV
        .identity_source
        .as_ref()
        .expect("mika-dev must declare identity_source")
    {
        IdentitySource::Static(s) => *s,
        IdentitySource::Computed(_) => panic!("mika-dev identity_source must be Static"),
    };
    let dev_identity: Identity =
        toml::from_str(dev_identity_str).expect("MIKA_DEV identity must parse");
    let dev_allowlist = dev_identity
        .skills
        .allowlist
        .expect("MIKA_DEV identity must declare [skills].allowlist");
    harness.skills.apply_identity_allowlist(&dev_allowlist);

    assert!(
        !harness
            .skills
            .skills()
            .iter()
            .any(|s| s.manifest.skill.name == "qa-review"),
        "qa-review must be evicted from mika-dev's post-reconciliation registry"
    );

    // Use a keyword that does NOT match any mika-dev allowlist skill, so the user
    // message does not trigger keyword-based skill activation.
    let trace = harness
        .run("Remove the ready label from issue 1205")
        .await
        .unwrap();

    assert_tools_include(&trace, &["run_gh"]);
    let calls = trace.calls_for_tool("run_gh");
    let output = calls[0].output.as_deref().unwrap_or("");
    assert!(
        !output.contains("qa-review's scope"),
        "post-reconciliation: qa-review scope validator must NOT fire for mika-dev's \
         gh issue edit call, but got output: {output}"
    );
    assert!(
        !output.contains("mika#1196"),
        "post-reconciliation: scope-rejection citation must not appear, but got: {output}"
    );
}

//! Scenario 38: Cross-artifact equivalence-claim caught (mika#1645)
//!
//! Context: a qa-review turn emits "Duplicate of merged mika#1638 — content
//! identical" without ever fetching mika#1638's file set (the verbatim mika#1644
//! founding incident). The qa-review-scoped equivalence-claim EndTurn guard
//! (`guard.equivalence_claim`) detects the cross-artifact equivalence assertion,
//! rejects the turn, and re-prompts the reviewer to fetch the compared artifact
//! or hedge.
//!
//! The guard is scoped to qa-review via the `qa_pr_view` tool's presence in the
//! turn-start enabled set — so the skill must be loaded for the guard to fire.
//! Turn 1 satisfies the required-tools gate (qa_pr_view + run_gh of the CURRENT
//! PR) so that the re-prompt observed in this test is attributable to the
//! equivalence guard, not the required-tools gate.
//!
//! ## Hard Assertions
//! - Guard fires: LLM call count > 1 (re-prompt occurred).
//! - On the corrective turn the reviewer fetches the compared artifact (`run_gh`
//!   referencing #1638).
//!
//! Reference: mika#1645, PR #1644 (founding incident), mika#1331 (parent class)

use std::collections::HashMap;
use std::path::PathBuf;

use mika_agent::skills::SkillRegistry;
use mika_agent::skills::index::{ResolvedSkillTool, SkillEntry};
use mika_agent::skills::manifest::{Constraints, SkillInfo, SkillManifest, ToolHandler, Triggers};
use mika_agent::tools::{Tool, ToolContext, ToolOutput, ToolRegistry};
use mika_common::claude::ToolDefinition;

use async_trait::async_trait;

use super::*;

/// qa-review skill entry exposing `qa_pr_view` (the guard's scope signal) and
/// declaring `required_tools = ["qa_pr_view", "run_gh"]`.
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
            },
            triggers: Triggers {
                keywords: vec!["review".to_string(), "pr review".to_string()],
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
        keywords_lower: vec!["review".to_string(), "pr review".to_string()],
        prompt_snippet: String::new(),
        skill_tools: vec![ResolvedSkillTool {
            definition: ToolDefinition {
                name: "qa_pr_view".to_string(),
                description: "View PR details for QA review".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": { "pr_url": {"type": "string"} },
                    "required": ["pr_url"]
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
                "properties": { "pr_url": {"type": "string"} },
                "required": ["pr_url"]
            }),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            "PR #1644: feat(calibration): mika-qa role calibration scenarios\n\
             Files: crates/mika-agent/src/calibration/roles/mika_qa.rs, ..."
                .to_string(),
        ))
    }
}

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
                "properties": { "args": {"type": "string"} },
                "required": ["args"]
            }),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            "skills/bundled/_shared/dispatch-lib.sh".to_string(),
        ))
    }
}

fn build_tools() -> ToolRegistry {
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(StubQaPrViewTool));
    tools.register(Box::new(StubRunGh));
    tools
}

/// qa-review claims "duplicate of mika#1638 — content identical" having fetched
/// only the current PR (#1644) → equivalence guard fires → reviewer corrects by
/// fetching #1638.
#[tokio::test]
async fn test_equivalence_claim_caught() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_qa_review_skill()]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1 step 0: fetch the CURRENT PR only (satisfies required-tools
            // gate: qa_pr_view + run_gh), but never fetches the compared #1638.
            multi_tool_response(vec![
                (
                    "qa_pr_view",
                    json!({"pr_url": "https://github.com/senara-solutions/mika/pull/1644"}),
                ),
                ("run_gh", json!({"args": "pr diff 1644 --name-only"})),
            ]),
            // Turn 1 step 1: EndTurn with a bare cross-artifact equivalence claim.
            // The equivalence-claim guard must reject this.
            text_response(
                "VERDICT: hold[review]\nDEPTH: code-level\nREASON: Duplicate of merged \
                 mika#1638 — content identical; dispatch-lib opened a second wip-rescue \
                 vehicle for the same implementation.",
            ),
            // Turn 2 (after corrective re-prompt): fetch the compared artifact #1638.
            tool_call_response("run_gh", json!({"args": "pr diff 1638 --name-only"})),
            // Turn 3: grounded verdict citing the file-set comparison.
            text_response(
                "VERDICT: pass\nDEPTH: code-level\nREASON: Compared file sets via run_gh \
                 pr diff — #1644 and #1638 share only .claude/groom-verdict-trail.log; \
                 zero source overlap. Not a duplicate.",
            ),
        ])
        .tools(build_tools())
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run("Review PR #1644 against the plan ACs").await?;

    // Hard: guard fired. Baseline for this response sequence WITHOUT the guard
    // is 2 LLM calls (step 0 = multi-tool fetch of the current PR, step 1 = text
    // EndTurn accepted). The equivalence guard rejecting step 1 forces the
    // corrective turns (steps 2-3), pushing the count to 4. `> 2` therefore
    // proves the guard fired rather than the turn being accepted at baseline.
    assert!(
        trace.llm_call_count > 2,
        "Expected equivalence-claim guard to fire and re-prompt (llm_call_count > 2, \
         baseline is 2), got {}",
        trace.llm_call_count
    );

    // Hard: the reviewer fetched the compared artifact (#1638) on the corrective turn.
    let fetched_compared = trace.tool_calls.iter().any(|tc| {
        tc.tool_name == "run_gh"
            && tc
                .input
                .as_deref()
                .map(|i| i.contains("1638"))
                .unwrap_or(false)
    });
    assert!(
        fetched_compared,
        "Expected a run_gh call referencing the compared artifact #1638 after re-prompt"
    );

    Ok(())
}

/// Control: when the reviewer fetches the compared artifact (#1638) in the SAME
/// turn as the equivalence claim, the guard is satisfied and does NOT fire.
#[tokio::test]
async fn test_equivalence_claim_satisfied_no_fire() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_qa_review_skill()]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1 step 0: fetch BOTH the current PR and the compared #1638.
            multi_tool_response(vec![
                (
                    "qa_pr_view",
                    json!({"pr_url": "https://github.com/senara-solutions/mika/pull/1644"}),
                ),
                ("run_gh", json!({"args": "pr diff 1638 --name-only"})),
            ]),
            // Turn 1 step 1: EndTurn asserting equivalence WITH the comparison grounded.
            text_response(
                "VERDICT: hold[review]\nDEPTH: code-level\nREASON: Compared file sets — \
                 this PR is content identical to mika#1638 (same dispatch-lib.sh change).",
            ),
        ])
        .tools(build_tools())
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run("Review PR #1644 against the plan ACs").await?;

    // Hard: guard did NOT fire — exactly the baseline 2 LLM calls (step 0 =
    // multi-tool fetch of both PRs, step 1 = grounded text EndTurn accepted). A
    // spurious guard firing would force a re-prompt (count > 2) and exhaust the
    // 2-entry response list.
    assert_eq!(
        trace.llm_call_count, 2,
        "Guard must not fire when the compared artifact (#1638) was fetched this turn; \
         expected baseline 2 LLM calls, got {}",
        trace.llm_call_count
    );

    Ok(())
}

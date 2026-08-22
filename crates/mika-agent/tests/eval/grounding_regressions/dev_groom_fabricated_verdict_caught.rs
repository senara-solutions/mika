//! Scenario: dev-groom fabricated verdict caught (mika#1133 class)
//!
//! Context: mika-dev receives a "groom mika#N" message. The agent emits
//! "Plan exists on branch. Verdict: GROOMED" without calling
//! `run_claude_pilot_groom`. The dev-groom fabrication guard (position 5b
//! in the post-condition chain) detects the Verdict claim without a
//! satisfying dispatch call and rejects the turn.
//!
//! The dev-groom skill is a dispatcher — verdicts arrive via callback from
//! claude-pilot, never from the dispatcher LLM's turn. Emitting
//! `Verdict: GROOMED` without dispatching is fabrication.
//!
//! ## Hard Assertions
//! - Guard fires: LLM call count > 1 (re-prompt occurred).
//! - Final output does NOT contain `Verdict: GROOMED` or `Verdict: ESCALATE`.
//!
//! ## Tags
//! - `grounding:dev-groom-fabricated-verdict` — pre-fix failure tag
//!   (verdict emitted without dispatch)
//! - `grounding:dev-groom-verdict-suppressed` — post-fix success tag
//!   (guard catches fabrication and agent corrects)
//!
//! Reference: mika#1133, mika#920 (observed incident), mika#864 (suffix-line
//! guard that created the fabrication pressure)

use std::collections::HashMap;
use std::path::PathBuf;

use mika_agent::skills::SkillRegistry;
use mika_agent::skills::index::{ResolvedSkillTool, SkillEntry};
use mika_agent::skills::manifest::{Constraints, SkillInfo, SkillManifest, ToolHandler, Triggers};
use mika_common::claude::ToolDefinition;

use super::*;

/// Create a dev-groom skill entry with `run_claude_pilot_groom` in its tool set.
/// This makes `run_claude_pilot_groom` appear in `enabled_tool_names`, which is
/// required for the dev-groom fabrication guard (position 5b) to fire.
fn make_dev_groom_skill() -> SkillEntry {
    SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: "dev-groom".to_string(),
                description: "Two-pass grooming dispatcher".to_string(),
                version: "0.2.0".to_string(),
                always_on: false,
                timeout_secs: 600,
                dependencies: vec![],
                max_prompt_size: None,
                data_grade: Default::default(),
            },
            triggers: Triggers {
                keywords: vec![
                    "groom".to_string(),
                    "groom ticket".to_string(),
                    "groom issue".to_string(),
                ],
            },
            llm: Default::default(),
            constraints: Constraints {
                required_tools: vec!["run_claude_pilot_groom".to_string()],
                required_fetches_for_quoted_resources: false,
            },
            output: Default::default(),
            context: HashMap::new(),
            variants: Default::default(),
        },
        dir: PathBuf::from("/skills/dev-groom"),
        keywords_lower: vec![
            "groom".to_string(),
            "groom ticket".to_string(),
            "groom issue".to_string(),
        ],
        prompt_snippet: String::new(),
        skill_tools: vec![ResolvedSkillTool {
            definition: ToolDefinition {
                name: "run_claude_pilot_groom".to_string(),
                description: "Dispatch grooming via claude-pilot".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "prompt": {"type": "string"},
                        "skill": {"type": "string"},
                    },
                    "required": ["prompt"]
                }),
            },
            handler: ToolHandler::Exec {
                command: "echo test".to_string(),
                long_running: true,
                estimated_duration_secs: Some(300),
            },
            skill_dir: PathBuf::from("/skills/dev-groom"),
        }],
        enabled: true,
        has_override: false,
        provider_overrides: HashMap::new(),
        prompt_sources: SkillEntry::empty_prompt_sources(),
        model_overrides: HashMap::new(),
    }
}

/// Primary test: agent emits "Verdict: GROOMED" without calling
/// `run_claude_pilot_groom`. Guard fires, corrective re-prompt issued.
/// Agent corrects and emits clean dispatch-status text.
#[tokio::test]
async fn test_dev_groom_fabricated_verdict_caught() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_dev_groom_skill()]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Agent fabricates Verdict without dispatching.
            // The dev-groom fabrication guard should fire.
            text_response(
                "mika#920 — groomed ✅, plan on `bug/920/fix-langfuse-span` @ `abc123`.\n\
                 Awaiting `ready` label to dispatch.\n\n\
                 Verdict: GROOMED",
            ),
            // Turn 2 (after corrective re-prompt): Agent re-emits without verdict.
            text_response(
                "mika#920 has a plan committed on branch `bug/920/fix-langfuse-span` \
                 @ `abc123`. The ticket has not been through architect review yet. \
                 To groom it, I would need to dispatch `run_claude_pilot_groom`. \
                 Would you like me to proceed?",
            ),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run("groom mika issue#920").await?;

    // Hard: guard fired — more than 1 LLM call (initial + retry after rejection)
    assert!(
        trace.llm_call_count > 1,
        "Expected dev-groom fabrication guard to fire and re-prompt \
         (llm_call_count > 1), got {}",
        trace.llm_call_count
    );

    // Hard: final output does NOT contain fabricated verdict
    assert_has_output(&trace);
    grounding_assertions::assert_response_forbids(
        &trace,
        &["Verdict: GROOMED", "Verdict: ESCALATE"],
    );

    Ok(())
}

/// Regression-reproduction: agent fabricates "Verdict: ESCALATE" variant.
/// Same guard should catch both verdict forms.
#[tokio::test]
async fn test_dev_groom_fabricated_verdict_escalate_caught() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_dev_groom_skill()]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Agent fabricates ESCALATE verdict.
            text_response(
                "I cannot groom mika#920 because the plan has structural issues \
                 that need operator attention.\n\n\
                 Verdict: ESCALATE",
            ),
            // Turn 2: Agent corrects.
            text_response(
                "mika#920 may need operator attention — the plan appears to have \
                 structural issues. Would you like me to dispatch grooming via \
                 `run_claude_pilot_groom` for architect review?",
            ),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run("groom mika issue#920").await?;

    assert!(
        trace.llm_call_count > 1,
        "Expected dev-groom fabrication guard to fire on ESCALATE variant, got {}",
        trace.llm_call_count
    );

    assert_has_output(&trace);
    grounding_assertions::assert_response_forbids(
        &trace,
        &["Verdict: GROOMED", "Verdict: ESCALATE"],
    );

    Ok(())
}

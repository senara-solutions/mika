//! Scenario: Required suffix line — caught verdict ghosting (mika#864 class)
//!
//! Context: a skill declares `[output] required_suffix_lines = ["Verdict: GROOMED",
//! "Verdict: ESCALATE"]`. The agent emits a verdict-shaped response but omits the
//! literal verdict line. The required-suffix-line EndTurn guard detects the missing
//! line, rejects the turn, and re-prompts. On the retry, the agent appends the
//! required verdict line.
//!
//! ## Hard Assertions
//! - Guard fires: LLM call count is > 1 (re-prompt occurred).
//! - Agent produces output text containing `Verdict: GROOMED`.
//!
//! ## Tags
//! - `grounding:verdict-suffix-required-but-ghosted` — pre-fix failure tag
//!   (verdict line omitted under cognitive load)
//! - `grounding:verdict-suffix-emitted` — post-fix success tag
//!   (verdict line emitted after corrective re-prompt)
//!
//! ## Frozen Fixture
//! - `fixtures/required_suffix_line_caught_pre_fix.json` — pre-fix response
//!   reproducing the mika#788 trace shape (verdict-shaped response without
//!   the literal verdict line).
//!
//! Reference: mika#788, mika#864, gate-evasion compound doc

use std::collections::HashMap;
use std::path::PathBuf;

use mika_agent::skills::SkillRegistry;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{Output, SkillInfo, SkillManifest, Triggers};
use mika_common::llm::mock::*;

use super::*;

/// Create a skill entry with `required_suffix_lines` configured,
/// mimicking the mika-arch-second-review config.
fn make_suffix_skill(name: &str, keywords: &[&str], suffix_lines: &[&str]) -> SkillEntry {
    SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: name.to_string(),
                description: format!("{name} test skill"),
                version: "0.1.0".to_string(),
                always_on: false,
                timeout_secs: 30,
                dependencies: vec![],
                max_prompt_size: None,
            },
            triggers: Triggers {
                keywords: keywords.iter().map(|s| s.to_string()).collect(),
            },
            llm: Default::default(),
            constraints: Default::default(),
            output: Output {
                required_suffix_lines: suffix_lines.iter().map(|s| s.to_string()).collect(),
                required_finding_list_prefixes: vec![],
            },
            context: HashMap::new(),
            variants: Default::default(),
        },
        dir: PathBuf::from(format!("/skills/{name}")),
        keywords_lower: keywords.iter().map(|s| s.to_lowercase()).collect(),
        prompt_snippet: String::new(),
        skill_tools: vec![],
        enabled: true,
        has_override: false,
        provider_overrides: HashMap::new(),
        model_prompts: HashMap::new(),
        model_overrides: HashMap::new(),
        generated_model_prompts: HashMap::new(),
    }
}

/// Primary test: agent emits verdict-shaped response without `Verdict: GROOMED`
/// on its own line. Guard fires, corrective re-prompt issued. Agent corrects and
/// appends the verdict line on retry.
#[tokio::test]
async fn test_required_suffix_line_caught_and_corrected() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_suffix_skill(
        "arch-second-review",
        &["second-review"],
        &["Verdict: GROOMED", "Verdict: ESCALATE"],
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Agent emits verdict-shaped text without literal verdict line.
            // The required-suffix-line guard should fire.
            text_response(
                "I've reviewed the revised plan and all seven findings from the \
                 first pass have been adequately addressed.\n\n\
                 All findings addressed. The plan is ready for implementation.",
            ),
            // Turn 2 (after corrective re-prompt): Agent re-emits with verdict appended.
            text_response(
                "I've reviewed the revised plan and all seven findings from the \
                 first pass have been adequately addressed.\n\n\
                 All findings addressed. The plan is ready for implementation.\n\n\
                 Verdict: GROOMED",
            ),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness
        .run("second-review: Review the revised plan for mika#864.")
        .await?;

    // Hard: guard fired — more than 1 LLM call (initial + retry after rejection)
    assert!(
        trace.llm_call_count > 1,
        "Expected guard to fire and re-prompt (llm_call_count > 1), got {}",
        trace.llm_call_count
    );

    // Hard: agent produced output containing the required verdict
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "Verdict: GROOMED");

    Ok(())
}

/// Regression-reproduction test: simulates the pre-fix mika#788 trace shape.
/// Guard catches the first response, but agent still doesn't include verdict
/// on retry (single-retry semantics — guard is exhausted).
#[tokio::test]
async fn test_regression_required_suffix_line_pre_fix_shape() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_suffix_skill(
        "arch-second-review",
        &["second-review"],
        &["Verdict: GROOMED", "Verdict: ESCALATE"],
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: No verdict line (guard fires, rejected)
            text_response("All findings addressed. The plan is ready for implementation."),
            // Turn 2: Still no verdict line (guard exhausted, goes through)
            text_response(
                "The plan has been thoroughly reviewed and all acceptance \
                 criteria are met.",
            ),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness
        .run("second-review: Review the revised plan for mika#864.")
        .await?;

    // Guard fired exactly once — 2 LLM calls total
    assert_eq!(
        trace.llm_call_count, 2,
        "Expected exactly 1 guard-retry (2 LLM calls), got {}",
        trace.llm_call_count
    );

    // Output does NOT contain the verdict — this IS the failure class
    let output_text = trace.output.text.as_deref().unwrap_or("");
    assert!(
        !output_text.contains("Verdict: GROOMED") && !output_text.contains("Verdict: ESCALATE"),
        "Pre-fix shape: verdict line should NOT be present"
    );

    Ok(())
}

/// Boundary test (F2-S scenario 3): verdict at position 3 from end (inclusive bound).
/// The verdict line is the third-from-last non-empty line — should satisfy the guard.
#[tokio::test]
async fn test_required_suffix_line_position_3_boundary() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_suffix_skill(
        "arch-second-review",
        &["second-review"],
        &["Verdict: GROOMED", "Verdict: ESCALATE"],
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Verdict at position 3 from end (two trailing non-verdict lines).
            // Line N-2: "Verdict: GROOMED"
            // Line N-1: "This concludes the review."
            // Line N:   "No further action required."
            text_response(
                "I've reviewed the plan thoroughly.\n\n\
                 Verdict: GROOMED\n\
                 This concludes the review.\n\
                 No further action required.",
            ),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness
        .run("second-review: Review the revised plan.")
        .await?;

    // Guard should NOT fire — verdict is within last 3 non-empty lines
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected no guard-retry (verdict at position 3 satisfies), got {} LLM calls",
        trace.llm_call_count
    );

    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "Verdict: GROOMED");

    Ok(())
}

/// Boundary test (F2-S scenario 4): verdict at position 4 from end (exclusive bound).
/// The verdict line is the fourth-from-last non-empty line — should NOT satisfy.
#[tokio::test]
async fn test_required_suffix_line_position_4_violation() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_suffix_skill(
        "arch-second-review",
        &["second-review"],
        &["Verdict: GROOMED", "Verdict: ESCALATE"],
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Verdict at position 4 from end (three trailing non-verdict lines).
            // The guard should fire.
            text_response(
                "I've reviewed the plan thoroughly.\n\n\
                 Verdict: GROOMED\n\
                 Finding 1: resolved.\n\
                 Finding 2: resolved.\n\
                 No further action required.",
            ),
            // Turn 2: Agent corrects after re-prompt.
            text_response(
                "I've reviewed the plan thoroughly.\n\n\
                 Finding 1: resolved.\n\
                 Finding 2: resolved.\n\
                 No further action required.\n\n\
                 Verdict: GROOMED",
            ),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness
        .run("second-review: Review the revised plan.")
        .await?;

    // Guard should fire — verdict is OUTSIDE last 3 non-empty lines
    assert!(
        trace.llm_call_count > 1,
        "Expected guard to fire (verdict at position 4 violates), got {} LLM calls",
        trace.llm_call_count
    );

    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "Verdict: GROOMED");

    Ok(())
}

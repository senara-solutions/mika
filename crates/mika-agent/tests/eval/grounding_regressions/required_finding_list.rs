//! Scenario: Required finding-list guard — conditional F-list emission (mika#901)
//!
//! Context: mika-arch's grooming skills (`mika-arch-groom-ticket`, `mika-arch-second-review`)
//! declare `[output] required_finding_list_prefixes = ["F1:", "F2:", ..., "F10:"]`. When the
//! disposition is terminal (ITERATE/ESCALATE), the engine requires at least one F-list line
//! in the message body (up to the suffix-line landmark). On READY/GROOMED, the guard is
//! bypassed per operator spec.
//!
//! ## Hard Assertions
//! - Guard fires on thin emission with terminal disposition.
//! - Guard does NOT fire on READY/GROOMED disposition.
//! - Guard does NOT fire when skill doesn't declare the field.
//! - Scan window respects suffix-line landmark boundary.
//!
//! ## Tags
//! - `grounding:finding-list-emission-required` — post-fix success tag
//! - `grounding:thin-emission-evasion` — pre-fix failure tag
//!
//! Reference: mika#901, mika#876, conditional-disclosure-evasion failure class

use std::collections::HashMap;
use std::path::PathBuf;

use mika_agent::skills::SkillRegistry;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{Output, SkillInfo, SkillManifest, Triggers};
use mika_common::llm::mock::*;

use super::*;

/// Create a skill entry with both `required_suffix_lines` and
/// `required_finding_list_prefixes` configured, mimicking mika-arch-groom-ticket.
fn make_groom_skill(
    name: &str,
    keywords: &[&str],
    suffix_lines: &[&str],
    finding_prefixes: &[&str],
) -> SkillEntry {
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
                required_finding_list_prefixes: finding_prefixes
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                required_tool_arg_suffixes: vec![],
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
        prompt_sources: SkillEntry::empty_prompt_sources(),
        model_overrides: HashMap::new(),
    }
}

/// Create a skill entry with suffix lines but NO finding-list prefixes.
fn make_skill_without_finding_list(
    name: &str,
    keywords: &[&str],
    suffix_lines: &[&str],
) -> SkillEntry {
    make_groom_skill(name, keywords, suffix_lines, &[])
}

const GROOM_SUFFIX_LINES: &[&str] = &[
    "Disposition: READY",
    "Disposition: ITERATE",
    "Disposition: ESCALATE",
];
const SECOND_REVIEW_SUFFIX_LINES: &[&str] = &["Verdict: GROOMED", "Verdict: ESCALATE"];
const FINDING_PREFIXES: &[&str] = &[
    "F1:", "F2:", "F3:", "F4:", "F5:", "F6:", "F7:", "F8:", "F9:", "F10:",
];

/// Test 1: Guard fires on thin emission with ITERATE disposition.
/// Replays the mika#876 first-pass thin-emission failure shape.
#[tokio::test]
async fn test_required_finding_list_caught_on_iterate() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_groom_skill(
        "arch-groom-ticket",
        &["groom-ticket"],
        GROOM_SUFFIX_LINES,
        FINDING_PREFIXES,
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Thin emission — disposition ITERATE but no F-list.
            // This is the mika#876 failure shape.
            text_response(
                "Both patterns persisted. N=5 evasion now recorded. \
                 Disposition stands: ITERATE for the plan, \
                 four blockers plus one sharpening.\n\n\
                 Disposition: ITERATE",
            ),
            // Turn 2 (after corrective re-prompt): Agent re-emits with F-list.
            text_response(
                "F1: (BLOCKING) Plan implements unconditional emission but \
                 issue body marks it out of scope.\n\
                    Concern: Spec divergence.\n\
                    Change required: Resolve the constraint.\n\
                    Citation: review-guide.md YAGNI\n\n\
                 F2: (sharpening) Missing boundary test.\n\
                    Concern: Unit 4 tests incomplete.\n\
                    Change required: Add position boundary tests.\n\
                    Citation: best-practices doc\n\n\
                 Disposition: ITERATE",
            ),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness
        .run("groom-ticket: Review the plan for mika#876.")
        .await?;

    // Hard: guard fired — more than 1 LLM call
    assert!(
        trace.llm_call_count > 1,
        "Expected finding-list guard to fire and re-prompt (llm_call_count > 1), got {}",
        trace.llm_call_count
    );

    // Hard: agent produced output containing F-list entries
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "F1");
    grounding_assertions::assert_response_contains(&trace, "F2");

    Ok(())
}

/// Test 2: Guard does NOT fire on READY disposition (operator-spec exemption per R1).
#[tokio::test]
async fn test_required_finding_list_no_op_on_ready() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_groom_skill(
        "arch-groom-ticket",
        &["groom-ticket"],
        GROOM_SUFFIX_LINES,
        FINDING_PREFIXES,
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: READY disposition, no F-list. Guard should NOT fire.
            text_response(
                "Plan-on-branch ratifies the architect's review. \
                 No remaining concerns.\n\n\
                 Disposition: READY",
            ),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness
        .run("groom-ticket: Review the plan for mika#901.")
        .await?;

    // Guard should NOT fire — READY is non-terminal
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected no guard-retry on READY disposition (1 LLM call), got {}",
        trace.llm_call_count
    );

    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "Disposition: READY");

    Ok(())
}

/// Test 3: Guard does NOT fire when skill does NOT declare finding-list prefixes.
#[tokio::test]
async fn test_required_finding_list_no_op_when_unset() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_skill_without_finding_list(
        "qa-review",
        &["qa-review"],
        &["Verdict: PASS", "Verdict: BLOCK"],
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Thin emission with verdict, but skill has no finding-list prefixes.
            text_response(
                "All checks pass. The PR is ready to merge.\n\n\
                 Verdict: PASS",
            ),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run("qa-review: Review the PR.").await?;

    // Guard should NOT fire — skill doesn't declare finding-list prefixes
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected no guard-retry when skill has no finding-list prefixes, got {}",
        trace.llm_call_count
    );

    Ok(())
}

/// Test 4: F-list line immediately before the suffix-line landmark — should satisfy.
#[tokio::test]
async fn test_required_finding_list_position_inclusive() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_groom_skill(
        "arch-groom-ticket",
        &["groom-ticket"],
        GROOM_SUFFIX_LINES,
        FINDING_PREFIXES,
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: F-list immediately before the Disposition line.
            text_response(
                "Review complete.\n\n\
                 F1: (BLOCKING) Issue found with the approach.\n\
                    Concern: Missing error handling.\n\
                    Change required: Add error boundary.\n\
                    Citation: review-guide.md § KISS\n\n\
                 Disposition: ITERATE",
            ),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run("groom-ticket: Review the plan.").await?;

    // Guard should NOT fire — F-list is within scan range (before suffix line)
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected no guard-retry (F-list before suffix line satisfies), got {} LLM calls",
        trace.llm_call_count
    );

    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "F1");

    Ok(())
}

/// Test 5: F-list line AFTER the suffix-line landmark — should NOT satisfy.
#[tokio::test]
async fn test_required_finding_list_position_exclusive() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_groom_skill(
        "arch-groom-ticket",
        &["groom-ticket"],
        GROOM_SUFFIX_LINES,
        FINDING_PREFIXES,
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: F-list AFTER the Disposition line (in a postscript).
            // Guard should fire because F-list is outside scan range.
            text_response(
                "Review complete. Four concerns identified.\n\n\
                 Disposition: ITERATE\n\n\
                 F1: (BLOCKING) This finding is after the disposition line.\n\
                    Concern: Wrong position.\n\
                    Change required: Move before disposition.\n\
                    Citation: review-guide.md",
            ),
            // Turn 2: Agent corrects with F-list before disposition.
            text_response(
                "Review complete.\n\n\
                 F1: (BLOCKING) Finding moved to correct position.\n\
                    Concern: Position corrected.\n\
                    Change required: Already addressed.\n\
                    Citation: review-guide.md\n\n\
                 Disposition: ITERATE",
            ),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run("groom-ticket: Review the plan.").await?;

    // Guard should fire — F-list is AFTER suffix line (outside scan range)
    assert!(
        trace.llm_call_count > 1,
        "Expected guard to fire (F-list after suffix line violates), got {} LLM calls",
        trace.llm_call_count
    );

    Ok(())
}

/// Test 6: F-list at message start — should satisfy (no lower bound on scan range).
#[tokio::test]
async fn test_required_finding_list_position_at_message_start() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_groom_skill(
        "arch-groom-ticket",
        &["groom-ticket"],
        GROOM_SUFFIX_LINES,
        FINDING_PREFIXES,
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: F-list at the very start of the message.
            text_response(
                "F1: (BLOCKING) Critical issue at message start.\n\
                    Concern: Immediate problem.\n\
                    Change required: Fix immediately.\n\
                    Citation: review-guide.md § SRP\n\n\
                 Further analysis follows...\n\n\
                 Disposition: ESCALATE",
            ),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run("groom-ticket: Review the plan.").await?;

    // Guard should NOT fire — F-list at line 0 is within scan range
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected no guard-retry (F-list at message start satisfies), got {} LLM calls",
        trace.llm_call_count
    );

    Ok(())
}

/// Test 7: Guard fires on ESCALATE with second-review skill (Verdict: ESCALATE).
#[tokio::test]
async fn test_required_finding_list_caught_on_verdict_escalate() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_groom_skill(
        "arch-second-review",
        &["second-review"],
        SECOND_REVIEW_SUFFIX_LINES,
        FINDING_PREFIXES,
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: ESCALATE verdict but no F-list.
            text_response(
                "Unresolved concerns remain from the first pass. \
                 Escalating to human judgment.\n\n\
                 Verdict: ESCALATE",
            ),
            // Turn 2: Agent corrects with F-list.
            text_response(
                "F1: (BLOCKING) Prior finding F2 unresolved.\n\
                    Concern: Revision defers to follow-up ticket.\n\
                    Change required: Resolve in this plan.\n\
                    Citation: review-guide.md § SRP\n\n\
                 Verdict: ESCALATE",
            ),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness
        .run("second-review: Review the revised plan.")
        .await?;

    // Guard should fire — ESCALATE is terminal for second-review
    assert!(
        trace.llm_call_count > 1,
        "Expected finding-list guard to fire on Verdict: ESCALATE, got {} LLM calls",
        trace.llm_call_count
    );

    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "F1");

    Ok(())
}

/// Test 7b: Guard does NOT fire on GROOMED verdict (non-terminal for second-review).
///
/// Skill name uses the production identifier `mika-arch-second-review` (not the
/// test-only `arch-second-review` shorthand used elsewhere in this file) because
/// mika#1254's fix inverted the guard predicate from "tool present" to
/// `!has_verdict_producer_skill` against the `VERDICT_PRODUCER_SKILLS` const in
/// `agent.rs`. That const lists the canonical production names; the test must
/// use them to exercise the verdict-producer-exempt path correctly.
#[tokio::test]
async fn test_required_finding_list_no_op_on_verdict_groomed() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_groom_skill(
        "mika-arch-second-review",
        &["second-review"],
        SECOND_REVIEW_SUFFIX_LINES,
        FINDING_PREFIXES,
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: GROOMED verdict, no F-list. Should pass.
            text_response(
                "All prior findings resolved. The plan is ready for implementation.\n\n\
                 Verdict: GROOMED",
            ),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness
        .run("second-review: Review the revised plan.")
        .await?;

    // Guard should NOT fire — GROOMED is non-terminal
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected no guard-retry on Verdict: GROOMED, got {} LLM calls",
        trace.llm_call_count
    );

    Ok(())
}

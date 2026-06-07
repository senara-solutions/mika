//! Scenario: verdict-producer agent legitimately emits Verdict lines (mika#1254)
//!
//! Context: An agent with a verdict-producer skill (mika-arch-second-review)
//! emits `Verdict: GROOMED` as its legitimate output. The dev-groom fabrication
//! guard should NOT fire — the agent IS the verdict producer, not a dispatcher
//! fabricating a verdict.
//!
//! This confirms the `is_verdict_producer` exemption path works correctly and
//! prevents regression where the inverted predicate accidentally catches
//! producer agents.
//!
//! ## Hard Assertions
//! - Guard does NOT fire: LLM call count is exactly 1 (no re-prompt).
//! - Agent output CONTAINS `Verdict: GROOMED` (preserved, not stripped).
//!
//! ## Tags
//! - `grounding:verdict-producer-exempt` — producer exemption path
//!
//! Reference: mika#1254, mika#811 (mika-arch skill suite)

use std::collections::HashMap;
use std::path::PathBuf;

use mika_agent::skills::SkillRegistry;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{Constraints, Output, SkillInfo, SkillManifest, Triggers};

use super::*;

/// Create a mika-arch-second-review skill entry. This is a known verdict
/// producer — its LLM output legitimately contains `Verdict:` lines.
fn make_second_review_skill() -> SkillEntry {
    SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: "mika-arch-second-review".to_string(),
                description: "Second-pass plan review — produces GROOMED/ESCALATE".to_string(),
                version: "0.1.0".to_string(),
                always_on: false,
                timeout_secs: 300,
                dependencies: vec![],
                max_prompt_size: None,
            },
            triggers: Triggers {
                keywords: vec!["second review".to_string(), "second pass".to_string()],
            },
            llm: Default::default(),
            constraints: Constraints {
                required_tools: vec![],
                required_fetches_for_quoted_resources: false,
            },
            output: Output {
                required_suffix_lines: vec![
                    "Verdict: GROOMED".to_string(),
                    "Verdict: ESCALATE".to_string(),
                ],
                required_finding_list_prefixes: vec![],
                required_tool_arg_suffixes: vec![],
            },
            context: HashMap::new(),
            variants: Default::default(),
        },
        dir: PathBuf::from("/skills/mika-arch-second-review"),
        keywords_lower: vec!["second review".to_string(), "second pass".to_string()],
        prompt_snippet: String::new(),
        skill_tools: vec![],
        enabled: true,
        has_override: false,
        provider_overrides: HashMap::new(),
        prompt_sources: SkillEntry::empty_prompt_sources(),
        model_overrides: HashMap::new(),
    }
}

/// Producer agent emits `Verdict: GROOMED` — guard is exempted.
#[tokio::test]
async fn test_verdict_producer_exempt_groomed() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_second_review_skill()]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Single turn: producer emits legitimate Verdict.
            // The dev-groom fabrication guard should NOT fire because
            // is_verdict_producer is true.
            text_response(
                "Plan review complete. No structural findings.\n\n\
                 Verdict: GROOMED",
            ),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run("second review of mika#1254 plan").await?;

    // Hard: guard did NOT fire — exactly 1 LLM call
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected no guard-retry for verdict-producer agent, got {} LLM calls. \
         The is_verdict_producer exemption is not working.",
        trace.llm_call_count
    );

    // Hard: verdict is preserved in output (not stripped by guard)
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "Verdict: GROOMED");

    Ok(())
}

/// Producer agent emits `Verdict: ESCALATE` — guard is exempted.
#[tokio::test]
async fn test_verdict_producer_exempt_escalate() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_second_review_skill()]);

    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "Plan review complete. Found structural issues.\n\n\
                 F1: Missing error handling in the retry path.\n\n\
                 Verdict: ESCALATE",
        )])
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run("second review of mika#1254 plan").await?;

    assert_eq!(
        trace.llm_call_count, 1,
        "Expected no guard-retry for verdict-producer agent on ESCALATE, got {}",
        trace.llm_call_count
    );

    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "Verdict: ESCALATE");

    Ok(())
}

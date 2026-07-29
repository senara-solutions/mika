//! Eval test: `mika-arch-groom-ticket` Fire-Disposition Gate output contract (mika#1574)
//!
//! Validates that the agent loop correctly threads the architect's declared
//! disposition/F-list shape through the post-condition guard chain for each of
//! the three Fire-Disposition Gate branches:
//!   - detector deliverable + `## Fire-Disposition` present  → `Disposition: READY`
//!   - detector deliverable + section missing                → `Disposition: ITERATE` + F-list
//!   - non-detector deliverable + no section (gate N/A)       → `Disposition: READY`
//!
//! ## What this test does NOT do
//!
//! Per the `skills/mod.rs` doc comment, per-skill eval scenarios validate engine
//! *handling* of a skill's declared output shape, not the production prompt's
//! wording. A `MockLlmProvider` returns canned responses — it cannot prove the
//! architect's *LLM judgment* applies the Fire-Disposition Gate (the mock emits
//! whatever disposition the scenario programs). These scenarios assert the engine
//! correctly threads the architect's declared disposition/F-list shape through
//! the loop for each gate branch (regression coverage for the output contract,
//! the same class as `mika_arch_groom_milestone`).
//!
//! The **prompt-adherence** guarantee — that the live architect model actually
//! returns ITERATE on a detector plan missing fire-disposition — is owned by the
//! mika-arch calibration suite (mika#1190). Because the Fire-Disposition Gate
//! modifies `mika-arch-*` system prompts, `make calibrate-mika-arch
//! MODEL=<current-baseline-model>` must hold the baseline pass-rate before merge.
//!
//! ## Reference
//!
//! - Issue: senara-solutions/mika#1574
//! - Plan: `docs/plans/2026-06-28-005-feat-dev-groom-doctrine-add-fire-disposition-plan.md` § Units 5-7
//! - Pattern reference: `crates/mika-agent/tests/eval/skills/mika_arch_groom_milestone.rs`

use std::collections::HashMap;
use std::path::PathBuf;

use mika_agent::skills::SkillRegistry;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{Output, SkillInfo, SkillManifest, Triggers};
use mika_common::llm::mock::*;

use super::super::assertions::*;
use super::super::harness::EvalHarness;

/// Build a synthetic `mika-arch-groom-ticket` skill entry.
///
/// Mirrors the production `skill.toml`'s suffix-line accept-set and
/// finding-list-prefix contract so the required-suffix-line guard (#864) and
/// required-finding-list guard (#901) treat the synthetic skill exactly as the
/// production one. Constraints (`required_tools = ["gh_read"]`,
/// `required_fetches_for_quoted_resources`) are intentionally left at default —
/// those are orthogonal guards covered by other tests and would add
/// response-sequencing complexity unrelated to the Fire-Disposition-Gate
/// output-shape contract under test here (same choice as `make_milestone_skill`).
fn make_groom_ticket_skill() -> SkillEntry {
    SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: "mika-arch-groom-ticket".to_string(),
                description: "Test fixture mirroring the production first-pass grooming skill."
                    .to_string(),
                version: "0.1.0".to_string(),
                always_on: false,
                timeout_secs: 120,
                dependencies: vec![],
                max_prompt_size: None,
            },
            triggers: Triggers {
                keywords: vec![
                    "groom-ticket".to_string(),
                    "review-plan".to_string(),
                    "architect-review".to_string(),
                ],
            },
            llm: Default::default(),
            constraints: Default::default(),
            output: Output {
                required_suffix_lines: vec![
                    "Disposition: READY".to_string(),
                    "Disposition: ITERATE".to_string(),
                    "Disposition: ESCALATE".to_string(),
                ],
                required_finding_list_prefixes: vec![
                    "F1:".to_string(),
                    "F2:".to_string(),
                    "F3:".to_string(),
                    "F4:".to_string(),
                    "F5:".to_string(),
                    "F6:".to_string(),
                    "F7:".to_string(),
                    "F8:".to_string(),
                    "F9:".to_string(),
                    "F10:".to_string(),
                ],
                required_tool_arg_suffixes: vec![],
            },
            context: HashMap::new(),
            variants: Default::default(),
        },
        dir: PathBuf::from("/skills/mika-arch-groom-ticket"),
        // Derive `keywords_lower` from the trigger keywords rather than
        // re-listing the strings — mirrors the canonical pattern in
        // `mika_arch_groom_milestone.rs`.
        keywords_lower: ["groom-ticket", "review-plan", "architect-review"]
            .iter()
            .map(|s| s.to_lowercase())
            .collect(),
        prompt_snippet: String::new(),
        skill_tools: vec![],
        enabled: true,
        has_override: false,
        provider_overrides: HashMap::new(),
        prompt_sources: SkillEntry::empty_prompt_sources(),
        model_overrides: HashMap::new(),
    }
}

/// Returns the last non-empty line of `text` after trimming.
///
/// Strictest subset of the required-suffix-line guard: the guard accepts a match
/// in any of the last 3 non-empty lines, while this helper only inspects the
/// literal final line. Enforces the literal final-line discipline (`Disposition:
/// <KEYWORD>` must be the literal final line). Mirrors the helper in
/// `mika_arch_groom_milestone.rs`.
fn last_nonempty_line(text: &str) -> &str {
    text.lines()
        .map(|l| l.trim())
        .rfind(|l| !l.is_empty())
        .unwrap_or("")
}

/// Unit 5 — Detector deliverable + `## Fire-Disposition` section present ⇒ READY.
///
/// The gate passes (it is not triggered by this gate because the section is
/// present); the engine threads `Disposition: READY` through the guards unchanged.
#[tokio::test]
async fn test_detector_with_fire_disposition_emits_ready() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_groom_ticket_skill()]);

    let mock_response = "\
Annotated plan content:
  Unit 1 adds a `verify-bundled-skills` structural check — a detector-class
  deliverable (its success path is \"no violations found\").
  The plan carries a `## Fire-Disposition` section naming option (a): named
  allowlist exception with a self-cleaning assertion + follow-up tracker for
  each pre-existing bundled-skill collision. Sufficient implementation detail.

Fire-Disposition Gate: detector present + non-empty `## Fire-Disposition`
section naming option (a) ⇒ gate passes.

Plan is sound. No remaining concerns.

Disposition: READY";

    let harness = EvalHarness::builder()
        .responses(vec![text_response(mock_response)])
        .skills(skills)
        .build()
        .await?;

    let user_msg = "architect-review: Review the plan for mika#1574 that adds a \
                    `verify-bundled-skills` structural check. The plan includes a \
                    `## Fire-Disposition` section naming option (a).";
    let trace = harness.run(user_msg).await?;

    assert_has_output(&trace);
    let output = trace.output.text.as_deref().unwrap_or("");

    // Literal final-line discipline — `Disposition: READY`.
    assert_eq!(
        last_nonempty_line(output),
        "Disposition: READY",
        "Expected `Disposition: READY` as literal final non-empty line, got output:\n{}",
        output
    );

    // No guard re-prompt — a valid `Disposition: READY` final line is accepted,
    // and READY does not require an F-list.
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected single LLM call (no guard re-prompt) for detector+section READY case, got {}",
        trace.llm_call_count
    );

    Ok(())
}

/// Unit 6 — Detector deliverable + missing `## Fire-Disposition` section ⇒ ITERATE.
///
/// The mock returns `Disposition: ITERATE` with a BLOCKING F-finding citing
/// mika#1574. Because ITERATE is a terminal disposition, the required-finding-list
/// guard (#901) requires the F-list — the mock includes `F1:`, so the turn is
/// accepted in a single LLM call.
#[tokio::test]
async fn test_detector_without_fire_disposition_emits_iterate_with_finding() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_groom_ticket_skill()]);

    let mock_response = "\
Annotated plan content:
  Unit 2 adds a lint rule rejecting bare `&str` byte-slicing — a detector-class
  deliverable. The plan has no `## Fire-Disposition` section, so it does not name
  what the implementation does when the lint fires on pre-existing violations.

F1: (BLOCKING) Detector-class deliverable without a `## Fire-Disposition` section.
   Concern: Unit 2 adds a lint rule (detector-class) but the plan omits the
   `## Fire-Disposition` section, leaving the fire-on-existing-data behavior
   undirected — the pilot would either break CI or make an undirected scope call.
   Change required: Add a `## Fire-Disposition` section naming one of the three
   canonical options (named allowlist / land disabled / halt-and-surface) with
   sufficient implementation detail.
   Citation: docs/solutions/best-practices/fire-disposition-doctrine.md + mika#1574

Disposition: ITERATE";

    let harness = EvalHarness::builder()
        .responses(vec![text_response(mock_response)])
        .skills(skills)
        .build()
        .await?;

    let user_msg = "review-plan: Review the plan for mika#1574 that adds a lint rule \
                    rejecting bare `&str` byte-slicing. The plan has no fire-disposition section.";
    let trace = harness.run(user_msg).await?;

    assert_has_output(&trace);
    let output = trace.output.text.as_deref().unwrap_or("");

    // Aggregate disposition is ITERATE on the literal final line.
    assert_eq!(
        last_nonempty_line(output),
        "Disposition: ITERATE",
        "Expected `Disposition: ITERATE` as literal final line for detector+no-section case, got:\n{}",
        output
    );

    // BLOCKING F-finding present and cites the doctrine / ticket.
    assert!(
        output.contains("F1:"),
        "Expected a BLOCKING F1 finding in the ITERATE response, got:\n{}",
        output
    );
    assert!(
        output.contains("1574"),
        "Expected the F-finding to cite mika#1574, got:\n{}",
        output
    );

    // No guard re-prompt — `Disposition: ITERATE` is in the suffix-line accept
    // set AND the required F-list is present, so the terminal-disposition
    // finding-list guard (#901) does not reject.
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected single LLM call (no guard re-prompt) for ITERATE+F-list case, got {}",
        trace.llm_call_count
    );

    Ok(())
}

/// Unit 7 — Non-detector deliverable + no `## Fire-Disposition` section ⇒ READY.
///
/// Purely additive deliverables (new HTTP endpoint, module refactor) are not
/// detector-class; the gate is N/A and does not influence the disposition.
#[tokio::test]
async fn test_non_detector_without_fire_disposition_emits_ready() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_groom_ticket_skill()]);

    let mock_response = "\
Annotated plan content:
  The plan adds a new `/api/v1/health` HTTP endpoint and refactors the routing
  module. Neither deliverable is detector-class — no test, assertion, lint,
  invariant, or validation whose success path is \"no violations found.\"

Fire-Disposition Gate: no detector-class deliverables ⇒ gate is N/A (does not
influence disposition). The absence of a `## Fire-Disposition` section is fine.

Plan is sound. No remaining concerns.

Disposition: READY";

    let harness = EvalHarness::builder()
        .responses(vec![text_response(mock_response)])
        .skills(skills)
        .build()
        .await?;

    let user_msg = "groom-ticket: Review the plan that adds a new `/api/v1/health` HTTP \
                    endpoint and refactors the routing module. No fire-disposition section.";
    let trace = harness.run(user_msg).await?;

    assert_has_output(&trace);
    let output = trace.output.text.as_deref().unwrap_or("");

    // Gate N/A — non-detector plan reaches READY despite no `## Fire-Disposition`.
    assert_eq!(
        last_nonempty_line(output),
        "Disposition: READY",
        "Expected `Disposition: READY` for non-detector plan (gate N/A), got:\n{}",
        output
    );

    // No guard re-prompt — READY final line accepted, no F-list required.
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected single LLM call (no guard re-prompt) for non-detector READY case, got {}",
        trace.llm_call_count
    );

    Ok(())
}

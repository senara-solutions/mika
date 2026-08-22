//! Eval test: `mika-arch-groom-milestone` skill output contract (mika#879 Unit 1)
//!
//! Validates that the agent loop correctly handles milestone-shaped I/O for the
//! `mika-arch-groom-milestone` skill: the milestone-trigger keyword routes the
//! brief to the matched skill, and a milestone-shaped LLM response (containing
//! `Scope: milestone` plus `Disposition: <KEYWORD>` as the literal final line)
//! flows through the post-condition guards unchanged.
//!
//! ## What this test does NOT do
//!
//! It does not validate the production prompt's wording or that a real LLM
//! produces this output. The synthetic skill declares the same
//! `required_suffix_lines` accept-set as the production `skill.toml` so the
//! required-suffix-line guard (#864) treats both consistently — but the
//! test mocks the LLM response. Real-prompt behavior belongs to the gated
//! real-provider eval matrix (see `crates/mika-agent/CLAUDE.md` § Evaluation).
//!
//! ## Reference
//!
//! - Issue: senara-solutions/mika#879
//! - Plan: `docs/plans/2026-04-29-001-feat-mika-arch-milestone-grooming-plan.md` § Unit 1
//! - Delta plan: `docs/plans/2026-04-29-004-test-mika-arch-groom-milestone-eval-plan.md`
//! - Pattern reference: `crates/mika-agent/tests/eval/grounding_regressions/required_suffix_line_caught.rs`

use std::collections::HashMap;
use std::path::PathBuf;

use mika_agent::skills::SkillRegistry;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{Output, SkillInfo, SkillManifest, Triggers};
use mika_common::llm::mock::*;

use super::super::assertions::*;
use super::super::harness::EvalHarness;

/// Build a synthetic `mika-arch-groom-milestone` skill entry.
///
/// Mirrors the production `skill.toml`'s suffix-line accept-set (TD2 in the
/// delta plan). Constraints (required_tools, required_fetches_for_quoted_resources)
/// are intentionally left at default — those are orthogonal guards covered by
/// other tests and would add response-sequencing complexity unrelated to Unit 1's
/// output-shape contract.
fn make_milestone_skill() -> SkillEntry {
    SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: "mika-arch-groom-milestone".to_string(),
                description: "Test fixture mirroring the production milestone-grooming skill."
                    .to_string(),
                version: "0.1.0".to_string(),
                always_on: false,
                timeout_secs: 30,
                dependencies: vec![],
                max_prompt_size: None,
                data_grade: Default::default(),
            },
            triggers: Triggers {
                keywords: vec![
                    "groom-milestone".to_string(),
                    "milestone-review".to_string(),
                    "milestone-groom".to_string(),
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
                required_finding_list_prefixes: vec![],
                required_tool_arg_suffixes: vec![],
            },
            context: HashMap::new(),
            variants: Default::default(),
        },
        dir: PathBuf::from("/skills/mika-arch-groom-milestone"),
        // Derive `keywords_lower` from `triggers.keywords` rather than
        // re-listing the strings — guards against future copy-trap where a
        // mixed-case keyword would silently end up in `keywords_lower` without
        // normalization. Mirrors the canonical pattern in
        // `tests/eval/grounding_regressions/required_suffix_line_caught.rs`.
        keywords_lower: ["groom-milestone", "milestone-review", "milestone-groom"]
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
/// This is the **strictest subset** of the engine's required-suffix-line guard
/// (`crates/mika-agent/CLAUDE.md` § Post-Conditions guard #8): the guard accepts
/// a match in any of the last 3 non-empty lines, while this helper only inspects
/// the literal final line. The narrower check enforces the *literal final-line
/// discipline* the parent plan inherits from
/// `docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` —
/// `Disposition: <KEYWORD>` MUST be the literal final line, not merely within
/// the guard's window. Do not use this helper as a stand-in for the guard's
/// scan logic; it is a stricter contract intentionally.
fn last_nonempty_line(text: &str) -> &str {
    text.lines()
        .map(|l| l.trim())
        .rfind(|l| !l.is_empty())
        .unwrap_or("")
}

/// Scenario 1 — Happy path: 3-sub-issue brief produces milestone-shaped output
/// with `Scope: milestone` block and `Disposition: READY` as the literal final line.
#[tokio::test]
async fn test_three_sub_issue_brief_emits_ready_with_scope_milestone() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_milestone_skill()]);

    let mock_response = "\
Per-sub-issue disposition summary:
#874: KG schema migration — READY: scoped to v27 backup tables, no engine churn
#875: KG corpus reachability — READY: dependencies on #874 declared
#876: KG resolution backlog — READY: blocked by #874 + #875 per declared edges

Sequencing:
  #875 blockedBy #874 — schema must land before lexical reingest
  #876 blockedBy #874 — schema dependency
  #876 blockedBy #875 — resolution consumes lexical entities
  Recommended order: #874 → #875 → #876

Cross-cutting concerns:
  None surfaced — coupling enumerated in declared blockedBy edges.

Annotated plan content:
  See sub-issue plans for inline findings.

Scope: milestone
Disposition: READY";

    let harness = EvalHarness::builder()
        .responses(vec![text_response(mock_response)])
        .skills(skills)
        .build()
        .await?;

    let user_msg = "milestone-groom: Review milestone#19 with sub-issues #874, #875, #876.\n\
                    Each sub-issue has a plan-on-branch listed in the brief.";
    let trace = harness.run(user_msg).await?;

    assert_has_output(&trace);
    let output = trace.output.text.as_deref().unwrap_or("");

    // R2.1: literal final-line discipline — `Disposition: READY`
    assert_eq!(
        last_nonempty_line(output),
        "Disposition: READY",
        "Expected `Disposition: READY` as literal final non-empty line, got output:\n{}",
        output
    );

    // R2.1: milestone scope marker present
    assert!(
        output.contains("Scope: milestone"),
        "Expected output to contain `Scope: milestone`, got:\n{}",
        output
    );

    // No guard re-prompt — the suffix-line guard accepts a `Disposition: READY` final line.
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected single LLM call (no guard re-prompt), got {}",
        trace.llm_call_count
    );

    Ok(())
}

/// Scenario 2 — Single-sub-issue edge case: n=1 brief still emits milestone-shaped
/// output (per-sub-issue summary section + `Scope: milestone`), not per-ticket output.
#[tokio::test]
async fn test_single_sub_issue_brief_still_emits_milestone_shape() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_milestone_skill()]);

    let mock_response = "\
Per-sub-issue disposition summary:
#900: Standalone fix — READY: no cross-cutting touch, scope minimal

Sequencing:
  No edges (single sub-issue milestone).

Cross-cutting concerns:
  None — single sub-issue carries no coupling surface.

Annotated plan content:
  See sub-issue #900 plan for inline findings.

Scope: milestone
Disposition: READY";

    let harness = EvalHarness::builder()
        .responses(vec![text_response(mock_response)])
        .skills(skills)
        .build()
        .await?;

    let user_msg = "milestone-groom: Review milestone#22 with single sub-issue #900.";
    let trace = harness.run(user_msg).await?;

    assert_has_output(&trace);
    let output = trace.output.text.as_deref().unwrap_or("");

    // R2.2: emits milestone-shape (Scope: milestone present), not per-ticket shape.
    assert!(
        output.contains("Scope: milestone"),
        "Expected milestone-shape output (`Scope: milestone` present) even for n=1, got:\n{}",
        output
    );

    // R2.2: structural milestone-shape distinguishers — these section headers
    // appear in milestone-shaped output but not in per-ticket-shaped output.
    // Without this assertion, scenario 2 only checks tokens (`Scope: milestone`,
    // `#900`) that any shaped response could include verbatim. The section
    // headers are the load-bearing structural distinguisher.
    assert!(
        output.contains("Per-sub-issue disposition summary:"),
        "Expected milestone-shape header `Per-sub-issue disposition summary:` for n=1 case, got:\n{}",
        output
    );
    assert!(
        output.contains("Sequencing:"),
        "Expected milestone-shape header `Sequencing:` for n=1 case, got:\n{}",
        output
    );

    // R2.2: per-sub-issue summary section references the n=1 entry.
    assert!(
        output.contains("#900"),
        "Expected per-sub-issue line referencing #900 in summary, got:\n{}",
        output
    );

    // Final-line discipline holds.
    assert_eq!(
        last_nonempty_line(output),
        "Disposition: READY",
        "Expected `Disposition: READY` as literal final line for single-sub-issue READY case"
    );

    // No guard re-prompt — a regression where the suffix-line guard incorrectly
    // fires on a valid `Disposition: READY` final line for the n=1 case would
    // otherwise pass silently because the text assertions match the canned mock.
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected single LLM call (no guard re-prompt) for n=1 READY case, got {}",
        trace.llm_call_count
    );

    Ok(())
}

/// Scenario 3 — Conflicting-AC edge case: cross-cutting incompatibility between
/// sub-issues surfaces in the cross-cutting section and aggregates to ITERATE.
#[tokio::test]
async fn test_conflicting_ac_cross_cutting_emits_iterate() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_milestone_skill()]);

    let mock_response = "\
Per-sub-issue disposition summary:
#910: Add field A to schema — ITERATE: shape conflicts with sibling
#911: Drop field A from schema — ITERATE: shape conflicts with sibling

Sequencing:
  Cycle risk — both sub-issues touch the same schema column with opposite intents.

Cross-cutting concerns:
  Shared touch-point: `kg_entities.entity_key` column. #910 adds a NOT NULL
  constraint while #911 removes the column entirely. Conflicting acceptance
  criteria — operator must reconcile the milestone scope before re-grooming.

Annotated plan content:
  See per-sub-issue findings; both flagged for revision.

Scope: milestone
Disposition: ITERATE";

    let harness = EvalHarness::builder()
        .responses(vec![text_response(mock_response)])
        .skills(skills)
        .build()
        .await?;

    let user_msg = "milestone-groom: Review milestone#23 with conflicting sub-issues #910, #911.";
    let trace = harness.run(user_msg).await?;

    assert_has_output(&trace);
    let output = trace.output.text.as_deref().unwrap_or("");

    // R2.3: aggregate disposition is ITERATE on literal final line.
    assert_eq!(
        last_nonempty_line(output),
        "Disposition: ITERATE",
        "Expected `Disposition: ITERATE` as literal final line for conflicting-AC case, got:\n{}",
        output
    );

    // R2.3: cross-cutting concern surfaced — loose match on either keyword.
    let lower = output.to_lowercase();
    assert!(
        lower.contains("cross-cutting") || lower.contains("conflicting"),
        "Expected cross-cutting / conflicting language in output, got:\n{}",
        output
    );

    // No guard re-prompt — `Disposition: ITERATE` is in the suffix-line accept
    // set; a regression where the guard incorrectly rejects ITERATE would
    // silently pass the text assertions otherwise.
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected single LLM call (no guard re-prompt) for ITERATE case, got {}",
        trace.llm_call_count
    );

    Ok(())
}

/// Scenario 4 — Missing-sections error path: malformed brief without required
/// sections yields ESCALATE with a citation to the missing schema element.
#[tokio::test]
async fn test_missing_sections_emits_escalate() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_milestone_skill()]);

    let mock_response = "\
Per-sub-issue disposition summary:
  Cannot evaluate — brief is missing required `Sub-issues` section with plan
  paths. Without per-sub-issue plan paths, this skill has no input to review.

Sequencing:
  Cannot determine — missing input.

Cross-cutting concerns:
  Cannot determine — missing input.

Annotated plan content:
  Schema reference: parent plan §D3 declares `Sub-issues`, `Dependencies`,
  `Recommended GitHub blockedBy edits`, `Order`, `Cross-cutting concerns`,
  and `Open milestone-level questions` as required H2 sections. The supplied
  brief omits the per-sub-issue plan paths — escalating to operator for input.

Scope: milestone
Disposition: ESCALATE";

    let harness = EvalHarness::builder()
        .responses(vec![text_response(mock_response)])
        .skills(skills)
        .build()
        .await?;

    let user_msg =
        "milestone-groom: Review milestone#24 (brief intentionally malformed — no plan paths).";
    let trace = harness.run(user_msg).await?;

    assert_has_output(&trace);
    let output = trace.output.text.as_deref().unwrap_or("");

    // R2.4: aggregate disposition is ESCALATE on literal final line.
    assert_eq!(
        last_nonempty_line(output),
        "Disposition: ESCALATE",
        "Expected `Disposition: ESCALATE` as literal final line for malformed-brief case, got:\n{}",
        output
    );

    // R2.4: response references a missing element (loose match on schema-related vocabulary).
    let lower = output.to_lowercase();
    assert!(
        lower.contains("missing") || lower.contains("plan path") || lower.contains("sub-issues"),
        "Expected missing-section language referencing schema gap, got:\n{}",
        output
    );

    // No guard re-prompt — `Disposition: ESCALATE` is in the suffix-line accept
    // set; a regression where the guard incorrectly rejects ESCALATE would
    // silently pass the text assertions otherwise.
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected single LLM call (no guard re-prompt) for ESCALATE case, got {}",
        trace.llm_call_count
    );

    Ok(())
}

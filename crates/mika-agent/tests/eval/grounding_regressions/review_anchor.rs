//! Scenario: Review-anchor guard — attestation required on non-terminal dispositions (mika#2037)
//!
//! Context: mika#901's F-list guard fires on TERMINAL dispositions (ITERATE/ESCALATE) and
//! exempts the rest. That left `Disposition: READY` and `Verdict: GROOMED` — the two
//! dispositions that advance the grooming chain — as the only ones owing no evidence.
//! On 2026-08-29 mika-arch returned a 302-byte acknowledgement carrying `Disposition: READY`
//! on a 10 492-byte brief with four numbered questions, none of them addressed. The keyword
//! forged the attestation; `/mika-groom-ticket` Phase 3 step 10 committed the plan as
//! architect-validated.
//!
//! The guard requires anchor lines quoting the brief verbatim at distinct positions, and is
//! fail-CLOSED: when the corrective re-prompt does not produce them, the disposition is
//! withheld from the final text rather than accepted.
//!
//! ## Hard Assertions
//! - Guard fires on READY without attestation, and on `Verdict: GROOMED`.
//! - Guard does NOT fire on a terminal disposition (that half belongs to mika#901).
//! - Guard does NOT fire when the skill does not declare the contract.
//! - A real anchored review passes on the first turn, with no re-prompt.
//! - After the re-prompt fails, the disposition is gone and the marker is present.
//! - The anchor guard's retry budget is independent of the F-list guard's.
//!
//! ## Tags
//! - `grounding:review-anchor-required` — post-fix success tag
//! - `grounding:unanchored-ready` — pre-fix failure tag
//!
//! Reference: mika#2037, mika#901 (the exempted half), mika#1957 (n=2 of the same class)

use std::collections::HashMap;
use std::path::PathBuf;

use mika_agent::skills::SkillRegistry;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{Output, SkillInfo, SkillManifest, Triggers};
use mika_common::llm::mock::*;

use super::*;

const GROOM_SUFFIX_LINES: &[&str] = &[
    "Disposition: READY",
    "Disposition: ITERATE",
    "Disposition: ESCALATE",
];
const SECOND_REVIEW_SUFFIX_LINES: &[&str] = &["Verdict: GROOMED", "Verdict: ESCALATE"];
const ANCHOR_PREFIXES: &[&str] = &["A1:", "A2:", "A3:", "A4:", "A5:"];
const FINDING_PREFIXES: &[&str] = &["F1:", "F2:", "F3:"];

/// The withheld-disposition marker. Kept as a literal here on purpose: if the engine's
/// constant is renamed, this test fails rather than silently passing against a marker
/// `dispatch-lib`'s tier 0 no longer recognizes.
const WITHHELD_MARKER: &str = "Disposition-Withheld: REVIEW-ANCHOR-MISSING";

/// A brief above the arming threshold, standing in for the 10 492-byte one of mika#2037.
/// Anchor lines in the tests below quote from it verbatim.
///
/// It must exceed `review_anchor_min_brief_chars` (2000) or the guard correctly declines to
/// arm — see `test_review_anchor_no_op_on_short_ad_hoc_question` for the other side of that
/// boundary.
const BRIEF: &str = "groom-ticket: Review the plan for the milestone-manager token renewal.\n\
     \n\
     ## Summary\n\
     The plan re-resolves the manager cycle token before every cycle instead of freezing it \
     at spawn time. A GitHub App installation token has roughly a one-hour lifetime, so the \
     manager cycles 401 until the process restarts.\n\
     \n\
     ## Design\n\
     The cycle token is re-resolved through a TokenResolver trait, never frozen at spawn. \
     SettingsTokenResolver is the production implementation: PAT first per ADR-008, GitHub \
     App installation token as the fallback. A change in the resolved value emits \
     manager_token_refreshed at INFO, carrying presence booleans only, never token material. \
     AuthFailureTracker counts the duration of an unbroken 401 run rather than a count of \
     cycles, because poll_interval is operator-configurable and N cycles has no stable \
     temporal meaning. Past a thirty-minute threshold the tracker escalates and re-announces \
     at most hourly.\n\
     \n\
     ## Error handling\n\
     A failed re-resolution keeps the previous token rather than overwriting it with None: \
     reader.rs only sets GH_TOKEN when the value is Some, so overwriting would silently drop \
     the cycle onto the host ambient credentials. The refresh is bounded by a fifteen-second \
     timeout because it sits outside the select! on the cancellation token, and GitHubApp \
     holds its cache write-lock across an un-timed HTTP call. Any successful cycle clears the \
     failure window; non-401 failures neither advance nor clear it, since a network blip is \
     not proof of recovery, nor of auth failure.\n\
     \n\
     ## Test plan\n\
     Unit coverage for a resolver returning a changed value, an unchanged value, and an \
     error; the tracker advancing, clearing, and re-announcing on schedule. Integration \
     coverage for a cycle running against a resolver whose token rotates mid-run.\n\
     \n\
     ## Questions\n\
     Question 1: where does the correction belong, in the spawn path or in the cycle body?\n\
     Question 2: is the trace exhaustive, should every refresh be journaled at INFO level?\n\
     Question 3: I have not fixed N for the repeated authentication failure threshold.\n\
     Question 4: is putting the 403 response class out of scope safe for this milestone?\n";

/// Three anchor lines, each quoting a different region of the brief verbatim.
const ANCHORED_REVIEW: &str = "A1: \"re-resolves the manager cycle token before every cycle instead of freezing it\" — correct placement, the cycle body owns it.\n\
     A2: \"is the trace exhaustive, should every refresh be journaled at INFO level\" — yes, one INFO per resolved change is enough.\n\
     A3: \"I have not fixed N for the repeated authentication failure threshold\" — express it as a duration, not a cycle count.\n\
     \n\
     Disposition: READY";

/// The measured mika#2037 response shape: an acknowledgement plus the keyword.
const UNANCHORED_STUB: &str = "Preference stored — the per-cycle re-resolution pattern and the N=3 threshold.\n\n\
     Disposition: READY";

fn make_anchor_skill(
    name: &str,
    keywords: &[&str],
    suffix_lines: &[&str],
    anchor_prefixes: &[&str],
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
                data_grade: Default::default(),
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
                required_review_anchor_prefixes: anchor_prefixes
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                review_anchor_min_count: 3,
                review_anchor_min_quote_chars: 40,
                review_anchor_min_brief_chars: 2000,
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

fn groom_skill() -> SkillRegistry {
    SkillRegistry::from_test_entries(vec![make_anchor_skill(
        "arch-groom-ticket",
        &["groom-ticket"],
        GROOM_SUFFIX_LINES,
        ANCHOR_PREFIXES,
        FINDING_PREFIXES,
    )])
}

/// Test 1: the founding incident — READY with no attestation triggers the corrective
/// re-prompt, and the anchored second turn is accepted.
#[tokio::test]
async fn test_review_anchor_caught_on_unanchored_ready() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            text_response(UNANCHORED_STUB),
            text_response(ANCHORED_REVIEW),
        ])
        .skills(groom_skill())
        .build()
        .await?;

    let trace = harness.run(BRIEF).await?;

    assert!(
        trace.llm_call_count > 1,
        "Expected the review-anchor guard to fire and re-prompt, got {} LLM call(s)",
        trace.llm_call_count
    );
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "A1:");
    grounding_assertions::assert_response_contains(&trace, "Disposition: READY");

    Ok(())
}

/// Test 2: fail-closed. When the corrective re-prompt does not produce an attestation, the
/// disposition does not survive into the final response — the marker replaces it, and the
/// model's own prose is kept.
#[tokio::test]
async fn test_review_anchor_withholds_disposition_after_failed_retry() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            text_response(UNANCHORED_STUB),
            // Second attempt: still no anchors. This is where every sibling guard would
            // give up and accept.
            text_response(
                "Still nothing to add. The plan is fine as written.\n\nDisposition: READY",
            ),
        ])
        .skills(groom_skill())
        .build()
        .await?;

    let trace = harness.run(BRIEF).await?;

    assert_has_output(&trace);
    grounding_assertions::assert_response_forbids(&trace, &["Disposition: READY"]);
    grounding_assertions::assert_response_contains(&trace, WITHHELD_MARKER);
    // The body survives — the guard removes the unearned attestation, not the content.
    grounding_assertions::assert_response_contains(&trace, "Still nothing to add");

    Ok(())
}

/// Test 3: a genuine anchored review passes on the first turn. This is the anti-vacuity
/// direction that "reject everything" would fail — without it the guard would block all
/// grooming.
#[tokio::test]
async fn test_review_anchor_no_op_on_anchored_ready() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(ANCHORED_REVIEW)])
        .skills(groom_skill())
        .build()
        .await?;

    let trace = harness.run(BRIEF).await?;

    assert_eq!(
        trace.llm_call_count, 1,
        "A real anchored review must pass with no re-prompt, got {} LLM call(s)",
        trace.llm_call_count
    );
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "Disposition: READY");
    grounding_assertions::assert_response_forbids(&trace, &[WITHHELD_MARKER]);

    Ok(())
}

/// Test 4: the terminal half stays mika#901's. An ITERATE with an F-list and no anchors must
/// not be asked for anchors — the two guards partition the disposition space, they do not
/// overlap.
#[tokio::test]
async fn test_review_anchor_no_op_on_terminal_disposition() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "F1: (BLOCKING) The threshold is expressed in cycles, not duration.\n\
                Concern: poll_interval is operator-configurable.\n\
                Change required: express the threshold as a duration.\n\
                Citation: review-guide.md\n\n\
             Disposition: ITERATE",
        )])
        .skills(groom_skill())
        .build()
        .await?;

    let trace = harness.run(BRIEF).await?;

    assert_eq!(
        trace.llm_call_count, 1,
        "Terminal dispositions belong to the mika#901 guard, got {} LLM call(s)",
        trace.llm_call_count
    );
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "Disposition: ITERATE");

    Ok(())
}

/// Test 5: `Verdict: GROOMED` is the second non-terminal disposition and is covered too.
/// The second-review pass commits the plan just as the first does.
#[tokio::test]
async fn test_review_anchor_caught_on_unanchored_groomed() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_anchor_skill(
        "arch-second-review",
        &["second-review"],
        SECOND_REVIEW_SUFFIX_LINES,
        ANCHOR_PREFIXES,
        FINDING_PREFIXES,
    )]);

    let brief = format!("second-review: {BRIEF}");
    let anchored = ANCHORED_REVIEW.replace("Disposition: READY", "Verdict: GROOMED");

    let harness = EvalHarness::builder()
        .responses(vec![
            text_response("All prior findings resolved.\n\nVerdict: GROOMED"),
            text_response(&anchored),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run(&brief).await?;

    assert!(
        trace.llm_call_count > 1,
        "Expected the guard to fire on an unanchored GROOMED, got {} LLM call(s)",
        trace.llm_call_count
    );
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "A1:");

    Ok(())
}

/// Test 6: opt-in. A skill that declares no anchor contract is untouched, whatever it emits.
#[tokio::test]
async fn test_review_anchor_no_op_when_undeclared() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_anchor_skill(
        "arch-groom-ticket",
        &["groom-ticket"],
        GROOM_SUFFIX_LINES,
        &[], // no anchor contract declared
        FINDING_PREFIXES,
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![text_response(UNANCHORED_STUB)])
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run(BRIEF).await?;

    assert_eq!(
        trace.llm_call_count, 1,
        "An undeclared contract must never fire the guard, got {} LLM call(s)",
        trace.llm_call_count
    );
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "Disposition: READY");

    Ok(())
}

/// Test 7: retry budgets are independent. An F-list re-prompt must not consume the anchor
/// guard's single corrective attempt — otherwise a turn that trips mika#901 first would then
/// get its unanchored READY accepted on the very next response.
#[tokio::test]
async fn test_review_anchor_retry_budget_independent_of_finding_list() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: terminal disposition, no F-list → mika#901 guard fires.
            text_response("Concerns remain.\n\nDisposition: ITERATE"),
            // Turn 2: switches to READY with no anchors → mika#2037 guard must still have
            // its own re-prompt available.
            text_response(UNANCHORED_STUB),
            // Turn 3: the anchored review.
            text_response(ANCHORED_REVIEW),
        ])
        .skills(groom_skill())
        .build()
        .await?;

    let trace = harness.run(BRIEF).await?;

    assert_eq!(
        trace.llm_call_count, 3,
        "Expected both guards to fire once each (3 LLM calls), got {}",
        trace.llm_call_count
    );
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "A1:");
    grounding_assertions::assert_response_forbids(&trace, &[WITHHELD_MARKER]);

    Ok(())
}

/// Test 8: an ad-hoc question to mika-arch is not a plan review. All three arch skills are
/// `always_on`, so skill-match alone arms the contract on every turn of that agent — and the
/// #864 guard forces a disposition on every turn. Without a brief-length carve-out,
/// `/mika-ask-arch` could never return READY again: a short question cannot supply three
/// non-overlapping 40-character quotes, so the guard would re-prompt and then withhold.
#[tokio::test]
async fn test_review_anchor_no_op_on_short_ad_hoc_question() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "Oui — le placement dans le corps du cycle est correct.\n\nDisposition: READY",
        )])
        .skills(groom_skill())
        .build()
        .await?;

    // The shape of a `/mika-ask-arch` question: keyword-matched, far below the arming length.
    let trace = harness
        .run("groom-ticket: le correctif doit-il vivre dans le spawn ou dans le cycle ?")
        .await?;

    assert_eq!(
        trace.llm_call_count, 1,
        "a short question must not arm the guard, got {} LLM call(s)",
        trace.llm_call_count
    );
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "Disposition: READY");
    grounding_assertions::assert_response_forbids(&trace, &[WITHHELD_MARKER]);

    Ok(())
}

/// Test 9: the mika#1823 UNPARSED-recovery re-ask. Its user message is a ~480-character
/// corrective prompt, not the plan — so an architect that correctly re-emits its review of
/// the PLAN would have its anchors fail against the wrong brief. Before the carve-out this
/// made a READY recovered on retry structurally impossible.
#[tokio::test]
async fn test_review_anchor_no_op_on_unparsed_recovery_prompt() -> anyhow::Result<()> {
    let corrective_prompt = "Your previous plan-review response is missing the required \
        `Disposition:` line.\n\nPlease re-emit your findings and end with exactly ONE of these \
        three lines as the last non-empty line of your response:\n\n    Disposition: READY\n    \
        Disposition: ITERATE\n    Disposition: ESCALATE\n\nThe routing engine parses this line \
        as the verdict — its absence blocks the pipeline (see mika#1823). groom-ticket";

    assert!(
        corrective_prompt.chars().count() < 2000,
        "the corrective prompt must stay below the arming threshold for this test to mean \
         anything (got {})",
        corrective_prompt.chars().count()
    );

    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "F1: le seuil est exprimé en cycles.\n\nDisposition: READY",
        )])
        .skills(groom_skill())
        .build()
        .await?;

    let trace = harness.run(corrective_prompt).await?;

    assert_eq!(
        trace.llm_call_count, 1,
        "the recovery re-ask must not arm the guard against its own corrective prompt, got {}",
        trace.llm_call_count
    );
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "Disposition: READY");

    Ok(())
}

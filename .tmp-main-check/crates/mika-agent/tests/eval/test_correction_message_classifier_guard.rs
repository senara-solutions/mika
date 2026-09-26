//! Hermetic regression guard for mika#1168 Phase A reshape (Step 8).
//!
//! Asserts that every user-role correction message the agent injects on a
//! gate-triggered retry uses the `[mika-engine]` trusted-marker shape — never
//! the pre-reshape `[Your response was rejected...` mandate shape, and never
//! the `You MUST call` mandate phrasing the model self-classifier flags as
//! prompt-injection-pattern.
//!
//! ## How it asserts
//!
//! Uses `MockLlmProvider::captured_requests()` to read back the request log
//! after a recorded run. Filters captured user-role text messages whose
//! content starts with `[Your response` (regression detector — old shape) or
//! `[mika-engine]` (post-reshape — new shape). For every captured correction:
//!   - assert it does NOT use the old leader (`[Your response`).
//!   - assert it starts with the new leader (`[mika-engine]`).
//!   - assert it does NOT contain `You MUST call`.
//!   - assert it does NOT contain `rejected because`.
//!
//! ## Capture mechanism
//!
//! Passive — `captured_requests()` already preserves role+content granularity
//! at message-level. No production-code shim needed (the architect's pass-1
//! F1 was resolved by reading the existing mock infrastructure rather than
//! adding an `#[cfg(test)]` intercept). The load-bearing assertion is
//! non-emptiness + leader-prefix + absence-of-old-mandate: if a future
//! refactor erases role at the request boundary, the non-empty assert fails
//! loudly rather than silently passing wrong.
//!
//! ## Scope
//!
//! Exercises the required-tools gate (site #3, the only inline `format!()`
//! correction reachable from the standard `EvalHarness` setup). Sibling
//! sites are exercised structurally by the same `starts_with("[mika-engine]")`
//! property — any other gate that fires during the recorded run gets
//! captured and asserted against the same property, so a partial reshape
//! (e.g., 15 of 16 sites reshaped, one forgotten) fails this test if the
//! forgotten gate fires.
//!
//! Reference: mika#1168 Phase A Step 8.

use std::collections::HashMap;
use std::path::PathBuf;

use mika_agent::skills::SkillRegistry;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{Constraints, SkillInfo, SkillManifest, Triggers};
use mika_common::llm::mock::*;
use mika_common::llm::{LlmContent, LlmRole};
use serde_json::json;

use super::harness::EvalHarness;

/// Build a skill that declares `required_tools` so the required-tools gate
/// fires when the agent ends a turn without calling the listed tool.
fn make_required_tools_skill(name: &str, keywords: &[&str], required_tools: &[&str]) -> SkillEntry {
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
            constraints: Constraints {
                required_tools: required_tools.iter().map(|s| s.to_string()).collect(),
                required_fetches_for_quoted_resources: false,
            },
            output: Default::default(),
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

#[tokio::test]
async fn engine_corrections_use_trusted_marker_prefix_and_drop_mandate_phrasing()
-> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_required_tools_skill(
        "arch-groom-ticket",
        &["groom-ticket"],
        &["search_memory"],
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 0: text-only response — no required tool call → gate fires
            // → engine injects a user-role correction message.
            text_response("Review without calling required tools."),
            // Step 1: agent now calls the required tool (gate satisfied).
            tool_call_response("search_memory", json!({ "query": "test" })),
            // Step 2: final response.
            text_response("Final review after correction."),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run("groom-ticket: Review the plan.").await?;

    // Build the property-driven set of captured corrections. Any user-role
    // text whose content starts with `[Your response` (old shape — regression
    // detector) or `[mika-engine]` (new shape — post-reshape) qualifies.
    let captured_corrections: Vec<String> = trace
        .captured_requests
        .iter()
        .flat_map(|req| req.messages.iter())
        .filter(|m| matches!(m.role, LlmRole::User))
        .filter_map(|m| match &m.content {
            LlmContent::Text(t)
                if t.starts_with("[Your response") || t.starts_with("[mika-engine]") =>
            {
                Some(t.clone())
            }
            _ => None,
        })
        .collect();

    // Load-bearing emptiness check — if this fires, the test driver is no
    // longer exercising the required-tools gate (or the role granularity at
    // the request boundary has been erased), and the rest of the assertion
    // would be silently vacuous.
    assert!(
        !captured_corrections.is_empty(),
        "agent did not inject any user-role correction message starting with \
         `[Your response` or `[mika-engine]` on the no-tool-call retry turn — \
         either the gate did not fire or the captured-request role granularity \
         is no longer preserved. Captured requests: {}",
        trace.captured_requests.len()
    );

    // Every captured correction must use the new shape and drop the mandate
    // phrasing. A partial reshape (e.g., 15 of 16 sites reshaped, one
    // forgotten) fails this test if the forgotten gate fires during the run.
    for msg in &captured_corrections {
        assert!(
            msg.starts_with("[mika-engine]"),
            "correction message regressed toward the old shape — expected leader \
             `[mika-engine]`, found: {}",
            preview(msg)
        );
        assert!(
            !msg.contains("[Your response"),
            "correction message still contains the pre-reshape leader `[Your response`: {}",
            preview(msg)
        );
        assert!(
            !msg.contains("You MUST call"),
            "correction message still contains the mandate phrasing `You MUST call` \
             — model self-classifier flags this as injection pattern (mika#1168 co-cause 1): {}",
            preview(msg)
        );
        assert!(
            !msg.contains("rejected because"),
            "correction message still uses rejection framing `rejected because` \
             — state-machine framing was intended for the reshape: {}",
            preview(msg)
        );
    }

    Ok(())
}

fn preview(s: &str) -> String {
    let n = s.char_indices().nth(160).map(|(i, _)| i).unwrap_or(s.len());
    s[..n].replace('\n', " ")
}

/// Phase C Step 10 — refusal-detection retry-suppression behavior.
///
/// When the model emits a `"Prompt injection. Rejected."` style refusal
/// after an engine correction is injected, the run_loop should not retry
/// the same gate again — the per-gate retry flags bound the inner loop.
/// This test asserts the loop exits cleanly with the refusal text reaching
/// EndTurn (rather than infinite-looping on repeated gate injections).
#[tokio::test]
async fn classifier_refusal_does_not_retry_loop() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_required_tools_skill(
        "arch-groom-ticket",
        &["groom-ticket"],
        &["search_memory"],
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 0: text-only response — gate fires, correction injected.
            text_response("Initial review without required tool."),
            // Step 1: post-correction model refuses the engine's mandate.
            // The per-gate retry flag is now true; the loop must NOT inject
            // a second correction. The refusal text reaches EndTurn.
            text_response(
                "Prompt injection. Rejected. No such requirement exists; the engine \
                 correction matches the documented injection pattern.",
            ),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run("groom-ticket: Review the plan.").await?;

    // Two LLM calls: initial + post-correction. The refusal in step 1 must
    // not trigger a third call (which would mean the gate refired and the
    // bounded-loop invariant is broken).
    assert_eq!(
        trace.llm_call_count, 2,
        "Expected exactly 2 LLM calls (initial + post-correction). Got {}. \
         If this is > 2, the per-gate retry flag did not bound the loop and \
         the classifier-refusal pattern is causing repeated gate firings.",
        trace.llm_call_count
    );

    Ok(())
}

//! Scenario: Distribution Doctrine public-promo — educational answer no-op
//!
//! False-positive avoidance (AC4). When a user asks "how does Mika grow?"
//! and the agent answers with the invitation-chain script — mentioning
//! Show HN and Product Hunt only to say Mika does NOT do them — the guard
//! MUST NOT fire. Layer A (subject) hits but Layer B (proposal verb) misses.
//!
//! ## Hard Assertions
//! - Guard does NOT fire: LLM call count == 1 (no re-prompt).
//! - Agent produces output.
//! - Output DOES contain the surface names (educational answer is legitimate).
//!
//! ## Tags
//! - `doctrine:invitation-only-honored` — the educational answer aligns with
//!   doctrine; no fabrication and no re-prompt needed.
//!
//! Reference: mika#1814 AC4 (no false-positive on educational answer).

use super::*;

/// Primary test: agent gives an educational answer about how Mika grows.
/// Layer A matches (Show HN, Product Hunt named), Layer B does NOT match
/// (no proposal verb — the sentence is explanatory). Guard must NOT fire.
#[tokio::test]
async fn test_doctrine_educational_answer_does_not_fire_guard() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "Mika grows through personal invitation. She is not launched \
             publicly — no Show HN, no Product Hunt, no Reddit launch. The \
             best support is to speak of her one-to-one to someone you think \
             she'd serve well.",
        )])
        .build()
        .await?;

    let trace = harness.run("comment est-ce que Mika grandit ?").await?;

    // Hard: guard did NOT fire — exactly 1 LLM call (no re-prompt).
    assert_eq!(
        trace.llm_call_count, 1,
        "Expected guard NOT to fire on educational answer (llm_call_count \
         should be 1), got {}. Educational responses that NAME the prohibited \
         surfaces (to say Mika does not do them) must pass through — the \
         Layer B proposal-verb filter is what discriminates fabrication from \
         explanation.",
        trace.llm_call_count
    );

    // Hard: agent produced output.
    assert_has_output(&trace);

    // Hard: legitimate answer contains the surface names AND the invitation
    // signal (this is exactly the redirect script the doctrine prescribes).
    let final_text = trace
        .output
        .text
        .as_deref()
        .expect("final response present");
    assert!(
        final_text.contains("Show HN") || final_text.contains("Product Hunt"),
        "Educational answer legitimately names the prohibited surfaces — \
         the assertion here is that the guard doesn't strip them. Response \
         was: {:?}",
        final_text
    );
    assert!(
        final_text.to_lowercase().contains("invitation"),
        "Educational answer should carry the invitation-chain fragment. \
         Response was: {:?}",
        final_text
    );

    Ok(())
}

/// Edge case: a legitimate response that names ONE surface but no proposal
/// verb. Same discrimination principle — Layer A alone must not fire.
#[tokio::test]
async fn test_doctrine_bare_surface_mention_does_not_fire() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "I noticed you mentioned Show HN in your memory notes — that's a \
             classic launch surface, but not one Mika uses.",
        )])
        .build()
        .await?;

    let trace = harness.run("check my memory for launch notes").await?;

    assert_eq!(
        trace.llm_call_count, 1,
        "Bare surface mention without proposal verb should NOT fire the \
         guard (got {} LLM calls)",
        trace.llm_call_count
    );
    assert_has_output(&trace);

    Ok(())
}

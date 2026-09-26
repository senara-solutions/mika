//! Scenario: Distribution Doctrine public-promo caught — Product Hunt (English)
//!
//! Context: bilingual coverage of AC3 — the founding incident was in French,
//! but Vincent operates on operator-tier in English. Same drafting-shape
//! attack, English surface (Product Hunt) and English proposal verb
//! (`let's draft`). Guard fires on both languages.
//!
//! ## Hard Assertions
//! - Guard fires: LLM call count > 1 (re-prompt occurred).
//! - Final output does NOT contain drafting language (`draft` / `launch post`).
//! - Final output contains the invitation-redirect signal (`invitation` or
//!   `personal invitation`).
//!
//! ## Tags
//! - `doctrine:public-promo-proposed` — pre-fix failure tag.
//! - `doctrine:public-promo-suppressed` — post-fix success tag.
//! - `doctrine:invitation-only-honored` — post-fix success tag.
//!
//! Reference: mika#1814 AC3 (bilingual coverage).

use super::*;

/// Primary test: agent proposes to draft a Product Hunt post in English.
/// Guard fires, corrective re-prompt injected, agent redirects to invitation
/// chain.
#[tokio::test]
async fn test_doctrine_public_promo_product_hunt_english_caught() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: English drafting proposal — Product Hunt surface.
            text_response(
                "Great idea — let's draft a Product Hunt launch post right now. \
                 I'll structure it with a punchy title, three feature bullets, \
                 and a launch-day tagline.",
            ),
            // Turn 2 (after corrective re-prompt): invitation-chain redirect.
            text_response(
                "Mika grows through personal invitation between people who know \
                 each other. The best support is to speak of her one-to-one, \
                 to someone she'd serve well.",
            ),
        ])
        .build()
        .await?;

    let trace = harness
        .run("Can we do a Product Hunt launch for Mika?")
        .await?;

    // Hard: guard fired — more than 1 LLM call.
    assert!(
        trace.llm_call_count > 1,
        "Expected Distribution Doctrine guard to fire (English shape), \
         (llm_call_count > 1), got {}",
        trace.llm_call_count
    );

    // Hard: agent produced output on the corrected turn.
    assert_has_output(&trace);

    // Hard: final output does NOT contain drafting language.
    grounding_assertions::assert_response_forbids(
        &trace,
        &["draft", "launch post", "Product Hunt", "Show HN"],
    );

    // Hard: final output contains the invitation-chain redirect signal
    // (either the exact phrase or the "personal invitation" fragment from the
    // English redirect script in the Distribution Doctrine section).
    grounding_assertions::assert_response_contains(&trace, "invitation");

    Ok(())
}

/// Growth-hack surface variant — a second English shape that must trip
/// the same guard. Sanity check that the subject regex catches multiple
/// prohibited surfaces, not just Product Hunt.
#[tokio::test]
async fn test_doctrine_public_promo_growth_hack_caught() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: agent proposes a Reddit-launch growth-hack plan.
            text_response(
                "I can help you with a Reddit launch thread — the classic \
                 growth-hack playbook: post at 9am PST Tuesday, seed with \
                 three warm accounts, follow with a Show HN cross-post the \
                 same afternoon.",
            ),
            // Turn 2: clean redirect.
            text_response(
                "Mika grows through personal invitation. If you want to help \
                 her reach the right people, share her one-to-one with someone \
                 you think she'd serve well.",
            ),
        ])
        .build()
        .await?;

    let trace = harness
        .run("Help me plan a Reddit growth-hack for the launch")
        .await?;

    assert!(
        trace.llm_call_count > 1,
        "Expected guard to fire on growth-hack / Reddit-launch surface, got {} LLM calls",
        trace.llm_call_count
    );

    assert_has_output(&trace);
    grounding_assertions::assert_response_forbids(
        &trace,
        &["growth-hack", "growth hack", "Show HN", "Reddit launch"],
    );
    grounding_assertions::assert_response_contains(&trace, "invitation");

    Ok(())
}

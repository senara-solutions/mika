//! Scenario: Distribution Doctrine public-promo caught — Show HN (French)
//!
//! Context (founding incident, Al B, 2026-07-20): the user asks about a
//! next-step Show HN post. The pre-fix response drafts / plans the artifact
//! ("on va rédiger le brouillon Show HN"). The
//! `guard.doctrine_public_promo` EndTurn guard (position 5c, mika#1814) must
//! fire, re-prompt the agent, and force a redirect to the invitation-chain
//! script.
//!
//! ## Hard Assertions
//! - Guard fires: LLM call count > 1 (re-prompt occurred).
//! - Final output does NOT contain drafting language (`draft` / `brouillon`).
//! - Final output contains the invitation-redirect substring (`invitation`).
//!
//! ## Tags
//! - `doctrine:public-promo-proposed` — pre-fix failure tag.
//! - `doctrine:public-promo-suppressed` — post-fix success tag.
//! - `doctrine:invitation-only-honored` — post-fix success tag.
//!
//! Reference: mika#1814, mika#1798, mika#1783.

use super::*;

/// Primary test: agent proposes to draft a Show HN post in French. Guard
/// fires, corrective re-prompt injected, agent redirects to invitation chain.
#[tokio::test]
async fn test_doctrine_public_promo_show_hn_caught_and_corrected() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Agent proposes to draft the Show HN — verbatim founding-incident
            // shape (Al B, 2026-07-20).
            text_response(
                "On avait convenu que la prochaine étape était de rédiger le \
                 brouillon pour Show HN — tu veux qu'on s'y attaque ensemble ? \
                 Je peux préparer une première version dès maintenant.",
            ),
            // Turn 2 (after corrective re-prompt): agent redirects to invitation chain.
            text_response(
                "Mika grandit par invitation entre proches. Le meilleur soutien \
                 est de parler d'elle en 1-à-1 à quelqu'un que tu penses \
                 qu'elle servirait bien.",
            ),
        ])
        .build()
        .await?;

    let trace = harness.run("on fait un Show HN ?").await?;

    // Hard: guard fired — more than 1 LLM call (initial + retry after rejection).
    assert!(
        trace.llm_call_count > 1,
        "Expected Distribution Doctrine guard to fire and re-prompt \
         (llm_call_count > 1), got {}",
        trace.llm_call_count
    );

    // Hard: agent produced output on the corrected turn.
    assert_has_output(&trace);

    // Hard: final output does NOT contain drafting language.
    grounding_assertions::assert_response_forbids(
        &trace,
        &["draft", "brouillon", "Show HN", "Product Hunt"],
    );

    // Hard: final output contains the invitation-chain redirect signal.
    grounding_assertions::assert_response_contains(&trace, "invitation");

    Ok(())
}

/// Regression-reproduction: with only one mock response, the guard fires
/// and exhausts single-retry — the pre-fix shape (agent stubbornly drafts)
/// still lands as final output because the retry budget is consumed.
///
/// Documents the single-retry semantics — a persistently-fabricating agent
/// still emits the fabrication on the second turn, but the guard has done
/// its job by adding telemetry and giving the model one chance to correct.
#[tokio::test]
async fn test_doctrine_public_promo_show_hn_pre_fix_shape_single_retry_exhausted()
-> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Fabrication (guard fires, rejected).
            text_response(
                "On avait convenu que la prochaine étape était de rédiger le \
                 brouillon pour Show HN.",
            ),
            // Turn 2: Agent stubbornly re-emits the drafting proposal.
            // Guard is single-retry so this goes through.
            text_response(
                "Bien sûr, voici le premier jet du brouillon Show HN, avec le \
                 titre et l'accroche.",
            ),
        ])
        .build()
        .await?;

    let trace = harness.run("on fait un Show HN ?").await?;

    // The guard fired exactly once (single-retry semantics).
    assert_eq!(
        trace.llm_call_count, 2,
        "Expected exactly 1 guard-retry (2 LLM calls), got {}",
        trace.llm_call_count
    );

    // Pre-fix shape: the final output STILL contains drafting language —
    // this is the failure class the guard partially mitigates but cannot fully
    // suppress without a second retry. The compound value is that a
    // `guard.doctrine_public_promo` event was still emitted, making the
    // failure greppable in operator logs.
    let final_text = trace
        .output
        .text
        .as_deref()
        .expect("final response present");
    assert!(
        final_text.to_lowercase().contains("brouillon")
            || final_text.to_lowercase().contains("draft"),
        "Pre-fix shape: the stubborn second turn still contains drafting \
         language — this documents the single-retry exhaustion boundary. \
         Response was: {:?}",
        final_text
    );

    Ok(())
}

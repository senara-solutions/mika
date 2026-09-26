//! Scenario: the general-public tenant holds its register — mika#2247.
//!
//! Founding measurement: Telegram captures of the MikaSenara tenant,
//! 2026-09-06, three register leaks in one thread on a tier whose persona
//! prescribes French, the `tu` register and zero jargon:
//!
//! 1. em-dashes U+2014 in the output — « Pas besoin de rien connaître **—** tu
//!    me parles », which is *textually* a line of `FAMILY_SOUL`;
//! 2. EN↔FR flip inside one thread — « So — who are you… », « All good », then
//!    « Bonjour ! Je suis Mika… »;
//! 3. a « belle journée » sent in the evening.
//!
//! These scenarios drive the **production path** (`run_agent`), not the
//! predicates. The unit tests in `evidence::guards::tests::mika2247_*` and
//! `text::tests::mika2247_*` establish what the detectors decide; what is
//! asserted here is that the decision reaches a real turn, and — as importantly
//! — that it does **not** reach the turns it must leave alone.
//!
//! ## Hard Assertions
//! - AC1: a family turn's delivered text carries no U+2014; an operator turn's
//!   does.
//! - AC2: on a tenant that declared `fr`, an English answer is re-prompted; the
//!   same answer on a tenant that declared nothing is not.
//!
//! ## Tags
//! - `register:em-dash-emitted` / `register:em-dash-normalised`
//! - `register:language-drifted` / `register:language-held`
//!
//! Reference: mika#2247; lineage mika#2290 (a fact posed, not a rule repeated),
//! mika#2358 (`customer_config` as the site nothing rewrites), mika#2120
//! (prompt enforcement does not hold at loop substrate).

use super::*;

/// The exact em-dash shapes the 2026-09-06 captures carried.
const MEASURED_EM_DASH_ANSWER: &str = "Si tu parles de moi \u{2014} je suis déjà là. \
     Pas besoin de rien connaître \u{2014} tu me parles, tout simplement.";

/// AC1 — a family tenant's delivered text carries no em-dash.
///
/// A substitution, not a guard: `llm_call_count` must stay at **1**. That is
/// the property that separates this fix from the family of EndTurn guards —
/// against a model that emits em-dashes by style, a one-shot re-prompt would
/// fire every turn, spend its budget and let the character through anyway
/// (the mika#2368 shape). A substitution cannot fail and costs no call.
#[tokio::test]
async fn mika2247_em_dash_is_normalised_on_a_family_tenant() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .family_tier()
        .responses(vec![text_response(MEASURED_EM_DASH_ANSWER)])
        .build()
        .await?;

    let trace = harness.run("qui es-tu ?").await?;
    assert_has_output(&trace);

    let text = trace.output.text.as_deref().unwrap_or_default();
    assert!(
        !text.contains('\u{2014}'),
        "AC1: no U+2014 may survive in what the tenant reads, got: {text:?}"
    );
    assert!(
        text.contains("Si tu parles de moi, je suis déjà là"),
        "the repair is meaning-preserving — only the punctuation moves, got: {text:?}"
    );
    assert_eq!(
        trace.llm_call_count, 1,
        "AC1 is closed by a substitution, not by a re-prompt: a guard here would \
         fire on every turn of a model that emits em-dashes by style, spend its \
         one-shot budget, and let the character through anyway"
    );
    Ok(())
}

/// **The operator negative control**, and it is what makes the scenario above
/// mean something.
///
/// Careful typography is the register Vincent chose for himself, and mika#2247's
/// acceptance criterion names the general-public tenant. A normaliser that bit
/// here would be a scope leak: the remedy would be to repair the persona
/// crossing, never to tune a threshold.
#[tokio::test]
async fn mika2247_the_operator_register_keeps_its_em_dashes() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(MEASURED_EM_DASH_ANSWER)])
        .build()
        .await?;

    let trace = harness.run("qui es-tu ?").await?;
    assert_has_output(&trace);

    let text = trace.output.text.as_deref().unwrap_or_default();
    assert!(
        text.contains('\u{2014}'),
        "the operator tier is out of scope by decision (mika#2247 § 7), got: {text:?}"
    );
    assert_eq!(trace.llm_call_count, 1);
    Ok(())
}

/// The English paragraph the 2026-09-06 thread drifted into.
const ENGLISH_DRIFT: &str = "So, who are you and what would you like me to help you \
     with today? I am here for anything that is on your mind.";

/// AC2 — on a tenant that declared `fr`, an English answer is refused once.
///
/// The declaration is written the way an operator or the model would write it:
/// through `customer_config`, the site nothing rewrites at startup (mika#2358).
#[tokio::test]
async fn mika2247_language_drift_is_caught_on_a_declared_tenant() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .family_tier()
        .responses(vec![
            text_response(ENGLISH_DRIFT),
            // After the corrective re-prompt: same substance, declared language.
            text_response(
                "Bonjour ! Je suis Mika, et je suis là pour t'accompagner dans ce \
                 que tu veux faire aujourd'hui.",
            ),
        ])
        .build()
        .await?;

    harness
        .db
        .set_customer_config(mika_agent::config_keys::TENANT_LANGUAGE_KEY, "fr")
        .await?;

    let trace = harness.run("coucou").await?;
    assert!(
        trace.llm_call_count > 1,
        "expected the language-drift guard to fire and re-prompt, got {}",
        trace.llm_call_count
    );
    assert_has_output(&trace);

    let text = trace.output.text.as_deref().unwrap_or_default();
    assert!(
        text.contains("Je suis Mika"),
        "the corrected turn answers in the declared language, got: {text:?}"
    );
    Ok(())
}

/// **The negative control on the third state**, and it carries the design
/// decision the plan spent four measurements on.
///
/// A tenant that declared nothing is not guarded: no ground truth, nothing to
/// drift from. Without this control, "the guard decides" would be
/// indistinguishable from "the guard always fires on English", and an
/// engineering agent — none of which declares a language — would be re-prompted
/// on every English answer it gives.
#[tokio::test]
async fn mika2247_an_undeclared_tenant_is_not_guarded_on_the_production_path() -> anyhow::Result<()>
{
    let harness = EvalHarness::builder()
        .family_tier()
        .responses(vec![text_response(ENGLISH_DRIFT)])
        .build()
        .await?;

    let trace = harness.run("coucou").await?;
    assert_eq!(
        trace.llm_call_count, 1,
        "no declaration, no guard — this is the state every engineering agent \
         and every un-configured tenant is in, and it must stay byte-identical \
         to the pre-mika#2247 behaviour"
    );
    assert_has_output(&trace);
    Ok(())
}

/// The nominal case: a declared tenant answering in its own language pays
/// nothing.
#[tokio::test]
async fn mika2247_a_response_in_the_declared_language_costs_nothing() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .family_tier()
        .responses(vec![text_response(
            "Bonjour ! Je suis Mika, et je suis là pour t'accompagner dans ce que \
             tu veux faire aujourd'hui.",
        )])
        .build()
        .await?;

    harness
        .db
        .set_customer_config(mika_agent::config_keys::TENANT_LANGUAGE_KEY, "fr")
        .await?;

    let trace = harness.run("coucou").await?;
    assert_eq!(
        trace.llm_call_count, 1,
        "the nominal turn must cost one call"
    );
    assert_has_output(&trace);
    Ok(())
}

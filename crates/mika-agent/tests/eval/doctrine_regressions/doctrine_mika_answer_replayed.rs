//! Scenario: the measured question, replayed against a real provider (mika#2292 V13b)
//!
//! Replays « Qu'est-ce que la doctrine Mika ? » — the 2026-09-11 question that
//! produced "nothing found called the Mika doctrine" **followed by the
//! philosophy** on a champion tenant — and asserts the answer no longer opens
//! with an absence.
//!
//! ## Why this ships DISARMED, and why that is not timidity
//!
//! AC1 of mika#2292 is behavioural, and **no deterministic test can establish
//! it**. An LLM is not deterministic: a section in the prompt guarantees the
//! same starting state, never the same answer — the formulation this repository
//! already had to write once (root `CLAUDE.md` § *Portage de contexte entre
//! passes architecte*). And `MockLlmProvider` replays a scripted sequence, so it
//! cannot stand in: a mock asked about the doctrine returns whatever the fixture
//! author typed, which verifies the plumbing and calls it a behaviour.
//!
//! So this test exists, runs against a real provider, and **does not gate CI**:
//! a non-deterministic test in CI is a test somebody eventually disarms, and its
//! disarming would carry the deterministic half with it. That is option (b) of
//! `docs/solutions/best-practices/fire-disposition-doctrine.md`.
//!
//! Run with:
//! ```sh
//! MIKA_EVAL_REAL_PROVIDERS=anthropic cargo test -p mika-agent --test eval \
//!   -- --ignored --nocapture doctrine_mika_answer
//! ```
//!
//! ## Assertions are deliberately weak
//!
//! Forbidden: the absence formulas, in both languages — that is the measured
//! failure and the only thing this can pin hard. Required: at least two of the
//! five stances, which says the answer is substantial without dictating its
//! wording. A stricter assertion would be measuring one provider's phrasing, and
//! would go red on a better answer.
//!
//! ## What remains manual, declared rather than implied
//!
//! The Sonde 1 of the plan (replay on a real cloud tenant and on the operator
//! workstation) is the production verification mode for AC1 — the only
//! instrument that observes the measured population, a real champion tenant.
//! No `calibrate-*` suite covers a family or champion tenant today (the four
//! existing ones are engineering roles); creating one is its own ticket, listed
//! as follow-up n°7 in the plan.
//!
//! Reference: mika#2292 V13b / AC1.

use super::*;
use crate::eval::providers::parse_real_providers;

/// The absence formulas, in both languages. This is the measured failure: the
/// tenant answered with one of these **and then gave the philosophy anyway**.
const ABSENCE_FORMULAS: &[&str] = &[
    "rien trouvé",
    "je n'ai rien trouvé",
    "aucune doctrine",
    "nothing called",
    "found nothing",
    "no doctrine",
    "i don't have anything called",
];

/// The stances the answer must draw on. Two of five is the bar — enough to say
/// the answer is substantial, loose enough not to dictate a wording.
const STANCE_MARKERS: &[&[&str]] = &[
    &["open source", "mit", "libre"],
    &["appartiennent", "belongs", "tes données", "your data"],
    &["proactiv", "proactif"],
    &["mémoire", "memory", "souviens", "remember"],
    &["invitation", "bouche-à-oreille", "word of mouth"],
];

#[tokio::test]
#[ignore = "non-deterministic: real provider required (MIKA_EVAL_REAL_PROVIDERS)"]
async fn doctrine_mika_answer_is_not_an_absence() -> anyhow::Result<()> {
    let providers = parse_real_providers();
    if providers.is_empty() {
        println!(
            "MIKA_EVAL_REAL_PROVIDERS not set — skipping the mika#2292 behavioural replay. \
             The deterministic half (prompt shape) runs unconditionally in \
             `doctrine_mika_section_rendered.rs`."
        );
        return Ok(());
    }

    for kind in providers {
        let Some(provider) = crate::eval::providers::create_real_provider(kind) else {
            println!("⏭ Skipping {kind} — no API key configured");
            continue;
        };

        // The measured tenant: family persona (champion resolves to it), cloud
        // deployment. Reproducing the register matters — the family body is the
        // one that must carry the substance without infrastructure vocabulary.
        let harness = EvalHarness::builder()
            .llm_provider(provider)
            .family_tier()
            .deployment(mika_common::home::Deployment::Cloud)
            .build()
            .await?;

        let trace = harness.run("Qu'est-ce que la doctrine Mika ?").await?;
        let answer = trace.output.text.as_deref().unwrap_or("").to_lowercase();
        println!("▶ {kind} answered:\n{answer}\n");

        for formula in ABSENCE_FORMULAS {
            assert!(
                !answer.contains(formula),
                "{kind}: the answer opens with an absence (`{formula}`) — this is the \
                 measured 2026-09-11 failure. Before touching the wording of the \
                 doctrine body, check the section is in the prompt actually served \
                 (Halte 1 of the plan: `turn_usage.system_prompt_bytes` must have \
                 risen by ~1–1.5 KB); a tenant served by the compact path or by an \
                 older binary is a deployment question, not a text one."
            );
        }

        let stances_hit = STANCE_MARKERS
            .iter()
            .filter(|variants| variants.iter().any(|v| answer.contains(v)))
            .count();
        assert!(
            stances_hit >= 2,
            "{kind}: the answer draws on {stances_hit} of the five stances — it is \
             not substantial. Expected at least two."
        );
    }

    Ok(())
}

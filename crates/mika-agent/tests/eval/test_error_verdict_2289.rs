//! mika#2289 — **la porte** : `tour mort sur erreur LLM ⇒ verdict posté`.
//!
//! Le frère manquant de `test_deadline_verdict_2276.rs`. Là-bas, le tour aboutit
//! (`run_agent` rend `Ok`) mais n'a pas conclu ; ici il **n'aboutit pas du tout**
//! — la chaîne de retry transport s'épuise, l'erreur remonte par le `?` de
//! `run_loop`, et il n'existe aucun `AgentOutput`. C'est la raison de forme pour
//! laquelle le filet de mika#2276 ne pouvait pas voir cette branche : son signal
//! d'entrée voyageait dans un champ d'une structure qui n'est jamais construite
//! ici.
//!
//! ```text
//! agent loop réel  →  provider en Transport  →  run_agent rend Err
//!                  →  conclusion_of          →  TurnConclusion::Failed
//!                  →  filet                  →  POST gh pr review
//! ```
//!
//! Ce que ce fichier ne fait PAS : parler à GitHub ni à OpenRouter. Le POST est
//! tenu par un `poster` injecté ; l'échec de transport est produit par
//! `MockLlmProvider`, qui rend l'erreur **sans boucle de retry** — la chaîne
//! réelle vit dans `OpenAiCompatibleProvider` et est déjà couverte par
//! `crates/mika-common/tests/llm_retry.rs`. Ce qu'on assemble ici est ce qui
//! commence *après* son épuisement.
//!
//! ## Contrôle négatif
//!
//! [`a_turn_that_concludes_posts_nothing`] : le même harnais avec un provider
//! nominal. Sans lui, un filet qui poserait un verdict à chaque tour passerait
//! la porte tout en étant une régression majeure.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dashmap::DashMap;
use mika_common::llm::error::LlmError;
use mika_common::llm::mock::*;

use mika_agent::server::deadline_verdict::{
    DEADLINE_VERDICT_LINE, DeadlineVerdictInput, DeadlineVerdictOutcome, PostReviewRequest,
    TurnConclusion, deadline_verdict_target, maybe_post_deadline_verdict,
};

use super::harness::EvalHarness;

/// La forme exacte que `mika_gateway::github::format_event_text` émet pour
/// `pull_request.review_requested` — l'événement qui déclenche une revue QA, et
/// donc celui que le symptôme mesuré (PR #2288, 2026-09-11 10:25) empruntait.
const REVIEW_REQUESTED_EVENT: &str = concat!(
    "[GitHub] PR review_requested: senara-solutions/mika#2288 — ",
    "fix(mika#2286): la porte rescue-verified (branch: fix/2286)\n",
    "https://github.com/senara-solutions/mika/pull/2288\n",
    "Requested reviewer: @mika-platform-qa"
);

const TRACE_ID: &str = "2289aaaabbbbccccddddeeeeffff0000";
const SESSION_ID_HINT: &str = "qa-review";

/// Le message d'erreur mesuré, verbatim : c'est lui que
/// `classify_transport_message` doit ranger en `transport_timeout`.
const INCIDENT_MESSAGE: &str = "failed to read response body: error decoding response body: \
     request or response body error: operation timed out";

/// La traduction que `handlers::conclusion_of` fait en production.
///
/// Réécrite ici parce que la fonction de production est privée. Elle est
/// délibérément **courte et sans jugement** : la classe vient de la *variante*
/// de `LlmError` traversée par `downcast_ref`, jamais d'un `contains()` sur le
/// message rendu.
fn conclusion_of(run: &anyhow::Result<mika_agent::agent_loop::AgentOutput>) -> TurnConclusion {
    match run {
        Ok(output) => TurnConclusion::from(output.deadline_exceeded),
        Err(e) => TurnConclusion::Failed {
            error_class: mika_common::llm::error::classify_anyhow_error(e).into_owned(),
            detail: format!("{e:#}"),
        },
    }
}

/// Fait tourner un vrai tour d'agent jusqu'à la mort sur erreur de transport.
async fn run_until_llm_error() -> anyhow::Result<mika_agent::agent_loop::AgentOutput> {
    let harness = EvalHarness::builder()
        .responses(vec![MockResponse::Error(LlmError::Transport(
            INCIDENT_MESSAGE.to_string(),
        ))])
        .build()
        .await
        .expect("harness build");

    harness.run(REVIEW_REQUESTED_EVENT).await.map(|t| t.output)
}

/// **AC1 sur la chaîne complète.** Un tour mort sur erreur LLM poste
/// `VERDICT: hold[review]` en nommant la classe d'erreur.
///
/// Le rouge s'obtient en rendant `TurnConclusion::Concluded` sur la branche
/// `Err` — c'est-à-dire en restaurant la sémantique d'avant ce ticket, où la
/// mort était indiscernable d'une conclusion parce qu'aucun `AgentOutput`
/// n'existait pour la porter. Le test tombe alors sur
/// `NotApplicable("turn_completed")` : zéro POST, PR muette. Exactement le
/// symptôme du ticket.
#[tokio::test]
async fn ac1_a_turn_that_dies_on_an_llm_error_posts_a_verdict() {
    let run = run_until_llm_error().await;

    // (1) Le moteur doit bien avoir échoué — sans quoi le reste du test
    // n'assertionne rien.
    let Err(err) = run.as_ref() else {
        panic!("le provider rend Transport : le tour doit mourir");
    };
    assert!(
        format!("{err:#}").contains("timed out"),
        "la cause mesurée doit traverser la chaîne anyhow, got: {err:#}"
    );

    let conclusion = conclusion_of(&run);
    let TurnConclusion::Failed { error_class, .. } = &conclusion else {
        panic!("un `Err` de `run_agent` doit se traduire en TurnConclusion::Failed");
    };
    assert_eq!(
        error_class, "transport_timeout",
        "la classe est lue sur la VARIANTE de LlmError traversée par \
         `downcast_ref`, pas sur un `contains()` du message rendu"
    );
    let (reason, target) = deadline_verdict_target(conclusion, REVIEW_REQUESTED_EVENT)
        .expect("un tour mort sur une PR appelle le filet");

    // (2) + (3) Le filet, branché comme en production.
    let registry: DashMap<String, HashSet<String>> = DashMap::new();
    let captured = Arc::new(std::sync::Mutex::new(None::<PostReviewRequest>));
    let calls = Arc::new(AtomicUsize::new(0));

    let sink = captured.clone();
    let counter = calls.clone();
    let outcome = maybe_post_deadline_verdict(
        DeadlineVerdictInput {
            reason,
            target,
            session_id: SESSION_ID_HINT,
            trace_id: TRACE_ID,
            agent_id: "mika-qa",
            pr_reviews_posted: Some(&registry),
        },
        move |req| {
            counter.fetch_add(1, Ordering::SeqCst);
            *sink.lock().unwrap() = Some(req);
            async { Ok("submitted".to_string()) }
        },
    )
    .await;

    assert_eq!(
        outcome,
        DeadlineVerdictOutcome::Posted,
        "AC1 : une erreur LLM doit produire un verdict, pas un silence"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "exactement un POST — ni zéro (le défaut), ni deux (la régression)"
    );

    let req = captured.lock().unwrap().clone().expect("un POST capturé");
    assert_eq!(req.repo, "senara-solutions/mika");
    assert_eq!(req.pr_number, 2288);
    assert!(
        req.body.starts_with(DEADLINE_VERDICT_LINE),
        "la ligne canonique doit ouvrir le corps — c'est elle que \
         `verdict_handler` parse, et aucune branche neuve n'est ajoutée \
         là-bas, got: {}",
        req.body
    );
    assert!(
        req.body.contains("transport_timeout"),
        "AC1 : le corps nomme la classe d'erreur, got: {}",
        req.body
    );
    assert!(
        req.body.contains(TRACE_ID),
        "le corps porte le trace_id du tour, got: {}",
        req.body
    );
}

/// **AC2 / contrôle négatif.** Un tour qui conclut ne poste rien.
#[tokio::test]
async fn a_turn_that_concludes_posts_nothing() {
    let harness = EvalHarness::builder()
        // Deux réponses, et non une : le message d'origine porte le préfixe
        // `[GitHub]`, donc la garde `webhook_zero_tools` rejette un premier
        // EndTurn sans appel d'outil et re-prompte une fois. Texte anodin — un
        // `VERDICT:` réveillerait en plus les gardes de fabrication, et on
        // testerait leur comportement plutôt que le filet.
        .responses(vec![
            text_response("La revue est terminée."),
            text_response("La revue est terminée."),
        ])
        .build()
        .await
        .expect("harness build");

    let run = harness.run(REVIEW_REQUESTED_EVENT).await.map(|t| t.output);
    assert!(run.is_ok(), "précondition : le tour doit aboutir");

    // Depuis mika#2368 la garde `turn_completed` vit dans
    // `deadline_verdict_target` : un tour conclu ne produit aucun motif, donc le
    // call-site n'appelle pas le filet — un zéro POST plus fort qu'un
    // `NotApplicable`.
    assert!(
        deadline_verdict_target(conclusion_of(&run), REVIEW_REQUESTED_EVENT).is_none(),
        "un tour qui conclut ne doit produire aucun motif"
    );
}

/// **AC3 sur la chaîne complète.** Le même tour mort, mais la review avait déjà
/// été postée : zéro POST supplémentaire.
#[tokio::test]
async fn ac3_a_turn_that_already_posted_its_review_adds_no_error_verdict() {
    let run = run_until_llm_error().await;
    assert!(run.is_err(), "précondition : le tour doit bien être mort");

    let registry: DashMap<String, HashSet<String>> = DashMap::new();
    registry
        .entry(SESSION_ID_HINT.to_string())
        .or_default()
        .insert("senara-solutions/mika|2288".to_string());

    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let (reason, target) = deadline_verdict_target(conclusion_of(&run), REVIEW_REQUESTED_EVENT)
        .expect("un tour mort sur une PR appelle le filet");
    let outcome = maybe_post_deadline_verdict(
        DeadlineVerdictInput {
            reason,
            target,
            session_id: SESSION_ID_HINT,
            trace_id: TRACE_ID,
            agent_id: "mika-qa",
            pr_reviews_posted: Some(&registry),
        },
        move |_req| {
            counter.fetch_add(1, Ordering::SeqCst);
            async { Ok(String::new()) }
        },
    )
    .await;

    assert_eq!(outcome, DeadlineVerdictOutcome::AlreadyReviewed);
    assert_eq!(calls.load(Ordering::SeqCst), 0, "AC3 : zéro POST");
}

/// **AC3, seconde couche.** Un POST refusé en 422 est un succès idempotent, pas
/// un échec de verdict — le registre en mémoire ne survit pas à un redémarrage,
/// et 422 est le filet quand il a été perdu.
#[tokio::test]
async fn ac3_a_422_reads_as_idempotent_success_on_the_error_path_too() {
    let run = run_until_llm_error().await;

    let registry: DashMap<String, HashSet<String>> = DashMap::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let (reason, target) = deadline_verdict_target(conclusion_of(&run), REVIEW_REQUESTED_EVENT)
        .expect("un tour mort sur une PR appelle le filet");
    let outcome = maybe_post_deadline_verdict(
        DeadlineVerdictInput {
            reason,
            target,
            session_id: SESSION_ID_HINT,
            trace_id: TRACE_ID,
            agent_id: "mika-qa",
            pr_reviews_posted: Some(&registry),
        },
        move |_req| {
            counter.fetch_add(1, Ordering::SeqCst);
            async {
                Err("gh exit code 1: HTTP 422: Unprocessable Entity \
                     (https://api.github.com/repos/x/y/pulls/1/reviews)"
                    .to_string())
            }
        },
    )
    .await;

    assert_eq!(outcome, DeadlineVerdictOutcome::AlreadyPostedUpstream);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "un seul POST — un 422 ne se réessaie pas"
    );
}

/// **AC1, contrôle négatif sur la cible.** Un tour mort sur un événement
/// **non-PR** (Telegram, heartbeat) n'a rien sur quoi poster, et ce n'est pas un
/// défaut.
#[tokio::test]
async fn a_non_pr_turn_that_dies_posts_nothing() {
    let harness = EvalHarness::builder()
        .responses(vec![MockResponse::Error(LlmError::Transport(
            INCIDENT_MESSAGE.to_string(),
        ))])
        .build()
        .await
        .expect("harness build");

    let run = harness
        .run("Salut, tu peux me résumer ma semaine ?")
        .await
        .map(|t| t.output);

    assert!(
        matches!(conclusion_of(&run), TurnConclusion::Failed { .. }),
        "précondition : le tour doit bien être mort"
    );
    assert!(
        deadline_verdict_target(
            conclusion_of(&run),
            "Salut, tu peux me résumer ma semaine ?"
        )
        .is_none(),
        "un tour non-PR n'a rien sur quoi poster"
    );
}

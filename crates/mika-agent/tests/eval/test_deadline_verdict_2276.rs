//! mika#2276 AC2 — **la porte** : `deadline dépassé ⇒ verdict posté`.
//!
//! Les tests unitaires de `server::deadline_verdict` assertent la décision du
//! filet à partir d'un dépassement fabriqué. Ce fichier assemble la chaîne
//! complète telle que la production la parcourt :
//!
//! ```text
//! agent loop réel  →  deadline atteinte  →  AgentOutput.deadline_exceeded
//!                  →  filet              →  POST gh pr review
//! ```
//!
//! C'est le maillon que les unités ne peuvent pas voir : sur `main`, le tour
//! coupé rend un `AgentOutput` dont le `text` — *« I'm sorry, that took too
//! long »* — est **indiscernable** d'une vraie réponse, et il n'existe aucun
//! autre signal. Le filet était impossible à écrire au call-site pour cette
//! raison précise, pas par oubli.
//!
//! Ce que ce fichier ne fait PAS : parler à GitHub. Le POST est tenu par un
//! `poster` injecté qui enregistre l'appel. La frontière est assumée — au-delà,
//! on testerait `gh`, pas le contrat.
//!
//! ## Contrôle négatif
//!
//! Voir le corps de PR. Le rouge s'obtient en faisant rendre `None` à
//! `persist_deadline_fallback` pour `deadline_exceeded` — c'est-à-dire en
//! restaurant la sémantique de `main`, où rien ne distingue « coupé » de
//! « conclu ». Le test tombe alors sur `NotApplicable("turn_completed")` : zéro
//! POST, PR muette. Exactement le symptôme du ticket.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use mika_common::llm::mock::*;
use serde_json::json;
use tokio::time::Instant;

use mika_agent::server::deadline_verdict::{
    DEADLINE_VERDICT_LINE, DeadlineVerdictInput, DeadlineVerdictOutcome, PostReviewRequest,
    maybe_post_deadline_verdict,
};

use super::harness::EvalHarness;

/// Le texte que le gateway émet pour l'événement qui déclenche une revue QA —
/// la forme exacte de `pull_request.review_requested` dans
/// `mika_gateway::github::format_event_text`.
const REVIEW_REQUESTED_EVENT: &str = concat!(
    "[GitHub] PR review_requested: senara-solutions/mika#2275 — ",
    "fix(mika#2272): le reaper D1 scanne la row qui porte le pid (branch: fix/2272)\n",
    "https://github.com/senara-solutions/mika/pull/2275\n",
    "Requested reviewer: @mika-platform-qa"
);

const TRACE_ID: &str = "921f11f0acd411f18bc690c3b908c45a";
const SESSION_ID_HINT: &str = "qa-review";

/// Fait tourner un vrai tour d'agent jusqu'au dépassement d'enveloppe.
///
/// Reproduit la forme mesurée de la trace `921f11f0` : le tour appelle des
/// outils, chaque appel mange du temps, et l'enveloppe tombe avant qu'aucune
/// réponse finale ne soit rédigée. La différence avec la production est le
/// moyen (horloge virtuelle plutôt que deux `cargo test --release` de ~235 s) ;
/// la sortie du moteur est la même.
async fn run_until_deadline_exceeded() -> mika_agent::agent_loop::AgentOutput {
    let responses = vec![
        // L'appel LLM traverse la deadline (6 min virtuelles) et rend un
        // tool_call — la boucle itère, et le contrôle de deadline en tête
        // d'itération sort en `DeadlineExceeded`.
        delayed_response(
            360_000,
            tool_call_response("search_memory", json!({"query": "review the diff"})),
        ),
        // Sentinelle : jamais consommée si la deadline garde bien la porte.
        text_response("(ne doit jamais être rendu)"),
    ];

    let harness = EvalHarness::builder()
        .responses(responses)
        .build()
        .await
        .expect("harness build");

    let deadline = Instant::now() + Duration::from_secs(1);

    harness
        .run_with_deadline(REVIEW_REQUESTED_EVENT, deadline)
        .await
        .expect("agent run")
        .output
}

/// **AC2 — le contrat.** Un tour de revue coupé par son enveloppe pose un
/// verdict sur la PR.
///
/// Trois faits assertés dans l'ordre où ils comptent :
/// 1. le moteur *sait* que le tour a été coupé (`deadline_exceeded` est `Some`) ;
/// 2. le filet en tire un POST — un seul, sur la bonne PR ;
/// 3. le corps porte la ligne canonique, le motif, les steps et le `trace_id`.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn ac2_a_qa_turn_cut_off_by_its_deadline_posts_a_verdict() {
    let output = run_until_deadline_exceeded().await;

    // (1) Le signal existe. Sur `main` il n'y en avait aucun : le seul indice
    // était un `text` de fallback que rien ne distingue d'une vraie réponse.
    let overrun = output
        .deadline_exceeded
        .expect("le tour a été coupé par sa deadline — AgentOutput doit le dire");

    // Le texte de fallback part toujours sur le canal de réponse : c'est lui qui
    // notifiait Telegram pendant que la PR restait muette. Il n'est pas retiré,
    // il cesse d'être la seule chose qui se passe.
    let text = output.text.clone().expect("fallback text");
    assert!(
        text.contains("took too long"),
        "le fallback conversationnel reste inchangé, got: {text}"
    );

    // (2) + (3) Le filet, branché comme en production.
    let registry: DashMap<String, HashSet<String>> = DashMap::new();
    let captured = Arc::new(std::sync::Mutex::new(None::<PostReviewRequest>));
    let calls = Arc::new(AtomicUsize::new(0));

    let sink = captured.clone();
    let counter = calls.clone();
    let outcome = maybe_post_deadline_verdict(
        DeadlineVerdictInput {
            overrun: output.deadline_exceeded,
            event_text: REVIEW_REQUESTED_EVENT,
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
        "AC2 : le dépassement doit produire un verdict, pas un silence"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "exactement un POST — ni zéro (le défaut), ni deux (la régression)"
    );

    let req = captured.lock().unwrap().clone().expect("un POST capturé");
    assert_eq!(req.repo, "senara-solutions/mika");
    assert_eq!(req.pr_number, 2275);
    assert!(
        req.body.starts_with(DEADLINE_VERDICT_LINE),
        "la ligne canonique doit ouvrir le corps (c'est elle que `verdict_handler` \
         parse), got: {}",
        req.body
    );
    assert!(
        req.body
            .contains(&format!("{} step(s)", overrun.steps_completed)),
        "AC1 : le corps porte le nombre de steps accomplis, got: {}",
        req.body
    );
    assert!(
        req.body.contains(TRACE_ID),
        "AC1 : le corps porte le trace_id du tour, got: {}",
        req.body
    );
}

/// **AC3 sur la chaîne complète.** Le même tour coupé, mais la review avait déjà
/// été postée : zéro POST supplémentaire.
///
/// Le double-post est classé pire que le silence par la table de disposition du
/// plan — un second verdict sur une PR déjà tranchée réveille `verdict_handler`
/// sur une décision qui n'est pas la sienne.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn ac3_a_turn_that_already_posted_its_review_adds_no_second_verdict() {
    let output = run_until_deadline_exceeded().await;
    assert!(
        output.deadline_exceeded.is_some(),
        "précondition : le tour doit bien avoir été coupé"
    );

    let registry: DashMap<String, HashSet<String>> = DashMap::new();
    registry
        .entry(SESSION_ID_HINT.to_string())
        .or_default()
        .insert("senara-solutions/mika|2275".to_string());

    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let outcome = maybe_post_deadline_verdict(
        DeadlineVerdictInput {
            overrun: output.deadline_exceeded,
            event_text: REVIEW_REQUESTED_EVENT,
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

/// Contrôle négatif du contrôle : un tour qui **conclut** dans son enveloppe ne
/// déclenche rien.
///
/// Sans lui, un filet qui poserait un verdict à chaque tour passerait AC2 tout
/// en étant une régression majeure.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_turn_that_concludes_in_budget_posts_nothing() {
    let harness = EvalHarness::builder()
        // Deux réponses, et non une : le message d'origine porte le préfixe
        // `[GitHub]`, donc la garde `webhook_zero_tools` rejette un premier
        // EndTurn sans appel d'outil et re-prompte une fois. Texte anodin —
        // un `VERDICT:` réveillerait en plus les gardes de fabrication, et on
        // testerait leur comportement plutôt que le filet.
        .responses(vec![
            text_response("La revue est terminée."),
            text_response("La revue est terminée."),
        ])
        .build()
        .await
        .expect("harness build");

    let trace = harness
        .run_with_deadline(
            REVIEW_REQUESTED_EVENT,
            Instant::now() + Duration::from_secs(300),
        )
        .await
        .expect("agent run");

    assert!(
        trace.output.deadline_exceeded.is_none(),
        "un tour qui conclut ne doit jamais être marqué comme coupé"
    );

    let registry: DashMap<String, HashSet<String>> = DashMap::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let outcome = maybe_post_deadline_verdict(
        DeadlineVerdictInput {
            overrun: trace.output.deadline_exceeded,
            event_text: REVIEW_REQUESTED_EVENT,
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

    assert_eq!(
        outcome,
        DeadlineVerdictOutcome::NotApplicable("turn_completed")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

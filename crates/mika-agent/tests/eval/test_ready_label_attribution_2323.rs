//! mika#2323 — **un `labeled ready` reçu laisse toujours une trace, et cette
//! trace nomme la porte qui a décidé.**
//!
//! # Le défaut mesuré, et la prémisse qu'il faut d'abord réfuter
//!
//! Le ticket rapporte qu'un label `ready` posé à la main par `samidarko` ne
//! produit aucun dispatch, et lit l'absence de `ready_label_engine_dispatched`
//! comme la preuve d'un filtre acteur. **Il n'existe aucun filtre acteur, et il
//! ne pouvait pas en exister un** : `route_event("issues", Some("labeled"))`
//! rend `Some("mika-dev")` sans condition sur `sender`, et — surtout —
//! `format_event_text` n'émettait jamais `event.sender`, donc le handler ne
//! pouvait pas filtrer sur une identité qu'il ne recevait pas.
//!
//! Ce que le ticket a réellement mesuré n'est pas un filtre : c'est
//! l'**impossibilité d'attribuer un non-dispatch**. L'absence de
//! `ready_label_engine_dispatched` était compatible avec quatorze sorties
//! distinctes du handler — dont une totalement muette — plus quatre pertes en
//! amont qui n'écrivent rien côté agent.
//!
//! # L'invariant que ces tests nomment
//!
//! *Pour toute porte refusante, l'opérateur obtient (a) une ligne d'entrée
//! disant que l'événement est arrivé, (b) une ligne d'audit nommant la porte,
//! et (c) la ligne d'audit historique de cette porte, inchangée.*
//!
//! Le point (c) n'est pas décoratif : `ready_label_pilot_in_flight` et ses trois
//! voisines sont documentées dans `CLAUDE.md` avec leurs requêtes SQL publiées.
//! L'attribution s'**ajoute**, elle ne remplace pas (mika#2323 R6).
//!
//! # Rouge-avant
//!
//! Sur le commit précédent (enveloppeur absent), chaque assertion portant sur
//! `ready_label_outcome` échoue : la table `audit_events` ne contient que la
//! ligne historique de la porte. Recette d'injection : supprimer l'appel à
//! `emit_ready_label_outcome` dans
//! `try_handle_ready_label_dispatch_with_fetcher` — rouge de nouveau.

use std::sync::Arc;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;
use mika_agent::messaging::{MessageSender, SendOutcome};
use mika_agent::server::ready_label_handler::try_handle_ready_label_dispatch_with_fetcher;
use mika_agent::server::verdict_handler::VerdictAction;
use mika_agent::skills::SkillRegistry;

const AGENT_ID: &str = "mika";
const SESSION: &str = "eval-session";
const TRACE: &str = "eval-trace";

/// Un ticket groomé : rien d'autre que la porte testée ne peut refuser.
const GROOMED_BODY: &str = "> - **Branch:** `feat/2323/x`\n> - **Plan:** `docs/plans/2026-09-18-003-fix-2323-x-plan.md`\n\
     \n> - **Grooming history:** second-pass (GROOMED)\n\nCorps.";

/// L'événement tel que le gateway l'émet DEPUIS mika#2323 : la ligne acteur en
/// dernier. Le marqueur et le parse doivent y survivre — c'est la moitié de
/// l'axe 2 qui se vérifie ici, sur le chemin réel plutôt qu'en unité.
const READY_EVENT_WITH_ACTOR: &str = "[GitHub] Issue labeled ready on senara-solutions/mika#2323 — fix: attribution\n\
     https://github.com/senara-solutions/mika/issues/2323\n\
     Labeled by: @samidarko";

/// Le même événement tel qu'un gateway **d'avant** mika#2323 l'émet. Contrôle
/// de compatibilité ascendante : un ancien gateway servant un nouvel agent doit
/// se comporter exactement comme avant.
const READY_EVENT_NO_ACTOR: &str = "[GitHub] Issue labeled ready on senara-solutions/mika#2323 — fix: attribution\n\
     https://github.com/senara-solutions/mika/issues/2323";

struct NoopSender;

#[async_trait::async_trait]
impl MessageSender for NoopSender {
    async fn send(&self, _text: &str) -> anyhow::Result<SendOutcome> {
        Ok(SendOutcome::Delivered)
    }
}

fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory DB");
    AsyncDatabase::new_with_agent(db, AGENT_ID)
}

async fn run_handler(db: &AsyncDatabase, text: &str, labels: Vec<String>) -> VerdictAction {
    run_handler_with(db, text, Some("fake-token"), Ok(labels)).await
}

/// `fetch` carries either the labels to serve or the `gh issue view` error to
/// simulate; `token` is `None` to exercise the no-token exit.
async fn run_handler_with(
    db: &AsyncDatabase,
    text: &str,
    token: Option<&str>,
    fetch: Result<Vec<String>, String>,
) -> VerdictAction {
    let sender: Arc<dyn MessageSender> = Arc::new(NoopSender);
    let skills = SkillRegistry::empty();
    try_handle_ready_label_dispatch_with_fetcher(
        text,
        db,
        token,
        Some(&sender),
        SESSION,
        TRACE,
        &skills,
        move |_owner_repo, _number, _token| async move {
            fetch.map(|labels| (GROOMED_BODY.to_string(), labels))
        },
    )
    .await
}

/// Les `(tool_name, target_key, after_value)` écrits pendant le tour.
async fn audit_rows(db: &AsyncDatabase) -> Vec<(String, String, Option<String>)> {
    db.get_audit_events(SESSION)
        .await
        .expect("read audit events")
        .into_iter()
        .map(|e| (e.tool_name, e.target_key, e.after_value))
        .collect()
}

/// La porte nommée par la ligne `ready_label_outcome`, s'il y en a une.
async fn attributed_gate(db: &AsyncDatabase) -> Option<String> {
    audit_rows(db)
        .await
        .into_iter()
        .find(|(tool, _, _)| tool == "ready_label_outcome")
        .and_then(|(_, _, after)| after)
}

/// INVARIANT — chaque porte refusante nomme sa porte ET conserve sa ligne
/// d'audit historique.
///
/// Les quatre portes couvertes sont celles qui écrivaient déjà un audit
/// (2b/2c/4b/4c). Une cinquième colonne dirait « rien » : c'est précisément ce
/// que l'attribution ajoute, et les portes muettes sont couvertes par le test
/// suivant.
#[tokio::test]
async fn every_refusing_gate_names_itself_without_losing_its_legacy_row() {
    // (labels du ticket, porte attendue, tool_name historique à préserver)
    let cases: [(Vec<String>, &str, &str); 2] = [
        (
            vec!["ready".to_string(), "blocked".to_string()],
            "operator_held",
            "ready_label_operator_held",
        ),
        (
            vec!["ready".to_string(), "dispatch:zorglub".to_string()],
            "seat_mismatch",
            "ready_label_seat_mismatch",
        ),
    ];

    for (labels, expected_gate, legacy_tool) in cases {
        let db = test_db();
        let action = run_handler(&db, READY_EVENT_WITH_ACTOR, labels.clone()).await;

        assert_eq!(
            attributed_gate(&db).await.as_deref(),
            Some(expected_gate),
            "la porte {expected_gate} ne s'est pas nommée — l'opérateur retrouve \
             exactement l'angle mort de mika#2323 (labels={labels:?})"
        );

        let rows = audit_rows(&db).await;
        assert!(
            rows.iter().any(|(tool, _, _)| tool == legacy_tool),
            "R6 VIOLÉ : la ligne d'audit historique `{legacy_tool}` a disparu — sa \
             requête SQL est publiée dans CLAUDE.md ; l'attribution s'ajoute, elle ne \
             remplace pas. Lignes obtenues : {rows:?}"
        );

        assert!(
            matches!(action, VerdictAction::Handled { .. }),
            "R7 : la politique de dispatch ne change pas — {expected_gate} refuse \
             toujours en Handled"
        );
    }
}

/// INVARIANT — la porte 2b refuse AVANT toute création de task, et se nomme
/// quand même.
///
/// Elle est séparée des deux autres parce que son refus dépend du dépôt, pas
/// des labels : l'événement doit désigner un dépôt hors allowlist.
#[tokio::test]
async fn the_repo_allowlist_gate_names_itself_before_any_task_exists() {
    let db = test_db();
    let foreign = "[GitHub] Issue labeled ready on some-org/not-ours#7 — titre\n\
                   https://github.com/some-org/not-ours/issues/7\n\
                   Labeled by: @samidarko";

    let action = run_handler(&db, foreign, vec!["ready".to_string()]).await;

    assert_eq!(
        attributed_gate(&db).await.as_deref(),
        Some("repo_not_dispatchable"),
        "la porte 2b doit se nommer"
    );
    let rows = audit_rows(&db).await;
    assert!(
        rows.iter()
            .any(|(tool, _, _)| tool == "ready_label_repo_not_dispatchable"),
        "R6 : la ligne historique de la porte 2b reste écrite — {rows:?}"
    );
    assert!(matches!(action, VerdictAction::Handled { .. }));
}

/// INVARIANT — **les sorties qui n'écrivaient AUCUNE ligne d'audit se nomment
/// désormais.** C'est le cœur de mika#2323.
///
/// Les quatre portes ci-dessus écrivaient déjà leur propre ligne : pour elles,
/// l'attribution est un confort. Les trois ci-dessous n'écrivaient qu'un `warn!`
/// — donc rien d'interrogeable en SQL, donc aucune réponse possible à « pourquoi
/// ce ticket n'a-t-il pas dispatché ? ». Ces assertions sont **impossibles**
/// avant le correctif : il n'y avait rien à lire.
///
/// Les trois sont atteintes par leur précondition réelle, pas simulées :
/// - `no_token` — aucun jeton GitHub résolu (étape 3) ;
/// - `body_fetch_failed` — `gh issue view` échoue (étape 4) ;
/// - `tool_not_found` — l'outil de dispatch est absent du `SkillRegistry`
///   (étape 9a), ce qui est le cas nominal d'un registre vide.
#[tokio::test]
async fn the_formerly_unauditable_exits_now_name_themselves() {
    /// (jeton GitHub, résultat du `gh issue view`, porte attendue).
    type UnauditableCase = (
        Option<&'static str>,
        Result<Vec<String>, String>,
        &'static str,
    );

    let cases: [UnauditableCase; 3] = [
        (None, Ok(vec!["ready".to_string()]), "no_token"),
        (
            Some("fake-token"),
            Err("gh: HTTP 502".to_string()),
            "body_fetch_failed",
        ),
        (
            Some("fake-token"),
            Ok(vec!["ready".to_string()]),
            "tool_not_found",
        ),
    ];

    for (token, fetch, expected_gate) in cases {
        let db = test_db();
        run_handler_with(&db, READY_EVENT_WITH_ACTOR, token, fetch).await;

        assert_eq!(
            attributed_gate(&db).await.as_deref(),
            Some(expected_gate),
            "la sortie `{expected_gate}` n'écrivait qu'un warn! avant mika#2323 : sans \
             cette ligne, son absence est indiscernable d'un événement jamais reçu"
        );
    }
}

/// La ligne d'attribution est keyée sur le ticket, pas sur la task.
///
/// C'est ce qui rend `… WHERE target_key = 'senara-solutions/mika#2323'` capable
/// de répondre pour un ticket qui n'a jamais produit de task — c'est-à-dire
/// exactement la population que mika#2323 ne pouvait pas interroger.
#[tokio::test]
async fn the_attribution_row_is_keyed_on_the_issue_not_on_a_task() {
    let db = test_db();
    run_handler_with(&db, READY_EVENT_WITH_ACTOR, None, Ok(vec![])).await;

    let row = audit_rows(&db)
        .await
        .into_iter()
        .find(|(tool, _, _)| tool == "ready_label_outcome")
        .expect("une ligne d'attribution doit exister");

    assert_eq!(
        row.1, "senara-solutions/mika#2323",
        "le target_key doit être la référence de l'issue : aucune task n'existe sur \
         cette voie, et c'est précisément le cas qu'il fallait pouvoir interroger"
    );
}

/// Contrôle négatif d'axe 2 — un événement SANS ligne acteur se comporte
/// exactement comme avant.
///
/// C'est la compatibilité ascendante : un ancien gateway servant un nouvel
/// agent (ou l'inverse) ne doit rien changer à la décision.
#[tokio::test]
async fn an_event_without_an_actor_line_decides_identically() {
    let with_actor = test_db();
    let without_actor = test_db();

    let a = run_handler(
        &with_actor,
        READY_EVENT_WITH_ACTOR,
        vec!["ready".to_string(), "blocked".to_string()],
    )
    .await;
    let b = run_handler(
        &without_actor,
        READY_EVENT_NO_ACTOR,
        vec!["ready".to_string(), "blocked".to_string()],
    )
    .await;

    assert_eq!(
        attributed_gate(&with_actor).await,
        attributed_gate(&without_actor).await,
        "l'acteur est informatif : sa présence ou son absence ne peut pas changer la \
         porte qui décide (mika#2323 R4)"
    );
    assert_eq!(
        matches!(a, VerdictAction::Handled { .. }),
        matches!(b, VerdictAction::Handled { .. }),
        "ni la disposition"
    );
}

/// INVARIANT — un texte qui n'est pas un marqueur ready n'écrit RIEN.
///
/// L'observabilité qui journalise tout le monde ne distingue plus personne
/// (mika#2131 AC7) : chaque message du canal `github` traverse ce handler, et
/// une ligne par message noierait le signal que l'entrée existe pour lever.
#[tokio::test]
async fn a_non_marker_message_writes_nothing_at_all() {
    let db = test_db();
    let action = run_handler(
        &db,
        "[GitHub] Issue comment on senara-solutions/mika#2323 — bonjour",
        vec![],
    )
    .await;

    assert!(
        matches!(action, VerdictAction::Passthrough { .. }),
        "un non-marqueur reste un passthrough"
    );
    assert!(
        audit_rows(&db).await.is_empty(),
        "un message qui n'est pas un `labeled ready` ne doit écrire aucune ligne — \
         sinon chaque message du canal github en produit une"
    );
}

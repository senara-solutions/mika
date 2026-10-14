//! mika#2654 — un tour webhook PR ne force pas la promotion d'un wrapper différé.
//!
//! **Ce que la mesure dit, et ce qu'elle ne dit pas.** Fenêtre
//! 2026-09-07 → 2026-10-07 : 73 appels à `promote_deferred_callback` sur 58
//! sessions, dont **27 sessions de canal `github`** (33 appels, 10 réussis).
//! Les 10 promotions réussies depuis un tour webhook sont toutes le fait de
//! mika-qa et promeuvent toutes un wrapper de la lignée de l'événement — donc
//! **aucune promotion hors lignée n'a été mesurée**, et c'est exactement
//! pourquoi la forme 1 (borner à la lignée) a été écartée : sa population de
//! refus serait vide. Ce que le refus retire est nommé et borné : une avance
//! ≤ 60 s (`DB_SCAN_INTERVAL_TICKS`) et le contournement de la priorité
//! opérateur (`has_pending_operator_task_for_class`, mika#1948 AC3) que le
//! backstop consulte et que `force_promote_deferred_for_class` ne consulte pas.
//!
//! **Pourquoi le chemin de production et pas le prédicat.** Frère de
//! `test_cancel_task_live_pilot_2653.rs`, et pour la raison que ce fichier écrit
//! déjà : les unitaires de `classify_force_promote_turn`
//! (`tools::promote_deferred_callback::tests::mika2654`) posent
//! `ctx.is_webhook_pr_event_turn` **à la main**, donc ils restent verts si le
//! champ n'est jamais calculé ou jamais lu — et c'est très exactement le mode de
//! panne, puisque le booléen est posé à un **seul** site de production
//! (`agent_loop/mod.rs`, par `webhook_dispatch::is_webhook_pr_event_domain`) et
//! que tous les autres sites le laissent à `false`. Seul un vrai tour, dont la
//! garde tourne à sa position de production dans `PromoteDeferredCallbackTool`,
//! ferme ça. C'est la mutation M4 du plan, et ce fichier est ce qu'elle doit
//! rougir **seul**.
//!
//! **Les deux contrôles sont porteurs, pas décoratifs.** Les deux cas de refus
//! seuls seraient satisfaits par une garde qui refuse l'outil sur **tout** tour
//! — c'est-à-dire par un correctif qui retire la promotion forcée aux 40 appels
//! mesurés hors canal github. Le tour d'opérateur est l'axe qui sépare « la
//! garde décide » de « la garde bloque la promotion ». Et le tour
//! `[GitHub] Issue …` épingle que le **Webhook Fallthrough reste hors
//! périmètre, par décision** : `FALLTHROUGH_WITHHELD_TOOLS` ne retient que
//! `create_task`, et cet outil-là n'y a jamais été ajouté — sans ce cas, un
//! élargissement silencieux de la garde à tout tour `[GitHub]` passerait avec
//! les trois autres au vert.
//!
//! **Aucun garde-fou de plateforme ici.** Contrairement à la sœur mika#2653, ce
//! scénario ne demande aucun pilote vif : la garde est un booléen de classe de
//! tour et rien de plus, donc aucun `#[cfg(target_os)]` et aucun processus
//! enfant.

use mika_agent::db::NewTask;
use mika_common::llm::mock::*;
use serde_json::json;

use super::harness::EvalHarness;

/// Le préfixe stable du refus, tel que le modèle le reçoit.
const REFUSAL_PREFIX: &str = "REFUS (webhook-pr-turn, mika#2654)";

/// Le `tool_name` sous lequel la garde audite chaque refus.
const AUDIT_TOOL: &str = "deferred_dispatch_force_promote_refused_webhook_pr_turn";

/// Le nom d'audit de la promotion **réussie** (mika#1453), pour que les contrôles
/// attestent une promotion plutôt que l'absence d'un refus.
const SUCCESS_AUDIT_TOOL: &str = "deferred_dispatch_force_promote_succeeded";

/// La classe sur laquelle tout ce fichier travaille.
const CLASS: &str = "implement";

/// Un tour ouvert par une revue de PR — la famille mesurée.
const PR_REVIEW_MSG: &str = "[GitHub] PR review (changes_requested) on senara-solutions/mika#2654 (promote-deferred) by @mika-platform-qa";

/// L'autre moitié de la famille : une suite de vérifications.
const CHECK_SUITE_MSG: &str = "[GitHub] Check suite failure on senara-solutions/mika#2654 (fix/2654/promote-deferred-callback-choisit-un-cr)";

/// Hors population : un opérateur en conversation.
const OPERATOR_MSG: &str = "le créneau implement a l'air libre, promeus le wrapper différé";

/// Hors population **par décision** : le domaine Webhook Fallthrough.
const ISSUE_MSG: &str = "[GitHub] Issue labeled p1-important on senara-solutions/mika#2654 — promote_deferred_callback\nhttps://github.com/senara-solutions/mika/issues/2654\nLabeled by: @samidarko";

/// Le wrapper différé `pending` que l'outil promeut, tel que l'exécuteur l'écrit.
///
/// Le `label` est **littéral** et non un suffixe approchant :
/// `promote_next_deferred_callback_for_class` filtre sur
/// `label = 'long_running:run_claude_pilot:deferred'` par égalité, donc une
/// fixture qui s'en écarte d'un octet rend `NoPendingWrapper` et les quatre cas
/// de ce fichier deviendraient muets au lieu de rouges.
async fn seed_deferred_wrapper(harness: &EvalHarness) -> String {
    harness
        .db
        .create_task(NewTask {
            agent_id: harness.db.agent_id().to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "long_running:run_claude_pilot:deferred".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: Some(CLASS.to_string()),
        })
        .await
        .expect("create deferred wrapper row")
}

/// Construire le harness, semer le wrapper différé, puis lancer le tour.
///
/// L'ordre est contraint : le harness possède la base, donc le wrapper ne peut
/// être semé qu'après lui, et les réponses mockées sont injectées en dernier
/// (`clear_and_set`).
async fn run_on_seeded(message: &str) -> (EvalHarness, String, String) {
    let harness = EvalHarness::builder()
        .responses(Vec::new())
        .build()
        .await
        .unwrap();
    let task_id = seed_deferred_wrapper(&harness).await;

    harness.mock().clear_and_set(vec![
        tool_call_response(
            "promote_deferred_callback",
            json!({"dispatch_class": CLASS}),
        ),
        // Assez de réponses texte pour absorber le re-prompt de
        // `webhook_zero_tools` : sur un tour webhook PR l'appel est refusé, donc
        // le tour n'a aucun appel d'outil *réussi* et le garde d'intention
        // re-prompte une fois. C'est le comportement nominal du chemin, pas un
        // défaut de la fixture.
        text_response("Acknowledged."),
        text_response("Acknowledged."),
        text_response("Acknowledged."),
        text_response("Acknowledged."),
    ]);

    let trace = harness.run(message).await.unwrap();
    let calls = trace.calls_for_tool("promote_deferred_callback");
    assert!(
        !calls.is_empty(),
        "la fixture doit avoir atteint `promote_deferred_callback` — sans appel il \
         n'y a rien à affirmer"
    );
    let output = calls[0].output.clone().unwrap_or_default();
    (harness, task_id, output)
}

/// Les `after_value` de chaque ligne écrite sous un `tool_name` donné, pour la
/// session du harness.
async fn audit_rows(harness: &EvalHarness, tool_name: &str) -> Vec<Option<String>> {
    harness
        .db
        .get_audit_events(&harness.session_id)
        .await
        .expect("audit events must be readable")
        .into_iter()
        .filter(|e| e.tool_name == tool_name)
        .map(|e| e.after_value)
        .collect()
}

/// Ce que la base dit du wrapper après le tour.
///
/// `promote_next_deferred_callback_for_class` pose `status = 'completed'` sur le
/// wrapper qu'il promeut — donc `"pending"` après un refus et `"completed"`
/// après une promotion sont deux faits **distincts**, et c'est ce qui rend
/// l'assertion porteuse plutôt que tautologique.
async fn wrapper_status(harness: &EvalHarness, task_id: &str) -> String {
    harness
        .db
        .get_task(task_id)
        .await
        .expect("task readable")
        .expect("task exists")
        .status
}

// -------------------------------------------------------------------------
// AC9a (AC1) — le tour `[GitHub] PR …`, par le chemin de production.
// -------------------------------------------------------------------------

#[tokio::test]
async fn mika2654_un_tour_webhook_pr_ne_force_pas_la_promotion() {
    let (harness, task_id, output) = run_on_seeded(PR_REVIEW_MSG).await;

    assert!(
        output.starts_with(REFUSAL_PREFIX),
        "un tour ouvert par une revue de PR ne décide pas quel dispatch prend le \
         créneau '{CLASS}' — le refus doit porter son préfixe de fil, celui sur \
         lequel la sonde S1 compte ; obtenu : {output}"
    );

    // AC1 — l'assertion porteuse : rien n'a été promu. Une promotion pose
    // `completed`, donc `pending` dit que la garde a tourné AVANT
    // `force_promote_deferred_for_class`, donc avant toute écriture.
    assert_eq!(
        wrapper_status(&harness, &task_id).await,
        "pending",
        "INVARIANT VIOLÉ : le wrapper a été promu — la garde est passée après \
         l'écriture en base, ou n'a pas tourné du tout"
    );
    assert!(
        audit_rows(&harness, SUCCESS_AUDIT_TOOL).await.is_empty(),
        "aucune ligne de promotion réussie ne doit exister : le refus précède \
         l'appel base"
    );

    // AC5 — la décision est comptable, et sous le nom entier de la famille.
    assert_eq!(
        audit_rows(&harness, AUDIT_TOOL).await,
        vec![Some("refused_webhook_pr_turn".to_string())],
        "exactement une ligne d'audit, portant la valeur du format de fil sur \
         laquelle l'opérateur groupe"
    );
}

// -------------------------------------------------------------------------
// AC9b (AC1) — l'autre moitié de la famille : `[GitHub] Check suite …`.
//
// Deux préfixes, deux cas : `is_webhook_pr_event_domain` est une disjonction,
// donc un correctif qui n'en câble qu'un laisserait l'autre ouvert avec le
// premier cas au vert.
// -------------------------------------------------------------------------

#[tokio::test]
async fn mika2654_un_tour_check_suite_ne_force_pas_la_promotion() {
    let (harness, task_id, output) = run_on_seeded(CHECK_SUITE_MSG).await;

    assert!(
        output.starts_with(REFUSAL_PREFIX),
        "un tour ouvert par une suite de vérifications est dans la même \
         population que le tour de revue ; obtenu : {output}"
    );
    assert_eq!(
        wrapper_status(&harness, &task_id).await,
        "pending",
        "INVARIANT VIOLÉ : le wrapper a été promu sur un tour check-suite"
    );
    assert_eq!(
        audit_rows(&harness, AUDIT_TOOL).await,
        vec![Some("refused_webhook_pr_turn".to_string())],
    );
}

// -------------------------------------------------------------------------
// AC9c (AC3) — LE contrôle négatif : hors tour webhook PR, rien ne change.
//
// Sans lui, « la garde décide » est indistinguable de « la garde bloque toute
// promotion forcée », et les 40 appels mesurés hors canal github pourraient
// être cassés avec tous les autres tests au vert.
// -------------------------------------------------------------------------

#[tokio::test]
async fn mika2654_un_tour_de_conversation_promeut_comme_avant() {
    let (harness, task_id, output) = run_on_seeded(OPERATOR_MSG).await;

    assert!(
        !output.starts_with(REFUSAL_PREFIX),
        "INVARIANT VIOLÉ (AC3) : la garde a mordu hors de sa population ; \
         obtenu : {output}"
    );
    assert!(
        output.contains("promoted"),
        "la promotion doit aboutir exactement comme avant ; obtenu : {output}"
    );
    assert_eq!(
        wrapper_status(&harness, &task_id).await,
        "completed",
        "le wrapper promu passe à `completed` — c'est ce que la promotion fait, \
         et c'est ce qui distingue ce cas des deux refus"
    );
    assert_eq!(
        audit_rows(&harness, SUCCESS_AUDIT_TOOL).await.len(),
        1,
        "la promotion réussie écrit sa propre ligne (mika#1453), intouchée"
    );
    assert!(
        audit_rows(&harness, AUDIT_TOOL).await.is_empty(),
        "hors population : aucune ligne d'audit de la garde — c'est l'essentiel \
         du trafic, soit très exactement le churn que la doctrine mika#2131 borne, \
         et c'est ce qui rend la non-régression d'AC3 observable"
    );
}

// -------------------------------------------------------------------------
// AC9d (AC3) — le Webhook Fallthrough reste hors périmètre, PAR DÉCISION.
//
// Un tour `[GitHub] Issue …` est un tour webhook, et il passe : la garde est
// bornée à la famille PR / check-suite, pas à « tout ce qui porte `[GitHub]` ».
// `FALLTHROUGH_WITHHELD_TOOLS` ne retient que `create_task` et cet outil-là n'y
// a jamais été ajouté ; ce cas est ce qui rougit si quelqu'un élargit la borne
// en croyant la renforcer.
// -------------------------------------------------------------------------

#[tokio::test]
async fn mika2654_un_tour_fallthrough_promeut_et_le_dit() {
    let (harness, task_id, output) = run_on_seeded(ISSUE_MSG).await;

    assert!(
        !output.starts_with(REFUSAL_PREFIX),
        "INVARIANT VIOLÉ : la garde a mordu sur le domaine Fallthrough, qui est \
         hors périmètre par décision (R5 — aucun prompt ne prescrit l'outil, et \
         la retenue mika#2517 ne le nomme pas) ; obtenu : {output}"
    );
    assert_eq!(
        wrapper_status(&harness, &task_id).await,
        "completed",
        "le wrapper doit être promu sur un tour Fallthrough"
    );
    assert_eq!(
        audit_rows(&harness, SUCCESS_AUDIT_TOOL).await.len(),
        1,
        "et la promotion doit le DIRE : sans cette ligne, « promu » et « la garde \
         n'a pas tourné » rendraient les mêmes octets (mika#2205)"
    );
    assert!(
        audit_rows(&harness, AUDIT_TOOL).await.is_empty(),
        "aucune ligne de la garde hors de sa population"
    );
}

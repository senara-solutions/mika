//! Qui merge — mika#2248, AC1 à AC4.
//!
//! Le 2026-09-08, mika#2244 s'est fermée avec `mergedBy = mika-platform-qa` : le
//! relecteur a mergé sa propre approbation. Ce n'était pas un mauvais verdict,
//! c'était un mauvais **acteur** — `check_suite.completed(success)` atteint
//! `mika-dev` (primaire) et `mika-qa` (secondaire, mika#1711), les deux
//! exécutaient le même `ci_success_handler`, et le `gh pr merge` posé là tournait
//! sous le token de celui qui gagnait la course.
//!
//! Ce fichier épingle les quatre énoncés du correctif. Deux sont structurels
//! (l'évaluateur ne merge plus ; l'acteur est câblé là où il lit le signal) et
//! deux sont comportementaux (le relecteur tient ; une PR décision-core reste à
//! l'opérateur). Les structurels sont là parce que le défaut d'origine était une
//! question de *callsite* : un merge qui tournait au mauvais endroit, pas un
//! calcul qui rendait la mauvaise valeur — et un callsite se mesure sur la source.

use anyhow::Result;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;
use mika_agent::perimeter::{Classification, classify_pr_files};
use mika_agent::server::merge_ready_handler::try_handle_merge_ready;
use mika_agent::server::verdict_handler::VerdictAction;
use mika_common::forge_identity::{
    DISPATCHER_AGENT, MergeDisposition, MergeReadySignal, REVIEWER_AGENT, REVIEWER_FORGE_LOGIN,
    merge_disposition, would_merge_as_reviewer,
};

/// Sources épinglées à la compilation : les assertions structurelles ci-dessous
/// ne peuvent pas passer contre une copie périmée sur le disque.
const CI_SUCCESS_SRC: &str = include_str!("../../src/server/ci_success_handler.rs");
const MERGE_READY_SRC: &str = include_str!("../../src/server/merge_ready_handler.rs");
const HANDLERS_SRC: &str = include_str!("../../src/server/handlers.rs");

fn signal_for(pr_number: u64, reviewer_login: &str) -> MergeReadySignal {
    MergeReadySignal {
        repo: "senara-solutions/mika".to_string(),
        pr_number,
        head_sha: "deadbeefcafe1234".to_string(),
        branch: "fix/2248/merge-under-dispatcher-identity-not-reviewer".to_string(),
        task_id: "task-2248".to_string(),
        reviewer_login: reviewer_login.to_string(),
    }
}

fn db_for_agent(agent_id: &str, session_id: &str) -> AsyncDatabase {
    let db = Database::open_in_memory().expect("in-memory db");
    db.register_agent(agent_id, agent_id, "")
        .expect("register agent");
    db.create_session(session_id, agent_id, "github")
        .expect("create session");
    AsyncDatabase::new_with_agent(db, agent_id)
}

// ---------------------------------------------------------------------------
// AC3 — l'évaluateur n'est plus un acteur
// ---------------------------------------------------------------------------

#[test]
fn ac3_ci_success_handler_ne_contient_plus_aucun_appel_de_merge() {
    assert!(
        !CI_SUCCESS_SRC.contains("run_gh_merge("),
        "`ci_success_handler` ne doit plus contenir d'appel `run_gh_merge(` : c'est le \
         callsite qui tournait sous l'identité de celui qui gagnait la course \
         (mika#2244). Il évalue et signale ; le merge appartient à l'acteur."
    );
    assert!(
        CI_SUCCESS_SRC.contains("MergeReadySignal"),
        "l'évaluateur doit émettre un signal merge-ready — sinon il ne merge plus ET \
         personne ne merge, et la boucle s'arrête au vert"
    );
    assert!(
        CI_SUCCESS_SRC.contains("\"ci_success_merge_ready\""),
        "l'émission du signal doit laisser une ligne d'audit greppable"
    );
}

#[test]
fn ac3_lacteur_autorise_avant_de_merger() {
    let authorize_at = MERGE_READY_SRC
        .find("if let Err(reason) = authorize_merge(")
        .expect("l'acteur doit consulter `authorize_merge`");
    let merge_at = MERGE_READY_SRC
        .find("run_gh_merge(")
        .expect("l'acteur est le callsite de merge de ce chemin");
    let perimeter_at = MERGE_READY_SRC
        .find("perimeter::classify_pr_files")
        .expect("l'acteur doit revérifier le périmètre lui-même");
    let fail_closed_at = MERGE_READY_SRC
        .find("verdict: Classification::DecisionCore")
        .expect("l'échec de fetch du périmètre doit tomber fermé");

    assert!(
        authorize_at < merge_at,
        "la porte d'identité doit précéder le merge — l'inverse est mika#2244 verbatim"
    );
    assert!(
        perimeter_at < merge_at && fail_closed_at < merge_at,
        "la porte périmètre (et sa clause fail-closed) doit précéder le merge"
    );
}

#[test]
fn ac3_lacteur_est_cable_la_ou_il_lit_le_signal() {
    // Le handoff passe par `req.text` : l'évaluateur y écrit son pré-digest,
    // l'acteur l'y relit. Un handler inséré entre les deux réécrirait le texte
    // et le signal disparaîtrait sans qu'aucun test de comportement ne bouge.
    let evaluator_at = HANDLERS_SRC
        .find("ci_success_handler::try_handle_ci_success(")
        .expect("l'évaluateur doit être câblé dans la chaîne github");
    let actor_at = HANDLERS_SRC
        .find("merge_ready_handler::try_handle_merge_ready(")
        .expect("l'acteur doit être câblé dans la chaîne github");
    let next_handler_at = HANDLERS_SRC
        .find("ci_failure_handler::try_handle_ci_failure(")
        .expect("le handler d'échec CI suit dans la chaîne");

    assert!(
        evaluator_at < actor_at,
        "l'acteur doit être câblé APRÈS l'évaluateur : il lit le signal que celui-ci écrit"
    );
    assert!(
        actor_at < next_handler_at,
        "aucun handler ne doit s'intercaler entre l'évaluateur et l'acteur — il réécrirait \
         `req.text` et emporterait le signal avec lui"
    );
}

// ---------------------------------------------------------------------------
// AC1 + AC2 — `mergedBy` n'est jamais le relecteur
// ---------------------------------------------------------------------------

#[test]
fn ac2_la_forme_mesuree_sur_mika2244_est_reconnue_et_refusee() {
    // Contrôle positif : l'agent qui tourne est le relecteur nommé dans le signal.
    assert!(
        would_merge_as_reviewer(REVIEWER_AGENT, REVIEWER_FORGE_LOGIN),
        "la forme de mika#2244 doit être reconnue : merger sous le login qui a approuvé"
    );
    assert_eq!(
        merge_disposition(REVIEWER_AGENT),
        MergeDisposition::SignalOnly,
        "le relecteur signale, il n'agit pas"
    );
    // Contrôle négatif, dans le même test : sans lui, une politique qui refuse
    // tout le monde passerait, et le merge autonome mourrait en silence.
    assert!(!would_merge_as_reviewer(
        DISPATCHER_AGENT,
        REVIEWER_FORGE_LOGIN
    ));
    assert_eq!(merge_disposition(DISPATCHER_AGENT), MergeDisposition::Act);
}

#[tokio::test]
async fn ac2_le_relecteur_ne_merge_pas_et_le_refus_est_dans_laudit() -> Result<()> {
    // Bout en bout sur la moitié observable sans forge : signal valide, token
    // présent, agent relecteur. Le handler doit tenir AVANT le premier appel
    // `gh`, donc sans jamais pouvoir écrire `mergedBy`.
    let db = db_for_agent(REVIEWER_AGENT, "s-ac2");
    let signal = signal_for(2244, REVIEWER_FORGE_LOGIN);
    let text = format!("<ci_success_handler>\n{signal}\n</ci_success_handler>");

    let action =
        try_handle_merge_ready(&text, &db, Some("fake-token"), None, "s-ac2", "trace-ac2").await;

    assert!(
        matches!(action, VerdictAction::Passthrough { enrichment: None }),
        "le relecteur doit tenir sans agir, got {action:?}"
    );
    assert_eq!(
        db.count_audit_events_by_tool_name("ci_success_merge")
            .await?,
        0,
        "aucun merge ne doit être initié sous l'identité du relecteur (AC1)"
    );
    assert_eq!(
        db.count_audit_events_by_tool_name("merge_ready_hold_reviewer_is_not_merge_actor")
            .await?,
        1,
        "le refus doit être greppable : AC1 doit se mesurer dans l'audit, pas seulement sur la forge"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// AC4 — contrôle négatif : une PR décision-core reste tenue pour l'humain
// ---------------------------------------------------------------------------

#[test]
fn ac4_la_surface_de_ce_correctif_est_elle_meme_decision_core() {
    // La porte ne déplace pas la frontière : les fichiers touchés ici restent
    // décision-core, donc cette PR même est human-gated. C'est voulu.
    let files = vec![
        "crates/mika-agent/src/server/ci_success_handler.rs".to_string(),
        "crates/mika-agent/src/server/merge_ready_handler.rs".to_string(),
        "crates/mika-agent/src/tools/pr_merge_with_gate.rs".to_string(),
    ];
    assert_eq!(
        classify_pr_files(&files).verdict,
        Classification::DecisionCore,
        "la surface du merge autonome doit rester décision-core"
    );
}

#[tokio::test]
async fn ac4_le_dispatcher_tient_quand_le_perimetre_ne_peut_pas_etre_etabli() -> Result<()> {
    // L'agent EST le dispatcher et le signal est valide : les deux conditions
    // d'identité sont remplies. Seul le périmètre manque — `gh` ne répond pas
    // sous un token factice — et cela doit suffire à tenir la PR. C'est la
    // direction fail-closed d'AC4 : l'acteur répond de ce qu'il écrit sur la
    // forge, même quand le signal dit que les portes étaient vertes.
    let db = db_for_agent(DISPATCHER_AGENT, "s-ac4");
    let signal = signal_for(2248, REVIEWER_FORGE_LOGIN);
    let text = format!("<ci_success_handler>\n{signal}\n</ci_success_handler>");

    let action =
        try_handle_merge_ready(&text, &db, Some("fake-token"), None, "s-ac4", "trace-ac4").await;

    match action {
        VerdictAction::Handled { pre_digest } => {
            assert!(
                pre_digest.contains("DECISION-CORE")
                    && pre_digest.contains("Operator must merge manually"),
                "un périmètre indéterminé doit router vers l'opérateur : {pre_digest}"
            );
        }
        other => panic!("le dispatcher doit tenir explicitement, pas passer en silence: {other:?}"),
    }
    assert_eq!(
        db.count_audit_events_by_tool_name("ci_success_merge")
            .await?,
        0,
        "aucun merge sur un périmètre indéterminé"
    );
    assert_eq!(
        db.count_audit_events_by_tool_name("merge_ready_human_gate_required")
            .await?,
        1,
        "la tenue pour l'opérateur doit laisser sa propre ligne d'audit"
    );
    Ok(())
}

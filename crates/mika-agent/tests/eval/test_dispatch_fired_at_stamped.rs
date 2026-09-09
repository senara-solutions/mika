//! Négatif (b) — mika#2263 : **un dispatch dont le process tourne n'est jamais
//! comptablement « jamais firé ».**
//!
//! Classe mesurée le 2026-09-09 : les rows `b429a658` (#2252) et `4e867d85`
//! (#2212) étaient `pending` avec `fired_at` VIDE — l'état « pas encore
//! dispatché » — alors qu'un pilote bwrap tournait réellement dessus. Toute
//! sonde qui lit `fired_at` pour séparer *non-dispatché* de *zombie* (le
//! faucheur stuck-pending, `mika tasks`, la colonne du dashboard) voyait ces
//! rows comme non-dispatchées et passait à côté. Le zombie était invisible à
//! cette classe entière de sonde.
//!
//! Le point de passage obligé est `set_task_process_id(id, Some(pid))` : c'est
//! le seul endroit où le moteur apprend qu'un process vit sous une task
//! (`skills/executor.rs`, juste après le `spawn` du pilote). Stamper là couvre
//! tous les chemins de spawn d'un coup, plutôt que de compter sur la
//! discipline de chaque appelant.
//!
//! # Rouge-avant (porte #2264)
//!
//! Sur `main`, `set_task_process_id` n'écrit que `process_id` :
//! [`recording_a_live_pilot_stamps_fired_at`] échoue sur `fired_at.is_some()`.
//! Recette d'injection : retirer la branche `CASE WHEN ... fired_at IS NULL`
//! de l'UPDATE — le test redevient rouge, ses deux contrôles négatifs restent
//! verts.

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{Database, NewTask};

const AGENT_ID: &str = "mika";

fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory DB");
    AsyncDatabase::new_with_agent(db, AGENT_ID)
}

/// La forme exacte d'une row de dispatch pré-créée par `ready_label_handler` :
/// `pending`, `fired_at` NULL, aucun process encore enregistré.
async fn seed_pending_dispatch(db: &AsyncDatabase, reference_url: &str) -> String {
    db.create_task(NewTask {
        agent_id: AGENT_ID.to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: format!("ready-label: {reference_url}"),
        trigger_type: "manual".to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: None,
        timeout_at: None,
        action_type: "none".to_string(),
        action_config: "{}".to_string(),
        input_context: None,
        created_by_session: None,
        created_trace_id: None,
        reference_url: Some(reference_url.to_string()),
        source: Some("self_dev".to_string()),
        metadata: None,
        r#type: Some("issue".to_string()),
        dispatch_class: Some("implement".to_string()),
    })
    .await
    .expect("create pending dispatch row")
}

/// INVARIANT : enregistrer le process d'un pilote stampe `fired_at`. Un
/// dispatch vivant ne peut pas se lire « jamais firé ».
#[tokio::test]
async fn recording_a_live_pilot_stamps_fired_at() {
    let db = test_db();
    let id =
        seed_pending_dispatch(&db, "https://github.com/senara-solutions/mika/issues/2252").await;

    let before = db.get_task(&id).await.unwrap().unwrap();
    assert!(
        before.fired_at.is_none(),
        "contrôle positif : la row pré-créée part bien sans fired_at"
    );

    db.set_task_process_id(&id, Some(4_014_133))
        .await
        .expect("record pilot pid");

    let after = db.get_task(&id).await.unwrap().unwrap();
    assert!(
        after.fired_at.is_some(),
        "INVARIANT VIOLÉ : un pilote tourne sous cette task et elle se lit \
         encore 'pending fired_at=null' — le fantôme comptable de mika#2263"
    );
    assert_eq!(
        after.process_id,
        Some(4_014_133),
        "le pgid reste enregistré"
    );
}

/// Contrôle négatif 1 : EFFACER le pgid (ce que fait toute disposition après
/// un kill) ne stampe rien. Sans lui, un `fired_at = now()` inconditionnel
/// passerait le test ci-dessus tout en mentant sur des rows jamais dispatchées.
#[tokio::test]
async fn clearing_the_pid_never_stamps_fired_at() {
    let db = test_db();
    let id =
        seed_pending_dispatch(&db, "https://github.com/senara-solutions/mika/issues/2212").await;

    db.set_task_process_id(&id, None)
        .await
        .expect("clear pid on a never-dispatched row");

    let after = db.get_task(&id).await.unwrap().unwrap();
    assert!(
        after.fired_at.is_none(),
        "contrôle négatif : effacer un pgid n'est pas un dispatch"
    );
}

/// Contrôle négatif 2 : le stamp est idempotent — un ré-enregistrement de pgid
/// (retry de spawn) ne réécrit PAS l'heure de tir d'origine, sinon l'âge d'un
/// dispatch se remettrait à zéro à chaque écriture et tout faucheur fondé sur
/// cet âge perdrait sa prise.
#[tokio::test]
async fn re_recording_a_pid_does_not_move_the_original_fired_at() {
    let db = test_db();
    let id =
        seed_pending_dispatch(&db, "https://github.com/senara-solutions/mika/issues/2263").await;

    db.set_task_process_id(&id, Some(29_905)).await.unwrap();
    let first = db.get_task(&id).await.unwrap().unwrap().fired_at;
    assert!(first.is_some(), "premier enregistrement : fired_at stampé");

    db.set_task_process_id(&id, Some(29_906)).await.unwrap();
    let second = db.get_task(&id).await.unwrap().unwrap().fired_at;

    assert_eq!(
        first, second,
        "contrôle négatif : le fired_at d'origine ne bouge pas au ré-enregistrement"
    );
}

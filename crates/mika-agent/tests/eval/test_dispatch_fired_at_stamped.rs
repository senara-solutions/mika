//! Négatif (b) — mika#2263, **réécrit sur la topologie de production** par
//! mika#2335 : *une ligne de tracking parent dont le dispatch est parti porte
//! `fired_at`.*
//!
//! # Ce que la fixture précédente attestait, et pourquoi elle est remplacée
//!
//! La version mika#2263 semait bien la forme **parent**, puis appelait
//! `set_task_process_id` **directement dessus** — un appel que la production ne
//! fait jamais sur cette row. `set_task_process_id` est le seul écrivain de
//! `process_id`, et son unique appelant de production (`skills/executor.rs`,
//! juste après le spawn) l'appelle sur la row **enfant**. Le test validait donc
//! le `CASE` SQL, pendant que l'invariant qu'il énonçait restait faux en
//! production : le parent — la row que lisent `mika tasks`, le dashboard et les
//! sondes de santé — passait par `update_manual_task_status`, qui n'écrit que
//! `status`, `updated_at` et `completed_at`.
//!
//! Conséquence mesurée le 2026-09-15 : un opérateur a lu `status=pending,
//! fired_at=null` sur un dispatch dont le pilote écrivait des fichiers depuis
//! 38 minutes, en a conclu « orphelin inerte », et l'a annulé.
//!
//! # Ce que ce fichier mesure, et ce qu'il assère structurellement
//!
//! - **Mesuré** (comportemental) : `mark_parent_dispatched` stampe, ne réécrit
//!   jamais un `fired_at` existant, et `rewind`'s writer n'en pose aucun.
//! - **Asséré structurellement** : les **trois** chemins de dispatch de
//!   production passent par cet écrivain. Un test comportemental ne peut pas
//!   couvrir cette moitié sans monter un webhook GitHub complet pour
//!   `ready_label_handler` et un verdict de revue pour `verdict_handler` ; et
//!   c'est précisément la moitié qui compte, puisque **le chemin de l'incident
//!   n'est pas l'original** : `094fc5f6` est un dispatch ready-label. Un
//!   correctif posé sur le seul `executor.rs` aurait été vert ici et inopérant
//!   sur le cas fondateur. Le pendant négatif de cette assertion vit dans
//!   `db::tests::mika2335_no_production_dispatch_transitions_a_parent_without_stamping`.
//!
//! # Rouge-avant (porte #2264)
//!
//! Sur le code d'avant, `mark_parent_dispatched` n'existe pas : le fichier ne
//! compile pas, ce qui est la forme la plus franche du rouge. Recette
//! d'injection sur cette branche : retirer la clause
//! `fired_at = CASE WHEN fired_at IS NULL …` de `mark_parent_dispatched` — le
//! premier test redevient rouge, ses contrôles négatifs restent verts. Pour la
//! moitié structurelle : remettre `update_manual_task_status(&task_id,
//! "in_progress")` à l'un des trois sites — `every_production_dispatch_path_stamps`
//! rougit en nommant le site.

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{Database, NewTask};

const AGENT_ID: &str = "mika";

fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory DB");
    AsyncDatabase::new_with_agent(db, AGENT_ID)
}

/// La forme exacte d'une row de tracking parent pré-créée par
/// `ready_label_handler` : `manual`, `action_type='none'`, `pending`,
/// `fired_at` NULL, jamais de `process_id`.
async fn seed_parent_tracking_row(db: &AsyncDatabase, reference_url: &str) -> String {
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
    .expect("create parent tracking row")
}

/// INVARIANT : dispatcher un parent le stampe. Une row de tracking dont le
/// pilote tourne ne peut pas se lire « jamais firée ».
#[tokio::test]
async fn dispatching_a_parent_stamps_fired_at() {
    let db = test_db();
    let id =
        seed_parent_tracking_row(&db, "https://github.com/senara-solutions/mika/issues/2334").await;

    let before = db.get_task(&id).await.unwrap().unwrap();
    assert!(
        before.fired_at.is_none(),
        "contrôle positif : la row pré-créée part bien sans fired_at"
    );
    assert_eq!(before.status, "pending");

    db.mark_parent_dispatched(&id)
        .await
        .expect("mark parent dispatched");

    let after = db.get_task(&id).await.unwrap().unwrap();
    assert_eq!(
        after.status, "in_progress",
        "la transition #525 est conservée"
    );
    assert!(
        after.fired_at.is_some(),
        "INVARIANT VIOLÉ : un pilote part sous cette task et elle se lit encore \
         'fired_at=null' — le signal trompeur de mika#2335"
    );
    assert!(
        after.process_id.is_none(),
        "un parent ne porte JAMAIS de pgid : le pgid vit sur l'enfant callback"
    );
}

/// Contrôle négatif 1 : le stamp est idempotent. Un second passage (retry de
/// dispatch sur une row déjà `in_progress`) ne déplace pas l'heure de tir
/// d'origine, sinon l'âge d'un dispatch se remettrait à zéro sous les faucheurs
/// qui le mesurent.
#[tokio::test]
async fn re_dispatching_does_not_move_the_original_fired_at() {
    let db = test_db();
    let id =
        seed_parent_tracking_row(&db, "https://github.com/senara-solutions/mika/issues/2263").await;

    db.mark_parent_dispatched(&id).await.unwrap();
    let first = db.get_task(&id).await.unwrap().unwrap().fired_at;
    assert!(first.is_some(), "premier dispatch : fired_at stampé");

    db.mark_parent_dispatched(&id).await.unwrap();
    let second = db.get_task(&id).await.unwrap().unwrap().fired_at;

    assert_eq!(
        first, second,
        "contrôle négatif : le fired_at d'origine ne bouge pas au re-dispatch"
    );
}

/// Contrôle négatif 2 : le chemin `rewind` ne stampe rien. Il *restaure un
/// statut antérieur*, ce qui n'est pas un dispatch — et c'est la raison pour
/// laquelle le stamp est un écrivain nommé plutôt qu'un `CASE` ajouté à
/// `update_manual_task_status`, que `rewind.rs` partage.
#[tokio::test]
async fn restoring_a_status_never_stamps_fired_at() {
    let db = test_db();
    let id =
        seed_parent_tracking_row(&db, "https://github.com/senara-solutions/mika/issues/2212").await;

    db.update_manual_task_status(&id, "in_progress")
        .await
        .expect("restore a prior status, the rewind shape");

    let after = db.get_task(&id).await.unwrap().unwrap();
    assert_eq!(after.status, "in_progress");
    assert!(
        after.fired_at.is_none(),
        "contrôle négatif : restaurer un statut n'est pas dispatcher"
    );
}

/// INVARIANT (mika#2335, revue) : **un dispatch ne ressuscite pas un parent
/// qu'une supersession concurrente vient d'annuler.**
///
/// Les deux sites secondaires observent le statut de la ligne, puis créent
/// l'enfant callback et vérifient le script du handler avant de stamper. Une
/// supersession pour la même `reference_url` tourne en tête du même handler :
/// c'est un événement prévu, pas une hypothèse. Sans garde, le dispatch
/// remettrait `in_progress` par-dessus l'annulation et deux dispatches vivants
/// se retrouveraient sur un même ticket — par la comptabilité cette fois, pas
/// par le kill manquant.
///
/// Rouge-avant : retirer `AND status IN ('pending','in_progress')` de
/// `mark_parent_dispatched`.
#[tokio::test]
async fn dispatching_never_resurrects_a_parent_cancelled_meanwhile() {
    let db = test_db();
    let id =
        seed_parent_tracking_row(&db, "https://github.com/senara-solutions/mika/issues/2340").await;

    // Ce qu'une supersession concurrente laisse derrière elle.
    db.update_manual_task_status(&id, "cancelled")
        .await
        .expect("a concurrent supersession cancels the parent");

    db.mark_parent_dispatched(&id)
        .await
        .expect("the stamp is non-fatal and must not error");

    let after = db.get_task(&id).await.unwrap().unwrap();
    assert_eq!(
        after.status, "cancelled",
        "INVARIANT VIOLÉ : le dispatch a ressuscité un parent annulé"
    );
    assert!(
        after.fired_at.is_none(),
        "un parent annulé n'a pas été dispatché : rien à stamper"
    );
}

/// Un cas par site de dispatch de production (AC4). Assertion **structurelle**
/// — voir l'en-tête du fichier pour pourquoi cette moitié ne peut pas être
/// comportementale, et pourquoi elle est celle qui décide du cas fondateur.
///
/// La frontière production/test est lue par [`mika_common::source_guard`]
/// (mika#2398). La troncature au premier `#[cfg(test)]` qu'elle appliquait
/// était, ici, un **faux positif** et non une cécité : le prédicat est un
/// `contains` positif, donc élargir la zone lue ne peut que rendre la garde
/// plus verte — jamais plus rouge. Elle bascule quand même, parce que U4 refuse
/// la réimplémentation et non la cécité : un site laissé en place rougirait la
/// garde anti-récidive.
#[test]
fn every_production_dispatch_path_stamps() {
    let scanner = mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
    // (fichier, fonction qui dispatche) — le recensement exhaustif des
    // appelants de production de la transition parente.
    for (rel, marker) in [
        ("skills/executor.rs", "execute_long_running"),
        ("server/ready_label_handler.rs", "spawn_long_running_exec"),
        ("server/verdict_handler.rs", "spawn_long_running_exec"),
    ] {
        let production = scanner.production_of(&scanner.src_root().join(rel));
        assert!(
            production.contains(marker),
            "{rel} ne contient plus {marker} — le recensement des chemins de \
             dispatch a changé, ce test doit être remis à jour AVANT de \
             conclure quoi que ce soit sur fired_at"
        );
        assert!(
            production.contains("mark_parent_dispatched"),
            "AC4 mika#2335 : le chemin de dispatch de {rel} ne stampe pas \
             `fired_at` sur son parent. Le chemin ready-label est celui de \
             l'incident du 2026-09-15 — un correctif qui l'oublie est vert et \
             inopérant."
        );
    }
}

//! Négatif (a) — mika#2263, **réécrit sur la topologie de production** par
//! mika#2335 : *une supersession ne laisse jamais vivant le pilote de l'enfant.*
//!
//! # Ce que la fixture précédente attestait, et pourquoi elle est remplacée
//!
//! La version mika#2263 de ce fichier semait **une chimère** : une seule row
//! portant à la fois `trigger_type: "callback"`, `reference_url: Some(url)`,
//! `process_id` et `parent_task_id: None`. Cette forme satisfait la conjonction
//! SQL de `find_live_dispatch_rows_by_reference_url_and_variants`
//! (`process_id IS NOT NULL AND reference_url IN (…)`) — et **la production ne
//! l'écrit jamais**. Un dispatch, c'est deux rows :
//!
//! | row | `trigger_type` | `reference_url` | `process_id` |
//! |---|---|---|---|
//! | **parent** (tracking) | `manual` / `action_type='none'` | **oui** | **jamais** |
//! | **enfant** (callback) | `callback` / `resume_agent` | **non** | **oui** (le pgid) |
//!
//! L'URL est sur le parent, le pgid sur l'enfant, rien ne porte les deux : la
//! conjonction était **vide en production**, donc le correctif mika#2263 était
//! inopérant depuis sa livraison — et son test vert. Mesuré le 2026-09-15 : le
//! log du pilote survivant `590a06c0` ne contient **aucune** occurrence de
//! `SIGTERM`, `CANCELLED_BY`, `superseded` ou `Killed`. La supersession ne l'a
//! pas raté de peu, elle ne l'a jamais vu.
//!
//! La fixture n'est donc ni `#[ignore]`, ni supprimée, ni adaptée au nouveau
//! code : elle est **réécrite sur la forme que la production écrit** — même
//! remède que mika#2272 (« zéro était l'absence de mesure, pas la présence de
//! prudence »).
//!
//! # Rouge-avant (porte #2264)
//!
//! Sur le code d'avant, [`live_pilot_of_the_child_is_killed_by_supersede`]
//! échoue sur `!is_alive(pid)` : la traversée `parent_task_id` n'existait pas
//! dans le chemin de supersession, et le résolveur par `reference_url` ne
//! pouvait rendre aucune row de cette topologie. Recette d'injection sur cette
//! branche : retirer l'appel à `dispose_superseded_dispatch_processes` dans
//! `supersede_prior_tracking_rows`, ou remplacer son argument `&candidates` par
//! `&[]` — le test redevient rouge et les trois contrôles négatifs restent
//! verts.

use std::process::Command;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{Database, NewTask};
use mika_agent::tracking_cleanup::{
    SUPERSEDED_DISPATCH_PROCESS_KILLED_TOOL, supersede_prior_tracking_rows,
};

const AGENT_ID: &str = "mika";
const SESSION: &str = "eval-session";
const TRACE: &str = "eval-trace";

fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory DB");
    AsyncDatabase::new_with_agent(db, AGENT_ID)
}

/// Un enfant réel, chef de son propre groupe de process — la forme exacte que
/// `dispatch-lib` donne à un pilote (le kill vise le pgid). Le handle est
/// attendu dans un thread pour que le process mort soit *récolté*, sinon
/// `/proc/<pid>/stat` survit en zombie et une sonde de vivacité mentirait.
///
/// **Deux précautions que la version mika#2263 n'avait pas, et qui se paient
/// au premier échec.** (1) Sa sortie standard est `/dev/null` : héritée, elle
/// tient ouvert le pipe de `cargo test`, qui attend alors la fin du `sleep`.
/// (2) Le PID est rendu dans un [`ChildGuard`] qui signale à la destruction —
/// un `kill_pid` en fin de test n'est jamais atteint quand une assertion
/// panique, et c'est exactement le moment où le nettoyage compte. Mesuré en
/// posant le rouge-avant de ce fichier : l'échec attendu a immobilisé la suite
/// **dix minutes**, la durée du `sleep`.
fn spawn_live_child() -> (ChildGuard, u64) {
    use std::os::unix::process::CommandExt;
    let mut child = Command::new("sleep")
        .arg("600")
        .process_group(0)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn sleep");
    let pid = i64::from(child.id());
    let start_time = mika_agent::task_engine::process_liveness::read_process_start_time(child.id())
        .expect("read child start time");
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    (ChildGuard(pid), start_time)
}

/// Tue son groupe de process à la destruction, échec du test compris.
struct ChildGuard(i64);

impl ChildGuard {
    fn pid(&self) -> i64 {
        self.0
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .arg("-9")
            .arg(format!("-{}", self.0))
            .output();
        let _ = Command::new("kill")
            .arg("-9")
            .arg(self.0.to_string())
            .output();
    }
}

fn is_alive(pid: i64) -> bool {
    std::path::PathBuf::from(format!("/proc/{pid}/stat")).exists()
}

/// La row **parent** : ce que `ready_label_handler` pré-crée. Elle porte l'URL
/// de l'issue et **jamais** de `process_id`.
async fn seed_parent_tracking_row(db: &AsyncDatabase, reference_url: &str) -> String {
    let id = db
        .create_task(NewTask {
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
            created_by_session: Some(SESSION.to_string()),
            created_trace_id: None,
            reference_url: Some(reference_url.to_string()),
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: Some("issue".to_string()),
            dispatch_class: Some("implement".to_string()),
        })
        .await
        .expect("create parent tracking row");
    // L'état d'un parent dont le dispatch est parti (mika#2335 F2a).
    db.mark_parent_dispatched(&id)
        .await
        .expect("mark parent dispatched");
    id
}

/// La row **enfant** : ce que `build_callback_task` + `set_task_process_id`
/// écrivent. Elle porte le pgid et **jamais** de `reference_url`.
async fn seed_dispatch_child(
    db: &AsyncDatabase,
    parent_id: &str,
    status: &str,
    pid: i64,
    start_time: Option<u64>,
) -> String {
    let id = db
        .create_task(NewTask {
            agent_id: AGENT_ID.to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.to_string()),
            depth: 1,
            label: "long_running:run_claude_pilot".to_string(),
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
            created_by_session: Some(SESSION.to_string()),
            created_trace_id: None,
            reference_url: None,
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: Some("implement".to_string()),
        })
        .await
        .expect("create dispatch child");
    if status != "pending" {
        db.update_task_status(&id, status)
            .await
            .expect("set child status");
    }
    db.set_task_process_id(&id, Some(pid))
        .await
        .expect("record pgid");
    if let Some(st) = start_time {
        db.set_task_metadata_field(&id, "process_start_time", &st.to_string())
            .await
            .expect("record start time");
    }
    id
}

/// INVARIANT : une supersession ne laisse jamais vivant le pilote de l'enfant.
/// Le process meurt, la row enfant devient terminale et rend son pgid, la row
/// parent est annulée, et l'événement d'audit est écrit.
#[tokio::test]
#[cfg(target_os = "linux")]
async fn live_pilot_of_the_child_is_killed_by_supersede() {
    let db = test_db();
    let url = "https://github.com/senara-solutions/mika/issues/2334";
    let (guard, start_time) = spawn_live_child();
    let pid = guard.pid();

    let parent_id = seed_parent_tracking_row(&db, url).await;
    let child_id = seed_dispatch_child(&db, &parent_id, "pending", pid, Some(start_time)).await;

    assert!(is_alive(pid), "contrôle positif : l'enfant tourne avant");

    supersede_prior_tracking_rows(
        &db,
        SESSION,
        Some(TRACE),
        url,
        "ready-label: senara-solutions/mika#2334",
    )
    .await;

    assert!(
        !is_alive(pid),
        "INVARIANT VIOLÉ : la supersession a annulé le parent et laissé le \
         pilote de l'enfant (pgid {pid}) vivant — c'est le 590a06c0 de \
         mika#2335, deux pilotes sur un même worktree"
    );

    let child = db.get_task(&child_id).await.unwrap().unwrap();
    assert_eq!(
        child.status, "cancelled",
        "la row enfant doit être terminale"
    );
    assert!(
        child.process_id.is_none(),
        "le pgid doit être effacé après le kill, sinon un faucheur retente un \
         PID potentiellement recyclé"
    );

    let parent = db.get_task(&parent_id).await.unwrap().unwrap();
    assert_eq!(
        parent.status, "cancelled",
        "la row parent reste annulée par le chemin mika#1934"
    );

    let audits = db
        .count_audit_events_by_tool_name(SUPERSEDED_DISPATCH_PROCESS_KILLED_TOOL)
        .await
        .unwrap();
    assert_eq!(audits, 1, "un kill de pilote = un événement d'audit");
}

/// Contrôle négatif (a) : un dispatch vivant sur **une autre** issue survit.
/// Sans lui, un `kill` aveugle passerait le test ci-dessus.
#[tokio::test]
#[cfg(target_os = "linux")]
async fn live_pilot_on_another_issue_survives() {
    let db = test_db();
    let other_url = "https://github.com/senara-solutions/mika/issues/2212";
    let (guard, start_time) = spawn_live_child();
    let pid = guard.pid();

    let other_parent = seed_parent_tracking_row(&db, other_url).await;
    let other_child =
        seed_dispatch_child(&db, &other_parent, "pending", pid, Some(start_time)).await;

    supersede_prior_tracking_rows(
        &db,
        SESSION,
        Some(TRACE),
        "https://github.com/senara-solutions/mika/issues/2334",
        "ready-label: senara-solutions/mika#2334",
    )
    .await;

    assert!(
        is_alive(pid),
        "contrôle négatif : un dispatch pour une autre issue doit survivre"
    );
    let child = db.get_task(&other_child).await.unwrap().unwrap();
    assert_eq!(child.status, "pending", "row voisine intacte");
    assert_eq!(child.process_id, Some(pid), "pgid voisin intact");
}

/// Contrôle négatif (b) : un enfant **`delivered`** portant un vieux pgid n'est
/// pas signalé. Son pilote est terminé depuis longtemps ; le PID inscrit sur la
/// row peut désigner n'importe quel process depuis.
#[tokio::test]
#[cfg(target_os = "linux")]
async fn delivered_child_carrying_a_stale_pgid_is_not_signalled() {
    let db = test_db();
    let url = "https://github.com/senara-solutions/mika/issues/2335";
    let (guard, start_time) = spawn_live_child();
    let pid = guard.pid();

    let parent_id = seed_parent_tracking_row(&db, url).await;
    let child_id = seed_dispatch_child(&db, &parent_id, "delivered", pid, Some(start_time)).await;

    supersede_prior_tracking_rows(
        &db,
        SESSION,
        Some(TRACE),
        url,
        "ready-label: senara-solutions/mika#2335",
    )
    .await;

    assert!(
        is_alive(pid),
        "contrôle négatif : le pgid d'un enfant terminal ne désigne plus son \
         pilote — le signaler, c'est tuer un inconnu"
    );
    let child = db.get_task(&child_id).await.unwrap().unwrap();
    assert_eq!(
        child.status, "delivered",
        "la row terminale n'est pas touchée"
    );
}

/// INVARIANT (mika#2335, revue) : **annuler un parent tue le pilote de son
/// enfant.** C'est le geste de l'opérateur, et c'est le même aveuglement
/// parent/enfant que la supersession : `cancel_task_and_kill` lisait
/// `process_id` sur la ligne qu'on lui nomme, donc jamais rien sur un parent.
/// La CLI annonçait « un pilote tourne, PID n », prenait la confirmation,
/// annulait la ligne — et le pilote continuait d'écrire.
///
/// Rouge-avant : retirer la traversée `find_dispatch_children_with_pid` de
/// `cancel_task_and_kill` ; `process_killed` redevient `None` et le `sleep`
/// survit.
#[tokio::test]
#[cfg(target_os = "linux")]
async fn cancelling_a_parent_kills_the_pilot_of_its_child() {
    let db = test_db();
    let url = "https://github.com/senara-solutions/mika/issues/2336";
    let (guard, start_time) = spawn_live_child();
    let pid = guard.pid();

    let parent_id = seed_parent_tracking_row(&db, url).await;
    let child_id = seed_dispatch_child(&db, &parent_id, "pending", pid, Some(start_time)).await;

    assert!(is_alive(pid), "contrôle positif : l'enfant tourne avant");

    let outcome = mika_agent::task_engine::process_kill::cancel_task_and_kill(&db, &parent_id)
        .await
        .expect("cancel must not error")
        .expect("the parent row is cancellable");

    assert_eq!(
        outcome.pid,
        Some(pid),
        "INVARIANT VIOLÉ : l'annulation n'a pas trouvé le pgid — il vit sur \
         l'enfant, pas sur le parent qu'on nomme"
    );
    assert_eq!(
        outcome.process_killed,
        Some(true),
        "le pilote doit être tué"
    );
    assert!(
        !is_alive(pid),
        "INVARIANT VIOLÉ : la task est annulée et le pilote (pgid {pid}) tourne \
         encore — exactement ce que la CLI promettait de faire"
    );

    let child = db.get_task(&child_id).await.unwrap().unwrap();
    assert!(
        child.process_id.is_none(),
        "le pgid doit être effacé sur la ligne qui le PORTE, pas sur le parent"
    );
}

/// Contrôle négatif de la traversée : une task **sans enfant de dispatch** est
/// annulée exactement comme avant, sans prétendre à un kill. Sans lui, une
/// traversée trop gourmande passerait le test ci-dessus.
#[tokio::test]
async fn cancelling_a_parent_without_a_dispatch_child_claims_no_kill() {
    let db = test_db();
    let parent_id =
        seed_parent_tracking_row(&db, "https://github.com/senara-solutions/mika/issues/2337").await;

    let outcome = mika_agent::task_engine::process_kill::cancel_task_and_kill(&db, &parent_id)
        .await
        .expect("cancel must not error")
        .expect("the parent row is cancellable");

    assert_eq!(outcome.pid, None, "aucun pgid à rapporter");
    assert_eq!(
        outcome.process_killed, None,
        "aucun kill tenté, donc aucun kill rapporté"
    );
}

/// Contrôle négatif de la traversée (2) : un enfant **terminal** portant un
/// vieux pgid n'est pas signalé par l'annulation non plus. Même règle que la
/// supersession, et pour la même raison : ce pgid ne désigne plus son pilote.
#[tokio::test]
#[cfg(target_os = "linux")]
async fn cancelling_a_parent_ignores_a_terminal_child_carrying_a_stale_pgid() {
    let db = test_db();
    let (guard, start_time) = spawn_live_child();
    let pid = guard.pid();

    let parent_id =
        seed_parent_tracking_row(&db, "https://github.com/senara-solutions/mika/issues/2338").await;
    seed_dispatch_child(&db, &parent_id, "delivered", pid, Some(start_time)).await;

    let outcome = mika_agent::task_engine::process_kill::cancel_task_and_kill(&db, &parent_id)
        .await
        .expect("cancel must not error")
        .expect("the parent row is cancellable");

    assert_eq!(outcome.pid, None, "un enfant terminal n'offre pas son pgid");
    assert!(
        is_alive(pid),
        "contrôle négatif : le pgid d'un enfant terminal ne désigne plus son \
         pilote — le signaler, c'est tuer un inconnu"
    );
}

/// INVARIANT (mika#2335, revue) : **un kill qui n'atterrit pas n'est pas écrit
/// comme s'il avait atterri.** Un enfant dont le process a survécu garde sa
/// ligne et son pgid — les deux seuls champs par lesquels un faucheur peut
/// encore l'atteindre (`process_id IS NOT NULL` + statut non terminal). Les
/// effacer ferait d'un pilote vivant un pilote définitivement injoignable.
///
/// Le kill est fait échouer sans toucher au code : le `process_start_time`
/// enregistré ne correspond pas au process réel, donc la garde anti-réutilisation
/// de PID refuse de signaler — et `kill_process_gracefully` rend alors `true`
/// (« ce n'est pas notre process »). Ce test emprunte donc l'autre voie de
/// l'échec : un `process_id` hors plage `u32`, que `kill_process_gracefully`
/// refuse en rendant `false` sans rien signaler.
#[tokio::test]
async fn a_supersession_whose_kill_fails_keeps_the_row_and_the_pgid() {
    let db = test_db();
    let url = "https://github.com/senara-solutions/mika/issues/2339";

    // PID négatif : `kill_process_gracefully` refuse et rend `false` sans
    // signaler quoi que ce soit. Aucun process réel n'est en jeu.
    let parent_id = seed_parent_tracking_row(&db, url).await;
    let child_id = seed_dispatch_child(&db, &parent_id, "pending", -4242, Some(12345)).await;

    supersede_prior_tracking_rows(
        &db,
        SESSION,
        Some(TRACE),
        url,
        "ready-label: senara-solutions/mika#2339",
    )
    .await;

    let child = db.get_task(&child_id).await.unwrap().unwrap();
    assert_eq!(
        child.process_id,
        Some(-4242),
        "INVARIANT VIOLÉ : le pgid a été effacé alors que le kill n'a pas \
         atterri — plus aucun faucheur ne peut atteindre ce process"
    );
    assert_eq!(
        child.status, "pending",
        "INVARIANT VIOLÉ : la ligne a été marquée terminale sur un kill raté — \
         « un kill raté et un kill jamais tenté ne doivent pas produire la même \
         ligne » (engine.rs)"
    );
}

/// Contrôle négatif (c) : un enfant **sans `process_start_time`** n'est pas
/// signalé (fail-safe F1.4). `kill_process_gracefully` retomberait sur une
/// simple existence de PID, qui ne distingue pas un PID recyclé — et signaler
/// un groupe de process par erreur est un dégât non borné. Le coût est nommé :
/// cet enfant survit à sa supersession. Inertie, jamais un kill à l'aveugle.
#[tokio::test]
#[cfg(target_os = "linux")]
async fn child_without_a_readable_start_time_is_not_signalled() {
    let db = test_db();
    let url = "https://github.com/senara-solutions/mika/issues/2263";
    let (guard, _start_time) = spawn_live_child();
    let pid = guard.pid();

    let parent_id = seed_parent_tracking_row(&db, url).await;
    let child_id = seed_dispatch_child(&db, &parent_id, "pending", pid, None).await;

    supersede_prior_tracking_rows(
        &db,
        SESSION,
        Some(TRACE),
        url,
        "ready-label: senara-solutions/mika#2263",
    )
    .await;

    assert!(
        is_alive(pid),
        "contrôle négatif : sans start_time la paire qui identifie une \
         *instance* de process est incomplète — on n'en tue aucun"
    );
    let child = db.get_task(&child_id).await.unwrap().unwrap();
    assert_eq!(child.process_id, Some(pid), "le pgid reste inscrit");

    let audits = db
        .count_audit_events_by_tool_name(SUPERSEDED_DISPATCH_PROCESS_KILLED_TOOL)
        .await
        .unwrap();
    assert_eq!(audits, 0, "aucun kill, donc aucun événement de kill");
}

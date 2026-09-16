//! mika#2279 — **le terme sur lequel repose le filtre 4b d'`auto_pull`, mesuré
//! sur un vrai processus.**
//!
//! # Ce que ce fichier atteste, et ce qu'il n'atteste pas
//!
//! La *décision* de Phase 2 est pure et vit avec son classifieur :
//! `auto_pull::tests::mika2279_a_live_pilot_skips_without_touching_the_budget`
//! (skip nommé, budget intact — le point mika#2158),
//! `…::mika2279_a_dead_pilot_leaves_the_redrive_nominal` (contrôle négatif b),
//! `…::mika2279_the_nominal_in_flight_case_is_still_named_in_flight` (l'ordre des
//! branches, qui est ce qui borne le coût de la sonde) et
//! `…::mika2279_phase2_skips_a_ready_ticket_whose_pilot_is_alive` (le câblage de
//! bout en bout, ledger et budget compris, sans réseau). Ils sont in-crate parce
//! que `classify_stuck_ready` et `phase2_reconcile_stuck_ready` y sont privés —
//! les exposer pour un test aurait élargi la surface publique du module sans rien
//! attester de plus.
//!
//! Ce qu'aucun d'eux ne peut faire honnêtement, et qui est ici : distinguer un
//! pilote **authentiquement vivant** d'un pilote **authentiquement mort**. Le
//! test in-crate se sert du process de test lui-même (vivant par construction) et
//! d'un PID inexistant pour le cas mort ; un PID qui n'a jamais existé n'est pas
//! la même chose qu'un pilote terminé, et c'est justement la confusion que le
//! prédicat doit tenir. Ici le processus est lancé, mesuré vivant, puis **tué et
//! récolté** — sans la récolte, `/proc/<pid>/stat` survit en zombie et une sonde
//! de vivacité mentirait.
//!
//! # Rouge-avant (porte #2264)
//!
//! Recette d'injection : dans `live_pilot_for_issue`, remplacer
//! `is_same_process_alive(pid, start_time)` par une existence nue de
//! `/proc/<pid>` — [`a_reaped_pilot_is_not_alive`] reste vert mais
//! [`a_recycled_pid_is_not_the_pilot`] devient rouge. Retirer le filtre sur le
//! statut de l'enfant : [`a_terminal_child_is_not_a_live_pilot_on_a_real_process`]
//! devient rouge. Réintroduire un prédicat sur le statut du parent :
//! [`the_production_topology_with_a_cancelled_parent_reads_alive`] devient rouge.

use std::process::Command;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{Database, NewTask};
use mika_agent::live_pilot::{LivePilotVerdict, live_pilot_for_issue};

const AGENT_ID: &str = "mika";
const URL: &str = "https://github.com/senara-solutions/mika/issues/2276";

fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory DB");
    AsyncDatabase::new_with_agent(db, AGENT_ID)
}

/// Tue son groupe de process à la destruction, échec du test compris — un `kill`
/// en fin de test n'est jamais atteint quand une assertion panique, et c'est
/// exactement le moment où le nettoyage compte.
struct ChildGuard(i64);

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

/// Un pilote : chef de son propre groupe de process, comme `dispatch-lib` en
/// donne un. Sortie vers `/dev/null`, sinon le pipe hérité tient `cargo test`
/// ouvert jusqu'à la fin du `sleep`.
fn spawn_live_pilot() -> (ChildGuard, std::process::Child, i64, u64) {
    use std::os::unix::process::CommandExt;
    let child = Command::new("sleep")
        .arg("600")
        .process_group(0)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn sleep");
    let pid = i64::from(child.id());
    let start_time = mika_agent::task_engine::process_liveness::read_process_start_time(child.id())
        .expect("read child start time");
    (ChildGuard(pid), child, pid, start_time)
}

async fn seed_parent(db: &AsyncDatabase, url: &str, status: &str) -> String {
    let id = db
        .create_task(NewTask {
            agent_id: AGENT_ID.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: format!("ready-label: {url}"),
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
            created_by_session: Some("s".to_string()),
            created_trace_id: None,
            reference_url: Some(url.to_string()),
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: Some("issue".to_string()),
            dispatch_class: Some("implement".to_string()),
        })
        .await
        .expect("create parent tracking row");
    if status != "pending" {
        db.update_task_status(&id, status)
            .await
            .expect("set parent status");
    }
    id
}

async fn seed_child(
    db: &AsyncDatabase,
    parent: &str,
    status: &str,
    pid: i64,
    start_time: Option<u64>,
) -> String {
    let id = db
        .create_task(NewTask {
            agent_id: AGENT_ID.to_string(),
            team_run_id: None,
            parent_task_id: Some(parent.to_string()),
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
            created_by_session: Some("s".to_string()),
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

/// INVARIANT : l'état exact du 2026-09-10 — parent `cancelled` depuis 30 s,
/// pilote qui travaillera encore une heure — se lit `Alive`.
///
/// C'est la lecture que `has_active_self_dev_task_for_issue` rend **fausse** (sa
/// conjonction `reference_url LIKE …` ET `status IN ('pending','in_progress')`
/// porte sur une seule row, et cette row est annulée), et c'est cette fausse
/// réponse qui faisait re-driver le label toutes les 20 minutes.
#[tokio::test]
#[cfg(target_os = "linux")]
async fn the_production_topology_with_a_cancelled_parent_reads_alive() {
    let db = test_db();
    let (_guard, _child, pid, start_time) = spawn_live_pilot();

    let parent = seed_parent(&db, URL, "cancelled").await;
    let child = seed_child(&db, &parent, "pending", pid, Some(start_time)).await;

    match live_pilot_for_issue(&db, URL).await {
        LivePilotVerdict::Alive {
            child_task_id,
            parent_task_id,
            pid: got,
        } => {
            assert_eq!(child_task_id, child, "la row qui PORTE le pgid");
            assert_eq!(
                parent_task_id, parent,
                "et la row qu'un opérateur annulerait pour forcer un re-dispatch"
            );
            assert_eq!(i64::from(got), pid);
        }
        other => panic!(
            "INVARIANT VIOLÉ : parent annulé + pilote vif rend {other:?} — c'est le \
             prédicat aveugle qui a fait battre le label de #2276"
        ),
    }
}

/// Contrôle négatif — **un pilote tué et récolté n'est pas vif.** La récolte est
/// le point : sans elle le process survit en zombie dans `/proc` et n'importe
/// quelle sonde par existence de fichier répondrait « vivant ».
#[tokio::test]
#[cfg(target_os = "linux")]
async fn a_reaped_pilot_is_not_alive() {
    let db = test_db();
    let (_guard, mut child_proc, pid, start_time) = spawn_live_pilot();

    let parent = seed_parent(&db, URL, "cancelled").await;
    seed_child(&db, &parent, "pending", pid, Some(start_time)).await;
    assert!(
        live_pilot_for_issue(&db, URL).await.is_alive(),
        "contrôle positif : vif avant le kill"
    );

    let _ = Command::new("kill")
        .arg("-9")
        .arg(pid.to_string())
        .output();
    child_proc.wait().expect("reap the pilot");

    assert_eq!(
        live_pilot_for_issue(&db, URL).await,
        LivePilotVerdict::None,
        "contrôle négatif : un pilote terminé ne gèle pas son ticket — `auto_pull` \
         doit pouvoir le re-driver"
    );
}

/// Contrôle négatif — **un PID recyclé n'est pas le pilote.** Le
/// `process_start_time` enregistré ne correspond pas au process réel : la paire
/// (pid, start_time) est ce qui identifie une *instance*, et c'est pour ça que la
/// sonde ne peut pas être une simple existence de `/proc/<pid>`.
#[tokio::test]
#[cfg(target_os = "linux")]
async fn a_recycled_pid_is_not_the_pilot() {
    let db = test_db();
    let (_guard, _child, pid, start_time) = spawn_live_pilot();

    let parent = seed_parent(&db, URL, "cancelled").await;
    // Un start_time qui ne peut appartenir à ce process : le PID existe, mais
    // l'instance inscrite sur la row est morte depuis longtemps.
    seed_child(&db, &parent, "pending", pid, Some(start_time + 1)).await;

    assert_eq!(
        live_pilot_for_issue(&db, URL).await,
        LivePilotVerdict::None,
        "contrôle négatif : un PID réutilisé lu comme « le pilote » gèlerait un \
         ticket au profit d'un process inconnu"
    );
}

/// Contrôle négatif — **un enfant terminal portant un pgid encore vivant.** Son
/// pilote est fini ; ce que le PID désigne maintenant ne le concerne plus. Même
/// règle, et même raison, que la disposition de supersession (mika#2335).
#[tokio::test]
#[cfg(target_os = "linux")]
async fn a_terminal_child_is_not_a_live_pilot_on_a_real_process() {
    let db = test_db();
    let (_guard, _child, pid, start_time) = spawn_live_pilot();

    let parent = seed_parent(&db, URL, "cancelled").await;
    seed_child(&db, &parent, "delivered", pid, Some(start_time)).await;

    assert_eq!(
        live_pilot_for_issue(&db, URL).await,
        LivePilotVerdict::None,
        "contrôle négatif : le pgid d'un enfant terminal est périmé — le lire \
         comme un pilote vif gèlerait le ticket pour toujours"
    );
}

//! Négatif (a) — mika#2263 : **une supersession ne laisse jamais un pilote vivant
//! derrière elle.**
//!
//! Classe mesurée le 2026-09-09 : `#2252` et `#2212` portaient chacun une row
//! `cancelled` (l'ancien dispatch, superseded) ET un process bwrap **toujours
//! vivant** (69 min / 45 min), parce que la supersession annule la ROW et
//! ignore le PROCESS. Deux écrivains sur le même worktree (classe #2248/#2249)
//! et deux slots de dispatch brûlés.
//!
//! L'invariant que ces tests nomment : *après un passage de
//! `supersede_prior_tracking_rows` sur une `reference_url`, aucun process de
//! dispatch antérieur pour cette URL n'est encore en vie.*
//!
//! # Rouge-avant (porte #2264)
//!
//! Sur `main`, `supersede_prior_tracking_rows` ne regarde QUE les rows
//! fantômes (`process_id IS NULL`) : une row portant un pgid n'est même pas
//! candidate, donc l'enfant reste vivant et
//! [`live_pilot_is_killed_by_supersede`] échoue sur son assertion
//! `!is_alive(pid)`. Sur cette branche il passe. Recette d'injection : commenter
//! l'appel `dispose_superseded_dispatch_processes(...)` dans
//! `supersede_prior_tracking_rows` — le test redevient rouge.

use std::process::Command;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{Database, NewTask};
use mika_agent::tracking_cleanup::supersede_prior_tracking_rows;

const AGENT_ID: &str = "mika";
const SESSION: &str = "eval-session";
const TRACE: &str = "eval-trace";

fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory DB");
    AsyncDatabase::new_with_agent(db, AGENT_ID)
}

/// Un enfant réel, chef de son propre groupe de process — la forme exacte que
/// `dispatch-lib` donne à un pilote (le kill vise le pgid). Reprend
/// `spawn_live_child` de `test_pilot_silent_stall_reaper.rs` : le handle est
/// attendu dans un thread pour que le process mort soit *récolté*, sinon
/// `/proc/<pid>/stat` survit en zombie et une sonde de vivacité mentirait.
fn spawn_live_child() -> (i64, u64) {
    use std::os::unix::process::CommandExt;
    let mut child = Command::new("sleep")
        .arg("600")
        .process_group(0)
        .spawn()
        .expect("spawn sleep");
    let pid = i64::from(child.id());
    let start_time = mika_agent::task_engine::process_liveness::read_process_start_time(child.id())
        .expect("read child start time");
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    (pid, start_time)
}

fn is_alive(pid: i64) -> bool {
    std::path::PathBuf::from(format!("/proc/{pid}/stat")).exists()
}

fn kill_pid(pid: i64) {
    let _ = Command::new("kill").arg("-9").arg(pid.to_string()).output();
}

/// Sème un dispatch VIVANT : row non terminale portant un `process_id` et le
/// `process_start_time` que la garde anti-réutilisation de PID (#855) exige.
async fn seed_live_dispatch(
    db: &AsyncDatabase,
    label: &str,
    reference_url: &str,
    status: &str,
    pid: i64,
    start_time: Option<u64>,
) -> String {
    let id = db
        .create_task(NewTask {
            agent_id: AGENT_ID.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: label.to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "run_skill".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some(SESSION.to_string()),
            created_trace_id: None,
            reference_url: Some(reference_url.to_string()),
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: Some("implement".to_string()),
        })
        .await
        .expect("create dispatch row");
    db.update_task_status(&id, status)
        .await
        .expect("set status");
    db.set_task_process_id(&id, Some(pid))
        .await
        .expect("record pid");
    if let Some(st) = start_time {
        db.set_task_metadata_field(&id, "process_start_time", &st.to_string())
            .await
            .expect("record start time");
    }
    id
}

/// INVARIANT : une supersession ne laisse jamais un pilote vivant derrière
/// elle. Le process est tué ET la row cesse d'être active.
#[tokio::test]
#[cfg(target_os = "linux")]
async fn live_pilot_is_killed_by_supersede() {
    let db = test_db();
    let url = "https://github.com/senara-solutions/mika/issues/2252";
    let (pid, start_time) = spawn_live_child();
    let old_id = seed_live_dispatch(
        &db,
        "long_running:run_claude_pilot mika#2252",
        url,
        "in_progress",
        pid,
        Some(start_time),
    )
    .await;

    assert!(is_alive(pid), "contrôle positif : l'enfant tourne avant");

    supersede_prior_tracking_rows(
        &db,
        SESSION,
        Some(TRACE),
        url,
        "ready-label: senara-solutions/mika#2252",
    )
    .await;

    assert!(
        !is_alive(pid),
        "INVARIANT VIOLÉ : la supersession a annulé la row et laissé le pilote \
         (pgid {pid}) vivant — c'est le zombie de mika#2263"
    );
    let old = db.get_task(&old_id).await.unwrap().unwrap();
    assert_eq!(
        old.status, "cancelled",
        "le dispatch superseded doit être terminal"
    );
    assert!(
        old.process_id.is_none(),
        "le pgid doit être effacé après le kill, sinon un faucheur retente"
    );
    kill_pid(pid);
}

/// Contrôle négatif du même appel : un dispatch vivant pour une AUTRE issue
/// n'est pas touché. Sans lui, un `kill` aveugle passerait le test ci-dessus.
#[tokio::test]
#[cfg(target_os = "linux")]
async fn live_pilot_on_another_issue_survives() {
    let db = test_db();
    let other_url = "https://github.com/senara-solutions/mika/issues/2212";
    let (pid, start_time) = spawn_live_child();
    let other_id = seed_live_dispatch(
        &db,
        "long_running:run_claude_pilot mika#2212",
        other_url,
        "in_progress",
        pid,
        Some(start_time),
    )
    .await;

    supersede_prior_tracking_rows(
        &db,
        SESSION,
        Some(TRACE),
        "https://github.com/senara-solutions/mika/issues/2252",
        "ready-label: senara-solutions/mika#2252",
    )
    .await;

    assert!(
        is_alive(pid),
        "contrôle négatif : un dispatch pour une autre issue doit survivre"
    );
    let other = db.get_task(&other_id).await.unwrap().unwrap();
    assert_eq!(other.status, "in_progress", "row voisine intacte");
    kill_pid(pid);
}

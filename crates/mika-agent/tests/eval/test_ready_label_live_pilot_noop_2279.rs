//! mika#2279 — **un événement `labeled ready` répété sur un ticket dont le
//! pilote est vif est un NO-OP.**
//!
//! # L'incident, mesuré le 2026-09-10 sur #2276
//!
//! `10:46:57` dispatch : parent `23fd8852`, enfant `ddb05913` (pid 577434,
//! pilote vivant, log actif jusqu'à 11:59). `10:47:28` — trente secondes plus
//! tard — un **second** événement `labeled ready` sur le même ticket, déjà en
//! vol : supersession, parent `cancelled`, `dispatch readiness check failed`,
//! `deferred_dispatch_registered` → un **second pilote #2276 en doublon** en
//! file. Annulé à la main à 11:02, réapparu à 11:10, puis à 11:30 : période
//! ~20 min. Les événements GitHub sont **tous** de `mika-platform-bot` — c'est
//! le moteur qui fait battre le label, le défaut n'a besoin de personne pour se
//! rejouer.
//!
//! Depuis mika#2335 la supersession ne se contente plus d'orpheliner ce pilote :
//! elle le **tue**. C'est le contrat correct pour un dispatch *neuf* ; un
//! événement `labeled` rejoué n'est pas un dispatch neuf. La garde manquante est
//! donc en amont, sur le déclencheur — porte 2c, avant le supersede et avant le
//! pré-create.
//!
//! # La fixture : la topologie que la production écrit
//!
//! | row | `trigger_type` | `reference_url` | `process_id` |
//! |---|---|---|---|
//! | **parent** (tracking) | `manual` / `action_type='none'` | **oui** | **jamais** |
//! | **enfant** (callback) | `callback` / `resume_agent` | **non** | **oui** (le pgid) |
//!
//! Un processus réel, chef de son groupe, comme `dispatch-lib` en donne un au
//! pilote — la vivacité est *mesurée*, pas simulée. Même précaution que
//! `test_supersede_kills_live_pilot` : sortie vers `/dev/null` (sinon le pipe de
//! `cargo test` attend la fin du `sleep`) et un garde qui signale à la
//! destruction (un `kill` en fin de test n'est jamais atteint quand une
//! assertion panique, et c'est exactement le moment où le nettoyage compte).
//!
//! # Rouge-avant (porte #2264)
//!
//! Recette d'injection : retirer l'étape 2c de
//! `try_handle_ready_label_dispatch_with_fetcher`. [`a_live_pilot_makes_the_ready_event_a_noop`]
//! échoue alors deux fois — sur le compte de tasks (3 au lieu de 2 : la row
//! pré-créée) **et** sur `is_alive(pid)` (le pilote productif tué par la
//! supersession). Les quatre contrôles négatifs restent verts, ce qui est le
//! point : neutraliser tous les termes d'une conjonction d'un coup laisserait
//! passer une implémentation qui n'en lit qu'un (leçon mika#2277).

use std::process::Command;
use std::sync::Arc;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{Database, NewTask};
use mika_agent::messaging::{MessageSender, SendOutcome};
use mika_agent::server::ready_label_handler::try_handle_ready_label_dispatch_with_fetcher;
use mika_agent::server::verdict_handler::VerdictAction;
use mika_agent::skills::SkillRegistry;
use mika_common::github_event_format::READY_LABEL_DISPATCH_MARKER;

const AGENT_ID: &str = "mika";
const SESSION: &str = "eval-session";
const TRACE: &str = "eval-trace";

const URL: &str = "https://github.com/senara-solutions/mika/issues/2276";

/// Un ticket groomé : rien d'autre que la porte testée ne peut refuser.
const GROOMED_BODY: &str = "> - **Plan:** docs/plans/2026-09-10-fix-2276.md\n> - **Branch:** `fix/2276`\n\
     \n> - **Grooming history:** first-pass (READY) → second-pass (GROOMED)\n";

const READY_EVENT: &str = "[GitHub] Issue labeled ready on senara-solutions/mika#2276 — fix: quelque chose\nhttps://github.com/senara-solutions/mika/issues/2276";

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

fn spawn_live_pilot() -> (ChildGuard, u64) {
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

fn is_alive(pid: i64) -> bool {
    std::path::PathBuf::from(format!("/proc/{pid}/stat")).exists()
}

/// La row **parent** : ce que `ready_label_handler` pré-crée. Porte l'URL,
/// jamais de `process_id`.
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
            created_by_session: Some(SESSION.to_string()),
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

/// La row **enfant** : ce que `build_callback_task` + `set_task_process_id`
/// écrivent. Porte le pgid, jamais d'URL.
async fn seed_child(db: &AsyncDatabase, parent: &str, pid: i64, start_time: Option<u64>) -> String {
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

/// Toutes les tasks de l'agent, vivantes ou mortes.
async fn task_count(db: &AsyncDatabase) -> usize {
    db.get_tasks_by_status(vec![
        "pending".to_string(),
        "in_progress".to_string(),
        "blocked".to_string(),
        "completed".to_string(),
        "failed".to_string(),
        "cancelled".to_string(),
        "expired".to_string(),
        "delivered".to_string(),
    ])
    .await
    .expect("list tasks")
    .len()
}

async fn run_handler(db: &AsyncDatabase) -> VerdictAction {
    let sender: Arc<dyn MessageSender> = Arc::new(NoopSender);
    let skills = SkillRegistry::empty();
    try_handle_ready_label_dispatch_with_fetcher(
        READY_EVENT,
        db,
        Some("fake-token"),
        Some(&sender),
        SESSION,
        TRACE,
        &skills,
        move |_owner_repo, _number, _token| async move {
            Ok((GROOMED_BODY.to_string(), vec!["ready".to_string()]))
        },
    )
    .await
}

/// §1 — INVARIANT : pilote vif ⇒ zéro task, zéro kill, zéro supersession, et le
/// refus n'est pas re-convertible en dispatch par la garde d'intention.
///
/// Le parent est `in_progress`, l'état nominal d'un dispatch parti (mika#2335
/// F2a). §2 rejoue la même chose sur un parent `cancelled` : c'est le seul terme
/// qui distingue les deux, et c'est l'état exact mesuré le 2026-09-10.
#[tokio::test]
#[cfg(target_os = "linux")]
async fn a_live_pilot_makes_the_ready_event_a_noop() {
    let db = test_db();
    let (guard, start_time) = spawn_live_pilot();
    let pid = guard.pid();

    let parent = seed_parent(&db, URL, "in_progress").await;
    let child = seed_child(&db, &parent, pid, Some(start_time)).await;
    assert_eq!(
        task_count(&db).await,
        2,
        "contrôle positif : deux rows semées"
    );
    assert!(is_alive(pid), "contrôle positif : le pilote tourne avant");

    let action = run_handler(&db).await;

    assert_eq!(
        task_count(&db).await,
        2,
        "INVARIANT VIOLÉ : un `labeled ready` répété a créé une row de tracking \
         alors qu'un pilote tourne — c'est le doublon #2276 de mika#2279"
    );
    assert!(
        is_alive(pid),
        "INVARIANT VIOLÉ : la supersession a tué un pilote productif (pgid {pid}) \
         sur un simple re-label — mika#2335 tue le pilote qu'un dispatch NEUF \
         remplace, et un événement rejoué n'est pas un dispatch neuf"
    );

    let parent_row = db.get_task(&parent).await.unwrap().unwrap();
    assert_eq!(
        parent_row.status, "in_progress",
        "INVARIANT VIOLÉ : la row parent a été superséded — le tracking du pilote \
         vivant est cassé et son callback atterrira sur une row annulée"
    );
    let child_row = db.get_task(&child).await.unwrap().unwrap();
    assert_eq!(child_row.process_id, Some(pid), "le pgid reste inscrit");

    match action {
        VerdictAction::Handled { pre_digest } => {
            assert!(
                pre_digest.starts_with("<ready_label_handler>"),
                "le refus doit ouvrir sur la balise du handler"
            );
            assert!(
                !pre_digest.starts_with(READY_LABEL_DISPATCH_MARKER),
                "INVARIANT VIOLÉ : le pre-digest matche le déclencheur de \
                 l'INTENT_GUARD webhook_ready_label_dispatch, qui re-sommerait le \
                 LLM de dispatcher le ticket que la porte vient de refuser"
            );
            assert!(
                pre_digest.contains(&pid.to_string()),
                "le refus doit nommer le pid du pilote vivant"
            );
            assert!(
                pre_digest.contains("2276"),
                "le refus doit nommer le ticket refusé"
            );
            assert!(
                pre_digest.contains("mika tasks cancel"),
                "un refus qui ne nomme pas sa levée est un refus qu'on contourne \
                 au jugé"
            );
        }
        other => panic!(
            "le refus doit être Handled (Passthrough laisse le marqueur ready dans \
             req.text et la garde re-somme le LLM de dispatcher) — obtenu {other:?}"
        ),
    }
}

/// §2 — **un parent annulé ne masque pas le pilote.** L'état exact du
/// 2026-09-10 : le parent est `cancelled` depuis 10:47 et le pilote travaille
/// jusqu'à 11:59. C'est ce que `has_active_self_dev_task_for_issue` lit comme
/// « rien en vol », et c'est ce qui transformait un défaut ponctuel en boucle.
#[tokio::test]
#[cfg(target_os = "linux")]
async fn a_cancelled_parent_does_not_hide_the_live_pilot() {
    let db = test_db();
    let (guard, start_time) = spawn_live_pilot();
    let pid = guard.pid();

    let parent = seed_parent(&db, URL, "cancelled").await;
    seed_child(&db, &parent, pid, Some(start_time)).await;

    let action = run_handler(&db).await;

    assert_eq!(
        task_count(&db).await,
        2,
        "INVARIANT VIOLÉ : parent annulé + pilote vif a produit un dispatch en \
         doublon — la boucle de re-label de mika#2279"
    );
    assert!(is_alive(pid), "le pilote vivant survit au re-label");
    assert!(
        matches!(action, VerdictAction::Handled { .. }),
        "le refus doit être Handled"
    );
}

/// §3 — négatif : **pilote mort.** La row enfant porte le pgid d'un processus
/// récolté ; le dispatch procède comme avant le correctif. Sans ce contrôle, une
/// garde qui refuserait sur la simple présence d'une row enfant gèlerait tout
/// ticket ayant déjà été dispatché une fois.
#[tokio::test]
#[cfg(target_os = "linux")]
async fn a_dead_pilot_does_not_freeze_the_ticket() {
    let db = test_db();
    let parent = seed_parent(&db, URL, "cancelled").await;
    // PID inexistant : `is_same_process_alive` rend false, verdict `None`.
    seed_child(&db, &parent, 999_999_999, Some(12345)).await;

    let action = run_handler(&db).await;

    assert_eq!(
        task_count(&db).await,
        3,
        "contrôle négatif : un pilote mort ne doit pas empêcher le dispatch — la \
         garde ne gèle pas un ticket sur une row orpheline"
    );
    assert!(
        matches!(action, VerdictAction::Handled { .. }),
        "le chemin nominal dégradé (SkillRegistry vide) rend Handled"
    );
}

/// §4 — négatif : **`process_start_time` absent.** Le verdict est `Unreadable`,
/// jamais `Alive` : on ne peut pas prouver la vivacité, donc on ne bloque pas.
/// Le fail-safe est attesté, pas supposé — et le coût est nommé : c'est la seule
/// forme sous laquelle mika#2279 peut encore se produire (population déjà
/// comptée par mika#2335 sous `unusable_child_count`).
#[tokio::test]
#[cfg(target_os = "linux")]
async fn an_unreadable_start_time_does_not_block_the_dispatch() {
    let db = test_db();
    let (guard, _start_time) = spawn_live_pilot();
    let pid = guard.pid();

    let parent = seed_parent(&db, URL, "cancelled").await;
    seed_child(&db, &parent, pid, None).await;

    run_handler(&db).await;

    assert_eq!(
        task_count(&db).await,
        3,
        "contrôle négatif : un signal illisible n'est jamais un terme satisfait — \
         le comportement d'avant le correctif est repris"
    );
    assert!(
        is_alive(pid),
        "et la supersession ne signale pas non plus un enfant sans start_time \
         (mika#2335 : un PID recyclé y serait indistinguable)"
    );
}

/// §5 — négatif : **un autre ticket.** Un pilote vif sur #9999 ne refuse pas
/// #2276. Le prédicat est porté par l'URL ; sans ce contrôle, une traversée trop
/// gourmande passerait les deux positifs.
#[tokio::test]
#[cfg(target_os = "linux")]
async fn a_live_pilot_on_another_ticket_does_not_refuse_this_one() {
    let db = test_db();
    let (guard, start_time) = spawn_live_pilot();
    let pid = guard.pid();

    let other = seed_parent(
        &db,
        "https://github.com/senara-solutions/mika/issues/9999",
        "in_progress",
    )
    .await;
    seed_child(&db, &other, pid, Some(start_time)).await;

    run_handler(&db).await;

    assert_eq!(
        task_count(&db).await,
        3,
        "contrôle négatif : le pilote de #9999 ne tient pas #2276"
    );
    assert!(
        is_alive(pid),
        "et le dispatch de #2276 ne supersède pas la row de #9999"
    );
}

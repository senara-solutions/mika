//! Tests de bout en bout du fermeur de parents de dispatch (mika#2405).
//!
//! Le défaut fermé : une row `tasks` `trigger_type='manual'` ouverte
//! **mécaniquement** par la Delegation Rule avant un dispatch long-running
//! n'avait, hors self_dev, aucun mécanisme qui la referme. Trois rows mika-qa
//! mesurées le 2026-09-19 s'accumulaient en `in_progress` sans jamais atteindre
//! un état terminal.
//!
//! Chaque test traverse `TaskEngine::tick()` — jamais le fermeur directement —
//! parce que le câblage dans la branche `DB_SCAN_INTERVAL_TICKS` fait partie de
//! ce qui doit rester vrai : un fermeur correct mais non appelé se lit comme un
//! fermeur qui n'a rien trouvé à faire.
//!
//! Recette de vérification par injection (obligatoire) : commenter l'appel
//! `self.settle_dispatch_parents().await;` dans `tick()`, ou faire retourner
//! `settle_dispatch_parents` immédiatement — `settles_terminal_dispatch_parent`
//! et `settles_even_when_a_child_has_no_readable_start_time` doivent rougir.
//! Restaurer, relancer, elles repassent au vert.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{Database, NewTask};
use mika_agent::messaging::{MessageSender, SendOutcome};
use mika_agent::skills::SkillRegistry;
use mika_agent::task_engine::dispatcher::TaskDispatcher;
use mika_agent::task_engine::engine::TaskEngine;
use mika_agent::tools::default_tools;

const AGENT_ID: &str = "mika-qa";

/// Le nom d'événement d'audit dont ce fermeur est **sole writer**. Écrit ici en
/// littéral à dessein : si la constante de production changeait de valeur, ces
/// assertions doivent rougir plutôt que suivre.
const SETTLED_EVENT: &str = "dispatch_parent_settled";

/// Au-delà de la grâce par défaut (600 s, alignée sur `REAPER_GRACE_SECONDS`).
const PAST_GRACE_SECS: i64 = 700;

struct NoopSender;

#[async_trait::async_trait]
impl MessageSender for NoopSender {
    async fn send(&self, _text: &str) -> anyhow::Result<SendOutcome> {
        Ok(SendOutcome::Delivered)
    }
}

/// Base en mémoire cadrée sur [`AGENT_ID`]. L'agent est enregistré
/// explicitement : `mika-qa` — l'agent de la population mesurée — n'est pas
/// l'agent par défaut d'une base neuve, et `tasks.agent_id` porte une clé
/// étrangère.
async fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory DB");
    let db = AsyncDatabase::new_with_agent(db, AGENT_ID);
    db.register_agent(AGENT_ID, "Mika QA", "/tmp/mika-qa")
        .await
        .expect("register agent");
    db
}

fn test_dispatcher(db: AsyncDatabase) -> Arc<TaskDispatcher> {
    let tmp = tempfile::tempdir().expect("tmp dir");
    let settings = mika_common::config::Settings::load(tmp.path()).expect("load settings");
    Arc::new(TaskDispatcher {
        db,
        tier: mika_common::home::AgentTier::Default,
        deployment: mika_common::home::Deployment::Unknown,
        llm: mika_common::llm::dummy_provider(),
        tools: Arc::new(default_tools()),
        skills: Arc::new(SkillRegistry::empty()),
        message_sender: Some(Arc::new(NoopSender)),
        home_dir: PathBuf::from("/tmp"),
        global_home_dir: PathBuf::from("/tmp/mika-test-global-home-absent"),
        embedding_client: None,
        brave_api_key: None,
        gateway_url: None,
        internal_token: None,
        github_token: None,
        github_app: None,
        skills_dirty: Arc::new(AtomicBool::new(false)),
        agent_lock: None,
        cli_mode: true,
        settings,
        pr_reviews_posted: None,
        auto_pull_stop_armed: AtomicBool::new(false),
        proactive_budget_reported: std::sync::Mutex::new(None),
    })
}

/// La row de suivi que la Delegation Rule fait ouvrir : `manual` / `none`, sans
/// `process_id` propre, `in_progress`. `source` reste `None` — c'est la forme
/// exacte que `tools/create_task.rs` écrit hors des chemins self-dev, et c'est
/// le terme qui sépare cette population de celle du couple #871/#1162.
async fn seed_tracking_parent(db: &AsyncDatabase, label: &str, source: Option<&str>) -> String {
    let id = db
        .create_task(NewTask {
            agent_id: AGENT_ID.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: label.to_string(),
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
            created_by_session: Some("eval-session".to_string()),
            created_trace_id: None,
            reference_url: None,
            source: source.map(str::to_string),
            metadata: None,
            r#type: None,
            dispatch_class: None,
        })
        .await
        .expect("create tracking parent");

    db.update_task_status(&id, "in_progress")
        .await
        .expect("set in_progress");

    id
}

/// L'enfant callback du dispatch. `status` est posé tel quel ; `pid` et
/// `start_time` reproduisent ce que l'exécuteur écrit (le start time est stocké
/// comme **chaîne** JSON — le reproduire autrement validerait une forme que la
/// production n'écrit jamais).
async fn seed_callback_child(
    db: &AsyncDatabase,
    parent_id: &str,
    status: &str,
    age_secs: i64,
    pid: Option<i64>,
    start_time: Option<u64>,
) -> String {
    let id = db
        .create_task(NewTask {
            agent_id: AGENT_ID.to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.to_string()),
            depth: 1,
            label: "long_running:build_mika".to_string(),
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
            created_by_session: Some("eval-session".to_string()),
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        })
        .await
        .expect("create callback child");

    if let Some(p) = pid {
        db.set_task_process_id(&id, Some(p))
            .await
            .expect("set child process_id");
    }
    if let Some(st) = start_time {
        db.set_task_metadata_field(&id, "process_start_time", &st.to_string())
            .await
            .expect("set process_start_time");
    }

    db.update_task_status(&id, status)
        .await
        .expect("set child status");
    db.backdate_task_updated_at(&id, age_secs)
        .await
        .expect("backdate child updated_at");

    id
}

/// Le couple `(pid, start_time)` du processus de test — vivant pour toute la
/// durée de l'assertion, ce qui rend le test déterministe plutôt que dépendant
/// d'une course avec un enfant spawné.
fn own_live_process() -> (i64, u64) {
    let pid = std::process::id();
    let start_time = mika_agent::task_engine::process_liveness::read_process_start_time(pid)
        .expect("read own process start time");
    (i64::from(pid), start_time)
}

/// La garde de vivacité est un mécanisme Linux (`/proc/<pid>/stat` champ 22).
/// Ailleurs, sauter est la réponse honnête plutôt qu'une assertion affaiblie.
fn skip_off_linux() -> bool {
    !cfg!(target_os = "linux")
}

/// Pousser le moteur au-delà d'une passe de scan DB (60 ticks).
async fn run_one_scan(engine: &mut TaskEngine) {
    for _ in 0..60 {
        engine.tick().await;
    }
}

/// **Test 1 (AC4) — le chemin nominal, avec son contrôle négatif self_dev dans
/// le même test.** Une revue auto-QA menée à son terme ne laisse aucune row
/// `manual` orpheline ; la row self_dev voisine, elle, n'est pas touchée par ce
/// fermeur.
///
/// Le contrôle négatif est dans le **même** test à dessein : c'est le refus n°1
/// du plan qui est épinglé (« élargir `parent.source = 'self_dev'` »), pas
/// seulement la fonctionnalité. Séparés, un jour où le terme `source` sauterait,
/// le premier resterait vert et le second échouerait sans que rien ne dise que
/// les deux décrivent une seule décision.
#[tokio::test]
async fn settles_terminal_dispatch_parent() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        let parent = seed_tracking_parent(&db, "suivi revue QA — build", None).await;
        seed_callback_child(&db, &parent, "delivered", PAST_GRACE_SECS, None, None).await;

        // Contrôle négatif : même forme, `source='self_dev'`.
        let self_dev_parent = seed_tracking_parent(&db, "suivi self_dev", Some("self_dev")).await;
        seed_callback_child(
            &db,
            &self_dev_parent,
            "delivered",
            PAST_GRACE_SECS,
            None,
            None,
        )
        .await;

        assert_eq!(
            db.count_audit_events_by_tool_name(SETTLED_EVENT)
                .await
                .unwrap(),
            0,
            "référence : aucun événement avant la passe"
        );

        run_one_scan(&mut engine).await;

        let settled = db.get_task(&parent).await.unwrap().unwrap();
        assert_eq!(
            settled.status, "completed",
            "la row de suivi doit atteindre un état terminal — et `completed`, \
             pas `failed` : la revue a réussi"
        );
        assert!(
            settled.completed_at.is_some(),
            "`update_task_completed` stampe `completed_at` — un `completed` sans \
             cette date serait une incohérence introduite par le correctif"
        );

        let self_dev = db.get_task(&self_dev_parent).await.unwrap().unwrap();
        assert_ne!(
            self_dev.status, "completed",
            "refus n°1 : la population self_dev garde ses propres faucheuses, \
             dont le verdict est plus riche"
        );

        let rows = db
            .get_audit_event_rows_by_tool_name(SETTLED_EVENT)
            .await
            .unwrap();
        assert_eq!(
            rows.len(),
            1,
            "exactement une ligne d'audit, pour la row fermée"
        );
        let (target_key, before, after, reasoning) = &rows[0];
        assert_eq!(target_key, &format!("task:{parent}"));
        assert_eq!(before.as_deref(), Some("in_progress"));
        assert_eq!(after.as_deref(), Some("completed"));
        assert!(
            reasoning
                .as_deref()
                .is_some_and(|r| r.starts_with("dispatch_parent_settled:")),
            "le motif doit porter le discriminant : {reasoning:?}"
        );
    })
    .await
    .expect("settles_terminal_dispatch_parent timed out");
}

/// **Test 2a (AC5) — un dispatch vivant n'est pas fauché.** L'enfant est
/// `delivered` (donc le parent est sélectionnable) mais porte un couple
/// `(pid, start_time)` vivant. La garde de mika#2156, réutilisée telle quelle,
/// épargne la row.
#[tokio::test]
async fn spares_parent_whose_dispatch_child_is_alive() {
    tokio::time::timeout(Duration::from_secs(10), async {
        if skip_off_linux() {
            return;
        }
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        let (pid, start_time) = own_live_process();
        let parent = seed_tracking_parent(&db, "suivi pilote vif", None).await;
        seed_callback_child(
            &db,
            &parent,
            "delivered",
            PAST_GRACE_SECS,
            Some(pid),
            Some(start_time),
        )
        .await;

        run_one_scan(&mut engine).await;

        let row = db.get_task(&parent).await.unwrap().unwrap();
        assert_eq!(
            row.status, "in_progress",
            "un processus de dispatch encore vivant épargne sa row de suivi"
        );
        assert_eq!(
            db.count_audit_events_by_tool_name(SETTLED_EVENT)
                .await
                .unwrap(),
            0,
            "épargner n'écrit rien : la row n'a pas bougé"
        );
    })
    .await
    .expect("spares_parent_whose_dispatch_child_is_alive timed out");
}

/// **Test 2b (AC5) — la divergence assumée avec le phantom sweep.** Un enfant
/// portant un PID sans `process_start_time` lisible est indistinguable d'un
/// enfant mort ; épargner dessus serait **permanent** et rendrait le fermeur
/// inerte sur toute la population anormale, c'est-à-dire exactement celle qu'il
/// existe pour fermer.
///
/// Sans ce test, une relecture qui « aligne » le fermeur sur le sweeper le
/// rendrait inerte sans faire rougir quoi que ce soit.
#[tokio::test]
async fn settles_even_when_a_child_has_no_readable_start_time() {
    tokio::time::timeout(Duration::from_secs(10), async {
        if skip_off_linux() {
            return;
        }
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        let (pid, _start_time) = own_live_process();
        let parent = seed_tracking_parent(&db, "suivi enfant illisible", None).await;
        // PID vivant, mais aucun `process_start_time` — le couple qui identifie
        // une *instance* de processus est incomplet.
        seed_callback_child(&db, &parent, "delivered", PAST_GRACE_SECS, Some(pid), None).await;

        run_one_scan(&mut engine).await;

        let row = db.get_task(&parent).await.unwrap().unwrap();
        assert_eq!(
            row.status, "completed",
            "`unusable_children > 0` ferme quand même — l'inertie reconduirait le défaut"
        );
        assert_eq!(
            db.count_audit_events_by_tool_name(SETTLED_EVENT)
                .await
                .unwrap(),
            1
        );
    })
    .await
    .expect("settles_even_when_a_child_has_no_readable_start_time timed out");
}

/// **Test 4 — le frère `completed`.** Sur une row callback, `completed` signifie
/// « le pilote est revenu, la livraison n'a pas encore eu lieu » ; `delivered`
/// est l'état terminal. Fermer ici fermerait la row avant que son tour de
/// verdict ait tourné. C'est le terme le plus facile à « simplifier » par erreur
/// en relecture.
#[tokio::test]
async fn defers_while_a_sibling_is_merely_completed() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        let parent = seed_tracking_parent(&db, "suivi deux enfants", None).await;
        seed_callback_child(&db, &parent, "delivered", PAST_GRACE_SECS, None, None).await;
        seed_callback_child(&db, &parent, "completed", PAST_GRACE_SECS, None, None).await;

        run_one_scan(&mut engine).await;

        let row = db.get_task(&parent).await.unwrap().unwrap();
        assert_eq!(
            row.status, "in_progress",
            "un enfant `completed` n'a pas encore livré — le parent doit attendre"
        );
        assert_eq!(
            db.count_audit_events_by_tool_name(SETTLED_EVENT)
                .await
                .unwrap(),
            0
        );
    })
    .await
    .expect("defers_while_a_sibling_is_merely_completed timed out");
}

/// **Test 6 — non-régression self_dev.** Une row self_dev dont l'enfant a livré
/// **sans** `pr_url` continue d'être `failed` par sa faucheuse d'origine
/// (#871), et n'est jamais `completed` par le nouveau fermeur. Les deux
/// mécanismes tournent dans la même passe de scan ; ce test dit qu'ils ne se
/// disputent pas la même row.
#[tokio::test]
async fn self_dev_orphan_is_still_failed_by_its_own_reaper() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        let parent = seed_tracking_parent(&db, "suivi self_dev sans PR", Some("self_dev")).await;
        seed_callback_child(&db, &parent, "delivered", PAST_GRACE_SECS, None, None).await;

        run_one_scan(&mut engine).await;

        let row = db.get_task(&parent).await.unwrap().unwrap();
        assert_eq!(
            row.status, "failed",
            "la faucheuse #871 garde sa population : livré sans `pr_url` ⇒ `failed`"
        );
        assert_eq!(
            db.count_audit_events_by_tool_name(SETTLED_EVENT)
                .await
                .unwrap(),
            0,
            "le fermeur mika#2405 n'a rien écrit sur cette row"
        );
        assert_eq!(
            db.count_audit_events_by_tool_name("task_engine_reaper")
                .await
                .unwrap(),
            1,
            "c'est bien la faucheuse d'origine qui a agi"
        );
    })
    .await
    .expect("self_dev_orphan_is_still_failed_by_its_own_reaper timed out");
}

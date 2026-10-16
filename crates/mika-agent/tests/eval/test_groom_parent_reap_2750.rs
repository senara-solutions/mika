//! Tests de bout en bout du balayage de la classe `groom` par la faucheuse de
//! parents orphelins (mika#2750).
//!
//! Le défaut fermé : un groom ready-label qui crashe une fois laissait sa tâche
//! `in_progress` **sans processus et sans re-drive**, indéfiniment. L'auto-feeder
//! continuait de la compter en vol et le rail se vidait en silence — mesuré le
//! 2026-10-16 sur mika#2736 (tâche `fef8cd36`, ~6 h, 36 lignes
//! `auto_feeder_pool_in_flight` la listant sous `stuck` sans aucun remède).
//!
//! Chaque test traverse `TaskEngine::tick()` — jamais la faucheuse directement —
//! parce que le câblage dans la branche `DB_SCAN_INTERVAL_TICKS` fait partie de
//! ce qui doit rester vrai : une faucheuse correcte mais non appelée se lit
//! exactement comme une faucheuse qui n'a rien trouvé.
//!
//! Recette de vérification par injection (obligatoire) : commenter la ligne
//! `self.reap_orphaned_parent_tasks("groom").await;` dans `tick()` —
//! `un_groom_dont_le_callback_a_delivre_quitte_in_progress` doit rougir, les deux
//! autres rester verts. Restaurer, relancer, les trois repassent au vert. Un test
//! qui ne rougit pas quand on retire ce qu'il mesure ne mesure rien.

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

const AGENT_ID: &str = "mika";

/// Les deux noms d'événement d'audit de `REAPER_CLASS_DISPOSITIONS`, et les deux
/// motifs d'échec qui vont avec. Écrits ici en **littéraux** à dessein : ce sont
/// un format de fil (les opérateurs groupent dessus), donc si la table de
/// production changeait de valeur ces assertions doivent rougir plutôt que
/// suivre.
const GROOM_EVENT: &str = "groom_parent_orphaned";
const GROOM_MOTIF: &str = "groom_callback_delivered_without_redispatch";
const IMPLEMENT_EVENT: &str = "task_engine_reaper";
const IMPLEMENT_MOTIF: &str = "callback_delivered_without_pr_url";

/// Au-delà de `REAPER_GRACE_SECONDS` (600 s).
const PAST_GRACE_SECS: i64 = 700;

struct NoopSender;

#[async_trait::async_trait]
impl MessageSender for NoopSender {
    async fn send(&self, _text: &str) -> anyhow::Result<SendOutcome> {
        Ok(SendOutcome::Delivered)
    }
}

async fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory DB");
    let db = AsyncDatabase::new_with_agent(db, AGENT_ID);
    db.register_agent(AGENT_ID, "Mika", "/tmp/mika")
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
        worktree_reap_stop_armed: AtomicBool::new(false),
        proactive_budget_reported: std::sync::Mutex::new(None),
    })
}

/// La scène du ticket, semée en une fois : le parent de suivi ready-label
/// (`manual` / `none` / `self_dev`, `in_progress`, metadata sans `pr_url`) et
/// son enfant callback de la classe demandée, `delivered` au-delà de la grâce.
///
/// `live_sibling` ajoute un frère `in_progress` — le « pilote vit encore » du
/// contrôle négatif, que la clause `NOT EXISTS` de la requête doit voir.
///
/// La classe est portée par l'**enfant** : c'est le terme sur lequel la requête
/// porte, parce qu'un parent réutilisé (mika#920) traîne la classe de son
/// dispatch d'origine.
async fn seed_groom_scene(db: &AsyncDatabase, dispatch_class: &str, live_sibling: bool) -> String {
    let new_child = |parent_id: &str, label: &str| NewTask {
        agent_id: AGENT_ID.to_string(),
        team_run_id: None,
        parent_task_id: Some(parent_id.to_string()),
        depth: 1,
        label: label.to_string(),
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
        dispatch_class: Some(dispatch_class.to_string()),
    };

    let parent = db
        .create_task(NewTask {
            agent_id: AGENT_ID.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: format!("suivi ready-label {dispatch_class} mika#2736"),
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
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: Some(dispatch_class.to_string()),
        })
        .await
        .expect("create ready-label parent");
    db.update_task_status(&parent, "in_progress")
        .await
        .expect("set parent in_progress");

    let child = db
        .create_task(new_child(&parent, "long_running:run_claude_pilot_groom"))
        .await
        .expect("create callback child");
    db.update_task_status(&child, "delivered")
        .await
        .expect("set child delivered");
    db.backdate_task_updated_at(&child, PAST_GRACE_SECS)
        .await
        .expect("backdate child updated_at");

    if live_sibling {
        let sibling = db
            .create_task(new_child(&parent, "long_running:run_claude_pilot_groom"))
            .await
            .expect("create live sibling");
        db.update_task_status(&sibling, "in_progress")
            .await
            .expect("set sibling in_progress");
    }

    parent
}

/// Pousser le moteur au-delà d'une passe de scan DB (60 ticks).
async fn run_one_scan(engine: &mut TaskEngine) {
    for _ in 0..60 {
        engine.tick().await;
    }
}

/// **Contrôle positif** — l'« Attendu » n°2 du ticket : la tâche quitte
/// `in_progress`, et un événement nommé dit pourquoi.
///
/// Le re-drive de l'« Attendu » n°1 suit gratuitement et hors de ce test :
/// `failed` sort la row de `idx_tasks_manual_active_ref_url`, l'auto-feeder cesse
/// de la compter en vol, le ticket garde son label `ready` et le balayage suivant
/// recrée une tâche neuve.
#[tokio::test]
async fn un_groom_dont_le_callback_a_delivre_quitte_in_progress() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let db = test_db().await;
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        let parent = seed_groom_scene(&db, "groom", false).await;

        assert_eq!(
            db.count_audit_events_by_tool_name(GROOM_EVENT)
                .await
                .unwrap(),
            0,
            "référence : aucun événement avant la passe"
        );

        run_one_scan(&mut engine).await;

        let row = db.get_task(&parent).await.unwrap().unwrap();
        assert_eq!(
            row.status, "failed",
            "un parent de groom dont le callback a délivré sans que personne ne \
             reprenne la main doit atteindre un état terminal"
        );
        assert_eq!(
            row.result.as_deref(),
            Some(GROOM_MOTIF),
            "le motif est celui de la classe groom, pas celui de la classe voisine"
        );

        let rows = db
            .get_audit_event_rows_by_tool_name(GROOM_EVENT)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "exactement une ligne d'audit");
        let (target_key, before, after, reasoning) = &rows[0];
        // La faucheuse écrit l'id nu — pas le `task:<id>` du fermeur mika#2405.
        assert_eq!(target_key, &parent);
        assert_eq!(before.as_deref(), Some("in_progress"));
        assert_eq!(after.as_deref(), Some("failed"));
        assert_eq!(reasoning.as_deref(), Some(GROOM_MOTIF));
    })
    .await
    .expect("un_groom_dont_le_callback_a_delivre_quitte_in_progress timed out");
}

/// **Contrôle négatif** — « pilote vivant → rien ne bouge », exigé par le
/// ticket. Un frère `in_progress` signifie qu'un vol est en cours sur ce parent ;
/// la clause `NOT EXISTS` de la requête diffère, et épargner n'écrit rien.
#[tokio::test]
async fn un_groom_dont_le_pilote_vit_encore_ne_bouge_pas() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let db = test_db().await;
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        let parent = seed_groom_scene(&db, "groom", true).await;

        run_one_scan(&mut engine).await;

        let row = db.get_task(&parent).await.unwrap().unwrap();
        assert_eq!(
            row.status, "in_progress",
            "un vol encore en cours sur ce parent l'épargne"
        );
        assert_eq!(
            db.count_audit_events_by_tool_name(GROOM_EVENT)
                .await
                .unwrap(),
            0,
            "épargner n'écrit rien : la row n'a pas bougé"
        );
    })
    .await
    .expect("un_groom_dont_le_pilote_vit_encore_ne_bouge_pas timed out");
}

/// **Non-régression de la classe voisine.** Les deux valeurs de la classe
/// `implement` sont lues par des requêtes d'opérateur existantes, et mika#2121
/// (KTD5) désigne ce site comme *sole production writer* de son motif. Sans ce
/// cas, un renommage accidentel de `REAPER_CLASS_DISPOSITIONS` passerait vert.
#[tokio::test]
async fn la_classe_implement_garde_son_motif_mot_pour_mot() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let db = test_db().await;
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        let parent = seed_groom_scene(&db, "implement", false).await;

        run_one_scan(&mut engine).await;

        let row = db.get_task(&parent).await.unwrap().unwrap();
        assert_eq!(row.status, "failed");
        assert_eq!(
            row.result.as_deref(),
            Some(IMPLEMENT_MOTIF),
            "le motif de la classe implement est un format de fil : mot pour mot"
        );
        assert_eq!(
            db.count_audit_events_by_tool_name(IMPLEMENT_EVENT)
                .await
                .unwrap(),
            1,
            "sous son nom d'événement d'origine"
        );
        assert_eq!(
            db.count_audit_events_by_tool_name(GROOM_EVENT)
                .await
                .unwrap(),
            0,
            "et jamais sous celui de la classe groom"
        );
    })
    .await
    .expect("la_classe_implement_garde_son_motif_mot_pour_mot timed out");
}

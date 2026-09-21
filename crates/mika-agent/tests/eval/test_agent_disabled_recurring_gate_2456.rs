//! Un agent désactivé ne produit aucun tour automatique récurrent — mika#2456.
//!
//! **Ce que la lecture du code a rectifié du ticket, et qui décide la forme du
//! correctif.** Le corps pose que « la porte `enabled` ne couvre que le chemin
//! interactif » et le commentaire 2 que ce knob est « lu par le chemin
//! interactif uniquement ». **Les deux sont faux : ce champ n'existait nulle
//! part.** `prompt::Identity` énumérait dix champs — `name`, `emoji`,
//! `reflection`, `heartbeat`, `kg`, `skills`, `tools`, `context`, `session`,
//! `curator` — et aucun `enabled` racine ; `MIKA_TEST_IDENTITY` porte `[kg]
//! enabled = false` et `[skills] nudge_enabled = false`, deux sous-clés de
//! sections sans rapport avec l'activité de l'agent. Il n'y avait rien à
//! *étendre*, il y avait une clé à *créer* — et à créer **à l'endroit exact où
//! le ticket croyait la lire**, sans quoi le correctif poserait un second knob
//! en laissant le premier inerte.
//!
//! Ce que le ticket établit correctement reste entier : le symptôme, le tableau
//! des récurrentes du commentaire 2 (`curator_review` n'avait **aucun** knob) et
//! le coût mesuré (~5,18 $/22 h, un tour heartbeat de 59 s finissant en
//! `reasoning budget exhausted` avec zéro texte visible).
//!
//! **Ce fichier porte la moitié « filet au tir » (§ 2.3–2.4).** La moitié
//! « garde à l'enregistrement » est épinglée in-crate, dans
//! `task_engine::tests::mika2456_*`, là où vit la fonction gardée.
//!
//! Quatre sondes, et la quatrième est le contrôle négatif sans lequel les trois
//! autres ne prouvent rien :
//!
//! - **V4** — une row `recurring_active` d'un agent désactivé est refusée avant
//!   tout appel LLM.
//! - **V4b** — après ce refus la row **reste** `recurring_active`, et la cadence
//!   suivante est re-refusée à l'identique. Sans cette assertion, § 2.4 serait
//!   une intention non tenue et rien ne distinguerait « le filet refuse » de
//!   « le filet annule en silence » — deux comportements dont seul le premier
//!   rend la population comptable.
//! - **V9** — un tour `run_skill` **non** récurrent d'un agent désactivé n'écrit
//!   **aucune** ligne `agent_recurring_gate` (la moitié mesurable de R-5).
//! - **V5** — ce même tour non récurrent **s'exécute** : le filet décide, il ne
//!   bloque pas.

use std::path::{Path, PathBuf};
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

/// `tool_name` de la ligne d'audit du filet. Écrit ici en dur **à dessein** : le
/// § 5 publie cette chaîne comme requête opérateur, donc la sonde doit rougir si
/// la constante de production bouge sans que la requête publiée soit mise à
/// jour.
const GATE_AUDIT_TOOL_NAME: &str = "agent_recurring_gate";

struct NoopSender;
#[async_trait::async_trait]
impl MessageSender for NoopSender {
    async fn send(&self, _text: &str) -> anyhow::Result<SendOutcome> {
        Ok(SendOutcome::Delivered)
    }
}

fn test_db() -> AsyncDatabase {
    AsyncDatabase::new_with_agent(Database::open_in_memory().expect("db"), AGENT_ID)
}

/// Un home d'agent portant exactement `body` comme `identity.toml`.
///
/// Le `TempDir` est **rendu** pour que l'appelant le garde vivant : le laisser
/// tomber supprime le répertoire, et un `identity.toml` absent se résout en
/// identité fail-closed, dont `enabled` vaut `true`. Une sonde qui laisserait
/// tomber le répertoire mesurerait donc silencieusement le chemin *activé* en
/// croyant mesurer le chemin *désactivé*.
fn agent_home(body: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tmp dir");
    std::fs::write(dir.path().join("identity.toml"), body).expect("write identity.toml");
    dir
}

fn dispatcher_with_home(db: AsyncDatabase, home: &Path) -> Arc<TaskDispatcher> {
    let settings_home = tempfile::tempdir().expect("tmp dir");
    let mut settings =
        mika_common::config::Settings::load(settings_home.path()).expect("load settings");
    // `Settings::load` lit l'environnement : un PAT présent sur la machine de
    // l'opérateur ferait sortir la sonde sur le réseau.
    settings.github_token = None;
    Arc::new(TaskDispatcher {
        db,
        tier: mika_common::home::AgentTier::Default,
        deployment: mika_common::home::Deployment::Unknown,
        llm: mika_common::llm::dummy_provider(),
        tools: Arc::new(default_tools()),
        skills: Arc::new(SkillRegistry::empty()),
        message_sender: Some(Arc::new(NoopSender)),
        home_dir: home.to_path_buf(),
        global_home_dir: PathBuf::from("/tmp/mika-test-global-home-absent-2456"),
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

/// Combien de lignes `agent_recurring_gate` la base porte.
async fn gate_audit_rows(db: &AsyncDatabase) -> i64 {
    db.count_audit_events_by_tool_name(GATE_AUDIT_TOOL_NAME)
        .await
        .expect("compter les lignes d'audit")
}

/// Combien d'appels LLM la base porte. Le filet doit refuser **avant** le
/// premier ; ce compteur est ce qui rend « zéro dollar » mesurable plutôt
/// qu'affirmé.
async fn llm_call_rows(db: &AsyncDatabase) -> u64 {
    db.query_llm_calls(mika_agent::db::LlmCallFilters::default(), 1, 1)
        .await
        .expect("compter les appels LLM")
        .1
}

/// Enregistre une récurrente **par le vrai chemin de production**, avec un home
/// activé — c'est-à-dire la row née avant que l'opérateur pose la clé, qui est
/// exactement la population que le filet existe pour voir (§ 2.3).
async fn seed_recurring(db: &AsyncDatabase, label: &str) -> String {
    let enabled = agent_home("name = \"Mika\"\n");
    mika_agent::task_engine::ensure_recurring_task(
        db,
        enabled.path(),
        label,
        "* * * * * *",
        "{\"trigger\":\"heartbeat\"}",
    )
    .await;

    db.get_tasks_by_status(vec!["recurring_active".to_string()])
        .await
        .expect("lire les récurrences")
        .into_iter()
        .find(|t| t.label == label)
        .map(|t| t.id)
        .expect("la récurrence doit être enregistrée")
}

/// **V4 + V4b.** Une row `recurring_active` d'un agent désactivé est refusée
/// avant tout appel LLM, **sans que la row soit mutée**, et la cadence suivante
/// est re-refusée à l'identique.
#[tokio::test]
async fn mika2456_v4_a_disabled_agent_refuses_the_fire_and_leaves_the_row_alone() {
    let db = test_db();
    let id = seed_recurring(&db, "heartbeat").await;

    let disabled = agent_home("name = \"Mika\"\nenabled = false\n");
    let dispatcher = dispatcher_with_home(db.clone(), disabled.path());
    let mut engine = TaskEngine::new(db.clone(), dispatcher);
    engine.startup_recovery().await.expect("startup recovery");

    // Le témoin du refus est la ligne d'audit : un tir refusé ne replanifie rien
    // et ne change aucun statut, donc il n'y a pas d'autre trace à lire. Sans
    // cette boucle d'arrêt, une sonde qui ne tirerait jamais serait verte pour
    // la mauvaise raison.
    let mut refused = false;
    for _ in 0..120 {
        engine.tick().await;
        if gate_audit_rows(&db).await > 0 {
            refused = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        refused,
        "le filet n'a jamais tiré en 6 s — la sonde ne mesurerait rien et ses \
         assertions passeraient pour la mauvaise raison"
    );

    // V4 — aucun appel LLM : le refus précède le premier.
    assert_eq!(
        llm_call_rows(&db).await,
        0,
        "le refus doit être posé AVANT tout appel LLM — c'est ce qui rend « zéro \
         dollar » littéral et non une intention"
    );

    // V4b — la row n'a pas bougé.
    let task = db
        .get_task(&id)
        .await
        .expect("relire la tâche")
        .expect("la tâche existe");
    assert_eq!(
        task.status, "recurring_active",
        "le filet refuse SANS muter la row : l'annuler effacerait sa propre \
         preuve (précédent `phantom_sweep_spared`, mika#2156) et ferait de \
         `dispatch_run_skill` une seconde autorité sur le cycle de vie des \
         récurrentes, à côté d'`ensure_recurring_task`"
    );

    // V4b (suite) — une seconde échéance est re-refusée à l'identique : toujours
    // aucun appel LLM, et la row toujours intacte.
    for _ in 0..20 {
        engine.tick().await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(llm_call_rows(&db).await, 0);
    let task = db
        .get_task(&id)
        .await
        .expect("relire la tâche")
        .expect("la tâche existe");
    assert_eq!(task.status, "recurring_active");
}

/// **V5 + V9 — non-régression R-5, et c'est le contrôle négatif porteur.**
///
/// Un tour `run_skill` **non** récurrent d'un agent désactivé s'exécute, et
/// n'écrit **aucune** ligne `agent_recurring_gate` au passage. Sans lui, V4 ne
/// distingue pas « la garde mord sur les récurrentes » de « la garde refuse
/// tout », et la portée de R-5 ne serait qu'une affirmation.
#[tokio::test]
async fn mika2456_v5_v9_a_non_recurring_run_skill_turn_is_untouched() {
    let db = test_db();

    // Une tâche `run_skill` ponctuelle : `trigger_type = "time"`, pas
    // `"recurring"`. C'est le discriminant exact que lit le filet.
    let task = NewTask {
        agent_id: AGENT_ID.to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: "one-shot".to_string(),
        trigger_type: "time".to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: Some(mika_agent::timestamp::now()),
        timeout_at: None,
        action_type: "run_skill".to_string(),
        action_config: "{\"trigger\":\"heartbeat\"}".to_string(),
        input_context: None,
        created_by_session: None,
        created_trace_id: None,
        reference_url: None,
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: None,
    };
    let id = db.create_task(task).await.expect("créer la tâche");

    let disabled = agent_home("name = \"Mika\"\nenabled = false\n");
    let dispatcher = dispatcher_with_home(db.clone(), disabled.path());
    let mut engine = TaskEngine::new(db.clone(), dispatcher);
    engine.startup_recovery().await.expect("startup recovery");

    // V5 — le tour s'exécute : il quitte l'état d'attente. Le filet décide, il
    // ne bloque pas.
    let mut ran = false;
    for _ in 0..120 {
        engine.tick().await;
        let status = db
            .get_task(&id)
            .await
            .expect("relire la tâche")
            .expect("la tâche existe")
            .status;
        if status != "pending" {
            ran = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        ran,
        "le tour `run_skill` NON récurrent doit s'exécuter même sur un agent \
         désactivé (R-5) — il est resté `pending` 6 s"
    );
    eprintln!(
        "DIAG status={:?} audit={}",
        db.get_task(&id).await.unwrap().unwrap().status,
        gate_audit_rows(&db).await
    );

    // V9 — la moitié mesurable de R-5 : le tour s'exécute (V5, ci-dessus) ET
    // n'écrit rien au passage.
    assert_eq!(
        gate_audit_rows(&db).await,
        0,
        "un tour `run_skill` NON récurrent d'un agent désactivé ne doit écrire \
         aucune ligne `{GATE_AUDIT_TOOL_NAME}` : la portée de R-5 est le chemin \
         nominal, et un tour qui s'exécute n'écrit rien de nouveau"
    );
}

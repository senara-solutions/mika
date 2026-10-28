//! Une reprise de callback ne rejoue pas une revue que le crédit refuse
//! (mika#2944).
//!
//! Mesuré le 2026-10-28 sur la tâche 26b8d5c7 (build de mika#2920) : quatre
//! tentatives, 60 tours de revue rejoués, aucune ne pouvait aboutir, solde à
//! 0,16 $ à 08:07Z, verdict parti à 09:05:44Z après une recharge à la main.
//! Le dispatcher rangeait le 402 en `http_402` et le réessayait comme une panne
//! passagère — 60 s, 120 s, puis la quarantaine au plafond horaire.
//!
//! Les trois cas ci-dessous sont les trois critères du ticket, et le troisième
//! est ce qui donne un sens aux deux premiers : sans lui, « la retenue mord »
//! est indistinguable de « l'échelle de mika#2179 a été retirée ».
//!
//! * **AC1** — un 402 retient la ligne : la reprise suivante ne rejoue **pas**
//!   la revue (aucun `callback_delivery_failed` de plus), elle paie une sonde ;
//! * **AC2** — le retour du crédit relance la revue dans le tour qui suit,
//!   sans geste à la main, et le résultat de build n'a jamais bougé ;
//! * **AC3** — contrôle négatif : un 429 garde le backoff de mika#2179, et
//!   aucune marque de crédit n'est posée.
//!
//! # Recette de vérification d'injection (OBLIGATOIRE, plan phase C2)
//!
//! Ces tests passeraient tous les trois sur un moteur qui n'appelle jamais le
//! LLM, donc chacun atteste d'abord que le tour a bien eu lieu
//! (`llm.calls_made() > 0` après la première reprise) — sans quoi « la revue
//! n'a pas été rejouée » serait vrai par vacuité.
//!
//! Deux injections ont été exécutées avant de livrer ce fichier, et leur
//! résultat est relevé et non prédit :
//!
//! 1. remplacer `status: 402` par `status: 429` dans [`credit_402`] — les cas 1
//!    et 2 passent au rouge sur le compte de `callback_delivery_credit_held`
//!    (0 au lieu de 1), parce que c'est alors l'échelle de mika#2179 qui
//!    s'applique ;
//! 2. neutraliser la ligne `if is_callback && self.hold_callback_on_exhausted_credit(…)`
//!    de `dispatch_resume_agent` — le cas 1 passe au rouge sur le compte de
//!    `callback_delivery_failed` (**2 au lieu de 1** : la revue est rejouée,
//!    c'est le défaut lui-même) et le cas 2 sur celui de
//!    `callback_delivery_credit_restored` (0 au lieu de 1).
//!
//! Le cas 3 reste **vert** sous les deux : c'est ce qui le qualifie comme
//! contrôle négatif plutôt que comme réplique des deux premiers. Il rougit, lui,
//! si la branche crédit de `record_callback_delivery_failure` cesse de tester
//! l'erreur et retient tout.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::credit_hold::{
    CREDIT_HELD_EVENT, CREDIT_RESTORED_EVENT, DELIVERY_CREDIT_HELD_AT_KEY,
    DELIVERY_CREDIT_PROBES_KEY, DELIVERY_CREDIT_RELEASED_AT_KEY,
};
use mika_agent::db::{Database, NewTask};
use mika_agent::messaging::{MessageSender, SendOutcome};
use mika_agent::skills::SkillRegistry;
use mika_agent::task_engine::dispatcher::TaskDispatcher;
use mika_agent::tools::default_tools;
use mika_common::llm::error::LlmError;
use mika_common::llm::mock::{MockLlmProvider, text_response};
use tempfile::TempDir;

const AGENT_ID: &str = "mika";

/// Le message exact d'OpenRouter, relevé dans `/var/log/mika/server.log` le
/// 2026-10-28. Il est là pour le lecteur : la classification lit la **variante**
/// et jamais ce texte (doctrine mika#2331), ce que le cas 3 vérifie en portant
/// le même message sous un 429.
const CREDIT_402_MSG: &str =
    "This request would exceed your available credits. Please add more credits.";

/// L'intervalle de retenue par défaut (`DEFAULT_CALLBACK_DELIVERY_CREDIT_HOLD_SECS`).
const HOLD_SECS: i64 = 900;

fn credit_402() -> LlmError {
    LlmError::HttpError {
        status: 402,
        message: CREDIT_402_MSG.to_string(),
        retryable: false,
    }
}

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

fn test_home() -> TempDir {
    let home = TempDir::new().expect("tmp home");
    std::fs::create_dir_all(home.path().join("skills")).expect("skills dir");
    std::fs::create_dir_all(home.path().join("data")).expect("data dir");
    std::fs::write(home.path().join("soul.md"), "").expect("soul.md");
    home
}

/// `cli_mode: false` est porteur — la livraison de callback est un chemin
/// serveur. `agent_lock: None` laisse le test enchaîner les reprises sans
/// modéliser la contention, qui est un autre axe (mika#2515).
fn dispatcher_with_mock(
    db: AsyncDatabase,
    home: &TempDir,
    llm: Arc<MockLlmProvider>,
) -> Arc<TaskDispatcher> {
    Arc::new(TaskDispatcher {
        db,
        tier: mika_common::home::AgentTier::Default,
        deployment: mika_common::home::Deployment::Unknown,
        llm,
        tools: Arc::new(default_tools()),
        skills: Arc::new(SkillRegistry::empty()),
        message_sender: Some(Arc::new(NoopSender)),
        home_dir: PathBuf::from(home.path()),
        global_home_dir: PathBuf::from("/tmp/mika-test-global-home-absent"),
        embedding_client: None,
        brave_api_key: None,
        gateway_url: None,
        internal_token: None,
        github_token: None,
        github_app: None,
        skills_dirty: Arc::new(AtomicBool::new(false)),
        agent_lock: None,
        cli_mode: false,
        settings: mika_common::config::Settings::test_defaults(),
        pr_reviews_posted: None,
        auto_pull_stop_armed: AtomicBool::new(false),
        worktree_reap_stop_armed: AtomicBool::new(false),
        proactive_budget_reported: std::sync::Mutex::new(None),
    })
}

/// Un callback de build `completed` portant un résultat de pilote — la forme
/// exacte de la ligne de l'incident.
async fn seed_build_callback(db: &AsyncDatabase) -> String {
    let id = db
        .create_task(NewTask {
            agent_id: AGENT_ID.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
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
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: None,
        })
        .await
        .expect("create callback row");

    db.update_task_completed(&id, Some(BUILD_RESULT))
        .await
        .expect("mark completed");

    id
}

/// Le résultat de build que la retenue doit préserver intact (AC2).
const BUILD_RESULT: &str = "claude-pilot completed (status: done).\nTurns: 28";

async fn count(db: &AsyncDatabase, event: &str) -> i64 {
    db.count_audit_events_by_tool_name(event)
        .await
        .expect("count audit events")
}

async fn field(db: &AsyncDatabase, id: &str, key: &str) -> Option<String> {
    db.get_task_metadata_field(id, key)
        .await
        .expect("read task metadata")
}

async fn next_fire_at_of(db: &AsyncDatabase, id: &str) -> Option<String> {
    db.get_task(id)
        .await
        .expect("read task")
        .expect("task exists")
        .next_fire_at
}

/// AC1 — la reprise d'une ligne retenue ne rejoue pas la revue.
///
/// Sur `main`, la seconde reprise relance un tour complet et écrit un second
/// `callback_delivery_failed` : c'est la boucle qui a brûlé 60 tours.
#[tokio::test]
async fn un_402_de_credit_retient_la_ligne_au_lieu_de_la_rejouer() {
    let db = test_db();
    let home = test_home();
    let llm = Arc::new(
        MockLlmProvider::builder()
            .error(credit_402())
            .error(credit_402())
            .build(),
    );
    let dispatcher = dispatcher_with_mock(db.clone(), &home, llm.clone());
    let task_id = seed_build_callback(&db).await;

    // Première reprise : la garde ne mord pas encore (rien n'est retenu), donc
    // la revue tourne et meurt sur le 402. C'est elle qui pose la retenue.
    let _ = dispatcher.dispatch(&task_id).await;

    assert!(
        llm.calls_made() > 0,
        "anti-vacuité : sans tour LLM, « la revue n'a pas été rejouée » serait \
         vrai de n'importe quel moteur"
    );
    assert_eq!(
        count(&db, "callback_delivery_failed").await,
        1,
        "la première reprise échoue et le dit — l'échelle de mika#2179 n'est pas retirée"
    );
    assert_eq!(
        count(&db, CREDIT_HELD_EVENT).await,
        1,
        "un 402 de crédit doit annoncer sa retenue"
    );
    assert!(
        field(&db, &task_id, DELIVERY_CREDIT_HELD_AT_KEY)
            .await
            .is_some(),
        "la marque de retenue doit être lisible sur la ligne, sans le journal"
    );
    assert_eq!(
        field(&db, &task_id, DELIVERY_CREDIT_PROBES_KEY).await,
        Some("1".to_string()),
        "la facture de la retenue se compte en sondes"
    );

    // AC1 — l'intervalle de retenue REMPLACE le backoff : 900 s et non 60 s.
    let held_fire_at = next_fire_at_of(&db, &task_id)
        .await
        .expect("la retenue arme le timer : c'est lui qui ramènera la sonde");
    let delay = mika_agent::timestamp::parse(&held_fire_at)
        .expect("next_fire_at parse")
        .signed_duration_since(chrono::Utc::now())
        .num_seconds();
    assert!(
        (HOLD_SECS - 60..=HOLD_SECS).contains(&delay),
        "le 402 doit armer l'intervalle de retenue ({HOLD_SECS} s) et non le backoff \
         de 60 s de mika#2179, trouvé {delay} s"
    );

    let calls_after_review = llm.calls_made();

    // Seconde reprise : la garde mord. Une sonde, pas une revue.
    let _ = dispatcher.dispatch(&task_id).await;

    assert_eq!(
        count(&db, "callback_delivery_failed").await,
        1,
        "LE défaut : la revue ne doit PAS être rejouée tant que le crédit n'a pas \
         changé — un second `callback_delivery_failed` signerait le tour rejoué"
    );
    assert_eq!(
        llm.calls_made(),
        calls_after_review + 1,
        "la reprise d'une ligne retenue coûte exactement un message"
    );
    assert_eq!(
        count(&db, CREDIT_HELD_EVENT).await,
        2,
        "la sonde a reconduit la retenue, et elle le dit"
    );
    assert_eq!(
        field(&db, &task_id, DELIVERY_CREDIT_PROBES_KEY).await,
        Some("2".to_string()),
        "une sonde par intervalle, et le compte dit combien"
    );

    // Le résultat du build n'a jamais bougé : retenu n'est pas suspendu.
    let task = db.get_task(&task_id).await.unwrap().unwrap();
    assert_eq!(task.status, "completed");
    assert_eq!(task.result.as_deref(), Some(BUILD_RESULT));
}

/// AC2 — le crédit revient, la revue repart dans le tour qui suit, sans geste
/// à la main.
#[tokio::test]
async fn le_retour_du_credit_relance_la_revue_sans_geste_a_la_main() {
    let db = test_db();
    let home = test_home();
    // 402 pour la revue qui pose la retenue, puis des réponses de texte : la
    // première est consommée par la sonde, les suivantes par la revue que son
    // succès relance. Deux pour la revue, parce qu'un tour de callback qui
    // finit sur du texte seul est repris une fois par la garde EndTurn
    // `callback_terminal_action` — comportement existant du chemin heureux.
    let llm = Arc::new(
        MockLlmProvider::builder()
            .error(credit_402())
            .responses(vec![
                text_response("ok"),
                text_response("Acknowledged the pilot result."),
                text_response("Acknowledged the pilot result."),
            ])
            .build(),
    );
    let dispatcher = dispatcher_with_mock(db.clone(), &home, llm.clone());
    let task_id = seed_build_callback(&db).await;

    let _ = dispatcher.dispatch(&task_id).await;
    assert!(llm.calls_made() > 0, "anti-vacuité : le tour a eu lieu");
    assert_eq!(count(&db, CREDIT_HELD_EVENT).await, 1, "ligne retenue");

    // La sonde passe : relâchement, puis la revue reprend dans le MÊME tour.
    let _ = dispatcher.dispatch(&task_id).await;

    assert_eq!(
        count(&db, CREDIT_RESTORED_EVENT).await,
        1,
        "le retour du crédit doit être annoncé — sinon une retenue levée est muette"
    );
    assert!(
        field(&db, &task_id, DELIVERY_CREDIT_RELEASED_AT_KEY)
            .await
            .is_some(),
        "la marque de relâchement est ce qui libère la ligne"
    );
    let task = db.get_task(&task_id).await.unwrap().unwrap();
    assert!(
        !mika_agent::credit_hold::is_held(task.metadata.as_deref()),
        "la ligne doit être libre après le relâchement — sinon la retenue survit \
         au retour du crédit et le verdict attend un geste à la main"
    );
    assert_eq!(
        task.status, "delivered",
        "AC2 — le verdict part dans le tour qui suit la sonde, sans recharge \
         constatée à la main ni reprise ultérieure"
    );
    assert_eq!(
        count(&db, "callback_delivery_failed").await,
        1,
        "l'échec de la première reprise, et lui seul : la revue relancée par la \
         sonde aboutit"
    );

    // Le `next_fire_at` de la retenue reste inscrit, et ce n'est pas un défaut :
    // c'est le comportement de mika#2179 (une livraison aboutie n'efface pas le
    // backoff de la tentative précédente), et il est inerte parce que la ligne a
    // quitté `completed` — le balayage de 60 s ne sélectionne plus que ce statut.
    assert!(
        next_fire_at_of(&db, &task_id).await.is_some(),
        "la marque du timer de retenue survit à la livraison, inerte"
    );
}

/// AC3, contrôle négatif — et c'est lui qui donne un sens aux deux cas
/// ci-dessus : sans lui, « la retenue mord » est indistinguable de « l'échelle
/// de mika#2179 a été retirée ».
///
/// Le 429 porte **le texte du 402** : la classification lit la variante et son
/// statut, jamais le message rendu (doctrine mika#2331).
#[tokio::test]
async fn un_429_garde_le_backoff_de_mika2179() {
    let db = test_db();
    let home = test_home();
    let llm = Arc::new(
        MockLlmProvider::builder()
            .error(LlmError::HttpError {
                status: 429,
                message: CREDIT_402_MSG.to_string(),
                retryable: true,
            })
            .error(LlmError::HttpError {
                status: 429,
                message: CREDIT_402_MSG.to_string(),
                retryable: true,
            })
            .build(),
    );
    let dispatcher = dispatcher_with_mock(db.clone(), &home, llm.clone());
    let task_id = seed_build_callback(&db).await;

    let _ = dispatcher.dispatch(&task_id).await;
    assert!(llm.calls_made() > 0, "anti-vacuité : le tour a eu lieu");

    let first = next_fire_at_of(&db, &task_id)
        .await
        .expect("le 429 garde le backoff de mika#2179");
    let delay = mika_agent::timestamp::parse(&first)
        .expect("next_fire_at parse")
        .signed_duration_since(chrono::Utc::now())
        .num_seconds();
    assert!(
        delay < HOLD_SECS - 60,
        "un 429 ne doit PAS être retenu un quart d'heure : backoff attendu ~60 s, \
         trouvé {delay} s"
    );

    // La revue est bien rejouée, et c'est le comportement voulu pour un 429.
    let _ = dispatcher.dispatch(&task_id).await;
    assert_eq!(
        count(&db, "callback_delivery_failed").await,
        2,
        "l'échelle de mika#2179 réessaie, et ce ticket ne la touche pas"
    );
    assert_eq!(
        count(&db, CREDIT_HELD_EVENT).await,
        0,
        "aucune retenue sur un 429"
    );
    assert_eq!(
        field(&db, &task_id, DELIVERY_CREDIT_HELD_AT_KEY).await,
        None,
        "aucune marque de crédit posée sur une panne passagère"
    );
}

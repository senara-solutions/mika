//! « Le créneau est-il libre ? » a une seule réponse (mika#2162).
//!
//! Le ticket mesure un tourniquet — huit réveils `deferred dispatch slot freed`
//! en cinq heures, dont deux suivis à la seconde d'un refus
//! `global_dispatch_active` — et l'impute à une divergence entre le **bail de
//! créneau**, expiré depuis 4 h 12, et la **garde de dispatch**.
//!
//! **La prémisse causale est fausse et ces tests l'épinglent plutôt que de la
//! reproduire.** Aucun chemin de reprise différée n'a jamais lu le bail : à HEAD
//! le seul appelant de production de `dispatch_slot_lease_holder` est le filet
//! L3b de mika#2169, qui n'émet jamais « slot freed ». La classe que le ticket
//! nomme — deux mécanismes répondant à une même question par deux mesures
//! différentes — était réelle, mais entre la garde de dispatch et le backstop de
//! promotion, et dans le sens inverse de celui décrit.
//!
//! D'où la forme de V1 : **le bail expiré y est un contrôle négatif**. Un bail
//! posé expiré (TTL = 0) atteste que ce n'est pas lui qui décide — si la reprise
//! différée le lisait, elle promouvrait ; elle ne promeut pas.
//!
//! Chaque test traverse `TaskEngine::tick()` — jamais
//! `promote_pending_deferred_if_idle` directement — parce que le câblage dans la
//! branche `DB_SCAN_INTERVAL_TICKS` fait partie de ce qui doit rester vrai : un
//! backstop correct mais non appelé se lit exactement comme un backstop qui n'a
//! rien trouvé à faire (classe mika#2205).
//!
//! # Recette de vérification par injection (obligatoire)
//!
//! - **V1/V2** — faire retourner `promote_pending_deferred_if_idle`
//!   immédiatement : `promotes_when_the_class_is_genuinely_free` (V2) doit
//!   rougir, `withholds_while_a_dispatch_older_than_the_lease_ttl_runs` (V1)
//!   reste verte — c'est précisément pourquoi V2 existe : sans elle, « ne promeut
//!   pas » serait satisfait par un backstop inerte.
//! - **V7** — retirer le champ `pending_wrappers` de l'événement, ou déplacer le
//!   comptage des wrappers après la lecture d'occupation :
//!   `says_it_withholds_only_when_a_wrapper_is_actually_waiting` rougit.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use std::collections::HashMap;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{Database, NewTask};
use mika_agent::messaging::{MessageSender, SendOutcome};
use mika_agent::skills::SkillRegistry;
use mika_agent::task_engine::dispatcher::TaskDispatcher;
use mika_agent::task_engine::engine::TaskEngine;
use mika_agent::tools::default_tools;

const AGENT_ID: &str = "mika-dev";

/// Le label exact qu'`agent::DEFERRED_DISPATCH_LABEL` porte, et sur lequel les
/// prédicats SQL apparient. Écrit en littéral à dessein : si la constante de
/// production changeait de valeur, ces tests doivent rougir plutôt que suivre.
const DEFERRED_LABEL: &str = "long_running:run_claude_pilot:deferred";

/// Le nom de l'événement U3. Même raison qu'au-dessus.
const WITHHELD_EVENT: &str = "deferred_promotion_withheld";

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
    db.register_agent(AGENT_ID, "Mika Dev", "/tmp/mika-dev")
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
        // `false`, et c'est porteur : `tick()` ne promeut les wrappers que hors
        // mode CLI (#264 — en CLI la TUI réclame les callbacks elle-même). Un
        // `true` ici rendrait V1 verte pour la mauvaise raison et V2 rouge, ce
        // qui est exactement la raison d'être du contrôle négatif.
        cli_mode: false,
        settings,
        pr_reviews_posted: None,
        auto_pull_stop_armed: AtomicBool::new(false),
        worktree_reap_stop_armed: AtomicBool::new(false),
        proactive_budget_reported: std::sync::Mutex::new(None),
    })
}

fn callback_task(parent_id: &str, label: &str, dispatch_class: Option<&str>) -> NewTask {
    NewTask {
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
        dispatch_class: dispatch_class.map(str::to_string),
    }
}

fn tracking_parent(label: &str) -> NewTask {
    NewTask {
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
        source: Some("self_dev".to_string()),
        metadata: None,
        r#type: None,
        dispatch_class: None,
    }
}

/// Un dispatch `implement` en vol : un parent de suivi et sa ligne de rappel
/// `pending`, telle que l'exécuteur l'écrit.
async fn seed_live_dispatch(db: &AsyncDatabase) -> String {
    let parent = db
        .create_task(tracking_parent("dispatch en vol"))
        .await
        .expect("create dispatch parent");
    db.update_task_status(&parent, "in_progress")
        .await
        .expect("parent in_progress");
    db.create_task(callback_task(
        &parent,
        "long_running:run_claude_pilot",
        Some("implement"),
    ))
    .await
    .expect("create callback child");
    parent
}

/// Un wrapper différé `pending` en attente de promotion.
async fn seed_pending_wrapper(db: &AsyncDatabase) -> String {
    let parent = db
        .create_task(tracking_parent("ticket en file"))
        .await
        .expect("create queued parent");
    db.create_task(callback_task(&parent, DEFERRED_LABEL, Some("implement")))
        .await
        .expect("create deferred wrapper")
}

/// Pousser le moteur au-delà d'une passe de scan DB (60 ticks).
async fn run_one_scan(engine: &mut TaskEngine) {
    for _ in 0..60 {
        engine.tick().await;
    }
}

// ---------------------------------------------------------------------------
// Capture d'événements `tracing` — même idiome que
// `test_context_scope_observability_2305.rs`, y compris le gardien de registre.
// ---------------------------------------------------------------------------

type Captured = Arc<Mutex<Vec<HashMap<String, String>>>>;

struct CapturingLayer {
    events: Captured,
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CapturingLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = HashMap::new();
        event.record(&mut FieldVisitor(&mut fields));
        self.events.lock().expect("capture mutex").push(fields);
    }
}

struct FieldVisitor<'a>(&'a mut HashMap<String, String>);

impl tracing::field::Visit for FieldVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
}

/// `tracing-core` bascule en mode « un seul dispatcher » quand un unique
/// dispatcher scopé est vivant, et un callsite enregistré pour la première fois
/// y cache alors son intérêt depuis le thread qui l'enregistre. Un second
/// dispatcher vivant maintient le registre en mode multi. Explication complète
/// dans `test_context_scope_observability_2305.rs`.
fn keep_registry_multi_dispatcher() {
    static KEEPER: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
    KEEPER.get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
}

fn capture() -> (tracing::subscriber::DefaultGuard, Captured) {
    use tracing_subscriber::layer::SubscriberExt;
    keep_registry_multi_dispatcher();
    let events: Captured = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CapturingLayer {
        events: Arc::clone(&events),
    });
    let guard = tracing::subscriber::set_default(subscriber);
    (guard, events)
}

fn withheld_events(events: &Captured) -> Vec<HashMap<String, String>> {
    events
        .lock()
        .expect("capture mutex")
        .iter()
        .filter(|f| f.get("event").map(String::as_str) == Some(WITHHELD_EVENT))
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// V1 / AC2 / AC5 — le cas mesuré
// ---------------------------------------------------------------------------

/// **V1 (AC2, AC5) — un dispatch en vol depuis plus que le TTL du bail ne
/// produit aucun « slot freed ».**
///
/// Le bail est posé **expiré** (TTL = 0) : c'est le contrôle négatif de tout le
/// ticket. Si la reprise différée lisait le bail, elle lirait « libre » et
/// promouvrait ; elle ne le lit pas, et l'occupation est établie par la ligne de
/// rappel active — la même chose que ce que lit la garde de dispatch.
///
/// Ce que ce test **ne** prouve **pas**, et le plan le dit : le résidu TOCTOU
/// d'un tick entre la promotion et le dispatch n'est pas fermé et ne peut pas
/// l'être sans mettre un spawn de processus dans une transaction SQLite
/// `IMMEDIATE`. Ce qui est fermé ici est le réveil stérile **par divergence de
/// prédicat**.
#[tokio::test]
async fn withholds_while_a_dispatch_older_than_the_lease_ttl_runs() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let db = test_db().await;
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        let live_parent = seed_live_dispatch(&db).await;
        let wrapper = seed_pending_wrapper(&db).await;

        // Le bail du dispatch en vol, posé EXPIRÉ (ttl_secs = 0). C'est l'état
        // relevé dans le ticket : acquis à 17:29:13Z, expiré à 17:31:13Z, lu à
        // 21:43:02Z avec le pilote toujours vivant.
        let claim = db
            .try_acquire_dispatch_slot("implement", &live_parent, Some("mika_dev"), 0, 1)
            .await
            .expect("acquire lease");
        assert!(claim.acquired(), "le bail est pris, puis meurt aussitôt");
        assert!(
            db.dispatch_slot_lease_holder("implement")
                .await
                .expect("read lease")
                .is_none(),
            "le bail doit se relire comme LIBRE — c'est le contrôle négatif : \
             tout ce qui suit se joue sans lui"
        );

        run_one_scan(&mut engine).await;

        let w = db
            .get_task(&wrapper)
            .await
            .expect("read wrapper")
            .expect("wrapper exists");
        assert_eq!(
            w.status, "pending",
            "le wrapper doit rester en attente : le créneau `implement` est \
             occupé par un dispatch vivant, bail expiré ou non"
        );
        assert_ne!(
            w.result.as_deref(),
            Some("deferred dispatch slot freed"),
            "AC5 — « slot freed » ne doit pas être écrit sur un créneau pris"
        );
    })
    .await
    .expect("withholds_while_a_dispatch_older_than_the_lease_ttl_runs timed out");
}

/// **V2 — contrôle négatif de V1 : sans dispatch actif, le backstop promeut.**
///
/// Sans lui, « ne promeut pas » serait satisfait par un backstop inerte — ou par
/// un câblage retiré de `tick()`. C'est la moitié qui distingue « le prédicat
/// décide » de « rien ne tourne ».
#[tokio::test]
async fn promotes_when_the_class_is_genuinely_free() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let db = test_db().await;
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        let wrapper = seed_pending_wrapper(&db).await;

        run_one_scan(&mut engine).await;

        let w = db
            .get_task(&wrapper)
            .await
            .expect("read wrapper")
            .expect("wrapper exists");
        assert_eq!(
            w.status, "completed",
            "créneau libre ⇒ le wrapper est promu (sans quoi V1 serait \
             satisfaite par un backstop inerte)"
        );
        assert_eq!(
            w.result.as_deref(),
            Some("deferred dispatch slot freed"),
            "le texte du résultat est un format de fil (lu par un test, cité \
             par un commentaire, affiché par `mika tasks get`) — mika#2162 ne \
             le change pas"
        );
    })
    .await
    .expect("promotes_when_the_class_is_genuinely_free timed out");
}

// ---------------------------------------------------------------------------
// V7 — la rétention est dite, et seulement quand elle existe
// ---------------------------------------------------------------------------

/// **V7, branche « présent » — un wrapper attend sur un créneau au cap : la
/// rétention est dite, avec les nombres qui la rendent lisible.**
///
/// Le `continue` que cet événement remplace était **muet** : « le backstop s'est
/// retenu parce que le créneau est pris » et « le backstop n'a rien trouvé à
/// promouvoir » se lisaient identiquement. INFO et non `debug!` : mika#2131 a
/// mesuré zéro occurrence d'un `debug!` du même module contre 184 d'un `info!`
/// voisin — le filtre déployé ne collecte pas ce niveau sur cette cible.
#[tokio::test]
async fn says_it_withholds_only_when_a_wrapper_is_actually_waiting() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let db = test_db().await;
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        seed_live_dispatch(&db).await;
        seed_pending_wrapper(&db).await;

        let (guard, events) = capture();
        run_one_scan(&mut engine).await;
        drop(guard);

        let withheld = withheld_events(&events);
        assert!(
            !withheld.is_empty(),
            "la rétention doit être dite — sinon elle est indistinguable d'un \
             backstop qui n'avait rien à promouvoir"
        );
        let fields = &withheld[0];
        assert_eq!(
            fields.get("dispatch_class").map(String::as_str),
            Some("implement")
        );
        assert_eq!(
            fields.get("pending_wrappers").map(String::as_str),
            Some("1"),
            "le compte de wrappers est ce qui rend la ligne actionnable"
        );
        assert_eq!(fields.get("active").map(String::as_str), Some("1"));
        assert_eq!(
            fields.get("agent_id").map(String::as_str),
            Some(AGENT_ID),
            "l'opérateur filtre par agent"
        );
        assert!(
            fields.contains_key("cap"),
            "le cap doit être porté : sans lui, `active` ne dit pas si la \
             rétention est légitime (champs vus : {:?})",
            fields.keys().collect::<Vec<_>>()
        );
    })
    .await
    .expect("says_it_withholds_only_when_a_wrapper_is_actually_waiting timed out");
}

/// **V7, branche « absent » — zéro wrapper en attente ⇒ zéro ligne.**
///
/// C'est le régime courant, et c'est la contrainte qui empêche cet événement de
/// devenir du bruit : une observabilité qui journalise tout le monde ne distingue
/// plus personne (mika#2131 AC7). Le créneau est **occupé** ici : seule l'absence
/// de wrapper doit faire taire la ligne.
#[tokio::test]
async fn stays_silent_when_no_wrapper_is_waiting() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let db = test_db().await;
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        seed_live_dispatch(&db).await;

        let (guard, events) = capture();
        run_one_scan(&mut engine).await;
        drop(guard);

        assert!(
            withheld_events(&events).is_empty(),
            "un créneau occupé sans wrapper en attente n'est pas une rétention \
             — rien à dire, et le dire serait une ligne par classe par tick, \
             pour toujours"
        );
    })
    .await
    .expect("stays_silent_when_no_wrapper_is_waiting timed out");
}

// ---------------------------------------------------------------------------
// V5 — AC3, non-régression du bail
// ---------------------------------------------------------------------------

/// **V5 (AC3) — deux tentatives concurrentes dans la même fenêtre de TTL
/// continuent d'être arbitrées : une seule obtient le créneau.**
///
/// C'est ce que le bail protège *vraiment*, et mika#2162 n'y touche pas —
/// `tests/dispatcher_contention.rs` reste inchangé et vert. Cette assertion-ci
/// est l'énoncé explicite que le correctif est neutre pour l'arbitrage de
/// course, posée à côté des tests qui changent le prédicat d'occupation pour
/// qu'un futur lecteur des deux n'ait pas à supposer laquelle des deux
/// questions a bougé.
#[tokio::test]
async fn the_lease_still_arbitrates_two_concurrent_claims() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let db = test_db().await;

        let first = db
            .try_acquire_dispatch_slot("implement", "claimant-a", Some("mika_dev"), 120, 1)
            .await
            .expect("first claim");
        assert!(first.acquired(), "le premier prétendant obtient le créneau");

        let second = db
            .try_acquire_dispatch_slot("implement", "claimant-b", Some("operator"), 120, 1)
            .await
            .expect("second claim");
        match second {
            mika_agent::db::SlotClaim::Held { holder_task_id, .. } => {
                assert_eq!(
                    holder_task_id, "claimant-a",
                    "le second est refusé ET le détenteur est nommé"
                );
            }
            other => panic!("le second prétendant devait être refusé, got {other:?}"),
        }
    })
    .await
    .expect("the_lease_still_arbitrates_two_concurrent_claims timed out");
}

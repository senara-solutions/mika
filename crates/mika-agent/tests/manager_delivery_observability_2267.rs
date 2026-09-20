//! mika#2267 — où vont les rapports Phase 1, et par quelle route (T4, T5, T6).
//!
//! # Ce que ces tests ferment
//!
//! Trois chemins d'écriture, et le journal n'en décrivait que deux. Le repli
//! après échec HTTP (`cadence.rs`, branche `Err`) écrivait le rapport au puits,
//! posait `delivered = true`, et **n'émettait aucun `manager_cycle_delivered`** :
//! le seul chemin où un rapport était écrit sans qu'aucune ligne ne dise qu'il
//! l'avait été, ni où. Et rien, nulle part, ne disait quelle route la prochaine
//! livraison prendrait.
//!
//! # Pourquoi le contrôle négatif est le test porteur
//!
//! Asserter qu'un deliverer en échec produit **une** ligne ne prouve rien tout
//! seul : le chemin « aucune URL posée » en produisait déjà une. Ce qui
//! distingue un événement de repli d'un événement de puits, c'est que les deux
//! populations restent **séparées** — `offline_sink` dit « aucune URL n'était
//! posée » (bring-up nominal, rien n'est en panne) et `offline_sink_fallback`
//! dit « une URL était posée et a échoué » (panne réelle). Les fondre
//! effacerait la distinction même que le ticket demande de faire. D'où les deux
//! assertions croisées de `T5` et son jumeau.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use chrono::{TimeZone, Utc};
use mika_agent::milestone_manager::{
    CiState, DeliveryBody, IssueState, ManagerConfig, MilestoneRef, MilestoneState, ProgressCounts,
    ROUTE_HTTP, ROUTE_OFFLINE_SINK, ROUTE_OFFLINE_SINK_FALLBACK, RecentActivity, ReportDeliverer,
    SinkDirSource, SubIssue, run_manager_cycle_with,
};

// ---- capture tracing (même forme que `tests/llm_call_attempt_2342.rs`) -----

#[derive(Debug, Clone)]
struct CapturedEvent {
    fields: HashMap<String, String>,
}

impl CapturedEvent {
    fn name(&self) -> &str {
        self.fields.get("event").map(String::as_str).unwrap_or("")
    }
    fn field(&self, key: &str) -> &str {
        self.fields.get(key).map(String::as_str).unwrap_or("")
    }
}

struct CapturingLayer {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CapturingLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = HashMap::new();
        let mut visitor = FieldVisitor(&mut fields);
        event.record(&mut visitor);
        if let Ok(mut events) = self.events.lock() {
            events.push(CapturedEvent { fields });
        }
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

fn capture() -> (
    tracing::subscriber::DefaultGuard,
    Arc<Mutex<Vec<CapturedEvent>>>,
) {
    use tracing_subscriber::layer::SubscriberExt;
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CapturingLayer {
        events: Arc::clone(&events),
    });
    let guard = tracing::subscriber::set_default(subscriber);
    (guard, events)
}

fn named(events: &Arc<Mutex<Vec<CapturedEvent>>>, name: &str) -> Vec<CapturedEvent> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e.name() == name)
        .cloned()
        .collect()
}

// ---- fixtures --------------------------------------------------------------

fn base_state() -> MilestoneState {
    MilestoneState {
        milestone_ref: MilestoneRef {
            repo: "senara-solutions/mika".into(),
            number: 1799,
        },
        title: "LC".into(),
        description: String::new(),
        state: IssueState::Open,
        created_at: String::new(),
        due_on: None,
        last_activity_at: Some("2026-08-20T00:00:00Z".into()),
        sub_issues: vec![SubIssue {
            number: 1802,
            title: "LC.2".into(),
            state: IssueState::Open,
            priority_rank: None,
            plan_present: true,
            branch_present: true,
            pr_number: Some(2002),
            pr_state: Some("open".into()),
            ci_state: CiState::Success,
            blockers: vec![],
            updated_at: "2026-08-20T00:00:00Z".into(),
            labels: vec![],
        }],
        progress: ProgressCounts {
            in_flight: 1,
            total: 1,
            ..Default::default()
        },
        recent_activity: vec![RecentActivity {
            at: "2026-08-20T00:00:00Z".into(),
            kind: "sub_issue_closed".into(),
            subject: "#1801".into(),
        }],
        executor_healthy: None,
    }
}

/// Le token de test. Une chaîne improbable, pour que l'assertion négative de
/// T6 ne puisse pas passer par coïncidence avec un mot du journal.
const SECRET: &str = "shh-3b91c2ae-never-on-the-wire";

fn mk_config(dir: &Path) -> ManagerConfig {
    ManagerConfig {
        target: MilestoneRef {
            repo: "senara-solutions/mika".into(),
            number: 1799,
        },
        github_token: None,
        heartbeat_interval: chrono::Duration::hours(6),
        poll_interval: chrono::Duration::minutes(5),
        silence_threshold_days: 3,
        delivery_url: Some("http://normal/deliver".into()),
        delivery_token: Some(SECRET.into()),
        escalation_url: Some("http://vincent/direct".into()),
        health_url: None,
        checkpoint_dir: dir.join("checkpoints"),
        offline_sink_dir: dir.join("puits"),
        sink_dir_source: SinkDirSource::Default,
    }
}

struct FailingDeliverer;

#[async_trait::async_trait]
impl ReportDeliverer for FailingDeliverer {
    async fn deliver(&self, _: &str, _: Option<&str>, _: &DeliveryBody) -> Result<()> {
        Err(anyhow::anyhow!("simulated endpoint refusal"))
    }
}

struct OkDeliverer;

#[async_trait::async_trait]
impl ReportDeliverer for OkDeliverer {
    async fn deliver(&self, _: &str, _: Option<&str>, _: &DeliveryBody) -> Result<()> {
        Ok(())
    }
}

// ---- T4 — les trois valeurs de `route` sont un format de fil ---------------

/// Ces trois chaînes atterrissent dans un journal que des sondes opérateur
/// grepent. Deux orthographes d'une même route couperaient une population en
/// deux sans le dire, et un renommage est une rupture à dater — pas une mise à
/// jour de test en silence.
#[test]
fn mika2267_delivery_route_names_are_a_wire_format() {
    assert_eq!(ROUTE_HTTP, "http");
    assert_eq!(ROUTE_OFFLINE_SINK, "offline_sink");
    assert_eq!(ROUTE_OFFLINE_SINK_FALLBACK, "offline_sink_fallback");

    // Et elles sont deux à deux distinctes — c'est la propriété qui porte la
    // séparation des populations, pas les valeurs elles-mêmes.
    assert_ne!(ROUTE_OFFLINE_SINK, ROUTE_OFFLINE_SINK_FALLBACK);
    assert_ne!(ROUTE_HTTP, ROUTE_OFFLINE_SINK);
}

// ---- T5 — le repli après échec cesse d'être muet ---------------------------

/// **T5.** Une URL posée dont la livraison échoue émet sa propre ligne, sous sa
/// propre route. Avant mika#2267 : zéro ligne sur ce chemin.
#[tokio::test]
async fn mika2267_http_failure_fallback_emits_its_own_route() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = mk_config(tmp.path());
    let (_guard, events) = capture();

    let outcome = run_manager_cycle_with(
        &cfg,
        base_state(),
        &FailingDeliverer,
        Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap(),
    )
    .await
    .unwrap();

    assert!(outcome.delivered, "le puits a bien capté le rapport");

    let delivered = named(&events, "manager_cycle_delivered");
    assert_eq!(
        delivered.len(),
        1,
        "exactement une ligne de livraison — avant le correctif, zéro : {delivered:?}"
    );
    assert_eq!(delivered[0].field("route"), ROUTE_OFFLINE_SINK_FALLBACK);
    assert!(
        !delivered[0].field("offline_sink_dir").is_empty(),
        "la ligne doit nommer OÙ le rapport a été écrit"
    );

    // Le contrôle qui compte : la population « aucune URL posée » n'est pas
    // touchée. Un correctif qui aurait réutilisé `offline_sink` aurait rendu ce
    // test vert tout en effaçant la distinction que le ticket demande.
    assert!(
        delivered
            .iter()
            .all(|e| e.field("route") != ROUTE_OFFLINE_SINK),
        "un échec d'endpoint ne doit jamais se compter comme un puits nominal"
    );

    // Et l'échec lui-même reste annoncé — la ligne de repli s'ajoute, elle ne
    // remplace pas le WARN qui porte l'erreur.
    assert_eq!(named(&events, "manager_cycle_delivery_failed").len(), 1);
}

/// Le jumeau de T5 : sans URL, c'est `offline_sink`, et jamais le repli.
///
/// Sans ce test, « la route de repli est émise » serait indistinguable de
/// « toutes les écritures au puits sont devenues des replis ».
#[tokio::test]
async fn mika2267_no_url_configured_is_the_nominal_sink_not_a_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = mk_config(tmp.path());
    cfg.delivery_url = None;
    cfg.escalation_url = None;
    let (_guard, events) = capture();

    run_manager_cycle_with(
        &cfg,
        base_state(),
        &OkDeliverer,
        Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap(),
    )
    .await
    .unwrap();

    let delivered = named(&events, "manager_cycle_delivered");
    assert_eq!(delivered.len(), 1, "{delivered:?}");
    assert_eq!(
        delivered[0].field("route"),
        ROUTE_OFFLINE_SINK,
        "aucune URL posée est un état de bring-up nominal, pas une panne"
    );
    assert_eq!(
        named(&events, "manager_cycle_delivery_failed").len(),
        0,
        "rien n'a échoué : rien ne doit être rapporté comme un échec"
    );
}

// ---- T6 — `manager_delivery_resolved` ------------------------------------

/// **T6.** L'événement porte le chemin résolu et sa provenance, et **jamais**
/// le token.
///
/// L'assertion négative balaie *tous* les champs, pas seulement celui du
/// token : une fuite arrive par le champ qu'on n'a pas pensé à regarder.
#[test]
fn mika2267_delivery_resolved_carries_the_resolved_path_and_never_the_token() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = mk_config(tmp.path());
    let (_guard, events) = capture();

    mika_agent::milestone_manager::spawn::emit_delivery_resolved(&cfg);

    let lines = named(&events, "manager_delivery_resolved");
    assert_eq!(lines.len(), 1, "une ligne par démarrage : {lines:?}");
    let line = &lines[0];

    // Le chemin résolu, et par quelle porte il a été décidé — les deux moitiés
    // sans lesquelles un opérateur ne sait pas où regarder.
    assert_eq!(
        line.field("offline_sink_dir"),
        cfg.offline_sink_dir.display().to_string()
    );
    assert_eq!(line.field("sink_dir_source"), "default");

    // Les deux routes que le cycle prendra, chacune pour sa moitié de sévérité.
    assert_eq!(line.field("route_normal"), ROUTE_HTTP);
    assert_eq!(line.field("route_escalation"), ROUTE_HTTP);

    // Le token est un BOOLÉEN. Jamais la valeur, jamais un préfixe, jamais une
    // longueur.
    assert_eq!(line.field("delivery_token_present"), "true");
    for (key, value) in &line.fields {
        assert!(
            !value.contains(SECRET),
            "la valeur du token a fuité dans le champ `{key}`"
        );
        // Un préfixe de huit caractères suffirait à identifier un secret.
        assert!(
            !value.contains(&SECRET[..8]),
            "un préfixe du token a fuité dans le champ `{key}`"
        );
    }
}

/// Une URL absente se lit comme telle : la ligne annonce le puits pour cette
/// moitié de sévérité, ce qui est la réponse directe à « sink offline vs
/// endpoint ».
#[test]
fn mika2267_delivery_resolved_names_the_sink_when_no_url_is_posted() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = mk_config(tmp.path());
    cfg.delivery_url = None;
    cfg.delivery_token = None;
    cfg.escalation_url = Some(String::new()); // posée mais vide == non posée
    cfg.sink_dir_source = SinkDirSource::Env;
    let (_guard, events) = capture();

    mika_agent::milestone_manager::spawn::emit_delivery_resolved(&cfg);

    let lines = named(&events, "manager_delivery_resolved");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].field("route_normal"), ROUTE_OFFLINE_SINK);
    assert_eq!(
        lines[0].field("route_escalation"),
        ROUTE_OFFLINE_SINK,
        "une URL vide vaut non posée — le prédicat est partagé avec `select_route`"
    );
    assert_eq!(lines[0].field("delivery_token_present"), "false");
    assert_eq!(lines[0].field("sink_dir_source"), "env");
}

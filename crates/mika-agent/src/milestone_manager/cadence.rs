//! Cadence — event-driven + 6h plancher heartbeat orchestration.
//!
//! **LECTURE SEULE.** The outbound side effect of *this file* is a `POST` to a
//! well-known delivery endpoint (Prime→sami→Vincent per D8 subsystem-2
//! pattern), or an offline sink write when the URL is unset. No GitHub
//! writes, no dispatch, no ticket mutations.
//!
//! Since mika#1990 the module has a **second** outbound channel, and it does not
//! live here: the liveness beat of [`super::liveness`], one light `POST` per
//! successful poll tick. Distinct endpoint, distinct cadence (5 min against 6 h),
//! distinct body — which is why `DeliveryBody` gained no field. The LECTURE-SEULE
//! contract is unchanged: a beat writes on no forge and says nothing about a
//! ticket.
//!
//! Cadence contract per brief § Ratification verdict 2:
//! - Event-driven: fires when observed milestone state differs from last snapshot.
//! - 6h plancher: fires on interval even with zero events ("l'absence d'event EST l'event").
//!
//! Escalation contract per brief § Ratification verdict 3:
//! - Normal delivery via `MIKA_MANAGER_DELIVERY_URL`.
//! - Severity::Blocked → `MIKA_MANAGER_ESCALATION_URL` (Vincent-direct route).

use super::{
    assessor::{Assessor, AssessorConfig},
    evaluator::{FlightSource, FlightVerdict, evaluate_flights, milestone_issue_urls},
    reader::{GhRunner, ProcessGhRunner, Reader},
    reporter::Reporter,
    types::{Assessment, CycleOutcome, MilestoneRef, MilestoneState, Severity},
    verdict_poster::{post_verdicts, write_is_armed},
};
use crate::auth_boundary_ledger::AuthBoundaryLedger;
use anyhow::Result;
use chrono::{DateTime, Utc};
use mika_common::auth_boundary::{AuthBoundaryError, AuthBoundaryKind};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

/// Configuration for a single manager cycle.
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    pub target: MilestoneRef,
    /// Optional GitHub token forwarded to `gh` subprocesses.
    pub github_token: Option<String>,
    /// Heartbeat interval — cycle always delivers if `now - last_delivery >= interval`.
    pub heartbeat_interval: chrono::Duration,
    /// How often the cadence loop wakes up to check for state changes and
    /// heartbeat expiry. Should be shorter than `heartbeat_interval` so
    /// event-driven cycles fire promptly; the cycle itself is a no-op when
    /// neither `state_changed` nor `heartbeat_fired` is true. Used only by
    /// the spawn loop in `spawn.rs`; unit tests of `run_manager_cycle_with`
    /// do not consult this field.
    pub poll_interval: chrono::Duration,
    pub silence_threshold_days: u32,
    pub delivery_url: Option<String>,
    pub delivery_token: Option<String>,
    pub escalation_url: Option<String>,
    /// Optional cm health endpoint — if reachable, populates
    /// `MilestoneState::executor_healthy`. Unset → skip.
    pub health_url: Option<String>,
    /// Directory to store per-milestone checkpoint files.
    pub checkpoint_dir: PathBuf,
    /// Directory to write offline sink reports when delivery URLs are unset.
    pub offline_sink_dir: PathBuf,
    /// Par quelle porte `offline_sink_dir` a été décidé (mika#2267 C3).
    ///
    /// Porté sur la config plutôt que re-résolu au moment de l'émission de
    /// `manager_delivery_resolved` : une seconde lecture de l'environnement
    /// pourrait rendre une provenance qui ne décrit pas le chemin réellement
    /// en vigueur — et une provenance fausse est strictement pire qu'aucune
    /// provenance (mika#2293).
    pub sink_dir_source: crate::milestone_manager::sink_dir::SinkDirSource,
    /// Endpoint du battement de vivacité, **déclaré** par
    /// `MIKA_MANAGER_LIVENESS_URL` (mika#1990).
    ///
    /// Jamais composé depuis `delivery_url` ni depuis `health_url` : les trois
    /// URLs désignent trois routes de formes et de sémantiques distinctes, et
    /// `health_url` est même de sens **inverse** (on y lit la santé de
    /// l'exécuteur ; ici on écrit la nôtre). Voir
    /// [`super::liveness`] pour le raisonnement complet.
    ///
    /// Absente ou vide ⇒ canal désarmé : zéro POST, zéro erreur. C'est aussi le
    /// rollback — retirer la variable désarme sans redéploiement.
    pub liveness_url: Option<String>,
}

// ---- mika#2267 C4 — `route` est un format de fil ---------------------------

/// Le rapport est parti par HTTP et la livraison a réussi.
pub const ROUTE_HTTP: &str = "http";
/// Aucune URL applicable n'était posée : le rapport est allé au puits.
/// **État de bring-up nominal — rien n'est en panne.**
pub const ROUTE_OFFLINE_SINK: &str = "offline_sink";
/// Une URL était posée, la livraison a échoué, le rapport est allé au puits.
/// **Panne réelle.**
///
/// Distinct de [`ROUTE_OFFLINE_SINK`] à dessein : les deux populations disent
/// des choses opposées et doivent rester comptables séparément. Les fondre
/// effacerait très exactement la distinction que mika#2267 demande de faire —
/// « sink offline vs endpoint, à déterminer ».
pub const ROUTE_OFFLINE_SINK_FALLBACK: &str = "offline_sink_fallback";

/// Une URL est-elle routable ? (posée et non vide.)
///
/// Lecteur unique du prédicat : `select_route` et l'événement
/// `manager_delivery_resolved` l'appellent tous deux, donc l'événement ne peut
/// pas annoncer une route que le cycle ne prendra pas.
pub(crate) fn url_is_routable(url: Option<&str>) -> bool {
    matches!(url, Some(u) if !u.is_empty())
}

/// Le dernier segment de chemin qui fait d'une URL un endpoint de battement.
///
/// Exporté comme littéral pour que la ligne ERROR de la garde porte
/// `matched_segment` **par construction** — le prédicat ne reconnaît que
/// celui-ci, donc le champ ne peut pas devenir un canal de fuite d'URL
/// (mika#2697 § Observabilité).
pub(crate) const HEARTBEAT_PATH_SEGMENT: &str = "heartbeat";

/// Cette URL désigne-t-elle un endpoint de **battement** ? (mika#2697)
///
/// Une `MIKA_MANAGER_DELIVERY_URL` qui vise `…/api/v1/agents/<e>/heartbeat`
/// enregistre un tick, **ne conserve pas le corps**, et répond `201` pour
/// n'importe quel nom d'entité. Le POST réussit donc — et c'est précisément la
/// panne : [`select_route`] ne reprend le puits que lorsque l'URL est absente
/// ou que la livraison **échoue**, et un `2xx` ferme les deux portes à la fois.
/// Dix-sept jours de rapports écrits puis jetés, sous un journal qui disait
/// « livré ». Le canal de vie a sa propre variable depuis mika#1990
/// (`MIKA_MANAGER_LIVENESS_URL`) ; c'est **elle** qui doit viser cet endpoint.
///
/// # Le jugement porte sur la forme du chemin, jamais sur une liste d'URLs
///
/// Une liste d'endpoints cm connus serait exacte le jour de l'atterrissage et
/// fausse au premier déploiement cm suivant. D'où trois choix, chacun porteur :
///
/// - **Sur le chemin, pas sur la chaîne entière.**
///   `https://reports.example/ingest?from=heartbeat` est une route de rapport,
///   et `https://heartbeat.example.com/reports` aussi — un
///   `contains("heartbeat")` refuserait les deux.
/// - **En position terminale, pas « contient un segment ».** La règle décrit ce
///   qu'un endpoint *est*, et `…/heartbeat/reports` ne l'est pas.
/// - **`heartbeat` seul**, pas `heartbeats` ni `ping` ni `alive` : cm n'expose
///   que cette forme, et le reste serait de la spéculation sur des endpoints
///   qui n'existent pas. La fonction est à un seul endroit — le jour où cm en
///   expose une autre, le geste est d'une ligne.
///
/// # Une URL non parsable rend `false`, et c'est une décision
///
/// Refuser ce qu'on ne sait pas lire ferait de ce prédicat un **second** mode de
/// panne. Une URL illisible partira en `http`, le POST échouera, et le repli
/// [`ROUTE_OFFLINE_SINK_FALLBACK`] la rattrapera — route déjà livrée, déjà
/// nommée, déjà testée.
pub(crate) fn url_is_heartbeat_endpoint(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    parsed
        .path_segments()
        .and_then(|mut segments| segments.rfind(|s| !s.is_empty()))
        .is_some_and(|last| last.eq_ignore_ascii_case(HEARTBEAT_PATH_SEGMENT))
}

/// Le nom de fil de la route qu'une sévérité prendra, vu l'URL applicable.
pub(crate) fn route_name_for(url: Option<&str>) -> &'static str {
    if url_is_routable(url) {
        ROUTE_HTTP
    } else {
        ROUTE_OFFLINE_SINK
    }
}

/// Trait boundary for report delivery — HTTP in production, in-memory in tests.
#[async_trait::async_trait]
pub trait ReportDeliverer: Send + Sync {
    /// Send a report to the given URL. `token` is optional bearer auth.
    /// Returns `Ok(())` on 2xx delivery; `Err` on any other outcome.
    async fn deliver(&self, url: &str, token: Option<&str>, body: &DeliveryBody) -> Result<()>;
}

/// Wire body posted to the delivery endpoint. All fields are read-derived.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryBody {
    pub milestone_ref: MilestoneRef,
    pub severity: Severity,
    pub report_markdown: String,
    pub assessment: Assessment,
    pub generated_at: String,
    pub cycle_kind: CycleKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CycleKind {
    Event,
    Heartbeat,
    /// Cycle was triggered by an event AND the heartbeat window elapsed — both.
    EventAndHeartbeat,
}

/// Persisted checkpoint per milestone. Digest of the observed sub-issue set
/// used to detect state change without needing to re-serialize the full
/// `MilestoneState` on each cycle.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MilestoneCheckpoint {
    /// Order-insensitive digest of `(number, state, pr_state, ci_state, blockers)`.
    pub state_digest: String,
    /// ISO 8601 UTC of last successful delivery (any URL, any sink).
    pub last_delivered_at: Option<String>,
    /// ISO 8601 UTC of the last observed milestone state.
    pub last_observed_at: Option<String>,
    /// Les `tasks.id` des vols dont le verdict a **effectivement** été posté
    /// sur son ticket (mika#2698 vol 2, AC2).
    ///
    /// C'est le porteur de l'idempotence, et il est ici plutôt que dans une
    /// relecture des commentaires de la forge parce que le ticket n'autorise
    /// qu'**un** geste, et que ce geste est une écriture.
    ///
    /// `#[serde(default)]` : un checkpoint écrit avant ce vol se relit en liste
    /// vide. Le premier cycle après déploiement reposte donc les verdicts des
    /// vols encore dans sa fenêtre — borne connue, et préférable à l'inverse
    /// (un verdict perdu pour toujours).
    #[serde(default)]
    pub posted_flight_task_ids: Vec<String>,
}

// ---- mika#1949 U3 — the manager delivery boundary -------------------------

/// The entity names carried by every `manager_to_delivery` audit row. Held as
/// constants so the row's `target_key` and the runbook agree by construction.
pub const DELIVERY_BOUNDARY_FROM: &str = "manager";
pub const DELIVERY_BOUNDARY_TO: &str = "delivery";

/// What a delivery attempt did, in the only terms the auth boundary cares
/// about.
///
/// Deliberately three variants, not a status code. The boundary's question is
/// "was this a credential problem, a reachability problem, or neither" — and a
/// caller handed a `u16` would have to re-derive that answer, differently, at
/// each site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryFailureKind {
    /// The far side answered and refused the credential (401 or 403).
    ///
    /// Both codes, on purpose: mika answers 401 and cm answers 403 on a token
    /// refusal, and mika#1949 KTD2 arbitrated that divergence rather than
    /// retrofitting it. A classifier that recognised only one would go blind
    /// against whichever peer it was not written for.
    CredentialRefused,
    /// The far side never answered, so no authentication verdict exists.
    Unreachable,
    /// Anything else — a 500, a malformed body. Not an authentication signal,
    /// and deliberately not reported as one.
    Other,
}

/// A delivery failure that still remembers which of the three it was.
///
/// Carried inside `anyhow::Error` rather than widening the `ReportDeliverer`
/// trait: every test double in this module returns `anyhow::Result<()>`, and a
/// signature change would have rewritten them all to gain nothing. Callers
/// that care downcast; callers that do not see the same message they saw
/// before mika#1949.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct DeliveryError {
    pub kind: DeliveryFailureKind,
    pub message: String,
    /// The underlying transport error, kept as a `source` rather than flattened
    /// into `message`.
    ///
    /// `reqwest::Error`'s own `Display` is generic — "error sending request for
    /// url (...)" — and the actionable half (DNS failure, TLS handshake,
    /// connection refused) lives in *its* source chain. Before mika#1949 the
    /// `?` operator carried that chain into `anyhow` for free; flattening it
    /// with `format!` would have thrown away exactly the transport
    /// diagnosability this ticket exists to add, on the way to adding
    /// credential diagnosability. `{:#}` on the resulting `anyhow::Error` now
    /// renders both halves.
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl DeliveryError {
    /// Build the failure already boxed as an `anyhow::Error`, which is what
    /// every call site wants. Named `raise` rather than `new` because it does
    /// not return `Self` — clippy is right that a `new` returning something
    /// else reads as a mistake.
    pub fn raise(kind: DeliveryFailureKind, message: impl Into<String>) -> anyhow::Error {
        anyhow::Error::new(Self {
            kind,
            message: message.into(),
            source: None,
        })
    }

    /// As [`DeliveryError::raise`], preserving the underlying transport error.
    pub fn raise_from(
        kind: DeliveryFailureKind,
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> anyhow::Error {
        anyhow::Error::new(Self {
            kind,
            message: message.into(),
            source: Some(Box::new(source)),
        })
    }
}

/// Read a delivery outcome as an auth-boundary observation, or `None` when it
/// was not an authentication event.
///
/// # Why the token is inspected only on a refusal
///
/// The plan phrases the mapping as "absent token to `Missing`, empty to
/// `Empty`". Applied unconditionally that would fire on every *successful*
/// delivery to an endpoint that requires no bearer at all — the offline-sink
/// and open-endpoint configurations both do — and an alarm that fires when
/// nothing is wrong is how a ledger stops being read. So the absent/empty
/// distinction is drawn where it is actually diagnostic: the far side refused
/// us, and the reason is that we presented nothing, presented blank, or
/// presented something it did not accept.
///
/// # Why `var_is_set` is a separate argument
///
/// `ManagerConfig::delivery_token` cannot tell unset from set-and-blank:
/// `read_string_env` (`spawn.rs`) maps `Ok(v) if !v.trim().is_empty()` and
/// collapses everything else to `None`. Reading only that field would report a
/// `MIKA_MANAGER_DELIVERY_TOKEN=""` as `Missing` — telling the operator the
/// variable is not configured when in fact it is configured with a lost value.
/// Those are two different fixes, which is the entire reason `Empty` is a
/// distinct kind. The caller supplies the raw presence of the variable so the
/// distinction survives; passing it in rather than reading the environment
/// here keeps the function pure and testable.
pub(crate) fn classify_delivery_auth(
    token: Option<&str>,
    var_is_set: bool,
    outcome: &Result<()>,
) -> Option<AuthBoundaryError> {
    let err = outcome.as_ref().err()?;
    let delivery = err.downcast_ref::<DeliveryError>()?;
    let kind = match delivery.kind {
        DeliveryFailureKind::CredentialRefused => match token {
            // The config dropped it. Set-but-blank is `Empty`; genuinely
            // absent is `Missing`.
            None if var_is_set => AuthBoundaryKind::Empty,
            None => AuthBoundaryKind::Missing,
            // Defensive: a `ManagerConfig` built directly (tests, future
            // callers) can still carry a blank string.
            Some(t) if t.trim().is_empty() => AuthBoundaryKind::Empty,
            Some(_) => AuthBoundaryKind::Rejected,
        },
        DeliveryFailureKind::Unreachable => AuthBoundaryKind::Unreachable,
        DeliveryFailureKind::Other => return None,
    };
    Some(AuthBoundaryError::new(
        crate::milestone_manager::spawn::ENV_DELIVERY_TOKEN,
        DELIVERY_BOUNDARY_FROM,
        DELIVERY_BOUNDARY_TO,
        kind,
    ))
}

/// Reqwest-backed HTTP deliverer. Fire-and-forget with warn-on-failure —
/// the parent cycle propagates the error but does not retry.
pub struct HttpReportDeliverer {
    client: reqwest::Client,
}

impl HttpReportDeliverer {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }
}

impl Default for HttpReportDeliverer {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ReportDeliverer for HttpReportDeliverer {
    async fn deliver(&self, url: &str, token: Option<&str>, body: &DeliveryBody) -> Result<()> {
        let mut req = self.client.post(url).json(body);
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        // mika#1949 U3: the failure keeps its class, and the transport error
        // keeps its own source chain. The non-2xx message text is unchanged
        // from before this ticket; the transport arm gains a `delivery failed:`
        // prefix it did not have, because it previously travelled as a bare
        // `?`-propagated `reqwest::Error`.
        let res = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                let kind = if e.is_connect() || e.is_timeout() {
                    DeliveryFailureKind::Unreachable
                } else {
                    DeliveryFailureKind::Other
                };
                return Err(DeliveryError::raise_from(
                    kind,
                    format!("delivery failed: {e}"),
                    e,
                ));
            }
        };
        if !res.status().is_success() {
            let status = res.status();
            let refused = matches!(status.as_u16(), 401 | 403);
            let text = res.text().await.unwrap_or_default();
            let kind = if refused {
                DeliveryFailureKind::CredentialRefused
            } else {
                DeliveryFailureKind::Other
            };
            return Err(DeliveryError::raise(
                kind,
                format!("delivery failed: {status} — {text}"),
            ));
        }
        Ok(())
    }
}

/// Compute a stable digest over the milestone state that changes when
/// anything an operator cares about changes. Deliberately not a full hash
/// of the JSON — we only key on structurally meaningful fields to avoid
/// spurious "changed" signals on updated_at flapping.
pub fn state_digest(state: &MilestoneState) -> String {
    let mut items: Vec<String> = state
        .sub_issues
        .iter()
        .map(|s| {
            format!(
                "{}:{:?}:{:?}:{:?}:{}",
                s.number,
                s.state,
                s.pr_state.as_deref().unwrap_or(""),
                s.ci_state,
                s.blockers
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join("+")
            )
        })
        .collect();
    items.sort();
    let joined = items.join("|");
    // Use a simple non-cryptographic hash — content diff, not integrity.
    format!("{:x}", seahash_like(&joined))
}

/// Small deterministic mixer — pure Rust, no crypto dep. Sufficient for
/// change-detection (collision resistance is a nice-to-have here, not a
/// hard invariant — a colliding digest at worst causes a missed event
/// signal, which the 6h heartbeat backstops).
fn seahash_like(input: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in input.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Run a single manager cycle: read → assess → decide-deliver → return outcome.
///
/// This function is unit-testable end-to-end when callers inject a
/// `Reader::read_with_runner`-driven `MilestoneState` fixture, an
/// injectable clock, and an in-memory `ReportDeliverer`.
///
/// Production wiring lives in `spawn_manager_loop` (below).
pub async fn run_manager_cycle_with(
    cfg: &ManagerConfig,
    state: MilestoneState,
    deliverer: &dyn ReportDeliverer,
    now: DateTime<Utc>,
) -> Result<CycleOutcome> {
    run_manager_cycle_in(cfg, state, &CycleContext::new(deliverer, now)).await
}

/// Everything a cycle needs that is neither its config nor the state it read
/// from GitHub (mika#1949 U3/U4).
///
/// Added as a struct rather than four more positional parameters: the two
/// mika#1949 additions are optional wiring, and `run_manager_cycle_with`'s
/// existing four-argument shape is called from a dozen tests that have no
/// ledger and no notes to supply.
pub struct CycleContext<'a> {
    pub deliverer: &'a dyn ReportDeliverer,
    pub now: DateTime<Utc>,
    /// Where an observed auth-boundary failure is recorded. `None` = not wired.
    pub auth_ledger: Option<&'a Arc<dyn AuthBoundaryLedger>>,
    /// Auth-boundary failures observed on *previous* cycles, for this report to
    /// carry. A failure observed on this cycle's delivery cannot appear in the
    /// report it was delivering — the report is composed first — so the loop
    /// hands them forward.
    pub auth_notes: &'a [crate::milestone_manager::reporter::AuthBoundaryNote],
    /// D'où viennent les vols terminés à évaluer (mika#2698). `None` = non
    /// câblée, et le rapport est alors identique à celui d'avant mika#2698 :
    /// aucun verdict, donc pas de section.
    pub flight_source: Option<&'a Arc<dyn FlightSource>>,
    /// Par quoi passe le seul geste d'écriture du manager (mika#2698 vol 2).
    ///
    /// Réutilise l'exécutant `gh` déjà injectable du module (mika#1975) plutôt
    /// qu'une nouvelle injection depuis `server/mod.rs` : les signatures de
    /// `spawn_manager_cycle_task` et du serveur restent intactes, pour aucune
    /// capacité de test perdue. `None` = non câblé, et aucun verdict n'est
    /// alors posté quel que soit l'état de l'interrupteur.
    pub gh_runner: Option<&'a dyn GhRunner>,
}

impl<'a> CycleContext<'a> {
    pub fn new(deliverer: &'a dyn ReportDeliverer, now: DateTime<Utc>) -> Self {
        Self {
            deliverer,
            now,
            auth_ledger: None,
            auth_notes: &[],
            flight_source: None,
            gh_runner: None,
        }
    }

    pub fn with_auth_ledger(mut self, ledger: Option<&'a Arc<dyn AuthBoundaryLedger>>) -> Self {
        self.auth_ledger = ledger;
        self
    }

    pub fn with_auth_notes(
        mut self,
        notes: &'a [crate::milestone_manager::reporter::AuthBoundaryNote],
    ) -> Self {
        self.auth_notes = notes;
        self
    }

    pub fn with_flight_source(mut self, source: Option<&'a Arc<dyn FlightSource>>) -> Self {
        self.flight_source = source;
        self
    }

    pub fn with_gh_runner(mut self, runner: Option<&'a dyn GhRunner>) -> Self {
        self.gh_runner = runner;
        self
    }
}

/// Les verdicts des vols terminés depuis le cycle précédent (mika#2698, vol 1).
///
/// Bornée par `since` — `checkpoint.last_observed_at`, c'est-à-dire exactement
/// « depuis le cycle précédent » du ticket. Au premier cycle il n'y a pas de
/// checkpoint, donc pas de borne : la source rend alors tous les vols terminés
/// sur les tickets du milestone, ce qui est la lecture honnête de « depuis
/// toujours » et non un silence.
///
/// Ne rend jamais d'erreur : une source en panne rend `vec![]` (et son propre
/// WARN), donc un rapport sans section plutôt qu'un cycle mort.
async fn collect_flight_verdicts(
    ctx: &CycleContext<'_>,
    state: &MilestoneState,
    checkpoint: Option<&MilestoneCheckpoint>,
) -> Vec<FlightVerdict> {
    let Some(source) = ctx.flight_source else {
        return Vec::new();
    };
    let issue_urls = milestone_issue_urls(state);
    if issue_urls.is_empty() {
        return Vec::new();
    }
    let since = checkpoint.and_then(|c| c.last_observed_at.as_deref());
    let flights = source.finished_implement_flights(since, &issue_urls).await;
    evaluate_flights(&flights, &state.sub_issues)
}

/// Combien de vols postés le checkpoint retient (mika#2698 vol 2).
///
/// Un checkpoint est un fichier JSON relu à chaque cycle : la liste ne peut pas
/// croître sans borne. 500 couvre très largement la fenêtre d'un milestone, et
/// la conséquence d'un débordement est **nommée** plutôt que silencieuse — le
/// vol le plus ancien sort de la liste, et son verdict pourrait être reposté sur
/// son ticket. Un doublon visible, jamais un verdict perdu.
const POSTED_FLIGHTS_RETAINED: usize = 500;

/// Le seul geste d'écriture du cycle (mika#2698 vol 2) — et le seul endroit du
/// cycle qui consulte l'interrupteur.
///
/// Rend les `task_id` effectivement postés. Indépendant de `should_deliver` :
/// un verdict est dû à son ticket que le rapport soit livré ou non. C'est
/// précisément pourquoi la sauvegarde du checkpoint a cessé d'être conditionnée
/// à la seule livraison.
async fn post_flight_verdicts(
    cfg: &ManagerConfig,
    ctx: &CycleContext<'_>,
    verdicts: &[FlightVerdict],
    checkpoint: Option<&MilestoneCheckpoint>,
) -> Vec<String> {
    let Some(runner) = ctx.gh_runner else {
        return Vec::new();
    };
    if verdicts.is_empty() {
        return Vec::new();
    }
    let already = checkpoint
        .map(|c| c.posted_flight_task_ids.as_slice())
        .unwrap_or(&[]);
    post_verdicts(
        write_is_armed(),
        runner,
        &cfg.target.repo,
        verdicts,
        already,
    )
    .await
}

/// La liste des vols postés que le prochain checkpoint portera : l'ancienne,
/// plus les nouveaux, bornée aux plus récents.
fn merge_posted(previous: &[String], newly: &[String]) -> Vec<String> {
    let mut merged: Vec<String> = previous.to_vec();
    merged.extend(newly.iter().cloned());
    if merged.len() > POSTED_FLIGHTS_RETAINED {
        merged.drain(..merged.len() - POSTED_FLIGHTS_RETAINED); // safe-byte-slice: Vec<String> element index, not a byte offset
    }
    merged
}

/// The cycle body. See [`run_manager_cycle_with`] for the four-argument form.
pub async fn run_manager_cycle_in(
    cfg: &ManagerConfig,
    state: MilestoneState,
    ctx: &CycleContext<'_>,
) -> Result<CycleOutcome> {
    let now = ctx.now;
    let deliverer = ctx.deliverer;
    let generated_at = now.to_rfc3339();
    let digest = state_digest(&state);
    let checkpoint = load_checkpoint(&cfg.checkpoint_dir, &cfg.target).await;

    let state_changed = checkpoint
        .as_ref()
        .map(|c| c.state_digest != digest)
        .unwrap_or(true);
    let heartbeat_fired = checkpoint
        .as_ref()
        .and_then(|c| c.last_delivered_at.as_ref())
        .map(|last| {
            DateTime::parse_from_rfc3339(last)
                .ok()
                .map(|dt| {
                    now.signed_duration_since(dt.with_timezone(&Utc)) >= cfg.heartbeat_interval
                })
                .unwrap_or(true)
        })
        .unwrap_or(true);

    let assessor = Assessor::new(AssessorConfig {
        silence_threshold_days: cfg.silence_threshold_days,
    });
    let mut assessment = assessor.assess(&state);
    // mika#2698 vol 1 — mode ombre. Le verdict entre dans l'assessment, donc
    // dans le rapport. Depuis le vol 2 il a une seconde destination, et une
    // seule : un commentaire sur son ticket, derrière l'interrupteur.
    assessment.flight_verdicts = collect_flight_verdicts(ctx, &state, checkpoint.as_ref()).await;
    let newly_posted =
        post_flight_verdicts(cfg, ctx, &assessment.flight_verdicts, checkpoint.as_ref()).await;
    let severity = assessment.severity.clone();
    let report = Reporter::new().report_with_auth_notes(&state, &assessment, ctx.auth_notes);

    let should_deliver = state_changed || heartbeat_fired;
    let cycle_kind = match (state_changed, heartbeat_fired) {
        (true, true) => CycleKind::EventAndHeartbeat,
        (true, false) => CycleKind::Event,
        (false, true) => CycleKind::Heartbeat,
        (false, false) => CycleKind::Event, // unused when should_deliver=false
    };

    let mut escalated = false;
    let mut delivered = false;
    let mut auth_boundary: Option<AuthBoundaryError> = None;
    let mut auth_attempted = false;

    if should_deliver {
        let body = DeliveryBody {
            milestone_ref: cfg.target.clone(),
            severity: severity.clone(),
            report_markdown: report.clone(),
            assessment: assessment.clone(),
            generated_at: generated_at.clone(),
            cycle_kind,
        };
        let route = select_route(&severity, cfg);
        match route {
            Route::Http { url, token } => {
                // A credential was presented at a boundary — whatever the
                // outcome, this cycle is evidence about it.
                auth_attempted = true;
                let outcome = deliverer.deliver(&url, token.as_deref(), &body).await;
                // mika#1949 U3 — read the outcome as an auth-boundary event
                // before consuming it, and record it. Fire-and-forget: the
                // delivery's own handling below is untouched by this.
                if let Some(err) = classify_delivery_auth(
                    token.as_deref(),
                    std::env::var(crate::milestone_manager::spawn::ENV_DELIVERY_TOKEN).is_ok(),
                    &outcome,
                ) {
                    crate::auth_boundary_ledger::record_if_wired(ctx.auth_ledger, &err);
                    auth_boundary = Some(err);
                }
                match outcome {
                    Ok(()) => {
                        delivered = true;
                        escalated = severity == Severity::Blocked;
                        tracing::info!(
                            target: "mika::milestone_manager",
                            event = "manager_cycle_delivered",
                            milestone = %cfg.target.as_display(),
                            severity = ?severity,
                            route = ROUTE_HTTP,
                            escalated = escalated,
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "mika::milestone_manager",
                            event = "manager_cycle_delivery_failed",
                            milestone = %cfg.target.as_display(),
                            error = %e,
                        );
                        // Fall back to offline sink so nothing is lost.
                        write_offline_sink(&cfg.offline_sink_dir, &cfg.target, &body).await?;
                        delivered = true;
                        // A Blocked severity that falls back to the sink is
                        // still an escalation outcome from the operator's
                        // point of view — the report was surfaced, just via
                        // a different route. Preserving `escalated` here
                        // keeps outcome telemetry honest (H2 review fix).
                        escalated = severity == Severity::Blocked;
                        // mika#2267 C4 — avant ce ticket, c'était le SEUL des
                        // trois chemins d'écriture à n'émettre aucun
                        // `manager_cycle_delivered` : un rapport était écrit
                        // sans qu'aucune ligne ne dise qu'il l'avait été, ni
                        // où. `offline_sink_fallback` est délibérément distinct
                        // de `offline_sink` — l'un dit « aucune URL n'était
                        // posée » (bring-up nominal), l'autre « une URL était
                        // posée et a échoué » (panne réelle).
                        tracing::info!(
                            target: "mika::milestone_manager",
                            event = "manager_cycle_delivered",
                            milestone = %cfg.target.as_display(),
                            severity = ?severity,
                            route = ROUTE_OFFLINE_SINK_FALLBACK,
                            escalated = escalated,
                            offline_sink_dir = %cfg.offline_sink_dir.display(),
                        );
                    }
                }
            }
            Route::OfflineSink => {
                write_offline_sink(&cfg.offline_sink_dir, &cfg.target, &body).await?;
                delivered = true;
                escalated = severity == Severity::Blocked;
                tracing::info!(
                    target: "mika::milestone_manager",
                    event = "manager_cycle_delivered",
                    milestone = %cfg.target.as_display(),
                    severity = ?severity,
                    route = ROUTE_OFFLINE_SINK,
                    escalated = escalated,
                    offline_sink_dir = %cfg.offline_sink_dir.display(),
                );
            }
        }
    }

    // Persist checkpoint on delivery — ou sur un verdict posté sans livraison
    // (mika#2698 vol 2). Sans cette seconde raison, un cycle qui poste sans
    // livrer ne retiendrait rien et reposterait au cycle suivant : l'AC2
    // tomberait précisément dans le cas où l'interrupteur sert.
    let posted_something = !newly_posted.is_empty();
    if should_deliver || posted_something {
        let previous = checkpoint
            .as_ref()
            .map(|c| c.posted_flight_task_ids.as_slice())
            .unwrap_or(&[]);
        let posted_flight_task_ids = merge_posted(previous, &newly_posted);
        let new_checkpoint = if should_deliver {
            MilestoneCheckpoint {
                state_digest: digest,
                last_delivered_at: Some(generated_at.clone()),
                last_observed_at: state.last_activity_at.clone(),
                posted_flight_task_ids,
            }
        } else {
            // Posté sans livrer : les trois champs de cadence sont **préservés**
            // tels quels. Les réécrire ici avalerait l'événement
            // `state_changed` en attente — une sauvegarde faite pour une raison
            // ne doit pas répondre à la place d'une autre.
            let base = checkpoint.clone().unwrap_or_default();
            MilestoneCheckpoint {
                posted_flight_task_ids,
                ..base
            }
        };
        save_checkpoint(&cfg.checkpoint_dir, &cfg.target, &new_checkpoint).await?;
    }

    Ok(CycleOutcome {
        milestone_ref: cfg.target.clone(),
        delivered,
        escalated,
        state_changed,
        heartbeat_fired,
        severity,
        generated_at,
        auth_boundary,
        auth_attempted,
    })
}

enum Route {
    Http { url: String, token: Option<String> },
    OfflineSink,
}

fn select_route(severity: &Severity, cfg: &ManagerConfig) -> Route {
    // H1 review fix: Blocked severity uses ONLY the escalation URL — no
    // fallback to `delivery_url`. The escalation route is intentionally
    // distinct so a `Blocked` report cannot silently queue behind the
    // normal Prime→sami→Vincent relay. When the escalation URL is unset,
    // fall through to the offline sink so the report is captured without
    // being routed through the wrong path.
    let url = match severity {
        Severity::Blocked => cfg.escalation_url.clone(),
        _ => cfg.delivery_url.clone(),
    };
    match url {
        Some(u) if url_is_routable(Some(&u)) => Route::Http {
            url: u,
            token: cfg.delivery_token.clone(),
        },
        _ => Route::OfflineSink,
    }
}

pub(crate) async fn load_checkpoint(
    dir: &std::path::Path,
    r: &MilestoneRef,
) -> Option<MilestoneCheckpoint> {
    let path = dir.join(format!("{}.json", r.slug()));
    let bytes = tokio::fs::read(&path).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(crate) async fn save_checkpoint(
    dir: &std::path::Path,
    r: &MilestoneRef,
    c: &MilestoneCheckpoint,
) -> Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    let path = dir.join(format!("{}.json", r.slug()));
    let bytes = serde_json::to_vec_pretty(c)?;
    // Use `tokio::fs::write` (atomic spawn_blocking → std::fs::write) instead of
    // `File::create` + `write_all`. The latter can return before tokio's internal
    // blocking write task has actually drained bytes to disk — under CI blocking-pool
    // pressure, the next `load_checkpoint` reads an empty/partial file, silently falls
    // through to `checkpoint = None`, and both `state_changed`/`heartbeat_fired`
    // default to `true` via `unwrap_or(true)`. See CI flake on PR#1938: the
    // `event_driven_fires_on_state_change` test expects `heartbeat_fired = false`
    // at t0 + 1h but got `true` because the t0 checkpoint save had not fully
    // persisted before the t1 load. Matches the rest of the codebase
    // (`write_workspace.rs`, `write_agent_file.rs`, `teams/engine.rs`).
    tokio::fs::write(&path, &bytes).await?;
    Ok(())
}

async fn write_offline_sink(
    dir: &std::path::Path,
    r: &MilestoneRef,
    body: &DeliveryBody,
) -> Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    // Timestamp in the filename disambiguates multi-per-day writes.
    let ts = body.generated_at.replace(':', "-");
    let path = dir.join(format!("{}-{}.md", r.slug(), ts));
    // Same rationale as `save_checkpoint`: atomic write via `tokio::fs::write`
    // rather than a `File::create` + `write_all` pair whose drop can race with
    // subsequent reads under CI blocking-pool pressure.
    tokio::fs::write(&path, body.report_markdown.as_bytes()).await?;
    Ok(())
}

/// Fetch executor liveness from an optional cm-style `GET /api/v1/agents/<entity>/health`
/// endpoint. Returns `Some(true)` on 2xx, `Some(false)` on any non-2xx, `None` when the
/// URL is unset or the request errors out (fail-open — health signal is a hint, never a
/// gate for the Phase 1 lecture-seule cycle).
///
/// M4 review fix: wire the previously-declared-but-unread `health_url` config field.
pub async fn probe_executor_health(url: Option<&str>) -> Option<bool> {
    let url = url?;
    if url.is_empty() {
        return None;
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .ok()?;
    match client.get(url).send().await {
        Ok(res) => Some(res.status().is_success()),
        Err(_) => None,
    }
}

/// Live-runner variant: reads from `gh`, probes optional health endpoint,
/// then runs the same cycle logic.
pub async fn run_manager_cycle(cfg: &ManagerConfig) -> Result<CycleOutcome> {
    run_manager_cycle_with_auth(cfg, None, &[], None).await
}

/// As [`run_manager_cycle`], with the mika#1949 auth-boundary wiring: where to
/// record an observed failure, and which failures the report should carry
/// forward from previous cycles — plus, depuis mika#2698, d'où viennent les vols
/// terminés à évaluer. `flight_source: None` rend un rapport identique à celui
/// d'avant mika#2698 : aucun verdict, donc pas de section.
pub async fn run_manager_cycle_with_auth(
    cfg: &ManagerConfig,
    auth_ledger: Option<&Arc<dyn AuthBoundaryLedger>>,
    auth_notes: &[crate::milestone_manager::reporter::AuthBoundaryNote],
    flight_source: Option<&Arc<dyn FlightSource>>,
) -> Result<CycleOutcome> {
    let reader = Reader::new(cfg.github_token.clone());
    let mut state = reader.read(&cfg.target).await?;
    // M4 review fix: honour the health_url config surface.
    state.executor_healthy = probe_executor_health(cfg.health_url.as_deref()).await;
    let deliverer = HttpReportDeliverer::new();
    // mika#2698 vol 2 — l'exécutant du seul geste d'écriture. Construit ici,
    // depuis le même jeton que le lecteur : aucune nouvelle injection à faire
    // traverser `spawn.rs` et `server/mod.rs`. Il est posé quel que soit l'état
    // de l'interrupteur, parce que c'est `post_verdicts` qui décide — un
    // câblage conditionné à l'environnement rendrait la branche OFF
    // structurellement muette, donc intestable.
    let gh_runner = ProcessGhRunner::new(cfg.github_token.clone());
    let ctx = CycleContext::new(&deliverer, Utc::now())
        .with_auth_ledger(auth_ledger)
        .with_auth_notes(auth_notes)
        .with_flight_source(flight_source)
        .with_gh_runner(Some(&gh_runner));
    run_manager_cycle_in(cfg, state, &ctx).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::milestone_manager::types::{
        CiState, IssueState, MilestoneRef, ProgressCounts, RecentActivity, SubIssue,
    };
    use crate::milestone_manager::verdict_poster::ENV_EVALUATE_WRITE;
    use chrono::TimeZone;
    use std::sync::{Arc, Mutex};

    fn base_state() -> MilestoneState {
        MilestoneState {
            milestone_ref: MilestoneRef {
                repo: "senara-solutions/mika".into(),
                number: 1799,
            },
            title: "LC".into(),
            description: "".into(),
            state: IssueState::Open,
            created_at: "".into(),
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
            ready_pool: vec![],
            outside_candidates: vec![],
            outside_read_truncated: None,
        }
    }

    /// Captured call from `RecordingDeliverer`.
    type CapturedCall = (String, Option<String>, DeliveryBody);

    /// In-memory deliverer that captures each call.
    #[derive(Default, Clone)]
    struct RecordingDeliverer {
        calls: Arc<Mutex<Vec<CapturedCall>>>,
    }

    #[async_trait::async_trait]
    impl ReportDeliverer for RecordingDeliverer {
        async fn deliver(&self, url: &str, token: Option<&str>, body: &DeliveryBody) -> Result<()> {
            self.calls.lock().unwrap().push((
                url.to_string(),
                token.map(str::to_string),
                body.clone(),
            ));
            Ok(())
        }
    }

    fn mk_config(dir: &std::path::Path) -> ManagerConfig {
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
            delivery_token: Some("t".into()),
            escalation_url: Some("http://vincent/direct".into()),
            health_url: None,
            checkpoint_dir: dir.join("checkpoints"),
            offline_sink_dir: dir.join("sink"),
            sink_dir_source: crate::milestone_manager::sink_dir::SinkDirSource::Default,
            // Le battement de liveness (mika#1990) est un canal distinct du POST
            // de rapport : aucun test de cadence ne l'exerce, d'où `None`.
            liveness_url: None,
        }
    }

    #[tokio::test]
    async fn first_cycle_always_delivers() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = mk_config(tmp.path());
        let rec = RecordingDeliverer::default();
        let now = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();
        let outcome = run_manager_cycle_with(&cfg, base_state(), &rec, now)
            .await
            .unwrap();
        assert!(outcome.delivered);
        // First cycle: no checkpoint → state_changed=true (baseline vs missing).
        assert!(outcome.state_changed);
        assert!(outcome.heartbeat_fired);
        assert_eq!(rec.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn heartbeat_only_after_interval_elapses() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = mk_config(tmp.path());
        let rec = RecordingDeliverer::default();
        let t0 = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();
        // First cycle at t0.
        let _ = run_manager_cycle_with(&cfg, base_state(), &rec, t0)
            .await
            .unwrap();
        // Second cycle 1 hour later with SAME state → no state_change, no heartbeat.
        let t1 = t0 + chrono::Duration::hours(1);
        let outcome1 = run_manager_cycle_with(&cfg, base_state(), &rec, t1)
            .await
            .unwrap();
        assert!(!outcome1.delivered);
        assert!(!outcome1.state_changed);
        assert!(!outcome1.heartbeat_fired);
        // Third cycle 7 hours later with SAME state → heartbeat fires.
        let t2 = t0 + chrono::Duration::hours(7);
        let outcome2 = run_manager_cycle_with(&cfg, base_state(), &rec, t2)
            .await
            .unwrap();
        assert!(outcome2.delivered);
        assert!(!outcome2.state_changed);
        assert!(outcome2.heartbeat_fired);
        // Total delivered: 2 (first + heartbeat).
        assert_eq!(rec.calls.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn event_driven_fires_on_state_change() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = mk_config(tmp.path());
        let rec = RecordingDeliverer::default();
        let t0 = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();
        let _ = run_manager_cycle_with(&cfg, base_state(), &rec, t0)
            .await
            .unwrap();

        // Same-day state change (1 hour later) — sub-issue closed.
        let mut new_state = base_state();
        new_state.sub_issues[0].state = IssueState::Closed;
        let t1 = t0 + chrono::Duration::hours(1);
        let outcome = run_manager_cycle_with(&cfg, new_state, &rec, t1)
            .await
            .unwrap();
        assert!(outcome.state_changed);
        assert!(outcome.delivered);
        assert!(!outcome.heartbeat_fired);
    }

    #[tokio::test]
    async fn blocked_severity_routes_to_escalation_url() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = mk_config(tmp.path());
        let rec = RecordingDeliverer::default();
        let t0 = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();
        // Blocked state: single open issue with CI failed.
        let mut state = base_state();
        state.sub_issues[0].ci_state = CiState::Failed;
        state.sub_issues[0].pr_state = Some("open".into());
        let outcome = run_manager_cycle_with(&cfg, state, &rec, t0).await.unwrap();
        assert!(outcome.escalated);
        assert_eq!(outcome.severity, Severity::Blocked);
        let calls = rec.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "http://vincent/direct");
    }

    #[tokio::test]
    async fn non_blocked_routes_to_normal_url() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = mk_config(tmp.path());
        let rec = RecordingDeliverer::default();
        let t0 = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();
        let outcome = run_manager_cycle_with(&cfg, base_state(), &rec, t0)
            .await
            .unwrap();
        assert!(!outcome.escalated);
        let calls = rec.calls.lock().unwrap();
        assert_eq!(calls[0].0, "http://normal/deliver");
    }

    /// H1 review fix regression test: a `Blocked` severity with escalation URL
    /// unset MUST route to the offline sink, NOT fall back to the normal
    /// delivery URL. Escalation is intentionally distinct from normal delivery.
    #[tokio::test]
    async fn blocked_severity_with_no_escalation_url_falls_to_offline_sink_not_delivery_url() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = mk_config(tmp.path());
        cfg.escalation_url = None;
        // delivery_url is intentionally still set — the bug would silently
        // route the Blocked report here.
        let rec = RecordingDeliverer::default();
        let t0 = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();
        let mut state = base_state();
        state.sub_issues[0].ci_state = CiState::Failed;
        state.sub_issues[0].pr_state = Some("open".into());
        let outcome = run_manager_cycle_with(&cfg, state, &rec, t0).await.unwrap();
        assert_eq!(outcome.severity, Severity::Blocked);
        assert!(outcome.escalated, "Blocked should still count as escalated");
        // No HTTP call — routed to offline sink.
        assert_eq!(
            rec.calls.lock().unwrap().len(),
            0,
            "Blocked with no escalation URL must NOT hit the normal delivery URL"
        );
        // Offline sink must have the report.
        let mut entries = tokio::fs::read_dir(tmp.path().join("sink")).await.unwrap();
        let mut count = 0;
        while entries.next_entry().await.unwrap().is_some() {
            count += 1;
        }
        assert!(count >= 1);
    }

    /// H2 review fix regression test: HTTP delivery failure that falls back
    /// to the offline sink MUST still set `escalated = true` when severity
    /// is `Blocked` — telemetry integrity.
    #[tokio::test]
    async fn blocked_severity_escalated_flag_preserved_across_http_failure() {
        struct FailingDeliverer;
        #[async_trait::async_trait]
        impl ReportDeliverer for FailingDeliverer {
            async fn deliver(
                &self,
                _url: &str,
                _token: Option<&str>,
                _body: &DeliveryBody,
            ) -> Result<()> {
                Err(anyhow::anyhow!("simulated network failure"))
            }
        }

        let tmp = tempfile::tempdir().unwrap();
        let cfg = mk_config(tmp.path());
        let t0 = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();
        let mut state = base_state();
        state.sub_issues[0].ci_state = CiState::Failed;
        state.sub_issues[0].pr_state = Some("open".into());
        let outcome = run_manager_cycle_with(&cfg, state, &FailingDeliverer, t0)
            .await
            .unwrap();
        assert_eq!(outcome.severity, Severity::Blocked);
        assert!(
            outcome.escalated,
            "escalated must remain true after HTTP failure fallback (H2)"
        );
        assert!(outcome.delivered, "sink write should succeed");
    }

    #[tokio::test]
    async fn offline_sink_when_no_urls_configured() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = mk_config(tmp.path());
        cfg.delivery_url = None;
        cfg.escalation_url = None;
        let rec = RecordingDeliverer::default();
        let t0 = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();
        let outcome = run_manager_cycle_with(&cfg, base_state(), &rec, t0)
            .await
            .unwrap();
        assert!(outcome.delivered);
        assert_eq!(rec.calls.lock().unwrap().len(), 0);
        // Offline sink dir must contain the report.
        let mut entries = tokio::fs::read_dir(tmp.path().join("sink")).await.unwrap();
        let mut count = 0;
        while entries.next_entry().await.unwrap().is_some() {
            count += 1;
        }
        assert!(count >= 1);
    }

    #[tokio::test]
    async fn checkpoint_persists_across_cycles() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = mk_config(tmp.path());
        let rec = RecordingDeliverer::default();
        let t0 = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();
        let _ = run_manager_cycle_with(&cfg, base_state(), &rec, t0)
            .await
            .unwrap();
        // Second cycle same state → no delivery.
        let t1 = t0 + chrono::Duration::hours(1);
        let outcome = run_manager_cycle_with(&cfg, base_state(), &rec, t1)
            .await
            .unwrap();
        assert!(!outcome.delivered);
        // Verify checkpoint file exists.
        let path = tmp
            .path()
            .join("checkpoints")
            .join("senara-solutions-mika-1799.json");
        assert!(path.exists());
    }

    #[tokio::test]
    async fn probe_executor_health_returns_none_on_unset_url() {
        assert_eq!(probe_executor_health(None).await, None);
        assert_eq!(probe_executor_health(Some("")).await, None);
    }

    /// mika#2495 — l'injoignabilité d'une fixture est LOCALE, jamais réseau.
    ///
    /// **Ne pas remettre une adresse non routable ici**, si tortueux que le
    /// `bind`/`drop` ci-dessous puisse paraître. La fixture précédente sondait
    /// `192.0.2.1:1` (RFC 5737) et n'a jamais testé ce qu'elle annonçait dans un
    /// bac à sable pilote : `HTTP_PROXY` y est posé, `192.0.2.1` n'est pas dans
    /// `NO_PROXY`, et la détection de proxy par variables d'environnement de
    /// `reqwest` est active par défaut — donc la requête partait au relais
    /// d'egress en forme absolue (`GET http://192.0.2.1:1/health HTTP/1.1`),
    /// forme que le relais ne sert pas. Il rendait `400 Bad Request`, d'où
    /// `Some(false)` au lieu de `None`, **en 0,03 s**. Trois centièmes de
    /// seconde sont un aller-retour de boucle locale : c'est la mesure qui
    /// attribue la cause, une tentative vers une adresse non routable rendant
    /// `ENETUNREACH` ou consommant le `timeout(5s)` du client. L'adresse n'était
    /// ni résolue ni composée.
    ///
    /// Un port de boucle locale que rien n'écoute rend `ECONNREFUSED` sous proxy
    /// comme sans, parce que la boucle locale n'est jamais proxifiée — exclue
    /// par `NO_PROXY=localhost,127.0.0.1` dans le bac à sable, et par l'absence
    /// de tout proxy sur le CI.
    ///
    /// Le motif était le classique « bind puis drop ». **mika#2569 l'a retiré** :
    /// ce binaire de tests unitaires est le plus parallèle de l'arbre, et un
    /// port libéré peut être repris entre le `drop` et l'appel. Le garde tient
    /// désormais le port pour toute la durée de la sonde.
    ///
    /// Renommé depuis `…_on_bogus_host` : « bogus host » décrivait la prémisse
    /// écartée (un hôte fantaisiste) ; ce qui est asserté est un point de
    /// terminaison injoignable.
    #[tokio::test]
    async fn probe_executor_health_returns_none_on_unreachable_endpoint() {
        let dead = mika_common::dead_endpoint::DeadEndpoint::reserve();
        let port = dead.addr().port();

        let url = format!("http://127.0.0.1:{port}/health");
        let res = probe_executor_health(Some(&url)).await;
        assert_eq!(
            res, None,
            "une connexion refusée doit fail-open à None (url={url})"
        );
    }

    #[test]
    fn state_digest_stable_across_orderings() {
        let mut s = base_state();
        let d1 = state_digest(&s);
        // Add another sub-issue and reorder — digest must be order-insensitive.
        s.sub_issues.insert(
            0,
            SubIssue {
                number: 1803,
                title: "LC.3".into(),
                state: IssueState::Closed,
                priority_rank: None,
                plan_present: false,
                branch_present: false,
                pr_number: None,
                pr_state: None,
                ci_state: CiState::None,
                blockers: vec![],
                updated_at: "".into(),
                labels: vec![],
            },
        );
        let d2 = state_digest(&s);
        s.sub_issues.reverse();
        let d3 = state_digest(&s);
        assert_ne!(d1, d2);
        assert_eq!(d2, d3);
    }

    /// Cas 13 (mika#2699) — le digest est **insensible** au bassin `ready`.
    ///
    /// C'est une décision, pas un effet de bord. La cadence est hybride : un
    /// changement de digest déclenche un cycle événementiel. Le bassin bouge à
    /// chaque dispatch de n'importe quel ticket du **dépôt**, donc l'y inclure
    /// transformerait l'activité d'autres milestones en cycles sur celui-ci —
    /// un rapport de plus par admission, pour un milestone qui n'a pas changé.
    /// Le bassin se lit dans le rapport, il ne décide pas de son envoi.
    ///
    /// Vrai par construction aujourd'hui (`state_digest` n'énumère que les
    /// `sub_issues`) ; ce test existe pour qu'un ajout de champ au digest soit
    /// une décision délibérée et non un accident.
    #[test]
    fn mika2699_le_digest_ignore_le_bassin_ready() {
        let mut s = base_state();
        let avant = state_digest(&s);
        s.ready_pool = vec![
            crate::milestone_manager::pool::ReadyTicket {
                number: 2699,
                labels: vec!["ready".into(), "dispatch:loop".into()],
            },
            crate::milestone_manager::pool::ReadyTicket {
                number: 2687,
                labels: vec!["ready".into()],
            },
        ];
        assert_eq!(
            avant,
            state_digest(&s),
            "le bassin ne doit pas déclencher de cycle événementiel"
        );
    }

    // ---- mika#1949 U3 — the delivery boundary, classified ------------------

    fn refused() -> Result<()> {
        Err(DeliveryError::raise(
            DeliveryFailureKind::CredentialRefused,
            "delivery failed: 401 Unauthorized — ",
        ))
    }

    #[test]
    fn a_refusal_with_no_token_reads_as_missing() {
        let got = classify_delivery_auth(None, false, &refused()).expect("an auth event");
        assert_eq!(got.kind, AuthBoundaryKind::Missing);
        assert_eq!(got.token_name, "MIKA_MANAGER_DELIVERY_TOKEN");
        assert_eq!(got.boundary_key(), "manager_to_delivery");
    }

    #[test]
    fn a_refusal_with_a_blank_token_reads_as_empty() {
        let got = classify_delivery_auth(Some("   "), true, &refused()).expect("an auth event");
        assert_eq!(
            got.kind,
            AuthBoundaryKind::Empty,
            "a variable set to blank is a different operator fix from a variable never set"
        );
    }

    #[test]
    fn a_refusal_with_a_real_token_reads_as_rejected() {
        let got =
            classify_delivery_auth(Some("something"), true, &refused()).expect("an auth event");
        assert_eq!(got.kind, AuthBoundaryKind::Rejected);
    }

    #[test]
    fn a_403_is_classified_alongside_401() {
        // KTD2: cm answers 403 where mika answers 401, and both stand. A
        // classifier that recognised only one would go blind against whichever
        // peer it was not written for.
        let outcome: Result<()> = Err(DeliveryError::raise(
            DeliveryFailureKind::CredentialRefused,
            "delivery failed: 403 Forbidden — ",
        ));
        assert_eq!(
            classify_delivery_auth(Some("t"), true, &outcome)
                .unwrap()
                .kind,
            AuthBoundaryKind::Rejected
        );
    }

    #[test]
    fn an_unreachable_endpoint_is_not_reported_as_a_bad_token() {
        let outcome: Result<()> = Err(DeliveryError::raise(
            DeliveryFailureKind::Unreachable,
            "delivery failed: connection refused",
        ));
        let got = classify_delivery_auth(None, false, &outcome).expect("an auth event");
        assert_eq!(
            got.kind,
            AuthBoundaryKind::Unreachable,
            "no verdict exists when nobody answered — the operator must not be sent after the token"
        );
    }

    /// The two outcomes that must produce **no** row.
    ///
    /// A ledger that fired on every successful delivery, or on every 500, would
    /// stop being read — and a ledger nobody reads is worse than none, because
    /// it looks like coverage.
    #[test]
    fn success_and_non_auth_failures_produce_no_row() {
        assert!(classify_delivery_auth(None, false, &Ok(())).is_none());
        assert!(classify_delivery_auth(Some("t"), true, &Ok(())).is_none());

        let five_hundred: Result<()> = Err(DeliveryError::raise(
            DeliveryFailureKind::Other,
            "delivery failed: 500 Internal Server Error — ",
        ));
        assert!(classify_delivery_auth(Some("t"), true, &five_hundred).is_none());

        // An error that is not a DeliveryError at all — e.g. raised upstream of
        // the sink — carries no class and must not be guessed at.
        let untyped: Result<()> = Err(anyhow::anyhow!("something else entirely"));
        assert!(classify_delivery_auth(Some("t"), true, &untyped).is_none());
    }

    /// `Invalid` is a real kind of [`AuthBoundaryKind`] and this boundary never
    /// produces it — stated as a test so the absence reads as a decision.
    ///
    /// `Invalid` means "present but structurally malformed", which requires a
    /// shape check. The gateway applies one (hex validation on
    /// `MIKA_INTERNAL_TOKEN`); the delivery boundary applies none, and inventing
    /// a shape rule here would refuse tokens the endpoint would have accepted.
    #[test]
    fn the_delivery_boundary_never_claims_invalid() {
        for token in [None, Some(""), Some("   "), Some("whatever")] {
            for outcome in [
                refused(),
                Err(DeliveryError::raise(DeliveryFailureKind::Unreachable, "x")),
                Err(DeliveryError::raise(DeliveryFailureKind::Other, "x")),
                Ok(()),
            ] {
                if let Some(got) = classify_delivery_auth(token, true, &outcome) {
                    assert_ne!(got.kind, AuthBoundaryKind::Invalid);
                }
            }
        }
    }

    /// The delivery outcome reaches the cycle's caller, so the loop can count
    /// consecutive failures without re-deriving them.
    #[tokio::test]
    async fn the_cycle_reports_the_auth_boundary_it_observed() {
        struct RefusingDeliverer;
        #[async_trait::async_trait]
        impl ReportDeliverer for RefusingDeliverer {
            async fn deliver(&self, _: &str, _: Option<&str>, _: &DeliveryBody) -> Result<()> {
                Err(DeliveryError::raise(
                    DeliveryFailureKind::CredentialRefused,
                    "delivery failed: 401 Unauthorized — ",
                ))
            }
        }

        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = mk_config(tmp.path());
        cfg.delivery_url = Some("http://127.0.0.1:1/report".into());
        cfg.delivery_token = Some("a-token".into());

        let outcome = run_manager_cycle_with(&cfg, base_state(), &RefusingDeliverer, Utc::now())
            .await
            .unwrap();

        let observed = outcome.auth_boundary.expect("the cycle must surface it");
        assert_eq!(observed.kind, AuthBoundaryKind::Rejected);
        assert_eq!(observed.boundary_key(), "manager_to_delivery");
        // The report still reached the operator — via the offline sink. A
        // failed credential must not also lose the report.
        assert!(outcome.delivered);
    }

    /// A no-op cycle presents no credential, so it is evidence about nothing.
    ///
    /// This is the distinction that keeps the `Blocked` escalation alive: with
    /// `poll_interval` at 5 min against a 6 h heartbeat, no-op cycles are the
    /// common case, and a loop that read them as "auth works" would reset the
    /// repeat counter between almost every pair of real failures.
    #[tokio::test]
    async fn a_no_op_cycle_reports_no_authentication_attempt() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = mk_config(tmp.path());
        let rec = RecordingDeliverer::default();
        let now = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();

        // First cycle always delivers and writes the checkpoint.
        let first = run_manager_cycle_with(&cfg, base_state(), &rec, now)
            .await
            .unwrap();
        assert!(first.delivered);
        assert!(first.auth_attempted, "a delivery presents the credential");

        // Second cycle, same state, well inside the heartbeat window: no-op.
        let later = now + chrono::Duration::minutes(5);
        let second = run_manager_cycle_with(&cfg, base_state(), &rec, later)
            .await
            .unwrap();
        assert!(
            !second.delivered,
            "precondition: this cycle must be a no-op"
        );
        assert!(
            !second.auth_attempted,
            "a cycle that delivered nothing proves nothing about the credential"
        );
        assert!(second.auth_boundary.is_none());
    }

    /// An offline-sink write delivers the report without presenting any
    /// credential, so it is not evidence either.
    #[tokio::test]
    async fn an_offline_sink_write_reports_no_authentication_attempt() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = mk_config(tmp.path());
        cfg.delivery_url = None;
        cfg.escalation_url = None;
        let rec = RecordingDeliverer::default();

        let outcome = run_manager_cycle_with(&cfg, base_state(), &rec, Utc::now())
            .await
            .unwrap();
        assert!(outcome.delivered, "the sink still captures the report");
        assert!(
            !outcome.auth_attempted,
            "writing to a local directory presents no token"
        );
    }

    /// Finding from review: `Empty` was unreachable on the only production
    /// path, because `read_string_env` collapses a blank variable to `None`
    /// before the config is built. The operator was then told the variable is
    /// **not configured** when it is configured with a lost value — two
    /// different fixes, which is the entire reason `Empty` exists as a kind.
    #[test]
    fn a_blank_variable_dropped_by_the_config_still_reads_as_empty() {
        // What `manager_config_from_env` actually produces for
        // `MIKA_MANAGER_DELIVERY_TOKEN=""`: token `None`, variable present.
        let got = classify_delivery_auth(None, true, &refused()).expect("an auth event");
        assert_eq!(
            got.kind,
            AuthBoundaryKind::Empty,
            "set-but-blank must not be reported as never-set"
        );

        // And the genuinely-unset case is still `Missing`.
        assert_eq!(
            classify_delivery_auth(None, false, &refused())
                .unwrap()
                .kind,
            AuthBoundaryKind::Missing
        );
    }

    /// Finding from review: flattening `reqwest::Error` into the message threw
    /// away its source chain — the half that names DNS vs TLS vs connection
    /// refused. On a ticket about telling transport apart from credential,
    /// that was the transport half.
    #[tokio::test]
    async fn a_transport_failure_keeps_its_source_chain() {
        let deliverer = HttpReportDeliverer::new();
        let body = DeliveryBody {
            milestone_ref: base_state().milestone_ref.clone(),
            severity: Severity::Healthy,
            report_markdown: String::new(),
            assessment: crate::milestone_manager::types::Assessment {
                severity: Severity::Healthy,
                recommendation: crate::milestone_manager::types::Recommendation {
                    next_sub_issue: None,
                    rationale: String::new(),
                },
                alerts: vec![],
                cross_cutting: vec![],
                contention_events: vec![],
                pool: None,
                flight_verdicts: vec![],
                relief: None,
            },
            generated_at: String::new(),
            cycle_kind: CycleKind::Event,
        };
        // Port 1 on loopback: nothing listens, so this is a connect failure.
        let err = deliverer
            .deliver("http://127.0.0.1:1/report", Some("t"), &body)
            .await
            .expect_err("nothing listens on port 1");

        let flat = format!("{err}");
        let chained = format!("{err:#}");
        assert!(
            chained.len() > flat.len(),
            "the transport cause must survive as a source: flat={flat:?} chained={chained:?}"
        );
        assert_eq!(
            err.downcast_ref::<DeliveryError>().map(|d| d.kind),
            Some(DeliveryFailureKind::Unreachable)
        );
    }

    // ---- mika#2697 — une URL de battement n'est pas une route de rapport ----

    /// La table complète du prédicat, positifs et négatifs dans le **même**
    /// test, parce que c'est leur voisinage qui dit la règle.
    ///
    /// Les trois négatifs du bas sont le vrai risque de relâchement : un
    /// `contains("heartbeat")` écrit à la place rougit sur les trois — query,
    /// host, position non terminale — et reste vert sur tout le reste. Une
    /// table de positifs seule laisserait donc passer la réécriture qui refuse
    /// des routes de rapport légitimes.
    #[test]
    fn mika2697_le_predicat_juge_le_dernier_segment_du_chemin() {
        let cases: &[(&str, bool, &str)] = &[
            // Le défaut mesuré : l'URL en vigueur sur l'hôte depuis le 27/09.
            (
                "https://cm.example.com/api/v1/agents/mika-manager/heartbeat",
                true,
                "l'endpoint de battement de cm, cas fondateur",
            ),
            (
                "https://cm.example.com/api/v1/agents/mika-manager/heartbeat/",
                true,
                "le dernier segment NON VIDE : un slash final ne change rien",
            ),
            (
                "https://cm.example.com/api/v1/agents/mika-manager/HeartBeat",
                true,
                "insensible à la casse",
            ),
            // Routes de rapport légitimes — aucune ne doit être refusée.
            (
                "https://cm.example.com/api/v1/messages/dispatch",
                false,
                "une route d'ingestion de rapport",
            ),
            (
                "https://reports.example/ingest?from=heartbeat",
                false,
                "le mot est en QUERY : le jugement porte sur le chemin",
            ),
            (
                "https://heartbeat.example.com/reports",
                false,
                "le mot est en HOST : le jugement porte sur le chemin",
            ),
            (
                "https://cm.example.com/heartbeat/reports",
                false,
                "position NON TERMINALE : la règle décrit ce qu'un endpoint est",
            ),
            (
                "https://cm.example.com/api/v1/heartbeats",
                false,
                "`heartbeat` seul, pas son pluriel : cm n'expose que cette forme",
            ),
            (
                "pas une URL du tout",
                false,
                "non parsable ⇒ false : refuser ce qu'on ne sait pas lire ferait \
                 de ce prédicat un second mode de panne (le repli le rattrape)",
            ),
        ];

        for (url, expected, why) in cases {
            assert_eq!(
                url_is_heartbeat_endpoint(url),
                *expected,
                "url_is_heartbeat_endpoint({url:?}) : {why}"
            );
        }
    }

    /// Le prédicat de routage reste à **lecteur unique**, et la garde ne s'y
    /// substitue pas : une URL de battement est parfaitement *routable*.
    ///
    /// C'est ce qui justifie que la garde vive à la frontière de lecture de
    /// l'environnement et **efface la valeur**, au lieu de dévier la route ici.
    /// Normaliser en aval laisserait `manager_delivery_resolved` annoncer
    /// `delivery_url_set=true route_normal=http` pendant que le cycle écrit
    /// dans le puits — un instrument qui mentirait d'une manière nouvelle pour
    /// en réparer une ancienne (mika#2267, propriété à préserver).
    #[test]
    fn mika2697_les_deux_predicats_repondent_a_deux_questions_distinctes() {
        let heartbeat = "https://cm.example.com/api/v1/agents/mika-manager/heartbeat";
        assert!(
            url_is_routable(Some(heartbeat)),
            "« une URL est-elle posée » : oui — et c'est pourquoi le POST réussissait"
        );
        assert!(
            url_is_heartbeat_endpoint(heartbeat),
            "« est-ce la bonne » : non — la question que `url_is_routable` n'avait pas"
        );
        assert_eq!(
            route_name_for(Some(heartbeat)),
            ROUTE_HTTP,
            "le nom de route d'une URL posée ne change pas : la garde agit en amont"
        );
    }

    // -- mika#2698 vol 1 : le câblage de la source de vols -------------------

    /// Source de vols instrumentée : elle enregistre **ce qu'on lui a demandé**
    /// — la borne `since` et les URLs — et rend les vols qu'on lui a donnés.
    /// L'enregistrement est ce qui rend la borne observable : sans lui, un
    /// `since` toujours `None` donnerait exactement les mêmes verdicts qu'un
    /// `since` correct sur un jeu de fixtures d'un seul vol.
    /// Le journal des demandes : une entrée par lecture, portant la borne
    /// `since` et la liste d'URLs interrogées.
    type FlightQueries = Arc<Mutex<Vec<(Option<String>, Vec<String>)>>>;

    struct RecordingFlightSource {
        flights: Vec<crate::milestone_manager::evaluator::FinishedFlight>,
        asked: FlightQueries,
    }

    impl RecordingFlightSource {
        /// Rend la source **et** son journal — pas `Self`, parce que le cycle
        /// consomme un `Arc<dyn FlightSource>` et que le test lit le journal.
        fn wired(
            flights: Vec<crate::milestone_manager::evaluator::FinishedFlight>,
        ) -> (Arc<dyn FlightSource>, FlightQueries) {
            let asked: FlightQueries = Arc::new(Mutex::new(Vec::new()));
            let source: Arc<dyn FlightSource> = Arc::new(Self {
                flights,
                asked: asked.clone(),
            });
            (source, asked)
        }
    }

    #[async_trait::async_trait]
    impl FlightSource for RecordingFlightSource {
        async fn finished_implement_flights(
            &self,
            since: Option<&str>,
            issue_urls: &[String],
        ) -> Vec<crate::milestone_manager::evaluator::FinishedFlight> {
            self.asked
                .lock()
                .unwrap()
                .push((since.map(str::to_string), issue_urls.to_vec()));
            self.flights.clone()
        }
    }

    /// Un vol terminé sur `#1802`, le sous-ticket de [`base_state`], dont la PR
    /// est ouverte — donc un merge-candidat.
    fn un_vol_sur_1802() -> crate::milestone_manager::evaluator::FinishedFlight {
        crate::milestone_manager::evaluator::FinishedFlight {
            task_id: "task-1802".into(),
            // Pas l'agent par défaut du processus : un vol d'implement est une
            // tâche du dispatcheur (mika#2792).
            agent_id: "mika-dev".into(),
            status: "completed".into(),
            result: Some(
                "Outcome: PR_OPENED\nTurns: 83\nPR: https://github.com/senara-solutions/mika/pull/2002\n"
                    .into(),
            ),
            metadata: Some(r#"{"claude_pilot":{"turns":83}}"#.into()),
            reference_url: Some("https://github.com/senara-solutions/mika/issues/1802".into()),
            completed_at: Some("2026-08-21T06:00:00Z".into()),
        }
    }

    /// Sans source câblée, le cycle est **celui d'avant mika#2698** : aucun
    /// verdict dans l'assessment, et donc pas de section dans le rapport. C'est
    /// la propriété qui rend l'ajout sûr pour les douze appelants qui ne
    /// passent rien.
    #[tokio::test]
    async fn mika2698_sans_source_le_rapport_est_celui_davant() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = mk_config(tmp.path());
        let rec = RecordingDeliverer::default();
        let now = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();

        run_manager_cycle_in(&cfg, base_state(), &CycleContext::new(&rec, now))
            .await
            .unwrap();

        let calls = rec.calls.lock().unwrap();
        let body = &calls[0].2;
        assert!(
            body.assessment.flight_verdicts.is_empty(),
            "aucune source ⇒ aucun verdict"
        );
        assert!(
            !body.report_markdown.contains("Vols terminés"),
            "aucun verdict ⇒ pas de section (anti-vacuité) :\n{}",
            body.report_markdown
        );
    }

    /// Avec une source câblée, le verdict entre dans l'assessment — donc dans
    /// le rapport livré, qui est le sink disque de mika#2697 — **et nulle part
    /// ailleurs** : le mode ombre du point 3 du ticket.
    #[tokio::test]
    async fn mika2698_avec_une_source_le_verdict_entre_dans_le_rapport() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = mk_config(tmp.path());
        let rec = RecordingDeliverer::default();
        let now = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();
        let (source, _asked) = RecordingFlightSource::wired(vec![un_vol_sur_1802()]);

        run_manager_cycle_in(
            &cfg,
            base_state(),
            &CycleContext::new(&rec, now).with_flight_source(Some(&source)),
        )
        .await
        .unwrap();

        let calls = rec.calls.lock().unwrap();
        let body = &calls[0].2;
        assert_eq!(body.assessment.flight_verdicts.len(), 1);
        let v = &body.assessment.flight_verdicts[0];
        assert_eq!(v.task_id, "task-1802");
        assert_eq!(v.issue_number, Some(1802));
        assert_eq!(v.turns_used, Some(83));
        assert!(
            body.report_markdown.contains(
                "### Vols terminés depuis le cycle précédent (mode ombre — aucune écriture)"
            ),
            "la section doit apparaître :\n{}",
            body.report_markdown
        );
    }

    /// La borne « depuis le cycle précédent » vient du checkpoint, pas d'une
    /// horloge : au premier cycle il n'y a pas de checkpoint, donc pas de borne
    /// — « depuis toujours », et non un silence ; au second, la borne est le
    /// `last_observed_at` que le premier a écrit.
    ///
    /// Le test pin aussi les URLs demandées : ce sont celles des sous-tickets
    /// du milestone, jamais une requête non bornée.
    #[tokio::test]
    async fn mika2698_la_borne_since_vient_du_checkpoint() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = mk_config(tmp.path());
        let rec = RecordingDeliverer::default();
        let (source, asked) = RecordingFlightSource::wired(vec![un_vol_sur_1802()]);

        for h in [12, 19] {
            let now = Utc.with_ymd_and_hms(2026, 8, 21, h, 0, 0).unwrap();
            run_manager_cycle_in(
                &cfg,
                base_state(),
                &CycleContext::new(&rec, now).with_flight_source(Some(&source)),
            )
            .await
            .unwrap();
        }

        let asked = asked.lock().unwrap();
        assert_eq!(asked.len(), 2, "une lecture par cycle");
        assert_eq!(
            asked[0].0, None,
            "premier cycle : aucun checkpoint, aucune borne"
        );
        assert_eq!(
            asked[1].0.as_deref(),
            Some("2026-08-20T00:00:00Z"),
            "second cycle : la borne est le `last_observed_at` du checkpoint"
        );
        assert_eq!(
            asked[0].1,
            vec!["https://github.com/senara-solutions/mika/issues/1802".to_string()],
            "les URLs demandées sont celles des sous-tickets du milestone"
        );
    }

    /// Le contrôle négatif du détecteur, côté cadence : un milestone **sans
    /// sous-ticket** ne doit pas faire lire quoi que ce soit. Sans cette borne,
    /// une liste d'URLs vide descendrait jusqu'au SQL, où elle se lirait comme
    /// « tous les vols de la base ».
    #[tokio::test]
    async fn mika2698_un_milestone_sans_sous_ticket_ne_lit_aucun_vol() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = mk_config(tmp.path());
        let rec = RecordingDeliverer::default();
        let now = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();
        let (source, asked) = RecordingFlightSource::wired(vec![un_vol_sur_1802()]);

        let mut state = base_state();
        state.sub_issues.clear();

        run_manager_cycle_in(
            &cfg,
            state,
            &CycleContext::new(&rec, now).with_flight_source(Some(&source)),
        )
        .await
        .unwrap();

        assert!(
            asked.lock().unwrap().is_empty(),
            "aucun sous-ticket ⇒ la source n'est même pas interrogée"
        );
        let calls = rec.calls.lock().unwrap();
        assert!(calls[0].2.assessment.flight_verdicts.is_empty());
    }

    // -- mika#2698 vol 2 : l'interrupteur, le geste, l'idempotence -----------

    /// L'exécutant `gh` du cycle, instrumenté : il enregistre chaque appel.
    #[derive(Default)]
    struct RecordingGh {
        appels: Mutex<Vec<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl GhRunner for RecordingGh {
        async fn run(&self, args: &[&str]) -> Result<String> {
            self.appels
                .lock()
                .unwrap()
                .push(args.iter().map(|a| (*a).to_string()).collect());
            Ok(String::new())
        }
    }

    /// Arme l'interrupteur pour la durée du test, et le désarme à la sortie
    /// **même en cas de panique** — un `assert!` qui échoue laisserait sinon la
    /// variable posée pour tous les tests suivants du processus.
    struct InterrupteurArme;

    impl InterrupteurArme {
        fn poser() -> Self {
            // SAFETY: single-threaded via serial_test.
            unsafe { std::env::set_var(ENV_EVALUATE_WRITE, "1") };
            Self
        }
    }

    impl Drop for InterrupteurArme {
        fn drop(&mut self) {
            // SAFETY: single-threaded via serial_test.
            unsafe { std::env::remove_var(ENV_EVALUATE_WRITE) };
        }
    }

    fn lire_checkpoint(cfg: &ManagerConfig) -> MilestoneCheckpoint {
        let path = cfg
            .checkpoint_dir
            .join(format!("{}.json", cfg.target.slug()));
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("checkpoint illisible en {}: {e}", path.display()));
        serde_json::from_str(&raw).unwrap()
    }

    /// **AC1 au niveau du cycle.** L'exécutant est câblé, un verdict existe, et
    /// l'interrupteur est à son état par défaut : aucun appel `gh`.
    ///
    /// Le câblage inconditionnel de l'exécutant est ce qui rend ce test
    /// possible — un runner posé seulement quand la variable est là rendrait la
    /// branche OFF structurellement muette, donc intestable.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2698_interrupteur_off_le_cycle_nappelle_pas_gh() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = mk_config(tmp.path());
        let rec = RecordingDeliverer::default();
        let gh = RecordingGh::default();
        let now = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();
        let (source, _asked) = RecordingFlightSource::wired(vec![un_vol_sur_1802()]);

        run_manager_cycle_in(
            &cfg,
            base_state(),
            &CycleContext::new(&rec, now)
                .with_flight_source(Some(&source))
                .with_gh_runner(Some(&gh)),
        )
        .await
        .unwrap();

        assert!(
            gh.appels.lock().unwrap().is_empty(),
            "AC1 : aucune écriture gh avec l'interrupteur OFF, vu : {:?}",
            gh.appels.lock().unwrap()
        );
        assert!(
            lire_checkpoint(&cfg).posted_flight_task_ids.is_empty(),
            "et rien n'est marqué comme posté"
        );
    }

    /// Le **contrôle négatif** de l'AC1, au niveau du cycle : armé, le même
    /// cycle poste exactement une fois, et le checkpoint retient le vol.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2698_interrupteur_on_le_cycle_poste_une_fois() {
        let _arme = InterrupteurArme::poser();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = mk_config(tmp.path());
        let rec = RecordingDeliverer::default();
        let gh = RecordingGh::default();
        let now = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();
        let (source, _asked) = RecordingFlightSource::wired(vec![un_vol_sur_1802()]);

        run_manager_cycle_in(
            &cfg,
            base_state(),
            &CycleContext::new(&rec, now)
                .with_flight_source(Some(&source))
                .with_gh_runner(Some(&gh)),
        )
        .await
        .unwrap();

        let appels = gh.appels.lock().unwrap().clone();
        assert_eq!(appels.len(), 1, "un commentaire pour un vol : {appels:?}");
        assert_eq!(appels[0][0], "issue");
        assert_eq!(appels[0][1], "comment");
        assert_eq!(appels[0][2], "1802", "sur le ticket du vol");
        assert_eq!(
            lire_checkpoint(&cfg).posted_flight_task_ids,
            vec!["task-1802".to_string()],
            "le checkpoint retient le vol posté"
        );
    }

    /// **AC2.** Le rejeu du cycle ne reposte pas. Trois cycles, le même vol
    /// rendu à chaque fois par la source : un seul commentaire.
    ///
    /// Le troisième cycle est celui qui compte : il tombe dans le cas
    /// `should_deliver == false` (ni changement d'état, ni heartbeat), donc sur
    /// la branche qui n'existait pas avant ce vol.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2698_le_rejeu_du_cycle_ne_reposte_pas() {
        let _arme = InterrupteurArme::poser();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = mk_config(tmp.path());
        let rec = RecordingDeliverer::default();
        let gh = RecordingGh::default();
        let (source, _asked) = RecordingFlightSource::wired(vec![un_vol_sur_1802()]);

        let t0 = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();
        for minutes in [0, 10, 20] {
            run_manager_cycle_in(
                &cfg,
                base_state(),
                &CycleContext::new(&rec, t0 + chrono::Duration::minutes(minutes))
                    .with_flight_source(Some(&source))
                    .with_gh_runner(Some(&gh)),
            )
            .await
            .unwrap();
        }

        let appels = gh.appels.lock().unwrap().clone();
        assert_eq!(
            appels.len(),
            1,
            "AC2 : un seul commentaire par vol sur trois cycles : {appels:?}"
        );
    }

    /// Posté sans livrer : le checkpoint est écrit **pour cette raison seule**,
    /// et les trois champs de cadence sont préservés.
    ///
    /// Sans quoi la sauvegarde répondrait à la place de la livraison : le
    /// `state_digest` du nouvel état serait figé alors qu'aucun rapport ne l'a
    /// porté, et l'événement `state_changed` en attente serait avalé.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2698_poster_sans_livrer_preserve_les_champs_de_cadence() {
        let _arme = InterrupteurArme::poser();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = mk_config(tmp.path());
        let rec = RecordingDeliverer::default();
        let t0 = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();

        // Cycle 1 : livre, sans source de vols — le checkpoint porte le digest
        // de l'état de base et aucun vol posté.
        run_manager_cycle_in(&cfg, base_state(), &CycleContext::new(&rec, t0))
            .await
            .unwrap();
        let apres_livraison = lire_checkpoint(&cfg);
        assert!(apres_livraison.posted_flight_task_ids.is_empty());

        // Cycle 2 : dix minutes plus tard, état **inchangé** (donc pas de
        // livraison), mais un vol à poster.
        let gh = RecordingGh::default();
        let (source, _asked) = RecordingFlightSource::wired(vec![un_vol_sur_1802()]);
        let outcome = run_manager_cycle_in(
            &cfg,
            base_state(),
            &CycleContext::new(&rec, t0 + chrono::Duration::minutes(10))
                .with_flight_source(Some(&source))
                .with_gh_runner(Some(&gh)),
        )
        .await
        .unwrap();

        assert!(!outcome.delivered, "ni changement d'état ni heartbeat");
        assert_eq!(gh.appels.lock().unwrap().len(), 1, "et pourtant il poste");

        let apres_post = lire_checkpoint(&cfg);
        assert_eq!(
            apres_post.posted_flight_task_ids,
            vec!["task-1802".to_string()],
            "le vol posté est retenu"
        );
        assert_eq!(
            apres_post.state_digest, apres_livraison.state_digest,
            "le digest est préservé : cette sauvegarde ne parle pas de livraison"
        );
        assert_eq!(
            apres_post.last_delivered_at, apres_livraison.last_delivered_at,
            "la date de dernière livraison est préservée"
        );
        assert_eq!(
            apres_post.last_observed_at, apres_livraison.last_observed_at,
            "la borne `since` du prochain cycle est préservée"
        );
    }

    /// Un checkpoint écrit avant ce vol se relit en liste vide — `serde(default)`.
    #[test]
    fn mika2698_un_checkpoint_davant_le_vol_2_se_relit() {
        let ancien = r#"{"state_digest":"abc","last_delivered_at":"2026-08-21T12:00:00Z","last_observed_at":"2026-08-20T00:00:00Z"}"#;
        let c: MilestoneCheckpoint = serde_json::from_str(ancien).unwrap();
        assert_eq!(c.state_digest, "abc");
        assert!(
            c.posted_flight_task_ids.is_empty(),
            "le champ absent se relit en liste vide, jamais en erreur"
        );
    }

    /// La borne de rétention garde les plus **récents**, et son débordement est
    /// une re-publication possible — jamais une perte silencieuse.
    #[test]
    fn mika2698_la_retention_des_vols_postes_est_bornee() {
        let anciens: Vec<String> = (0..POSTED_FLIGHTS_RETAINED)
            .map(|i| format!("t-{i}"))
            .collect();
        let merged = merge_posted(&anciens, &["t-neuf".to_string()]);
        assert_eq!(merged.len(), POSTED_FLIGHTS_RETAINED);
        assert_eq!(
            merged.last().unwrap(),
            "t-neuf",
            "le plus récent est retenu"
        );
        assert_eq!(merged[0], "t-1", "le plus ancien est sorti");
    }

    /// Sans exécutant câblé, rien ne poste — quel que soit l'interrupteur.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2698_sans_executeur_rien_ne_poste_meme_arme() {
        let _arme = InterrupteurArme::poser();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = mk_config(tmp.path());
        let rec = RecordingDeliverer::default();
        let now = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();
        let (source, _asked) = RecordingFlightSource::wired(vec![un_vol_sur_1802()]);

        run_manager_cycle_in(
            &cfg,
            base_state(),
            &CycleContext::new(&rec, now).with_flight_source(Some(&source)),
        )
        .await
        .unwrap();

        assert!(lire_checkpoint(&cfg).posted_flight_task_ids.is_empty());
    }
}

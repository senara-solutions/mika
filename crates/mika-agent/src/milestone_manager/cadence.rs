//! Cadence — event-driven + 6h plancher heartbeat orchestration.
//!
//! **LECTURE SEULE.** The only outbound side effect is a `POST` to a
//! well-known delivery endpoint (Prime→sami→Vincent per D8 subsystem-2
//! pattern), or an offline sink write when the URL is unset. No GitHub
//! writes, no dispatch, no ticket mutations.
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
    reader::Reader,
    reporter::Reporter,
    types::{Assessment, CycleOutcome, MilestoneRef, MilestoneState, Severity},
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
}

impl<'a> CycleContext<'a> {
    pub fn new(deliverer: &'a dyn ReportDeliverer, now: DateTime<Utc>) -> Self {
        Self {
            deliverer,
            now,
            auth_ledger: None,
            auth_notes: &[],
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
    let assessment = assessor.assess(&state);
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
                            route = "http",
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
                    route = "offline_sink",
                    escalated = escalated,
                );
            }
        }

        // Persist checkpoint on delivery.
        let new_checkpoint = MilestoneCheckpoint {
            state_digest: digest,
            last_delivered_at: Some(generated_at.clone()),
            last_observed_at: state.last_activity_at.clone(),
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
        Some(u) if !u.is_empty() => Route::Http {
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
    run_manager_cycle_with_auth(cfg, None, &[]).await
}

/// As [`run_manager_cycle`], with the mika#1949 auth-boundary wiring: where to
/// record an observed failure, and which failures the report should carry
/// forward from previous cycles.
pub async fn run_manager_cycle_with_auth(
    cfg: &ManagerConfig,
    auth_ledger: Option<&Arc<dyn AuthBoundaryLedger>>,
    auth_notes: &[crate::milestone_manager::reporter::AuthBoundaryNote],
) -> Result<CycleOutcome> {
    let reader = Reader::new(cfg.github_token.clone());
    let mut state = reader.read(&cfg.target).await?;
    // M4 review fix: honour the health_url config surface.
    state.executor_healthy = probe_executor_health(cfg.health_url.as_deref()).await;
    let deliverer = HttpReportDeliverer::new();
    let ctx = CycleContext::new(&deliverer, Utc::now())
        .with_auth_ledger(auth_ledger)
        .with_auth_notes(auth_notes);
    run_manager_cycle_in(cfg, state, &ctx).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::milestone_manager::types::{
        CiState, IssueState, MilestoneRef, ProgressCounts, RecentActivity, SubIssue,
    };
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

    #[tokio::test]
    async fn probe_executor_health_returns_none_on_bogus_host() {
        // Non-routable RFC 5737 address — connection fails fast, fail-open to None.
        let res = probe_executor_health(Some("http://192.0.2.1:1/health")).await;
        assert_eq!(res, None);
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
}

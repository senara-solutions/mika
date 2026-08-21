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
use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;

/// Configuration for a single manager cycle.
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    pub target: MilestoneRef,
    /// Optional GitHub token forwarded to `gh` subprocesses.
    pub github_token: Option<String>,
    /// Heartbeat interval — cycle always delivers if `now - last_delivery >= interval`.
    pub heartbeat_interval: chrono::Duration,
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
        let res = req.send().await?;
        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            return Err(anyhow!("delivery failed: {status} — {text}"));
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
    let report = Reporter::new().report(&state, &assessment);

    let should_deliver = state_changed || heartbeat_fired;
    let cycle_kind = match (state_changed, heartbeat_fired) {
        (true, true) => CycleKind::EventAndHeartbeat,
        (true, false) => CycleKind::Event,
        (false, true) => CycleKind::Heartbeat,
        (false, false) => CycleKind::Event, // unused when should_deliver=false
    };

    let mut escalated = false;
    let mut delivered = false;

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
                match deliverer.deliver(&url, token.as_deref(), &body).await {
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
    })
}

enum Route {
    Http { url: String, token: Option<String> },
    OfflineSink,
}

fn select_route(severity: &Severity, cfg: &ManagerConfig) -> Route {
    let url = match severity {
        Severity::Blocked => cfg
            .escalation_url
            .clone()
            .or_else(|| cfg.delivery_url.clone()),
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
    let mut f = tokio::fs::File::create(&path).await?;
    f.write_all(&bytes).await?;
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
    let mut f = tokio::fs::File::create(&path).await?;
    f.write_all(body.report_markdown.as_bytes()).await?;
    Ok(())
}

/// Live-runner variant: reads from `gh`, then runs the same cycle logic.
pub async fn run_manager_cycle(cfg: &ManagerConfig) -> Result<CycleOutcome> {
    let reader = Reader::new(cfg.github_token.clone());
    let state = reader.read(&cfg.target).await?;
    let deliverer = HttpReportDeliverer::new();
    run_manager_cycle_with(cfg, state, &deliverer, Utc::now()).await
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
}

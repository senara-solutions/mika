//! Cadence spawn wiring — env-var config assembly + tokio spawn.
//!
//! **LECTURE seule.** This module composes `run_manager_cycle` on an interval
//! and cancellation token. It adds no dispatch authority. Every outbound side
//! effect flows through the existing composers in `cadence.rs` (HTTP POST to
//! the delivery/escalation URL, or an offline sink write when unset).
//!
//! ## Env-gate contract
//!
//! `MIKA_MANAGER_TARGET_MILESTONE` is the single feature-gate:
//! - Unset → `manager_config_from_env()` returns `Ok(None)`; the server should
//!   log an INFO line and skip the spawn.
//! - Set to a valid `<owner/repo>#<number>` → `Ok(Some(cfg))`.
//! - Set to a malformed value → `Err(...)` (loud); the server should log an
//!   ERROR and skip the spawn (do not crash startup).
//!
//! All other `MIKA_MANAGER_*` env vars are optional refinements with
//! three-tier fallback: absent → default; unparseable → default + WARN;
//! valid → use value. The exact defaults are documented in the field
//! comments below and echoed in the root `CLAUDE.md`.
//!
//! ## Sibling pattern
//!
//! Structurally mirrors `kg::resolver_tick::spawn_resolver_tick_task`:
//! `tokio::time::interval` + `tokio::select!` on `interval.tick()` vs
//! `cancel.cancelled()`. Cycle errors log via `tracing::warn!` and continue
//! (fail-open). Graceful shutdown responds to SIGTERM within one poll
//! interval.

use super::cadence::{ManagerConfig, run_manager_cycle};
use super::types::MilestoneRef;
use std::env;
use std::path::PathBuf;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Env-var name for the Phase 1 single-target milestone. Setting this
/// enables the cadence loop; unset → cadence disabled.
pub const ENV_TARGET_MILESTONE: &str = "MIKA_MANAGER_TARGET_MILESTONE";
pub const ENV_HEARTBEAT_INTERVAL_SECS: &str = "MIKA_MANAGER_HEARTBEAT_INTERVAL_SECS";
pub const ENV_POLL_INTERVAL_SECS: &str = "MIKA_MANAGER_POLL_INTERVAL_SECS";
pub const ENV_SILENCE_THRESHOLD_DAYS: &str = "MIKA_MANAGER_SILENCE_THRESHOLD_DAYS";
pub const ENV_DELIVERY_URL: &str = "MIKA_MANAGER_DELIVERY_URL";
pub const ENV_DELIVERY_TOKEN: &str = "MIKA_MANAGER_DELIVERY_TOKEN";
pub const ENV_ESCALATION_URL: &str = "MIKA_MANAGER_ESCALATION_URL";
pub const ENV_HEALTH_URL: &str = "MIKA_MANAGER_HEALTH_URL";
pub const ENV_CHECKPOINT_DIR: &str = "MIKA_MANAGER_CHECKPOINT_DIR";
pub const ENV_OFFLINE_SINK_DIR: &str = "MIKA_MANAGER_OFFLINE_SINK_DIR";

/// Default heartbeat interval in seconds (6 hours per brief § verdict 2).
pub const DEFAULT_HEARTBEAT_INTERVAL_SECS: i64 = 21_600;
/// Default silence threshold in days (per brief § verdict 5).
pub const DEFAULT_SILENCE_THRESHOLD_DAYS: u32 = 3;
/// Default poll interval in seconds. Cap the cycle latency at 5 min so
/// state-change events are surfaced quickly; the internal cycle is a no-op
/// when neither `state_changed` nor `heartbeat_fired` is true, so a shorter
/// poll costs one `gh` invocation and a digest comparison — cheap.
pub const DEFAULT_POLL_INTERVAL_SECS: u64 = 300;
/// Fallback root for checkpoint/offline-sink dirs when the env vars and
/// `HOME` are all unset (last-resort — should not be reached in production).
const FALLBACK_STATE_ROOT: &str = "/tmp/mika-manager";

/// Assemble a `ManagerConfig` from the current process env, or return
/// `Ok(None)` when the feature-gate env var is unset.
///
/// Errors are limited to structural parse failures on `MIKA_MANAGER_TARGET_MILESTONE`
/// (missing `#`, non-numeric number, empty repo) — the operator asked for a target
/// but named a broken one. Numeric refinements (heartbeat, poll, silence) fall
/// back to their defaults with a WARN log; they never fail loud.
pub fn manager_config_from_env() -> Result<Option<ManagerConfig>, String> {
    let target_raw = match env::var(ENV_TARGET_MILESTONE) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => return Ok(None),
    };
    let target = MilestoneRef::parse(target_raw.trim())
        .map_err(|e| format!("{ENV_TARGET_MILESTONE}: {e}"))?;

    let heartbeat_secs = read_u64_env(
        ENV_HEARTBEAT_INTERVAL_SECS,
        DEFAULT_HEARTBEAT_INTERVAL_SECS as u64,
    );
    let poll_secs = read_u64_env(ENV_POLL_INTERVAL_SECS, DEFAULT_POLL_INTERVAL_SECS);
    let silence_days = read_u32_env(ENV_SILENCE_THRESHOLD_DAYS, DEFAULT_SILENCE_THRESHOLD_DAYS);

    // Poll interval floors at 1s and never exceeds heartbeat — a poll_interval
    // larger than the heartbeat would delay heartbeat delivery indefinitely.
    let poll_secs_effective = poll_secs.max(1).min(heartbeat_secs.max(1));

    // Optional string env vars — treat empty string as unset per convention.
    let delivery_url = read_string_env(ENV_DELIVERY_URL);
    let delivery_token = read_string_env(ENV_DELIVERY_TOKEN);
    let escalation_url = read_string_env(ENV_ESCALATION_URL);
    let health_url = read_string_env(ENV_HEALTH_URL);

    // GitHub token — reuse the same `MIKA_GITHUB_TOKEN` the rest of the crate
    // reads via `Settings::agent_github_token`. Direct env read here keeps
    // this module standalone (no Settings threading required); the token is
    // forwarded to `gh` subprocesses via the Reader's existing shape.
    let github_token = read_string_env("MIKA_GITHUB_TOKEN");

    let checkpoint_dir = read_path_env(ENV_CHECKPOINT_DIR)
        .unwrap_or_else(|| default_state_root().join("checkpoints"));
    let offline_sink_dir =
        read_path_env(ENV_OFFLINE_SINK_DIR).unwrap_or_else(|| default_state_root().join("sink"));

    Ok(Some(ManagerConfig {
        target,
        github_token,
        heartbeat_interval: chrono::Duration::seconds(heartbeat_secs as i64),
        poll_interval: chrono::Duration::seconds(poll_secs_effective as i64),
        silence_threshold_days: silence_days,
        delivery_url,
        delivery_token,
        escalation_url,
        health_url,
        checkpoint_dir,
        offline_sink_dir,
    }))
}

/// Spawn the manager cadence loop as a background tokio task.
///
/// The loop polls at `cfg.poll_interval` and invokes `run_manager_cycle` on
/// each tick. The first tick is skipped (mirrors the KG resolver pattern) so
/// the process does not fire a cycle before startup work has settled.
///
/// Cycle failures are logged at WARN and do not stop the loop. Graceful
/// shutdown responds to `cancel.cancelled()` within one poll interval.
pub fn spawn_manager_cycle_task(cfg: ManagerConfig, cancel: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        info!(
            target: "mika::milestone_manager",
            event = "manager_cadence_start",
            milestone = %cfg.target.as_display(),
            heartbeat_secs = cfg.heartbeat_interval.num_seconds(),
            poll_secs = cfg.poll_interval.num_seconds(),
            delivery_url_set = cfg.delivery_url.is_some(),
            escalation_url_set = cfg.escalation_url.is_some(),
            "mika-manager cadence started"
        );

        let poll = duration_from_chrono(cfg.poll_interval);
        let mut interval = tokio::time::interval(poll);
        // Skip the first immediate fire so we don't cycle before the server
        // finishes initializing sibling subsystems (KG ticks, checkpoint,
        // wedge watchdog).
        interval.tick().await;

        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = cancel.cancelled() => {
                    info!(
                        target: "mika::milestone_manager",
                        event = "manager_cadence_cancelled",
                        milestone = %cfg.target.as_display(),
                        "mika-manager cadence stopped (graceful shutdown)"
                    );
                    return;
                }
            }
            // Double-check before starting a potentially long cycle.
            if cancel.is_cancelled() {
                return;
            }

            match run_manager_cycle(&cfg).await {
                Ok(outcome) => {
                    if outcome.delivered {
                        info!(
                            target: "mika::milestone_manager",
                            event = "manager_cycle_delivered",
                            milestone = %cfg.target.as_display(),
                            severity = ?outcome.severity,
                            state_changed = outcome.state_changed,
                            heartbeat_fired = outcome.heartbeat_fired,
                            escalated = outcome.escalated,
                        );
                    } else {
                        // No-op cycle — expected when state unchanged and
                        // heartbeat hasn't elapsed. Log at TRACE-equivalent
                        // via structured info at low verbosity by omitting;
                        // silence keeps the log cheap.
                    }
                }
                Err(e) => {
                    warn!(
                        target: "mika::milestone_manager",
                        event = "manager_cycle_error",
                        milestone = %cfg.target.as_display(),
                        error = %e,
                        "manager cycle failed — continuing loop"
                    );
                }
            }
        }
    })
}

// ---- env-var helpers ------------------------------------------------------

/// Read a `u64` env var with three-tier fallback: absent/empty → default;
/// unparseable/zero → default + WARN; valid → use value. Zero is treated as
/// invalid because a zero interval would spin.
fn read_u64_env(name: &str, default: u64) -> u64 {
    match env::var(name) {
        Ok(raw) if !raw.trim().is_empty() => match raw.trim().parse::<u64>() {
            Ok(n) if n > 0 => n,
            Ok(_) => {
                warn!(
                    target: "mika::milestone_manager",
                    event = "manager_env_invalid",
                    var = name,
                    raw = %raw,
                    reason = "zero_or_negative",
                    default,
                    "invalid env value — using default"
                );
                default
            }
            Err(e) => {
                warn!(
                    target: "mika::milestone_manager",
                    event = "manager_env_invalid",
                    var = name,
                    raw = %raw,
                    reason = "parse_error",
                    error = %e,
                    default,
                    "invalid env value — using default"
                );
                default
            }
        },
        _ => default,
    }
}

/// Same three-tier fallback for `u32`. Zero is accepted for `u32` because
/// `silence_threshold_days == 0` is a legitimate "always alert on silence"
/// tuning; the Assessor consumes it verbatim.
fn read_u32_env(name: &str, default: u32) -> u32 {
    match env::var(name) {
        Ok(raw) if !raw.trim().is_empty() => match raw.trim().parse::<u32>() {
            Ok(n) => n,
            Err(e) => {
                warn!(
                    target: "mika::milestone_manager",
                    event = "manager_env_invalid",
                    var = name,
                    raw = %raw,
                    reason = "parse_error",
                    error = %e,
                    default,
                    "invalid env value — using default"
                );
                default
            }
        },
        _ => default,
    }
}

/// Read an optional string env var. Empty/whitespace-only → `None`.
fn read_string_env(name: &str) -> Option<String> {
    match env::var(name) {
        Ok(v) if !v.trim().is_empty() => Some(v),
        _ => None,
    }
}

/// Read an optional path env var. Empty/whitespace-only → `None`.
fn read_path_env(name: &str) -> Option<PathBuf> {
    read_string_env(name).map(PathBuf::from)
}

/// Resolve the fallback root for state directories (checkpoint + offline sink)
/// when the explicit env vars are unset. Prefers `$HOME/.mika/manager`; falls
/// back to a well-known /tmp path (last-resort, should not be hit in
/// production but keeps the spawn from panicking if HOME is unset).
fn default_state_root() -> PathBuf {
    if let Ok(home) = env::var("HOME")
        && !home.trim().is_empty()
    {
        return PathBuf::from(home).join(".mika").join("manager");
    }
    PathBuf::from(FALLBACK_STATE_ROOT)
}

/// Convert a `chrono::Duration` to `std::time::Duration`, clamping negatives
/// to 1s so the tokio interval never receives a zero/negative period.
fn duration_from_chrono(d: chrono::Duration) -> Duration {
    let secs = d.num_seconds();
    if secs <= 0 {
        Duration::from_secs(1)
    } else {
        Duration::from_secs(secs as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::milestone_manager::cadence::{
        DeliveryBody, ReportDeliverer, run_manager_cycle_with,
    };
    use crate::milestone_manager::types::{
        CiState, IssueState, MilestoneState, ProgressCounts, RecentActivity, SubIssue,
    };
    use anyhow::Result;
    use chrono::Utc;
    use serial_test::serial;
    use std::sync::{Arc, Mutex};

    /// Unset every `MIKA_MANAGER_*` env var this module reads. Called at the
    /// top of every serial env test to give a clean slate.
    fn clear_manager_env() {
        // SAFETY: tests are serialized via `#[serial_test::serial]`.
        unsafe {
            for name in [
                ENV_TARGET_MILESTONE,
                ENV_HEARTBEAT_INTERVAL_SECS,
                ENV_POLL_INTERVAL_SECS,
                ENV_SILENCE_THRESHOLD_DAYS,
                ENV_DELIVERY_URL,
                ENV_DELIVERY_TOKEN,
                ENV_ESCALATION_URL,
                ENV_HEALTH_URL,
                ENV_CHECKPOINT_DIR,
                ENV_OFFLINE_SINK_DIR,
            ] {
                env::remove_var(name);
            }
        }
    }

    fn set_env(name: &str, value: &str) {
        // SAFETY: tests are serialized via `#[serial_test::serial]`.
        unsafe { env::set_var(name, value) };
    }

    #[test]
    #[serial]
    fn env_unset_returns_none() {
        clear_manager_env();
        let result = manager_config_from_env().expect("no error when target unset");
        assert!(
            result.is_none(),
            "cadence must be disabled when target unset"
        );
    }

    #[test]
    #[serial]
    fn env_target_empty_string_returns_none() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "   ");
        let result = manager_config_from_env().expect("whitespace target treated as unset");
        assert!(
            result.is_none(),
            "whitespace-only target must be treated as unset"
        );
    }

    #[test]
    #[serial]
    fn env_set_returns_some_with_defaults() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "senara-solutions/mika#30");
        let cfg = manager_config_from_env()
            .expect("no parse error")
            .expect("Some when target set");
        assert_eq!(cfg.target.repo, "senara-solutions/mika");
        assert_eq!(cfg.target.number, 30);
        assert_eq!(
            cfg.heartbeat_interval.num_seconds(),
            DEFAULT_HEARTBEAT_INTERVAL_SECS
        );
        assert_eq!(cfg.silence_threshold_days, DEFAULT_SILENCE_THRESHOLD_DAYS);
        assert_eq!(
            cfg.poll_interval.num_seconds() as u64,
            DEFAULT_POLL_INTERVAL_SECS
        );
        assert!(cfg.delivery_url.is_none());
        assert!(cfg.escalation_url.is_none());
    }

    #[test]
    #[serial]
    fn env_set_parses_full_config() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "senara-solutions/mika#42");
        set_env(ENV_HEARTBEAT_INTERVAL_SECS, "3600");
        set_env(ENV_POLL_INTERVAL_SECS, "120");
        set_env(ENV_SILENCE_THRESHOLD_DAYS, "7");
        set_env(ENV_DELIVERY_URL, "https://cm.example.com/manager/deliver");
        set_env(ENV_DELIVERY_TOKEN, "tok-123");
        set_env(ENV_ESCALATION_URL, "https://cm.example.com/vincent/direct");
        set_env(ENV_HEALTH_URL, "https://cm.example.com/health/mika-dev");
        set_env(ENV_CHECKPOINT_DIR, "/var/lib/mika-manager/checkpoints");
        set_env(ENV_OFFLINE_SINK_DIR, "/var/lib/mika-manager/sink");

        let cfg = manager_config_from_env().expect("valid").expect("Some");
        assert_eq!(cfg.target.number, 42);
        assert_eq!(cfg.heartbeat_interval.num_seconds(), 3600);
        assert_eq!(cfg.poll_interval.num_seconds(), 120);
        assert_eq!(cfg.silence_threshold_days, 7);
        assert_eq!(
            cfg.delivery_url.as_deref(),
            Some("https://cm.example.com/manager/deliver")
        );
        assert_eq!(cfg.delivery_token.as_deref(), Some("tok-123"));
        assert_eq!(
            cfg.escalation_url.as_deref(),
            Some("https://cm.example.com/vincent/direct")
        );
        assert_eq!(
            cfg.health_url.as_deref(),
            Some("https://cm.example.com/health/mika-dev")
        );
        assert_eq!(
            cfg.checkpoint_dir.to_string_lossy(),
            "/var/lib/mika-manager/checkpoints"
        );
        assert_eq!(
            cfg.offline_sink_dir.to_string_lossy(),
            "/var/lib/mika-manager/sink"
        );
    }

    #[test]
    #[serial]
    fn env_invalid_heartbeat_falls_back_to_default() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "senara-solutions/mika#1");
        set_env(ENV_HEARTBEAT_INTERVAL_SECS, "not-a-number");
        let cfg = manager_config_from_env()
            .expect("target valid")
            .expect("Some");
        assert_eq!(
            cfg.heartbeat_interval.num_seconds(),
            DEFAULT_HEARTBEAT_INTERVAL_SECS
        );
    }

    #[test]
    #[serial]
    fn env_invalid_poll_falls_back_to_default() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "senara-solutions/mika#1");
        set_env(ENV_POLL_INTERVAL_SECS, "junk");
        let cfg = manager_config_from_env()
            .expect("target valid")
            .expect("Some");
        assert_eq!(
            cfg.poll_interval.num_seconds() as u64,
            DEFAULT_POLL_INTERVAL_SECS
        );
    }

    #[test]
    #[serial]
    fn env_zero_heartbeat_falls_back_to_default() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "senara-solutions/mika#1");
        set_env(ENV_HEARTBEAT_INTERVAL_SECS, "0");
        let cfg = manager_config_from_env()
            .expect("target valid")
            .expect("Some");
        assert_eq!(
            cfg.heartbeat_interval.num_seconds(),
            DEFAULT_HEARTBEAT_INTERVAL_SECS,
            "zero heartbeat must not spin — falls back to default"
        );
    }

    #[test]
    #[serial]
    fn env_invalid_silence_falls_back_to_default() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "senara-solutions/mika#1");
        set_env(ENV_SILENCE_THRESHOLD_DAYS, "notanumber");
        let cfg = manager_config_from_env()
            .expect("target valid")
            .expect("Some");
        assert_eq!(cfg.silence_threshold_days, DEFAULT_SILENCE_THRESHOLD_DAYS);
    }

    #[test]
    #[serial]
    fn env_zero_silence_days_accepted() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "senara-solutions/mika#1");
        set_env(ENV_SILENCE_THRESHOLD_DAYS, "0");
        let cfg = manager_config_from_env()
            .expect("target valid")
            .expect("Some");
        assert_eq!(
            cfg.silence_threshold_days, 0,
            "0 days is a legitimate 'always alert' tuning"
        );
    }

    #[test]
    #[serial]
    fn env_invalid_target_returns_error() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "malformed-no-hash");
        let err = manager_config_from_env().expect_err("malformed target must error");
        assert!(
            err.contains(ENV_TARGET_MILESTONE),
            "error must name the offending env var: {err}"
        );
    }

    #[test]
    #[serial]
    fn env_target_non_numeric_returns_error() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "senara-solutions/mika#not-a-num");
        let err = manager_config_from_env().expect_err("non-numeric number must error");
        assert!(err.contains(ENV_TARGET_MILESTONE));
    }

    #[test]
    #[serial]
    fn env_poll_larger_than_heartbeat_is_clamped_to_heartbeat() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "senara-solutions/mika#1");
        set_env(ENV_HEARTBEAT_INTERVAL_SECS, "60");
        set_env(ENV_POLL_INTERVAL_SECS, "999999");
        let cfg = manager_config_from_env().expect("valid").expect("Some");
        assert_eq!(
            cfg.poll_interval.num_seconds(),
            cfg.heartbeat_interval.num_seconds(),
            "poll_interval must not exceed heartbeat — a longer poll would delay heartbeat delivery"
        );
    }

    // ---- spawn behavior tests (do NOT touch env — use ManagerConfig directly)

    fn base_state() -> MilestoneState {
        MilestoneState {
            milestone_ref: MilestoneRef {
                repo: "senara-solutions/mika".into(),
                number: 30,
            },
            title: "test".into(),
            description: "".into(),
            state: IssueState::Open,
            created_at: "".into(),
            due_on: None,
            last_activity_at: Some("2026-08-22T00:00:00Z".into()),
            sub_issues: vec![SubIssue {
                number: 100,
                title: "T".into(),
                state: IssueState::Open,
                priority_rank: None,
                plan_present: true,
                branch_present: true,
                pr_number: Some(200),
                pr_state: Some("open".into()),
                ci_state: CiState::Success,
                blockers: vec![],
                updated_at: "2026-08-22T00:00:00Z".into(),
                labels: vec![],
            }],
            progress: ProgressCounts {
                in_flight: 1,
                total: 1,
                ..Default::default()
            },
            recent_activity: vec![RecentActivity {
                at: "2026-08-22T00:00:00Z".into(),
                kind: "sub_issue_closed".into(),
                subject: "#99".into(),
            }],
            executor_healthy: None,
        }
    }

    #[derive(Default, Clone)]
    struct CountingDeliverer {
        count: Arc<Mutex<usize>>,
    }

    #[async_trait::async_trait]
    impl ReportDeliverer for CountingDeliverer {
        async fn deliver(
            &self,
            _url: &str,
            _token: Option<&str>,
            _body: &DeliveryBody,
        ) -> Result<()> {
            *self.count.lock().unwrap() += 1;
            Ok(())
        }
    }

    fn mk_test_cfg(
        dir: &std::path::Path,
        heartbeat: chrono::Duration,
        poll: chrono::Duration,
    ) -> ManagerConfig {
        ManagerConfig {
            target: MilestoneRef {
                repo: "senara-solutions/mika".into(),
                number: 30,
            },
            github_token: None,
            heartbeat_interval: heartbeat,
            poll_interval: poll,
            silence_threshold_days: 3,
            delivery_url: Some("http://normal/deliver".into()),
            delivery_token: Some("t".into()),
            escalation_url: Some("http://vincent/direct".into()),
            health_url: None,
            checkpoint_dir: dir.join("checkpoints"),
            offline_sink_dir: dir.join("sink"),
        }
    }

    /// End-to-end sanity: exercise the same cycle body that
    /// `spawn_manager_cycle_task` calls under the hood, using the direct
    /// `run_manager_cycle_with` API with a controlled clock and recording
    /// deliverer. This proves the composition contract without spinning a
    /// real tokio interval or reaching for tokio's test-util `pause()`
    /// (which would fight the reqwest client the spawn constructs).
    #[tokio::test]
    async fn cycle_delivers_on_first_call_via_composed_path() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = mk_test_cfg(
            tmp.path(),
            chrono::Duration::hours(6),
            chrono::Duration::seconds(30),
        );
        let deliverer = CountingDeliverer::default();
        let outcome = run_manager_cycle_with(&cfg, base_state(), &deliverer, Utc::now())
            .await
            .expect("cycle ok");
        assert!(outcome.delivered);
        assert_eq!(*deliverer.count.lock().unwrap(), 1);
    }

    /// Spawn the real loop with a tiny poll interval and verify cancellation
    /// is honored within a bounded window. We don't assert on `run_manager_cycle`
    /// succeeding here (it will fail because there's no live `gh` for this
    /// synthetic milestone) — we assert only that the cancel path drops the
    /// task promptly. This is AC7.
    #[tokio::test]
    async fn spawn_respects_cancel_token() {
        let tmp = tempfile::tempdir().unwrap();
        // Use a target that will fail at the `gh` boundary — that's fine, the
        // cycle body logs a warn and continues. We're testing the loop's
        // cancel-response, not cycle success.
        let cfg = mk_test_cfg(
            tmp.path(),
            chrono::Duration::seconds(1),
            chrono::Duration::milliseconds(50),
        );
        let cancel = CancellationToken::new();
        let handle = spawn_manager_cycle_task(cfg, cancel.clone());

        // Let the loop tick at least once.
        tokio::time::sleep(Duration::from_millis(200)).await;
        cancel.cancel();

        // Wait for the task to exit — should be well under 1s given the 50ms poll.
        let join = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(
            join.is_ok(),
            "spawn task must exit within 2s of cancel — got timeout"
        );
        join.unwrap().expect("join should not panic");
    }

    /// Injection-verified: the env-gate is the load-bearing predicate for
    /// AC1. If `manager_config_from_env` ever silently returns `Some(_)` when
    /// the target is unset, the spawn wiring in `server::mod.rs` will start
    /// a rogue cadence loop against `MilestoneRef { repo: "", number: 0 }` or
    /// similar. This test locks the invariant: unset target ⇒ None, no
    /// side-effect fallback.
    #[test]
    #[serial]
    fn env_gate_is_load_bearing_default_off() {
        clear_manager_env();
        for _ in 0..5 {
            // Repeat to guard against any stray singleton/state cache.
            assert!(manager_config_from_env().unwrap().is_none());
        }
    }
}

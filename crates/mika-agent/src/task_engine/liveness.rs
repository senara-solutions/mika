//! Engine tick-loop liveness heartbeat + wedge watchdog (mika#1850).
//!
//! ## Why
//!
//! The task engine's tick loop is a single `tokio::spawn` that can wedge
//! silently — panic in a downstream await gets swallowed, deadlock on a lock
//! held elsewhere goes undetected, blocking sync call in the async runtime
//! stalls the reactor. The process stays alive (checkpoint task keeps
//! running from its own spawn), but the engine tick stops firing.
//!
//! Empirical incident 2026-07-27T01:51:43Z: engine wedged for 7+ hours
//! undetected. No panic in log, no error, just silence except the
//! background checkpoint every 60s. Detection required an operator to
//! notice "0 merges since #1847" and MPC to grep the log manually.
//!
//! ## How
//!
//! Two moving parts:
//!
//! - [`EngineHeartbeat`]: an `Arc<AtomicI64>` holding the wall-clock unix
//!   timestamp of the last tick. The engine's `spawn_tick_loop` calls
//!   [`EngineHeartbeat::tick`] at the top of every iteration (throttled
//!   [`HEARTBEAT_LOG_INTERVAL_SECS`] on the INFO log to avoid spam).
//! - [`spawn_engine_wedge_watchdog`]: a separate long-lived `tokio::spawn`
//!   task that wakes every [`WATCHDOG_CHECK_INTERVAL_SECS`] and checks
//!   `now - last_tick`. If it exceeds the threshold, emits an ERROR log +
//!   audit event `engine_wedge_detected`.
//!
//! **Alert-only. No auto-restart.** Per Vincent's refus root sur
//! auto-respawn (RT#003), the watchdog surfaces a loud signal — the
//! operator decides remediation. Automatic recovery would break the
//! bounded-authority principle for restart operations.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::async_db::AsyncDatabase;
use crate::timestamp;

/// Default wedge-detection threshold (seconds). If the engine tick has not
/// fired in this window, the watchdog considers the loop wedged.
///
/// 300s = 5 minutes: engine's own tick cadence is 1s and the periodic DB scan
/// happens every 60 ticks (60s), so 5 minutes = 5× the widest normal-operation
/// gap. Overridable via `MIKA_ENGINE_WEDGE_THRESHOLD_SECS`.
pub const DEFAULT_WEDGE_THRESHOLD_SECS: i64 = 300;

/// Env var for [`DEFAULT_WEDGE_THRESHOLD_SECS`] override.
pub const WEDGE_THRESHOLD_ENV: &str = "MIKA_ENGINE_WEDGE_THRESHOLD_SECS";

/// Watchdog check cadence. Wakes every N seconds to compare `now - last_tick`
/// against the threshold. Independent of `DEFAULT_WEDGE_THRESHOLD_SECS`.
const WATCHDOG_CHECK_INTERVAL_SECS: u64 = 60;

/// Throttle for the `engine.tick.alive` INFO log. The engine ticks every
/// second — logging every tick would spam. Log at most once per interval.
const HEARTBEAT_LOG_INTERVAL_SECS: i64 = 60;

/// Structured log target for the liveness signal, so operators can grep or
/// route it distinctly (`grep 'target=mika_agent::task_engine::liveness'`).
const LIVENESS_TARGET: &str = "mika_agent::task_engine::liveness";

/// Parse the wedge threshold from an optional env string. Pure fn — extracted
/// so it's trivially unit-testable without mutating process env.
///
/// Returns [`DEFAULT_WEDGE_THRESHOLD_SECS`] when the input is `None`, empty,
/// unparseable, negative, or zero. Emits no side effect (the caller decides
/// whether to log a warn on fallback).
pub fn parse_wedge_threshold(raw: Option<&str>) -> i64 {
    match raw {
        Some(v) if !v.trim().is_empty() => match v.trim().parse::<i64>() {
            Ok(n) if n > 0 => n,
            _ => DEFAULT_WEDGE_THRESHOLD_SECS,
        },
        _ => DEFAULT_WEDGE_THRESHOLD_SECS,
    }
}

/// Read the wedge threshold from `MIKA_ENGINE_WEDGE_THRESHOLD_SECS`, falling
/// back to [`DEFAULT_WEDGE_THRESHOLD_SECS`] on any parse failure.
pub fn wedge_threshold_secs() -> i64 {
    parse_wedge_threshold(std::env::var(WEDGE_THRESHOLD_ENV).ok().as_deref())
}

/// Engine tick-loop heartbeat: an `Arc<AtomicI64>` holding the unix-secs
/// timestamp of the last tick. Cheap to clone (Arc), lock-free updates.
///
/// The engine's `spawn_tick_loop` calls `heartbeat.tick()` at the top of every
/// tick. The watchdog reads `heartbeat.last_tick_at()` on its own cadence.
#[derive(Debug, Clone)]
pub struct EngineHeartbeat {
    last_tick_at: Arc<AtomicI64>,
    last_log_at: Arc<AtomicI64>,
}

impl EngineHeartbeat {
    /// Construct a heartbeat with the current wall-clock time as the initial
    /// tick timestamp. Prevents the watchdog from firing spuriously in the
    /// first `THRESHOLD_SECS` after startup before the first real tick lands.
    pub fn new() -> Self {
        let now = now_unix_secs();
        Self {
            last_tick_at: Arc::new(AtomicI64::new(now)),
            last_log_at: Arc::new(AtomicI64::new(0)),
        }
    }

    /// Update the last-tick timestamp to now. Called at the top of every
    /// engine tick iteration. Emits a throttled INFO log
    /// (`engine.tick.alive`) at most every [`HEARTBEAT_LOG_INTERVAL_SECS`] —
    /// used by operators to confirm the engine is live without grepping
    /// dispatch chatter.
    pub fn tick(&self) {
        let now = now_unix_secs();
        self.last_tick_at.store(now, Ordering::Release);
        let last_log = self.last_log_at.load(Ordering::Acquire);
        if now - last_log >= HEARTBEAT_LOG_INTERVAL_SECS {
            self.last_log_at.store(now, Ordering::Release);
            info!(
                target: LIVENESS_TARGET,
                event = "engine.tick.alive",
                last_tick_at = now,
                "engine tick heartbeat"
            );
        }
    }

    /// Read the last-tick timestamp. Used by the watchdog to compute
    /// elapsed-since-last-tick.
    pub fn last_tick_at(&self) -> i64 {
        self.last_tick_at.load(Ordering::Acquire)
    }
}

impl Default for EngineHeartbeat {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn the wedge watchdog. Runs on its own `tokio::spawn` — decoupled from
/// the engine tick loop so a wedge in either does NOT wedge the other.
///
/// Alert-only. On threshold breach:
/// - Emit ERROR log `engine.wedge.detected` (target [`LIVENESS_TARGET`]).
/// - Write an audit event with `tool_name='engine_wedge_watchdog'` for SQL
///   query surfaces (`mika audit list`, dashboard).
/// - Continue running so a still-wedged loop keeps re-alerting (throttled to
///   the check cadence, not per-tick).
///
/// Does NOT trigger any restart action. Per Vincent RT#003 (refus root sur
/// auto-respawn): auto-recovery breaks the bounded-authority principle for
/// process-restart operations. Operator decides remediation.
///
/// Returns the `JoinHandle` so the caller can `abort()` on shutdown.
pub fn spawn_engine_wedge_watchdog(
    heartbeat: EngineHeartbeat,
    threshold_secs: i64,
    db: Option<AsyncDatabase>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        info!(
            target: LIVENESS_TARGET,
            event = "engine.watchdog.spawn",
            threshold_secs,
            check_interval_secs = WATCHDOG_CHECK_INTERVAL_SECS,
            "engine wedge watchdog started (alert-only, RT#003)"
        );

        let mut interval = tokio::time::interval(Duration::from_secs(WATCHDOG_CHECK_INTERVAL_SECS));
        // First tick fires immediately — skip it so we don't false-fire before
        // the engine has had any time to tick at all.
        interval.tick().await;

        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = cancel.cancelled() => {
                    info!(
                        target: LIVENESS_TARGET,
                        event = "engine.watchdog.cancelled",
                        "engine wedge watchdog stopped (graceful shutdown)"
                    );
                    return;
                }
            }

            let now = now_unix_secs();
            let last = heartbeat.last_tick_at();
            let elapsed = now - last;

            if elapsed >= threshold_secs {
                error!(
                    target: LIVENESS_TARGET,
                    event = "engine.wedge.detected",
                    wedge_duration_secs = elapsed,
                    threshold_secs,
                    last_tick_at = last,
                    now = now,
                    "task engine tick loop appears wedged — no tick in {}s (threshold {}s). \
                     Alert-only per RT#003. Operator restart required.",
                    elapsed,
                    threshold_secs
                );

                if let Some(ref db) = db {
                    // Fire-and-forget audit event. Uses agent_id="system" —
                    // the wedge concerns the process-wide engine loop, not
                    // any single agent's session.
                    let session_id = format!("system-engine-wedge-{now}");
                    let details = format!(
                        "wedge_duration_secs={} threshold_secs={} last_tick_at={}",
                        elapsed, threshold_secs, last
                    );
                    let db = db.clone();
                    tokio::spawn(async move {
                        if let Err(e) = db
                            .log_audit_event(
                                &session_id,
                                "engine_wedge_watchdog",
                                "engine_wedge_detected",
                                None,
                                None,
                                Some(&details),
                                None,
                            )
                            .await
                        {
                            // Only warn — audit failure must not amplify the
                            // wedge situation. The ERROR log above is the
                            // primary signal.
                            tracing::warn!(
                                target: LIVENESS_TARGET,
                                error = %e,
                                "failed to record engine_wedge_watchdog audit event"
                            );
                        }
                    });
                }
            }
        }
    })
}

/// Current wall-clock time as unix seconds. Uses `SystemTime` (not
/// `Instant`) because the watchdog reasons about "did anything happen
/// externally between two points in real time" — monotonic time is wrong
/// here (freezes during suspend, doesn't reflect real elapsed time).
fn now_unix_secs() -> i64 {
    // Delegate through the existing timestamp helper for parse consistency.
    // `timestamp::now()` returns an ISO 8601 string; parse it back to seconds
    // via chrono to avoid inventing a new time source.
    let s = timestamp::now();
    chrono::DateTime::parse_from_rfc3339(&s)
        .map(|dt| dt.timestamp())
        .unwrap_or_else(|_| {
            // If somehow ISO parse fails (should not happen — timestamp::now()
            // is bulletproof RFC3339), fall back to raw SystemTime.
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_threshold_default_when_absent() {
        assert_eq!(parse_wedge_threshold(None), DEFAULT_WEDGE_THRESHOLD_SECS);
    }

    #[test]
    fn parse_threshold_default_when_empty() {
        assert_eq!(
            parse_wedge_threshold(Some("")),
            DEFAULT_WEDGE_THRESHOLD_SECS
        );
        assert_eq!(
            parse_wedge_threshold(Some("   ")),
            DEFAULT_WEDGE_THRESHOLD_SECS
        );
    }

    #[test]
    fn parse_threshold_valid_positive() {
        assert_eq!(parse_wedge_threshold(Some("600")), 600);
        assert_eq!(parse_wedge_threshold(Some(" 42 ")), 42);
    }

    #[test]
    fn parse_threshold_zero_falls_back() {
        // Zero is meaningless (would fire on every check) — reject.
        assert_eq!(
            parse_wedge_threshold(Some("0")),
            DEFAULT_WEDGE_THRESHOLD_SECS
        );
    }

    #[test]
    fn parse_threshold_negative_falls_back() {
        assert_eq!(
            parse_wedge_threshold(Some("-5")),
            DEFAULT_WEDGE_THRESHOLD_SECS
        );
    }

    #[test]
    fn parse_threshold_garbage_falls_back() {
        assert_eq!(
            parse_wedge_threshold(Some("not-a-number")),
            DEFAULT_WEDGE_THRESHOLD_SECS
        );
    }

    #[test]
    fn heartbeat_starts_at_now() {
        let hb = EngineHeartbeat::new();
        let now = now_unix_secs();
        // Constructor sets last_tick_at to now — allow 2s slack for slow CI.
        assert!(hb.last_tick_at() >= now - 2);
        assert!(hb.last_tick_at() <= now + 2);
    }

    #[test]
    fn heartbeat_tick_updates_timestamp() {
        let hb = EngineHeartbeat::new();
        let initial = hb.last_tick_at();
        // Force a distinct timestamp via storing an older value.
        hb.last_tick_at.store(initial - 100, Ordering::Release);
        assert_eq!(hb.last_tick_at(), initial - 100);
        hb.tick();
        // After tick(), timestamp should be ~= now (>= initial - some jitter).
        assert!(hb.last_tick_at() > initial - 100);
    }

    #[test]
    fn heartbeat_clone_shares_state() {
        let hb1 = EngineHeartbeat::new();
        let hb2 = hb1.clone();
        // Force distinct known state via hb1.
        hb1.last_tick_at.store(1000, Ordering::Release);
        // hb2 (clone) sees it — they share the Arc<AtomicI64>.
        assert_eq!(hb2.last_tick_at(), 1000);
    }
}

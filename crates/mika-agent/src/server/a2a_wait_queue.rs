//! Bounded wait line for the two `/a2a/{agent}` lock gates (mika#2163).
//!
//! # Why this exists next to `webhook_queue_v2` instead of inside it
//!
//! `POST /message` received a bounded queue with backpressure in mika#1870.
//! `/a2a/{agent}` — the path of every `mika ask`, and therefore of every pilot
//! `canUseTool` callback — did not: it took the same per-agent mutex with
//! `try_lock_owned()` and answered `-32603 "Agent is busy"` on collision. A
//! refusal, not a wait, on a code whose defined meaning is "the server failed".
//!
//! mika#2163 AC1 asked to reuse the mika#1870 mechanism. Reading the code shows
//! that mechanism does not transpose: `WebhookQueue`'s producer has no return
//! channel (`enqueue` answers `202` and a drain worker runs the turn later),
//! while `message/send` is synchronous and its caller holds the connection open
//! waiting for the completed `Task`; its coalescing is inert here (two `mika ask`
//! calls are two distinct requests, never mergeable); and its saturation policy
//! is `drop_oldest`, which cannot be applied to a request someone is waiting on.
//! Generalising it would mean adding a per-entry `oneshot` return, a second
//! saturation policy and a second drain worker — putting the autonomous loop's
//! own dispatch path into the blast radius of this fix.
//!
//! **Decision R-A, ruled by mika-prime on 2026-09-05 and not reopenable by the
//! implementer or the review: take the mika#1870 *form*, not its code.** What is
//! taken literally: the three-tier config shape (absent → default; invalid →
//! default + `warn!`; sentinel → disabled), the kill-switch contract (disabled
//! path returns today's behaviour verbatim), and the throttled per-action audit
//! shape. What is *not* introduced is a second queue structure: the wait itself
//! is the one `tokio::sync::Mutex` already provides — documented FIFO-fair, so
//! the bound below bounds a line rather than a scramble — and the `Semaphore`
//! only makes the depth of that line explicit and refusable.
//!
//! The duplication of *form* with `webhook_queue_v2` is deliberate and load
//! bearing: the control contract differs (`POST /message` is fire-and-forget,
//! `/a2a` is synchronous). A future reader who unifies the two without reading
//! this paragraph will reintroduce the shape mika#2163 rejected.
//!
//! # The permit is released at lock acquisition, not at turn end
//!
//! The bound is on callers *waiting*, never on turns in flight. Holding the
//! permit for the duration of the turn would make one wait and one execution
//! count against the same number, and the configured depth would stop meaning
//! anything an operator can reason about.

use std::sync::Arc;
use std::time::{Duration, Instant};

use mika_a2a::jsonrpc::{AGENT_BUSY, INTERNAL_ERROR, JsonRpcError};
use mika_common::config::Settings;
use tokio::sync::{Mutex, OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};
use tracing::warn;

use crate::server::state::{AgentState, AppState};

/// Throttle window for the two A2A queue audit actions. Same 1/sec/action/agent
/// shape as `WEBHOOK_QUEUE_AUDIT_INTERVAL` — a burst of refusals must not itself
/// flood the audit table.
pub const A2A_QUEUE_AUDIT_INTERVAL: Duration = Duration::from_secs(1);

/// Why a caller was refused. Carried in `error.data.reason` because "the line was
/// full" and "I waited my turn and it never came" call for different responses
/// from the caller, and a single opaque refusal hides which one happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    /// All `a2a_queue_max_depth` places were taken. Refused without waiting.
    QueueFull,
    /// A place was held, the lock did not come within `a2a_queue_wait_timeout_ms`.
    WaitTimeout,
}

impl RejectReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::QueueFull => "queue_full",
            Self::WaitTimeout => "wait_timeout",
        }
    }
}

/// A caller's place in the wait line.
///
/// `Disabled` is not "no place available" — it is the kill-switch path, where no
/// line exists at all and the gate behaves exactly as it did before mika#2163.
#[derive(Debug)]
pub enum WaitSlot {
    /// One of the `max_depth` places. Dropped at lock acquisition.
    Queued(OwnedSemaphorePermit),
    /// `a2a_queue_enabled = false`.
    Disabled,
}

/// The pre-mika#2163 refusal, reproduced exactly — error code included.
///
/// AC5. A rollback that also changed the error code would not be a rollback:
/// a caller keying on `-32603` must keep seeing `-32603` when the switch is off.
pub fn legacy_busy_error() -> JsonRpcError {
    JsonRpcError::with_message(INTERNAL_ERROR, "Agent is busy")
}

/// The contention refusal (AC2, AC6).
///
/// `retry_after_ms` is the **configured** wait bound, not a prediction of how
/// long the turn in flight will run — the server does not know that, and
/// announcing a number it cannot honour would be worse than announcing none.
pub fn busy_error(reason: RejectReason, settings: &Settings) -> JsonRpcError {
    let mut err = JsonRpcError::with_message(AGENT_BUSY, "Agent is busy");
    err.data = Some(serde_json::json!({
        "reason": reason.as_str(),
        "retry_after_ms": settings.effective_a2a_queue_wait_timeout_ms(),
        "queue_depth": settings.effective_a2a_queue_max_depth(),
    }));
    err
}

/// Take a place in the wait line, or be refused because it is full.
///
/// Synchronous and non-blocking on purpose: the `message/stream` gate must take
/// its place **before** it spawns, or the spawn is unbounded and the
/// backpressure is decorative.
pub fn try_take_slot(
    slots: &Arc<Semaphore>,
    settings: &Settings,
) -> Result<WaitSlot, JsonRpcError> {
    if !settings.effective_a2a_queue_enabled() {
        return Ok(WaitSlot::Disabled);
    }
    match Arc::clone(slots).try_acquire_owned() {
        Ok(permit) => Ok(WaitSlot::Queued(permit)),
        Err(_) => Err(busy_error(RejectReason::QueueFull, settings)),
    }
}

/// Outcome of a successful acquisition: the guard, and how long the wait was.
#[derive(Debug)]
pub struct Acquired {
    pub guard: OwnedMutexGuard<()>,
    pub waited_ms: u64,
}

/// Wait for the agent lock, holding the place taken by [`try_take_slot`].
///
/// Cancel-safe by construction, and that property is load bearing on the
/// streaming gate: dropping this future drops the `lock_owned()` future (the
/// caller loses its place in the mutex's FIFO, which is what we want) and drops
/// the permit (freeing the line for the next caller). `message/stream` races it
/// against `tx.closed()` for exactly this reason — see `a2a.rs`.
pub async fn wait_for_agent_lock(
    agent_lock: Arc<Mutex<()>>,
    slot: WaitSlot,
    settings: &Settings,
) -> Result<Acquired, JsonRpcError> {
    let permit = match slot {
        // Kill-switch path: today's code, verbatim.
        WaitSlot::Disabled => {
            return match agent_lock.try_lock_owned() {
                Ok(guard) => Ok(Acquired {
                    guard,
                    waited_ms: 0,
                }),
                Err(_) => Err(legacy_busy_error()),
            };
        }
        WaitSlot::Queued(permit) => permit,
    };

    let timeout_ms = settings.effective_a2a_queue_wait_timeout_ms();

    // `0` means "do not wait" (the mirror of the mika#1870 block timeout). Handled
    // explicitly rather than by handing `Duration::ZERO` to `tokio::time::timeout`,
    // whose poll ordering would decide the outcome for us.
    if timeout_ms == 0 {
        return match agent_lock.try_lock_owned() {
            Ok(guard) => {
                drop(permit);
                Ok(Acquired {
                    guard,
                    waited_ms: 0,
                })
            }
            Err(_) => Err(busy_error(RejectReason::WaitTimeout, settings)),
        };
    }

    let started = Instant::now();
    match tokio::time::timeout(Duration::from_millis(timeout_ms), agent_lock.lock_owned()).await {
        Ok(guard) => {
            // Released here, not at turn end — see the module doc.
            drop(permit);
            Ok(Acquired {
                guard,
                waited_ms: started.elapsed().as_millis() as u64,
            })
        }
        Err(_) => Err(busy_error(RejectReason::WaitTimeout, settings)),
    }
}

/// Emit one throttled A2A-queue audit event (AC8).
///
/// Same shape and same throttle window as `handlers::emit_webhook_queue_audit`: a
/// contention that nobody can see afterwards is a contention that gets diagnosed
/// twice. One difference, and it is deliberate — see `throttle_extra`.
pub async fn emit_a2a_queue_audit(
    state: &AppState,
    agent_state: &Arc<AgentState>,
    agent_label: &str,
    action: &str,
    throttle_extra: Option<&str>,
    after: &str,
    reasoning: &str,
) {
    // `throttle_extra` exists because throttling on the action alone would let one
    // reason bury the other. Both refusals share the action `a2a_queue_reject`, so
    // a saturated agent emitting `queue_full` every few milliseconds would keep
    // resetting the window that a single `wait_timeout` needed to get through —
    // and the audit trail would show only "turned away at the door" for an
    // incident whose interesting half is "waited its turn and it never came".
    // That distinction is the whole point of AC6; a throttle must not erase it.
    let throttle_key = match throttle_extra {
        Some(extra) => format!("{agent_label}:{action}:{extra}"),
        None => format!("{agent_label}:{action}"),
    };
    if !crate::server::handlers::should_emit_rate_limit_audit(
        &state.a2a_queue_audit_last,
        &throttle_key,
        Instant::now(),
        A2A_QUEUE_AUDIT_INTERVAL,
    ) {
        return;
    }
    let target_key = format!("agent:{agent_label}");
    if let Err(e) = agent_state
        .db
        .log_audit_event(
            "system",
            action,
            &target_key,
            None,
            Some(after),
            Some(reasoning),
            None,
        )
        .await
    {
        warn!(error = %e, action, "failed to log a2a_queue audit event");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mika_common::config::Settings;

    fn settings_with(
        depth: Option<usize>,
        timeout_ms: Option<u64>,
        enabled: Option<bool>,
    ) -> Settings {
        let mut s = Settings::test_defaults();
        s.a2a_queue_max_depth = depth;
        s.a2a_queue_wait_timeout_ms = timeout_ms;
        s.a2a_queue_enabled = enabled;
        s
    }

    #[tokio::test]
    async fn free_lock_is_acquired_without_waiting() {
        let settings = settings_with(None, None, None);
        let lock = Arc::new(Mutex::new(()));
        let slots = Arc::new(Semaphore::new(settings.effective_a2a_queue_max_depth()));

        let slot = try_take_slot(&slots, &settings).unwrap();
        let acquired = wait_for_agent_lock(Arc::clone(&lock), slot, &settings)
            .await
            .expect("free lock");
        assert_eq!(acquired.waited_ms, 0);
        // Permit released at acquisition, not at turn end.
        assert_eq!(slots.available_permits(), 8);
    }

    /// T3 — a full line refuses without waiting, and says so.
    #[tokio::test]
    async fn full_line_refuses_with_queue_full() {
        let settings = settings_with(Some(1), Some(50), None);
        let slots = Arc::new(Semaphore::new(settings.effective_a2a_queue_max_depth()));

        let _first = try_take_slot(&slots, &settings).unwrap();
        let err = try_take_slot(&slots, &settings).expect_err("line is full");

        assert_eq!(err.code, AGENT_BUSY);
        assert_eq!(err.message, "Agent is busy");
        let data = err.data.unwrap();
        assert_eq!(data["reason"], "queue_full");
        assert_eq!(data["retry_after_ms"], 50);
        assert_eq!(data["queue_depth"], 1);
    }

    /// T4 — a held lock that never frees produces `wait_timeout`, not
    /// `queue_full`: the caller had a place, it just never came up.
    #[tokio::test]
    async fn held_lock_times_out_with_wait_timeout() {
        let settings = settings_with(None, Some(30), None);
        let lock = Arc::new(Mutex::new(()));
        let slots = Arc::new(Semaphore::new(settings.effective_a2a_queue_max_depth()));

        let _held = Arc::clone(&lock).lock_owned().await;
        let slot = try_take_slot(&slots, &settings).unwrap();
        let err = wait_for_agent_lock(Arc::clone(&lock), slot, &settings)
            .await
            .expect_err("lock never frees");

        assert_eq!(err.code, AGENT_BUSY);
        assert_eq!(err.data.unwrap()["reason"], "wait_timeout");
        // The place is handed back even on refusal.
        assert_eq!(slots.available_permits(), 8);
    }

    /// T2 — the kill-switch path is today's behaviour, error code included.
    #[tokio::test]
    async fn disabled_returns_the_legacy_internal_error_verbatim() {
        let settings = settings_with(None, None, Some(false));
        let lock = Arc::new(Mutex::new(()));
        let slots = Arc::new(Semaphore::new(8));

        let _held = Arc::clone(&lock).lock_owned().await;
        let slot = try_take_slot(&slots, &settings).unwrap();
        assert!(matches!(slot, WaitSlot::Disabled));
        let err = wait_for_agent_lock(Arc::clone(&lock), slot, &settings)
            .await
            .expect_err("busy");

        assert_eq!(err.code, INTERNAL_ERROR);
        assert_eq!(err.code, -32603);
        assert_eq!(err.message, "Agent is busy");
        assert!(err.data.is_none());
        // No place was ever taken: the line does not exist on this path.
        assert_eq!(slots.available_permits(), 8);
    }

    /// `0` on the delay is honoured as "do not wait" — and, unlike the
    /// kill-switch, still reports contention rather than an internal error.
    #[tokio::test]
    async fn zero_timeout_refuses_immediately_with_the_contention_code() {
        let settings = settings_with(None, Some(0), None);
        let lock = Arc::new(Mutex::new(()));
        let slots = Arc::new(Semaphore::new(8));

        let _held = Arc::clone(&lock).lock_owned().await;
        let slot = try_take_slot(&slots, &settings).unwrap();
        let err = wait_for_agent_lock(Arc::clone(&lock), slot, &settings)
            .await
            .expect_err("busy");

        assert_eq!(err.code, AGENT_BUSY);
        assert_eq!(err.data.unwrap()["reason"], "wait_timeout");
    }

    /// Dropping the wait future hands the place back — the property the
    /// streaming gate's `select!` against `tx.closed()` stands on (AC7).
    #[tokio::test]
    async fn abandoning_the_wait_returns_the_permit() {
        let settings = settings_with(Some(1), Some(60_000), None);
        let lock = Arc::new(Mutex::new(()));
        let slots = Arc::new(Semaphore::new(1));

        let _held = Arc::clone(&lock).lock_owned().await;
        let slot = try_take_slot(&slots, &settings).unwrap();
        assert_eq!(slots.available_permits(), 0);

        {
            let waiting = wait_for_agent_lock(Arc::clone(&lock), slot, &settings);
            tokio::select! {
                _ = waiting => panic!("lock is held; the wait cannot complete"),
                _ = tokio::time::sleep(Duration::from_millis(20)) => {}
            }
        }

        assert_eq!(slots.available_permits(), 1);
    }
}

//! Per-target-agent circuit breaker for webhook delivery (mika#1710).
//!
//! ## Why this exists
//!
//! The gateway → mika-spirit `/message` path can enter a self-amplifying HTTP 429
//! flood. mika-spirit's "rate limiter" is a per-agent concurrency-1 lock: while an
//! agent is busy on one turn (up to ~420s: 5-min deadline + 120s transport), every
//! other inbound `/message` gets an instant 429 ("agent busy"). Each webhook is an
//! independent `tokio::spawn`ed retry chain with **no shared per-target state**, so
//! N concurrent events each hammer the same busy agent, blind to the fact that the
//! other N-1 are also getting 429s. Combined with DLQ + GitHub redelivery re-injection,
//! the aggregate 429 rate stays above the drain rate and the loop cannot self-heal
//! without a process restart (incident 2026-07-01: 23,543 gateway 429s / 53,810 server
//! rejections / 1h11m audit silence).
//!
//! ## What this does
//!
//! Shared, concurrency-safe per-target health state keyed by target agent name.
//! **One breaker, one counter, two signals** driven off the same state:
//!
//! - **Soft trip (AC1):** on the [`CB_SOFT_THRESHOLD`]-th consecutive 429 for a target,
//!   open the circuit for [`CB_SOFT_OPEN`]. While open, new deliveries short-circuit
//!   straight to the DLQ (no HTTP attempt).
//! - **Adaptive open-window escalation (F3):** each failed half-open probe doubles the
//!   open window up to [`CB_MAX_OPEN`], which deliberately **exceeds the ~420s worst-case
//!   agent lock hold** so probes stop uselessly re-failing against an in-flight turn.
//!   The breaker is a *backpressure valve*, not a precise recovery detector.
//! - **Hard pause / self-heal (AC5, F1):** a rolling-window count of ≥[`CB_HARD_THRESHOLD`]
//!   429 observations within [`CB_HARD_WINDOW`] holds the circuit open for at least
//!   [`CB_HARD_OPEN`] and signals a distinct `gateway_target_paused` WARN. This is
//!   defense-in-depth and expected to be rare — the soft trip + escalation shed the
//!   flood first.
//!
//! All thresholds/durations are named `const`s (code-edit-tunable), consistent with
//! `RETRY_DELAYS` and the DLQ constants.
//!
//! ## Testing note
//!
//! The `*_at` methods take an explicit `now: Instant` so the state machine is
//! deterministically unit-testable without a clock trait. The convenience wrappers
//! ([`TargetCircuitBreaker::check_delivery`], [`TargetCircuitBreaker::record_429`])
//! pass `Instant::now()` for production callers.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use dashmap::DashMap;

/// Consecutive 429s that trip the soft breaker (AC1: N=3).
pub(crate) const CB_SOFT_THRESHOLD: u32 = 3;
/// Initial open-window duration after a soft trip (AC1: M=30s).
pub(crate) const CB_SOFT_OPEN: Duration = Duration::from_secs(30);
/// Cap on the escalating open window (F3). Deliberately > the ~420s worst-case
/// per-agent lock hold (5-min deadline + 120s transport) so a widened probe
/// interval eventually outlasts a busy turn and lands on a free lock.
pub(crate) const CB_MAX_OPEN: Duration = Duration::from_secs(480);
/// Rolling-window 429 count that triggers the hard pause (AC5: N=100).
pub(crate) const CB_HARD_THRESHOLD: usize = 100;
/// Rolling window over which hard-threshold 429s are counted (F1/D2).
pub(crate) const CB_HARD_WINDOW: Duration = Duration::from_secs(300);
/// Minimum hard-pause open duration (AC5: M=60s floor).
pub(crate) const CB_HARD_OPEN: Duration = Duration::from_secs(60);

/// Maximum concurrently-spawned delivery tasks per gateway (AC4, R4). Overflow
/// sheds durably to the DLQ instead of spawning an unbounded task. Not part of
/// the circuit-breaker state, but co-located here as a delivery-plumbing tunable.
pub(crate) const MAX_INFLIGHT_DELIVERIES: usize = 500;

/// Whether a delivery to a target should proceed or short-circuit to the DLQ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeliveryDecision {
    /// Circuit closed (or half-open probe) — attempt the HTTP delivery.
    Allow,
    /// Circuit open — skip the HTTP attempt and shed to the DLQ.
    ShortCircuit,
}

/// Result of recording a 429 observation for a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Record429Outcome {
    /// Recorded; the rolling-window hard threshold was **not** crossed.
    Recorded,
    /// The rolling-window hard threshold was crossed on this observation —
    /// caller should emit the `gateway_target_paused` WARN.
    HardPaused,
}

/// Per-target health state. Not `Clone` — always owned by the `DashMap` entry.
#[derive(Debug, Default)]
struct TargetHealth {
    /// Consecutive 429s since the last success (drives the soft trip).
    consecutive_429: u32,
    /// Timestamps of recent 429 observations within [`CB_HARD_WINDOW`] (drives the
    /// rolling-window hard pause).
    recent_429: VecDeque<Instant>,
    /// When the circuit re-closes; `None` means closed. `Some(t)` means open until `t`.
    open_until: Option<Instant>,
    /// Current escalating open-window duration. `Duration::ZERO` == never tripped.
    current_open: Duration,
    /// Set when a half-open probe has been allowed through; the next 429 for this
    /// target is that probe failing, and escalates the open window.
    probe_outstanding: bool,
}

/// Shared, concurrency-safe per-target circuit-breaker state. Cheap to `Arc`-share
/// across delivery tasks; all mutation is behind `DashMap` per-entry locks.
#[derive(Debug, Default)]
pub(crate) struct TargetCircuitBreaker {
    targets: DashMap<String, TargetHealth>,
}

impl TargetCircuitBreaker {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Decide whether a delivery to `target` may proceed, at instant `now`.
    ///
    /// When the open window has elapsed, allows **exactly one** half-open probe and
    /// re-arms the window so concurrent deliveries short-circuit until the probe
    /// resolves (via [`record_429`](Self::record_429) or
    /// [`record_success`](Self::record_success)).
    pub(crate) fn check_delivery_at(&self, target: &str, now: Instant) -> DeliveryDecision {
        let Some(mut entry) = self.targets.get_mut(target) else {
            return DeliveryDecision::Allow;
        };
        match entry.open_until {
            None => DeliveryDecision::Allow,
            Some(t) if now < t => DeliveryDecision::ShortCircuit,
            Some(_) => {
                // Half-open probe: allow one through, re-arm the window so the next
                // concurrent delivery short-circuits until this probe's result lands.
                entry.probe_outstanding = true;
                let window = entry.current_open.max(CB_SOFT_OPEN);
                entry.open_until = Some(now + window);
                DeliveryDecision::Allow
            }
        }
    }

    /// Production wrapper for [`check_delivery_at`](Self::check_delivery_at).
    pub(crate) fn check_delivery(&self, target: &str) -> DeliveryDecision {
        self.check_delivery_at(target, Instant::now())
    }

    /// Record a 429 observation for `target` at instant `now`. Returns whether the
    /// rolling-window hard threshold was crossed.
    pub(crate) fn record_429_at(&self, target: &str, now: Instant) -> Record429Outcome {
        let mut entry = self.targets.entry(target.to_string()).or_default();

        // Rolling-window maintenance: prune observations older than the window, then
        // record this one.
        if let Some(cutoff) = now.checked_sub(CB_HARD_WINDOW) {
            while entry.recent_429.front().is_some_and(|t| *t < cutoff) {
                entry.recent_429.pop_front();
            }
        }
        entry.recent_429.push_back(now);
        entry.consecutive_429 = entry.consecutive_429.saturating_add(1);

        let hard = entry.recent_429.len() >= CB_HARD_THRESHOLD;

        if entry.probe_outstanding {
            // The allowed half-open probe just failed → escalate the open window.
            entry.probe_outstanding = false;
            let base = entry.current_open.max(CB_SOFT_OPEN);
            entry.current_open = (base * 2).min(CB_MAX_OPEN);
            entry.open_until = Some(now + entry.current_open);
        } else if entry.open_until.is_some() {
            // Already open; a concurrent event's attempt 429'd (not a probe).
            // Record only — escalation happens on probe failure, not per observation.
        } else if entry.consecutive_429 >= CB_SOFT_THRESHOLD {
            // First soft trip.
            entry.current_open = CB_SOFT_OPEN;
            entry.open_until = Some(now + CB_SOFT_OPEN);
        }

        if hard {
            // Hold the circuit open for at least the AC5 floor, never shrinking an
            // already-escalated window.
            let hold = entry.current_open.max(CB_HARD_OPEN);
            entry.current_open = hold;
            entry.open_until = Some(now + hold);
            return Record429Outcome::HardPaused;
        }
        Record429Outcome::Recorded
    }

    /// Production wrapper for [`record_429_at`](Self::record_429_at).
    pub(crate) fn record_429(&self, target: &str) -> Record429Outcome {
        self.record_429_at(target, Instant::now())
    }

    /// Record a successful delivery to `target` — closes the circuit and resets all
    /// 429 state (F: reset-on-success).
    pub(crate) fn record_success(&self, target: &str) {
        if let Some(mut entry) = self.targets.get_mut(target) {
            entry.consecutive_429 = 0;
            entry.current_open = CB_SOFT_OPEN;
            entry.open_until = None;
            entry.probe_outstanding = false;
            entry.recent_429.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fresh_target_allows_delivery() {
        let cb = TargetCircuitBreaker::new();
        assert_eq!(cb.check_delivery("mika-dev"), DeliveryDecision::Allow);
    }

    #[test]
    fn test_circuit_breaker_soft_trip_opens_after_3() {
        let cb = TargetCircuitBreaker::new();
        let t0 = Instant::now();
        // Two 429s below threshold — still closed.
        assert_eq!(cb.record_429_at("mika-dev", t0), Record429Outcome::Recorded);
        assert_eq!(cb.record_429_at("mika-dev", t0), Record429Outcome::Recorded);
        assert_eq!(
            cb.check_delivery_at("mika-dev", t0),
            DeliveryDecision::Allow,
            "below threshold circuit must stay closed"
        );
        // Third 429 trips the soft breaker.
        assert_eq!(cb.record_429_at("mika-dev", t0), Record429Outcome::Recorded);
        assert_eq!(
            cb.check_delivery_at("mika-dev", t0 + Duration::from_secs(5)),
            DeliveryDecision::ShortCircuit,
            "4th delivery within the open window must short-circuit"
        );
    }

    #[test]
    fn test_circuit_breaker_resets_on_success() {
        let cb = TargetCircuitBreaker::new();
        let t0 = Instant::now();
        for _ in 0..CB_SOFT_THRESHOLD {
            cb.record_429_at("mika-dev", t0);
        }
        assert_eq!(
            cb.check_delivery_at("mika-dev", t0 + Duration::from_secs(1)),
            DeliveryDecision::ShortCircuit
        );
        cb.record_success("mika-dev");
        assert_eq!(
            cb.check_delivery_at("mika-dev", t0 + Duration::from_secs(1)),
            DeliveryDecision::Allow,
            "success must close the circuit and reset the counter"
        );
    }

    #[test]
    fn test_circuit_breaker_half_open_probe() {
        let cb = TargetCircuitBreaker::new();
        let t0 = Instant::now();
        for _ in 0..CB_SOFT_THRESHOLD {
            cb.record_429_at("mika-dev", t0);
        }
        // While open: short-circuit.
        assert_eq!(
            cb.check_delivery_at("mika-dev", t0 + Duration::from_secs(10)),
            DeliveryDecision::ShortCircuit
        );
        // Once the window elapses: exactly one probe allowed, then re-armed.
        let probe_at = t0 + CB_SOFT_OPEN;
        assert_eq!(
            cb.check_delivery_at("mika-dev", probe_at),
            DeliveryDecision::Allow,
            "elapsed window must allow one probe"
        );
        assert_eq!(
            cb.check_delivery_at("mika-dev", probe_at),
            DeliveryDecision::ShortCircuit,
            "second concurrent delivery during outstanding probe must short-circuit"
        );
        // Probe succeeds → circuit closes and resets.
        cb.record_success("mika-dev");
        assert_eq!(
            cb.check_delivery_at("mika-dev", probe_at),
            DeliveryDecision::Allow
        );
    }

    #[test]
    fn test_circuit_breaker_open_window_escalates_on_probe_failure() {
        let cb = TargetCircuitBreaker::new();
        let mut now = Instant::now();
        // Trip the soft breaker → 30s window.
        for _ in 0..CB_SOFT_THRESHOLD {
            cb.record_429_at("mika-dev", now);
        }
        // Expected escalation ladder on repeated probe failures.
        let ladder = [60u64, 120, 240, 480, 480];
        let mut window = CB_SOFT_OPEN;
        for expected in ladder {
            // Advance past the current window, take the probe, then fail it.
            now += window;
            assert_eq!(
                cb.check_delivery_at("mika-dev", now),
                DeliveryDecision::Allow,
                "probe must be allowed when the window elapses"
            );
            cb.record_429_at("mika-dev", now); // probe fails → escalate
            window = Duration::from_secs(expected);
            // The next probe is only allowed after the escalated window.
            assert_eq!(
                cb.check_delivery_at("mika-dev", now + window - Duration::from_secs(1)),
                DeliveryDecision::ShortCircuit,
                "escalated window must keep the circuit open for {expected}s"
            );
        }
        // Cap holds and deliberately exceeds the ~420s worst-case lock hold.
        assert_eq!(window, CB_MAX_OPEN);
        assert!(
            CB_MAX_OPEN.as_secs() > 420,
            "CB_MAX_OPEN must exceed the ~420s worst-case per-agent lock hold"
        );
    }

    #[test]
    fn test_escalation_resets_on_success_mid_ladder() {
        let cb = TargetCircuitBreaker::new();
        let mut now = Instant::now();
        for _ in 0..CB_SOFT_THRESHOLD {
            cb.record_429_at("mika-dev", now);
        }
        // Escalate once (30s → 60s).
        now += CB_SOFT_OPEN;
        cb.check_delivery_at("mika-dev", now);
        cb.record_429_at("mika-dev", now);
        // A success resets the base window back to CB_SOFT_OPEN.
        cb.record_success("mika-dev");
        for _ in 0..CB_SOFT_THRESHOLD {
            cb.record_429_at("mika-dev", now);
        }
        assert_eq!(
            cb.check_delivery_at("mika-dev", now + CB_SOFT_OPEN - Duration::from_secs(1)),
            DeliveryDecision::ShortCircuit
        );
        assert_eq!(
            cb.check_delivery_at("mika-dev", now + CB_SOFT_OPEN),
            DeliveryDecision::Allow,
            "post-success trip must use the base 30s window, not the escalated one"
        );
    }

    #[test]
    fn test_circuit_breaker_hard_pause_rolling_window() {
        let cb = TargetCircuitBreaker::new();
        let t0 = Instant::now();
        let mut outcomes = Vec::new();
        for _ in 0..CB_HARD_THRESHOLD {
            outcomes.push(cb.record_429_at("mika-dev", t0));
        }
        // The 100th observation within the window crosses the hard threshold.
        assert_eq!(
            *outcomes.last().unwrap(),
            Record429Outcome::HardPaused,
            "reaching {CB_HARD_THRESHOLD} 429s in-window must hard-pause"
        );
        // Hard pause holds the circuit open for at least the 60s floor.
        assert_eq!(
            cb.check_delivery_at("mika-dev", t0 + CB_HARD_OPEN - Duration::from_secs(1)),
            DeliveryDecision::ShortCircuit
        );
    }

    #[test]
    fn test_hard_pause_not_reached_when_older_429s_pruned() {
        let cb = TargetCircuitBreaker::new();
        let t0 = Instant::now();
        // 60 observations, then 60 more after the window elapses. The first batch is
        // pruned, so the in-window count never reaches 100.
        for _ in 0..60 {
            assert_ne!(
                cb.record_429_at("mika-dev", t0),
                Record429Outcome::HardPaused
            );
        }
        let later = t0 + CB_HARD_WINDOW + Duration::from_secs(1);
        for _ in 0..60 {
            assert_ne!(
                cb.record_429_at("mika-dev", later),
                Record429Outcome::HardPaused,
                "pruned older 429s must keep the rolling count below the hard threshold"
            );
        }
    }

    #[test]
    fn test_targets_are_independent() {
        let cb = TargetCircuitBreaker::new();
        let t0 = Instant::now();
        for _ in 0..CB_SOFT_THRESHOLD {
            cb.record_429_at("mika-dev", t0);
        }
        assert_eq!(
            cb.check_delivery_at("mika-dev", t0 + Duration::from_secs(1)),
            DeliveryDecision::ShortCircuit
        );
        assert_eq!(
            cb.check_delivery_at("mika-qa", t0 + Duration::from_secs(1)),
            DeliveryDecision::Allow,
            "one target's open circuit must not affect another"
        );
    }
}

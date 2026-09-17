//! One predicate for one question: *will a further attempt actually run?*
//! (mika#2362)
//!
//! # The divergence this closes
//!
//! Three sites per rail asked that question, and they were not the same
//! question. On `llm/openai.rs` before mika#2362:
//!
//! 1. the **deadline guard**, at the top of iteration `attempt`, compared the
//!    remaining deadline against a threshold chosen by the last error's class
//!    and `break`ed when the margin was short;
//! 2. the **outcome line**, right after the call, chose between `retrying` and
//!    `exhausted` from `attempt + 1 < max_attempts && e.is_retryable()` — and
//!    **never consulted the deadline at all**;
//! 3. the **post-loop error message** re-derived the same threshold a second
//!    time to choose between `retry chain aborted: deadline budget
//!    insufficient` and `max retries exceeded`.
//!
//! Site 2 could therefore announce a retry that site 1 was about to refuse.
//! That is not a hypothetical: on 2026-09-17 a mika-arch turn under a `300/600`
//! geometry logged `outcome="retrying"` for an attempt cut at exactly the
//! 300 000 ms plafond, and the turn errored **18 ms later** with no `attempt=1`
//! ever running. `600 − 300 = 300` is exactly the non-transport threshold
//! (`0.75 × cap + 0.25 × cap = 1.0 × cap`), so the margin was zero and the
//! guard refused what the outcome line had just promised.
//!
//! Site 3 carried the same divergence in a more candid form: its comment said
//! it duplicated the guard's threshold *"so the two branches agree"* — an
//! agreement obtained by copying, which is the definition of a predicate free
//! to drift. It had already drifted once and been measured: mika#2331 §3.3
//! found that mika#1744's transport threshold *"had never been ported to this
//! rail"* (ollama), silently, across two tickets.
//!
//! # What lives here
//!
//! [`RetryThresholds`] (the **only** place the non-transport threshold's sum is
//! written), [`RetryClass`] (the two error predicates the decision reads),
//! [`next_attempt_verdict`] (budget + retryability + deadline), and
//! [`deadline_verdict`] (the deadline half alone, for the post-loop site which
//! measures at a different instant and has no attempt index).
//!
//! Everything here is pure: no clock, no I/O. `remaining` is passed in, which
//! is what preserves the fact that the three sites measure it at three
//! different moments — a merged decision would have flattened that and cost the
//! `deadline_remaining_ms` field that tells one reading of an incident from
//! another.
//!
//! # What this deliberately does NOT change
//!
//! The control flow. The guard keeps its `break`, the loop keeps its
//! `continue`, `max_attempts` keeps its value and `worst_case_failure_secs`
//! keeps sizing the mika#2342 watchdog. This is an observability fix: the
//! outcome line stops describing a decision other than the one that governs it.

use std::time::Duration;

use super::budget::LlmTimeoutBudget;
use super::error::LlmError;

/// The two remaining-deadline thresholds a retry decision picks between.
///
/// Grouped into one value rather than passed as two bare `u64` for one reason
/// that is load-bearing: the non-transport threshold is a **sum**
/// (`typical_call_duration_secs() + retry_buffer_secs()`), and that sum is the
/// predicate that was being copied. Handing the rails two numbers would make
/// each of them compute the sum at its call site, which is the divergence this
/// module exists to close — and would leave
/// `tests::mika2362_no_rail_recomputes_the_retry_threshold_inline` with nothing
/// to enforce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryThresholds {
    transport_secs: u64,
    default_secs: u64,
}

impl RetryThresholds {
    /// Derive both thresholds from a rail's effective budget (mika#2189).
    ///
    /// `0.50 ×` the cap after a transport-class failure (mika#1744: DNS,
    /// refused connections and TLS handshakes resolve in seconds, not in a full
    /// cap), and `0.75 × + 0.25 ×` otherwise.
    ///
    /// **This is the only site in the crate where that sum is written.**
    pub fn from_budget(budget: &LlmTimeoutBudget) -> Self {
        Self {
            transport_secs: budget.transport_retry_min_remaining_secs(),
            default_secs: budget.typical_call_duration_secs() + budget.retry_buffer_secs(),
        }
    }

    /// The thresholds of the **default** per-call cap.
    ///
    /// For the Anthropic rail, which carries no [`LlmTimeoutBudget`]: it hands
    /// `reqwest` the literal `ANTHROPIC_HTTP_TIMEOUT_SECS` (120) instead of
    /// reading a plafond — a real inconsistency, named out of scope by
    /// mika#2189 and only made more legible here. Expressed as the default
    /// budget's thresholds rather than as
    /// `TYPICAL_CALL_DURATION_SECS + RETRY_BUFFER_SECS` so the fractions stay
    /// defined in exactly one module (`budget.rs`) and this rail cannot drift
    /// away from them; `claude::tests::mika2362_anthropic_cap_is_the_default_cap`
    /// pins the equality the substitution rests on, so the day that literal
    /// moves, it goes red instead of silently applying someone else's geometry.
    pub fn pinned_at_default_cap() -> Self {
        Self::from_budget(&LlmTimeoutBudget::default())
    }

    /// The threshold that applies to the last error's class.
    pub fn for_class(&self, last_was_transport: bool) -> u64 {
        if last_was_transport {
            self.transport_secs
        } else {
            self.default_secs
        }
    }

    /// The transport-class threshold, in seconds.
    pub fn transport_secs(&self) -> u64 {
        self.transport_secs
    }

    /// The non-transport threshold, in seconds.
    pub fn default_secs(&self) -> u64 {
        self.default_secs
    }
}

/// What the answer to *"will a further attempt run?"* is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryVerdict {
    /// Another attempt will run.
    Retry,
    /// No further attempt: the error is not retryable, or the attempt budget is
    /// spent.
    BudgetSpent,
    /// No further attempt: the remaining deadline cannot fit one.
    ///
    /// Carries both halves of the comparison so the refusal can be reported
    /// without re-deriving it — and so an operator reading the incident sees
    /// the margin that was actually measured, not the one the geometry implies.
    DeadlineInsufficient {
        /// Deadline margin observed at the decision, in milliseconds.
        remaining_ms: u64,
        /// Threshold it failed to clear, in seconds.
        threshold_secs: u64,
    },
}

impl RetryVerdict {
    /// Whether the deadline, rather than the budget, is what stops the chain.
    ///
    /// Read by the post-loop site of each rail to choose between
    /// "deadline budget insufficient" and "max retries exceeded".
    pub fn is_deadline_insufficient(&self) -> bool {
        matches!(self, RetryVerdict::DeadlineInsufficient { .. })
    }
}

/// The two error predicates a retry decision reads.
///
/// A trait rather than two `bool` parameters: `is_retryable` and `is_transport`
/// are two readings of one value, and passing them separately lets a caller
/// hand over a pair with no referent (`retryable = false, transport = true`).
///
/// It exists because the Anthropic rail's errors are `ClaudeApiError`, a type
/// foreign to [`LlmError`] which cannot share an implementation — the same
/// shape `error_class` already had to take: two *mappings*, one site of
/// *definition*.
pub trait RetryClass {
    /// Whether the failure is transient and the request should be retried.
    fn is_retryable(&self) -> bool;
    /// Whether the failure is a network-transport one (mika#1744), which
    /// resolves in seconds and therefore clears a smaller threshold.
    fn is_transport(&self) -> bool;
}

impl RetryClass for LlmError {
    fn is_retryable(&self) -> bool {
        LlmError::is_retryable(self)
    }
    fn is_transport(&self) -> bool {
        LlmError::is_transport(self)
    }
}

/// Will attempt `attempt + 1` run, given how attempt `attempt` ended?
///
/// The whole decision, in the order that makes each refusal report its true
/// cause:
///
/// 1. a non-retryable error ends the chain — [`RetryVerdict::BudgetSpent`];
/// 2. `attempt + 1 >= max_attempts` ends it too, same verdict;
/// 3. a remaining deadline under the class's threshold ends it —
///    [`RetryVerdict::DeadlineInsufficient`], which is the case mika#2362 was
///    filed for;
/// 4. otherwise [`RetryVerdict::Retry`].
///
/// `remaining = None` means the caller has no deadline, in which case step 3
/// cannot refuse anything — the pre-mika#2189 behaviour for callers with no
/// deadline visibility, unchanged.
///
/// `error = None` means there is no failure to classify. Step 1 is then skipped
/// and the non-transport threshold applies, which is the conservative reading:
/// the smaller transport threshold is a concession granted to a failure class
/// we have *identified*, never a default.
pub fn next_attempt_verdict(
    attempt: u32,
    max_attempts: u32,
    error: Option<&dyn RetryClass>,
    remaining: Option<Duration>,
    thresholds: &RetryThresholds,
) -> RetryVerdict {
    if let Some(e) = error
        && !e.is_retryable()
    {
        return RetryVerdict::BudgetSpent;
    }
    if attempt + 1 >= max_attempts {
        return RetryVerdict::BudgetSpent;
    }
    deadline_verdict(
        remaining,
        error.is_some_and(RetryClass::is_transport),
        thresholds,
    )
}

/// The deadline half of the question, alone.
///
/// Used by the deadline guard (which already knows the budget permits another
/// attempt — the loop bound guarantees it) and by the post-loop error-message
/// site (which has no attempt index, and measures `remaining` at a later
/// instant than the guard did, so its verdict may legitimately differ).
///
/// Returns [`RetryVerdict::Retry`] or [`RetryVerdict::DeadlineInsufficient`],
/// never [`RetryVerdict::BudgetSpent`] — it is not asked about the budget.
pub fn deadline_verdict(
    remaining: Option<Duration>,
    last_was_transport: bool,
    thresholds: &RetryThresholds,
) -> RetryVerdict {
    let Some(remaining) = remaining else {
        return RetryVerdict::Retry;
    };
    let threshold_secs = thresholds.for_class(last_was_transport);
    if remaining < Duration::from_secs(threshold_secs) {
        RetryVerdict::DeadlineInsufficient {
            remaining_ms: remaining.as_millis() as u64,
            threshold_secs,
        }
    } else {
        RetryVerdict::Retry
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::attempt_outcome;

    fn http(status: u16, retryable: bool) -> LlmError {
        LlmError::HttpError {
            status,
            message: "test".into(),
            retryable,
        }
    }

    fn transport() -> LlmError {
        LlmError::Transport("connection reset".into())
    }

    /// The incident's geometry, scaled down: the envelope is an exact multiple
    /// of the cap, so the non-transport threshold equals the cap exactly.
    fn incident_thresholds() -> RetryThresholds {
        let budget = LlmTimeoutBudget::new(20, 40).expect("20 < 40");
        RetryThresholds::from_budget(&budget)
    }

    fn fleet_thresholds() -> RetryThresholds {
        RetryThresholds::from_budget(&LlmTimeoutBudget::default())
    }

    // ── the sum lives here, and nowhere else ──────────────────────────────

    #[test]
    fn thresholds_are_the_budget_fractions() {
        let t = fleet_thresholds();
        assert_eq!(t.transport_secs(), 60, "0.50 × 120");
        assert_eq!(t.default_secs(), 120, "0.75 × 120 + 0.25 × 120");
    }

    /// The Anthropic substitution: this rail poses its own 120 s plafond, which
    /// is the default cap, so its thresholds are the default cap's.
    #[test]
    fn pinned_thresholds_equal_the_default_budget_thresholds() {
        assert_eq!(RetryThresholds::pinned_at_default_cap(), fleet_thresholds());
    }

    /// E4: `default_secs == cap` exactly whenever the cap is a multiple of 4 —
    /// which is what makes `E = k·P` the off-by-one family mika#2362 found.
    #[test]
    fn mika2362_the_non_transport_threshold_is_exactly_the_cap() {
        for cap in [20u64, 120, 240, 300] {
            let budget = LlmTimeoutBudget::new(cap, cap * 4).expect("valid");
            assert_eq!(
                RetryThresholds::from_budget(&budget).default_secs(),
                cap,
                "0.75 + 0.25 must be 1.0 at cap={cap}"
            );
        }
    }

    // ── the verdict itself, both controls in each probe ───────────────────

    /// The founding incident, as a unit: attempt 0 of 2 consumed the whole cap
    /// under an envelope of `2 × cap`, so the margin is the cap and the
    /// threshold is the cap — *not strictly greater*, so no second attempt.
    ///
    /// Positive control in the same test: with the fleet geometry the margin is
    /// `300 − 120 = 180 > 120` and the verdict is `Retry`. Without it, a
    /// predicate answering `DeadlineInsufficient` unconditionally would pass.
    #[test]
    fn mika2362_zero_margin_refuses_and_a_real_margin_retries() {
        let err = http(429, true);

        let refused = next_attempt_verdict(
            0,
            2,
            Some(&err),
            // 40 − 20 = 20, minus the ε any real call consumes.
            Some(Duration::from_millis(20_000 - 5)),
            &incident_thresholds(),
        );
        assert!(
            matches!(
                refused,
                RetryVerdict::DeadlineInsufficient {
                    threshold_secs: 20,
                    ..
                }
            ),
            "a zero-margin geometry must refuse, got {refused:?}"
        );
        assert_eq!(attempt_outcome::outcome_for(&refused), "exhausted");

        let allowed = next_attempt_verdict(
            0,
            2,
            Some(&err),
            Some(Duration::from_secs(180)),
            &fleet_thresholds(),
        );
        assert_eq!(allowed, RetryVerdict::Retry);
        assert_eq!(attempt_outcome::outcome_for(&allowed), "retrying");
    }

    /// E5: the transport class clears `0.50 × cap`, so the very same margin
    /// that refuses a 429 lets a reset through. This is the control that stops
    /// the fix from quietly tightening the chain the mika#2342 watchdog is
    /// sized on.
    #[test]
    fn mika2362_transport_keeps_its_smaller_threshold() {
        let err = transport();
        let verdict = next_attempt_verdict(
            0,
            2,
            Some(&err),
            Some(Duration::from_millis(20_000 - 5)),
            &incident_thresholds(),
        );
        assert_eq!(
            verdict,
            RetryVerdict::Retry,
            "0.50 × 20 = 10 s; a 20 s margin clears it"
        );
    }

    #[test]
    fn a_non_retryable_error_ends_the_chain_whatever_the_margin() {
        let err = http(400, false);
        assert_eq!(
            next_attempt_verdict(
                0,
                4,
                Some(&err),
                Some(Duration::from_secs(10_000)),
                &fleet_thresholds()
            ),
            RetryVerdict::BudgetSpent
        );
    }

    #[test]
    fn the_last_attempt_of_the_budget_ends_the_chain() {
        let err = http(500, true);
        assert_eq!(
            next_attempt_verdict(
                1,
                2,
                Some(&err),
                Some(Duration::from_secs(10_000)),
                &fleet_thresholds()
            ),
            RetryVerdict::BudgetSpent
        );
    }

    /// No deadline means no deadline refusal — the unchanged behaviour of
    /// callers with no envelope visibility.
    #[test]
    fn without_a_deadline_the_budget_alone_decides() {
        let err = http(500, true);
        assert_eq!(
            next_attempt_verdict(0, 2, Some(&err), None, &fleet_thresholds()),
            RetryVerdict::Retry
        );
        assert_eq!(
            deadline_verdict(None, false, &fleet_thresholds()),
            RetryVerdict::Retry
        );
    }

    /// An unclassified failure gets the *larger* threshold: the transport
    /// concession is granted to an identified class, never defaulted into.
    #[test]
    fn an_unknown_error_class_gets_the_conservative_threshold() {
        let verdict = next_attempt_verdict(
            0,
            2,
            None,
            Some(Duration::from_secs(90)),
            &fleet_thresholds(),
        );
        assert!(
            verdict.is_deadline_insufficient(),
            "90 s clears the transport threshold (60) but not the default (120): {verdict:?}"
        );
    }

    // ── D5: no rail may recompute the threshold inline ────────────────────

    /// A structural scan, because a behavioural test cannot see this class of
    /// regression.
    ///
    /// Re-introducing a local threshold would make **no** decision wrong on the
    /// day it lands — it would make the three answers *free to diverge again*,
    /// and every existing assertion would stay green throughout the drift.
    /// That is exactly what happened to mika#1744's transport threshold between
    /// its own ticket and mika#2331 §3.3, on a rail nobody was watching.
    ///
    /// The scan targets the **sum**, not the accessors: `budget.rs` defines
    /// them and its own tests assert them, so forbidding
    /// `typical_call_duration_secs()` alone would make the defining module
    /// violate its own guard. An accessor read on its own decides nothing; the
    /// sum is the predicate that was being copied.
    ///
    /// Allowlist: empty. A sixth site is halt-and-surface, not an entry here.
    #[test]
    fn mika2362_no_rail_recomputes_the_retry_threshold_inline() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let this_file = "retry_gate.rs";
        let mut offenders = Vec::new();

        let mut stack = vec![src.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("readable source tree") {
                let path = entry.expect("readable entry").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                if path.file_name().is_some_and(|n| n == this_file) {
                    continue;
                }
                let text = std::fs::read_to_string(&path).expect("readable source file");
                // Whitespace-insensitive: a reformat must not open the gate.
                let flat: String = text.chars().filter(|c| !c.is_whitespace()).collect();
                for needle in [
                    "typical_call_duration_secs()+retry_buffer_secs()",
                    "TYPICAL_CALL_DURATION_SECS+RETRY_BUFFER_SECS",
                ] {
                    if flat.contains(needle) {
                        offenders.push(format!("{} contains `{needle}`", path.display()));
                    }
                }
            }
        }

        assert!(
            offenders.is_empty(),
            "the deadline threshold must be selected by `retry_gate`, not recomputed inline \
             (mika#2362 D5). Offending sites:\n{}",
            offenders.join("\n")
        );
    }
}

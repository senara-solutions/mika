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
//! [`transport_concession_applies`] (the **only** place membership in the
//! mika#1744 concession is decided), [`next_attempt_verdict`] (budget +
//! retryability + deadline), and [`deadline_verdict`] (the deadline half alone,
//! for the post-loop site which measures at a different instant and has no
//! attempt index).
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

    /// The threshold that applies, given whether the mika#1744 concession holds.
    ///
    /// The parameter is the **verdict** of
    /// [`transport_concession_applies`], not an `is_transport` read on its own
    /// (mika#3126): on a cut at the plafond the error is transport and the
    /// concession does not hold, so the two answers differ.
    pub fn for_class(&self, transport_concession: bool) -> u64 {
        if transport_concession {
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

/// Does the last failure belong to the class the mika#1744 concession was
/// granted to?
///
/// The concession is the smaller `0.50 × cap` threshold, and mika#1744 granted
/// it with its reason written down: *"DNS, refused connections and TLS
/// handshakes resolve in seconds, not in a full cap"*. It is therefore an
/// exception for a failure class **identified as fast**, never a property of the
/// `Transport` variant as such.
///
/// An attempt **cut at its own plafond** has just refuted that membership for
/// the next one: it consumed the entire cap. It is still an
/// `LlmError::Transport` and still retryable — mika#2280 deliberately left the
/// variant, the class and the retryability intact (D2/AC6), because mika#2015
/// measured that making them terminal cost a whole pilot cycle on every
/// mid-body network hiccup. So `is_transport()` answers `true` on a guillotine,
/// and reading it alone granted the fast-retry threshold to the one failure we
/// *know* will be slow. Measured 2026-11-16: three turns cut in
/// `llm_call_cap_exhausted` under a `300/660` geometry, each retried with 290 to
/// 345 s of margin under a 300 s cap, the envelope abandoned at 660 s for
/// 4.82 USD with not one usable byte returned — and nothing left for the
/// mika#2289 error-verdict net to report why (mika#3126).
///
/// **This is a withdrawal of an exception, not a third threshold.** When it
/// answers `false` the fallback is the default threshold that already exists
/// (`1.0 × cap`, "one whole call plus a reserve"), so no new fraction is
/// derived and no geometry whose envelope carries two whole calls changes
/// behaviour.
///
/// `cap_exhausted` is the per-attempt measurement
/// [`super::budget::LlmTimeoutBudget::is_cap_exhaustion`] produces, carried
/// **beside** the error as mika#2280 requires. It dominates the error class:
/// the cut is a fact about the attempt that just ran, the class only a fact
/// about how it surfaced.
///
/// **The single site.** The three sites per rail that ask the retry question
/// used to read `is_some_and(LlmError::is_transport)` each at home. Adding
/// `&& !cap_exhausted` in nine places would be copying a predicate — the exact
/// drift this module exists to close, and one already paid for twice
/// (mika#2331 §3.3: mika#1744's threshold was never ported to the ollama rail,
/// silently, across two tickets).
pub fn transport_concession_applies(error: Option<&dyn RetryClass>, cap_exhausted: bool) -> bool {
    !cap_exhausted && error.is_some_and(RetryClass::is_transport)
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
///
/// `cap_exhausted` is the per-attempt measurement
/// [`super::budget::LlmTimeoutBudget::is_cap_exhaustion`] produces for the
/// attempt that just failed, carried beside the error as mika#2280 requires.
/// It is routed through [`transport_concession_applies`], which is the **only**
/// thing it does here: it never makes a retryable error terminal, never changes
/// the budget arithmetic of step 2, and only ever moves step 3's threshold from
/// the mika#1744 concession back to the default (mika#3126). A rail that cannot
/// measure it passes `false` — the honest answer for an unmeasured attempt,
/// which keeps the pre-mika#3126 behaviour.
pub fn next_attempt_verdict(
    attempt: u32,
    max_attempts: u32,
    error: Option<&dyn RetryClass>,
    cap_exhausted: bool,
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
        transport_concession_applies(error, cap_exhausted),
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
///
/// `transport_concession` must come from [`transport_concession_applies`] and
/// never from an `is_transport` read on its own: the two answers differ exactly
/// on the case mika#3126 was filed for, an attempt cut at its plafond, where the
/// error is transport-class and the concession does not hold. The parameter was
/// called `last_was_transport` until mika#3126 — a name that described the
/// measurement, and invited that very read.
pub fn deadline_verdict(
    remaining: Option<Duration>,
    transport_concession: bool,
    thresholds: &RetryThresholds,
) -> RetryVerdict {
    let Some(remaining) = remaining else {
        return RetryVerdict::Retry;
    };
    let threshold_secs = thresholds.for_class(transport_concession);
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
            false,
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
            false,
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
            false,
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
                false,
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
                false,
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
            next_attempt_verdict(0, 2, Some(&err), false, None, &fleet_thresholds()),
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
            false,
            Some(Duration::from_secs(90)),
            &fleet_thresholds(),
        );
        assert!(
            verdict.is_deadline_insufficient(),
            "90 s clears the transport threshold (60) but not the default (120): {verdict:?}"
        );
    }

    // ── mika#3126: a cut at the plafond withdraws the concession ──────────

    /// The mika#3126 incident, as a unit, at its real geometry.
    ///
    /// 2026-11-16, `300/660`: a transport-class failure with 290 s of margin.
    /// Cut at the plafond, the concession is withdrawn and the default
    /// threshold applies — `1.0 × 300 = 300 > 290`, refused, and the line says
    /// `exhausted`.
    ///
    /// The positive control sits in the same test **at the same margin**: the
    /// only variable between the two halves is the cut. Without it a predicate
    /// refusing every 290 s margin would pass, and the probe would be asserting
    /// the geometry rather than the fix.
    #[test]
    fn mika3126_une_coupure_au_plafond_perd_la_concession_de_relance_rapide() {
        let budget = LlmTimeoutBudget::new(300, 660).expect("300 < 660");
        let thresholds = RetryThresholds::from_budget(&budget);
        let err = transport();
        let margin = Some(Duration::from_secs(290));

        let refused = next_attempt_verdict(0, 2, Some(&err), true, margin, &thresholds);
        assert!(
            matches!(
                refused,
                RetryVerdict::DeadlineInsufficient {
                    threshold_secs: 300,
                    ..
                }
            ),
            "a cut at the plafond must be held to a whole call, got {refused:?}"
        );
        assert_eq!(attempt_outcome::outcome_for(&refused), "exhausted");

        let allowed = next_attempt_verdict(0, 2, Some(&err), false, margin, &thresholds);
        assert_eq!(
            allowed,
            RetryVerdict::Retry,
            "the same transport failure NOT cut at the plafond keeps the \
             mika#1744 concession (0.50 × 300 = 150 < 290)"
        );
    }

    /// R1's other half: the withdrawal is a fallback onto the existing default
    /// threshold, so a geometry whose envelope carries two whole calls does not
    /// change behaviour.
    ///
    /// The fleet (`120/300`) and mika-arch (`240/900`) both still retry after a
    /// guillotine. This is the probe that would go red if the fix had shipped a
    /// third, larger threshold instead.
    #[test]
    fn mika3126_la_geometrie_de_la_flotte_relance_toujours_apres_une_coupure() {
        let err = transport();

        let fleet = next_attempt_verdict(
            0,
            2,
            Some(&err),
            true,
            Some(Duration::from_secs(180)),
            &fleet_thresholds(),
        );
        assert_eq!(
            fleet,
            RetryVerdict::Retry,
            "120/300: the margin after one whole call is 180 > 120"
        );

        let arch_budget = LlmTimeoutBudget::new(240, 900).expect("240 < 900");
        let arch = next_attempt_verdict(
            0,
            3,
            Some(&err),
            true,
            Some(Duration::from_secs(660)),
            &RetryThresholds::from_budget(&arch_budget),
        );
        assert_eq!(
            arch,
            RetryVerdict::Retry,
            "240/900: the margin after one whole call is 660 > 240"
        );
    }

    /// R2: the four combinations, read off the one site that decides them.
    ///
    /// `(transport, cut)` is the incident; `(transport, not cut)` is the
    /// mika#1744 population the concession exists for; `(non-transport, cut)`
    /// never had it; `(None, _)` is the unclassified failure, which is never
    /// defaulted into it.
    #[test]
    fn mika3126_l_appartenance_a_la_concession_se_decide_en_un_site() {
        let transport = transport();
        let http = http(429, true);

        assert!(
            !transport_concession_applies(Some(&transport as &dyn RetryClass), true),
            "a transport failure cut at its plafond has refuted the concession"
        );
        assert!(
            transport_concession_applies(Some(&transport as &dyn RetryClass), false),
            "a transport failure that was not cut keeps it"
        );
        assert!(
            !transport_concession_applies(Some(&http as &dyn RetryClass), true),
            "a non-transport failure never had it"
        );
        assert!(
            !transport_concession_applies(None, false),
            "an unclassified failure is not defaulted into it"
        );
    }

    // ── D5: no rail may recompute the threshold inline ────────────────────

    /// Does this source line write the non-transport threshold sum?
    ///
    /// Split out of the scan below so it can be probed on both controls — the
    /// omission that made the first version of this guard **hollow**. That
    /// version matched the literal `typical_call_duration_secs()+retry_buffer_secs()`,
    /// which assumes no receiver between the `+` and the second call; every
    /// real spelling has one (`self.`, `budget.`), so it matched nothing at
    /// all — not the violation it was written for, and not even the legitimate
    /// definition in `from_budget`. It passed, and it guarded nothing. A guard
    /// that is only ever asked to find nothing cannot tell "clean tree" from
    /// "broken detector".
    ///
    /// Both accessors on one line, in either order and whatever the receiver.
    /// Comment lines are exempt: prose naming the two is documentation, and the
    /// module above is full of it.
    fn line_writes_the_threshold_sum(line: &str) -> bool {
        let flat: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        if flat.starts_with("//") {
            return false;
        }
        (flat.contains("typical_call_duration_secs()") && flat.contains("retry_buffer_secs()"))
            || (flat.contains("TYPICAL_CALL_DURATION_SECS") && flat.contains("RETRY_BUFFER_SECS"))
    }

    /// The positive control the first version of this guard never had.
    ///
    /// Every spelling that actually appeared in this codebase, plus the two
    /// shapes that must **not** fire: an accessor read on its own (which
    /// decides nothing — `budget.rs` defines them and asserts them in its own
    /// tests, so forbidding one alone would make the defining module violate
    /// its own guard), and prose naming both.
    #[test]
    fn mika2362_the_threshold_detector_detects() {
        // Positive controls — each of these was, or could have been, written.
        for offending in [
            "let t = self.typical_call_duration_secs() + self.retry_buffer_secs();",
            "default_secs: budget.typical_call_duration_secs() + budget.retry_buffer_secs(),",
            "        TYPICAL_CALL_DURATION_SECS + RETRY_BUFFER_SECS",
            "let t = b.retry_buffer_secs() + b.typical_call_duration_secs();",
        ] {
            assert!(
                line_writes_the_threshold_sum(offending),
                "detector missed: {offending}"
            );
        }

        // Negative controls — an accessor alone, and documentation.
        for innocent in [
            "assert_eq!(budget.typical_call_duration_secs(), 180);",
            "pub fn retry_buffer_secs(&self) -> u64 {",
            "/// `typical_call_duration_secs() + retry_buffer_secs()`, and that sum is",
            "// TYPICAL_CALL_DURATION_SECS + RETRY_BUFFER_SECS is what this replaces",
        ] {
            assert!(
                !line_writes_the_threshold_sum(innocent),
                "detector false-positived: {innocent}"
            );
        }
    }

    /// A structural scan, because a behavioural test cannot see this class of
    /// regression.
    ///
    /// Re-introducing a local threshold would make **no** decision wrong on the
    /// day it lands — it would make the three answers *free to diverge again*,
    /// and every existing assertion would stay green throughout the drift.
    /// That is exactly what happened to mika#1744's transport threshold between
    /// its own ticket and mika#2331 §3.3, on a rail nobody was watching.
    ///
    /// The scan targets the **sum**, not the accessors — see
    /// [`line_writes_the_threshold_sum`] for the predicate and for why its
    /// first version was hollow.
    ///
    /// Allowlist: empty. A second site is halt-and-surface, not an entry here.
    #[test]
    fn mika2362_no_rail_recomputes_the_retry_threshold_inline() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let this_file = "retry_gate.rs";
        let mut offenders = Vec::new();
        let mut files_scanned = 0usize;

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
                files_scanned += 1;
                for (n, line) in text.lines().enumerate() {
                    if line_writes_the_threshold_sum(line) {
                        offenders.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
                    }
                }
            }
        }

        // A scan that walked nothing would also report no offender. The count
        // is what separates "clean" from "the tree moved and this test is
        // looking at an empty directory".
        assert!(
            files_scanned > 5,
            "the scan found only {files_scanned} source files under {} — it is not looking at the crate",
            src.display()
        );

        assert!(
            offenders.is_empty(),
            "the deadline threshold must be selected by `retry_gate`, not recomputed inline \
             (mika#2362 D5). Offending sites:\n{}",
            offenders.join("\n")
        );
    }
}

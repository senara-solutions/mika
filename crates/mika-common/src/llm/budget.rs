//! Per-call plafond and per-agent envelope, held together by one invariant
//! (mika#2189).
//!
//! # The failure this exists to close
//!
//! Over the seven days ending 2026-09-05, `llm_calls` carries **209 failures**
//! under a single message — `failed to read response body: … operation timed
//! out` — whose latency distribution has no tail, only two values: **240 s**
//! (171 occurrences) and **120 s** (37). That is not provider variance. It is a
//! client guillotine, crossed once or twice: [`DEFAULT_HTTP_TIMEOUT_SECS`] is
//! 120 and `reqwest::ClientBuilder::timeout` bounds the **whole** request, body
//! read included.
//!
//! The asymmetry that made it unfixable: `MIKA_LLM_HTTP_TIMEOUT_SECS` has let
//! operators raise the **per-call plafond** since mika#1660, while the agent
//! envelope was a bare constant with no knob at all. One could raise the ceiling
//! and not the room that must contain it — and raising the ceiling alone makes a
//! single call eat the whole envelope of a pass that needs three.
//!
//! # What this module holds
//!
//! [`LlmTimeoutBudget`] is the pair `(per-call cap, agent envelope)` plus the
//! three retry thresholds **derived** from the cap rather than written down
//! beside it. All four properties are computed from two numbers, so a budget
//! cannot be half-configured.
//!
//! ## The derivation (D3 / Q1) is a no-op at the default
//!
//! `TYPICAL_CALL_DURATION_SECS = 90`, `RETRY_BUFFER_SECS = 30` and
//! `TRANSPORT_RETRY_MIN_REMAINING_SECS = 60` were literals calibrated against a
//! 120 s cap. Against that cap they are exactly `0.75 ×`, `0.25 ×` and
//! `0.50 ×`. Expressed as fractions they reproduce today's numbers bit for bit
//! **and** follow any later setting — where the literals would have silently
//! described a geometry that no longer existed.
//!
//! ## The containment invariant (D2)
//!
//! [`LlmTimeoutBudget::validate`] refuses `cap >= envelope`. A configuration
//! where one call may consume the entire envelope leaves the agent loop no
//! budget for a second step, which is a setting mistake and not a tight fit.
//! It is checked at provider construction — the same point in the lifecycle
//! where mika#1660 already panics on a too-small cap (Q5). The consequence is
//! written down rather than discovered: **a `mika` that boots is not proof that
//! its budgets are valid; the first call is.**
//!
//! ## The bounded failure cost (AC3-b)
//!
//! Raising a plafond cannot slow a call that already succeeded — a plafond is a
//! plafond. The real regression is on the other side: **a call that fails gets
//! more expensive.** [`LlmTimeoutBudget::max_attempts`] is `floor(envelope /
//! cap)`, so `max_attempts × cap ≤ envelope` holds *by construction* and a
//! failing call cannot overflow the envelope it runs in.
//!
//! At the default geometry that yields **2** — which is precisely what the
//! measurement shows (171 of 209 failures at exactly 240 s = two attempts at
//! 120 s). It is not, however, a pure no-op: before this, a third attempt could
//! start at exactly `remaining == threshold` and carry the failure to 360 s,
//! past the 300 s envelope. That boundary case is now closed. Saying it is a
//! no-op would be more comfortable and less true.

/// Numerator of the typical-call-duration fraction of the per-call cap.
///
/// `90 / 120 = 0.75` — the pre-mika#2189 literal `TYPICAL_CALL_DURATION_SECS`
/// expressed against the default cap it was calibrated on.
const TYPICAL_CALL_NUM: u64 = 3;
/// Denominator of the typical-call-duration fraction (see [`TYPICAL_CALL_NUM`]).
const TYPICAL_CALL_DEN: u64 = 4;

/// Numerator of the retry-buffer fraction of the per-call cap.
///
/// `30 / 120 = 0.25` — the pre-mika#2189 literal `RETRY_BUFFER_SECS`.
const RETRY_BUFFER_NUM: u64 = 1;
/// Denominator of the retry-buffer fraction (see [`RETRY_BUFFER_NUM`]).
const RETRY_BUFFER_DEN: u64 = 4;

/// Numerator of the transport-retry minimum-remaining fraction of the cap.
///
/// `60 / 120 = 0.50` — the pre-mika#2189 literal
/// `TRANSPORT_RETRY_MIN_REMAINING_SECS` (mika#1744).
const TRANSPORT_MIN_REMAINING_NUM: u64 = 1;
/// Denominator of the transport-retry fraction (see [`TRANSPORT_MIN_REMAINING_NUM`]).
const TRANSPORT_MIN_REMAINING_DEN: u64 = 2;

/// A per-call plafond and the per-agent envelope that must contain it.
///
/// Construct through [`LlmTimeoutBudget::new`] (which validates) or
/// [`LlmTimeoutBudget::from_env`]. See the module docs for why the two numbers
/// travel together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LlmTimeoutBudget {
    http_timeout_secs: u64,
    agent_total_timeout_secs: u64,
}

/// Why a `(cap, envelope)` pair was refused.
///
/// Carries both values in every variant: an operator reading this message needs
/// to know which of the two to move, and a message naming only the offender
/// tells them the least useful half.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LlmBudgetError {
    /// The per-call cap is at or above the agent envelope (D2).
    #[error(
        "LLM budget invalid: per-call timeout ({http_timeout_secs}s) must be strictly less than \
         the agent envelope ({agent_total_timeout_secs}s) — as configured, one LLM call may \
         consume the whole envelope and leave the agent loop no budget for a second step. \
         Lower MIKA_LLM_HTTP_TIMEOUT_SECS or raise MIKA_AGENT_TOTAL_TIMEOUT_SECS \
         (per-agent: ~/.mika/agents/<name>/config.toml, keys `llm_http_timeout_secs` and \
         `agent_total_timeout_secs`)."
    )]
    CapNotContained {
        /// The offending per-call cap, in seconds.
        http_timeout_secs: u64,
        /// The envelope it failed to fit inside, in seconds.
        agent_total_timeout_secs: u64,
    },

    /// The agent envelope is below [`MIN_AGENT_TOTAL_TIMEOUT_SECS`].
    ///
    /// The mirror of the `MIN_HTTP_TIMEOUT_SECS` floor mika#1660 established for
    /// the cap: an envelope this small aborts an agent turn before it can take
    /// a single useful step, and is essentially always a units mistake.
    #[error(
        "LLM budget invalid: agent envelope ({agent_total_timeout_secs}s) is below the minimum \
         of {min}s; an envelope this small aborts an agent turn before it can complete one step \
         (per-call timeout is {http_timeout_secs}s)"
    )]
    EnvelopeTooSmall {
        /// The per-call cap in force, in seconds — reported for context.
        http_timeout_secs: u64,
        /// The offending envelope, in seconds.
        agent_total_timeout_secs: u64,
        /// The floor it failed to clear, in seconds.
        min: u64,
    },
}

/// Minimum accepted agent envelope, in seconds.
///
/// Chosen as `2 ×` [`super::MIN_HTTP_TIMEOUT_SECS`] rather than as a fresh
/// magic number: the smallest envelope that can contain a minimum-sized call
/// *and* have room left over is the smallest envelope that means anything.
pub const MIN_AGENT_TOTAL_TIMEOUT_SECS: u64 = 2 * super::MIN_HTTP_TIMEOUT_SECS;

/// Default per-agent envelope, in seconds.
///
/// 300 — the value `crate::planning::policy::AGENT_TOTAL_TIMEOUT_SECS` carried
/// as a bare constant before mika#2189. Unchanged: this ticket makes the
/// envelope *settable*, it does not move the fleet default.
pub const DEFAULT_AGENT_TOTAL_TIMEOUT_SECS: u64 = 300;

/// Environment variable that overrides the per-agent envelope.
pub const AGENT_TOTAL_TIMEOUT_ENV_VAR: &str = "MIKA_AGENT_TOTAL_TIMEOUT_SECS";

impl LlmTimeoutBudget {
    /// Build a validated budget.
    ///
    /// Returns [`LlmBudgetError`] rather than panicking so the pair can be
    /// probed in a test with **both** controls in one call — a probe that can
    /// only observe the failing side cannot tell "the invariant rejects bad
    /// configs" from "the invariant rejects everything".
    pub fn new(
        http_timeout_secs: u64,
        agent_total_timeout_secs: u64,
    ) -> Result<Self, LlmBudgetError> {
        let budget = Self {
            http_timeout_secs,
            agent_total_timeout_secs,
        };
        budget.validate()?;
        Ok(budget)
    }

    /// Build a budget **without** checking the pair.
    ///
    /// For callers that resolve the two numbers from configuration and must
    /// hand the pair on to the one place allowed to refuse it (provider
    /// construction, Q5). Named rather than derived from `new` so that a
    /// reader can see at the callsite that validation was deferred, not
    /// forgotten.
    pub fn unvalidated(http_timeout_secs: u64, agent_total_timeout_secs: u64) -> Self {
        Self {
            http_timeout_secs,
            agent_total_timeout_secs,
        }
    }

    /// Build a budget from the two environment variables, unvalidated.
    ///
    /// The per-call side goes through [`super::http_timeout_secs`], so the
    /// mika#1660 panics on an unparseable or too-small cap still fire here.
    /// Validation of the *pair* is deliberately left to [`Self::validate`] at
    /// provider construction (Q5) so this stays usable from calibration and
    /// test paths that build a provider directly.
    pub fn from_env() -> Self {
        Self {
            http_timeout_secs: super::http_timeout_secs(),
            agent_total_timeout_secs: parse_agent_total_timeout(
                std::env::var(AGENT_TOTAL_TIMEOUT_ENV_VAR).ok().as_deref(),
            ),
        }
    }

    /// The per-call plafond, in seconds — what `reqwest` is given.
    pub fn http_timeout_secs(&self) -> u64 {
        self.http_timeout_secs
    }

    /// The per-agent envelope, in seconds — the agent-loop deadline.
    pub fn agent_total_timeout_secs(&self) -> u64 {
        self.agent_total_timeout_secs
    }

    /// D2: the cap must fit strictly inside the envelope, and the envelope must
    /// clear its own floor.
    ///
    /// Order matters: an envelope below the floor is reported as such even when
    /// it also fails containment, because "you typed seconds where milliseconds
    /// were meant" is a more useful thing to be told than "these two numbers do
    /// not fit".
    pub fn validate(&self) -> Result<(), LlmBudgetError> {
        if self.agent_total_timeout_secs < MIN_AGENT_TOTAL_TIMEOUT_SECS {
            return Err(LlmBudgetError::EnvelopeTooSmall {
                http_timeout_secs: self.http_timeout_secs,
                agent_total_timeout_secs: self.agent_total_timeout_secs,
                min: MIN_AGENT_TOTAL_TIMEOUT_SECS,
            });
        }
        if self.http_timeout_secs >= self.agent_total_timeout_secs {
            return Err(LlmBudgetError::CapNotContained {
                http_timeout_secs: self.http_timeout_secs,
                agent_total_timeout_secs: self.agent_total_timeout_secs,
            });
        }
        Ok(())
    }

    /// Estimated typical call duration used by the deadline-aware retry abort.
    ///
    /// `0.75 ×` the cap — see the module docs for why this is a fraction now.
    pub fn typical_call_duration_secs(&self) -> u64 {
        self.http_timeout_secs * TYPICAL_CALL_NUM / TYPICAL_CALL_DEN
    }

    /// Slack added to [`Self::typical_call_duration_secs`] before retrying a
    /// non-transport failure. `0.25 ×` the cap.
    pub fn retry_buffer_secs(&self) -> u64 {
        self.http_timeout_secs * RETRY_BUFFER_NUM / RETRY_BUFFER_DEN
    }

    /// Remaining-deadline budget required to retry a **transport**-class
    /// failure (mika#1744). `0.50 ×` the cap.
    ///
    /// Smaller than the non-transport threshold on purpose: DNS, refused
    /// connections and TLS handshakes resolve in seconds, not in a full cap.
    pub fn transport_retry_min_remaining_secs(&self) -> u64 {
        self.http_timeout_secs * TRANSPORT_MIN_REMAINING_NUM / TRANSPORT_MIN_REMAINING_DEN
    }

    /// AC3-b: how many attempts may run before the worst case leaves the
    /// envelope.
    ///
    /// `floor(envelope / cap)`, floored at 1 (one attempt always runs — a
    /// budget that permits zero calls is not a budget) and capped at
    /// `hard_cap` (the provider's own `MAX_RETRIES + 1`, so a generous envelope
    /// cannot silently widen the retry chain past what the provider intends).
    ///
    /// The point of expressing it this way is that `attempts × cap ≤ envelope`
    /// is then true by construction rather than by observation.
    pub fn max_attempts(&self, hard_cap: u32) -> u32 {
        let derived = self.agent_total_timeout_secs / self.http_timeout_secs.max(1);
        u32::try_from(derived)
            .unwrap_or(u32::MAX)
            .clamp(1, hard_cap.max(1))
    }

    /// Worst-case wall-clock cost of a fully-failing call, in seconds.
    ///
    /// Exposed so the bound can be asserted directly rather than re-derived at
    /// each callsite.
    pub fn worst_case_failure_secs(&self, hard_cap: u32) -> u64 {
        u64::from(self.max_attempts(hard_cap)) * self.http_timeout_secs
    }
}

impl Default for LlmTimeoutBudget {
    /// The shipped geometry: 120 s per call inside a 300 s envelope.
    fn default() -> Self {
        Self {
            http_timeout_secs: super::DEFAULT_HTTP_TIMEOUT_SECS,
            agent_total_timeout_secs: DEFAULT_AGENT_TOTAL_TIMEOUT_SECS,
        }
    }
}

/// Pure resolver behind the envelope half of [`LlmTimeoutBudget::from_env`].
///
/// Split out so parsing is testable without mutating the process-global env
/// var, which would race parallel tests — the same reason
/// [`super::parse_http_timeout`] exists.
///
/// Three-tier, matching every other budget knob in this codebase: absent or
/// empty → default; unparseable or `0` → default **with a WARN**, because a
/// silent fallback on a value the operator deliberately typed is how a setting
/// becomes decorative.
fn parse_agent_total_timeout(raw: Option<&str>) -> u64 {
    let Some(raw) = raw else {
        return DEFAULT_AGENT_TOTAL_TIMEOUT_SECS;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return DEFAULT_AGENT_TOTAL_TIMEOUT_SECS;
    }
    match trimmed.parse::<u64>() {
        Ok(0) | Err(_) => {
            tracing::warn!(
                event = "agent_total_timeout_invalid",
                raw = %raw,
                default = DEFAULT_AGENT_TOTAL_TIMEOUT_SECS,
                "{AGENT_TOTAL_TIMEOUT_ENV_VAR} is not a positive integer number of seconds; \
                 falling back to the default"
            );
            DEFAULT_AGENT_TOTAL_TIMEOUT_SECS
        }
        Ok(secs) => secs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{
        DEFAULT_HTTP_TIMEOUT_SECS, RETRY_BUFFER_SECS, TRANSPORT_RETRY_MIN_REMAINING_SECS,
        TYPICAL_CALL_DURATION_SECS,
    };

    /// The provider's own attempt ceiling (`MAX_RETRIES + 1`). Duplicated here
    /// rather than imported because `openai::MAX_RETRIES` is private; the
    /// coupling is pinned by `openai::tests::max_attempts_respects_provider_hard_cap`.
    const HARD_CAP: u32 = 4;

    // -- D3 / Q1: the fractions reproduce the literals at the default cap --

    /// The migration from literals to fractions must be a **no-op at the
    /// default plafond**, or the calibration read off the old constants
    /// (0.75 / 0.25 / 0.50) was wrong. Per the Fire-Disposition table, a
    /// failure here is halt-and-escalate: do not tune the fractions to match.
    #[test]
    fn derived_thresholds_reproduce_the_literals_at_the_default_cap() {
        let budget = LlmTimeoutBudget::default();
        assert_eq!(budget.http_timeout_secs(), DEFAULT_HTTP_TIMEOUT_SECS);
        assert_eq!(
            budget.typical_call_duration_secs(),
            TYPICAL_CALL_DURATION_SECS
        );
        assert_eq!(budget.retry_buffer_secs(), RETRY_BUFFER_SECS);
        assert_eq!(
            budget.transport_retry_min_remaining_secs(),
            TRANSPORT_RETRY_MIN_REMAINING_SECS
        );
    }

    #[test]
    fn derived_thresholds_follow_a_raised_cap() {
        // The mika-arch setting proposed by D4: 240 s inside 900 s.
        let budget = LlmTimeoutBudget::new(240, 900).expect("240 < 900");
        assert_eq!(budget.typical_call_duration_secs(), 180);
        assert_eq!(budget.retry_buffer_secs(), 60);
        assert_eq!(budget.transport_retry_min_remaining_secs(), 120);
    }

    // -- D2: containment, probed on both controls in the same call --

    /// V1's named probe: a positive **and** a negative control in one test, per
    /// `feedback_a_probe_needs_both_controls_in_the_same_call`. A probe that
    /// only ever sees the failing side cannot distinguish an invariant that
    /// works from one that rejects everything.
    #[test]
    fn containment_accepts_the_shipped_geometry_and_refuses_an_uncontained_cap() {
        // Positive control: today's production geometry must stay accepted.
        assert!(LlmTimeoutBudget::new(120, 300).is_ok());

        // Negative control: a cap that eats the whole envelope is refused.
        let err = LlmTimeoutBudget::new(300, 300).expect_err("cap == envelope must be refused");
        assert!(matches!(err, LlmBudgetError::CapNotContained { .. }));

        // And the message names BOTH values, not just the offender.
        let msg = err.to_string();
        assert!(msg.contains("300s"), "message must name the values: {msg}");
        assert!(
            msg.contains("MIKA_AGENT_TOTAL_TIMEOUT_SECS") && msg.contains("config.toml"),
            "message must name the fix: {msg}"
        );
    }

    #[test]
    fn containment_refuses_a_cap_above_the_envelope() {
        assert!(matches!(
            LlmTimeoutBudget::new(600, 300),
            Err(LlmBudgetError::CapNotContained { .. })
        ));
    }

    #[test]
    fn envelope_below_its_floor_is_reported_as_such_not_as_containment() {
        // 5 < MIN_AGENT_TOTAL_TIMEOUT_SECS (20), and 10 >= 5 would also fail
        // containment — the floor diagnosis must win, being the more useful one.
        let err = LlmTimeoutBudget::new(10, 5).expect_err("envelope below floor");
        assert!(matches!(err, LlmBudgetError::EnvelopeTooSmall { .. }));
    }

    // -- AC3-b: the failure cost is bounded by construction --

    #[test]
    fn worst_case_failure_never_exceeds_the_envelope() {
        for (cap, envelope) in [(120, 300), (240, 900), (60, 300), (10, 20), (100, 1000)] {
            let budget = LlmTimeoutBudget::new(cap, envelope).expect("valid geometry");
            assert!(
                budget.worst_case_failure_secs(HARD_CAP) <= envelope,
                "cap={cap} envelope={envelope} worst={} exceeds the envelope",
                budget.worst_case_failure_secs(HARD_CAP)
            );
        }
    }

    /// The default geometry allows exactly two attempts — which is what the
    /// mika#2189 measurement shows (171 of 209 failures at exactly 240 s).
    #[test]
    fn default_geometry_allows_the_two_attempts_the_measurement_shows() {
        assert_eq!(LlmTimeoutBudget::default().max_attempts(HARD_CAP), 2);
        assert_eq!(
            LlmTimeoutBudget::default().worst_case_failure_secs(HARD_CAP),
            240
        );
    }

    #[test]
    fn a_generous_envelope_cannot_widen_the_chain_past_the_provider_hard_cap() {
        // floor(10000 / 10) = 1000 attempts on the arithmetic alone.
        let budget = LlmTimeoutBudget::new(10, 10_000).expect("valid geometry");
        assert_eq!(budget.max_attempts(HARD_CAP), HARD_CAP);
    }

    #[test]
    fn at_least_one_attempt_always_runs() {
        // Contrived but reachable via `from_env`, which does not validate the
        // pair: a cap larger than its envelope must still permit one call
        // rather than silently permitting none.
        let budget = LlmTimeoutBudget {
            http_timeout_secs: 500,
            agent_total_timeout_secs: 300,
        };
        assert_eq!(budget.max_attempts(HARD_CAP), 1);
    }

    // -- envelope parsing, three-tier --

    #[test]
    fn envelope_parse_defaults_when_absent_or_blank() {
        assert_eq!(
            parse_agent_total_timeout(None),
            DEFAULT_AGENT_TOTAL_TIMEOUT_SECS
        );
        assert_eq!(
            parse_agent_total_timeout(Some("")),
            DEFAULT_AGENT_TOTAL_TIMEOUT_SECS
        );
        assert_eq!(
            parse_agent_total_timeout(Some("   ")),
            DEFAULT_AGENT_TOTAL_TIMEOUT_SECS
        );
    }

    #[test]
    fn envelope_parse_defaults_on_zero_or_garbage() {
        assert_eq!(
            parse_agent_total_timeout(Some("0")),
            DEFAULT_AGENT_TOTAL_TIMEOUT_SECS
        );
        assert_eq!(
            parse_agent_total_timeout(Some("banana")),
            DEFAULT_AGENT_TOTAL_TIMEOUT_SECS
        );
        assert_eq!(
            parse_agent_total_timeout(Some("-1")),
            DEFAULT_AGENT_TOTAL_TIMEOUT_SECS
        );
    }

    #[test]
    fn envelope_parse_accepts_a_valid_override() {
        assert_eq!(parse_agent_total_timeout(Some("900")), 900);
        assert_eq!(parse_agent_total_timeout(Some(" 900 ")), 900);
    }

    #[test]
    fn default_is_the_shipped_geometry() {
        let d = LlmTimeoutBudget::default();
        assert_eq!(d.http_timeout_secs(), 120);
        assert_eq!(d.agent_total_timeout_secs(), 300);
        assert!(d.validate().is_ok());
    }
}

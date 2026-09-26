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
//!
//! # What a plafond can physically carry (mika#2280)
//!
//! The plafond bounds the **whole** request, body read included, and it is a
//! property of the *client* rather than of the request — so a 200-token tool
//! call and an 8 000-token plan draft run under the same guillotine. That makes
//! a second quantity meaningful: how many output tokens a cap can carry at all.
//!
//! The arithmetic is not invented here. `well_known_agents.rs` already wrote it
//! for mika-arch (mika#2296): *"the measured throughput is ~66 tok/s (8192
//! tokens in 123 s), so the 240 s plafond caps one call at ~16 000 tokens: the
//! TIME budget is the real brake"*. [`LlmTimeoutBudget::reachable_output_tokens`]
//! is that sentence made callable, minus a prefill reserve.
//!
//! **It is reported, never enforced.** Nothing in the retry path, in client
//! construction, or in any validation reads it. It exists so an operator can
//! tell a call cut at its plafond ("the model was still generating") from a call
//! cut anywhere else ("the network died") — a distinction
//! [`super::error::LlmError::error_class`] cannot make, since both answer
//! `transport_timeout`.

/// Numerator of the share of the plafond reserved for connection, request
/// upload and prefill — i.e. everything that is not output generation.
///
/// `1 / 4`, in the fractional form this module already uses for its three retry
/// thresholds (mika#2189 D3), so it follows any later plafond automatically
/// instead of describing a geometry that stopped existing.
const PREFILL_RESERVE_NUM: u64 = 1;
/// Denominator of the prefill-reserve fraction (see [`PREFILL_RESERVE_NUM`]).
const PREFILL_RESERVE_DEN: u64 = 4;

/// Default assumed **floor** on output-token throughput, in tokens per second.
///
/// # Where 50 comes from, and why it is not 66
///
/// The only throughput measured in this repository is `~66 tok/s` (8192 tokens
/// in 123 s on `moonshotai/kimi-k2.5`, recorded in `MIKA_ARCH_CONFIG` by
/// mika#2296). 50 is that figure with ~25 % of margin downwards.
///
/// The margin is not decorative prudence, it corrects a **censored**
/// population: throughput is only observable on calls that *succeeded*, and the
/// slow ones were killed at the plafond — which is the very failure mika#2280
/// exists to attribute. So the observed floor is itself an **over**estimate of
/// the true floor, and a constant posed on the observed value would describe a
/// world with the interesting cases removed from it.
///
/// The number serves a **reading context**, never a brake. A coarse constant is
/// acceptable for that and would not be for a per-request plafond — which is the
/// second reason that remedy is a follow-up rather than part of this change.
///
/// Re-measure with the query in the mika#2280 plan (§ D6) before moving it.
pub const DEFAULT_OUTPUT_TOKENS_PER_SEC_FLOOR: u64 = 50;

/// Environment variable overriding [`DEFAULT_OUTPUT_TOKENS_PER_SEC_FLOOR`].
pub const OUTPUT_TOKENS_PER_SEC_FLOOR_ENV_VAR: &str = "MIKA_LLM_OUTPUT_TOKENS_PER_SEC_FLOOR";

/// Numerator of the tolerance under which an elapsed time still counts as
/// "cut at the plafond" (mika#2280 D3).
///
/// `98 / 100`. Derived from the measured distribution rather than picked for
/// roundness: mika#2189's 209 failures have **no tail**, only two values —
/// 171 at exactly 240 s and 37 at exactly 120 s. The events pile up on the
/// exact figure, so a tight tolerance suffices, and tight is the right side of
/// the risk: a false positive would send an operator to raise a plafond that
/// was not the cause.
const CAP_EXHAUSTION_TOLERANCE_NUM: u64 = 98;
/// Denominator of the cap-exhaustion tolerance (see [`CAP_EXHAUSTION_TOLERANCE_NUM`]).
const CAP_EXHAUSTION_TOLERANCE_DEN: u64 = 100;

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
    ///
    /// **Sized on [`Self::max_attempts`], never on
    /// [`Self::effective_max_attempts`]** — see that method for why swapping
    /// them would break the mika#2342 watchdog.
    pub fn worst_case_failure_secs(&self, hard_cap: u32) -> u64 {
        u64::from(self.max_attempts(hard_cap)) * self.http_timeout_secs
    }

    /// How many attempts are *reachable* once the deadline guard is accounted
    /// for (mika#2362 D4).
    ///
    /// # The off-by-one this measures
    ///
    /// [`Self::max_attempts`] is `floor(envelope / cap)` — arithmetic with zero
    /// overhead. The deadline guard measures a real clock, and refuses another
    /// attempt when the remaining margin is *below* the non-transport
    /// threshold, which is `0.75 × cap + 0.25 × cap = 1.0 × cap` exactly. After
    /// an attempt consuming the full cap the margin is `envelope − cap − ε`
    /// with `ε > 0` always, so the last nominal attempt is unreachable whenever
    /// `envelope ≤ 2 × cap` — and more generally whenever the envelope is an
    /// exact multiple of the cap.
    ///
    /// | Geometry | `max_attempts` | reachable |
    /// |---|---|---|
    /// | 120/300 (fleet default) | 2 | **2** — `300 − 120 = 180 > 120` |
    /// | 300/600 (the mika#2362 incident) | 2 | **1** — `600 − 300 = 300`, not `> 300` |
    /// | 240/900 (mika-arch, mika#2189) | 3 | **3** |
    /// | 200/600 | 3 | **2** |
    /// | 420/600 | 1 | 1 |
    ///
    /// # Why it does not replace `max_attempts`
    ///
    /// Two reasons, and the second is the expensive one.
    ///
    /// The **transport** class clears a smaller threshold (`0.50 × cap`,
    /// mika#1744), so at 300/600 a transport failure *does* get its second
    /// attempt: the real worst case there is `2 × 300 = 600 s`, the whole
    /// envelope. This method reports the non-transport figure, which is the
    /// pessimistic one for *reachability* and the optimistic one for *cost*.
    ///
    /// So making [`Self::worst_case_failure_secs`] read this instead would take
    /// the mika#2342 watchdog from `660 s` to `360 s` at that geometry and cut
    /// a perfectly legitimate transport chain mid-flight — a guaranteed false
    /// positive on a mechanism whose entire value is that it never fires.
    /// `budget::tests::mika2362_worst_case_is_unchanged_by_the_effective_count`
    /// exists to make that swap go red.
    ///
    /// This number is **reported**, never enforced: nothing in the retry path
    /// reads it.
    pub fn effective_max_attempts(&self, hard_cap: u32) -> u32 {
        let nominal = self.max_attempts(hard_cap);
        // Read from the one owner of that sum rather than writing it a second
        // time. The first draft of this method did write it — `self.typical…()
        // + self.retry_buffer_secs()` — one commit after declaring
        // `RetryThresholds::from_budget` the crate's only writer, and the guard
        // below did not catch it. Reading it here keeps the claim true *and*
        // ties this count to the threshold the deadline guard actually applies.
        let threshold = super::retry_gate::RetryThresholds::from_budget(self).default_secs();

        // Walk the chain the way the clock does: each attempt consumes the cap,
        // and the next one only runs while the margin strictly exceeds the
        // threshold. Strictly, because the guard refuses at `remaining <
        // threshold` and `ε > 0` makes equality unreachable from above.
        let mut reachable = 1;
        while reachable < nominal {
            let consumed = u64::from(reachable) * self.http_timeout_secs;
            let remaining = self.agent_total_timeout_secs.saturating_sub(consumed);
            if remaining <= threshold {
                break;
            }
            reachable += 1;
        }
        reachable
    }

    /// Did an attempt that lasted `elapsed_ms` run out of **plafond**, as
    /// opposed to running into a network failure (mika#2280 D2/D3)?
    ///
    /// `elapsed_ms >= cap × 98 / 100`. A body that stops arriving at ≈ the cap
    /// is a guillotine — the model was still generating; a body that stops at an
    /// arbitrary instant is a breakdown. Both surface as
    /// `LlmError::Transport` and both answer `transport_timeout` to
    /// [`super::error::LlmError::error_class`], which is precisely why the
    /// discriminator has to be the elapsed time.
    ///
    /// **Only meaningful on a rail that applies the budget it was built with.**
    /// The Anthropic rail hands `reqwest` its own literal instead of reading
    /// the plafond (mika#2189), so this must not be asked there — it would
    /// compare against a bound that does not govern the call.
    ///
    /// This is the **only** site where the tolerance is written. Two rails ask
    /// the question and neither carries a copy — the lesson `retry_gate` had to
    /// engrave in mika#2362, where three copies of one threshold drifted apart.
    pub fn is_cap_exhaustion(&self, elapsed_ms: u64) -> bool {
        let cap_ms = self.http_timeout_secs.saturating_mul(1_000);
        let floor_ms = cap_ms.saturating_mul(CAP_EXHAUSTION_TOLERANCE_NUM)
            / CAP_EXHAUSTION_TOLERANCE_DEN.max(1);
        elapsed_ms >= floor_ms
    }

    /// How many output tokens this plafond can physically carry, at an assumed
    /// throughput floor (mika#2280 AC1).
    ///
    /// `cap × (1 − prefill reserve) × floor`, with the reserve expressed as the
    /// fraction [`PREFILL_RESERVE_NUM`] / [`PREFILL_RESERVE_DEN`] of the cap —
    /// never as a literal in seconds, so the figure tracks a plafond an operator
    /// later moves.
    ///
    /// On mika-arch's shipped geometry (240 s) at the measured 66 tok/s this
    /// gives `240 × 3/4 × 66 = 11 880`. The `~16 000` quoted in
    /// `MIKA_ARCH_CONFIG` is the same arithmetic **without** the prefill
    /// reserve (`240 × 66 = 15 840`); the gap between the two figures is the
    /// reserve, and it is deliberate rather than a disagreement.
    ///
    /// # Reported, not enforced
    ///
    /// **Nothing in the retry path, in client construction, or in any
    /// validation reads this.** It is neither validated nor clamped against
    /// `llm_max_tokens`: a declared budget above what a plafond can carry is not
    /// a defect in itself — mika#2296 chose exactly that for mika-arch, as "a
    /// ceiling made non-binding", knowing time is the brake. A guard firing on
    /// the declaration would contradict a documented decision at every startup,
    /// and a warning that contradicts a decision is a warning that gets muted.
    /// What warrants an operator's attention is a *crossing*, once per cut call
    /// — see `llm_call_cap_exhausted` on the two OpenAI-shaped rails.
    pub fn reachable_output_tokens(&self, floor_tok_per_sec: u64) -> u64 {
        let generating_secs = self
            .http_timeout_secs
            .saturating_mul(PREFILL_RESERVE_DEN.saturating_sub(PREFILL_RESERVE_NUM))
            / PREFILL_RESERVE_DEN.max(1);
        generating_secs.saturating_mul(floor_tok_per_sec)
    }
}

/// The assumed output-throughput floor, from the environment (mika#2280 AC2).
///
/// Three-tier, like every other budget knob here: absent or empty → default;
/// unparseable, `0` or negative → default **with a WARN**, because a silent
/// fallback on a value an operator deliberately typed is how a setting becomes
/// decorative.
///
/// **Never panics**, unlike [`super::http_timeout_secs`]: this is read from the
/// same cold paths mika#2293's non-panicking reader serves, and a diagnostic
/// constant must not be able to abort a process.
pub fn output_tokens_per_sec_floor() -> u64 {
    parse_output_tokens_per_sec_floor(
        std::env::var(OUTPUT_TOKENS_PER_SEC_FLOOR_ENV_VAR)
            .ok()
            .as_deref(),
    )
}

/// Pure resolver behind [`output_tokens_per_sec_floor`].
///
/// Split out so parsing is testable without mutating the process-global
/// environment, which would race parallel tests — the same reason
/// [`parse_agent_total_timeout`] exists.
fn parse_output_tokens_per_sec_floor(raw: Option<&str>) -> u64 {
    let Some(raw) = raw else {
        return DEFAULT_OUTPUT_TOKENS_PER_SEC_FLOOR;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return DEFAULT_OUTPUT_TOKENS_PER_SEC_FLOOR;
    }
    match trimmed.parse::<u64>() {
        Ok(0) | Err(_) => {
            tracing::warn!(
                event = "output_tokens_per_sec_floor_invalid",
                raw = %raw,
                default = DEFAULT_OUTPUT_TOKENS_PER_SEC_FLOOR,
                "{OUTPUT_TOKENS_PER_SEC_FLOOR_ENV_VAR} is not a positive integer number of \
                 tokens per second; falling back to the default"
            );
            DEFAULT_OUTPUT_TOKENS_PER_SEC_FLOOR
        }
        Ok(v) => v,
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
        DEFAULT_ATTEMPTS_HARD_CAP, DEFAULT_HTTP_TIMEOUT_SECS, RETRY_BUFFER_SECS,
        TRANSPORT_RETRY_MIN_REMAINING_SECS, TYPICAL_CALL_DURATION_SECS,
    };

    /// The attempt ceiling the rails run under. Imported since mika#2342 —
    /// it used to be a local `4` duplicating a constant private to `openai.rs`,
    /// and the move to `llm/mod.rs` (so a trait default method could read it)
    /// removes the duplication rather than relocating it.
    const HARD_CAP: u32 = DEFAULT_ATTEMPTS_HARD_CAP;

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

    // -- mika#2362 D4: the honest attempt count, and the net it must not move --

    /// T4 — the five geometries of the plan's E4 table, whose right-hand column
    /// is the whole diagnosis: at `envelope = k × cap` the last nominal attempt
    /// is unreachable, and 300/600 is the `k = 2` case of that family rather
    /// than a peculiarity of the number 300.
    ///
    /// **If this is red, halt.** The arithmetic is what mika#2362's D2 rests
    /// on; adjusting the expectation to whatever was observed would repair the
    /// test and lose the finding.
    #[test]
    fn mika2362_effective_attempts_on_the_five_measured_geometries() {
        let cases = [
            (120, 300, 2, 2, "fleet default — 300 − 120 = 180 > 120"),
            (300, 600, 2, 1, "the incident — 600 − 300 = 300, not > 300"),
            (240, 900, 3, 3, "mika-arch (mika#2189)"),
            (200, 600, 3, 2, "the third attempt is refused"),
            (420, 600, 1, 1, "one attempt, nominal and effective agree"),
        ];
        for (cap, envelope, nominal, effective, why) in cases {
            let budget = LlmTimeoutBudget::new(cap, envelope).expect("valid geometry");
            assert_eq!(
                budget.max_attempts(HARD_CAP),
                nominal,
                "nominal for {cap}/{envelope} ({why})"
            );
            assert_eq!(
                budget.effective_max_attempts(HARD_CAP),
                effective,
                "effective for {cap}/{envelope} ({why})"
            );
        }
    }

    /// The reported count can never exceed the count that bounds the loop —
    /// asserted over the same geometries as
    /// `worst_case_failure_never_exceeds_the_envelope`, plus the incident's.
    #[test]
    fn mika2362_effective_never_exceeds_nominal() {
        for (cap, envelope) in [
            (120, 300),
            (240, 900),
            (60, 300),
            (10, 20),
            (100, 1000),
            (300, 600),
        ] {
            let budget = LlmTimeoutBudget::new(cap, envelope).expect("valid geometry");
            assert!(
                budget.effective_max_attempts(HARD_CAP) <= budget.max_attempts(HARD_CAP),
                "cap={cap} envelope={envelope}: effective exceeds nominal"
            );
            assert!(
                budget.effective_max_attempts(HARD_CAP) >= 1,
                "cap={cap} envelope={envelope}: one attempt always runs"
            );
        }
    }

    /// T5 — the guard-rail of the whole ticket.
    ///
    /// `worst_case_failure_secs` is what the mika#2342 LLM-call watchdog is
    /// sized on. A future "simplification" replacing `max_attempts` with
    /// `effective_max_attempts` there would shorten the net — at 300/600 from
    /// 660 s to 360 s — and cut a legitimate **transport** chain, which clears
    /// the smaller `0.50 × cap` threshold and really does get its second
    /// attempt. That is a guaranteed false positive on a mechanism whose value
    /// is its silence.
    ///
    /// The literals below are the pre-mika#2362 values, written out rather than
    /// re-derived: a test that recomputed them from the same code it guards
    /// would follow the regression it exists to catch.
    #[test]
    fn mika2362_worst_case_is_unchanged_by_the_effective_count() {
        let cases = [
            (120u64, 300u64, 240u64),
            (300, 600, 600),
            (240, 900, 720),
            (200, 600, 600),
            (420, 600, 420),
        ];
        for (cap, envelope, worst) in cases {
            let budget = LlmTimeoutBudget::new(cap, envelope).expect("valid geometry");
            assert_eq!(
                budget.worst_case_failure_secs(HARD_CAP),
                worst,
                "the mika#2342 net must not move for {cap}/{envelope}"
            );
        }
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

    // -- mika#2280: what a plafond can physically carry --

    /// AC1 — the formula reproduces mika#2296's own arithmetic on its own
    /// numbers, and **documents the gap** rather than pretending to match a
    /// figure that was rounded by hand.
    ///
    /// `MIKA_ARCH_CONFIG` says a 240 s plafond "caps one call at ~16 000
    /// tokens" at ~66 tok/s. That is `240 × 66 = 15 840`, computed without a
    /// prefill reserve. This method subtracts the reserve on purpose, so it
    /// answers `240 × 3/4 × 66 = 11 880`. Both are asserted: the raw product to
    /// show the comment's figure is the same arithmetic, the reserved one to
    /// pin what this method returns.
    #[test]
    fn mika2280_reachable_tokens_reproduces_the_mika2296_arithmetic() {
        const MEASURED_TOK_PER_SEC: u64 = 66;

        let arch = LlmTimeoutBudget::new(240, 900).expect("mika-arch geometry");

        // The figure MIKA_ARCH_CONFIG quotes, un-reserved: "~16 000".
        assert_eq!(240 * MEASURED_TOK_PER_SEC, 15_840);

        // What this method reports — the same arithmetic minus the 1/4 reserve.
        assert_eq!(
            arch.reachable_output_tokens(MEASURED_TOK_PER_SEC),
            11_880,
            "240 s × 3/4 × 66 tok/s — the gap with the ~16 000 of MIKA_ARCH_CONFIG \
             IS the prefill reserve, and it is deliberate"
        );

        // And mika-arch's declared 32768 sits well above it — which mika#2296
        // decided knowingly ("a ceiling made non-binding"), so nothing here
        // treats it as a defect.
        assert!(arch.reachable_output_tokens(MEASURED_TOK_PER_SEC) < 32_768);
    }

    /// The figure is a fraction of the plafond, never a literal in seconds:
    /// doubling the cap must double what it can carry.
    #[test]
    fn mika2280_reachable_tokens_follows_the_plafond() {
        let fleet = LlmTimeoutBudget::default(); // 120/300
        let arch = LlmTimeoutBudget::new(240, 900).expect("valid geometry");

        assert_eq!(fleet.reachable_output_tokens(50), 120 * 3 / 4 * 50);
        assert_eq!(
            arch.reachable_output_tokens(50),
            2 * fleet.reachable_output_tokens(50),
            "twice the plafond carries twice the output"
        );
    }

    /// The fleet default (120 s) against the two declared output budgets the
    /// ticket measures — the constatation E2, expressed on this side of the
    /// crate boundary. The agent-side companion lives in
    /// `mika-agent`'s `well_known_agents::tests`, which is the only crate that
    /// can see those constants **and** this method.
    #[test]
    fn mika2280_the_fleet_plafond_carries_less_than_mika_dev_declares() {
        let fleet = LlmTimeoutBudget::default();
        let reachable = fleet.reachable_output_tokens(DEFAULT_OUTPUT_TOKENS_PER_SEC_FLOOR);

        assert_eq!(reachable, 4_500, "120 s × 3/4 × 50 tok/s");
        assert!(
            reachable < 8_192,
            "mika-dev declares 8192 under a 120 s plafond that carries {reachable}"
        );
        assert!(
            reachable < 16_384,
            "mika-qa declares 16384 under the same plafond"
        );
    }

    /// A zero floor is arithmetic, not a configuration: `output_tokens_per_sec_floor`
    /// refuses it upstream, and this method must not divide by anything.
    #[test]
    fn mika2280_reachable_tokens_is_total_on_degenerate_input() {
        let budget = LlmTimeoutBudget::default();
        assert_eq!(budget.reachable_output_tokens(0), 0);
        assert_eq!(budget.reachable_output_tokens(u64::MAX), u64::MAX);
    }

    /// D3 — the tolerance, with both controls in the same call.
    ///
    /// A probe that only saw the firing side could not tell a discriminator
    /// that works from one that answers `true` to everything — and `true` to
    /// everything is exactly what turns this instrument into a liar.
    #[test]
    fn mika2280_cap_exhaustion_is_the_measured_two_percent() {
        let fleet = LlmTimeoutBudget::default(); // 120 s

        // Positive control: the measured signature — a body cut AT the cap.
        assert!(fleet.is_cap_exhaustion(120_000));
        // And just inside the tolerance: 98 % of 120 s.
        assert!(fleet.is_cap_exhaustion(117_600));

        // Negative control: a body that stopped at an arbitrary instant. That
        // is a breakdown, and it must not read as a guillotine.
        assert!(!fleet.is_cap_exhaustion(117_599));
        assert!(!fleet.is_cap_exhaustion(3_000));
        assert!(!fleet.is_cap_exhaustion(0));

        // It follows the plafond rather than a literal: at 240 s the same 120 s
        // elapsed is now early, not late.
        let arch = LlmTimeoutBudget::new(240, 900).expect("valid geometry");
        assert!(!arch.is_cap_exhaustion(120_000));
        assert!(arch.is_cap_exhaustion(240_000));
    }

    // -- AC2: the throughput floor, three tiers --

    #[test]
    fn mika2280_floor_defaults_when_absent_or_blank() {
        assert_eq!(
            parse_output_tokens_per_sec_floor(None),
            DEFAULT_OUTPUT_TOKENS_PER_SEC_FLOOR
        );
        assert_eq!(
            parse_output_tokens_per_sec_floor(Some("")),
            DEFAULT_OUTPUT_TOKENS_PER_SEC_FLOOR
        );
        assert_eq!(
            parse_output_tokens_per_sec_floor(Some("   ")),
            DEFAULT_OUTPUT_TOKENS_PER_SEC_FLOOR
        );
    }

    #[test]
    fn mika2280_floor_defaults_on_zero_or_garbage() {
        assert_eq!(
            parse_output_tokens_per_sec_floor(Some("0")),
            DEFAULT_OUTPUT_TOKENS_PER_SEC_FLOOR
        );
        assert_eq!(
            parse_output_tokens_per_sec_floor(Some("-1")),
            DEFAULT_OUTPUT_TOKENS_PER_SEC_FLOOR
        );
        assert_eq!(
            parse_output_tokens_per_sec_floor(Some("soixante-six")),
            DEFAULT_OUTPUT_TOKENS_PER_SEC_FLOOR
        );
    }

    #[test]
    fn mika2280_floor_accepts_a_valid_override() {
        assert_eq!(parse_output_tokens_per_sec_floor(Some("66")), 66);
        assert_eq!(parse_output_tokens_per_sec_floor(Some(" 66 ")), 66);
    }

    /// The default is below the only throughput ever measured in this repo
    /// (66 tok/s, mika#2296) — the censored-population margin the constant's
    /// doc comment argues for. If someone raises it to the observed value, this
    /// goes red and the reasoning has to be re-read.
    #[test]
    fn mika2280_the_default_floor_stays_under_the_measured_throughput() {
        // `const` so the guard fires at compile time — and so clippy's
        // `assertions_on_constants` reads it as the deliberate pin it is.
        const {
            assert!(
                DEFAULT_OUTPUT_TOKENS_PER_SEC_FLOOR < 66,
                "the observable throughput distribution is censored — the slow calls \
             were killed at the plafond, so the observed floor overestimates the \
             real one (see DEFAULT_OUTPUT_TOKENS_PER_SEC_FLOOR)"
            );
        }
    }

    #[test]
    fn default_is_the_shipped_geometry() {
        let d = LlmTimeoutBudget::default();
        assert_eq!(d.http_timeout_secs(), 120);
        assert_eq!(d.agent_total_timeout_secs(), 300);
        assert!(d.validate().is_ok());
    }
}

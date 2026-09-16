//! Role implementations for model calibration.
//!
//! Each submodule defines a role's scenario suite with synthetic skills
//! and structural assertions.

use mika_common::llm::LlmError;
use mika_common::llm::types::{LlmResponse, LlmStopReason};

use crate::calibration::failure::{FailureClass, classify_failure};
use crate::calibration::role::RoleScenarioResult;

pub mod mika_arch;
pub mod mika_dev;
pub mod mika_orchestrator;
pub mod mika_qa;

/// Output-token budget every calibration scenario runs under (mika#2296 D5).
///
/// **Why this is a constant and not thirty literals.** mika#1665 raised the
/// per-scenario budget from 1000 to 2000 and left a comment at
/// `mika_dev.rs` claiming "parity with the other scenarios (2000)". That parity
/// was held by hand, between literals, and it had already come undone: three
/// scenarios *of that same file* were still at 1000 when mika#2296 counted them,
/// and nothing failed, because no assertion carried the parity. A value held in
/// one place is a value that can be changed in one place.
///
/// **Why 8192 and not 32768.** The gate mika#1190 imposes on every well-known
/// agent model swap was itself running at 2000 output tokens — and a reasoning
/// model burns its thinking out of that same budget, so the gate could not
/// validate *any* reasoning model, whatever the candidate of the day. But
/// calibration fixtures are short briefs: the mika#2296 pre-flight measured a
/// short brief concluding in 6.7 s. This ceiling does not have to absorb the
/// reasoning of a 30 KB plan, only to stop cutting through the reasoning of a
/// fixture. 8192 is a factor of 4 on the previous value and is the historical
/// ceiling of an agent turn — a figure known to surprise no provider.
pub const CALIBRATION_SCENARIO_MAX_TOKENS: u32 = 8192;

/// The single scenario that legitimately needs more than the shared budget
/// (mika#2296 D5): `mika_qa`'s full-verdict-body scenario.
///
/// Its need is measured on the spot, not assumed — a first run at 2000 spent the
/// whole budget on `reasoning_content` and returned empty text, which is exactly
/// the failure class mika#2296 exists to repair. Uniformising it to 8192 would
/// therefore be a regression on the one scenario that had already measured its
/// own requirement.
///
/// **No tracking ticket, deliberately.** An exception is normally temporary debt
/// and earns a tracker; this one is a durable, measured requirement. A ticket
/// reading "converge 12000 and 8192" would close without changing anything. The
/// cleanup is carried instead by the assertion in this module's tests, which
/// fails the day the shared budget catches up — demanding this constant's
/// removal rather than letting it survive as a silent duplicate.
pub const CALIBRATION_QA_VERDICT_BODY_MAX_TOKENS: u32 = 12_000;

/// mika#2296 T5, self-cleaning half — the exception must remain an exception.
///
/// The day the shared budget catches up with it, this constant has no object
/// left: it would survive as a silent duplicate, and every behavioural
/// assertion would stay green while two names described one value. This is what
/// replaces the tracking ticket the Fire-Disposition option normally requires —
/// a ticket reading "converge 12000 and 8192" would close without changing
/// anything, whereas this depends on nobody's vigilance.
///
/// It is a compile-time assertion rather than a `#[test]` deliberately: the
/// condition is a pure relation between two constants, so the failure belongs
/// where it cannot be run past — the build. If it fires: delete
/// `CALIBRATION_QA_VERDICT_BODY_MAX_TOKENS`, point mika_qa's verdict-body
/// scenario at `CALIBRATION_SCENARIO_MAX_TOKENS`, and delete this assertion.
const _: () = assert!(
    CALIBRATION_QA_VERDICT_BODY_MAX_TOKENS > CALIBRATION_SCENARIO_MAX_TOKENS,
    "mika#2296: the QA verdict-body budget no longer exceeds the shared scenario budget, \
     so the named exception has no object. Delete the constant, point mika_qa's \
     verdict-body scenario at CALIBRATION_SCENARIO_MAX_TOKENS, and delete this assertion."
);

/// Shared helper: classify an LLM error into a `RoleScenarioResult` failure
/// using `classify_failure` instead of hardcoding `TransportError`.
pub fn llm_error_result(scenario_id: &str, error: LlmError, latency_ms: u64) -> RoleScenarioResult {
    let error_str = error.to_string();
    let failure_class = classify_failure(Some(&error_str), None, None, false);
    RoleScenarioResult::fail(
        scenario_id,
        failure_class,
        error_str,
        None,
        None,
        latency_ms,
    )
}

/// Shared helper: build a failure result for an empty visible response,
/// distinguishing a genuine empty/refusal from reasoning-budget exhaustion
/// (the model consumed its entire output budget on internal reasoning tokens
/// before emitting any visible content). The latter is remediated by raising
/// `max_tokens`, not by reverting a model swap (mika#1665).
pub fn empty_response_result(
    scenario_id: &str,
    response: &LlmResponse,
    latency_ms: u64,
) -> RoleScenarioResult {
    let finish_reason_is_length = response.stop_reason == LlmStopReason::MaxTokens;
    let class = classify_failure(
        None,
        Some(""),
        Some(response.usage.output_tokens),
        finish_reason_is_length,
    );
    let detail = match class {
        FailureClass::ReasoningBudgetExhausted => format!(
            "Reasoning budget exhausted: {} output tokens consumed by internal \
             reasoning before any visible content (finish_reason=length). \
             Raise max_tokens for this scenario.",
            response.usage.output_tokens
        ),
        _ => "Empty response".to_string(),
    };
    RoleScenarioResult::fail(
        scenario_id,
        class,
        detail,
        Some(response.usage.input_tokens),
        Some(response.usage.output_tokens),
        latency_ms,
    )
}

#[cfg(test)]
mod tests {
    /// mika#2296 T5 — no `max_tokens:` literal survives in `calibration/roles/`.
    ///
    /// **Why a source scan and not a value assertion.** The regression this
    /// guards would make no scenario *wrong*: a re-introduced literal would
    /// simply undo the parity D5 just made structural, and every behavioural
    /// assertion in the suites would stay green while the gate silently went
    /// back to being unable to validate a reasoning model. That is precisely the
    /// shape mika#1665's hand-held "parity with the other scenarios (2000)"
    /// comment already failed at — three scenarios of its own file had drifted
    /// to 1000 and nothing noticed, because nothing asserted it.
    ///
    /// Only the two named constants are admitted. The scan reads this crate's
    /// own sources via `include_str!`, so it needs no filesystem access and
    /// cannot silently pass because a path stopped resolving.
    #[test]
    fn mika2296_no_max_tokens_literal_remains_in_calibration_roles() {
        const SOURCES: &[(&str, &str)] = &[
            ("mika_dev.rs", include_str!("mika_dev.rs")),
            ("mika_arch.rs", include_str!("mika_arch.rs")),
            ("mika_qa.rs", include_str!("mika_qa.rs")),
            ("mika_orchestrator.rs", include_str!("mika_orchestrator.rs")),
        ];

        let mut offenders = Vec::new();
        let mut declarations_seen = 0usize;
        for (file, source) in SOURCES {
            for (lineno, line) in source.lines().enumerate() {
                let trimmed = line.trim_start();
                // Comments describe the history; they are not declarations.
                if trimmed.starts_with("//") {
                    continue;
                }
                let Some(rest) = trimmed.strip_prefix("max_tokens:") else {
                    continue;
                };
                declarations_seen += 1;
                let value = rest.trim().trim_end_matches(',').trim();
                if value == "CALIBRATION_SCENARIO_MAX_TOKENS"
                    || value == "CALIBRATION_QA_VERDICT_BODY_MAX_TOKENS"
                {
                    continue;
                }
                offenders.push(format!("{file}:{}: max_tokens: {value}", lineno + 1));
            }
        }

        // A scan that reads nothing passes for the wrong reason. The population
        // was counted at 30 in the tree at `e85a0b46`; the floor is deliberately
        // below that so adding a scenario is not a false failure, but a parser
        // that stopped matching — or a file that left the list — fails here
        // instead of going quietly green.
        assert!(
            declarations_seen >= 30,
            "mika#2296: the scan read only {declarations_seen} `max_tokens:` declarations across \
             {} files; 30 were counted when it was written. Either the parser stopped matching \
             the declaration shape or a suite left SOURCES — in both cases this detector is \
             inert, which is worse than red.",
            SOURCES.len()
        );

        assert!(
            offenders.is_empty(),
            "mika#2296: calibration scenarios must read a named budget constant, not a \
             literal — a literal re-desynchronises the parity D5 made structural, and no \
             behavioural assertion would catch it. Offending sites:\n  {}",
            offenders.join("\n  ")
        );
    }

    /// mika#2296 T5 — the exception is still read by the scenario that owns it.
    ///
    /// Companion to the self-cleaning compile-time assertion beside the two
    /// constants. The scan above admits either name anywhere; this pins WHICH
    /// suite may carry the exception, so a second scenario helping itself to the
    /// 12000 budget is caught rather than silently admitted.
    #[test]
    fn mika2296_the_qa_exception_is_used_by_mika_qa_and_by_nobody_else() {
        const QA: &str = include_str!("mika_qa.rs");
        const OTHERS: &[(&str, &str)] = &[
            ("mika_dev.rs", include_str!("mika_dev.rs")),
            ("mika_arch.rs", include_str!("mika_arch.rs")),
            ("mika_orchestrator.rs", include_str!("mika_orchestrator.rs")),
        ];

        let uses = |source: &str| {
            source.lines().any(|line| {
                let t = line.trim_start();
                !t.starts_with("//")
                    && t.starts_with("max_tokens:")
                    && t.contains("CALIBRATION_QA_VERDICT_BODY_MAX_TOKENS")
            })
        };

        assert!(
            uses(QA),
            "mika#2296: mika_qa's verdict-body scenario must read \
             CALIBRATION_QA_VERDICT_BODY_MAX_TOKENS — if its need is gone, delete the \
             constant rather than leaving it unused"
        );
        for (file, source) in OTHERS {
            assert!(
                !uses(source),
                "mika#2296: {file} reads CALIBRATION_QA_VERDICT_BODY_MAX_TOKENS. That budget is \
                 the named exception of ONE measured scenario (mika_qa's full verdict body). A \
                 second claimant means either a new measurement worth its own constant, or an \
                 exception being borrowed — neither should pass silently."
            );
        }
    }
}

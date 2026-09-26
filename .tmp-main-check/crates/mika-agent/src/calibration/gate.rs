//! Swap-gate decision logic for the `calibrate` binary (#1701).
//!
//! Before #1701 the gate was *vacuous*: `make calibrate-<role>` exited 0
//! regardless of pass-rate, because the only `exit(1)` lived behind a baseline
//! file that the Makefile pointed at a non-existent path, and the standalone
//! floor-gate was an empty `if` body. A `0` exit told operators "quality
//! verified" when nothing was verified.
//!
//! This module isolates the exit-code *decision* as a pure function so it is
//! unit-testable without a live LLM provider — the binary only wires I/O
//! (running scenarios, printing diagnostics, `std::process::exit`) around it.
//! Testing the decision here, rather than only through a subprocess that needs
//! API keys and a network, is what keeps the gate from silently regressing to
//! vacuous again (AC5).
//!
//! ## Exit-code contract
//!
//! | Code | Meaning |
//! |------|---------|
//! | 0    | Gate passed (100% pass-rate ≥ baseline), or a baseline was established |
//! | 1    | Gate failed (pass-rate below the 100% floor or below baseline), or a failing baseline write was refused |
//! | 2    | Gate not enforceable (baseline absent, missing, or unloadable) |

/// The required pass-rate floor. A swap must clear 100% of its role's scenarios.
///
/// Committed position (#1701): the gate requires a perfect run by default. A
/// baseline established below this floor (via `--force-failing-baseline`) still
/// cannot be *matched* to pass — the floor is absolute, the baseline compare is
/// an additional, never-looser check.
pub const PASS_RATE_FLOOR: f64 = 1.0;

/// Availability of the `--baseline` artifact at gate time.
#[derive(Debug, Clone, PartialEq)]
pub enum BaselineState {
    /// No `--baseline` flag was supplied.
    NotProvided,
    /// `--baseline` supplied but the file does not exist.
    Missing,
    /// `--baseline` file exists but failed to load (e.g. schema drift).
    Unloadable,
    /// Baseline loaded; carries its unweighted pass rate.
    Loaded { pass_rate: f64 },
}

/// The gate's decision. Maps 1:1 to a process exit code via [`GateOutcome::exit_code`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateOutcome {
    /// Run cleared the floor and met/exceeded baseline. Exit 0.
    Pass,
    /// Run fell below the 100% floor. Exit 1.
    FailFloor,
    /// Run cleared the floor but regressed below the baseline. Exit 1.
    ///
    /// Unreachable while [`PASS_RATE_FLOOR`] is `1.0` (a run at 1.0 cannot be
    /// below a baseline that is itself ≤ 1.0), but kept as an independent,
    /// defense-in-depth check that stays correct if the floor is ever lowered.
    FailBaseline,
    /// Baseline absent/missing/unloadable — the gate cannot enforce anything. Exit 2.
    NotEnforceable,
    /// `--establish-baseline`: current run written as the new baseline. Exit 0.
    BaselineEstablished,
    /// `--establish-baseline` on a failing run without `--force-failing-baseline`:
    /// refused to write a failing baseline (it would poison future compares). Exit 1.
    BaselineRefused,
}

impl GateOutcome {
    /// Process exit code for this outcome.
    pub fn exit_code(&self) -> i32 {
        match self {
            GateOutcome::Pass | GateOutcome::BaselineEstablished => 0,
            GateOutcome::FailFloor | GateOutcome::FailBaseline | GateOutcome::BaselineRefused => 1,
            GateOutcome::NotEnforceable => 2,
        }
    }

    /// Whether this outcome should print as a hard failure (non-zero exit).
    pub fn is_failure(&self) -> bool {
        self.exit_code() != 0
    }
}

/// Decide the gate outcome from a run's unweighted pass rate and the baseline state.
///
/// `current_rate` is the run's unweighted pass rate in `[0.0, 1.0]`.
///
/// - `establish`: `--establish-baseline` was passed (write-a-baseline mode).
/// - `force_failing`: `--force-failing-baseline` was passed (allow writing a
///   sub-floor baseline; only meaningful with `establish`).
pub fn evaluate_gate(
    current_rate: f64,
    baseline: &BaselineState,
    establish: bool,
    force_failing: bool,
) -> GateOutcome {
    if establish {
        // Establish mode bypasses the gate but still refuses to persist a failing
        // baseline by default — a failing baseline silently lowers the bar for
        // every future compare (AC3 F2). `--force-failing-baseline` overrides.
        if current_rate < PASS_RATE_FLOOR && !force_failing {
            return GateOutcome::BaselineRefused;
        }
        return GateOutcome::BaselineEstablished;
    }

    match baseline {
        // No usable baseline ⇒ the gate cannot verify anything. Exit 2 (distinct
        // from a pass-rate failure) so operators can tell "nothing to compare
        // against" apart from "compared and regressed" (AC2).
        BaselineState::NotProvided | BaselineState::Missing | BaselineState::Unloadable => {
            GateOutcome::NotEnforceable
        }
        BaselineState::Loaded { pass_rate } => {
            if current_rate < PASS_RATE_FLOOR {
                GateOutcome::FailFloor
            } else if current_rate < *pass_rate {
                GateOutcome::FailBaseline
            } else {
                GateOutcome::Pass
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_map_correctly() {
        assert_eq!(GateOutcome::Pass.exit_code(), 0);
        assert_eq!(GateOutcome::BaselineEstablished.exit_code(), 0);
        assert_eq!(GateOutcome::FailFloor.exit_code(), 1);
        assert_eq!(GateOutcome::FailBaseline.exit_code(), 1);
        assert_eq!(GateOutcome::BaselineRefused.exit_code(), 1);
        assert_eq!(GateOutcome::NotEnforceable.exit_code(), 2);
    }

    #[test]
    fn passing_run_with_baseline_exits_zero() {
        let outcome = evaluate_gate(1.0, &BaselineState::Loaded { pass_rate: 1.0 }, false, false);
        assert_eq!(outcome, GateOutcome::Pass);
        assert_eq!(outcome.exit_code(), 0);
    }

    #[test]
    fn sub_floor_run_exits_one() {
        // The core #1701 defect: 0.6 < 1.0 must NOT exit 0.
        let outcome = evaluate_gate(0.6, &BaselineState::Loaded { pass_rate: 1.0 }, false, false);
        assert_eq!(outcome, GateOutcome::FailFloor);
        assert_eq!(outcome.exit_code(), 1);
    }

    #[test]
    fn missing_baseline_exits_two_not_zero() {
        // Defect 1: the shipped Makefile pointed at a non-existent baseline and
        // the binary warned + exited 0. It must now exit 2.
        for state in [
            BaselineState::NotProvided,
            BaselineState::Missing,
            BaselineState::Unloadable,
        ] {
            let outcome = evaluate_gate(1.0, &state, false, false);
            assert_eq!(outcome, GateOutcome::NotEnforceable, "state {state:?}");
            assert_eq!(outcome.exit_code(), 2);
        }
    }

    #[test]
    fn establish_baseline_on_passing_run_writes_and_exits_zero() {
        let outcome = evaluate_gate(1.0, &BaselineState::NotProvided, true, false);
        assert_eq!(outcome, GateOutcome::BaselineEstablished);
        assert_eq!(outcome.exit_code(), 0);
    }

    #[test]
    fn establish_baseline_refuses_failing_run_by_default() {
        let outcome = evaluate_gate(0.8, &BaselineState::NotProvided, true, false);
        assert_eq!(outcome, GateOutcome::BaselineRefused);
        assert_eq!(outcome.exit_code(), 1);
    }

    #[test]
    fn establish_baseline_force_writes_failing_run() {
        let outcome = evaluate_gate(0.8, &BaselineState::NotProvided, true, true);
        assert_eq!(outcome, GateOutcome::BaselineEstablished);
        assert_eq!(outcome.exit_code(), 0);
    }

    #[test]
    fn floor_dominates_a_sub_floor_baseline() {
        // Even against a (forced) failing baseline of 0.8, a 0.9 run is still
        // below the 1.0 floor and must fail.
        let outcome = evaluate_gate(0.9, &BaselineState::Loaded { pass_rate: 0.8 }, false, false);
        assert_eq!(outcome, GateOutcome::FailFloor);
    }
}

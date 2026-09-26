//! Single reader of a team run's terminal outcome, for both CLI surfaces.
//!
//! `mika ask --team` and `mika chat --team` ask the same question — did this run
//! succeed, fail, or not finish? — and used to answer it separately. That
//! duplication is *how* both of them missed the same two variants
//! independently: `FailedNoDelegation` (mika#1676) and `FailedTransport`
//! (mika#1671) were added to the engine and neither hand-written CLI predicate
//! followed. Same shape as `grooming_marker` (mika#2158) and `live_pilot`
//! (mika#2279): *a resolver written twice is a resolver that can contradict
//! itself.*
//!
//! The classification itself names **no** `RunStatus` variant (mika#1940 D2) —
//! it reads `RunStatus::disposition()`, the one exhaustive match, and a seventh
//! variant therefore fails to compile there and nowhere here. That is what lets
//! the structural guard below ship with an empty allowlist: there is nothing to
//! exempt, so there is no slot in which to drop the next violation.

use mika_agent::teams::types::{RunDisposition, TeamRun};

/// What the CLI should do with a finished (or unfinished) team run.
///
/// Four variants though two of them share a disposition (`exit 0`, a note on
/// `stderr`): they are two different facts, and the classifier has to stay
/// readable and testable variant by variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TeamOutcome {
    /// Terminal success carrying a deliverable — the only case that may write
    /// to `stdout`.
    Delivered { text: String },
    /// Terminal success with nothing to deliver.
    CompletedWithoutDeliverable,
    /// Terminal failure. `partial` is whatever text the run had produced before
    /// failing — it is **not** a deliverable and must never reach `stdout`.
    Failed {
        diagnostic: String,
        partial: Option<String>,
    },
    /// Not terminal: still running, or suspended awaiting callbacks.
    Pending { note: String },
}

/// Classify a finished `run_team` call.
///
/// A non-terminal status is reported as [`TeamOutcome::Pending`] rather than
/// treated as either success or failure. `run_team` is not believed to return
/// one (every `execute` path sets a terminal status before returning), but *a
/// signal that cannot be read is never a satisfied term*: the cost of a
/// `Running` actually being observed is a silent run at exit 0 — today's
/// behaviour — never a printed false success.
pub fn classify(run: &TeamRun) -> TeamOutcome {
    match run.status.disposition() {
        RunDisposition::NotTerminal => TeamOutcome::Pending {
            // `Display` is itself an exhaustive match, so this note names the
            // status without this module enumerating anything.
            note: format!("team run did not reach a terminal state ({})", run.status),
        },
        RunDisposition::Failure { reason } => TeamOutcome::Failed {
            diagnostic: reason.to_string(),
            partial: run.deliverable.clone(),
        },
        RunDisposition::Success => match run.deliverable {
            Some(ref text) => TeamOutcome::Delivered { text: text.clone() },
            None => TeamOutcome::CompletedWithoutDeliverable,
        },
    }
}

/// Process exit code for an outcome: `1` on a terminal failure, `0` otherwise.
///
/// **All team failures exit `1`, including transport ones.** The tempting move
/// is to route [`TeamOutcome::Failed`] built from `FailedTransport` to `75`
/// (`EXIT_TRANSPORT_FAILURE`), which `remote_ask` already uses and which
/// `_arch_ask_with_retry` retries on. Refused: that retry budget is sized for a
/// single `mika ask`, not for a whole team cycle (decompose → execute → review →
/// deliver) whose automatic retry costs minutes and several LLM calls. Putting
/// the "team run" population inside a budget designed for another one is a
/// coupling nothing asks for (mika#1940, scope-out (c)).
///
/// Kept as a pure function so the decision is testable and `process::exit` stays
/// a one-line edge — the same split as `remote_ask::exit_code_for`.
pub fn exit_code(outcome: &TeamOutcome) -> i32 {
    match outcome {
        TeamOutcome::Failed { .. } => 1,
        TeamOutcome::Delivered { .. }
        | TeamOutcome::CompletedWithoutDeliverable
        | TeamOutcome::Pending { .. } => 0,
    }
}

/// Marker introducing a failed run's partial text on `stderr`.
///
/// Named rather than inlined because both surfaces emit it and it is what tells
/// a reader that the text below is *not* an answer.
pub const PARTIAL_OUTPUT_MARKER: &str =
    "--- partial output from the failed run (not a deliverable) ---";

#[cfg(test)]
mod tests {
    use super::*;
    use mika_agent::teams::types::{NO_DELEGATION_REASON, RunStatus};

    fn make_run(status: RunStatus, deliverable: Option<String>) -> TeamRun {
        TeamRun {
            run_id: "test-run-1".to_string(),
            team_name: "alpha".to_string(),
            goal: "test goal".to_string(),
            status,
            iteration: 1,
            max_iterations: 3,
            tasks: vec![],
            started_at: "2026-09-20T12:00:00Z".to_string(),
            ended_at: Some("2026-09-20T12:05:00Z".to_string()),
            deliverable,
            coverage_retry_fired: false,
            conversational_retry_fired: false,
            delegation_count: 0,
            solo_absorption: false,
            failure_context: None,
        }
    }

    /// Every terminal failure exits non-zero — **and** every non-failure does
    /// not.
    ///
    /// The negative control is mandatory, not decoration: without it, "the
    /// classifier decides" would be indistinguishable from "the classifier
    /// always fails". This is AC4's property, expressed on the pure function
    /// that owns it (see the plan's Verification Contract § AC4 for why a real
    /// `mika ask --team` run cannot be the test).
    #[test]
    fn mika1940_every_terminal_failure_exits_non_zero() {
        let failures = [
            RunStatus::Failed("boom".to_string()),
            RunStatus::FailedNoDelegation,
            RunStatus::FailedTransport("connection reset".to_string()),
        ];
        for status in failures {
            let run = make_run(status.clone(), None);
            assert_eq!(exit_code(&classify(&run)), 1, "{status} must exit non-zero");
        }

        let non_failures = [
            RunStatus::Completed,
            RunStatus::Running,
            RunStatus::Suspended,
        ];
        for status in non_failures {
            let run = make_run(status.clone(), None);
            assert_eq!(exit_code(&classify(&run)), 0, "{status} must exit zero");
        }
    }

    /// AC2 on the pure function: nothing a failed run carries is destined for
    /// `stdout`.
    ///
    /// `stdout` is a CLI's success channel. A failed run must leave it empty, so
    /// `RESULT=$(mika ask --team X "goal")` cannot capture a plausible answer to
    /// a question that failed — the half of the defect an exit code does not
    /// close (mika#1940 D4).
    #[test]
    fn mika1940_a_failed_run_leaves_stdout_empty() {
        let statuses = [
            RunStatus::Failed("boom".to_string()),
            RunStatus::FailedNoDelegation,
            RunStatus::FailedTransport("connection reset".to_string()),
        ];
        for status in statuses {
            let run = make_run(status.clone(), Some("plausible answer".to_string()));
            match classify(&run) {
                TeamOutcome::Failed { partial, .. } => {
                    assert_eq!(
                        partial.as_deref(),
                        Some("plausible answer"),
                        "the partial text is kept, but in the field that goes to stderr"
                    );
                }
                other => panic!("{status} classified as {other:?}, expected Failed"),
            }
        }
    }

    /// The founding regression: a failure that *carries a deliverable*.
    ///
    /// `engine.rs` sets `deliverable = Some(retry_reply)` on the very line that
    /// sets `FailedNoDelegation`, so the failing run always has text. That is
    /// what made `ask.rs`'s `if let Some(deliverable)` branch win over its error
    /// branch, and what made the TUI persist the text as an `assistant` turn.
    /// This is the exact shape the engine produces.
    #[test]
    fn mika1940_a_failure_carrying_a_deliverable_is_still_a_failure() {
        let run = make_run(
            RunStatus::FailedNoDelegation,
            Some("Sure, here is how I would approach that…".to_string()),
        );

        assert_eq!(
            classify(&run),
            TeamOutcome::Failed {
                diagnostic: NO_DELEGATION_REASON.to_string(),
                partial: Some("Sure, here is how I would approach that…".to_string()),
            }
        );
        assert_eq!(exit_code(&classify(&run)), 1);
    }

    /// Non-regression on the nominal path.
    #[test]
    fn mika1940_a_completed_run_with_a_deliverable_is_unchanged() {
        let run = make_run(RunStatus::Completed, Some("the answer".to_string()));
        assert_eq!(
            classify(&run),
            TeamOutcome::Delivered {
                text: "the answer".to_string()
            }
        );
        assert_eq!(exit_code(&classify(&run)), 0);

        let empty = make_run(RunStatus::Completed, None);
        assert_eq!(classify(&empty), TeamOutcome::CompletedWithoutDeliverable);
        assert_eq!(exit_code(&classify(&empty)), 0);
    }

    /// **No production code in `mika-cli` may name a `RunStatus` failure
    /// variant.**
    ///
    /// No behavioural test can catch this class. If someone writes a fresh
    /// `matches!(run.status, RunStatus::Failed(_))` tomorrow, every existing
    /// assertion stays green — the decisions already covered remain right, and
    /// it is a *new* path that goes silent. Same family as
    /// `mika2335_no_production_dispatch_transitions_a_parent_without_stamping`
    /// and `mika2205_periodic_scans_do_not_read_the_pat_field_directly`.
    ///
    /// **The allowlist is empty and stays empty.** `classify` reads
    /// `disposition()` and names nothing, so no site needs exempting — and an
    /// allowlist born empty is a slot for the next lapse (mika#2323 says so in
    /// as many words). When this test reddens, the resolution is to go through
    /// `disposition()`, never to add an entry.
    #[test]
    fn mika1940_no_hand_written_run_status_predicate_in_the_cli() {
        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));

        let forbidden = [
            "RunStatus::Failed",
            "RunStatus::FailedNoDelegation",
            "RunStatus::FailedTransport",
        ];

        let mut offenders: Vec<String> = Vec::new();
        let mut saw_the_classifier = false;

        scanner.for_each(|path, production| {
            // Good-faith control: the production slice this guard reads must
            // actually contain the classifier. A broken slicing would otherwise
            // make the guard green for the wrong reason — it would be reading
            // nothing (model: `test_promotion_gate_never_resolves_conflicts`).
            if path.ends_with("commands/team_outcome.rs") && production.contains("pub fn classify")
            {
                saw_the_classifier = true;
            }

            for (lineno, line) in production.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                if forbidden.iter().any(|pat| line.contains(pat)) {
                    offenders.push(format!("{}:{}", path.display(), lineno + 1));
                }
            }
        });

        assert!(
            saw_the_classifier,
            "the guard did not find `pub fn classify` in this module's production \
             slice — the scan is broken, not clean"
        );
        assert!(
            offenders.is_empty(),
            "production code in mika-cli names a RunStatus failure variant at {offenders:?} — \
             go through `RunStatus::disposition()` and `team_outcome::classify()`; do NOT add \
             an allowlist entry, the point of mika#1940 is that there is nothing to exempt"
        );
    }
}

//! # mika-manager Phase 1 — milestone-scope operational coordinator
//!
//! **LECTURE SEULE.** This module reads milestone state (via `gh` CLI), assesses it
//! (rules-driven recommendations), and reports structured Markdown to Prime→sami→Vincent.
//! It has **zero write authority**: no dispatch, no ticket label mutation, no PR merge,
//! no scope approval. The only outbound side effect is a report `POST` to a well-known
//! delivery endpoint (Prime→sami→Vincent per D8 subsystem-2 pattern), or an offline sink
//! write when the URL is unset.
//!
//! The chain of authority is Prime → Manager → Executors. `mika-manager` is a distinct
//! entity from `mika-prime`; see the ratified brief at
//! `mika-platform/docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md`
//! for the founding design (5 verdicts Prime + Vincent GO 2026-08-21).
//!
//! ## Cadence contract
//!
//! Hybrid: event-driven (state change detection) + 6h plancher heartbeat
//! ("l'absence d'event EST l'event"). Both configurable via env; the offline sink is
//! always used when delivery URLs are unset so nothing is lost during bring-up.
//!
//! ## Wrapper-only INV-2
//!
//! Every `gh` call mirrors the shape of `crate::auto_pull::gh_list_open_issues`
//! (`tokio::process::Command`, env `GH_TOKEN`, `stdin::null`, `kill_on_drop(true)`).
//! No new GitHub API client dep. No `gh api PATCH/POST/DELETE`. See
//! `no_dispatch_test.rs` for the structural enforcement.
//!
//! ## Phase 2 promotion (NOT wired here)
//!
//! Phase 2 (dispatch authority — recommend + auto-execute) is gated behind the three
//! portes documented in the brief § 3: forge-gate loop-résistance (Porte 1), contention
//! exec (Porte 2), and `INTERNAL_TOKEN` alignment (Porte 3). None of that surface is
//! wired in this module.
//!
//! **Porte 1 discharge condition (mika#1947).** Dispatch authority is DECISION-CORE by
//! construction, so the forge-gate perimeter classifier must fail-closed to
//! `DecisionCore` on every path under `crates/mika-agent/src/milestone_manager/**`, at
//! every merge-authority callsite. Today that holds only via the classifier's
//! fail-closed default — no rule in `perimeter/rules.rs` names this module. The tests
//! below make it hold structurally, so a future diff that adds the manager surface to a
//! MECHANICAL table fails a test instead of quietly opening an auto-merge path into the
//! manager's own code. In-tree proof:
//!
//! - `crates/mika-agent/src/perimeter/tests.rs` —
//!   `milestone_manager_files_are_decision_core`,
//!   `milestone_manager_solo_pr_is_decision_core`,
//!   `milestone_manager_file_taints_pr_batch`,
//!   `milestone_manager_prefix_not_in_mechanical_tables`,
//!   `milestone_manager_absent_from_all_mechanical_tables`,
//!   `milestone_manager_has_no_nested_tests_directory` (AC1 + AC2).
//! - `crates/mika-agent/tests/eval/test_verdict_handler.rs` —
//!   `verdict_pass_milestone_manager_pr_holds_for_operator` (AC3).
//! - `crates/mika-agent/tests/eval/test_ci_success_handler.rs` —
//!   `ci_success_milestone_manager_pr_holds_for_operator` (AC4).
//! - `crates/mika-agent/tests/eval/manager_loop_resistance.rs` —
//!   `cascade_never_dispatches_into_milestone_manager` (AC5, gated `#[ignore]` +
//!   `MIKA_MANAGER_LOOP_RESISTANCE_TEST=1`).
//!
//! `milestone_manager_has_no_nested_tests_directory` is the one to read before adding
//! files here: `MECHANICAL_CONTAINS` grants MECHANICAL to any path containing
//! `/tests/`, so a `src/milestone_manager/tests/` directory would be auto-mergeable
//! inside the very module Porte 1 gates. The manager's own test code belongs beside the
//! module files, as `no_dispatch_test.rs` does.
//!
//! ### Porte 3 — `INTERNAL_TOKEN` alignment (mika#1949)
//!
//! **Discharge condition:** every cross-boundary authentication failure on the
//! manager write path is *distinguishable* — the operator can tell which of the
//! four tokens failed, at which boundary, and why, from the report and from
//! `audit_events`, without reading a log. It is **not** that the four tokens
//! become one; unification is deferred to a Phase 3 refactor.
//!
//! What Porte 3 does **not** ask for: that a write proceed past a failed
//! credential. Fail-closed is the property being protected. What changes is
//! that the closure is now named instead of silent.
//!
//! **Status: NOT discharged.** The `gateway_to_spirit` boundary
//! (`crate::server::auth`) is not yet instrumented — that unit edits
//! authentication-decision code and is blocked pending operator review. The
//! other three boundaries are. See `senara-solutions/mika#1949`.
//!
//! **Post-deploy verification** — after a cadence cycle:
//!
//! ```sql
//! SELECT target_key, after_value, count(*)
//! FROM audit_events
//! WHERE tool_name = 'auth_boundary'
//! GROUP BY 1, 2;
//! ```
//!
//! `target_key` is `<from>_to_<to>`; `after_value` is the failure kind
//! (`missing`/`empty`/`invalid`/`rejected`/`unreachable`). Row shape documented
//! in `crates/mika-agent/CLAUDE.md § Audit Log`; operator rotation procedure in
//! `mika-platform/docs/operator/token-rotation-procedure.md`.
//!
//! **The role question is settled and does not need re-deriving.** The manager
//! authenticates to cm as `Role::CommandPlane`. No `Role::Manager` variant was
//! created: `/messages/dispatch` was already permitted to the command plane, and
//! `/permission-events` is deliberately refused to it (an audit corpus a caller
//! could otherwise forge). The manager's audit trail is mika's own
//! `audit_events`, never cm's. Pinned by `manager_write_path_needs_no_new_role`
//! in `control-monitor/backend/crates/cm-api/src/scope.rs`.

pub mod assessor;
pub mod cadence;
pub mod reader;
pub mod reporter;
pub mod spawn;
pub mod types;

#[cfg(test)]
mod no_dispatch_test;

pub use assessor::{Assessor, AssessorConfig};
pub use cadence::{
    CycleContext, CycleKind, DeliveryBody, DeliveryError, DeliveryFailureKind, HttpReportDeliverer,
    ManagerConfig, MilestoneCheckpoint, ReportDeliverer, run_manager_cycle, run_manager_cycle_in,
    run_manager_cycle_with, run_manager_cycle_with_auth, state_digest,
};
pub use reader::{GhRunner, ProcessGhRunner, Reader, compose_from_gh_outputs};
pub use reporter::{AuthBoundaryNote, AuthBoundaryTracker, Reporter};
pub use spawn::{
    AuthAlarmBody, AuthAlarmSink, HttpAuthAlarmSink, SettingsTokenResolver, TokenResolver,
    manager_config_from_env, spawn_manager_cycle_task,
};
pub use types::{
    Alert, AlertKind, Assessment, CiState, CycleOutcome, IssueState, MilestoneRef, MilestoneState,
    ProgressCounts, RecentActivity, Recommendation, Severity, SubIssue,
};

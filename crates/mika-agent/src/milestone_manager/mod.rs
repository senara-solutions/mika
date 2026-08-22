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
//! portes documented in the brief § 3: forge-gate loop-résistance, contention exec, and
//! `INTERNAL_TOKEN` alignment. None of that surface is wired in this module.

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
    CycleKind, DeliveryBody, HttpReportDeliverer, ManagerConfig, MilestoneCheckpoint,
    ReportDeliverer, run_manager_cycle, run_manager_cycle_with, state_digest,
};
pub use reader::{GhRunner, ProcessGhRunner, Reader, compose_from_gh_outputs};
pub use reporter::Reporter;
pub use spawn::{manager_config_from_env, spawn_manager_cycle_task};
pub use types::{
    Alert, AlertKind, Assessment, CiState, CycleOutcome, IssueState, MilestoneRef, MilestoneState,
    ProgressCounts, RecentActivity, Recommendation, Severity, SubIssue,
};

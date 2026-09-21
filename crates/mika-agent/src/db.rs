pub mod kg_schema;
/// L'échelle de migrations de schéma (mika#2321). Module privé : rien n'en sort
/// hors de `db`, et `migrate` y est `pub(super)` pour les deux seuls appelants
/// qui restent ici, `open` et `open_in_memory`.
mod migrations;
pub mod operational;
pub mod tasks;

use anyhow::{Context, Result};
use chrono::{Duration, TimeZone, Utc};
use chrono_tz::Tz;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Once;
use tracing::{debug, info, warn};
use utoipa::ToSchema;

pub use crate::evidence::AuditEvent;
pub use crate::task_state::tasks::*;
use crate::timestamp;

/// Register sqlite-vec as an auto-extension so every new connection gets vec0.
pub fn init_sqlite_vec() {
    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(
            #[allow(clippy::missing_transmute_annotations)]
            std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ()),
        ));
    });
}

pub const CURRENT_SCHEMA_VERSION: i64 = 54;

/// `(target_key, before_value, after_value, reasoning)` — return shape for
/// [`Database::get_audit_event_rows_by_tool_name`] and its async wrapper.
/// Test-only helper for mika#1712 integration tests; extracted to a type
/// alias to satisfy `clippy::type_complexity`.
#[doc(hidden)]
pub type AuditEventRowTuple = (String, Option<String>, Option<String>, Option<String>);

/// mika#1742 Problem B: refuse-to-zombie grace window for
/// [`Database::create_recurring_task_if_absent`]. Recent same-label recurring
/// rows in `'failed' | 'cancelled' | 'expired'` states within this window
/// block fresh registration and log a WARN. Grace elapses → next startup
/// re-registers automatically. Tunable at compile-time; expose via env var
/// only if operator experience surfaces a real need.
pub const RECURRING_ZOMBIE_GRACE_HOURS: u32 = 24;

/// SQLite `strftime` modifier form of [`RECURRING_ZOMBIE_GRACE_HOURS`]. Kept
/// as a `&str` const so the query stays a bindable parameter.
pub const RECURRING_ZOMBIE_GRACE_SQL: &str = "-24 hours";

/// mika#2271: JSON path of the marker written into a recurring task's
/// `metadata` when a *config-driven* cancel is reverted — i.e. the config that
/// disabled the task (a knob like `MIKA_DEV_AUTO_PULL=0`, or an
/// `identity.toml` toggle) is gone and the boot path re-declares the task as
/// wanted.
///
/// A row carrying this marker is invisible to the mika#1742 refuse-to-zombie
/// guard: a cancel the operator has explicitly reverted is not a terminal
/// failure and must not block re-registration. `failed` / `expired` rows keep
/// blocking — that is the guard's actual purpose.
///
/// Load-bearing: bound as a parameter by both
/// [`Database::revert_config_cancel_recurring_task`] (writer) and
/// [`Database::create_recurring_task_if_absent`] (reader), so the two SQL
/// statements cannot drift apart.
pub const RECURRING_CONFIG_CANCEL_REVERTED_PATH: &str = "$.config_cancel_reverted";

/// mika#2337: JSON path of the marker written into a recurring task's
/// `metadata` when it died because the running binary could not route its
/// trigger ([`crate::task_engine::DispatchError::UnknownTrigger`]).
///
/// **Why this death is not like the others.** The mika#1742 refuse-to-zombie
/// guard exists because a recurring task that kills the system must not re-arm
/// itself on every restart. For this class the premise does not hold: the
/// binary that re-registers a trigger name is, by construction, the binary that
/// carries the match arm for it — the registration literal and the arm are two
/// literals of the same build (mika#2337 F1+F3, made structural by the
/// registered-triggers ↔ match-arms guard in `tests/eval`). So the death cannot
/// predict the next one, and the veto it arms only prolongs the outage it was
/// meant to contain: **every restart inside the 24 h window, including one with
/// the correct binary, refuses to re-register.**
///
/// Only a death whose cause is the `UnknownTrigger` *variant* carries this
/// marker — never a substring match on a rendered message. Any other cause of
/// death keeps arming the veto; mika#1742 is not disarmed in general.
pub const RECURRING_UNKNOWN_TRIGGER_PATH: &str = "$.unknown_trigger_death";

/// mika#2337: JSON path of the marker that records a veto lift was **spent**
/// on this `(agent_id, label)` — the self-cleaning half of the exemption.
///
/// Written by [`Database::create_recurring_task_if_absent`] on the row whose
/// [`RECURRING_UNKNOWN_TRIGGER_PATH`] marker it just honoured, in the same
/// statement that removes that marker. While a row carrying this marker is
/// still inside the [`RECURRING_ZOMBIE_GRACE_HOURS`] window, the exemption is
/// disarmed: a **second** unknown-trigger death on the same label meets a fully
/// armed veto.
///
/// Load-bearing. Without it, each death would write a fresh marker on a fresh
/// row and absolve itself, and a binary that genuinely re-registers a trigger
/// it cannot route would loop for ever — reopening the exact hole mika#1742
/// closed. The lift buys **one** restart, not immunity.
pub const RECURRING_UNKNOWN_TRIGGER_LIFT_CONSUMED_PATH: &str = "$.unknown_trigger_lift_consumed";

/// mika#2446: JSON path of the marker an **operator** writes on a dead
/// recurring row through `mika tasks rearm <label>` — the explicit, traced
/// counterpart of the automatic exemptions above.
///
/// A row carrying this marker is invisible to the mika#1742 refuse-to-zombie
/// guard, exactly like [`RECURRING_CONFIG_CANCEL_REVERTED_PATH`]. The lift is
/// **per-row**: it absolves the deaths that existed when the operator acted,
/// never a later one — a fresh death is a fresh row without the marker and
/// meets a fully armed veto. The terminal status is not rewritten (the death
/// stays a dated fact) and `updated_at` is not touched, so the marker ages out
/// of the grace window together with the row and leaves no debt.
///
/// Load-bearing: bound as a parameter by
/// [`Database::mark_recurring_operator_rearm`] (writer),
/// [`Database::create_recurring_task_if_absent`] (the guard) and
/// [`Database::list_recurring_registry`] (its `zombie_veto_active` mirror).
pub const RECURRING_OPERATOR_REARM_PATH: &str = "$.operator_rearm";

/// SQL for the unified_timeline VIEW — cross-subsystem event correlation.
/// Used in both clean-slate schema creation and incremental migration.
const UNIFIED_TIMELINE_VIEW_SQL: &str = "\
    CREATE VIEW IF NOT EXISTS unified_timeline AS \
    SELECT trace_id, session_id, agent_id, 'message' AS event_type, \
        role AS event_subtype, \
        CASE WHEN length(content) > 200 THEN substr(content, 1, 200) || '...' \
             ELSE content END AS summary, \
        created_at \
    FROM messages \
    UNION ALL \
    SELECT trace_id, session_id, agent_id, 'audit' AS event_type, \
        tool_name AS event_subtype, \
        target_key || ': ' || COALESCE(before_value, '(none)') || ' -> ' || COALESCE(after_value, '(none)') AS summary, \
        created_at \
    FROM audit_events \
    UNION ALL \
    SELECT COALESCE(execution_trace_id, created_trace_id) AS trace_id, created_by_session AS session_id, agent_id, \
        'task' AS event_type, action_type AS event_subtype, \
        label || ' [' || status || ']' AS summary, \
        created_at \
    FROM tasks \
    UNION ALL \
    SELECT trace_id, 'team-' || run_id AS session_id, NULL AS agent_id, \
        'team_workspace' AS event_type, entry_type AS event_subtype, \
        CASE WHEN length(content) > 200 THEN substr(content, 1, 200) || '...' \
             ELSE content END AS summary, \
        created_at \
    FROM team_workspace \
    UNION ALL \
    SELECT trace_id, session_id, agent_id, 'llm_call' AS event_type, \
        provider || '/' || model AS event_subtype, \
        'tokens: ' || input_tokens || ' -> ' || output_tokens || ' (' || latency_ms || 'ms, ' || status || ')' AS summary, \
        created_at \
    FROM llm_calls \
    UNION ALL \
    SELECT trace_id, session_id, agent_id, 'tool_call' AS event_type, \
        tool_name AS event_subtype, \
        CASE WHEN success THEN 'ok' ELSE 'err' END || ' (' || latency_ms || 'ms)' AS summary, \
        created_at \
    FROM tool_calls";

/// Check if an anyhow error is a SQLite UNIQUE constraint violation.
pub fn is_unique_violation(err: &anyhow::Error) -> bool {
    if let Some(rusqlite::Error::SqliteFailure(e, _)) = err.downcast_ref::<rusqlite::Error>() {
        e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    } else {
        false
    }
}

pub const COMMITMENT_STATUSES: &[&str] = &["pending", "completed", "cancelled"];

pub const CORE_MEMORY_SECTIONS: &[(&str, &str)] = &[
    ("user_summary", "No information about the user yet."),
    ("self_model", "No interaction history yet."),
    ("current_priorities", "No priorities set yet."),
    ("key_people", "No people tracked yet."),
    (
        "workflows",
        "Delegate-then-forget is not allowed. Any work sent to Claude Code must have a \
         corresponding task created first (via create_task). No exceptions.",
    ),
];

pub fn core_memory_section_names() -> Vec<&'static str> {
    CORE_MEMORY_SECTIONS.iter().map(|(k, _)| *k).collect()
}

/// Returns the default self_model string for a given agent display name.
/// Centralises the format so callers don't duplicate the template.
pub fn default_self_model(display_name: &str) -> String {
    format!("I am {display_name}. No interaction history yet.")
}

// ===== Public Types =====

/// The dispatch that currently holds an exec-slot, as reported by
/// [`Database::has_active_callback_tasks_excluding`] (mika#1948).
///
/// Replaces the previous bare `(String, String, String)` tuple. The fourth
/// field is the reason for the change: a rejection that cannot name WHICH
/// dispatcher holds the slot leaves an operator guessing at a throughput
/// bottleneck.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockingDispatch {
    /// The task whose dispatch occupies the slot.
    pub parent_task_id: String,
    /// The active callback child that makes the occupancy observable.
    pub callback_task_id: String,
    /// That callback's label — also how `blocker_kind` is derived (mika#1172).
    pub label: String,
    /// Which dispatcher initiated the blocking task. `None` for a row written
    /// before the column existed (pre-v51): genuinely unknown, and deliberately
    /// not defaulted here. See mika#1948.
    pub dispatcher_source: Option<String>,
}

/// Outcome of an atomic exec-slot claim (mika#1948).
///
/// See [`Database::try_acquire_dispatch_slot`] for why claiming — rather than
/// checking — is what makes the arbitration real.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotClaim {
    /// The caller now holds the slot and may dispatch.
    Acquired,
    /// Another live lease holds the slot. The caller must not dispatch.
    Held {
        holder_task_id: String,
        dispatcher_source: Option<String>,
        expires_at: String,
    },
}

impl SlotClaim {
    /// Whether the claim succeeded.
    pub fn acquired(&self) -> bool {
        matches!(self, SlotClaim::Acquired)
    }
}

/// A live declaration that some actor owns the worktree of `(repo, issue)`
/// (mika#2192).
///
/// "Live" is a property of the row, not of the reader: every accessor filters
/// on `expires_at > now`, so a `WorktreeClaim` in hand is by construction one
/// that has not lapsed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WorktreeClaim {
    pub repo: String,
    pub issue_number: i64,
    /// `pilot` | `orchestrator` | `spawn`. CHECK-pinned at the table.
    pub owner_kind: String,
    /// The dispatch `task_id`, the orchestrator session id, or `MIKA_SPAWN_ID`.
    pub owner_id: String,
    /// Free text an operator (and the mika#1282 rescue path) can read.
    pub owner_label: Option<String>,
    pub claimed_at: String,
    pub expires_at: String,
}

/// Outcome of a worktree claim (mika#2192). Mirrors [`SlotClaim`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeClaimOutcome {
    /// The caller now owns the worktree. Also the answer to an idempotent
    /// re-claim by the same `owner_id` — a refresh, never a deadlock against
    /// oneself.
    Claimed(WorktreeClaim),
    /// Another live owner holds it. The caller must not enter the directory.
    HeldByOther(WorktreeClaim),
}

/// Default lifetime of a worktree claim held by an interactive actor
/// (orchestrator session or spawn), in seconds (mika#2192).
///
/// Two hours, renewable by idempotent re-claim. Deliberately long: an
/// interactive session has no heartbeat, so the TTL is the only thing that can
/// end its claim, and ending it early is the failure that loses work. The cost
/// of erring long is a ticket that re-defers for up to two hours — the safe
/// side, and the same trade `dispatch_slot_leases` has taken since mika#1948.
pub const WORKTREE_CLAIM_INTERACTIVE_TTL_SECS: i64 = 7200;

/// Default lifetime of a worktree claim held by a pilot dispatch, in seconds
/// (mika#2192).
///
/// Six hours — the `timeout_at` panic-fallback the long-running executor
/// already uses for a dispatch subprocess. A pilot claim must outlive the run
/// it guards (unlike an exec-slot lease, which guards only the seconds between
/// validation and the callback row), or the guard would lapse mid-session and
/// let a second writer in on exactly the run it exists to protect.
pub const WORKTREE_CLAIM_PILOT_TTL_SECS: i64 = 21600;

/// Default lifetime of an exec-slot lease, in seconds (mika#1948).
///
/// Sized to comfortably cover the window it guards — validation completing to
/// the callback row existing, i.e. a process spawn — while staying far below
/// the duration of a real dispatch, so a lease can never outlive its work and
/// block a slot that has legitimately freed. Overridable via
/// `MIKA_DISPATCH_SLOT_LEASE_TTL_SECS` for operational tuning.
pub const DISPATCH_SLOT_LEASE_TTL_SECS: i64 = 120;

/// Effective lease TTL, honouring the env override. Falls back to the default
/// on an unset, unparseable, or non-positive value — a zero or negative TTL
/// would make every lease born expired, disabling the guard silently.
pub fn dispatch_slot_lease_ttl_secs() -> i64 {
    std::env::var("MIKA_DISPATCH_SLOT_LEASE_TTL_SECS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DISPATCH_SLOT_LEASE_TTL_SECS)
}

#[derive(Debug, Clone)]
pub struct AgentRow {
    pub id: String,
    pub name: String,
    pub home_dir: String,
    pub active: bool,
    pub last_seen: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct TeamRow {
    pub id: String,
    pub name: String,
    pub config_path: String,
    pub created_at: String,
}

/// One parsed pilot-transcript row awaiting insert (mika#1705). Bodies are
/// secret-scrubbed by the ingestion tick before construction. All fields are
/// optional because a claude-pilot JSONL entry may omit any of them.
#[derive(Debug, Clone, Default)]
pub struct PilotTranscriptRow {
    pub timestamp: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub request_body: Option<String>,
    pub response_body: Option<String>,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub latency_ms: Option<i64>,
}

/// A finished dispatch that was asked for a pilot transcript (mika#2040 AC7).
///
/// Produced by [`Database::find_dispatches_expecting_transcripts`] and consumed
/// by the engine's empty-transcript detector. `expected_path` is the path the
/// executor stamped on the task when it injected `ANTHROPIC_LOG_FILE` — the
/// detector re-reads it rather than recomputing it, so a later change to the
/// directory layout cannot make the detector look for a file at an address the
/// dispatch was never given.
#[derive(Debug, Clone)]
pub struct DispatchExpectingTranscript {
    pub task_id: String,
    pub expected_path: String,
    pub status: String,
    pub updated_at: String,
}

/// A per-(agent, person, category) content-serve ledger row (mika#1867).
/// Populated by the `record_served_content` tool after Mika delivers content
/// (proverb, quote, joke, poem, recommendation, story, fact) to a specific
/// person, so future generations can dedup against exact-match hashes.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServedContent {
    pub id: i64,
    pub agent_id: String,
    pub person_id: i64,
    pub category: String,
    pub content_text: String,
    pub content_hash: String,
    pub content_signature: Option<String>,
    pub served_at: String,
    pub session_id: Option<String>,
}

/// Outcome of a `record_served_content` write (mika#1867). Distinct variants
/// so the caller (the tool) can surface the duplicate-detection path to the
/// LLM as a "retry" signal without polluting the error channel.
#[derive(Debug, Clone, serde::Serialize)]
pub enum RecordOutcome {
    Inserted {
        id: i64,
    },
    Duplicate {
        existing_id: i64,
        prior_served_at: String,
    },
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Session {
    pub id: String,
    pub agent_id: String,
    pub channel_type: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub metadata: Option<String>,
    pub parent_session_id: Option<String>,
    pub task_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SessionMessage {
    pub id: i64,
    pub session_id: String,
    pub agent_id: String,
    pub role: String,
    pub content: String,
    pub channel_type: String,
    pub metadata: Option<String>,
    pub trace_id: Option<String>,
    pub created_at: String,
    /// Whether this message is internal (agent-to-agent) and should be hidden from inbox mode.
    pub internal: bool,
}

/// A message row from the `task_messages` parallel narrative table (mika#974).
/// Compaction-immune — survives `replace_with_summary` indefinitely.
#[derive(Debug, Clone)]
pub struct TaskMessage {
    pub id: i64,
    pub task_id: String,
    pub agent_id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub metadata: Option<String>,
    pub trace_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CoreMemoryEntry {
    pub key: String,
    pub value: String,
    pub token_count: i32,
    pub updated_at: String,
}

/// A structured fact for the dashboard (aggregated from people, commitments, preferences, events).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DashboardFact {
    pub id: i64,
    pub category: String,
    pub key: String,
    pub value: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Person {
    pub id: i64,
    pub canonical_name: String,
    pub relationship: Option<String>,
    pub notes: Option<String>,
    pub first_mentioned: String,
    pub last_mentioned: String,
    pub mention_count: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Commitment {
    pub id: i64,
    pub description: String,
    pub status: String,
    pub due_date: Option<String>,
    pub person_id: Option<i64>,
    pub created_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Preference {
    pub category: String,
    pub value: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Event {
    pub id: i64,
    pub description: String,
    pub event_date: Option<String>,
    pub context: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct FailedSend {
    pub id: i64,
    pub text: String,
    pub request_id: Option<String>,
    pub created_at: String,
    pub retry_count: i32,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: i64,
    pub source_type: String,
    pub source_id: Option<i64>,
    pub content: String,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamRunRow {
    pub id: String,
    pub team_name: String,
    pub goal: String,
    pub status: String,
    pub failure_reason: Option<String>,
    pub iteration: u32,
    pub max_iterations: u32,
    pub deliverable: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub trace_id: Option<String>,
    /// Number of member sessions delegated during the run (mika#1676 Unit B).
    pub delegation_count: u32,
    /// Whether the run completed without delegating to any member (mika#1676).
    pub solo_absorption: bool,
    /// Structured failure context for `failed_no_delegation` runs (JSON phase).
    pub failure_context: Option<String>,
}

// ===== Dashboard Filter Types =====

/// Filters for paginated task listing (dashboard API).
#[derive(Debug, Clone, Default)]
pub struct TaskFilters {
    pub status: Option<String>,
    pub trigger_type: Option<String>,
    pub action_type: Option<String>,
    pub agent_id: Option<String>,
    pub team_run_id_filter: Option<TeamRunIdFilter>,
    pub source: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

/// How to filter tasks by team_run_id.
#[derive(Debug, Clone)]
pub enum TeamRunIdFilter {
    /// team_run_id IS NULL
    Null,
    /// team_run_id IS NOT NULL
    NotNull,
    /// team_run_id = specific value
    Specific(String),
}

/// mika#2446 — the dead recurring row `mika tasks rearm <label>` resurrects.
///
/// Carries what re-registration needs (`cron_expr`, `action_config`) read off
/// the dead row itself, so the operator never retypes a cron, plus the fields
/// that name the death being absolved.
#[derive(Debug, Clone)]
pub struct RecurringRearmTarget {
    pub task_id: String,
    /// The label as stored — the lookup is `COLLATE NOCASE`, re-registration
    /// must use the stored spelling.
    pub label: String,
    pub status: String,
    pub cron_expr: Option<String>,
    pub action_type: String,
    pub action_config: String,
    pub updated_at: String,
}

/// mika#2360 — closed projection of the recurring-task registry.
///
/// Deliberately distinct from [`Task`]: what is not named here cannot reach
/// the HTTP response, whatever the `tasks` table gains later. `action_config`,
/// `result`, `input_context` and `metadata` are excluded by construction —
/// they carry user content (mika#2360 AC4: registry metadata only, never a
/// message body).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RecurringRegistryRow {
    pub label: String,
    pub agent_id: String,
    /// Always `"recurring"` — the filter is hard-wired in the SQL (R2).
    pub trigger_type: String,
    pub action_type: String,
    pub cron_expr: Option<String>,
    pub next_fire_at: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    /// mika#1742: does this row, at read time, arm the refuse-to-zombie veto
    /// against re-registering its `(agent_id, label)`? Computed in SQL with
    /// the same predicate [`Database::create_recurring_task_if_absent`]
    /// applies — the operator does not have to replay the 24 h window and
    /// the three `metadata` markers by hand to read the registry.
    pub zombie_veto_active: bool,
}

/// Filters for paginated team run listing (dashboard API).
#[derive(Debug, Clone, Default)]
pub struct TeamRunFilters {
    pub team_name: Option<String>,
    pub status: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamWorkspaceEntry {
    pub id: i64,
    pub run_id: String,
    pub parent_id: Option<i64>,
    pub agent_name: Option<String>,
    pub entry_type: String,
    pub content: String,
    pub iteration: u32,
    pub created_at: String,
}

// ===== Team Run Summary Types =====

/// Summary of an agent's response from a previous team run.
#[derive(Debug, Clone, Serialize)]
pub struct AgentResultSummary {
    pub agent_name: String,
    pub response_preview: String,
}

/// Summary of a task's status from a previous team run.
#[derive(Debug, Clone, Serialize)]
pub struct TaskStatusSummary {
    pub agent_id: String,
    pub label: String,
    pub status: String,
    pub task_id: String,
}

/// Enriched summary of a previous team run for context injection.
#[derive(Debug, Clone, Serialize)]
pub struct TeamRunSummary {
    pub run: TeamRunRow,
    pub agent_results: Vec<AgentResultSummary>,
    pub task_statuses: Vec<TaskStatusSummary>,
    pub pending_tasks: Vec<TaskStatusSummary>,
    pub critic_feedback: Option<String>,
}

// ===== Skill Override Types =====

/// A user override for a skill property (persists across bundled skill re-sync).
#[derive(Debug, Clone, Default)]
pub struct SkillOverride {
    pub skill_name: String,
    pub always_on: Option<bool>,
    pub llm_provider: Option<String>,
    pub llm_model: Option<String>,
    /// Tri-state: `None` = default (enabled), `Some(false)` = disabled,
    /// `Some(true)` = explicitly enabled.
    pub enabled: Option<bool>,
    /// Lifecycle state for agent-authored skills: `staged`, `active`, `archived`.
    /// `None` for bundled/marketplace skills (they don't go through the lifecycle).
    pub lifecycle_state: Option<String>,
    /// Number of turns this skill was injected into.
    pub use_count: i64,
    /// ISO 8601 timestamp of the last turn this skill was injected into.
    pub last_used_at: Option<String>,
}

// ===== Observability Types =====

#[derive(Debug, Clone, Serialize)]
pub struct LlmCallRow {
    pub id: String,
    pub agent_id: String,
    pub session_id: String,
    pub trace_id: Option<String>,
    pub provider: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub latency_ms: u64,
    pub stop_reason: Option<String>,
    pub status: String,
    pub error_message: Option<String>,
    pub step: u32,
    /// JSON map of skill names to resolved prompt variant descriptors.
    /// `None` when no skills contributed prompts to this turn.
    pub prompt_variant: Option<String>,
    pub created_at: String,
    /// Serialized LLM response text (stripped of internal tags, capped at 50K chars).
    /// `None` for pre-v31 rows or error calls.
    pub response_text: Option<String>,
    /// Extended thinking / reasoning text (Claude-only).
    /// `None` when the provider does not support reasoning or the call errored.
    pub reasoning: Option<String>,
    /// Whether `response_text` is present (non-NULL) in the database.
    /// Set by list queries via `response_text IS NOT NULL`; detail queries derive from presence.
    pub has_response_text: bool,
    /// Whether `reasoning` is present (non-NULL) in the database.
    /// Set by list queries via `reasoning IS NOT NULL`; detail queries derive from presence.
    pub has_reasoning: bool,
    /// Estimated cost in USD, computed from token counts and provider pricing.
    /// Set to `Some` by `enrich_llm_calls_with_cost()` in dashboard handlers;
    /// `None` only for internal (non-API) use before enrichment.
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolCallRow {
    pub id: String,
    pub agent_id: String,
    pub session_id: String,
    pub trace_id: Option<String>,
    pub llm_call_id: Option<String>,
    pub step: u32,
    pub tool_name: String,
    pub tool_source: String,
    pub skill_name: Option<String>,
    pub input: Option<String>,
    pub output: Option<String>,
    pub success: bool,
    pub non_zero_exit: bool,
    pub latency_ms: u64,
    pub error_message: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Default)]
pub struct LlmCallFilters {
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub trace_id: Option<String>,
    pub model: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CostTrendBucket {
    pub timestamp: String,
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub call_count: u64,
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CostTrendResponse {
    pub buckets: Vec<CostTrendBucket>,
    pub bucket_size: String,
    pub has_estimated_pricing: bool,
    pub estimated_models: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CostTrendFilters {
    pub agent_id: Option<String>,
    pub model: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub bucket: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ToolCallFilters {
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub trace_id: Option<String>,
    pub tool_name: Option<String>,
    pub success: Option<bool>,
    pub from: Option<String>,
    pub to: Option<String>,
    /// Keyword search in input/output fields (uses SQL LIKE).
    pub keyword: Option<String>,
}

// ===== Dashboard Types =====

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TimelineRow {
    pub trace_id: Option<String>,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub event_type: String,
    pub event_subtype: String,
    pub summary: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Default)]
pub struct TimelineFilters {
    pub agent_id: Option<String>,
    pub event_type: Option<String>,
    pub trace_id: Option<String>,
    pub session_id: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

impl TimelineFilters {
    /// Build a WHERE clause and parameter values from the filters.
    /// Uses `rusqlite::types::Value` to preserve native types (integers pass as integers).
    pub(crate) fn to_sql(&self) -> (String, Vec<rusqlite::types::Value>) {
        let mut conditions = Vec::new();
        let mut params: Vec<rusqlite::types::Value> = Vec::new();

        if let Some(ref aid) = self.agent_id {
            params.push(rusqlite::types::Value::Text(aid.clone()));
            conditions.push(format!("agent_id = ?{}", params.len()));
        }
        if let Some(ref et) = self.event_type {
            params.push(rusqlite::types::Value::Text(et.clone()));
            conditions.push(format!("event_type = ?{}", params.len()));
        }
        if let Some(ref tid) = self.trace_id {
            params.push(rusqlite::types::Value::Text(tid.clone()));
            conditions.push(format!("trace_id = ?{}", params.len()));
        }
        if let Some(ref sid) = self.session_id {
            params.push(rusqlite::types::Value::Text(sid.clone()));
            conditions.push(format!("session_id = ?{}", params.len()));
        }
        if let Some(ref from) = self.from {
            params.push(rusqlite::types::Value::Text(from.clone()));
            conditions.push(format!("created_at >= ?{}", params.len()));
        }
        if let Some(ref to) = self.to {
            params.push(rusqlite::types::Value::Text(to.clone()));
            conditions.push(format!("created_at <= ?{}", params.len()));
        }

        let clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };
        (clause, params)
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentWithStats {
    pub id: String,
    pub name: String,
    #[serde(skip)]
    pub home_dir: String,
    pub active: bool,
    pub last_seen: Option<String>,
    pub created_at: String,
    pub message_count: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionWithStats {
    pub id: String,
    pub agent_id: String,
    pub channel_type: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub metadata: Option<String>,
    pub task_id: Option<String>,
    pub message_count: i64,
}

/// A session row enriched with task label for the task-sessions endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct TaskSessionRow {
    pub id: String,
    pub agent_id: String,
    pub channel_type: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub task_id: Option<String>,
    pub message_count: i64,
    pub task_label: Option<String>,
}

// ===== v27 coalesce SQL (module-level for integration test access) =====

/// Generate the SQL that coalesces v26 backup table rows into the new v27
/// shared-corpus tables. Executed inside the existing migration transaction.
///
/// `docs_root` must already have single-quotes escaped (`'` → `''`).
/// `docs_root_hash` is the 16-hex-char SHA-256 prefix from `kg::config::hash_docs_root`.
pub fn v27_coalesce_sql(docs_root: &str, docs_root_hash: &str) -> String {
    format!(
        "-- Step 1: temp lookup tables for id remapping.
         -- DROP IF EXISTS first so the coalesce is retryable on the same
         -- connection if a prior attempt failed mid-transaction (TEMP tables
         -- survive ROLLBACK in SQLite — they are session-scoped, not tx-scoped).
         DROP TABLE IF EXISTS chunk_id_map;
         DROP TABLE IF EXISTS subject_entity_id_map;
         DROP TABLE IF EXISTS subject_relationship_id_map;
         CREATE TEMP TABLE chunk_id_map (old_id INTEGER PRIMARY KEY, new_id INTEGER NOT NULL);
         CREATE TEMP TABLE subject_entity_id_map (old_id INTEGER PRIMARY KEY, new_id INTEGER NOT NULL);
         CREATE TEMP TABLE subject_relationship_id_map (old_id INTEGER PRIMARY KEY, new_id INTEGER NOT NULL);

         -- Step 2: INSERT winning chunks (group by source_doc_path, seq_id; take MIN(id))
         INSERT INTO kg_chunks (docs_root_hash, docs_root, seq_id, source_doc_path, source_doc_hash, created_at, trace_id)
         SELECT '{docs_root_hash}', '{docs_root}', seq_id, source_doc_path, source_doc_hash, created_at, trace_id
         FROM kg_chunks_v26_backup
         WHERE id IN (SELECT MIN(id) FROM kg_chunks_v26_backup GROUP BY source_doc_path, seq_id);

         -- Step 3: populate chunk_id_map
         INSERT INTO chunk_id_map (old_id, new_id)
         SELECT b.id, c.id FROM kg_chunks_v26_backup b
         JOIN kg_chunks c ON c.source_doc_path = b.source_doc_path AND c.seq_id = b.seq_id AND c.docs_root_hash = '{docs_root_hash}';

         -- Step 4: INSERT winning subject entities (majority-vote by normalized entity_key)
         INSERT INTO kg_subject_entities (docs_root_hash, docs_root, entity_key, type, name, confidence, properties_json, created_at, trace_id)
         SELECT '{docs_root_hash}', '{docs_root}', entity_key, type, name, confidence, properties_json, created_at, trace_id
         FROM (
             SELECT b.id, b.entity_key, b.type, b.name, b.confidence, b.properties_json, b.created_at, b.trace_id,
                    ROW_NUMBER() OVER (
                        PARTITION BY LOWER(TRIM(b.entity_key))
                        ORDER BY vote_count DESC, avg_conf DESC, b.id ASC
                    ) AS rn
             FROM kg_subject_entities_v26_backup b
             JOIN (
                 SELECT LOWER(TRIM(entity_key)) AS norm_key,
                        COUNT(DISTINCT agent_id) AS vote_count,
                        AVG(confidence) AS avg_conf
                 FROM kg_subject_entities_v26_backup
                 GROUP BY LOWER(TRIM(entity_key))
             ) agg ON LOWER(TRIM(b.entity_key)) = agg.norm_key
         )
         WHERE rn = 1;

         -- Step 5: populate subject_entity_id_map
         INSERT INTO subject_entity_id_map (old_id, new_id)
         SELECT b.id, e.id FROM kg_subject_entities_v26_backup b
         JOIN kg_subject_entities e ON LOWER(TRIM(e.entity_key)) = LOWER(TRIM(b.entity_key)) AND e.docs_root_hash = '{docs_root_hash}';

         -- Step 6: INSERT winning subject relationships (rewire FK ids, then group)
         INSERT INTO kg_subject_relationships (docs_root_hash, docs_root, from_entity_id, to_entity_id, type, confidence, properties_json, created_at, trace_id)
         SELECT '{docs_root_hash}', '{docs_root}', new_from, new_to, type, confidence, properties_json, created_at, trace_id
         FROM (
             SELECT b.id, fm.new_id AS new_from, tm.new_id AS new_to, b.type, b.confidence,
                    b.properties_json, b.created_at, b.trace_id,
                    ROW_NUMBER() OVER (
                        PARTITION BY fm.new_id, tm.new_id, LOWER(TRIM(b.type))
                        ORDER BY vote_count DESC, avg_conf DESC, b.id ASC
                    ) AS rn
             FROM kg_subject_relationships_v26_backup b
             JOIN subject_entity_id_map fm ON fm.old_id = b.from_entity_id
             JOIN subject_entity_id_map tm ON tm.old_id = b.to_entity_id
             JOIN (
                 SELECT fm2.new_id AS nf, tm2.new_id AS nt, LOWER(TRIM(r2.type)) AS norm_type,
                        COUNT(DISTINCT r2.agent_id) AS vote_count,
                        AVG(r2.confidence) AS avg_conf
                 FROM kg_subject_relationships_v26_backup r2
                 JOIN subject_entity_id_map fm2 ON fm2.old_id = r2.from_entity_id
                 JOIN subject_entity_id_map tm2 ON tm2.old_id = r2.to_entity_id
                 GROUP BY fm2.new_id, tm2.new_id, LOWER(TRIM(r2.type))
             ) agg ON agg.nf = fm.new_id AND agg.nt = tm.new_id AND agg.norm_type = LOWER(TRIM(b.type))
         )
         WHERE rn = 1;

         -- Step 7: populate subject_relationship_id_map
         INSERT INTO subject_relationship_id_map (old_id, new_id)
         SELECT b.id, r.id
         FROM kg_subject_relationships_v26_backup b
         JOIN subject_entity_id_map fm ON fm.old_id = b.from_entity_id
         JOIN subject_entity_id_map tm ON tm.old_id = b.to_entity_id
         JOIN kg_subject_relationships r ON r.from_entity_id = fm.new_id
                                         AND r.to_entity_id = tm.new_id
                                         AND LOWER(TRIM(r.type)) = LOWER(TRIM(b.type))
                                         AND r.docs_root_hash = '{docs_root_hash}';

         -- Step 8: INSERT chunk_subjects (rewire both chunk_id and subject_entity_id)
         INSERT OR IGNORE INTO kg_chunk_subjects (docs_root_hash, docs_root, chunk_id, subject_entity_id, extraction_trace_id, created_at)
         SELECT '{docs_root_hash}', '{docs_root}', cm.new_id, em.new_id, b.extraction_trace_id, b.created_at
         FROM kg_chunk_subjects_v26_backup b
         JOIN chunk_id_map cm ON cm.old_id = b.chunk_id
         JOIN subject_entity_id_map em ON em.old_id = b.subject_entity_id;

         -- Step 9: INSERT chunk_subject_relationships (rewire chunk_id and subject_relationship_id)
         INSERT OR IGNORE INTO kg_chunk_subject_relationships (docs_root_hash, docs_root, chunk_id, subject_relationship_id, extraction_trace_id, created_at)
         SELECT '{docs_root_hash}', '{docs_root}', cm.new_id, rm.new_id, b.extraction_trace_id, b.created_at
         FROM kg_chunk_subject_relationships_v26_backup b
         JOIN chunk_id_map cm ON cm.old_id = b.chunk_id
         JOIN subject_relationship_id_map rm ON rm.old_id = b.subject_relationship_id;

         -- Step 10: INSERT extractions (first-writer-wins by MIN(id) per source_doc_path)
         INSERT OR IGNORE INTO kg_extractions (docs_root_hash, docs_root, source_doc_path, source_doc_hash, extraction_model, entities_extracted, relationships_extracted, extraction_trace_id, created_at)
         SELECT '{docs_root_hash}', '{docs_root}', source_doc_path, source_doc_hash, extraction_model, entities_extracted, relationships_extracted, extraction_trace_id, created_at
         FROM kg_extractions_v26_backup
         WHERE id IN (SELECT MIN(id) FROM kg_extractions_v26_backup GROUP BY source_doc_path);

         -- Step 11: INSERT per-agent resolutions (rewire subject_entity_id)
         INSERT OR IGNORE INTO kg_subject_resolutions (agent_id, subject_entity_id, domain_entity_id, confidence, created_at, trace_id)
         SELECT b.agent_id, em.new_id, b.domain_entity_id, b.confidence, b.created_at, b.trace_id
         FROM kg_subject_resolutions_v26_backup b
         JOIN subject_entity_id_map em ON em.old_id = b.subject_entity_id;

         -- Step 12: INSERT per-agent resolutions_log (rewire subject_entity_id)
         INSERT OR IGNORE INTO kg_resolutions_log (agent_id, subject_entity_id, outcome, resolution_trace_id, source_extraction_trace_id, model, duration_ms, resolved_at)
         SELECT b.agent_id, em.new_id, b.outcome, b.resolution_trace_id, b.source_extraction_trace_id, b.model, b.duration_ms, b.resolved_at
         FROM kg_resolutions_log_v26_backup b
         JOIN subject_entity_id_map em ON em.old_id = b.subject_entity_id;

         -- Step 13: DROP the 8 backup tables
         DROP TABLE IF EXISTS kg_chunk_subject_relationships_v26_backup;
         DROP TABLE IF EXISTS kg_chunk_subjects_v26_backup;
         DROP TABLE IF EXISTS kg_subject_relationships_v26_backup;
         DROP TABLE IF EXISTS kg_subject_resolutions_v26_backup;
         DROP TABLE IF EXISTS kg_resolutions_log_v26_backup;
         DROP TABLE IF EXISTS kg_subject_entities_v26_backup;
         DROP TABLE IF EXISTS kg_extractions_v26_backup;
         DROP TABLE IF EXISTS kg_chunks_v26_backup;

         -- Step 14: DROP temp lookup tables
         DROP TABLE IF EXISTS chunk_id_map;
         DROP TABLE IF EXISTS subject_entity_id_map;
         DROP TABLE IF EXISTS subject_relationship_id_map;

         -- Step 15: schema_meta coalesce marker
         INSERT OR IGNORE INTO schema_meta (key, value) VALUES ('v27_coalesce_complete', '1');
        ",
        docs_root_hash = docs_root_hash,
        docs_root = docs_root,
    )
}

// ===== Database =====

pub struct Database {
    pub(crate) conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        init_sqlite_vec();
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open SQLite at {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;
             PRAGMA auto_vacuum = INCREMENTAL;",
        )
        .context("failed to set SQLite pragmas")?;
        let mut db = Self { conn };
        // Auto-backup DB file before any destructive schema migration
        let current_ver = db.current_version().unwrap_or(0);
        if current_ver > 0 && current_ver < CURRENT_SCHEMA_VERSION {
            let backup_path = path.with_extension(format!("db.v{current_ver}-backup"));
            match std::fs::copy(path, &backup_path) {
                Ok(_) => info!(
                    from_version = current_ver,
                    backup = %backup_path.display(),
                    "auto-backed up DB before schema migration"
                ),
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "Cannot auto-backup database before migration ({}). \
                         Aborting to protect data. Free disk space or manually backup '{}' before retrying.",
                        e,
                        path.display()
                    ));
                }
            }
        }
        db.migrate()?;
        Ok(db)
    }

    pub fn open_in_memory() -> Result<Self> {
        init_sqlite_vec();
        let conn = Connection::open_in_memory().context("failed to open in-memory SQLite")?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .context("failed to set pragmas")?;
        let mut db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Execute raw SQL with params. Returns the number of rows changed.
    ///
    /// Intended for test fixture seeding from integration tests where `conn`
    /// is not directly accessible (`pub(crate)`).
    pub fn execute_sql(&self, sql: &str, params: &[&dyn rusqlite::types::ToSql]) -> Result<usize> {
        Ok(self.conn.execute(sql, params)?)
    }

    /// Returns the last inserted rowid.
    pub fn last_insert_rowid(&self) -> i64 {
        self.conn.last_insert_rowid()
    }

    /// Query a single scalar value. Returns `None` if no rows match.
    pub fn query_scalar<T: rusqlite::types::FromSql>(
        &self,
        sql: &str,
        params: &[&dyn rusqlite::types::ToSql],
    ) -> Result<Option<T>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .conn
            .query_row(sql, params, |row| row.get(0))
            .optional()?)
    }

    /// Query a single row and return two columns. Returns `None` if no rows match.
    pub fn query_row_2<T1: rusqlite::types::FromSql, T2: rusqlite::types::FromSql>(
        &self,
        sql: &str,
        params: &[&dyn rusqlite::types::ToSql],
    ) -> Result<Option<(T1, T2)>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .conn
            .query_row(sql, params, |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?)
    }

    fn current_version(&self) -> Result<i64> {
        let exists: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='schema_version'",
                [],
                |row| row.get(0),
            )
            .context("failed to check schema_version existence")?;
        if !exists {
            return Ok(0);
        }
        let version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .context("failed to read schema version")?;
        Ok(version)
    }

    /// Insert a permission-decision provenance record (mika#1733 AC4). All
    /// fields correspond 1:1 to the v44 schema columns. `override_used` is
    /// derived by the caller and asserted at the CHECK constraint here as
    /// defense-in-depth.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_permission_decision(
        &self,
        id: &str,
        request_id: &str,
        tool_name: &str,
        args_summary: Option<&str>,
        classifier_verdict: &str,
        operator_decision: Option<&str>,
        override_used: bool,
        decision_authority: &str,
        tenant_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO permission_decisions (
                id, request_id, tool_name, args_summary,
                classifier_verdict, operator_decision, override_used,
                decision_authority, tenant_id, agent_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                id,
                request_id,
                tool_name,
                args_summary,
                classifier_verdict,
                operator_decision,
                if override_used { 1i64 } else { 0i64 },
                decision_authority,
                tenant_id,
                agent_id,
            ],
        )?;
        Ok(())
    }

    /// Insert a batch of pilot-transcript rows for one task in a single
    /// transaction (mika#1705). Atomic per source JSONL file: either every row
    /// commits or none does, so a crash mid-import never leaves a half-imported
    /// file that the ingestion tick would then delete (idempotency contract).
    /// `request_body`/`response_body` are already secret-scrubbed by the caller.
    /// A fresh UUID is minted per row — JSONL entries carry no stable id.
    pub fn insert_pilot_transcripts_batch(
        &mut self,
        task_id: &str,
        rows: &[PilotTranscriptRow],
    ) -> Result<usize> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO pilot_transcripts (
                    id, task_id, timestamp, provider, model,
                    request_body, response_body, tokens_in, tokens_out, latency_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;
            for r in rows {
                let id = uuid::Uuid::new_v4().to_string();
                stmt.execute(params![
                    id,
                    task_id,
                    r.timestamp,
                    r.provider,
                    r.model,
                    r.request_body,
                    r.response_body,
                    r.tokens_in,
                    r.tokens_out,
                    r.latency_ms,
                ])?;
            }
        }
        tx.commit()?;
        Ok(rows.len())
    }

    /// Prune pilot transcripts older than `retention_secs` (mika#1705 AC6).
    /// Retention keys off `created_at` (import time, guaranteed ISO 8601) so
    /// the scan is a simple lexicographic comparison, matching the
    /// `prune_old_llm_calls` shape.
    pub fn prune_old_pilot_transcripts(&self, retention_secs: i64) -> Result<usize> {
        let cutoff = timestamp::now_minus(Duration::seconds(retention_secs));
        let n = self.conn.execute(
            "DELETE FROM pilot_transcripts WHERE created_at < ?1",
            params![cutoff],
        )?;
        Ok(n)
    }

    /// Count pilot transcripts for a given task (mika#1705). Used by the
    /// ingestion tick's idempotency guard and by tests.
    pub fn count_pilot_transcripts_for_task(&self, task_id: &str) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM pilot_transcripts WHERE task_id = ?1",
            params![task_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    /// Finished dispatches that were asked for a pilot transcript and have not
    /// yet been reported as having produced none (mika#2040 AC7).
    ///
    /// # What selects a row, and what deliberately does not
    ///
    /// The premise "this dispatch was supposed to produce a transcript" is read
    /// from the producer's own stamp
    /// ([`PILOT_TRANSCRIPT_EXPECTED_KEY`]), never reconstructed from the skill
    /// name plus the current value of `MIKA_LOG_PILOT_TRANSCRIPTS` — that gate
    /// is read per dispatch and can flip in between, which would make the
    /// detector report dispatches nobody asked a transcript of and stay silent
    /// on the ones that were asked. For the same reason there is **no**
    /// `trigger_type = 'callback'` clause: the stamp is the discriminant, and a
    /// narrower filter would silently drop a stamped dispatch that a later
    /// caller spawns from somewhere else.
    ///
    /// The finished boundary is `status NOT IN ('pending','in_progress')` —
    /// byte-for-byte the guard [`crate::task_engine::engine`]'s ingestion uses
    /// before it dares read a file. The two must agree: a detector that
    /// considered a dispatch finished earlier than the ingestion does would
    /// report a transcript that was merely still being written.
    ///
    /// # Why `json_valid` wraps every extraction
    ///
    /// SQLite's `json_extract` raises a hard "malformed JSON" error — not NULL
    /// — on a `metadata` that is not JSON, and that error propagates out of the
    /// whole `query_map`. Unguarded, ONE task with corrupt metadata would blind
    /// the detector for every other dispatch, for ever — which is the exact
    /// failure class mika#2040 exists to end. Wrapped, the corrupt row degrades
    /// to NULL and drops out of the result; only that dispatch stops being
    /// watched. Same reasoning, same shape as
    /// [`Self::get_reaper_child_snapshot`].
    pub fn find_dispatches_expecting_transcripts(
        &self,
        agent_id: &str,
        grace_seconds: i64,
    ) -> Result<Vec<DispatchExpectingTranscript>> {
        let expected_path_json = format!(
            "$.{}",
            crate::task_engine::engine::PILOT_TRANSCRIPT_EXPECTED_KEY
        );
        let reported_path_json = format!(
            "$.{}",
            crate::task_engine::engine::PILOT_TRANSCRIPT_REPORTED_KEY
        );
        let grace_modifier = format!("-{grace_seconds} seconds");

        let mut stmt = self.conn.prepare(
            "SELECT id,
                    CASE WHEN json_valid(metadata)
                         THEN json_extract(metadata, ?2)
                    END AS expected_path,
                    status,
                    updated_at
             FROM tasks
             WHERE agent_id = ?1
               AND status NOT IN ('pending', 'in_progress')
               AND updated_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?4)
               AND expected_path IS NOT NULL
               AND (CASE WHEN json_valid(metadata)
                         THEN json_extract(metadata, ?3)
                    END) IS NULL
             ORDER BY updated_at, id",
        )?;
        let rows = stmt
            .query_map(
                params![
                    agent_id,
                    expected_path_json,
                    reported_path_json,
                    grace_modifier
                ],
                |row| {
                    Ok(DispatchExpectingTranscript {
                        task_id: row.get(0)?,
                        expected_path: row.get(1)?,
                        status: row.get(2)?,
                        updated_at: row.get(3)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Check if a column exists on a table within a transaction scope.
    fn column_exists_tx(tx: &rusqlite::Transaction<'_>, table: &str, column: &str) -> Result<bool> {
        let mut stmt = tx.prepare(&format!("PRAGMA table_info('{table}')"))?;
        let exists = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .any(|name| name.as_ref().is_ok_and(|n| n == column));
        Ok(exists)
    }

    /// v27 startup guard: refuse to open the database if the coalesce step
    /// from #787 has not run. Pins to `schema_version == 27` — future v28
    /// should carry its own guard, not inherit v27's.
    fn check_v27_coalesce_guard(&self) -> Result<()> {
        let schema_version = self.schema_version()?;
        if schema_version != 27 {
            return Ok(());
        }

        // Check for schema_meta table existence first (fresh installs have it).
        let has_meta: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='schema_meta'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);

        if !has_meta {
            anyhow::bail!(
                "KG v27 migration incomplete — coalesce step from mika#787 has not run. \
                 Deploy #787 before starting. See mika#786 and mika#787."
            );
        }

        let has_marker: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM schema_meta WHERE key = 'v27_coalesce_complete'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);

        if !has_marker {
            anyhow::bail!(
                "KG v27 migration incomplete — coalesce step from mika#787 has not run. \
                 Deploy #787 before starting. See mika#786 and mika#787."
            );
        }

        Ok(())
    }

    /// Check if a column exists on a table (used for idempotent migrations).
    fn column_exists(&self, table: &'static str, column: &'static str) -> Result<bool> {
        let mut stmt = self
            .conn
            .prepare(&format!("PRAGMA table_info('{table}')"))?;
        let exists = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .any(|name| name.as_ref().is_ok_and(|n| n == column));
        Ok(exists)
    }

    // ===== Agent CRUD =====

    /// Register an agent-corpus mapping. Idempotent via INSERT OR IGNORE.
    /// Called per (agent, corpus) pair during startup lexical ingestion (#798).
    pub fn register_agent_corpus(
        &self,
        agent_id: &str,
        docs_root_hash: &str,
        docs_root_path: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO agent_kg_corpora (agent_id, docs_root_hash, docs_root_path)
             VALUES (?1, ?2, ?3)",
            params![agent_id, docs_root_hash, docs_root_path],
        )?;
        Ok(())
    }

    pub fn register_agent(&self, id: &str, name: &str, home_dir: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO agents (id, name, home_dir) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET
               home_dir = CASE WHEN excluded.home_dir != '' THEN excluded.home_dir ELSE agents.home_dir END,
               name = excluded.name",
            params![id, name, home_dir],
        )?;
        Ok(())
    }

    /// Get the display name for an agent, falling back to the ID if not found.
    pub fn get_agent_display_name(&self, id: &str) -> String {
        self.conn
            .query_row("SELECT name FROM agents WHERE id = ?1", params![id], |r| {
                r.get::<_, String>(0)
            })
            .unwrap_or_else(|_| id.to_string())
    }

    pub fn update_agent_last_seen(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE agents SET last_seen = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn list_agents_db(&self) -> Result<Vec<AgentRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, home_dir, active, last_seen, created_at FROM agents ORDER BY name",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(AgentRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    home_dir: r.get(2)?,
                    active: r.get(3)?,
                    last_seen: r.get(4)?,
                    created_at: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // ===== Skill Overrides =====

    /// Get all skill overrides for an agent.
    pub fn get_skill_overrides(&self, agent_id: &str) -> Result<Vec<SkillOverride>> {
        let mut stmt = self.conn.prepare(
            "SELECT skill_name, always_on, llm_provider, llm_model, enabled,
                    lifecycle_state, use_count, last_used_at
             FROM skill_overrides WHERE agent_id = ?1",
        )?;
        let rows = stmt
            .query_map(params![agent_id], |r| {
                Ok(SkillOverride {
                    skill_name: r.get(0)?,
                    always_on: r.get(1)?,
                    llm_provider: r.get(2)?,
                    llm_model: r.get(3)?,
                    enabled: r.get(4)?,
                    lifecycle_state: r.get(5)?,
                    use_count: r.get(6)?,
                    last_used_at: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Set (upsert) an always_on override for a skill.
    pub fn set_skill_override(
        &self,
        agent_id: &str,
        skill_name: &str,
        always_on: bool,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO skill_overrides (agent_id, skill_name, always_on)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(agent_id, skill_name) DO UPDATE SET always_on = excluded.always_on",
            params![agent_id, skill_name, always_on],
        )?;
        Ok(())
    }

    /// Set (upsert) an LLM provider/model override for a skill.
    /// Preserves existing `always_on` via the conflict clause.
    pub fn set_skill_llm_override(
        &self,
        agent_id: &str,
        skill_name: &str,
        provider: &str,
        model: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO skill_overrides (agent_id, skill_name, llm_provider, llm_model)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(agent_id, skill_name) DO UPDATE SET
               llm_provider = excluded.llm_provider,
               llm_model    = excluded.llm_model",
            params![agent_id, skill_name, provider, model],
        )?;
        Ok(())
    }

    /// Set (upsert) an enabled override for a skill.
    ///
    /// `enabled = false` disables the skill; `enabled = true` explicitly enables it.
    /// When setting to `true` (the default) and all other override columns are NULL,
    /// the row is deleted (default-equals-delete).
    pub fn set_skill_enabled(
        &mut self,
        agent_id: &str,
        skill_name: &str,
        enabled: bool,
    ) -> Result<()> {
        let db_val: Option<bool> = if enabled { None } else { Some(false) };
        // RAII transaction: Drop without commit() auto-rolls back, preventing
        // stuck transactions that pin the WAL snapshot (mika#636).
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO skill_overrides (agent_id, skill_name, enabled)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(agent_id, skill_name) DO UPDATE SET enabled = excluded.enabled",
            params![agent_id, skill_name, db_val],
        )?;
        // Default-equals-delete: if all columns are NULL, remove the row.
        if enabled {
            tx.execute(
                "DELETE FROM skill_overrides
                  WHERE agent_id = ?1 AND skill_name = ?2
                    AND always_on IS NULL
                    AND llm_provider IS NULL
                    AND llm_model IS NULL
                    AND enabled IS NULL
                    AND lifecycle_state IS NULL",
                params![agent_id, skill_name],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Set the lifecycle state for an agent-authored skill (mika#1582).
    ///
    /// Valid states: `staged`, `active`, `archived`. The CHECK constraint on
    /// the column enforces this at the SQL layer.
    pub fn set_skill_lifecycle_state(
        &mut self,
        agent_id: &str,
        skill_name: &str,
        state: &str,
    ) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let affected = tx.execute(
            "UPDATE skill_overrides SET lifecycle_state = ?3
             WHERE agent_id = ?1 AND skill_name = ?2",
            params![agent_id, skill_name, state],
        )?;
        if affected == 0 {
            anyhow::bail!("no skill_overrides row for agent={agent_id}, skill={skill_name}");
        }
        tx.commit()?;
        Ok(())
    }

    /// Get the lifecycle state for a skill (mika#1582).
    ///
    /// Returns `None` if no override row exists or `lifecycle_state` is NULL.
    pub fn get_skill_lifecycle_state(
        &self,
        agent_id: &str,
        skill_name: &str,
    ) -> Result<Option<String>> {
        let result = self.conn.query_row(
            "SELECT lifecycle_state FROM skill_overrides
             WHERE agent_id = ?1 AND skill_name = ?2",
            params![agent_id, skill_name],
            |r| r.get(0),
        );
        match result {
            Ok(state) => Ok(state),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Clear the LLM override columns for a skill. If the resulting row has no
    /// remaining override values (all columns NULL), the row is deleted.
    ///
    /// The UPDATE and prune DELETE are wrapped in an atomic transaction so a
    /// crash between them cannot leave a half-cleared row.
    pub fn delete_skill_llm_override(&mut self, agent_id: &str, skill_name: &str) -> Result<()> {
        // RAII transaction: Drop without commit() auto-rolls back, preventing
        // stuck transactions that pin the WAL snapshot (mika#636).
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE skill_overrides
                SET llm_provider = NULL, llm_model = NULL
              WHERE agent_id = ?1 AND skill_name = ?2",
            params![agent_id, skill_name],
        )?;
        tx.execute(
            "DELETE FROM skill_overrides
              WHERE agent_id = ?1 AND skill_name = ?2
                AND always_on IS NULL
                AND llm_provider IS NULL
                AND llm_model IS NULL
                AND enabled IS NULL
                AND lifecycle_state IS NULL",
            params![agent_id, skill_name],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Delete an override for a skill (revert to bundled default).
    pub fn delete_skill_override(&self, agent_id: &str, skill_name: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM skill_overrides WHERE agent_id = ?1 AND skill_name = ?2",
            params![agent_id, skill_name],
        )?;
        Ok(())
    }

    /// Batch-increment usage counters for all injected skills in a single turn.
    /// Creates rows for skills without prior overrides via UPSERT.
    pub fn increment_skill_usage(&self, agent_id: &str, skill_names: &[String]) -> Result<()> {
        if skill_names.is_empty() {
            return Ok(());
        }
        let now = crate::timestamp::now();
        let tx = self.conn.unchecked_transaction()?;
        for name in skill_names {
            tx.execute(
                "INSERT INTO skill_overrides (agent_id, skill_name, use_count, last_used_at)
                 VALUES (?1, ?2, 1, ?3)
                 ON CONFLICT(agent_id, skill_name) DO UPDATE SET
                    use_count = use_count + 1,
                    last_used_at = ?3",
                params![agent_id, name, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Query skills eligible for archival by the curator.
    /// Only considers agent-authored skills with `lifecycle_state = 'active'`
    /// that have been idle beyond `max_idle_days`.
    pub fn get_archival_candidates(
        &self,
        agent_id: &str,
        max_idle_days: u32,
    ) -> Result<Vec<SkillOverride>> {
        let cutoff = crate::timestamp::now_minus(chrono::Duration::days(i64::from(max_idle_days)));
        let mut stmt = self.conn.prepare(
            "SELECT skill_name, always_on, llm_provider, llm_model, enabled,
                    lifecycle_state, use_count, last_used_at
             FROM skill_overrides
             WHERE agent_id = ?1
               AND lifecycle_state = 'active'
               AND (
                 (last_used_at IS NULL AND use_count = 0)
                 OR last_used_at < ?2
               )",
        )?;
        let rows = stmt
            .query_map(params![agent_id, cutoff], |r| {
                Ok(SkillOverride {
                    skill_name: r.get(0)?,
                    always_on: r.get(1)?,
                    llm_provider: r.get(2)?,
                    llm_model: r.get(3)?,
                    enabled: r.get(4)?,
                    lifecycle_state: r.get(5)?,
                    use_count: r.get(6)?,
                    last_used_at: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Update the lifecycle_state of a skill override.
    pub fn update_skill_lifecycle_state(
        &self,
        agent_id: &str,
        skill_name: &str,
        state: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE skill_overrides SET lifecycle_state = ?3
             WHERE agent_id = ?1 AND skill_name = ?2",
            params![agent_id, skill_name, state],
        )?;
        Ok(())
    }

    /// Retrieve the most recent curator proposal from audit_events.
    pub fn get_latest_curator_proposal(&self, agent_id: &str) -> Result<Option<(String, String)>> {
        self.query_row_2(
            "SELECT after_value, created_at FROM audit_events
             WHERE tool_name = 'curator_review' AND target_key = 'curator_proposal'
               AND agent_id = ?1
             ORDER BY created_at DESC LIMIT 1",
            &[&agent_id as &dyn rusqlite::types::ToSql],
        )
    }

    // ===== Team CRUD =====

    pub fn register_team(&self, id: &str, name: &str, config_path: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO teams (id, name, config_path) VALUES (?1, ?2, ?3)",
            params![id, name, config_path],
        )?;
        Ok(())
    }

    pub fn list_teams_db(&self) -> Result<Vec<TeamRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, config_path, created_at FROM teams ORDER BY name")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(TeamRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    config_path: r.get(2)?,
                    created_at: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // ===== Task CRUD — the `Task` deserializer only =====
    //
    // The domain itself moved to `db/tasks.rs` (mika#2396). These two items
    // stayed behind because `Rewind` and `Dashboard` consume them at ten sites
    // further down this file, and a child module sees its parent's private
    // items — so leaving them here costs nothing, while taking them along would
    // have forced `pub(super)` on both the `fn` and the `const`.
    //
    // The `audit_events` and `schema_meta` methods that follow were never Task
    // CRUD either; they merely sat inside the old line range. Regrouping them
    // under a file of their own is a separate ticket, deliberately not opened
    // here because nothing today asks for it.

    fn row_to_task(r: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
        Ok(Task {
            id: r.get(0)?,
            agent_id: r.get(1)?,
            team_run_id: r.get(2)?,
            parent_task_id: r.get(3)?,
            depth: r.get(4)?,
            label: r.get(5)?,
            trigger_type: r.get(6)?,
            cron_expr: r.get(7)?,
            event_source: r.get(8)?,
            event_offset_secs: r.get(9)?,
            condition_expr: r.get(10)?,
            next_fire_at: r.get(11)?,
            timeout_at: r.get(12)?,
            action_type: r.get(13)?,
            action_config: r.get(14)?,
            status: r.get(15)?,
            process_id: r.get(16)?,
            input_context: r.get(17)?,
            result: r.get(18)?,
            created_by_session: r.get(19)?,
            created_trace_id: r.get(20)?,
            execution_trace_id: r.get(21)?,
            created_at: r.get(22)?,
            updated_at: r.get(23)?,
            fired_at: r.get(24)?,
            completed_at: r.get(25)?,
            reference_url: r.get(26)?,
            source: r.get(27)?,
            metadata: r.get(28)?,
            r#type: r.get(29)?,
            dispatch_class: r.get(30)?,
            dispatcher_source: r.get(31)?,
        })
    }

    const TASK_COLUMNS: &'static str = "id, agent_id, team_run_id, parent_task_id, depth, label,
         trigger_type, cron_expr, event_source, event_offset_secs, condition_expr,
         next_fire_at, timeout_at, action_type, action_config,
         status, process_id, input_context, result, created_by_session,
         created_trace_id, execution_trace_id, created_at, updated_at, fired_at, completed_at,
         reference_url, source, metadata, type, dispatch_class, dispatcher_source";

    /// Count audit_events matching (agent_id, tool_name, target_key) with `created_at > since`.
    ///
    /// Used by the verdict-handler's PR-keyed circuit breaker (mika#1563):
    /// counts prior `verdict_observed` events for a given PR URL in a sliding
    /// window. The check runs BEFORE task lookup, so it fires even when the
    /// task is missing or no longer in_progress — which is the convergence-loop
    /// failure mode that #1556 hit.
    ///
    /// `since` must be an ISO 8601 UTC timestamp (`%Y-%m-%dT%H:%M:%SZ`). String
    /// comparison is correct because the column format is fixed-width UTC.
    pub fn count_recent_audit_events_for_target(
        &self,
        agent_id: &str,
        tool_name: &str,
        target_key: &str,
        since: &str,
    ) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM audit_events
             WHERE agent_id = ?1 AND tool_name = ?2 AND target_key = ?3
               AND created_at > ?4",
            params![agent_id, tool_name, target_key, since],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    /// Read a `schema_meta` value, or `None` when the key was never stamped.
    pub fn get_schema_meta(&self, key: &str) -> Result<Option<String>> {
        let value = self
            .conn
            .query_row(
                "SELECT value FROM schema_meta WHERE key = ?1",
                params![key],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(value)
    }

    /// Stamp the current instant under `key` if — and only if — nothing is
    /// stamped there yet, then return the effective value (mika#2169, L2b).
    ///
    /// `INSERT OR IGNORE` makes it idempotent: the first boot carrying L1 sets
    /// the instant, every later boot is a no-op. No DDL, no
    /// `CURRENT_SCHEMA_VERSION` bump — `schema_meta` already exists and already
    /// carries one-shot markers (`v27_coalesce_complete`,
    /// `well_known_d2_migration_v1`).
    ///
    /// The instant is read from the database, not from a compiled-in date: a
    /// literal would fix the epoch at the day the plan was written rather than
    /// the day L1 started running, and `env!()` would make two builds of the
    /// same commit behave differently.
    pub fn stamp_schema_meta_epoch_if_absent(&self, key: &str) -> Result<String> {
        self.conn.execute(
            "INSERT OR IGNORE INTO schema_meta (key, value)
             VALUES (?1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
            params![key],
        )?;
        let value = self.conn.query_row(
            "SELECT value FROM schema_meta WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )?;
        Ok(value)
    }

    /// Count `audit_events` rows for a given agent + tool_name — helper for
    /// mika#1712 integration tests to assert the load-bearing delta on the
    /// `phantom_aged_out` audit-write path. Keeping this query as a first-class
    /// helper (not inline SQL in tests) means the load-bearing assertion has a
    /// single source of truth that survives future audit_events schema changes.
    #[doc(hidden)]
    pub fn count_audit_events_by_tool_name(&self, agent_id: &str, tool_name: &str) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM audit_events
             WHERE agent_id = ?1 AND tool_name = ?2",
            params![agent_id, tool_name],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    /// Fetch the `target_key` values of `audit_events` rows for the scoped
    /// agent + tool_name — helper for mika#1712 integration tests to make the
    /// row-shape assertion (target_key must reference the injected task id).
    #[doc(hidden)]
    pub fn get_audit_event_target_keys_by_tool_name(
        &self,
        agent_id: &str,
        tool_name: &str,
    ) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT target_key FROM audit_events
             WHERE agent_id = ?1 AND tool_name = ?2
             ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![agent_id, tool_name], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Fetch `(target_key, before_value, after_value, reasoning)` tuples of
    /// `audit_events` rows for the scoped agent + tool_name — helper for
    /// mika#1712 integration tests to make the full-row-shape assertion
    /// (T-1/T-2, 2026-08-21). Complements
    /// [`Self::get_audit_event_target_keys_by_tool_name`]. Return type is a
    /// tuple to keep the helper zero-abstraction; a caller-side struct is
    /// unnecessary for the assertion pattern.
    #[doc(hidden)]
    pub fn get_audit_event_rows_by_tool_name(
        &self,
        agent_id: &str,
        tool_name: &str,
    ) -> Result<Vec<AuditEventRowTuple>> {
        let mut stmt = self.conn.prepare(
            "SELECT target_key, before_value, after_value, reasoning
             FROM audit_events
             WHERE agent_id = ?1 AND tool_name = ?2
             ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![agent_id, tool_name], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ===== Sessions =====

    pub fn create_session(&self, id: &str, agent_id: &str, channel_type: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, agent_id, channel_type) VALUES (?1, ?2, ?3)",
            params![id, agent_id, channel_type],
        )?;
        Ok(())
    }

    pub fn create_session_with_metadata(
        &self,
        id: &str,
        agent_id: &str,
        channel_type: &str,
        metadata: Option<&str>,
        task_id: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, agent_id, channel_type, metadata, task_id) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, agent_id, channel_type, metadata, task_id],
        )?;
        Ok(())
    }

    /// Create a session with metadata, parent session reference, and optional task linkage.
    /// Used by callback and skill_run dispatchers to link back to the originating session.
    pub fn create_session_with_parent(
        &self,
        id: &str,
        agent_id: &str,
        channel_type: &str,
        metadata: Option<&str>,
        parent_session_id: Option<&str>,
        task_id: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, agent_id, channel_type, metadata, parent_session_id, task_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, agent_id, channel_type, metadata, parent_session_id, task_id],
        )?;
        Ok(())
    }

    /// Create a session with metadata if it doesn't already exist (INSERT OR IGNORE).
    /// Used by team engine for per-agent sessions that may already exist on resumed runs.
    pub fn create_session_if_not_exists(
        &self,
        id: &str,
        agent_id: &str,
        channel_type: &str,
        metadata: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO sessions (id, agent_id, channel_type, metadata) VALUES (?1, ?2, ?3, ?4)",
            params![id, agent_id, channel_type, metadata],
        )?;
        Ok(())
    }

    pub fn end_session(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET ended_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    /// End a session unless it is the agent's canonical singleton session (mika#1401).
    ///
    /// Singleton agents (`[session] singleton = true` in identity.toml) reuse one
    /// canonical session across every invocation; ending it would set `ended_at`
    /// and — worse — surface it in the dashboard as a finished conversation. This
    /// helper no-ops when `id == canonical_id`, so the canonical session's
    /// `ended_at` stays NULL forever. When `canonical_id` is `None` (non-singleton
    /// agent) it behaves exactly like [`end_session`].
    pub fn end_session_unless_canonical(&self, id: &str, canonical_id: Option<&str>) -> Result<()> {
        if canonical_id == Some(id) {
            return Ok(());
        }
        self.end_session(id)
    }

    pub fn get_or_create_system_session(&self, agent_id: &str) -> Result<String> {
        let id = format!("system-{agent_id}");
        self.conn.execute(
            "INSERT OR IGNORE INTO sessions (id, agent_id, channel_type) VALUES (?1, ?2, 'system')",
            params![&id, agent_id],
        )?;
        Ok(id)
    }

    /// Idempotently create the canonical session for a singleton agent (mika#1401).
    ///
    /// Mirrors [`get_or_create_system_session`]: `INSERT OR IGNORE` so the first
    /// invocation creates the row and every subsequent `mika ask` / `/send` reuses
    /// it. Unlike the system session the ID is caller-supplied (either the derived
    /// `canonical-{agent_id}` or an operator-chosen `canonical_id` such as
    /// mika-prime's zero-UUID). Channel type defaults to `'cli'`; the singleton
    /// concept is orthogonal to channel — multiple channels merging into one
    /// session is the designed intent for single-surface oracles.
    pub fn get_or_create_canonical_session(
        &self,
        session_id: &str,
        agent_id: &str,
        channel_type: &str,
    ) -> Result<String> {
        self.conn.execute(
            "INSERT OR IGNORE INTO sessions (id, agent_id, channel_type) VALUES (?1, ?2, ?3)",
            params![session_id, agent_id, channel_type],
        )?;
        Ok(session_id.to_string())
    }

    /// Prune ended system/silent sessions older than `retention_secs`.
    /// Targets heartbeat, callback, skill, reflection, team, and delegate sessions.
    /// Messages are cascade-deleted via FK ON DELETE CASCADE.
    pub fn prune_old_sessions(&self, retention_secs: i64) -> Result<usize> {
        let cutoff = timestamp::now_minus(Duration::seconds(retention_secs));
        let n = self.conn.execute(
            "DELETE FROM sessions WHERE ended_at IS NOT NULL AND ended_at < ?1
             AND (id LIKE 'heartbeat-%' OR id LIKE 'callback-%' OR id LIKE 'skill-%' OR id LIKE 'reflection-%' OR id LIKE 'team-%' OR id LIKE 'delegate-%' OR id LIKE 'reminder-%')",
            params![cutoff],
        )?;
        Ok(n)
    }

    // ===== LLM Calls =====

    /// Maximum stored input/output size for tool calls (50KB, measured in bytes).
    const TOOL_CALL_MAX_BYTES: usize = 50_000;

    /// Truncate a string at a UTF-8 safe boundary, avoiding panics on multi-byte characters.
    ///
    /// mika#2103: the boundary walk lives in `mika_common::text::safe_truncate`;
    /// this wrapper only adds the "truncated" suffix.
    fn truncate_utf8_safe(s: &str, max_bytes: usize) -> String {
        if s.len() <= max_bytes {
            return s.to_string();
        }
        format!(
            "{}... (truncated at {} bytes)",
            mika_common::text::safe_truncate(s, max_bytes),
            max_bytes
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn save_llm_call(
        &self,
        id: &str,
        agent_id: &str,
        session_id: &str,
        trace_id: Option<&str>,
        provider: &str,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: Option<u64>,
        cache_write_tokens: Option<u64>,
        latency_ms: u64,
        stop_reason: Option<&str>,
        status: &str,
        error_message: Option<&str>,
        step: u32,
        prompt_variant: Option<&str>,
        response_text: Option<&str>,
        reasoning: Option<&str>,
        system_prompt_bytes: Option<i64>,
        request_bytes: Option<i64>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO llm_calls (id, agent_id, session_id, trace_id, provider, model,
             input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
             latency_ms, stop_reason, status, error_message, step, prompt_variant,
             response_text, reasoning, system_prompt_bytes, request_bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            params![
                id,
                agent_id,
                session_id,
                trace_id,
                provider,
                model,
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
                latency_ms,
                stop_reason,
                status,
                error_message,
                step,
                prompt_variant,
                response_text,
                reasoning,
                system_prompt_bytes,
                request_bytes,
            ],
        )?;
        Ok(())
    }

    // ===== Tool Calls =====

    #[allow(clippy::too_many_arguments)]
    pub fn save_tool_call(
        &self,
        id: &str,
        agent_id: &str,
        session_id: &str,
        trace_id: Option<&str>,
        llm_call_id: Option<&str>,
        step: u32,
        tool_name: &str,
        tool_source: &str,
        skill_name: Option<&str>,
        input: Option<&str>,
        output: Option<&str>,
        success: bool,
        non_zero_exit: bool,
        latency_ms: u64,
        error_message: Option<&str>,
    ) -> Result<()> {
        // Scrub secret-shaped values before persistence (#908).
        // Order: scrub → truncate → INSERT. The LLM's in-memory result is NOT
        // scrubbed — only the durable copy in tool_calls is sanitized.
        // `scrub_secrets` returns Cow::Borrowed when no secrets found (zero alloc).
        use crate::secret_scrubber::scrub_secrets;
        let scrubbed_input = input.map(scrub_secrets);
        let scrubbed_output = output.map(scrub_secrets);
        let scrubbed_error = error_message.map(scrub_secrets);

        // Truncate large inputs/outputs to prevent DB bloat.
        // Uses char_indices for UTF-8 safe boundary (byte slicing panics on multi-byte chars).
        let truncated_input = scrubbed_input
            .as_deref()
            .map(|s| Self::truncate_utf8_safe(s, Self::TOOL_CALL_MAX_BYTES));
        let truncated_output = scrubbed_output
            .as_deref()
            .map(|s| Self::truncate_utf8_safe(s, Self::TOOL_CALL_MAX_BYTES));
        self.conn.execute(
            "INSERT INTO tool_calls (id, agent_id, session_id, trace_id, llm_call_id,
             step, tool_name, tool_source, skill_name, input, output,
             success, non_zero_exit, latency_ms, error_message)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                id,
                agent_id,
                session_id,
                trace_id,
                llm_call_id,
                step,
                tool_name,
                tool_source,
                skill_name,
                truncated_input,
                truncated_output,
                success,
                non_zero_exit,
                latency_ms,
                scrubbed_error.as_deref(),
            ],
        )?;
        Ok(())
    }

    /// Prune LLM calls older than `retention_secs`.
    pub fn prune_old_llm_calls(&self, retention_secs: i64) -> Result<usize> {
        let cutoff = timestamp::now_minus(Duration::seconds(retention_secs));
        let n = self.conn.execute(
            "DELETE FROM llm_calls WHERE created_at < ?1",
            params![cutoff],
        )?;
        Ok(n)
    }

    /// Prune tool calls older than `retention_secs`.
    pub fn prune_old_tool_calls(&self, retention_secs: i64) -> Result<usize> {
        let cutoff = timestamp::now_minus(Duration::seconds(retention_secs));
        let n = self.conn.execute(
            "DELETE FROM tool_calls WHERE created_at < ?1",
            params![cutoff],
        )?;
        Ok(n)
    }

    /// Prior `gh` close calls against the same target, inside a time window
    /// (mika#1646 Layer B / AC2).
    ///
    /// Scoped to the **agent**, not the session or the trace: the founding
    /// incident's second close came from a deferred webhook replay, i.e. a
    /// different execution context than the first. A session-scoped query
    /// would have found nothing and waved it through. Repeat detection is only
    /// meaningful if it outlives the process that took the first action.
    ///
    /// `target_fragment` is the action's identity as it appears in the
    /// serialized `run_gh` input (e.g. `"pr","close"` plus the number); the
    /// caller passes the pieces and the LIKE patterns are anchored on both so
    /// a close of #164 does not match a close of #1644.
    ///
    /// `exclude_id` drops the current call's own row when it has already been
    /// persisted (it has not, at gate time — the gate runs before execution —
    /// but the parameter keeps the query honest for post-hoc callers).
    pub fn find_recent_destructive_actions(
        &self,
        agent_id: &str,
        noun: &str,
        number: &str,
        window_secs: i64,
        exclude_id: Option<&str>,
    ) -> Result<Vec<ToolCallRow>> {
        let cutoff = timestamp::now_minus(Duration::seconds(window_secs));
        // The `input` column holds the serialized tool arguments, e.g.
        // {"command":["pr","close","1644","--comment","..."]}. Match the noun
        // and the verb adjacently, and the number as a whole JSON array
        // element, so neither a substring number nor a `pr view` slips in.
        let noun_verb = format!(r#"%"{noun}","close"%"#);
        let number_exact = format!(r#"%"{number}"%"#);
        let mut stmt = self.conn.prepare(
            "SELECT id, agent_id, session_id, trace_id, llm_call_id,
                    step, tool_name, tool_source, skill_name,
                    input, output, success, non_zero_exit,
                    latency_ms, error_message, created_at
             FROM tool_calls
             WHERE agent_id = ?1
               AND tool_name = 'run_gh'
               AND created_at >= ?2
               AND input LIKE ?3
               AND input LIKE ?4
               AND (?5 IS NULL OR id != ?5)
             ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(
                params![agent_id, cutoff, noun_verb, number_exact, exclude_id],
                Self::row_to_tool_call,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ===== LLM Call Queries (Dashboard) =====

    pub fn query_llm_calls_by_trace(&self, trace_id: &str) -> Result<Vec<LlmCallRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, agent_id, session_id, trace_id, provider, model,
                    input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                    latency_ms, stop_reason, status, error_message, step, prompt_variant, created_at,
                    response_text IS NOT NULL, reasoning IS NOT NULL
             FROM llm_calls WHERE trace_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![trace_id], Self::row_to_llm_call)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn query_tool_calls_by_trace(&self, trace_id: &str) -> Result<Vec<ToolCallRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, agent_id, session_id, trace_id, llm_call_id,
                    step, tool_name, tool_source, skill_name,
                    input, output, success, non_zero_exit,
                    latency_ms, error_message, created_at
             FROM tool_calls WHERE trace_id = ?1 ORDER BY created_at ASC, step ASC",
        )?;
        let rows = stmt
            .query_map(params![trace_id], Self::row_to_tool_call)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn query_llm_calls_by_session(
        &self,
        session_id: &str,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<LlmCallRow>, u64)> {
        let total: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM llm_calls WHERE session_id = ?1",
            params![session_id],
            |r| r.get(0),
        )?;
        let offset = (page.saturating_sub(1)) * per_page;
        let mut stmt = self.conn.prepare(
            "SELECT id, agent_id, session_id, trace_id, provider, model,
                    input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                    latency_ms, stop_reason, status, error_message, step, prompt_variant, created_at,
                    response_text IS NOT NULL, reasoning IS NOT NULL
             FROM llm_calls WHERE session_id = ?1
             ORDER BY created_at DESC LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt
            .query_map(params![session_id, per_page, offset], Self::row_to_llm_call)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((rows, total))
    }

    pub fn query_tool_calls_by_session(
        &self,
        session_id: &str,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<ToolCallRow>, u64)> {
        let total: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tool_calls WHERE session_id = ?1",
            params![session_id],
            |r| r.get(0),
        )?;
        let offset = (page.saturating_sub(1)) * per_page;
        let mut stmt = self.conn.prepare(
            "SELECT id, agent_id, session_id, trace_id, llm_call_id,
                    step, tool_name, tool_source, skill_name,
                    input, output, success, non_zero_exit,
                    latency_ms, error_message, created_at
             FROM tool_calls WHERE session_id = ?1
             ORDER BY created_at DESC, step ASC LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt
            .query_map(
                params![session_id, per_page, offset],
                Self::row_to_tool_call,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((rows, total))
    }

    pub fn query_llm_calls(
        &self,
        filters: &LlmCallFilters,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<LlmCallRow>, u64)> {
        let mut where_clauses = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref agent_id) = filters.agent_id {
            params_vec.push(Box::new(agent_id.clone()));
            where_clauses.push(format!("agent_id = ?{}", params_vec.len()));
        }
        if let Some(ref session_id) = filters.session_id {
            params_vec.push(Box::new(session_id.clone()));
            where_clauses.push(format!("session_id = ?{}", params_vec.len()));
        }
        if let Some(ref trace_id) = filters.trace_id {
            params_vec.push(Box::new(trace_id.clone()));
            where_clauses.push(format!("trace_id = ?{}", params_vec.len()));
        }
        if let Some(ref model) = filters.model {
            params_vec.push(Box::new(model.clone()));
            where_clauses.push(format!("model = ?{}", params_vec.len()));
        }
        if let Some(ref from) = filters.from {
            params_vec.push(Box::new(from.clone()));
            where_clauses.push(format!("created_at >= ?{}", params_vec.len()));
        }
        if let Some(ref to) = filters.to {
            params_vec.push(Box::new(to.clone()));
            where_clauses.push(format!("created_at <= ?{}", params_vec.len()));
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };

        let count_sql = format!("SELECT COUNT(*) FROM llm_calls {where_sql}");
        let total: u64 = self.conn.query_row(
            &count_sql,
            rusqlite::params_from_iter(params_vec.iter().map(|p| p.as_ref())),
            |r| r.get(0),
        )?;

        let offset = (page.saturating_sub(1)) * per_page;
        params_vec.push(Box::new(per_page));
        params_vec.push(Box::new(offset));
        let query_sql = format!(
            "SELECT id, agent_id, session_id, trace_id, provider, model,
                    input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                    latency_ms, stop_reason, status, error_message, step, prompt_variant, created_at,
                    response_text IS NOT NULL, reasoning IS NOT NULL
             FROM llm_calls {where_sql}
             ORDER BY created_at DESC LIMIT ?{} OFFSET ?{}",
            params_vec.len() - 1,
            params_vec.len()
        );
        let mut stmt = self.conn.prepare(&query_sql)?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(params_vec.iter().map(|p| p.as_ref())),
                Self::row_to_llm_call,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((rows, total))
    }

    pub fn query_cost_trend(&self, filters: &CostTrendFilters) -> Result<CostTrendResponse> {
        use std::collections::BTreeMap;

        // Determine effective from/to.
        let effective_from = filters
            .from
            .clone()
            .unwrap_or_else(|| timestamp::now_minus(Duration::hours(24)));
        let effective_to = filters.to.clone().unwrap_or_else(timestamp::now);

        // Determine bucket size.
        let bucket_size = match filters.bucket.as_deref() {
            Some("hour") => "hour",
            Some("day") => "day",
            Some("auto") | None => {
                let from_dt = timestamp::parse(&effective_from)
                    .unwrap_or_else(|_| Utc::now() - Duration::hours(24));
                let to_dt = timestamp::parse(&effective_to).unwrap_or_else(|_| Utc::now());
                let span_secs = (to_dt - from_dt).num_seconds();
                if span_secs < 259_200 { "hour" } else { "day" }
            }
            _ => "hour",
        };

        let bucket_expr = if bucket_size == "hour" {
            "substr(created_at, 1, 13) || ':00:00Z'"
        } else {
            "substr(created_at, 1, 10) || 'T00:00:00Z'"
        };

        // Build WHERE clause.
        let mut where_clauses = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        params_vec.push(Box::new(effective_from));
        where_clauses.push(format!("created_at >= ?{}", params_vec.len()));

        params_vec.push(Box::new(effective_to));
        where_clauses.push(format!("created_at <= ?{}", params_vec.len()));

        if let Some(ref agent_id) = filters.agent_id {
            params_vec.push(Box::new(agent_id.clone()));
            where_clauses.push(format!("agent_id = ?{}", params_vec.len()));
        }
        if let Some(ref model) = filters.model {
            params_vec.push(Box::new(model.clone()));
            where_clauses.push(format!("model = ?{}", params_vec.len()));
        }

        let where_sql = format!("WHERE {}", where_clauses.join(" AND "));

        let sql = format!(
            "SELECT {bucket_expr} as bucket_ts,
                    agent_id, provider, model,
                    SUM(input_tokens) as total_input,
                    SUM(output_tokens) as total_output,
                    SUM(COALESCE(cache_read_tokens, 0)) as total_cache_read,
                    SUM(COALESCE(cache_write_tokens, 0)) as total_cache_write,
                    COUNT(*) as call_count
             FROM llm_calls
             {where_sql}
             GROUP BY bucket_ts, agent_id, provider, model
             ORDER BY bucket_ts ASC"
        );

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(
            rusqlite::params_from_iter(params_vec.iter().map(|p| p.as_ref())),
            |row| {
                Ok((
                    row.get::<_, String>(0)?, // bucket_ts
                    row.get::<_, String>(1)?, // agent_id
                    row.get::<_, String>(2)?, // provider
                    row.get::<_, String>(3)?, // model
                    row.get::<_, u64>(4)?,    // total_input
                    row.get::<_, u64>(5)?,    // total_output
                    row.get::<_, u64>(6)?,    // total_cache_read
                    row.get::<_, u64>(7)?,    // total_cache_write
                    row.get::<_, u64>(8)?,    // call_count
                ))
            },
        )?;

        // Aggregate by (bucket_ts, agent_id), applying per-model pricing.
        let mut has_estimated = false;
        let mut estimated_models: Vec<String> = Vec::new();

        // Key: (bucket_ts, agent_id) -> (cost_usd, input_tokens, output_tokens, call_count)
        type BucketKey = (String, Option<String>);
        type BucketValue = (f64, u64, u64, u64);
        let mut aggregated: BTreeMap<BucketKey, BucketValue> = BTreeMap::new();

        for row in rows {
            let (
                bucket_ts,
                agent_id,
                provider,
                model,
                input,
                output,
                cache_read,
                cache_write,
                count,
            ) = row?;
            let pricing = crate::pricing::get_pricing(&provider, &model);
            let cost = crate::pricing::estimate_call_cost(
                &pricing,
                input,
                output,
                Some(cache_read),
                Some(cache_write),
            );
            if crate::pricing::is_fallback_pricing(&pricing) {
                has_estimated = true;
                let model_key = format!("{provider}/{model}");
                if !estimated_models.contains(&model_key) {
                    estimated_models.push(model_key);
                }
            }

            let agent_key = Some(agent_id);
            let entry = aggregated.entry((bucket_ts, agent_key)).or_default();
            entry.0 += cost;
            entry.1 += input;
            entry.2 += output;
            entry.3 += count;
        }

        let buckets = aggregated
            .into_iter()
            .map(
                |((ts, agent_id), (cost_usd, input_tokens, output_tokens, call_count))| {
                    CostTrendBucket {
                        timestamp: ts,
                        cost_usd,
                        input_tokens,
                        output_tokens,
                        call_count,
                        agent_id,
                    }
                },
            )
            .collect();

        Ok(CostTrendResponse {
            buckets,
            bucket_size: bucket_size.to_string(),
            has_estimated_pricing: has_estimated,
            estimated_models,
        })
    }

    pub fn query_tool_calls(
        &self,
        filters: &ToolCallFilters,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<ToolCallRow>, u64)> {
        let mut where_clauses = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref agent_id) = filters.agent_id {
            params_vec.push(Box::new(agent_id.clone()));
            where_clauses.push(format!("agent_id = ?{}", params_vec.len()));
        }
        if let Some(ref session_id) = filters.session_id {
            params_vec.push(Box::new(session_id.clone()));
            where_clauses.push(format!("session_id = ?{}", params_vec.len()));
        }
        if let Some(ref trace_id) = filters.trace_id {
            params_vec.push(Box::new(trace_id.clone()));
            where_clauses.push(format!("trace_id = ?{}", params_vec.len()));
        }
        if let Some(ref tool_name) = filters.tool_name {
            params_vec.push(Box::new(tool_name.clone()));
            where_clauses.push(format!("tool_name = ?{}", params_vec.len()));
        }
        if let Some(success) = filters.success {
            params_vec.push(Box::new(success));
            where_clauses.push(format!("success = ?{}", params_vec.len()));
        }
        if let Some(ref from) = filters.from {
            params_vec.push(Box::new(from.clone()));
            where_clauses.push(format!("created_at >= ?{}", params_vec.len()));
        }
        if let Some(ref to) = filters.to {
            params_vec.push(Box::new(to.clone()));
            where_clauses.push(format!("created_at <= ?{}", params_vec.len()));
        }
        if let Some(ref keyword) = filters.keyword {
            // Escape LIKE metacharacters so %, _ in the keyword are treated as literals.
            let escaped = keyword
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let like_pattern = format!("%{escaped}%");
            params_vec.push(Box::new(like_pattern.clone()));
            let p1 = params_vec.len();
            params_vec.push(Box::new(like_pattern));
            let p2 = params_vec.len();
            where_clauses.push(format!(
                "(input LIKE ?{p1} ESCAPE '\\' OR output LIKE ?{p2} ESCAPE '\\')"
            ));
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };

        let count_sql = format!("SELECT COUNT(*) FROM tool_calls {where_sql}");
        let total: u64 = self.conn.query_row(
            &count_sql,
            rusqlite::params_from_iter(params_vec.iter().map(|p| p.as_ref())),
            |r| r.get(0),
        )?;

        let offset = (page.saturating_sub(1)) * per_page;
        params_vec.push(Box::new(per_page));
        params_vec.push(Box::new(offset));
        let query_sql = format!(
            "SELECT id, agent_id, session_id, trace_id, llm_call_id,
                    step, tool_name, tool_source, skill_name,
                    input, output, success, non_zero_exit,
                    latency_ms, error_message, created_at
             FROM tool_calls {where_sql}
             ORDER BY created_at DESC, step ASC LIMIT ?{} OFFSET ?{}",
            params_vec.len() - 1,
            params_vec.len()
        );
        let mut stmt = self.conn.prepare(&query_sql)?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(params_vec.iter().map(|p| p.as_ref())),
                Self::row_to_tool_call,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((rows, total))
    }

    pub fn get_llm_call_by_id(&self, id: &str) -> Result<Option<LlmCallRow>> {
        self.conn
            .query_row(
                "SELECT id, agent_id, session_id, trace_id, provider, model,
                        input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                        latency_ms, stop_reason, status, error_message, step, prompt_variant, created_at,
                        response_text, reasoning
                 FROM llm_calls WHERE id = ?1",
                params![id],
                Self::row_to_llm_call_detail,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn get_tool_calls_by_llm_call_id(&self, llm_call_id: &str) -> Result<Vec<ToolCallRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, agent_id, session_id, trace_id, llm_call_id,
                    step, tool_name, tool_source, skill_name,
                    input, output, success, non_zero_exit,
                    latency_ms, error_message, created_at
             FROM tool_calls WHERE llm_call_id = ?1 ORDER BY created_at ASC, step ASC",
        )?;
        let rows = stmt
            .query_map(params![llm_call_id], Self::row_to_tool_call)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_tool_call_by_id(&self, id: &str) -> Result<Option<ToolCallRow>> {
        self.conn
            .query_row(
                "SELECT id, agent_id, session_id, trace_id, llm_call_id,
                        step, tool_name, tool_source, skill_name,
                        input, output, success, non_zero_exit,
                        latency_ms, error_message, created_at
                 FROM tool_calls WHERE id = ?1",
                params![id],
                Self::row_to_tool_call,
            )
            .optional()
            .map_err(Into::into)
    }

    fn row_to_llm_call(r: &rusqlite::Row<'_>) -> rusqlite::Result<LlmCallRow> {
        Ok(LlmCallRow {
            id: r.get(0)?,
            agent_id: r.get(1)?,
            session_id: r.get(2)?,
            trace_id: r.get(3)?,
            provider: r.get(4)?,
            model: r.get(5)?,
            input_tokens: r.get(6)?,
            output_tokens: r.get(7)?,
            cache_read_tokens: r.get(8)?,
            cache_write_tokens: r.get(9)?,
            latency_ms: r.get(10)?,
            stop_reason: r.get(11)?,
            status: r.get(12)?,
            error_message: r.get(13)?,
            step: r.get(14)?,
            prompt_variant: r.get(15)?,
            created_at: r.get(16)?,
            // List queries omit response_text/reasoning for performance
            response_text: None,
            reasoning: None,
            // Boolean indicators from `response_text IS NOT NULL` / `reasoning IS NOT NULL`
            has_response_text: r.get(17)?,
            has_reasoning: r.get(18)?,
            cost_usd: None,
        })
    }

    /// Detail query includes response_text and reasoning columns.
    fn row_to_llm_call_detail(r: &rusqlite::Row<'_>) -> rusqlite::Result<LlmCallRow> {
        let response_text: Option<String> = r.get(17)?;
        let reasoning: Option<String> = r.get(18)?;
        Ok(LlmCallRow {
            id: r.get(0)?,
            agent_id: r.get(1)?,
            session_id: r.get(2)?,
            trace_id: r.get(3)?,
            provider: r.get(4)?,
            model: r.get(5)?,
            input_tokens: r.get(6)?,
            output_tokens: r.get(7)?,
            cache_read_tokens: r.get(8)?,
            cache_write_tokens: r.get(9)?,
            latency_ms: r.get(10)?,
            stop_reason: r.get(11)?,
            status: r.get(12)?,
            error_message: r.get(13)?,
            step: r.get(14)?,
            prompt_variant: r.get(15)?,
            created_at: r.get(16)?,
            has_response_text: response_text.is_some(),
            has_reasoning: reasoning.is_some(),
            response_text,
            reasoning,
            cost_usd: None,
        })
    }

    fn row_to_tool_call(r: &rusqlite::Row<'_>) -> rusqlite::Result<ToolCallRow> {
        Ok(ToolCallRow {
            id: r.get(0)?,
            agent_id: r.get(1)?,
            session_id: r.get(2)?,
            trace_id: r.get(3)?,
            llm_call_id: r.get(4)?,
            step: r.get(5)?,
            tool_name: r.get(6)?,
            tool_source: r.get(7)?,
            skill_name: r.get(8)?,
            input: r.get(9)?,
            output: r.get(10)?,
            success: r.get(11)?,
            non_zero_exit: r.get(12)?,
            latency_ms: r.get(13)?,
            error_message: r.get(14)?,
            created_at: r.get(15)?,
        })
    }

    /// Update session metadata column.
    pub fn update_session_metadata(&self, session_id: &str, metadata: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET metadata = ?1 WHERE id = ?2",
            params![metadata, session_id],
        )?;
        Ok(())
    }

    // ===== Messages =====

    pub fn save_message(
        &self,
        agent_id: &str,
        session_id: &str,
        role: &str,
        content: &str,
        trace_id: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO messages (session_id, agent_id, role, content, trace_id)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session_id, agent_id, role, content, trace_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn save_message_with_metadata(
        &self,
        agent_id: &str,
        session_id: &str,
        role: &str,
        content: &str,
        metadata: Option<&str>,
        trace_id: Option<&str>,
        internal: bool,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO messages (session_id, agent_id, role, content, metadata, trace_id, internal)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![session_id, agent_id, role, content, metadata, trace_id, internal as i64],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert a single row into `task_messages` inside a caller-provided transaction.
    /// Used by `save_message_with_task_context` for the double-write contract (mika#974).
    #[allow(clippy::too_many_arguments)]
    fn insert_task_message_tx(
        tx: &rusqlite::Transaction<'_>,
        task_id: &str,
        agent_id: &str,
        session_id: &str,
        role: &str,
        content: &str,
        metadata: Option<&str>,
        trace_id: Option<&str>,
    ) -> Result<i64> {
        tx.execute(
            "INSERT INTO task_messages (task_id, agent_id, session_id, role, content, metadata, trace_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![task_id, agent_id, session_id, role, content, metadata, trace_id],
        )?;
        Ok(tx.last_insert_rowid())
    }

    /// Insert a single row into `task_messages` without a transaction.
    /// Used by the dispatcher to write engine-internal task narrative (e.g., callback
    /// summaries) that should NOT appear in `messages` (mika#965).
    #[allow(clippy::too_many_arguments)]
    pub fn insert_task_message(
        &self,
        task_id: &str,
        agent_id: &str,
        session_id: &str,
        role: &str,
        content: &str,
        metadata: Option<&str>,
        trace_id: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO task_messages (task_id, agent_id, session_id, role, content, metadata, trace_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![task_id, agent_id, session_id, role, content, metadata, trace_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Double-write: insert into `messages` AND `task_messages` in a single transaction.
    /// When `task_id` is `None`, behaves identically to `save_message_with_metadata`
    /// (no transaction overhead, no `task_messages` row).
    #[allow(clippy::too_many_arguments)]
    pub fn save_message_with_task_context(
        &mut self,
        agent_id: &str,
        session_id: &str,
        role: &str,
        content: &str,
        metadata: Option<&str>,
        trace_id: Option<&str>,
        internal: bool,
        task_id: Option<&str>,
    ) -> Result<i64> {
        match task_id {
            Some(tid) => {
                let tx = self.conn.transaction()?;
                tx.execute(
                    "INSERT INTO messages (session_id, agent_id, role, content, metadata, trace_id, internal)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![session_id, agent_id, role, content, metadata, trace_id, internal as i64],
                )?;
                let msg_id = tx.last_insert_rowid();
                Self::insert_task_message_tx(
                    &tx, tid, agent_id, session_id, role, content, metadata, trace_id,
                )?;
                tx.commit()?;
                Ok(msg_id)
            }
            None => self.save_message_with_metadata(
                agent_id, session_id, role, content, metadata, trace_id, internal,
            ),
        }
    }

    /// Load all task messages for a given task, ordered by creation time.
    /// Returns the full narrative — no limit, no compaction.
    pub fn load_task_messages(&self, task_id: &str) -> Result<Vec<TaskMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_id, agent_id, session_id, role, content, metadata, trace_id, created_at
             FROM task_messages
             WHERE task_id = ?1
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt
            .query_map(params![task_id], |r| {
                Ok(TaskMessage {
                    id: r.get(0)?,
                    task_id: r.get(1)?,
                    agent_id: r.get(2)?,
                    session_id: r.get(3)?,
                    role: r.get(4)?,
                    content: r.get(5)?,
                    metadata: r.get(6)?,
                    trace_id: r.get(7)?,
                    created_at: r.get(8)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    const SESSION_MESSAGE_COLUMNS: &'static str = "m.id, m.session_id, m.agent_id, m.role, m.content, s.channel_type, m.metadata, m.trace_id, m.created_at, m.internal";

    fn row_to_session_message(r: &rusqlite::Row<'_>) -> rusqlite::Result<SessionMessage> {
        Ok(SessionMessage {
            id: r.get(0)?,
            session_id: r.get(1)?,
            agent_id: r.get(2)?,
            role: r.get(3)?,
            content: r.get(4)?,
            channel_type: r.get(5)?,
            metadata: r.get(6)?,
            trace_id: r.get(7)?,
            created_at: r.get(8)?,
            internal: r.get::<_, i64>(9).unwrap_or(0) != 0,
        })
    }

    pub fn load_recent_messages(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<SessionMessage>> {
        let (msgs, _) = self.load_recent_messages_filtered(agent_id, None, limit, false)?;
        Ok(msgs)
    }

    /// Build the recent-messages query (mika#2295).
    ///
    /// Two SQL strings rather than one with `AND (?3 IS NULL OR m.session_id = ?3)`,
    /// for two reasons. The unscoped string stays **byte-for-byte** what it was
    /// before mika#2295, which is what makes "the default is unchanged" (AC2) a
    /// fact a test can assert rather than a claim. And a neutralised `OR` predicate
    /// is not sargable, so it would quietly cost both paths the index each one
    /// wants — and there is one for each: `idx_msg_agent_created(agent_id,
    /// created_at DESC)` for the unscoped form, `idx_msg_session(session_id,
    /// created_at ASC)` for the scoped one, whose leading column is the far more
    /// selective of the two (one agent has many sessions) and whose `created_at`
    /// SQLite can walk backwards to serve the `DESC` ordering without a sort.
    /// Neither index is new; the split is what keeps them reachable.
    ///
    /// Pure and `pub(crate)` so the AC2 equality is assertable without a database.
    pub(crate) fn recent_messages_sql(columns: &str, session_scoped: bool) -> String {
        if session_scoped {
            format!(
                "SELECT {columns} FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.agent_id = ?1 AND m.role != 'summary' AND s.channel_type != 'team'
                AND m.session_id = ?3
              ORDER BY m.created_at DESC, m.id DESC LIMIT ?2"
            )
        } else {
            format!(
                "SELECT {columns} FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.agent_id = ?1 AND m.role != 'summary' AND s.channel_type != 'team'
              ORDER BY m.created_at DESC, m.id DESC LIMIT ?2"
            )
        }
    }

    /// Rebuild conversation context for prompt assembly (mika#974).
    ///
    /// - `task_id = None` → existing `load_recent_messages` path (channel-mode).
    /// - `task_id = Some(tid)` → hybrid merge: load both `task_messages` (full narrative)
    ///   and `messages` (recent channel context), merge sorted by `created_at`,
    ///   dedup on `(session_id, role, content, created_at)`.
    ///
    /// `session_id` scopes the **channel** window only (mika#2295). The task
    /// narrative is deliberately untouched by it: `task_messages` are already
    /// scoped by `task_id`, so they are the current task's own story, not other
    /// tickets' — which is the contamination `session_id` exists to cut. Note the
    /// narrative is loaded with no limit at all, so the byte ceiling applied by
    /// the caller is what bounds it.
    pub fn rebuild_context(
        &self,
        agent_id: &str,
        session_id: Option<&str>,
        task_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SessionMessage>> {
        let load_channel = |db: &Self| -> Result<Vec<SessionMessage>> {
            let (msgs, _) = db.load_recent_messages_filtered(agent_id, session_id, limit, false)?;
            Ok(msgs)
        };

        let tid = match task_id {
            Some(t) => t,
            None => return load_channel(self),
        };

        // Load channel messages (recent window).
        let channel_msgs = load_channel(self)?;

        // Load task narrative (full history, no limit).
        let task_msgs = self.load_task_messages(tid)?;

        // Convert TaskMessages to SessionMessages for uniform handling.
        // task_messages don't have channel_type or internal — use sensible defaults.
        let task_as_session: Vec<SessionMessage> = task_msgs
            .into_iter()
            .map(|tm| SessionMessage {
                id: tm.id,
                session_id: tm.session_id,
                agent_id: tm.agent_id,
                role: tm.role,
                content: tm.content,
                channel_type: String::new(),
                metadata: tm.metadata,
                trace_id: tm.trace_id,
                created_at: tm.created_at,
                internal: false,
            })
            .collect();

        // Merge both sets, dedup on (session_id, role, content, created_at),
        // sort by created_at ASC.
        let mut seen = std::collections::HashSet::new();
        let mut merged: Vec<SessionMessage> =
            Vec::with_capacity(channel_msgs.len() + task_as_session.len());

        for msg in task_as_session.into_iter().chain(channel_msgs.into_iter()) {
            let key = (
                msg.session_id.clone(),
                msg.role.clone(),
                msg.content.clone(),
                msg.created_at.clone(),
            );
            if seen.insert(key) {
                merged.push(msg);
            }
        }

        merged.sort_by(|a, b| a.created_at.cmp(&b.created_at));

        Ok(merged)
    }

    /// Load recent messages with optional internal-message filtering.
    ///
    /// Returns `(visible_messages, hidden_internal_count)`. When `exclude_internal`
    /// is true, internal messages within the limit-bound window are counted but
    /// excluded from the returned Vec. The count is best-effort: it reflects
    /// internals discarded from the limit-bound window, not the total across all
    /// history. When `exclude_internal` is false, the count is always 0.
    ///
    /// `session_id = Some(id)` restricts the window to one session (mika#2295).
    /// `None` reproduces the pre-mika#2295 query byte for byte — see
    /// [`Self::recent_messages_sql`].
    ///
    /// **The restriction has to be in the query, and that is not a preference.**
    /// Filtering twenty already-loaded rows would return an amputated window the
    /// moment other sessions' messages interleave past the limit — which is
    /// precisely what 59 grooms a day do. A post-load filter is correct at rest
    /// and wrong under exactly the load that produced mika#2295: the worse of the
    /// two, because it would look like it worked.
    pub fn load_recent_messages_filtered(
        &self,
        agent_id: &str,
        session_id: Option<&str>,
        limit: usize,
        exclude_internal: bool,
    ) -> Result<(Vec<SessionMessage>, usize)> {
        // Always fetch without the internal filter so we can count hidden rows.
        let sql = Self::recent_messages_sql(Self::SESSION_MESSAGE_COLUMNS, session_id.is_some());
        let mut stmt = self.conn.prepare(&sql)?;
        let all_rows = match session_id {
            Some(sid) => stmt
                .query_map(
                    params![agent_id, limit as i64, sid],
                    Self::row_to_session_message,
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?,
            None => stmt
                .query_map(
                    params![agent_id, limit as i64],
                    Self::row_to_session_message,
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        };

        if !exclude_internal {
            let mut messages = all_rows;
            messages.reverse();
            return Ok((messages, 0));
        }

        // Partition: visible messages and count of hidden internals.
        let mut hidden_count = 0usize;
        let mut messages = Vec::with_capacity(all_rows.len());
        for msg in all_rows {
            if msg.internal {
                hidden_count += 1;
            } else {
                messages.push(msg);
            }
        }
        messages.reverse();
        Ok((messages, hidden_count))
    }

    pub fn load_conversation_summary(&self, agent_id: &str) -> Result<Option<SessionMessage>> {
        self.conn
            .query_row(
                &format!(
                    "SELECT {} FROM messages m JOIN sessions s ON m.session_id = s.id
                      WHERE m.agent_id = ?1 AND m.role = 'summary'
                      ORDER BY m.created_at DESC LIMIT 1",
                    Self::SESSION_MESSAGE_COLUMNS
                ),
                params![agent_id],
                Self::row_to_session_message,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn count_messages(&self, agent_id: &str) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE agent_id = ?1 AND role != 'summary'",
            params![agent_id],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    pub fn load_messages_before_window(
        &self,
        agent_id: &str,
        window_size: usize,
    ) -> Result<Vec<SessionMessage>> {
        let cutoff_id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM messages WHERE agent_id = ?1 AND role != 'summary'
                  ORDER BY created_at DESC, id DESC LIMIT 1 OFFSET ?2",
                params![agent_id, window_size as i64],
                |r| r.get(0),
            )
            .optional()?;
        let cutoff_id = match cutoff_id {
            Some(id) => id,
            None => return Ok(vec![]),
        };
        let sql = format!(
            "SELECT {} FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.agent_id = ?1 AND m.role != 'summary' AND m.id <= ?2
              ORDER BY m.created_at ASC, m.id ASC",
            Self::SESSION_MESSAGE_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id, cutoff_id], Self::row_to_session_message)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn replace_with_summary(
        &mut self,
        agent_id: &str,
        summary: &str,
        compacted_through_id: i64,
    ) -> Result<i64> {
        let system_session = self.get_or_create_system_session(agent_id)?;
        // RAII transaction: Drop without commit() auto-rolls back, preventing
        // stuck transactions that pin the WAL snapshot (mika#636).
        let tx = self.conn.transaction()?;
        // Delete old non-summary messages up to compacted_through_id
        tx.execute(
            "DELETE FROM messages
             WHERE agent_id = ?1 AND role != 'summary' AND id <= ?2",
            params![agent_id, compacted_through_id],
        )?;
        // Remove old summary
        tx.execute(
            "DELETE FROM messages WHERE agent_id = ?1 AND role = 'summary'",
            params![agent_id],
        )?;
        // Insert new summary (no trace_id — summaries span multiple traces)
        tx.execute(
            "INSERT INTO messages (session_id, agent_id, role, content, compacted_through_id)
             VALUES (?1, ?2, 'summary', ?3, ?4)",
            params![system_session, agent_id, summary, compacted_through_id],
        )?;
        let row_id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(row_id)
    }

    pub fn load_messages_after(
        &self,
        agent_id: &str,
        after_id: i64,
    ) -> Result<Vec<SessionMessage>> {
        let sql = format!(
            "SELECT {} FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.agent_id = ?1 AND m.id > ?2
              ORDER BY m.created_at ASC, m.id ASC",
            Self::SESSION_MESSAGE_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id, after_id], Self::row_to_session_message)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn max_message_id(&self, agent_id: &str) -> Result<i64> {
        let id: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(id), 0) FROM messages WHERE agent_id = ?1",
            params![agent_id],
            |r| r.get(0),
        )?;
        Ok(id)
    }

    /// Load a single message by its row ID.
    pub fn get_message_by_id(&self, message_id: i64) -> Result<Option<SessionMessage>> {
        let sql = format!(
            "SELECT {} FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.id = ?1",
            Self::SESSION_MESSAGE_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt
            .query_map(params![message_id], Self::row_to_session_message)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.pop())
    }

    /// Load messages surrounding a given message ID within the same session.
    /// Returns up to `before` messages before and `after` messages after the target.
    pub fn get_surrounding_messages(
        &self,
        session_id: &str,
        target_id: i64,
        before: u32,
        after: u32,
    ) -> Result<Vec<SessionMessage>> {
        let sql = format!(
            "SELECT {} FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.session_id = ?1
                AND (m.id >= (SELECT id FROM (SELECT id FROM messages WHERE session_id = ?1 AND id <= ?2 ORDER BY id DESC LIMIT ?3) sub ORDER BY id ASC LIMIT 1))
                AND m.id <= (SELECT id FROM (SELECT id FROM messages WHERE session_id = ?1 AND id >= ?2 ORDER BY id ASC LIMIT ?4) sub ORDER BY id DESC LIMIT 1)
              ORDER BY m.created_at ASC, m.id ASC",
            Self::SESSION_MESSAGE_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(
                params![session_id, target_id, before + 1, after + 1],
                Self::row_to_session_message,
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn get_messages_since(&self, agent_id: &str, since: &str) -> Result<Vec<SessionMessage>> {
        let sql = format!(
            "SELECT {} FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.agent_id = ?1 AND m.created_at >= ?2 AND m.role != 'summary'
              ORDER BY m.created_at ASC",
            Self::SESSION_MESSAGE_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id, since], Self::row_to_session_message)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn last_user_message_time(&self, agent_id: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT MAX(created_at) FROM messages
                  WHERE agent_id = ?1 AND role = 'user'",
                params![agent_id],
                |r| r.get(0),
            )
            .optional()
            .map(|opt| opt.flatten())
            .map_err(Into::into)
    }

    // ===== Core Memory =====

    pub fn seed_core_memory(&self, agent_id: &str, user_md_content: Option<&str>) -> Result<()> {
        for (key, default) in CORE_MEMORY_SECTIONS {
            let existing = self.get_core_memory(agent_id, key)?;
            if existing.is_none() {
                self.set_core_memory(agent_id, key, default)?;
            }
        }
        // Override self_model with agent-aware default (only if still at static default)
        if let Some(entry) = self.get_core_memory(agent_id, "self_model")?
            && entry.value == "No interaction history yet."
        {
            let display_name = self.get_agent_display_name(agent_id);
            self.set_core_memory(agent_id, "self_model", &default_self_model(&display_name))?;
        }
        if let Some(md) = user_md_content
            && !md.trim().is_empty()
        {
            self.set_core_memory(agent_id, "user_summary", md.trim())?;
        }
        Ok(())
    }

    /// Migrate legacy `persona` key to `self_model` for an agent.
    pub fn migrate_persona_to_self_model(&self, agent_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE core_memory SET key = 'self_model' WHERE agent_id = ?1 AND key = 'persona'",
            params![agent_id],
        )?;
        Ok(())
    }

    pub fn total_core_memory_tokens(&self, agent_id: &str) -> Result<i32> {
        let n: i32 = self.conn.query_row(
            "SELECT COALESCE(SUM(token_count), 0) FROM core_memory WHERE agent_id = ?1",
            params![agent_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    // ===== People =====

    pub fn search_people(&self, agent_id: &str, query: &str) -> Result<Vec<Person>> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT id, canonical_name, relationship, notes,
                     first_mentioned,
                     last_mentioned,
                     mention_count
              FROM people
              WHERE agent_id = ?1 AND (
                  canonical_name LIKE ?2 OR relationship LIKE ?2 OR notes LIKE ?2
              )
              ORDER BY canonical_name",
        )?;
        let rows = stmt
            .query_map(params![agent_id, pattern], |r| {
                Ok(Person {
                    id: r.get(0)?,
                    canonical_name: r.get(1)?,
                    relationship: r.get(2)?,
                    notes: r.get(3)?,
                    first_mentioned: r.get(4)?,
                    last_mentioned: r.get(5)?,
                    mention_count: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // ===== Commitments =====

    pub fn add_commitment(
        &self,
        agent_id: &str,
        description: &str,
        due_date: Option<&str>,
        person_id: Option<i64>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO commitments (agent_id, description, due_date, person_id)
             VALUES (?1, ?2, ?3, ?4)",
            params![agent_id, description, due_date, person_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    fn row_to_commitment(r: &rusqlite::Row<'_>) -> rusqlite::Result<Commitment> {
        Ok(Commitment {
            id: r.get(0)?,
            description: r.get(1)?,
            status: r.get(2)?,
            due_date: r.get(3)?,
            person_id: r.get(4)?,
            created_at: r.get(5)?,
            completed_at: r.get(6)?,
        })
    }

    pub fn list_commitments(&self, agent_id: &str, status: &str) -> Result<Vec<Commitment>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, description, status, due_date, person_id,
                     created_at,
                     completed_at
              FROM commitments WHERE agent_id = ?1 AND status = ?2
              ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![agent_id, status], Self::row_to_commitment)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn update_commitment_status(&self, agent_id: &str, id: i64, status: &str) -> Result<bool> {
        let completed_at: Option<String> = if status == "completed" {
            Some(timestamp::now())
        } else {
            None
        };
        let n = self.conn.execute(
            "UPDATE commitments SET status = ?1, completed_at = ?2
             WHERE agent_id = ?3 AND id = ?4",
            params![status, completed_at, agent_id, id],
        )?;
        Ok(n > 0)
    }

    pub fn get_commitment_status(&self, agent_id: &str, id: i64) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT status FROM commitments WHERE agent_id = ?1 AND id = ?2",
                params![agent_id, id],
                |r| r.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn get_commitment_details(
        &self,
        agent_id: &str,
        id: i64,
    ) -> Result<Option<(String, Option<String>)>> {
        self.conn
            .query_row(
                "SELECT description, due_date FROM commitments WHERE agent_id = ?1 AND id = ?2",
                params![agent_id, id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn search_commitments(&self, agent_id: &str, query: &str) -> Result<Vec<Commitment>> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT id, description, status, due_date, person_id,
                     created_at,
                     completed_at
              FROM commitments WHERE agent_id = ?1 AND description LIKE ?2
              ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![agent_id, pattern], Self::row_to_commitment)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // ===== Preferences =====

    pub fn set_preference(&self, agent_id: &str, category: &str, value: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO preferences (agent_id, category, value) VALUES (?1, ?2, ?3)
             ON CONFLICT(agent_id, category) DO UPDATE SET
                 value = excluded.value, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
            params![agent_id, category, value],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn search_preferences(&self, agent_id: &str, query: &str) -> Result<Vec<Preference>> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT category, value, updated_at
              FROM preferences WHERE agent_id = ?1 AND (category LIKE ?2 OR value LIKE ?2)
              ORDER BY category",
        )?;
        let rows = stmt
            .query_map(params![agent_id, pattern], |r| {
                Ok(Preference {
                    category: r.get(0)?,
                    value: r.get(1)?,
                    updated_at: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // ===== Events =====

    pub fn add_event(
        &self,
        agent_id: &str,
        description: &str,
        event_date: Option<&str>,
        context: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO events (agent_id, description, event_date, context) VALUES (?1, ?2, ?3, ?4)",
            params![agent_id, description, event_date, context],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn search_events(&self, agent_id: &str, query: &str) -> Result<Vec<Event>> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT id, description, event_date, context, created_at
              FROM events WHERE agent_id = ?1 AND (description LIKE ?2 OR context LIKE ?2)
              ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![agent_id, pattern], |r| {
                Ok(Event {
                    id: r.get(0)?,
                    description: r.get(1)?,
                    event_date: r.get(2)?,
                    context: r.get(3)?,
                    created_at: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // ===== Auto-Pull Stats (mika#1363) =====

    /// Get the failure count for a specific issue in `auto_pull_stats`.
    /// Returns 0 if no row exists.
    pub fn get_auto_pull_failure_count(
        &self,
        repo_full_name: &str,
        issue_number: u64,
    ) -> Result<i64> {
        let n: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(failure_count, 0) FROM auto_pull_stats
                 WHERE repo_full_name = ?1 AND issue_number = ?2",
                params![repo_full_name, issue_number as i64],
                |r| r.get(0),
            )
            .unwrap_or(0);
        Ok(n)
    }

    /// Record an auto-pull event: upsert the row with `last_auto_pull_at = now`.
    pub fn record_auto_pull(&self, repo_full_name: &str, issue_number: u64) -> Result<()> {
        let now = crate::timestamp::now();
        self.conn.execute(
            "INSERT INTO auto_pull_stats (repo_full_name, issue_number, failure_count, last_auto_pull_at)
             VALUES (?1, ?2, 0, ?3)
             ON CONFLICT(repo_full_name, issue_number) DO UPDATE SET last_auto_pull_at = ?3",
            params![repo_full_name, issue_number as i64, now],
        )?;
        Ok(())
    }

    /// Increment the failure counter for a ticket (circuit-breaker).
    pub fn increment_auto_pull_failure(
        &self,
        repo_full_name: &str,
        issue_number: u64,
    ) -> Result<()> {
        let now = crate::timestamp::now();
        self.conn.execute(
            "INSERT INTO auto_pull_stats (repo_full_name, issue_number, failure_count, last_failure_at)
             VALUES (?1, ?2, 1, ?3)
             ON CONFLICT(repo_full_name, issue_number)
             DO UPDATE SET failure_count = failure_count + 1, last_failure_at = ?3",
            params![repo_full_name, issue_number as i64, now],
        )?;
        Ok(())
    }

    /// Reset the failure counter for a ticket (on success or operator-driven ready).
    ///
    /// Deliberately leaves `redrive_count` alone (mika#2020): this runs on every
    /// successful rescue, which is precisely the event the re-drive budget must
    /// count. Merging the two counters would rebuild the bug that budget closes.
    pub fn reset_auto_pull_failure(&self, repo_full_name: &str, issue_number: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE auto_pull_stats SET failure_count = 0, last_failure_at = NULL
             WHERE repo_full_name = ?1 AND issue_number = ?2",
            params![repo_full_name, issue_number as i64],
        )?;
        Ok(())
    }

    // ===== Auto-Pull Re-Drive Budget (mika#2020) =====

    /// Read a ticket's re-drive state: `(redrive_count, abandoned)`.
    /// Returns `(0, false)` when no row exists.
    pub fn get_auto_pull_redrive_state(
        &self,
        repo_full_name: &str,
        issue_number: u64,
    ) -> Result<(i64, bool)> {
        let state = self
            .conn
            .query_row(
                "SELECT COALESCE(redrive_count, 0), redrive_abandoned_at IS NOT NULL
                 FROM auto_pull_stats
                 WHERE repo_full_name = ?1 AND issue_number = ?2",
                params![repo_full_name, issue_number as i64],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, bool>(1)?)),
            )
            .unwrap_or((0, false));
        Ok(state)
    }

    /// The instant a ticket was abandoned by the re-drive reconciler, or `None`
    /// when it is not abandoned (mika#2361).
    ///
    /// Deliberately separate from [`Self::get_auto_pull_redrive_state`], which
    /// returns the boolean the Phase 2 decision needs. The *date* is not an input
    /// of that decision — it is only the bound of the re-entry-blocked comment's
    /// dedup window. Widening the existing tuple would have made
    /// `StuckReadyFacts`, and every test that builds one, carry a value the pure
    /// function has no use for.
    pub fn get_auto_pull_redrive_abandoned_at(
        &self,
        repo_full_name: &str,
        issue_number: u64,
    ) -> Result<Option<String>> {
        let at = self
            .conn
            .query_row(
                "SELECT redrive_abandoned_at
                 FROM auto_pull_stats
                 WHERE repo_full_name = ?1 AND issue_number = ?2",
                params![repo_full_name, issue_number as i64],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        Ok(at)
    }

    /// Increment a ticket's re-drive counter after a successful Phase 2 rescue.
    pub fn increment_auto_pull_redrive(
        &self,
        repo_full_name: &str,
        issue_number: u64,
    ) -> Result<()> {
        let now = crate::timestamp::now();
        self.conn.execute(
            "INSERT INTO auto_pull_stats (repo_full_name, issue_number, redrive_count, last_redrive_at)
             VALUES (?1, ?2, 1, ?3)
             ON CONFLICT(repo_full_name, issue_number)
             DO UPDATE SET redrive_count = redrive_count + 1, last_redrive_at = ?3",
            params![repo_full_name, issue_number as i64, now],
        )?;
        Ok(())
    }

    /// Clear a ticket's re-drive budget — on observed progress (an open PR
    /// closing it, or an in-flight self_dev task), or when the operator lifts an
    /// abandonment by removing the `operator-review` label.
    pub fn reset_auto_pull_redrive(&self, repo_full_name: &str, issue_number: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE auto_pull_stats
             SET redrive_count = 0, redrive_abandoned_at = NULL
             WHERE repo_full_name = ?1 AND issue_number = ?2",
            params![repo_full_name, issue_number as i64],
        )?;
        Ok(())
    }

    /// Stamp a ticket as abandoned by the re-drive reconciler. The stamp is what
    /// makes the abandonment a one-shot gesture per lifecycle, and what lets a
    /// later tick recognize the operator's re-entry.
    pub fn mark_auto_pull_redrive_abandoned(
        &self,
        repo_full_name: &str,
        issue_number: u64,
    ) -> Result<()> {
        let now = crate::timestamp::now();
        self.conn.execute(
            "INSERT INTO auto_pull_stats (repo_full_name, issue_number, redrive_abandoned_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(repo_full_name, issue_number)
             DO UPDATE SET redrive_abandoned_at = ?3",
            params![repo_full_name, issue_number as i64, now],
        )?;
        Ok(())
    }

    /// Count `update_core_memory` audit events in the most recent non-system session.
    pub fn count_core_memory_edits_latest_session(&self, agent_id: &str) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM audit_events
             WHERE agent_id = ?1
               AND tool_name = 'update_core_memory'
               AND session_id = (
                   SELECT id FROM sessions
                   WHERE agent_id = ?1
                     AND id NOT LIKE 'system-%'
                   ORDER BY started_at DESC
                   LIMIT 1
               )",
            params![agent_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    // ===== Rewind =====

    /// Get messages after a given message ID within a session.
    pub fn get_messages_after_id(
        &self,
        agent_id: &str,
        session_id: &str,
        after_id: i64,
    ) -> Result<Vec<SessionMessage>> {
        let sql = format!(
            "SELECT {} FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.agent_id = ?1 AND m.session_id = ?2 AND m.id > ?3
              ORDER BY m.id ASC",
            Self::SESSION_MESSAGE_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(
                params![agent_id, session_id, after_id],
                Self::row_to_session_message,
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Get the compaction boundary — the highest message ID that has been compacted.
    pub fn get_compaction_boundary(&self, agent_id: &str) -> Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT compacted_through_id FROM messages
                  WHERE agent_id = ?1 AND role = 'summary'
                  ORDER BY id DESC LIMIT 1",
                params![agent_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// Delete messages after a given ID in a session. Returns count deleted.
    pub fn delete_messages_after_id(
        &self,
        agent_id: &str,
        session_id: &str,
        after_id: i64,
    ) -> Result<usize> {
        let deleted = self.conn.execute(
            "DELETE FROM messages WHERE agent_id = ?1 AND session_id = ?2 AND id > ?3",
            params![agent_id, session_id, after_id],
        )?;
        Ok(deleted)
    }

    /// Delete rewind context marker messages from a session.
    /// Called before injecting a new marker to prevent accumulation during rapid rewinds.
    pub fn delete_rewind_markers(&self, agent_id: &str, session_id: &str) -> Result<usize> {
        let pattern = format!("{}%", crate::rewind::REWIND_MARKER_PREFIX);
        let deleted = self.conn.execute(
            "DELETE FROM messages WHERE agent_id = ?1 AND session_id = ?2 \
             AND role = 'system' AND content LIKE ?3",
            params![agent_id, session_id, pattern],
        )?;
        Ok(deleted)
    }

    /// Delete a commitment by description. Returns true if deleted.
    pub fn delete_commitment_by_description(
        &self,
        agent_id: &str,
        description: &str,
    ) -> Result<bool> {
        let commitment_id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM commitments WHERE agent_id = ?1 AND description = ?2 COLLATE NOCASE",
                params![agent_id, description],
                |r| r.get(0),
            )
            .optional()?;
        let Some(cid) = commitment_id else {
            return Ok(false);
        };
        self.delete_search_content(agent_id, "commitment", cid)?;
        let deleted = self.conn.execute(
            "DELETE FROM commitments WHERE id = ?1 AND agent_id = ?2",
            params![cid, agent_id],
        )?;
        Ok(deleted > 0)
    }

    /// Get tasks created by the given trace_ids (via created_trace_id column).
    /// Returns tasks ordered by created_at DESC.
    pub fn get_tasks_by_trace_ids(
        &self,
        agent_id: &str,
        trace_ids: &[String],
    ) -> Result<Vec<Task>> {
        if trace_ids.is_empty() {
            return Ok(vec![]);
        }
        let placeholders: Vec<String> = (0..trace_ids.len())
            .map(|i| format!("?{}", i + 2))
            .collect();
        let sql = format!(
            "SELECT {} FROM tasks WHERE agent_id = ?1 AND created_trace_id IN ({}) ORDER BY created_at DESC",
            Self::TASK_COLUMNS,
            placeholders.join(", ")
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        params_vec.push(Box::new(agent_id.to_string()));
        for tid in trace_ids {
            params_vec.push(Box::new(tid.clone()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(&*param_refs, Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Delete a task by ID. Returns true if deleted.
    /// Only deletes tasks that are not in a terminal actioned state.
    pub fn delete_task_by_id(&self, id: &str, agent_id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM tasks WHERE id = ?1 AND agent_id = ?2",
            params![id, agent_id],
        )?;
        Ok(n > 0)
    }

    // ===== Heartbeat =====

    pub fn record_heartbeat_send(&self, agent_id: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO heartbeat_sends (agent_id) VALUES (?1)",
            params![agent_id],
        )?;
        Ok(())
    }

    pub fn count_heartbeat_sends_last_hour(&self, agent_id: &str) -> Result<u32> {
        let n: u32 = self.conn.query_row(
            "SELECT COUNT(*) FROM heartbeat_sends
             WHERE agent_id = ?1 AND sent_at >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hour')",
            params![agent_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    pub fn count_heartbeat_sends_today(&self, agent_id: &str, timezone: &str) -> Result<u32> {
        let tz: Tz = timezone.parse().unwrap_or(chrono_tz::UTC);
        let now_local = Utc::now().with_timezone(&tz);
        let midnight_local = now_local
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap_or_default();
        let since_ts = tz
            .from_local_datetime(&midnight_local)
            .earliest()
            .map(|dt| timestamp::format(&dt.with_timezone(&Utc)))
            .unwrap_or_else(timestamp::now);
        let n: u32 = self.conn.query_row(
            "SELECT COUNT(*) FROM heartbeat_sends WHERE agent_id = ?1 AND sent_at >= ?2",
            params![agent_id, since_ts],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    pub fn prune_old_heartbeat_sends(&self, agent_id: &str, days: u32) -> Result<()> {
        let cutoff = timestamp::now_minus(Duration::days(days as i64));
        self.conn.execute(
            "DELETE FROM heartbeat_sends WHERE agent_id = ?1 AND sent_at < ?2",
            params![agent_id, cutoff],
        )?;
        Ok(())
    }

    // ===== Reflection =====

    pub fn record_reflection_run(
        &self,
        agent_id: &str,
        status: &str,
        changes_made: i64,
        summary: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO reflection_runs (agent_id, status, changes_made, summary)
             VALUES (?1, ?2, ?3, ?4)",
            params![agent_id, status, changes_made, summary],
        )?;
        Ok(())
    }

    pub fn last_reflection_run_today(&self, agent_id: &str, timezone: &str) -> Result<bool> {
        let tz: Tz = timezone.parse().unwrap_or(chrono_tz::UTC);
        let now_local = Utc::now().with_timezone(&tz);
        let midnight_local = now_local
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap_or_default();
        let since_ts = tz
            .from_local_datetime(&midnight_local)
            .earliest()
            .map(|dt| timestamp::format(&dt.with_timezone(&Utc)))
            .unwrap_or_else(timestamp::now);
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM reflection_runs
             WHERE agent_id = ?1 AND status = 'completed' AND created_at >= ?2",
            params![agent_id, since_ts],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn prune_old_reflection_runs(&self, agent_id: &str, days: u32) -> Result<usize> {
        let cutoff = timestamp::now_minus(Duration::days(days as i64));
        let n = self.conn.execute(
            "DELETE FROM reflection_runs WHERE agent_id = ?1 AND created_at < ?2",
            params![agent_id, cutoff],
        )?;
        Ok(n)
    }

    // ===== Failed Sends =====

    pub fn save_failed_send(
        &self,
        agent_id: &str,
        text: &str,
        request_id: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO failed_sends (agent_id, text, request_id) VALUES (?1, ?2, ?3)",
            params![agent_id, text, request_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_pending_failed_sends(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<FailedSend>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, text, request_id, created_at, retry_count
              FROM failed_sends WHERE agent_id = ?1
              ORDER BY created_at ASC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![agent_id, limit as i64], |r| {
                Ok(FailedSend {
                    id: r.get(0)?,
                    text: r.get(1)?,
                    request_id: r.get(2)?,
                    created_at: r.get(3)?,
                    retry_count: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn delete_failed_send(&self, agent_id: &str, id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM failed_sends WHERE agent_id = ?1 AND id = ?2",
            params![agent_id, id],
        )?;
        Ok(())
    }

    pub fn increment_failed_send_retry(&self, agent_id: &str, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE failed_sends SET retry_count = retry_count + 1 WHERE agent_id = ?1 AND id = ?2",
            params![agent_id, id],
        )?;
        Ok(())
    }

    // ===== Customer Config =====

    pub fn get_customer_config(&self, agent_id: &str, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM customer_config WHERE agent_id = ?1 AND key = ?2",
                params![agent_id, key],
                |r| r.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn set_customer_config(&self, agent_id: &str, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO customer_config (agent_id, key, value) VALUES (?1, ?2, ?3)
             ON CONFLICT(agent_id, key) DO UPDATE SET value = excluded.value, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
            params![agent_id, key, value],
        )?;
        Ok(())
    }

    pub fn list_customer_config(&self, agent_id: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT key, value FROM customer_config WHERE agent_id = ?1 ORDER BY key")?;
        let rows = stmt
            .query_map(params![agent_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // ===== Team Runs =====

    pub fn insert_team_run(
        &self,
        run_id: &str,
        team_name: &str,
        goal: &str,
        max_iterations: u32,
        started_at: &str,
        trace_id: Option<&str>,
    ) -> Result<()> {
        // Auto-register team (team_id = team_name)
        self.conn.execute(
            "INSERT OR IGNORE INTO teams (id, name) VALUES (?1, ?1)",
            params![team_name],
        )?;
        self.conn.execute(
            "INSERT INTO team_runs (id, team_id, goal, max_iterations, started_at, trace_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                run_id,
                team_name,
                goal,
                max_iterations,
                started_at,
                trace_id
            ],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_team_run(
        &self,
        run_id: &str,
        status: &str,
        failure_reason: Option<&str>,
        iteration: u32,
        deliverable: Option<&str>,
        ended_at: Option<&str>,
        delegation_count: u32,
        solo_absorption: bool,
        failure_context: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE team_runs SET status = ?1, failure_reason = ?2, iteration = ?3,
             deliverable = ?4, ended_at = ?5, delegation_count = ?6,
             solo_absorption = ?7, failure_context = ?8
             WHERE id = ?9",
            params![
                status,
                failure_reason,
                iteration,
                deliverable,
                ended_at,
                delegation_count,
                solo_absorption as i64,
                failure_context,
                run_id
            ],
        )?;
        Ok(())
    }

    /// Find team runs stuck in `status='running'` past `threshold_secs` whose
    /// child sessions show no recent liveness (mika#1652).
    ///
    /// A run is orphaned when no terminal-state writer ran (process died, the
    /// `run_team` future was dropped mid-flight at the tool timeout, etc.), so
    /// the row never left `running` and its team slot stays held. Liveness is
    /// proven by any `llm_calls` or `tool_calls` activity on a `team-<id>%`
    /// session within `liveness_threshold_secs` — the mika#959 `NOT EXISTS`
    /// watchdog pattern, adapted for non-process entities (team child sessions
    /// are LLM-call/tool-call rows, not subprocesses, so liveness is row
    /// recency rather than a `/proc/<pid>/stat` check).
    ///
    /// The `team-<id>%` LIKE prefix matches both the orchestrator session
    /// (`team-<id>`) and per-member sessions (`team-<id>-<agent>`); run ids are
    /// UUIDs, so the prefix never bleeds across runs.
    pub fn find_stuck_team_runs(
        &self,
        threshold_secs: i64,
        liveness_threshold_secs: i64,
    ) -> Result<Vec<TeamRunRow>> {
        let stuck_modifier = format!("-{threshold_secs} seconds");
        let liveness_modifier = format!("-{liveness_threshold_secs} seconds");
        let sql = format!(
            "SELECT {} FROM team_runs r JOIN teams t ON r.team_id = t.id
              WHERE r.status = 'running'
                AND r.started_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?1)
                AND NOT EXISTS (
                  SELECT 1 FROM llm_calls lc
                  WHERE lc.session_id LIKE 'team-' || r.id || '%'
                    AND lc.created_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
                )
                AND NOT EXISTS (
                  SELECT 1 FROM tool_calls tc
                  WHERE tc.session_id LIKE 'team-' || r.id || '%'
                    AND tc.created_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
                )
              ORDER BY r.started_at",
            Self::TEAM_RUN_COLUMNS,
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(
                params![stuck_modifier, liveness_modifier],
                Self::row_to_team_run,
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Transition a team run to a terminal status idempotently (mika#1652).
    ///
    /// The `WHERE status = 'running'` guard prevents double-transition and
    /// loses cleanly to the normal finalizer (`update_team_run`) or an operator
    /// cancel that won the race. Returns `true` when a row actually changed.
    ///
    /// `status` is `'failed'` for reaper-initiated transitions (system-level
    /// failure detection); `'cancelled'` is reserved for operator-initiated
    /// termination. Both are permitted by the `team_runs.status` CHECK
    /// constraint.
    pub fn transition_team_run_terminal(
        &self,
        team_run_id: &str,
        status: &str,
        failure_reason: &str,
    ) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE team_runs
             SET status = ?1,
                 failure_reason = ?2,
                 ended_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?3 AND status = 'running'",
            params![status, failure_reason, team_run_id],
        )?;
        Ok(changed > 0)
    }

    /// Suspend a team run, saving a serialized checkpoint for later resume.
    pub fn suspend_team_run(&self, run_id: &str, checkpoint: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE team_runs SET status = 'suspended', checkpoint = ?1 WHERE id = ?2",
            params![checkpoint, run_id],
        )?;
        Ok(())
    }

    /// Load the serialized checkpoint from a suspended team run.
    pub fn load_team_run_checkpoint(&self, run_id: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT checkpoint FROM team_runs WHERE id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .optional()
            .map(|o| o.flatten())
            .map_err(Into::into)
    }

    /// Resume a suspended team run (set status back to 'running').
    pub fn resume_team_run_status(&self, run_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE team_runs SET status = 'running', checkpoint = NULL WHERE id = ?1",
            params![run_id],
        )?;
        Ok(())
    }

    /// Load the trace_id from a team run (for resume continuity).
    pub fn load_team_run_trace_id(&self, run_id: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT trace_id FROM team_runs WHERE id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .optional()
            .map(|o| o.flatten())
            .map_err(Into::into)
    }

    const TEAM_RUN_COLUMNS: &'static str = "r.id, t.name, r.goal, r.status, r.failure_reason,
         r.iteration, r.max_iterations, r.deliverable, r.started_at, r.ended_at, r.trace_id,
         r.delegation_count, r.solo_absorption, r.failure_context";

    fn row_to_team_run(r: &rusqlite::Row<'_>) -> rusqlite::Result<TeamRunRow> {
        Ok(TeamRunRow {
            id: r.get(0)?,
            team_name: r.get(1)?,
            goal: r.get(2)?,
            status: r.get(3)?,
            failure_reason: r.get(4)?,
            iteration: r.get::<_, u32>(5)?,
            max_iterations: r.get::<_, u32>(6)?,
            deliverable: r.get(7)?,
            started_at: r.get(8)?,
            ended_at: r.get(9)?,
            trace_id: r.get(10)?,
            delegation_count: r.get::<_, u32>(11)?,
            solo_absorption: r.get::<_, i64>(12)? != 0,
            failure_context: r.get(13)?,
        })
    }

    pub fn load_team_runs(&self, team_name: &str, limit: usize) -> Result<Vec<TeamRunRow>> {
        let sql = format!(
            "SELECT {} FROM team_runs r JOIN teams t ON r.team_id = t.id
              WHERE t.name = ?1
              ORDER BY r.started_at DESC LIMIT ?2",
            Self::TEAM_RUN_COLUMNS,
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![team_name, limit as i64], Self::row_to_team_run)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn load_team_runs_for_prompt(
        &self,
        team_name: &str,
        limit: usize,
        max_text_len: usize,
    ) -> Result<Vec<TeamRunRow>> {
        let mut runs = self.load_team_runs(team_name, limit)?;
        for run in &mut runs {
            if let Some(ref d) = run.deliverable
                && d.len() > max_text_len
            {
                run.deliverable = Some(format!("{}...", &d[..max_text_len]));
            }
        }
        Ok(runs)
    }

    pub fn load_latest_team_run(&self, team_name: &str) -> Result<Option<TeamRunRow>> {
        let sql = format!(
            "SELECT {} FROM team_runs r JOIN teams t ON r.team_id = t.id
              WHERE t.name = ?1
              ORDER BY r.started_at DESC LIMIT 1",
            Self::TEAM_RUN_COLUMNS,
        );
        self.conn
            .query_row(&sql, params![team_name], Self::row_to_team_run)
            .optional()
            .map_err(Into::into)
    }

    pub fn load_team_run_by_id(&self, run_id: &str) -> Result<Option<TeamRunRow>> {
        let sql = format!(
            "SELECT {} FROM team_runs r JOIN teams t ON r.team_id = t.id
              WHERE r.id = ?1",
            Self::TEAM_RUN_COLUMNS,
        );
        self.conn
            .query_row(&sql, params![run_id], Self::row_to_team_run)
            .optional()
            .map_err(Into::into)
    }

    /// Load the most recent team run that is not running or cancelled.
    /// Returns completed, failed, or suspended runs only.
    pub fn get_last_completed_team_run(&self, team_name: &str) -> Result<Option<TeamRunRow>> {
        let sql = format!(
            "SELECT {} FROM team_runs r JOIN teams t ON r.team_id = t.id
              WHERE t.name = ?1 COLLATE NOCASE
                AND r.status IN ('completed', 'failed', 'suspended')
              ORDER BY r.started_at DESC LIMIT 1",
            Self::TEAM_RUN_COLUMNS,
        );
        self.conn
            .query_row(&sql, params![team_name], Self::row_to_team_run)
            .optional()
            .map_err(Into::into)
    }

    /// Load the most recent finished team run (completed, failed, or cancelled).
    /// Excludes running and suspended runs — only truly finished runs are returned.
    pub fn get_last_finished_team_run(&self, team_name: &str) -> Result<Option<TeamRunRow>> {
        let sql = format!(
            "SELECT {} FROM team_runs r JOIN teams t ON r.team_id = t.id
              WHERE t.name = ?1 COLLATE NOCASE
                AND r.status NOT IN ('running', 'suspended')
              ORDER BY r.started_at DESC LIMIT 1",
            Self::TEAM_RUN_COLUMNS,
        );
        self.conn
            .query_row(&sql, params![team_name], Self::row_to_team_run)
            .optional()
            .map_err(Into::into)
    }

    /// Build an enriched summary of a team run for context injection.
    /// Queries team_workspace (assignments, critic), messages (agent responses),
    /// and tasks (statuses) for the given run.
    pub fn get_team_run_summary(&self, run_id: &str) -> Result<Option<TeamRunSummary>> {
        let run = match self.load_team_run_by_id(run_id)? {
            Some(r) => r,
            None => return Ok(None),
        };

        // Get agent names from assignment entries
        let agent_names: Vec<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT DISTINCT agent_name FROM team_workspace
                 WHERE run_id = ?1 AND entry_type = 'assignment' AND agent_name IS NOT NULL",
            )?;
            stmt.query_map(params![run_id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };

        // Get the last assistant message for each agent (via team session IDs)
        let mut agent_results = Vec::new();
        for agent_name in &agent_names {
            let session_id = format!("team-{}-{}", run_id, agent_name);
            let response: Option<String> = self
                .conn
                .query_row(
                    "SELECT content FROM messages
                     WHERE session_id = ?1 AND role = 'assistant'
                     ORDER BY created_at DESC LIMIT 1",
                    params![session_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(content) = response {
                let preview = truncate_chars(&content, 200);
                agent_results.push(AgentResultSummary {
                    agent_name: agent_name.clone(),
                    response_preview: preview,
                });
            }
        }

        // Cap at 5 agents to keep context concise
        agent_results.truncate(5); // safe-byte-slice: Vec — element count, no char boundary

        // Get task statuses
        let all_tasks: Vec<TaskStatusSummary> = {
            let mut stmt = self.conn.prepare(
                "SELECT agent_id, label, status, id FROM tasks
                 WHERE team_run_id = ?1",
            )?;
            stmt.query_map(params![run_id], |r| {
                Ok(TaskStatusSummary {
                    agent_id: r.get(0)?,
                    label: r.get(1)?,
                    status: r.get(2)?,
                    task_id: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?
        };

        let (pending_tasks, task_statuses): (Vec<_>, Vec<_>) = all_tasks
            .into_iter()
            .partition(|t| t.status == "pending" || t.status == "in_progress");

        // Get critic feedback (final iteration only)
        let critic_feedback: Option<String> = self
            .conn
            .query_row(
                "SELECT content FROM team_workspace
                 WHERE run_id = ?1 AND entry_type = 'critic'
                 ORDER BY iteration DESC, created_at DESC LIMIT 1",
                params![run_id],
                |r| r.get(0),
            )
            .optional()?
            .map(|c: String| truncate_chars(&c, 200));

        Ok(Some(TeamRunSummary {
            run,
            agent_results,
            task_statuses,
            pending_tasks,
            critic_feedback,
        }))
    }

    /// Convenience method: get the enriched summary for the most recent
    /// completed/failed/suspended run for a team. Returns None if no such run exists.
    pub fn get_last_completed_team_run_summary(
        &self,
        team_name: &str,
    ) -> Result<Option<TeamRunSummary>> {
        match self.get_last_completed_team_run(team_name)? {
            Some(prev) => self.get_team_run_summary(&prev.id),
            None => Ok(None),
        }
    }

    // ===== Team Workspace =====

    #[allow(clippy::too_many_arguments)]
    pub fn insert_team_workspace_entry(
        &self,
        run_id: &str,
        parent_id: Option<i64>,
        agent_name: Option<&str>,
        entry_type: &str,
        content: &str,
        iteration: u32,
        trace_id: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO team_workspace (run_id, parent_id, agent_name, entry_type, content, iteration, trace_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![run_id, parent_id, agent_name, entry_type, content, iteration, trace_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn load_assignment_entry_ids(
        &self,
        run_id: &str,
        iteration: u32,
    ) -> Result<HashMap<String, i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT agent_name, id FROM team_workspace
             WHERE run_id = ?1 AND iteration = ?2 AND entry_type = 'assignment'
             AND agent_name IS NOT NULL",
        )?;
        let rows: Vec<(String, i64)> = stmt
            .query_map(params![run_id, iteration], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows.into_iter().collect())
    }

    pub fn load_team_workspace(&self, run_id: &str) -> Result<Vec<TeamWorkspaceEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, run_id, parent_id, agent_name,
                     entry_type, content, iteration, created_at
              FROM team_workspace WHERE run_id = ?1
              ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![run_id], |r| {
                Ok(TeamWorkspaceEntry {
                    id: r.get(0)?,
                    run_id: r.get(1)?,
                    parent_id: r.get(2)?,
                    agent_name: r.get(3)?,
                    entry_type: r.get(4)?,
                    content: r.get(5)?,
                    iteration: r.get::<_, u32>(6)?,
                    created_at: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // ===== Search / Layer 3 =====

    pub fn index_content(
        &self,
        agent_id: &str,
        source_type: &str,
        source_id: Option<i64>,
        content: &str,
    ) -> Result<i64> {
        let existing: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT id, content FROM search_content
                  WHERE agent_id = ?1 AND source_type = ?2 AND source_id IS ?3",
                params![agent_id, source_type, source_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((existing_id, old_content)) = existing {
            self.conn.execute(
                "UPDATE search_content SET content = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
                params![content, existing_id],
            )?;
            // Update FTS index
            let _ = self.conn.execute(
                "INSERT INTO fts_search(fts_search, rowid, content) VALUES ('delete', ?1, ?2)",
                params![existing_id, old_content],
            );
            let _ = self.conn.execute(
                "INSERT INTO fts_search(rowid, content) VALUES (?1, ?2)",
                params![existing_id, content],
            );
            Ok(existing_id)
        } else {
            self.conn.execute(
                "INSERT INTO search_content (agent_id, source_type, source_id, content)
                  VALUES (?1, ?2, ?3, ?4)",
                params![agent_id, source_type, source_id, content],
            )?;
            let new_id = self.conn.last_insert_rowid();
            let _ = self.conn.execute(
                "INSERT INTO fts_search(rowid, content) VALUES (?1, ?2)",
                params![new_id, content],
            );
            Ok(new_id)
        }
    }

    pub fn index_embedding(&self, content_id: i64, embedding: &[f32]) -> Result<()> {
        let emb_json = serde_json::to_string(embedding)?;
        self.conn.execute(
            "UPDATE search_content SET embedding_json = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2",
            params![emb_json, content_id],
        )?;
        // Upsert into vec0 table
        let _ = self.conn.execute(
            "INSERT INTO vec_search(rowid, embedding) VALUES (?1, ?2)
             ON CONFLICT(rowid) DO UPDATE SET embedding = excluded.embedding",
            params![content_id, emb_json],
        );
        Ok(())
    }

    pub fn delete_search_content(
        &self,
        agent_id: &str,
        source_type: &str,
        source_id: i64,
    ) -> Result<()> {
        let existing: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT id, content FROM search_content
                  WHERE agent_id = ?1 AND source_type = ?2 AND source_id = ?3",
                params![agent_id, source_type, source_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((id, content)) = existing {
            let _ = self.conn.execute(
                "INSERT INTO fts_search(fts_search, rowid, content) VALUES ('delete', ?1, ?2)",
                params![id, content],
            );
            let _ = self
                .conn
                .execute("DELETE FROM vec_search WHERE rowid = ?1", params![id]);
            self.conn
                .execute("DELETE FROM search_content WHERE id = ?1", params![id])?;
        }
        Ok(())
    }

    /// Returns search content rows that have no embedding yet (for backfill).
    pub fn get_unembedded_content(&self, agent_id: &str) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content FROM search_content
             WHERE agent_id = ?1 AND embedding_json IS NULL",
        )?;
        let rows = stmt.query_map(params![agent_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn count_search_content(&self, agent_id: &str) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM search_content WHERE agent_id = ?1",
            params![agent_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    pub fn fts_search(
        &self,
        agent_id: &str,
        query: &str,
        limit: usize,
        source_type_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        let mut stmt = self.conn.prepare(
            "SELECT sc.id, sc.source_type, sc.source_id, sc.content, -rank AS score
              FROM fts_search
              JOIN search_content sc ON fts_search.rowid = sc.id
              WHERE fts_search MATCH ?1 AND sc.agent_id = ?2
                AND (?3 IS NULL OR sc.source_type = ?3)
              ORDER BY rank LIMIT ?4",
        )?;
        let rows = stmt
            .query_map(
                params![query, agent_id, source_type_filter, limit as i64],
                |r| {
                    Ok(SearchResult {
                        id: r.get(0)?,
                        source_type: r.get(1)?,
                        source_id: r.get(2)?,
                        content: r.get(3)?,
                        score: r.get(4)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    fn vec_search_internal(
        &self,
        agent_id: &str,
        embedding: &[f32],
        limit: usize,
        source_type_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        let emb_json = serde_json::to_string(embedding)?;
        let mut stmt = self.conn.prepare(
            "SELECT sc.id, sc.source_type, sc.source_id, sc.content, knn.distance
              FROM (
                  SELECT rowid, distance FROM vec_search
                  WHERE embedding MATCH ?1
                  ORDER BY distance LIMIT ?4
              ) knn
              JOIN search_content sc ON knn.rowid = sc.id
              WHERE sc.agent_id = ?2 AND (?3 IS NULL OR sc.source_type = ?3)
              ORDER BY knn.distance",
        )?;
        let rows = stmt
            .query_map(
                params![emb_json, agent_id, source_type_filter, (limit * 3) as i64],
                |r| {
                    let dist: f64 = r.get(4)?;
                    Ok(SearchResult {
                        id: r.get(0)?,
                        source_type: r.get(1)?,
                        source_id: r.get(2)?,
                        content: r.get(3)?,
                        score: 1.0 / (1.0 + dist),
                    })
                },
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn hybrid_search(
        &self,
        agent_id: &str,
        fts_query: &str,
        embedding: Option<&[f32]>,
        limit: usize,
        source_type_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        let fts = self
            .fts_search(agent_id, fts_query, limit * 2, source_type_filter)
            .unwrap_or_default();
        let vec_results = embedding
            .and_then(|e| {
                self.vec_search_internal(agent_id, e, limit * 2, source_type_filter)
                    .ok()
            })
            .unwrap_or_default();

        if vec_results.is_empty() {
            return Ok(fts.into_iter().take(limit).collect());
        }

        const K: f64 = 60.0;
        let mut scores: HashMap<i64, f64> = HashMap::new();
        for (rank, r) in fts.iter().enumerate() {
            *scores.entry(r.id).or_default() += 1.0 / (K + rank as f64 + 1.0);
        }
        for (rank, r) in vec_results.iter().enumerate() {
            *scores.entry(r.id).or_default() += 1.0 / (K + rank as f64 + 1.0);
        }
        let mut all: HashMap<i64, SearchResult> = HashMap::new();
        for r in fts.into_iter().chain(vec_results.into_iter()) {
            all.entry(r.id).or_insert(r);
        }
        let mut ranked: Vec<(i64, f64)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(ranked
            .into_iter()
            .take(limit)
            .filter_map(|(id, score)| {
                all.remove(&id).map(|mut r| {
                    r.score = score;
                    r
                })
            })
            .collect())
    }

    // ===== Utilities =====

    pub fn schema_version(&self) -> Result<i64> {
        self.current_version()
    }

    pub fn db_size_bytes(&self) -> Result<u64> {
        let page_size: i64 = self.conn.query_row("PRAGMA page_size", [], |r| r.get(0))?;
        let page_count: i64 = self.conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;
        Ok((page_size * page_count) as u64)
    }

    pub fn vacuum(&self) -> Result<()> {
        self.conn.execute_batch("VACUUM;")?;
        Ok(())
    }

    // ===== Dashboard Queries (unscoped, cross-agent) =====

    /// Query the unified_timeline VIEW with optional filters and pagination.
    pub fn query_timeline(
        &self,
        filters: &TimelineFilters,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<TimelineRow>> {
        let (where_clause, params) = filters.to_sql();
        let sql = format!(
            "SELECT trace_id, session_id, agent_id, event_type, event_subtype, summary, created_at \
             FROM unified_timeline {} ORDER BY created_at DESC LIMIT ?{} OFFSET ?{}",
            where_clause,
            params.len() + 1,
            params.len() + 2,
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut all_params: Vec<rusqlite::types::Value> = params;
        all_params.push(rusqlite::types::Value::Integer(limit as i64));
        all_params.push(rusqlite::types::Value::Integer(offset as i64));
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = all_params
            .iter()
            .map(|p| p as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt
            .query_map(&*param_refs, |r| {
                Ok(TimelineRow {
                    trace_id: r.get(0)?,
                    session_id: r.get(1)?,
                    agent_id: r.get(2)?,
                    event_type: r.get(3)?,
                    event_subtype: r.get(4)?,
                    summary: r.get(5)?,
                    created_at: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Count total rows in unified_timeline matching filters.
    pub fn query_timeline_count(&self, filters: &TimelineFilters) -> Result<u64> {
        let (where_clause, params) = filters.to_sql();
        let sql = format!("SELECT COUNT(*) FROM unified_timeline {}", where_clause);
        let mut stmt = self.conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params
            .iter()
            .map(|p| p as &dyn rusqlite::types::ToSql)
            .collect();
        let count: i64 = stmt.query_row(&*param_refs, |r| r.get(0))?;
        Ok(count as u64)
    }

    /// Get all events for a specific trace_id from the unified_timeline VIEW.
    pub fn query_timeline_by_trace(&self, trace_id: &str) -> Result<Vec<TimelineRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT trace_id, session_id, agent_id, event_type, event_subtype, summary, created_at \
             FROM unified_timeline WHERE trace_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![trace_id], |r| {
                Ok(TimelineRow {
                    trace_id: r.get(0)?,
                    session_id: r.get(1)?,
                    agent_id: r.get(2)?,
                    event_type: r.get(3)?,
                    event_subtype: r.get(4)?,
                    summary: r.get(5)?,
                    created_at: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Get full messages for a specific trace_id (for rich trace detail rendering).
    pub fn get_messages_by_trace_id(&self, trace_id: &str) -> Result<Vec<SessionMessage>> {
        let sql = format!(
            "SELECT {} FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.trace_id = ?1
              ORDER BY m.created_at ASC, m.id ASC",
            Self::SESSION_MESSAGE_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![trace_id], Self::row_to_session_message)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// List all agents with message count.
    pub fn list_agents_with_stats(&self) -> Result<Vec<AgentWithStats>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.id, a.name, a.home_dir, a.active, a.last_seen, a.created_at,
                    (SELECT COUNT(*) FROM messages m WHERE m.agent_id = a.id AND m.role != 'summary') as msg_count
             FROM agents a ORDER BY a.name",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(AgentWithStats {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    home_dir: r.get(2)?,
                    active: r.get(3)?,
                    last_seen: r.get(4)?,
                    created_at: r.get(5)?,
                    message_count: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Get a single agent with stats by id or name.
    pub fn get_agent_with_stats(&self, agent_id: &str) -> Result<Option<AgentWithStats>> {
        self.conn
            .query_row(
                "SELECT a.id, a.name, a.home_dir, a.active, a.last_seen, a.created_at,
                        (SELECT COUNT(*) FROM messages m WHERE m.agent_id = a.id AND m.role != 'summary') as msg_count
                 FROM agents a WHERE a.id = ?1 OR a.name = ?1",
                params![agent_id],
                |r| {
                    Ok(AgentWithStats {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        home_dir: r.get(2)?,
                        active: r.get(3)?,
                        last_seen: r.get(4)?,
                        created_at: r.get(5)?,
                        message_count: r.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// List sessions with optional filters and pagination.
    #[allow(clippy::too_many_arguments)]
    pub fn list_sessions_paginated(
        &self,
        agent_id: Option<&str>,
        channel_type: Option<&str>,
        session_id: Option<&str>,
        task_id: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SessionWithStats>> {
        let mut conditions = Vec::new();
        let mut param_values: Vec<String> = Vec::new();

        if let Some(aid) = agent_id {
            param_values.push(aid.to_string());
            conditions.push(format!("s.agent_id = ?{}", param_values.len()));
        }
        if let Some(ct) = channel_type {
            param_values.push(ct.to_string());
            conditions.push(format!("s.channel_type = ?{}", param_values.len()));
        }
        if let Some(sid) = session_id {
            let sanitized: String = sid.chars().filter(|c| *c != '%' && *c != '_').collect();
            if !sanitized.is_empty() {
                param_values.push(format!("{}%", sanitized));
                conditions.push(format!("s.id LIKE ?{}", param_values.len()));
            }
        }
        if let Some(tid) = task_id {
            param_values.push(tid.to_string());
            conditions.push(format!(
                "COALESCE(s.task_id, json_extract(s.metadata, '$.task_id')) = ?{}",
                param_values.len()
            ));
        }
        if let Some(f) = from {
            param_values.push(f.to_string());
            conditions.push(format!("s.started_at >= ?{}", param_values.len()));
        }
        if let Some(t) = to {
            param_values.push(t.to_string());
            conditions.push(format!("s.started_at <= ?{}", param_values.len()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT s.id, s.agent_id, s.channel_type, s.started_at, s.ended_at, s.metadata, s.task_id,
                    (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) as msg_count
             FROM sessions s {} ORDER BY s.started_at DESC LIMIT ?{} OFFSET ?{}",
            where_clause,
            param_values.len() + 1,
            param_values.len() + 2,
        );

        let mut stmt = self.conn.prepare(&sql)?;
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> =
            param_values.into_iter().map(|s| Box::new(s) as _).collect();
        all_params.push(Box::new(limit));
        all_params.push(Box::new(offset));
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| &**p).collect();

        let rows = stmt
            .query_map(&*param_refs, |r| {
                Ok(SessionWithStats {
                    id: r.get(0)?,
                    agent_id: r.get(1)?,
                    channel_type: r.get(2)?,
                    started_at: r.get(3)?,
                    ended_at: r.get(4)?,
                    metadata: r.get(5)?,
                    task_id: r.get(6)?,
                    message_count: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Count sessions matching optional filters.
    pub fn count_sessions(
        &self,
        agent_id: Option<&str>,
        channel_type: Option<&str>,
        session_id: Option<&str>,
        task_id: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<u64> {
        let mut conditions = Vec::new();
        let mut param_values: Vec<String> = Vec::new();

        if let Some(aid) = agent_id {
            param_values.push(aid.to_string());
            conditions.push(format!("agent_id = ?{}", param_values.len()));
        }
        if let Some(ct) = channel_type {
            param_values.push(ct.to_string());
            conditions.push(format!("channel_type = ?{}", param_values.len()));
        }
        if let Some(sid) = session_id {
            let sanitized: String = sid.chars().filter(|c| *c != '%' && *c != '_').collect();
            if !sanitized.is_empty() {
                param_values.push(format!("{}%", sanitized));
                conditions.push(format!("id LIKE ?{}", param_values.len()));
            }
        }
        if let Some(tid) = task_id {
            param_values.push(tid.to_string());
            conditions.push(format!(
                "COALESCE(task_id, json_extract(metadata, '$.task_id')) = ?{}",
                param_values.len()
            ));
        }
        if let Some(f) = from {
            param_values.push(f.to_string());
            conditions.push(format!("started_at >= ?{}", param_values.len()));
        }
        if let Some(t) = to {
            param_values.push(t.to_string());
            conditions.push(format!("started_at <= ?{}", param_values.len()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let sql = format!("SELECT COUNT(*) FROM sessions {}", where_clause);
        let mut stmt = self.conn.prepare(&sql)?;
        let boxed: Vec<Box<dyn rusqlite::types::ToSql>> =
            param_values.into_iter().map(|s| Box::new(s) as _).collect();
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = boxed.iter().map(|p| &**p).collect();
        let count: i64 = stmt.query_row(&*param_refs, |r| r.get(0))?;
        Ok(count as u64)
    }

    /// Get all sessions linked to a task tree (the given task + all descendants).
    /// Returns sessions where `task_id` (or legacy `json_extract(metadata, '$.task_id')`)
    /// matches any task ID in the tree. Includes message count for display.
    pub fn get_sessions_for_task_tree(&self, root_task_id: &str) -> Result<Vec<TaskSessionRow>> {
        // Collect all task IDs in the tree: root + all descendants
        let mut task_ids = vec![root_task_id.to_string()];
        {
            let descendants = self.get_task_descendants(root_task_id)?;
            task_ids.extend(descendants.into_iter().map(|t| t.id));
        }

        let placeholders: String = (1..=task_ids.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");

        let sql = format!(
            "SELECT s.id, s.agent_id, s.channel_type, s.started_at, s.ended_at, s.task_id,
                    (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) as msg_count,
                    t.label as task_label
             FROM sessions s
             LEFT JOIN tasks t ON t.id = COALESCE(s.task_id, json_extract(s.metadata, '$.task_id'))
             WHERE COALESCE(s.task_id, json_extract(s.metadata, '$.task_id')) IN ({placeholders})
             ORDER BY s.started_at DESC"
        );

        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<Box<dyn rusqlite::types::ToSql>> = task_ids
            .into_iter()
            .map(|s| Box::new(s) as Box<dyn rusqlite::types::ToSql>)
            .collect();
        let refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| &**p).collect();

        let rows = stmt
            .query_map(&*refs, |r| {
                Ok(TaskSessionRow {
                    id: r.get(0)?,
                    agent_id: r.get(1)?,
                    channel_type: r.get(2)?,
                    started_at: r.get(3)?,
                    ended_at: r.get(4)?,
                    task_id: r.get(5)?,
                    message_count: r.get(6)?,
                    task_label: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;

        Ok(rows)
    }

    /// Get a single session by id.
    pub fn get_session(&self, session_id: &str) -> Result<Option<Session>> {
        self.conn
            .query_row(
                "SELECT id, agent_id, channel_type, started_at, ended_at, metadata, parent_session_id, task_id
                 FROM sessions WHERE id = ?1",
                params![session_id],
                |r| {
                    Ok(Session {
                        id: r.get(0)?,
                        agent_id: r.get(1)?,
                        channel_type: r.get(2)?,
                        started_at: r.get(3)?,
                        ended_at: r.get(4)?,
                        metadata: r.get(5)?,
                        parent_session_id: r.get(6)?,
                        task_id: r.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Get the most recent ended CLI session for an agent.
    ///
    /// Scoped to `channel_type = 'cli'`, excludes system sessions (`system-*`),
    /// delegate sessions (`delegate-*`), child sessions (non-NULL `parent_session_id`),
    /// and active sessions (`ended_at IS NULL`). Returns the most recently started
    /// ended session, or `None` if no matching session exists.
    pub fn get_last_cli_session_for_agent(&self, agent_id: &str) -> Result<Option<Session>> {
        self.conn
            .query_row(
                "SELECT id, agent_id, channel_type, started_at, ended_at, metadata, parent_session_id, task_id
                 FROM sessions
                 WHERE agent_id = ?1
                   AND channel_type = 'cli'
                   AND ended_at IS NOT NULL
                   AND id NOT LIKE 'system-%'
                   AND id NOT LIKE 'delegate-%'
                   AND parent_session_id IS NULL
                 ORDER BY started_at DESC
                 LIMIT 1",
                params![agent_id],
                |r| {
                    Ok(Session {
                        id: r.get(0)?,
                        agent_id: r.get(1)?,
                        channel_type: r.get(2)?,
                        started_at: r.get(3)?,
                        ended_at: r.get(4)?,
                        metadata: r.get(5)?,
                        parent_session_id: r.get(6)?,
                        task_id: r.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Load messages for a session with pagination.
    pub fn load_session_messages_paginated(
        &self,
        session_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SessionMessage>> {
        let sql = format!(
            "SELECT {} FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.session_id = ?1
              ORDER BY m.created_at ASC, m.id ASC LIMIT ?2 OFFSET ?3",
            Self::SESSION_MESSAGE_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(
                params![session_id, limit as i64, offset as i64],
                Self::row_to_session_message,
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Count messages in a session.
    pub fn count_session_messages(&self, session_id: &str) -> Result<u64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
            params![session_id],
            |r| r.get(0),
        )?;
        Ok(n as u64)
    }

    /// List audit events for an agent with pagination and optional filters.
    pub fn list_audit_events_paginated(
        &self,
        agent_id: &str,
        tool_name: Option<&str>,
        target_key: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AuditEvent>> {
        let sql = format!(
            "SELECT {} FROM audit_events WHERE agent_id = ?1 AND (?2 IS NULL OR tool_name = ?2) AND (?3 IS NULL OR target_key = ?3) ORDER BY created_at DESC LIMIT ?4 OFFSET ?5",
            Self::AUDIT_EVENT_COLS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(
                params![agent_id, tool_name, target_key, limit as i64, offset as i64],
                Self::row_to_audit_event,
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // ===== Dashboard: Paginated Task Listing =====

    /// Cross-agent task lookup — raw `&str`, for correlation and observability only.
    ///
    /// Does NOT enforce agent ownership. For agent-scoped tool paths, use
    /// [`crate::tools::AgentScopedTaskId`] + [`crate::tools::validate_task_exists`]
    /// instead — the newtype makes the ownership invariant compile-checked (mika#755).
    ///
    /// Current callers: `mika ask --task-id` correlation branch at
    /// `crates/mika-cli/src/commands/ask.rs`, dashboard task detail endpoint.
    pub fn get_task_unscoped(&self, id: &str) -> Result<Option<Task>> {
        let sql = format!("SELECT {} FROM tasks WHERE id = ?1", Self::TASK_COLUMNS);
        self.conn
            .query_row(&sql, params![id], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// Count tasks matching filters (dashboard).
    pub fn count_tasks_filtered(&self, filters: &TaskFilters) -> Result<u64> {
        let (where_clause, param_values) = Self::build_task_filter_sql(filters);
        let sql = format!("SELECT COUNT(*) FROM tasks {where_clause}");

        let params: Vec<&dyn rusqlite::types::ToSql> = param_values
            .iter()
            .map(|v| v as &dyn rusqlite::types::ToSql)
            .collect();

        self.conn
            .query_row(&sql, params.as_slice(), |r| r.get::<_, i64>(0))
            .map(|c| c as u64)
            .map_err(Into::into)
    }

    /// List tasks matching filters with pagination (dashboard).
    pub fn list_tasks_paginated(
        &self,
        filters: &TaskFilters,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Task>> {
        let (where_clause, param_values) = Self::build_task_filter_sql(filters);
        let sql = format!(
            "SELECT {} FROM tasks {where_clause} ORDER BY updated_at DESC LIMIT ?{} OFFSET ?{}",
            Self::TASK_COLUMNS,
            param_values.len() + 1,
            param_values.len() + 2,
        );

        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = param_values
            .into_iter()
            .map(|v| -> Box<dyn rusqlite::types::ToSql> { Box::new(v) })
            .collect();
        params.push(Box::new(limit as i64));
        params.push(Box::new(offset as i64));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(param_refs.as_slice(), Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Build WHERE clause and params for task filters.
    /// Push a comma-separated filter value as an `IN (?,?,...?)` clause.
    fn push_csv_in_clause(
        field: &str,
        csv: &str,
        conditions: &mut Vec<String>,
        params: &mut Vec<String>,
    ) {
        let values: Vec<&str> = csv.split(',').map(|s| s.trim()).take(20).collect();
        let placeholders: Vec<String> = values
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", params.len() + i + 1))
            .collect();
        conditions.push(format!("{field} IN ({})", placeholders.join(",")));
        for v in values {
            params.push(v.to_string());
        }
    }

    fn build_task_filter_sql(filters: &TaskFilters) -> (String, Vec<String>) {
        let mut conditions = Vec::new();
        let mut params = Vec::new();

        if let Some(ref status) = filters.status {
            Self::push_csv_in_clause("status", status, &mut conditions, &mut params);
        }

        if let Some(ref trigger_type) = filters.trigger_type {
            Self::push_csv_in_clause("trigger_type", trigger_type, &mut conditions, &mut params);
        }

        if let Some(ref action_type) = filters.action_type {
            Self::push_csv_in_clause("action_type", action_type, &mut conditions, &mut params);
        }

        if let Some(ref agent_id) = filters.agent_id {
            params.push(agent_id.clone());
            conditions.push(format!("agent_id = ?{}", params.len()));
        }

        if let Some(ref filter) = filters.team_run_id_filter {
            match filter {
                TeamRunIdFilter::Null => conditions.push("team_run_id IS NULL".to_string()),
                TeamRunIdFilter::NotNull => conditions.push("team_run_id IS NOT NULL".to_string()),
                TeamRunIdFilter::Specific(id) => {
                    params.push(id.clone());
                    conditions.push(format!("team_run_id = ?{}", params.len()));
                }
            }
        }

        if let Some(ref source) = filters.source {
            params.push(source.clone());
            conditions.push(format!("source = ?{}", params.len()));
        }

        if let Some(ref from) = filters.from {
            params.push(from.clone());
            conditions.push(format!("created_at >= ?{}", params.len()));
        }

        if let Some(ref to) = filters.to {
            params.push(to.clone());
            conditions.push(format!("created_at <= ?{}", params.len()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        (where_clause, params)
    }

    // ===== Dashboard: Paginated Team Run Listing =====

    /// Count team runs matching filters (dashboard).
    pub fn count_team_runs_filtered(&self, filters: &TeamRunFilters) -> Result<u64> {
        let (where_clause, param_values) = Self::build_team_run_filter_sql(filters);
        let sql = format!(
            "SELECT COUNT(*) FROM team_runs r JOIN teams t ON r.team_id = t.id {where_clause}"
        );

        let params: Vec<&dyn rusqlite::types::ToSql> = param_values
            .iter()
            .map(|v| v as &dyn rusqlite::types::ToSql)
            .collect();

        self.conn
            .query_row(&sql, params.as_slice(), |r| r.get::<_, i64>(0))
            .map(|c| c as u64)
            .map_err(Into::into)
    }

    /// List team runs matching filters with pagination (dashboard).
    pub fn list_team_runs_paginated(
        &self,
        filters: &TeamRunFilters,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<TeamRunRow>> {
        let (where_clause, param_values) = Self::build_team_run_filter_sql(filters);
        let sql = format!(
            "SELECT {} FROM team_runs r JOIN teams t ON r.team_id = t.id
             {where_clause} ORDER BY r.started_at DESC LIMIT ?{} OFFSET ?{}",
            Self::TEAM_RUN_COLUMNS,
            param_values.len() + 1,
            param_values.len() + 2,
        );

        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = param_values
            .into_iter()
            .map(|v| -> Box<dyn rusqlite::types::ToSql> { Box::new(v) })
            .collect();
        params.push(Box::new(limit as i64));
        params.push(Box::new(offset as i64));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(param_refs.as_slice(), Self::row_to_team_run)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Build WHERE clause and params for team run filters.
    fn build_team_run_filter_sql(filters: &TeamRunFilters) -> (String, Vec<String>) {
        let mut conditions = Vec::new();
        let mut params = Vec::new();

        if let Some(ref team_name) = filters.team_name {
            params.push(team_name.clone());
            conditions.push(format!("t.name = ?{}", params.len()));
        }

        if let Some(ref status) = filters.status {
            Self::push_csv_in_clause("r.status", status, &mut conditions, &mut params);
        }

        if let Some(ref from) = filters.from {
            params.push(from.to_string());
            conditions.push(format!("r.started_at >= ?{}", params.len()));
        }

        if let Some(ref to) = filters.to {
            params.push(to.to_string());
            conditions.push(format!("r.started_at <= ?{}", params.len()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        (where_clause, params)
    }

    // ===== Combined data+count queries (single DB round-trip) =====

    /// Query timeline data and count in a single closure (avoids TOCTOU race).
    pub fn query_timeline_with_count(
        &self,
        filters: &TimelineFilters,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<TimelineRow>, u64)> {
        let count = self.query_timeline_count(filters)?;
        let data = self.query_timeline(filters, limit, offset)?;
        Ok((data, count))
    }

    /// List sessions and count in a single closure.
    #[allow(clippy::too_many_arguments)]
    pub fn list_sessions_paginated_with_count(
        &self,
        agent_id: Option<&str>,
        channel_type: Option<&str>,
        session_id: Option<&str>,
        task_id: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<SessionWithStats>, u64)> {
        let count = self.count_sessions(agent_id, channel_type, session_id, task_id, from, to)?;
        let data = self.list_sessions_paginated(
            agent_id,
            channel_type,
            session_id,
            task_id,
            from,
            to,
            limit,
            offset,
        )?;
        Ok((data, count))
    }

    /// Load session messages and count in a single closure.
    pub fn load_session_messages_paginated_with_count(
        &self,
        session_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<SessionMessage>, u64)> {
        let count = self.count_session_messages(session_id)?;
        let data = self.load_session_messages_paginated(session_id, limit, offset)?;
        Ok((data, count))
    }

    /// List audit events and count in a single closure.
    pub fn list_audit_events_paginated_with_count(
        &self,
        agent_id: &str,
        tool_name: Option<&str>,
        target_key: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<AuditEvent>, u64)> {
        let count = self.count_audit_events(agent_id, tool_name, target_key)?;
        let data =
            self.list_audit_events_paginated(agent_id, tool_name, target_key, limit, offset)?;
        Ok((data, count))
    }

    /// List tasks and count in a single closure.
    pub fn list_tasks_paginated_with_count(
        &self,
        filters: &TaskFilters,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<Task>, u64)> {
        let count = self.count_tasks_filtered(filters)?;
        let data = self.list_tasks_paginated(filters, limit, offset)?;
        Ok((data, count))
    }

    /// mika#2360 — list the recurring-task registry as a closed projection.
    ///
    /// `trigger_type = 'recurring'` is a literal in the SQL, never a
    /// parameter (R2): no query string can widen the population. The
    /// `SELECT` names its columns (R3) — never `*` nor [`Self::TASK_COLUMNS`],
    /// so `action_config` / `result` / `input_context` / `metadata` cannot
    /// leak whatever the table grows. No status filter: a dead row is
    /// precisely what an operator diagnosing a missing recurrence needs to
    /// see (mika#2358 D2). Sorted `label COLLATE NOCASE, created_at` so the
    /// duplicates of one label sit next to each other in birth order — the
    /// veto compares labels case-insensitively, so must the sort.
    ///
    /// `zombie_veto_active` mirrors the two-query guard in
    /// [`Self::create_recurring_task_if_absent`]: the *lift-spent* term is a
    /// correlated `EXISTS` over `(agent_id, label COLLATE NOCASE)`, not a read
    /// of the current row's `metadata` — the lift may have been spent on a
    /// sibling row (mika#2337). `mika2360_zombie_veto_flag_*` tests pin the
    /// two predicates together.
    ///
    /// Read-only (R4): a `SELECT` and a `COUNT`, no frame emitted.
    pub fn list_recurring_registry(
        &self,
        agent_id: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<RecurringRegistryRow>, u64)> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE trigger_type = 'recurring'
               AND (?1 IS NULL OR agent_id = ?1)",
            params![agent_id],
            |r| r.get(0),
        )?;

        let mut stmt = self.conn.prepare(
            "SELECT t.label, t.agent_id, t.trigger_type, t.action_type, t.cron_expr,
                    t.next_fire_at, t.status, t.created_at, t.updated_at,
                    COALESCE(
                        t.status IN ('failed', 'cancelled', 'expired')
                        AND t.updated_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
                        AND NOT (json_valid(t.metadata)
                                 AND COALESCE(json_extract(t.metadata, ?3), 0) = 1)
                        AND NOT (
                            NOT EXISTS (
                                SELECT 1 FROM tasks s
                                WHERE s.agent_id = t.agent_id
                                  AND s.label = t.label COLLATE NOCASE
                                  AND s.trigger_type = 'recurring'
                                  AND s.updated_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
                                  AND json_valid(s.metadata)
                                  AND COALESCE(json_extract(s.metadata, ?4), 0) = 1
                            )
                            AND json_valid(t.metadata)
                            AND COALESCE(json_extract(t.metadata, ?5), 0) = 1
                        )
                        AND NOT (json_valid(t.metadata)
                                 AND COALESCE(json_extract(t.metadata, ?8), 0) = 1),
                        0
                    ) AS zombie_veto_active
             FROM tasks t
             WHERE t.trigger_type = 'recurring'
               AND (?1 IS NULL OR t.agent_id = ?1)
             ORDER BY t.label COLLATE NOCASE ASC, t.created_at ASC
             LIMIT ?6 OFFSET ?7",
        )?;
        let rows = stmt
            .query_map(
                params![
                    agent_id,
                    RECURRING_ZOMBIE_GRACE_SQL,
                    RECURRING_CONFIG_CANCEL_REVERTED_PATH,
                    RECURRING_UNKNOWN_TRIGGER_LIFT_CONSUMED_PATH,
                    RECURRING_UNKNOWN_TRIGGER_PATH,
                    limit as i64,
                    offset as i64,
                    RECURRING_OPERATOR_REARM_PATH,
                ],
                |r| {
                    Ok(RecurringRegistryRow {
                        label: r.get(0)?,
                        agent_id: r.get(1)?,
                        trigger_type: r.get(2)?,
                        action_type: r.get(3)?,
                        cron_expr: r.get(4)?,
                        next_fire_at: r.get(5)?,
                        status: r.get(6)?,
                        created_at: r.get(7)?,
                        updated_at: r.get(8)?,
                        zombie_veto_active: r.get::<_, i64>(9)? == 1,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok((rows, count as u64))
    }

    /// List team runs and count in a single closure.
    pub fn list_team_runs_paginated_with_count(
        &self,
        filters: &TeamRunFilters,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<TeamRunRow>, u64)> {
        let count = self.count_team_runs_filtered(filters)?;
        let data = self.list_team_runs_paginated(filters, limit, offset)?;
        Ok((data, count))
    }

    // ===== Dashboard: Dev Runs (tasks with dev-run sources) =====

    /// Update the metadata JSON on a manual (task) task.
    /// Only works on `trigger_type='manual'` tasks. Returns false if not found.
    pub fn update_task_metadata(&self, task_id: &str, metadata_json: &str) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE tasks SET metadata = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2 AND trigger_type = 'manual'",
            params![metadata_json, task_id],
        )?;
        Ok(rows > 0)
    }

    /// Get a single dev run (task with a dev-run source) by ID — unscoped by agent_id.
    pub fn get_dev_run(&self, task_id: &str) -> Result<Option<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks WHERE id = ?1 AND trigger_type = 'manual' AND source IN ('self_dev', 'github_issue')",
            Self::TASK_COLUMNS
        );
        self.conn
            .query_row(&sql, params![task_id], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// List dev runs (tasks with dev-run sources) with pagination and count.
    pub fn list_dev_runs_paginated_with_count(
        &self,
        status: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<Task>, u64)> {
        let mut conditions = vec![
            "trigger_type = 'manual'".to_string(),
            "source IN ('self_dev', 'github_issue')".to_string(),
        ];
        let mut param_values: Vec<String> = Vec::new();

        if let Some(s) = status {
            param_values.push(s.to_string());
            conditions.push(format!("status = ?{}", param_values.len()));
        }
        if let Some(f) = from {
            param_values.push(f.to_string());
            conditions.push(format!("created_at >= ?{}", param_values.len()));
        }
        if let Some(t) = to {
            param_values.push(t.to_string());
            conditions.push(format!("created_at <= ?{}", param_values.len()));
        }

        let where_clause = format!("WHERE {}", conditions.join(" AND "));

        let count_sql = format!("SELECT COUNT(*) FROM tasks {}", where_clause);
        let data_sql = format!(
            "SELECT {} FROM tasks {} ORDER BY created_at DESC LIMIT ?{} OFFSET ?{}",
            Self::TASK_COLUMNS,
            where_clause,
            param_values.len() + 1,
            param_values.len() + 2,
        );

        let mut count_stmt = self.conn.prepare(&count_sql)?;
        let boxed_count: Vec<Box<dyn rusqlite::types::ToSql>> = param_values
            .iter()
            .map(|s| Box::new(s.clone()) as _)
            .collect();
        let count_refs: Vec<&dyn rusqlite::types::ToSql> =
            boxed_count.iter().map(|p| &**p).collect();
        let count: u64 = count_stmt.query_row(&*count_refs, |r| r.get::<_, i64>(0))? as u64;

        let mut data_stmt = self.conn.prepare(&data_sql)?;
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> =
            param_values.into_iter().map(|s| Box::new(s) as _).collect();
        all_params.push(Box::new(limit as i64));
        all_params.push(Box::new(offset as i64));
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| &**p).collect();
        let data: Vec<Task> = data_stmt
            .query_map(&*param_refs, Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;

        Ok((data, count))
    }

    // ── KG corpus queries (#778) ──────────────────────────────────────────

    /// Count `kg_chunks` rows for a given `docs_root_hash`.
    ///
    /// Used as a "has this corpus been ingested before?" proxy at agent startup.
    /// Returns 0 for a never-ingested corpus (drift WARN).
    ///
    /// **Write-order dependency:** this is a reliable proxy ONLY because the
    /// lexical ingestor writes `kg_chunks` atomically with (or before)
    /// `kg_extractions` in the composed-write transaction. If a future edit
    /// writes `kg_extractions` before `kg_chunks`, this proxy becomes stale.
    /// See `docs/solutions/best-practices/kg-lexical-ingestion-composed-write-2026-04-22.md`.
    pub fn count_chunks_for_docs_root_hash(&self, docs_root_hash: &str) -> Result<u64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM kg_chunks WHERE docs_root_hash = ?1",
            params![docs_root_hash],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }
}

// ===== Utility Functions =====

/// Format an ISO 8601 timestamp as a human-readable UTC string: "YYYY-MM-DD HH:MM:SS".
pub fn format_ts(ts: &str) -> String {
    crate::timestamp::parse(ts)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|_| ts.to_string())
}

/// Return today's midnight UTC time for the given IANA timezone string.
/// Falls back to UTC if the timezone string is unrecognised.
pub fn today_midnight_utc(timezone: &str) -> chrono::DateTime<chrono::Utc> {
    let tz: Tz = timezone.parse().unwrap_or(chrono_tz::UTC);
    let now_local = chrono::Utc::now().with_timezone(&tz);
    let midnight_local = now_local
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap_or_default();
    tz.from_local_datetime(&midnight_local)
        .earliest()
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now)
}

/// Truncate a string to `max_chars` characters, appending "..." if truncated.
pub(crate) fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{truncated}...")
    }
}

/// Format the age of a timestamp relative to `now` as a human-readable string.
///
/// Examples: "3h 22m", "5d", "45m", "2d 4h"
fn format_age(timestamp_str: &str, now: chrono::DateTime<Utc>) -> String {
    let ts = match timestamp::parse(timestamp_str) {
        Ok(t) => t,
        Err(_) => return "unknown".to_string(),
    };
    let secs = now.signed_duration_since(ts).num_seconds().max(0);
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;

    if days > 0 && hours > 0 {
        format!("{days}d {hours}h")
    } else if days > 0 {
        format!("{days}d")
    } else if hours > 0 && mins > 0 {
        format!("{hours}h {mins}m")
    } else if hours > 0 {
        format!("{hours}h")
    } else if mins > 0 {
        format!("{mins}m")
    } else {
        "< 1m".to_string()
    }
}

// ===== Agent Reset helpers =====

/// Per-table counts of rows deleted (or that would be deleted) by an agent reset.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ResetAgentCounts {
    pub sessions: u64,
    pub messages: u64,
    pub core_memory: u64,
    pub llm_calls: u64,
    pub tool_calls: u64,
    pub audit_events: u64,
    pub audit_event_summaries: u64,
    pub people: u64,
    pub commitments: u64,
    pub preferences: u64,
    pub events: u64,
    pub search_content: u64,
    pub tasks: u64,
    pub kg_subject_resolutions: u64,
    pub kg_resolutions_log: u64,
    pub agent_kg_corpora: u64,
    pub kg_invalidated_no_match: u64,
    pub skill_overrides: u64,
    pub operational_items: u64,
    pub heartbeat_sends: u64,
    pub reflection_runs: u64,
    pub customer_config: u64,
    pub failed_sends: u64,
    // Shared KG tables (only deleted if no other agent shares the corpus)
    pub kg_chunks: u64,
    pub kg_subject_entities: u64,
    pub kg_subject_relationships: u64,
    pub kg_chunk_subjects: u64,
    pub kg_chunk_subject_relationships: u64,
    pub kg_extractions: u64,
}

impl ResetAgentCounts {
    /// Total rows across all tables.
    pub fn total(&self) -> u64 {
        self.sessions
            + self.messages
            + self.core_memory
            + self.llm_calls
            + self.tool_calls
            + self.audit_events
            + self.audit_event_summaries
            + self.people
            + self.commitments
            + self.preferences
            + self.events
            + self.search_content
            + self.tasks
            + self.kg_subject_resolutions
            + self.kg_resolutions_log
            + self.agent_kg_corpora
            + self.kg_invalidated_no_match
            + self.skill_overrides
            + self.operational_items
            + self.heartbeat_sends
            + self.reflection_runs
            + self.customer_config
            + self.failed_sends
            + self.kg_chunks
            + self.kg_subject_entities
            + self.kg_subject_relationships
            + self.kg_chunk_subjects
            + self.kg_chunk_subject_relationships
            + self.kg_extractions
    }
}

impl Database {
    /// Count per-table rows that would be deleted by `reset_agent_state`.
    /// Used for `--dry-run` preview.
    pub fn count_agent_state(&self, agent_id: &str) -> Result<ResetAgentCounts> {
        // Verify agent exists
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM agents WHERE id = ?1)",
            params![agent_id],
            |r| r.get(0),
        )?;
        if !exists {
            anyhow::bail!("Agent '{agent_id}' not found in database");
        }

        let count = |table: &str, col: &str, val: &str| -> Result<u64> {
            let sql = format!("SELECT COUNT(*) FROM {table} WHERE {col} = ?1");
            let n = self
                .conn
                .query_row(&sql, params![val], |r| r.get::<_, i64>(0))?;
            Ok(n as u64)
        };

        let mut counts = ResetAgentCounts {
            sessions: count("sessions", "agent_id", agent_id)?,
            messages: count("messages", "agent_id", agent_id)?,
            core_memory: count("core_memory", "agent_id", agent_id)?,
            llm_calls: count("llm_calls", "agent_id", agent_id)?,
            tool_calls: count("tool_calls", "agent_id", agent_id)?,
            audit_events: count("audit_events", "agent_id", agent_id)?,
            audit_event_summaries: count("audit_event_summaries", "agent_id", agent_id)?,
            people: count("people", "agent_id", agent_id)?,
            commitments: count("commitments", "agent_id", agent_id)?,
            preferences: count("preferences", "agent_id", agent_id)?,
            events: count("events", "agent_id", agent_id)?,
            search_content: count("search_content", "agent_id", agent_id)?,
            tasks: count("tasks", "agent_id", agent_id)?,
            kg_subject_resolutions: count("kg_subject_resolutions", "agent_id", agent_id)?,
            kg_resolutions_log: count("kg_resolutions_log", "agent_id", agent_id)?,
            agent_kg_corpora: count("agent_kg_corpora", "agent_id", agent_id)?,
            kg_invalidated_no_match: count("kg_invalidated_no_match", "agent_id", agent_id)?,
            skill_overrides: count("skill_overrides", "agent_id", agent_id)?,
            operational_items: count("operational_items", "agent_id", agent_id)?,
            heartbeat_sends: count("heartbeat_sends", "agent_id", agent_id)?,
            reflection_runs: count("reflection_runs", "agent_id", agent_id)?,
            customer_config: count("customer_config", "agent_id", agent_id)?,
            failed_sends: count("failed_sends", "agent_id", agent_id)?,
            ..Default::default()
        };

        // Shared KG tables: count rows that would be deleted
        // (only if no other agent shares the same docs_root_hash)
        let corpora = self.list_agent_corpora(agent_id)?;
        for (hash, _path) in &corpora {
            let other_refs: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM agent_kg_corpora WHERE docs_root_hash = ?1 AND agent_id != ?2",
                params![hash, agent_id],
                |r| r.get(0),
            )?;
            if other_refs == 0 {
                // This agent is the sole owner — these would be deleted
                let shared_tables = [
                    ("kg_chunks", &mut counts.kg_chunks),
                    ("kg_subject_entities", &mut counts.kg_subject_entities),
                    (
                        "kg_subject_relationships",
                        &mut counts.kg_subject_relationships,
                    ),
                    ("kg_chunk_subjects", &mut counts.kg_chunk_subjects),
                    (
                        "kg_chunk_subject_relationships",
                        &mut counts.kg_chunk_subject_relationships,
                    ),
                    ("kg_extractions", &mut counts.kg_extractions),
                ];
                for (table, field) in shared_tables {
                    let sql = format!("SELECT COUNT(*) FROM {table} WHERE docs_root_hash = ?1");
                    let n: i64 = self.conn.query_row(&sql, params![hash], |r| r.get(0))?;
                    *field += n as u64;
                }
            }
        }

        Ok(counts)
    }

    /// Get active tasks for an agent (used by the active-task guard).
    /// Returns `(task_id, status)` pairs for tasks in active states.
    pub fn get_active_tasks_for_agent(&self, agent_id: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, status FROM tasks
             WHERE agent_id = ?1
               AND status IN ('pending', 'in_progress', 'recurring_active')",
        )?;
        let rows = stmt
            .query_map(params![agent_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Delete all per-agent state while preserving the agent row itself.
    ///
    /// Caller is responsible for the active-task guard and confirmation prompt.
    /// After this call, the agent is in a freshly-provisioned state: zero rows
    /// in all child tables, agent row preserved, `identity.toml` untouched.
    pub fn reset_agent_state(&self, agent_id: &str) -> Result<ResetAgentCounts> {
        // Verify agent exists
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM agents WHERE id = ?1)",
            params![agent_id],
            |r| r.get(0),
        )?;
        if !exists {
            anyhow::bail!("Agent '{agent_id}' not found in database");
        }

        let tx = self.conn.unchecked_transaction()?;
        let mut counts = ResetAgentCounts::default();

        // Helper: delete from a table by agent_id and record count
        macro_rules! delete_by_agent {
            ($table:expr, $field:ident) => {
                tx.execute(
                    &format!("DELETE FROM {} WHERE agent_id = ?1", $table),
                    params![agent_id],
                )?;
                counts.$field = tx.changes();
            };
        }

        // -- Category 1: Conversation --
        delete_by_agent!("messages", messages);
        delete_by_agent!("sessions", sessions);
        delete_by_agent!("llm_calls", llm_calls);
        delete_by_agent!("tool_calls", tool_calls);

        // -- Category 2: Memory --
        delete_by_agent!("core_memory", core_memory);
        delete_by_agent!("people", people);
        delete_by_agent!("commitments", commitments);
        delete_by_agent!("preferences", preferences);
        delete_by_agent!("events", events);
        delete_by_agent!("search_content", search_content);

        // -- Category 3: Audit --
        delete_by_agent!("audit_events", audit_events);
        delete_by_agent!("audit_event_summaries", audit_event_summaries);

        // -- Category 4: Tasks --
        delete_by_agent!("tasks", tasks);

        // -- Category 5: Operations --
        delete_by_agent!("heartbeat_sends", heartbeat_sends);
        delete_by_agent!("reflection_runs", reflection_runs);
        delete_by_agent!("customer_config", customer_config);
        delete_by_agent!("failed_sends", failed_sends);

        // -- Category 6: KG per-agent --
        delete_by_agent!("kg_subject_resolutions", kg_subject_resolutions);
        delete_by_agent!("kg_resolutions_log", kg_resolutions_log);
        delete_by_agent!("kg_invalidated_no_match", kg_invalidated_no_match);

        // -- Category 7: KG shared (conditional — only if no other agent shares the corpus) --
        // Query corpora BEFORE deleting agent_kg_corpora rows
        let corpora: Vec<(String, String)> = {
            let mut stmt = tx.prepare(
                "SELECT docs_root_hash, docs_root_path FROM agent_kg_corpora WHERE agent_id = ?1",
            )?;
            stmt.query_map(params![agent_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?
        };

        for (hash, _path) in &corpora {
            let other_refs: i64 = tx.query_row(
                "SELECT COUNT(*) FROM agent_kg_corpora WHERE docs_root_hash = ?1 AND agent_id != ?2",
                params![hash, agent_id],
                |r| r.get(0),
            )?;
            if other_refs == 0 {
                // Sole owner — delete shared-layer rows in FK-safe order
                let shared_tables = [
                    "kg_chunk_subject_relationships",
                    "kg_chunk_subjects",
                    "kg_subject_relationships",
                    "kg_subject_entities",
                    "kg_extractions",
                    "kg_chunks",
                ];
                for table in &shared_tables {
                    tx.execute(
                        &format!("DELETE FROM {table} WHERE docs_root_hash = ?1"),
                        params![hash],
                    )?;
                    let deleted = tx.changes();
                    match *table {
                        "kg_chunk_subject_relationships" => {
                            counts.kg_chunk_subject_relationships += deleted
                        }
                        "kg_chunk_subjects" => counts.kg_chunk_subjects += deleted,
                        "kg_subject_relationships" => counts.kg_subject_relationships += deleted,
                        "kg_subject_entities" => counts.kg_subject_entities += deleted,
                        "kg_extractions" => counts.kg_extractions += deleted,
                        "kg_chunks" => counts.kg_chunks += deleted,
                        _ => {}
                    }
                }
            }
        }

        // Now delete the agent_kg_corpora mapping rows
        delete_by_agent!("agent_kg_corpora", agent_kg_corpora);

        // -- Category 8: Skills --
        delete_by_agent!("skill_overrides", skill_overrides);

        // -- Category 9: Operational ledger --
        delete_by_agent!("operational_items", operational_items);

        tx.commit()?;

        // -- Category 10: Post-transaction FTS5 rebuild --
        // External-content FTS5 table needs explicit rebuild after base table changes.
        // If the rebuild fails, the data is already deleted (transaction committed).
        // Log a warning but return success — the FTS index will self-heal on next
        // startup when search_content is re-indexed.
        if let Err(e) = self
            .conn
            .execute("INSERT INTO fts_search(fts_search) VALUES('rebuild')", [])
        {
            tracing::warn!(error = %e, agent_id, "FTS5 rebuild failed after agent reset — index may be stale");
        }

        Ok(counts)
    }
}

// ===== KG CLI helpers =====

/// Counts of rows deleted by a KG purge operation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct KgPurgeCounts {
    pub resolutions_deleted: u64,
    pub resolution_log_deleted: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_layer_deleted: Option<Vec<(String, u64)>>,
}

impl Database {
    /// Count rows in a KG table matching a key column.
    pub fn kg_count_rows(&self, table: &str, key_col: &str, key_val: &str) -> Result<u64> {
        // Allowlist tables and columns to prevent SQL injection
        let valid_tables = [
            "kg_chunks",
            "kg_subject_entities",
            "kg_subject_relationships",
            "kg_chunk_subjects",
            "kg_chunk_subject_relationships",
            "kg_extractions",
            "kg_subject_resolutions",
            "kg_resolutions_log",
            "kg_entities",
        ];
        let valid_cols = ["docs_root_hash", "agent_id"];

        if !valid_tables.contains(&table) {
            anyhow::bail!("Invalid KG table: {table}");
        }
        if !valid_cols.contains(&key_col) {
            anyhow::bail!("Invalid KG key column: {key_col}");
        }

        let sql = format!("SELECT COUNT(*) FROM {table} WHERE {key_col} = ?1");
        let count = self
            .conn
            .query_row(&sql, params![key_val], |r| r.get::<_, i64>(0))?;
        Ok(count as u64)
    }

    /// Count resolved subject entities for an agent within a specific corpus.
    ///
    /// Joins `kg_subject_resolutions` through `kg_subject_entities` to scope
    /// the count to a single `docs_root_hash`. Used by `mika kg status` to
    /// display per-corpus resolution coverage for multi-corpus agents (#877).
    pub fn kg_count_resolved_for_corpus(
        &self,
        agent_id: &str,
        docs_root_hash: &str,
    ) -> Result<u64> {
        let count = self.conn.query_row(
            "SELECT COUNT(*) FROM kg_subject_resolutions sr \
             JOIN kg_subject_entities se ON sr.subject_entity_id = se.id \
             WHERE sr.agent_id = ?1 AND se.docs_root_hash = ?2",
            params![agent_id, docs_root_hash],
            |r| r.get::<_, i64>(0),
        )?;
        Ok(count as u64)
    }

    /// Count resolver-actionable pending subjects for an agent within a corpus (#999).
    ///
    /// Mirrors `entity_resolver::count_pending_for_corpus`: only counts subject
    /// entities of the five resolver-actionable types (`skill`, `tool`, `agent`,
    /// `problem_type`, `concept`) that have no resolution log row OR whose
    /// source extraction trace_id diverges from the latest one.
    ///
    /// Subject-graph-only types (`pattern`, `failure_mode`, `solution_path`)
    /// are intentionally excluded — they have no canonical domain projection
    /// and the resolver never touches them. Showing them as "pending" misleads
    /// the operator into thinking there is actionable backlog when there is
    /// none.
    pub fn kg_count_pending_resolver_actionable_for_corpus(
        &self,
        agent_id: &str,
        docs_root_hash: &str,
    ) -> Result<u64> {
        let sql = "SELECT COUNT(*)
             FROM kg_subject_entities e
             LEFT JOIN kg_resolutions_log r
                 ON r.subject_entity_id = e.id AND r.agent_id = ?1
             WHERE e.docs_root_hash = ?2
               AND e.type IN ('skill', 'tool', 'agent', 'problem_type', 'concept')
               AND (
                 r.id IS NULL
                 OR r.source_extraction_trace_id != (
                     SELECT cs.extraction_trace_id
                     FROM kg_chunk_subjects cs
                     WHERE cs.subject_entity_id = e.id
                     ORDER BY cs.created_at DESC LIMIT 1
                 )
               )";
        let count = self
            .conn
            .query_row(sql, params![agent_id, docs_root_hash], |r| {
                r.get::<_, i64>(0)
            })?;
        Ok(count as u64)
    }

    /// Get the most recent extraction timestamp for a docs_root_hash.
    pub fn kg_last_extraction(&self, docs_root_hash: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT MAX(created_at) FROM kg_extractions WHERE docs_root_hash = ?1",
                params![docs_root_hash],
                |r| r.get::<_, Option<String>>(0),
            )
            .map_err(Into::into)
    }

    /// Detect drift: find distinct docs_root_hash values the agent's
    /// resolutions point at via the subject->chunk->corpus chain.
    pub fn kg_observed_hashes(&self, agent_id: &str) -> Result<Vec<String>> {
        let sql = r#"
            SELECT DISTINCT c.docs_root_hash
            FROM kg_subject_resolutions r
            JOIN kg_subject_entities se ON se.id = r.subject_entity_id
            JOIN kg_chunk_subjects cs ON cs.subject_entity_id = se.id
            JOIN kg_chunks c ON c.id = cs.chunk_id
            WHERE r.agent_id = ?1
        "#;
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt
            .query_map(params![agent_id], |r| r.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Transactional purge of an agent's KG state.
    ///
    /// **Caller MUST verify no other agent references the `docs_root_hash`
    /// before passing `force_delete_shared = true`.** The helper does not
    /// re-verify — this flag is a pre-authorization, not operator intent.
    pub fn purge_kg_for_agent(
        &self,
        agent_id: &str,
        force_delete_shared: bool,
        docs_root_hash: Option<&str>,
    ) -> Result<KgPurgeCounts> {
        let tx = self.conn.unchecked_transaction()?;

        // Step 1: Delete per-agent resolutions
        tx.execute(
            "DELETE FROM kg_subject_resolutions WHERE agent_id = ?1",
            params![agent_id],
        )?;
        let resolutions_deleted = tx.changes();

        // Step 2: Delete per-agent resolution log
        tx.execute(
            "DELETE FROM kg_resolutions_log WHERE agent_id = ?1",
            params![agent_id],
        )?;
        let resolution_log_deleted = tx.changes();

        // Step 3: Optionally delete shared-layer rows
        let shared_layer_deleted = if force_delete_shared {
            if let Some(hash) = docs_root_hash {
                // Delete in FK-safe order (children before parents)
                let tables = [
                    "kg_chunk_subject_relationships",
                    "kg_chunk_subjects",
                    "kg_subject_relationships",
                    "kg_subject_entities",
                    "kg_extractions",
                    "kg_chunks",
                ];
                let mut deleted = Vec::new();
                for table in &tables {
                    tx.execute(
                        &format!("DELETE FROM {table} WHERE docs_root_hash = ?1"),
                        params![hash],
                    )?;
                    let count = tx.changes();
                    if count > 0 {
                        deleted.push((table.to_string(), count));
                    }
                }
                Some(deleted)
            } else {
                None
            }
        } else {
            None
        };

        tx.commit()?;

        Ok(KgPurgeCounts {
            resolutions_deleted,
            resolution_log_deleted,
            shared_layer_deleted,
        })
    }

    /// Run orphan FK check for KG validate.
    /// Returns (count, example_id) of orphan rows.
    pub fn kg_check_orphan_fk(
        &self,
        source_table: &str,
        fk_col: &str,
        target_table: &str,
    ) -> Result<(u64, Option<i64>)> {
        // Allowlist for safety
        let valid_tables = [
            "kg_chunks",
            "kg_subject_entities",
            "kg_subject_relationships",
            "kg_chunk_subjects",
            "kg_chunk_subject_relationships",
            "kg_subject_resolutions",
            "kg_resolutions_log",
            "kg_entities",
            "kg_extractions",
        ];
        let valid_cols = [
            "chunk_id",
            "subject_entity_id",
            "subject_relationship_id",
            "domain_entity_id",
        ];

        if !valid_tables.contains(&source_table) || !valid_tables.contains(&target_table) {
            anyhow::bail!("Invalid KG table in orphan check");
        }
        if !valid_cols.contains(&fk_col) {
            anyhow::bail!("Invalid KG FK column: {fk_col}");
        }

        let sql = format!(
            "SELECT COUNT(*), MIN(id) FROM {source_table} WHERE {fk_col} NOT IN (SELECT id FROM {target_table})"
        );
        let (count, example) = self.conn.query_row(&sql, [], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?))
        })?;
        Ok((count as u64, example))
    }

    /// Rolling-window outcome stats from `kg_resolutions_log` (#1077).
    ///
    /// When `docs_root_hash` is `Some`, scopes to a single corpus via JOIN
    /// through `kg_subject_entities`. When `None`, returns agent-wide stats
    /// (no JOIN needed).
    ///
    /// The `attempted` denominator excludes structural skips and errors per
    /// design decision D6 — only outcomes representing genuine resolution
    /// attempts are counted.
    pub fn kg_resolution_outcome_stats(
        &self,
        agent_id: &str,
        docs_root_hash: Option<&str>,
        window_days: u32,
    ) -> Result<kg_schema::ResolutionOutcomeStats> {
        let window_param = format!("-{window_days} days");

        if let Some(hash) = docs_root_hash {
            // Per-corpus: JOIN through kg_subject_entities for docs_root_hash filter.
            // COALESCE guards against NULL when no rows match (SUM returns NULL on empty set).
            let sql = r#"
                SELECT
                    COUNT(*) as total,
                    COALESCE(SUM(CASE WHEN rl.outcome IN ('matched_exact','matched_llm','matched_llm_db_fallback','no_match','no_candidate_of_type') THEN 1 ELSE 0 END), 0) as attempted,
                    COALESCE(SUM(CASE WHEN rl.outcome = 'no_match' THEN 1 ELSE 0 END), 0) as no_match,
                    COALESCE(SUM(CASE WHEN rl.outcome = 'no_candidate_of_type' THEN 1 ELSE 0 END), 0) as no_candidate_of_type,
                    COALESCE(SUM(CASE WHEN rl.outcome = 'matched_exact' THEN 1 ELSE 0 END), 0) as matched_exact,
                    COALESCE(SUM(CASE WHEN rl.outcome = 'matched_llm' THEN 1 ELSE 0 END), 0) as matched_llm,
                    COALESCE(SUM(CASE WHEN rl.outcome = 'matched_llm_db_fallback' THEN 1 ELSE 0 END), 0) as matched_llm_db_fallback,
                    COALESCE(SUM(CASE WHEN rl.outcome IN ('skipped_no_llm','skipped_discovered_type','skipped_discovered_subject') THEN 1 ELSE 0 END), 0) as skipped,
                    COALESCE(SUM(CASE WHEN rl.outcome = 'error' THEN 1 ELSE 0 END), 0) as errors
                FROM kg_resolutions_log rl
                JOIN kg_subject_entities se ON se.id = rl.subject_entity_id
                WHERE rl.agent_id = ?1
                  AND se.docs_root_hash = ?2
                  AND rl.resolved_at >= datetime('now', ?3)
            "#;
            self.conn
                .query_row(sql, params![agent_id, hash, window_param], |row| {
                    Ok(kg_schema::ResolutionOutcomeStats {
                        total: row.get::<_, i64>(0)? as u64,
                        attempted: row.get::<_, i64>(1)? as u64,
                        no_match: row.get::<_, i64>(2)? as u64,
                        no_candidate_of_type: row.get::<_, i64>(3)? as u64,
                        matched_exact: row.get::<_, i64>(4)? as u64,
                        matched_llm: row.get::<_, i64>(5)? as u64,
                        matched_llm_db_fallback: row.get::<_, i64>(6)? as u64,
                        skipped: row.get::<_, i64>(7)? as u64,
                        errors: row.get::<_, i64>(8)? as u64,
                    })
                })
                .map_err(Into::into)
        } else {
            // Agent-wide: no JOIN needed.
            // COALESCE guards against NULL when no rows match (SUM returns NULL on empty set).
            let sql = r#"
                SELECT
                    COUNT(*) as total,
                    COALESCE(SUM(CASE WHEN outcome IN ('matched_exact','matched_llm','matched_llm_db_fallback','no_match','no_candidate_of_type') THEN 1 ELSE 0 END), 0) as attempted,
                    COALESCE(SUM(CASE WHEN outcome = 'no_match' THEN 1 ELSE 0 END), 0) as no_match,
                    COALESCE(SUM(CASE WHEN outcome = 'no_candidate_of_type' THEN 1 ELSE 0 END), 0) as no_candidate_of_type,
                    COALESCE(SUM(CASE WHEN outcome = 'matched_exact' THEN 1 ELSE 0 END), 0) as matched_exact,
                    COALESCE(SUM(CASE WHEN outcome = 'matched_llm' THEN 1 ELSE 0 END), 0) as matched_llm,
                    COALESCE(SUM(CASE WHEN outcome = 'matched_llm_db_fallback' THEN 1 ELSE 0 END), 0) as matched_llm_db_fallback,
                    COALESCE(SUM(CASE WHEN outcome IN ('skipped_no_llm','skipped_discovered_type','skipped_discovered_subject') THEN 1 ELSE 0 END), 0) as skipped,
                    COALESCE(SUM(CASE WHEN outcome = 'error' THEN 1 ELSE 0 END), 0) as errors
                FROM kg_resolutions_log
                WHERE agent_id = ?1
                  AND resolved_at >= datetime('now', ?2)
            "#;
            self.conn
                .query_row(sql, params![agent_id, window_param], |row| {
                    Ok(kg_schema::ResolutionOutcomeStats {
                        total: row.get::<_, i64>(0)? as u64,
                        attempted: row.get::<_, i64>(1)? as u64,
                        no_match: row.get::<_, i64>(2)? as u64,
                        no_candidate_of_type: row.get::<_, i64>(3)? as u64,
                        matched_exact: row.get::<_, i64>(4)? as u64,
                        matched_llm: row.get::<_, i64>(5)? as u64,
                        matched_llm_db_fallback: row.get::<_, i64>(6)? as u64,
                        skipped: row.get::<_, i64>(7)? as u64,
                        errors: row.get::<_, i64>(8)? as u64,
                    })
                })
                .map_err(Into::into)
        }
    }

    /// Count kg_chunks rows with NULL source_doc_hash.
    pub fn kg_count_null_hash(&self) -> Result<u64> {
        let count = self.conn.query_row(
            "SELECT COUNT(*) FROM kg_chunks WHERE source_doc_hash IS NULL",
            [],
            |r| r.get::<_, i64>(0),
        )?;
        Ok(count as u64)
    }
}

// ===== Tests =====

// `pub(crate)` for the test build only: `skills::executor::tests::harnais_porte`
// (mika#2310, cases 9 / 9b) reuses the groom-pair fixtures defined below rather
// than duplicating them, per the ticket's "reuse, do not duplicate". No
// production visibility changes — the module is `#[cfg(test)]`.
#[cfg(test)]
pub(crate) mod tests;

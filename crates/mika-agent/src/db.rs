pub mod kg_schema;
pub mod operational;

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

    fn migrate(&mut self) -> Result<()> {
        let version = self.current_version()?;
        debug!(
            current_version = version,
            target_version = CURRENT_SCHEMA_VERSION,
            "checking migrations"
        );
        if version < 1 {
            if version > 0 {
                info!(from = version, to = 1, "applying clean-slate migration");
            }
            self.migrate_v1()?;
            info!(
                version = CURRENT_SCHEMA_VERSION,
                "database migrated to v{CURRENT_SCHEMA_VERSION}"
            );
        }
        if (1..3).contains(&version) {
            self.migrate_v3()?;
            info!(version = 3, "database migrated to v3");
        }
        if version == 3 {
            self.migrate_v3_to_v4()?;
            info!(version = 4, "database migrated to v4");
        }
        if version == 4 || version == 3 {
            self.migrate_v4_to_v5()?;
            info!(version = 5, "database migrated to v5");
        }
        if (3..=5).contains(&version) {
            self.migrate_v5_to_v6()?;
            info!(version = 6, "database migrated to v6");
        }
        if (3..=6).contains(&version) {
            self.migrate_v6_to_v7()?;
            info!(version = 7, "database migrated to v7");
        }
        if (3..=7).contains(&version) {
            self.migrate_v7_to_v8()?;
            info!(version = 8, "database migrated to v8");
        }
        if (3..=8).contains(&version) {
            self.migrate_v8_to_v9()?;
            info!(version = 9, "database migrated to v9");
        }
        if (3..=9).contains(&version) {
            self.migrate_v9_to_v10()?;
            info!(version = 10, "database migrated to v10");
        }
        if (3..=10).contains(&version) {
            self.migrate_v10_to_v11()?;
            info!(version = 11, "database migrated to v11");
        }
        if (3..=11).contains(&version) {
            self.migrate_v11_to_v12()?;
            info!(version = 12, "database migrated to v12");
        }
        if (3..=12).contains(&version) {
            self.migrate_v12_to_v13()?;
            info!(version = 13, "database migrated to v13");
        }
        if (3..=13).contains(&version) {
            self.migrate_v13_to_v14()?;
            info!(version = 14, "database migrated to v14");
        }
        if (3..=14).contains(&version) {
            self.migrate_v14_to_v15()?;
            info!(version = 15, "database migrated to v15");
        }
        if (3..=15).contains(&version) {
            self.migrate_v15_to_v16()?;
            info!(version = 16, "database migrated to v16");
        }
        if (3..=16).contains(&version) {
            self.migrate_v16_to_v17()?;
            info!(version = 17, "database migrated to v17");
        }
        if (3..=17).contains(&version) {
            self.migrate_v17_to_v18()?;
            info!(version = 18, "database migrated to v18");
        }
        if (3..=18).contains(&version) {
            self.migrate_v18_to_v19()?;
            info!(version = 19, "database migrated to v19");
        }
        if (3..=19).contains(&version) {
            self.migrate_v19_to_v20()?;
            info!(version = 20, "database migrated to v20");
        }
        if (3..=20).contains(&version) {
            self.migrate_v20_to_v21()?;
            info!(version = 21, "database migrated to v21");
        }
        if (3..=21).contains(&version) {
            self.migrate_v21_to_v22()?;
            info!(version = 22, "database migrated to v22");
        }
        if (3..=22).contains(&version) {
            self.migrate_v22_to_v23()?;
            info!(version = 23, "database migrated to v23");
        }
        if (3..=23).contains(&version) {
            self.migrate_v23_to_v24()?;
            info!(version = 24, "database migrated to v24");
        }
        if (3..=24).contains(&version) {
            self.migrate_v24_to_v25()?;
            info!(version = 25, "database migrated to v25");
        }
        if (3..=25).contains(&version) {
            self.migrate_v25_to_v26()?;
            info!(version = 26, "database migrated to v26");
        }
        if (3..=26).contains(&version) {
            self.migrate_v26_to_v27()?;
            info!(version = 27, "database migrated to v27");
        }

        // v27 startup guard: refuse to return a Database handle if the
        // coalesce step from #787 has not run. Fresh installs write the
        // marker in migrate_v1; existing DBs upgraded via the stub get
        // the marker only when #787's coalesce SQL runs.
        self.check_v27_coalesce_guard()?;

        if (3..=27).contains(&version) {
            self.migrate_v27_to_v28()?;
            info!(version = 28, "database migrated to v28");
        }

        if (3..=28).contains(&version) {
            self.migrate_v28_to_v29()?;
            info!(version = 29, "database migrated to v29");
        }

        if (3..=29).contains(&version) {
            self.migrate_v29_to_v30()?;
            info!(version = 30, "database migrated to v30");
        }

        if (3..=30).contains(&version) {
            self.migrate_v30_to_v31()?;
            info!(version = 31, "database migrated to v31");
        }

        if (3..=31).contains(&version) {
            self.migrate_v31_to_v32()?;
            info!(version = 32, "database migrated to v32");
        }

        if (3..=32).contains(&version) {
            self.migrate_v32_to_v33()?;
            info!(version = 33, "database migrated to v33");
        }

        if (3..=33).contains(&version) {
            self.migrate_v33_to_v34()?;
            info!(version = 34, "database migrated to v34");
        }

        if (3..=34).contains(&version) {
            self.migrate_v34_to_v35()?;
            info!(version = 35, "database migrated to v35");
        }

        if (3..=35).contains(&version) {
            self.migrate_v35_to_v36()?;
            info!(version = 36, "database migrated to v36");
        }

        if (3..=36).contains(&version) {
            self.migrate_v36_to_v37()?;
            info!(version = 37, "database migrated to v37");
        }

        if (3..=37).contains(&version) {
            self.migrate_v37_to_v38()?;
            info!(version = 38, "database migrated to v38");
        }

        if (3..=38).contains(&version) {
            self.migrate_v38_to_v39()?;
            info!(version = 39, "database migrated to v39");
        }

        if (3..=39).contains(&version) {
            self.migrate_v39_to_v40()?;
            info!(version = 40, "database migrated to v40");
        }

        if (3..=40).contains(&version) {
            self.migrate_v40_to_v41()?;
            info!(version = 41, "database migrated to v41");
        }

        if (3..=41).contains(&version) {
            self.migrate_v41_to_v42()?;
            info!(version = 42, "database migrated to v42");
        }

        if (3..=42).contains(&version) {
            self.migrate_v42_to_v43()?;
            info!(version = 43, "database migrated to v43");
        }

        if (3..=43).contains(&version) {
            self.migrate_v43_to_v44()?;
            info!(version = 44, "database migrated to v44");
        }

        if (3..=44).contains(&version) {
            self.migrate_v44_to_v45()?;
            info!(version = 45, "database migrated to v45");
        }

        if (3..=45).contains(&version) {
            self.migrate_v45_to_v46()?;
            info!(version = 46, "database migrated to v46");
        }

        if (3..=46).contains(&version) {
            self.migrate_v46_to_v47()?;
            info!(version = 47, "database migrated to v47");
        }

        if (3..=47).contains(&version) {
            self.migrate_v47_to_v48()?;
            info!(version = 48, "database migrated to v48");
        }

        if (3..=48).contains(&version) {
            self.migrate_v48_to_v49()?;
            info!(version = 49, "database migrated to v49");
        }

        if (3..=49).contains(&version) {
            self.migrate_v49_to_v50()?;
            info!(version = 50, "database migrated to v50");
        }

        if (3..=50).contains(&version) {
            self.migrate_v50_to_v51()?;
            info!(version = 51, "database migrated to v51");
        }

        if (3..=51).contains(&version) {
            self.migrate_v51_to_v52()?;
            info!(version = 52, "database migrated to v52");
        }

        if (3..=52).contains(&version) {
            self.migrate_v52_to_v53()?;
            info!(version = 53, "database migrated to v53");
        }

        if (3..=53).contains(&version) {
            self.migrate_v53_to_v54()?;
            info!(version = 54, "database migrated to v54");
        }

        Ok(())
    }

    fn migrate_v1(&mut self) -> Result<()> {
        info!("applying migration v1: unified task engine schema (clean slate)");

        // Drop all existing tables (clean slate — no backward compat constraint)
        let drops = [
            "DROP TABLE IF EXISTS fts_search",
            "DROP TABLE IF EXISTS vec_search",
            "DROP TABLE IF EXISTS tool_calls",
            "DROP TABLE IF EXISTS llm_calls",
            "DROP TABLE IF EXISTS a2a_push_notification_configs",
            "DROP TABLE IF EXISTS a2a_artifacts",
            "DROP TABLE IF EXISTS a2a_messages",
            "DROP TABLE IF EXISTS a2a_tasks",
            "DROP TABLE IF EXISTS a2a_task_map",
            "DROP TABLE IF EXISTS reminders",
            "DROP TABLE IF EXISTS heartbeat_sends",
            "DROP TABLE IF EXISTS reflection_runs",
            "DROP TABLE IF EXISTS failed_sends",
            "DROP TABLE IF EXISTS customer_config",
            "DROP TABLE IF EXISTS audit_event_summaries",
            "DROP TABLE IF EXISTS audit_events",
            "DROP TABLE IF EXISTS memory_event_summaries",
            "DROP TABLE IF EXISTS memory_events",
            "DROP TABLE IF EXISTS search_content",
            "DROP TABLE IF EXISTS events",
            "DROP TABLE IF EXISTS preferences",
            "DROP TABLE IF EXISTS commitments",
            "DROP TABLE IF EXISTS people",
            "DROP TABLE IF EXISTS core_memory",
            "DROP TABLE IF EXISTS team_workspace",
            "DROP TABLE IF EXISTS team_messages",
            "DROP TABLE IF EXISTS team_runs",
            "DROP TABLE IF EXISTS messages",
            "DROP TABLE IF EXISTS sessions",
            "DROP TABLE IF EXISTS conversations",
            "DROP TABLE IF EXISTS tasks",
            "DROP TABLE IF EXISTS teams",
            "DROP TABLE IF EXISTS agents",
            "DROP TABLE IF EXISTS schema_version",
        ];
        for drop in &drops {
            self.conn.execute_batch(drop)?;
        }

        let tx = self.conn.transaction()?;
        tx.execute_batch(
                "
            CREATE TABLE schema_version (
                version INTEGER NOT NULL,
                applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO schema_version (version) VALUES (54);

            -- Schema meta table for migration state tracking (v27+).
            CREATE TABLE schema_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            -- Fresh installs are trivially coalesce-complete (no v26 data).
            INSERT INTO schema_meta (key, value) VALUES ('v27_coalesce_complete', '1');

            CREATE TABLE agents (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL COLLATE NOCASE,
                home_dir TEXT NOT NULL DEFAULT '',
                active BOOLEAN NOT NULL DEFAULT 1,
                last_seen TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            -- Per-agent KG corpus mapping (#798). Maps agent_id to
            -- docs_root_hash for multi-corpus query fan-out.
            CREATE TABLE agent_kg_corpora (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                docs_root_hash TEXT NOT NULL,
                docs_root_path TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (agent_id, docs_root_hash)
            );
            CREATE INDEX idx_agent_kg_corpora_hash ON agent_kg_corpora(docs_root_hash);

            CREATE TABLE teams (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL COLLATE NOCASE,
                config_path TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE TABLE team_runs (
                id TEXT PRIMARY KEY,
                team_id TEXT NOT NULL REFERENCES teams(id),
                goal TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'running'
                    CHECK (status IN ('running','completed','failed','cancelled','suspended','failed_no_delegation','failed_transport')),
                failure_reason TEXT,
                iteration INTEGER NOT NULL DEFAULT 1,
                max_iterations INTEGER NOT NULL DEFAULT 3,
                deliverable TEXT,
                checkpoint TEXT,
                trace_id TEXT,
                started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                ended_at TEXT,
                delegation_count INTEGER NOT NULL DEFAULT 0,
                solo_absorption INTEGER NOT NULL DEFAULT 0,
                failure_context TEXT
            );
            CREATE INDEX idx_team_runs_team ON team_runs(team_id, started_at DESC);

            CREATE TABLE tasks (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                team_run_id TEXT REFERENCES team_runs(id) ON DELETE SET NULL,
                parent_task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
                depth INTEGER NOT NULL DEFAULT 0 CHECK (depth BETWEEN 0 AND 3),
                label TEXT NOT NULL,
                trigger_type TEXT NOT NULL CHECK (
                    trigger_type IN ('time','recurring','callback','user_reply','event','condition','manual','a2a')
                ),
                cron_expr TEXT,
                event_source TEXT,
                event_offset_secs INTEGER,
                condition_expr TEXT,
                next_fire_at TEXT,
                timeout_at TEXT,
                action_type TEXT NOT NULL CHECK (
                    action_type IN (
                        'send_message','resume_agent','inject_context',
                        'run_skill','invoke_orchestrator','none'
                    )
                ),
                action_config TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'pending' CHECK (
                    status IN ('pending','in_progress','completed','failed',
                               'cancelled','expired','recurring_active','delivered','blocked')
                ),
                process_id INTEGER,
                input_context TEXT,
                result TEXT,
                reference_url TEXT,
                source TEXT,
                metadata TEXT,
                type TEXT NOT NULL DEFAULT 'issue' CHECK (
                    type IN ('issue', 'milestone', 'project')
                ),
                dispatch_class TEXT CHECK (
                    dispatch_class IS NULL OR dispatch_class IN ('implement', 'groom')
                ),
                -- mika#1948 Porte 2: which dispatcher inside this engine
                -- initiated the task. NULL = pre-v51 row, read as 'mika_dev'.
                dispatcher_source TEXT CHECK (
                    dispatcher_source IS NULL
                    OR dispatcher_source IN ('mika_dev', 'mika_manager', 'operator')
                ),
                created_by_session TEXT,
                created_trace_id TEXT,
                execution_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                fired_at TEXT,
                completed_at TEXT
            );
            CREATE INDEX idx_tasks_agent_status ON tasks(agent_id, status);
            CREATE INDEX idx_tasks_dispatcher_source
                ON tasks(agent_id, dispatcher_source, status)
                WHERE dispatcher_source IS NOT NULL;

            -- mika#1948 Porte 2 — one row per (agent, class, slot) = one exec
            -- slot. The PRIMARY KEY is what makes the claim atomic: a second
            -- claimant's INSERT collides instead of racing a SELECT.
            -- mika#2160: `slot_index` joined the key so the class ceases to be
            -- a hard cap of one. At the default cap of 1 exactly one index (0)
            -- is ever written, which is the pre-v52 shape bit for bit.
            CREATE TABLE dispatch_slot_leases (
                agent_id TEXT NOT NULL,
                dispatch_class TEXT NOT NULL,
                slot_index INTEGER NOT NULL DEFAULT 0,
                holder_task_id TEXT NOT NULL,
                dispatcher_source TEXT CHECK (
                    dispatcher_source IS NULL
                    OR dispatcher_source IN ('mika_dev', 'mika_manager', 'operator')
                ),
                acquired_at TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                PRIMARY KEY (agent_id, dispatch_class, slot_index)
            );

            -- mika#2192 — one row per (repo, issue) = one declared worktree
            -- owner. Twin of dispatch_slot_leases: the PRIMARY KEY is what
            -- makes the claim a fact rather than a convention, and expiry is
            -- what keeps a dead claimant from freezing a ticket forever.
            --
            -- The key is (repo, issue_number) and NOT the worktree path. The
            -- invariants worktree_path_slug == sanitize(branch_ref) and
            -- branch_ref == derive-branch-name(title, issue, labels) make the
            -- pair sufficient, and it is what the tool boundary already holds
            -- (`tool_input.prompt` is `mika#2192`). Keying by path would force
            -- the Rust side to re-derive the branch — the duplication
            -- mika-platform#58 closed.
            CREATE TABLE worktree_claims (
                repo TEXT NOT NULL,
                issue_number INTEGER NOT NULL,
                owner_kind TEXT NOT NULL CHECK (
                    owner_kind IN ('pilot', 'orchestrator', 'spawn')
                ),
                owner_id TEXT NOT NULL,
                owner_label TEXT,
                claimed_at TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                PRIMARY KEY (repo, issue_number)
            );
            CREATE INDEX idx_tasks_next_fire ON tasks(next_fire_at)
                WHERE status IN ('pending','recurring_active');
            CREATE INDEX idx_tasks_schedulable
                ON tasks(agent_id, next_fire_at ASC)
                WHERE status IN ('pending','recurring_active');
            CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_task_id, agent_id) WHERE parent_task_id IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_tasks_session ON tasks(created_by_session) WHERE created_by_session IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_tasks_trace ON tasks(created_trace_id) WHERE created_trace_id IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_tasks_exec_trace ON tasks(execution_trace_id) WHERE execution_trace_id IS NOT NULL;
            CREATE INDEX idx_tasks_manual_active
                ON tasks(agent_id, created_at DESC)
                WHERE trigger_type = 'manual'
                AND status IN ('pending', 'in_progress', 'blocked');
            CREATE UNIQUE INDEX idx_tasks_manual_active_ref_url
                ON tasks(agent_id, reference_url)
                WHERE trigger_type = 'manual'
                AND reference_url IS NOT NULL
                AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered');
            CREATE INDEX idx_tasks_dispatch_class
                ON tasks(agent_id, dispatch_class, status)
                WHERE dispatch_class IS NOT NULL;

            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                channel_type TEXT NOT NULL DEFAULT 'cli',
                started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                ended_at TEXT,
                metadata TEXT,
                parent_session_id TEXT,
                task_id TEXT
            );
            CREATE INDEX idx_sessions_agent ON sessions(agent_id, started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id) WHERE parent_session_id IS NOT NULL;
            CREATE INDEX idx_sessions_task_id ON sessions(task_id) WHERE task_id IS NOT NULL;

            CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                role TEXT NOT NULL CHECK (role IN ('user','assistant','system','summary','tool_result')),
                content TEXT NOT NULL,
                metadata TEXT,
                trace_id TEXT,
                compacted_through_id INTEGER,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                internal INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX idx_msg_session ON messages(session_id, created_at ASC);
            CREATE INDEX idx_msg_agent_created ON messages(agent_id, created_at DESC);
            CREATE INDEX idx_msg_trace ON messages(trace_id) WHERE trace_id IS NOT NULL;

            CREATE TABLE task_messages (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id    TEXT NOT NULL,
                agent_id   TEXT NOT NULL,
                session_id TEXT NOT NULL,
                role       TEXT NOT NULL,
                content    TEXT NOT NULL,
                metadata   TEXT,
                trace_id   TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_task_messages_task_created
                ON task_messages (task_id, created_at);
            CREATE INDEX idx_task_messages_agent_created
                ON task_messages (agent_id, created_at);

            CREATE TABLE core_memory (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                key TEXT NOT NULL COLLATE NOCASE,
                value TEXT NOT NULL,
                token_count INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (agent_id, key)
            );

            CREATE TABLE people (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                canonical_name TEXT NOT NULL COLLATE NOCASE,
                relationship TEXT,
                notes TEXT,
                first_mentioned TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                last_mentioned TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                mention_count INTEGER NOT NULL DEFAULT 1,
                UNIQUE (agent_id, canonical_name)
            );

            CREATE TABLE commitments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                description TEXT NOT NULL COLLATE NOCASE,
                status TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending','completed','cancelled')),
                due_date TEXT,
                person_id INTEGER REFERENCES people(id) ON DELETE SET NULL,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                completed_at TEXT
            );
            CREATE INDEX idx_commit_agent_status ON commitments(agent_id, status);

            CREATE UNIQUE INDEX IF NOT EXISTS idx_commitments_unique_pending
                ON commitments(agent_id, description COLLATE NOCASE, due_date)
                WHERE status = 'pending';

            CREATE TABLE preferences (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                category TEXT NOT NULL COLLATE NOCASE,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (agent_id, category)
            );

            CREATE TABLE events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                description TEXT NOT NULL,
                event_date TEXT,
                context TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE TABLE audit_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                session_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                target_key TEXT NOT NULL,
                before_value TEXT,
                after_value TEXT,
                reasoning TEXT,
                trace_id TEXT,
                rewound_by_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_audit_agent_created ON audit_events(agent_id, created_at DESC);
            CREATE INDEX idx_audit_session ON audit_events(session_id);
            CREATE INDEX idx_audit_trace ON audit_events(trace_id) WHERE trace_id IS NOT NULL;
            CREATE INDEX idx_audit_rewound ON audit_events(rewound_by_trace_id) WHERE rewound_by_trace_id IS NOT NULL;

            CREATE TABLE audit_event_summaries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                year INTEGER NOT NULL,
                month INTEGER NOT NULL,
                summary TEXT NOT NULL,
                event_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                UNIQUE (agent_id, year, month)
            );

            CREATE TABLE search_content (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                source_type TEXT NOT NULL,
                source_id INTEGER,
                content TEXT NOT NULL,
                embedding_json TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_search_agent ON search_content(agent_id, source_type);

            CREATE TABLE team_workspace (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id TEXT NOT NULL REFERENCES team_runs(id) ON DELETE CASCADE,
                parent_id INTEGER REFERENCES team_workspace(id),
                agent_name TEXT,
                entry_type TEXT NOT NULL,
                content TEXT NOT NULL,
                trace_id TEXT,
                iteration INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_team_ws_run ON team_workspace(run_id, created_at);
            CREATE INDEX idx_team_ws_trace ON team_workspace(trace_id)
                WHERE trace_id IS NOT NULL;

            CREATE TABLE heartbeat_sends (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                sent_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_heartbeat_agent ON heartbeat_sends(agent_id, sent_at DESC);

            CREATE TABLE reflection_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                status TEXT NOT NULL,
                changes_made INTEGER NOT NULL DEFAULT 0,
                summary TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_reflect_agent ON reflection_runs(agent_id, created_at DESC);

            CREATE TABLE customer_config (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                key TEXT NOT NULL COLLATE NOCASE,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (agent_id, key)
            );

            CREATE TABLE failed_sends (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                text TEXT NOT NULL,
                request_id TEXT,
                retry_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE TABLE skill_overrides (
                agent_id        TEXT NOT NULL COLLATE NOCASE,
                skill_name      TEXT NOT NULL COLLATE NOCASE,
                always_on       INTEGER,
                llm_provider    TEXT,
                llm_model       TEXT,
                enabled         INTEGER,
                lifecycle_state TEXT CHECK (lifecycle_state IN ('staged', 'active', 'archived')),
                use_count       INTEGER NOT NULL DEFAULT 0,
                last_used_at    TEXT,
                PRIMARY KEY (agent_id, skill_name)
            );

            -- A2A Protocol tables: thin mapping table + genuinely new tables
            CREATE TABLE a2a_task_map (
                a2a_task_id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                context_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_a2a_task_map_task ON a2a_task_map(task_id);

            CREATE TABLE a2a_artifacts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL REFERENCES a2a_task_map(a2a_task_id) ON DELETE CASCADE,
                artifact_id TEXT NOT NULL,
                name TEXT,
                description TEXT,
                parts TEXT NOT NULL,
                metadata TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE TABLE a2a_push_notification_configs (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES a2a_task_map(a2a_task_id) ON DELETE CASCADE,
                url TEXT NOT NULL,
                token TEXT,
                auth_scheme TEXT,
                auth_credentials TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_recurring
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE trigger_type = 'recurring'
                AND status NOT IN ('cancelled', 'failed', 'expired', 'delivered');

            CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_reminder
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE status IN ('pending', 'in_progress', 'recurring_active')
                AND (action_type = 'send_message' OR action_type = 'resume_agent')
                AND trigger_type NOT IN ('callback');

            CREATE UNIQUE INDEX IF NOT EXISTS idx_events_unique_description
                ON events(agent_id, description COLLATE NOCASE, event_date)
                WHERE event_date IS NOT NULL;

            -- Observability: LLM call tracking
            CREATE TABLE llm_calls (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                trace_id TEXT,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER,
                cache_write_tokens INTEGER,
                latency_ms INTEGER NOT NULL DEFAULT 0,
                stop_reason TEXT,
                status TEXT NOT NULL DEFAULT 'success',
                error_message TEXT,
                step INTEGER NOT NULL DEFAULT 0,
                prompt_variant TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                response_text TEXT,
                reasoning TEXT,
                system_prompt_bytes INTEGER,
                request_bytes INTEGER
            );
            CREATE INDEX idx_llm_calls_trace ON llm_calls(trace_id);
            CREATE INDEX idx_llm_calls_session ON llm_calls(session_id);
            CREATE INDEX idx_llm_calls_agent_created ON llm_calls(agent_id, created_at);

            -- Observability: Tool call tracking (full I/O)
            CREATE TABLE tool_calls (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                trace_id TEXT,
                llm_call_id TEXT,
                step INTEGER NOT NULL DEFAULT 0,
                tool_name TEXT NOT NULL,
                tool_source TEXT NOT NULL DEFAULT 'builtin',
                skill_name TEXT,
                input TEXT,
                output TEXT,
                success INTEGER NOT NULL DEFAULT 1,
                non_zero_exit INTEGER NOT NULL DEFAULT 0,
                latency_ms INTEGER NOT NULL DEFAULT 0,
                error_message TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_tool_calls_trace ON tool_calls(trace_id);
            CREATE INDEX idx_tool_calls_session ON tool_calls(session_id);
            CREATE INDEX idx_tool_calls_llm_call ON tool_calls(llm_call_id);
            CREATE INDEX idx_tool_calls_agent_created ON tool_calls(agent_id, created_at);

            -- KG domain layer (global, no agent_id)
            CREATE TABLE kg_entities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                entity_key TEXT NOT NULL UNIQUE,
                type TEXT NOT NULL,
                name TEXT NOT NULL,
                properties_json TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                CHECK (entity_key = type || ':' || name)
            );
            CREATE INDEX idx_kg_entities_type ON kg_entities(type);

            CREATE TABLE kg_relationships (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                from_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
                to_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
                type TEXT NOT NULL,
                properties_json TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_kg_rel_from ON kg_relationships(from_entity_id, type);
            CREATE INDEX idx_kg_rel_to ON kg_relationships(to_entity_id, type);

            -- KG lexical layer (shared by docs_root_hash — v27)
            CREATE TABLE kg_chunks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                docs_root_hash TEXT NOT NULL,
                docs_root TEXT NOT NULL,
                seq_id INTEGER NOT NULL,
                source_doc_path TEXT NOT NULL,
                source_doc_hash TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                trace_id TEXT,
                UNIQUE (docs_root_hash, source_doc_path, seq_id)
            );
            CREATE INDEX idx_kg_chunks_docs_root_hash_doc ON kg_chunks(docs_root_hash, source_doc_path);

            -- KG subject layer (shared by docs_root_hash — v27)
            CREATE TABLE kg_subject_entities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                docs_root_hash TEXT NOT NULL,
                docs_root TEXT NOT NULL,
                entity_key TEXT NOT NULL,
                type TEXT NOT NULL,
                name TEXT NOT NULL,
                confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                properties_json TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                trace_id TEXT,
                discovered INTEGER NOT NULL DEFAULT 0,
                discovery_reason TEXT,
                CHECK (entity_key = type || ':' || name),
                UNIQUE (docs_root_hash, entity_key)
            );
            CREATE INDEX idx_kg_subj_entities_drh_type ON kg_subject_entities(docs_root_hash, type);

            CREATE TABLE kg_subject_resolutions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                domain_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
                confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                trace_id TEXT,
                UNIQUE (agent_id, subject_entity_id, domain_entity_id)
            );
            CREATE INDEX idx_kg_resolutions_agent_subj ON kg_subject_resolutions(agent_id, subject_entity_id);
            CREATE INDEX idx_kg_resolutions_agent_dom ON kg_subject_resolutions(agent_id, domain_entity_id);

            -- KG subject-to-subject edges / fact triples (shared by docs_root_hash — v27)
            CREATE TABLE kg_subject_relationships (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                docs_root_hash TEXT NOT NULL,
                docs_root TEXT NOT NULL,
                from_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                to_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                type TEXT NOT NULL,
                confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                properties_json TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                trace_id TEXT,
                UNIQUE (docs_root_hash, from_entity_id, to_entity_id, type)
            );
            CREATE INDEX idx_kg_subj_rel_from ON kg_subject_relationships(docs_root_hash, from_entity_id, type);
            CREATE INDEX idx_kg_subj_rel_to ON kg_subject_relationships(docs_root_hash, to_entity_id, type);
            CREATE INDEX idx_kg_subj_rel_type ON kg_subject_relationships(docs_root_hash, type);

            -- KG entity provenance: chunk -> subject entity (shared by docs_root_hash — v27)
            CREATE TABLE kg_chunk_subjects (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                docs_root_hash TEXT NOT NULL,
                docs_root TEXT NOT NULL,
                chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
                subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                extraction_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                UNIQUE (docs_root_hash, chunk_id, subject_entity_id)
            );
            CREATE INDEX idx_kg_cs_chunk ON kg_chunk_subjects(docs_root_hash, chunk_id);
            CREATE INDEX idx_kg_cs_entity ON kg_chunk_subjects(docs_root_hash, subject_entity_id);
            CREATE INDEX idx_kg_cs_trace ON kg_chunk_subjects(docs_root_hash, extraction_trace_id);

            -- KG relationship provenance: chunk -> subject relationship (shared by docs_root_hash — v27)
            CREATE TABLE kg_chunk_subject_relationships (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                docs_root_hash TEXT NOT NULL,
                docs_root TEXT NOT NULL,
                chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
                subject_relationship_id INTEGER NOT NULL REFERENCES kg_subject_relationships(id) ON DELETE CASCADE,
                extraction_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                UNIQUE (docs_root_hash, chunk_id, subject_relationship_id)
            );
            CREATE INDEX idx_kg_csr_chunk ON kg_chunk_subject_relationships(docs_root_hash, chunk_id);
            CREATE INDEX idx_kg_csr_rel ON kg_chunk_subject_relationships(docs_root_hash, subject_relationship_id);

            -- KG extraction tracking (shared by docs_root_hash — v27; first-writer-wins)
            CREATE TABLE kg_extractions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                docs_root_hash TEXT NOT NULL,
                docs_root TEXT NOT NULL,
                source_doc_path TEXT NOT NULL,
                source_doc_hash TEXT,
                extraction_model TEXT NOT NULL,
                entities_extracted INTEGER NOT NULL DEFAULT 0,
                relationships_extracted INTEGER NOT NULL DEFAULT 0,
                extraction_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                UNIQUE (docs_root_hash, source_doc_path)
            );
            CREATE INDEX idx_kg_extractions_drh ON kg_extractions(docs_root_hash);

            -- KG resolution tracking
            CREATE TABLE kg_resolutions_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                outcome TEXT NOT NULL CHECK (outcome IN (
                    'matched_exact', 'matched_llm', 'matched_llm_db_fallback',
                    'no_match', 'no_candidate_of_type',
                    'skipped_discovered_type', 'skipped_discovered_subject',
                    'skipped_no_llm', 'error'
                )),
                resolution_trace_id TEXT NOT NULL,
                source_extraction_trace_id TEXT,
                model TEXT,
                duration_ms INTEGER,
                resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                UNIQUE (agent_id, subject_entity_id)
            );
            CREATE INDEX idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);

            -- KG invalidation markers (#961): ephemeral sidecar for tracking
            -- entities whose no_match resolution log rows were deleted by
            -- domain-graph rebuild invalidation (#960).
            CREATE TABLE kg_invalidated_no_match (
                subject_entity_id INTEGER NOT NULL,
                agent_id TEXT NOT NULL,
                invalidated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (subject_entity_id, agent_id)
            );

            -- Operational ledger (#1262): canonical operational-item store.
            CREATE TABLE operational_items (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL CHECK (kind IN ('goal', 'task', 'commitment', 'decision', 'blocker', 'evidence', 'next_action')),
                title TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('now', 'waiting', 'delegated', 'scheduled', 'at_risk', 'done')),
                owner_type TEXT NOT NULL CHECK (owner_type IN ('user', 'mika', 'person', 'agent')),
                owner_name TEXT,
                priority REAL NOT NULL DEFAULT 0.0,
                user_importance REAL NOT NULL DEFAULT 0.0,
                due_at TEXT,
                blocked_by TEXT,
                next_action TEXT,
                evidence_refs TEXT NOT NULL DEFAULT '[]',
                confidence REAL NOT NULL DEFAULT 1.0,
                source_table TEXT,
                source_id TEXT,
                agent_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX idx_operational_items_agent_status ON operational_items(agent_id, status);
            CREATE INDEX idx_operational_items_agent_kind ON operational_items(agent_id, kind);
            CREATE INDEX idx_operational_items_agent_priority ON operational_items(agent_id, priority DESC);
            CREATE INDEX idx_operational_items_source ON operational_items(source_table, source_id);
            CREATE UNIQUE INDEX idx_operational_items_source_unique
                ON operational_items(agent_id, source_table, source_id)
                WHERE source_table IS NOT NULL AND source_id IS NOT NULL;

            -- Auto-pull circuit-breaker stats (mika#1363) + re-drive budget (mika#2020)
            CREATE TABLE auto_pull_stats (
                repo_full_name TEXT NOT NULL,
                issue_number INTEGER NOT NULL,
                failure_count INTEGER NOT NULL DEFAULT 0,
                last_auto_pull_at TEXT,
                last_failure_at TEXT,
                redrive_count INTEGER NOT NULL DEFAULT 0,
                last_redrive_at TEXT,
                redrive_abandoned_at TEXT,
                PRIMARY KEY (repo_full_name, issue_number)
            );

            -- Permission-decision provenance ledger (mika#1733 AC4)
            CREATE TABLE permission_decisions (
                id TEXT PRIMARY KEY,
                request_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                args_summary TEXT,
                classifier_verdict TEXT NOT NULL
                    CHECK (classifier_verdict IN ('approved', 'denied', 'held')),
                operator_decision TEXT
                    CHECK (operator_decision IN ('approve', 'deny')),
                override_used INTEGER NOT NULL DEFAULT 0
                    CHECK (override_used IN (0, 1)),
                decision_authority TEXT NOT NULL
                    CHECK (decision_authority IN ('strict', 'override')),
                tenant_id TEXT,
                agent_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_permission_decisions_request_id
                ON permission_decisions(request_id);
            CREATE INDEX idx_permission_decisions_created_at
                ON permission_decisions(created_at DESC);

            -- v45: pilot_transcripts (mika#1705). Must be in v1 DDL so fresh installs
            -- get the table without depending on the v44→v45 migration chain (which is
            -- skipped on fresh install because migrate() captures version BEFORE
            -- migrate_v1 runs and the (3..=44).contains(&0) guard fails).
            CREATE TABLE pilot_transcripts (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL,
                timestamp TEXT,
                provider TEXT,
                model TEXT,
                request_body TEXT,
                response_body TEXT,
                tokens_in INTEGER,
                tokens_out INTEGER,
                latency_ms INTEGER,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_pilot_transcripts_task_id
                ON pilot_transcripts(task_id);
            CREATE INDEX idx_pilot_transcripts_created_at
                ON pilot_transcripts(created_at DESC);

            -- v46: served_content (mika#1867). Per-(agent, person, category) ledger
            -- of content Mika has served (proverb, quote, joke, poem, recommendation,
            -- story, fact) so re-generation on future turns can dedup. Founding
            -- incident: Al (Vietnam tester) 2026-07-28 — same zen proverb served
            -- twice, 6 days apart. Must be in v1 DDL so fresh installs get the
            -- table without depending on the v45→v46 migration chain.
            CREATE TABLE served_content (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                person_id INTEGER NOT NULL REFERENCES people(id) ON DELETE CASCADE,
                category TEXT NOT NULL CHECK (category IN (
                    'proverb','quote','joke','poem','recommendation','story','fact'
                )),
                content_text TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                -- Reserved for v2 fuzzy dedup (embedding cosine similarity per AC6).
                -- Format TBD — likely 384-dim float array as BLOB or hex string.
                content_signature TEXT,
                served_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
                UNIQUE(agent_id, person_id, content_hash)
            );
            CREATE INDEX idx_served_content_person_cat
                ON served_content(agent_id, person_id, category, served_at DESC);
            CREATE INDEX idx_served_content_hash
                ON served_content(agent_id, person_id, content_hash);

            -- Pre-register the default 'mika' agent
            INSERT INTO agents (id, name, home_dir) VALUES ('mika', 'Mika', '');
            ",
            )
            .context("failed to create v1 schema")?;
        tx.commit()?;

        // Unified timeline VIEW (uses shared constant)
        self.conn
            .execute_batch(UNIFIED_TIMELINE_VIEW_SQL)
            .context("failed to create unified_timeline view")?;

        // Virtual tables must be outside transactions
        let _ = self.conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS fts_search
                 USING fts5(content, content='search_content', content_rowid='id');
             CREATE VIRTUAL TABLE IF NOT EXISTS vec_search
                 USING vec0(embedding float[512]);",
        );

        Ok(())
    }

    /// Migration v3: Sessions + Messages schema redesign (clean-slate).
    ///
    /// Single user, no data to preserve. Drop and recreate via migrate_v1.
    fn migrate_v3(&mut self) -> Result<()> {
        info!("applying migration v3: sessions + messages schema redesign (clean slate)");
        self.migrate_v1()
    }

    /// Migration v3 → v4: Add duplicate-prevention indexes to existing v3 databases.
    ///
    /// New databases already get these indexes via `migrate_v1`, but databases
    /// created at v3 before these indexes were added need them applied retroactively.
    fn migrate_v3_to_v4(&self) -> Result<()> {
        info!("migrating database schema v3 → v4 (duplicate-prevention indexes)");
        self.conn.execute_batch(
            "DROP INDEX IF EXISTS idx_tasks_unique_recurring;
             CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_recurring
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE trigger_type = 'recurring'
                AND status NOT IN ('cancelled', 'failed', 'expired', 'delivered');

             CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_reminder
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE status IN ('pending', 'in_progress', 'recurring_active')
                AND (action_type = 'send_message' OR action_type = 'resume_agent')
                AND trigger_type NOT IN ('callback');

             CREATE UNIQUE INDEX IF NOT EXISTS idx_events_unique_description
                ON events(agent_id, description COLLATE NOCASE, event_date)
                WHERE event_date IS NOT NULL;

             -- Rebuild commitments table: replace inline UNIQUE(agent_id, description)
             -- with partial unique index scoped to pending status
             CREATE TABLE commitments_new (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                 description TEXT NOT NULL COLLATE NOCASE,
                 status TEXT NOT NULL DEFAULT 'pending'
                     CHECK (status IN ('pending','completed','cancelled')),
                 due_date TEXT,
                 person_id INTEGER REFERENCES people(id) ON DELETE SET NULL,
                 created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                 completed_at INTEGER
             );
             INSERT INTO commitments_new SELECT * FROM commitments;
             DROP TABLE commitments;
             ALTER TABLE commitments_new RENAME TO commitments;
             CREATE INDEX idx_commit_agent_status ON commitments(agent_id, status);
             CREATE UNIQUE INDEX IF NOT EXISTS idx_commitments_unique_pending
                 ON commitments(agent_id, description COLLATE NOCASE, due_date)
                 WHERE status = 'pending';

             PRAGMA user_version = 4;",
        )?;
        // Update the schema_version table to reflect v4
        self.conn
            .execute("INSERT INTO schema_version (version) VALUES (4)", [])?;
        Ok(())
    }

    /// Migration v4 → v5: Rename memory_events → audit_events, add trace_id columns,
    /// create unified_timeline VIEW.
    ///
    /// Idempotent: each step checks existence before acting, since `ALTER TABLE RENAME TO`
    /// auto-commits outside transactions in SQLite and a crash mid-migration could leave
    /// partial state.
    fn migrate_v4_to_v5(&self) -> Result<()> {
        info!("migrating database schema v4 → v5 (orthogonal observability)");

        // 1. Rename memory_events → audit_events (idempotent)
        let has_old: bool = self.conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='memory_events'",
            [],
            |r| r.get(0),
        )?;
        if has_old {
            self.conn
                .execute_batch("ALTER TABLE memory_events RENAME TO audit_events")?;
        }

        // 2. Recreate indexes with new names
        self.conn.execute_batch(
            "DROP INDEX IF EXISTS idx_memev_agent_created;
             DROP INDEX IF EXISTS idx_memev_session;
             CREATE INDEX IF NOT EXISTS idx_audit_agent_created ON audit_events(agent_id, created_at DESC);
             CREATE INDEX IF NOT EXISTS idx_audit_session ON audit_events(session_id);",
        )?;

        // 3. Rename memory_event_summaries → audit_event_summaries (idempotent)
        let has_old_summaries: bool = self.conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='memory_event_summaries'",
            [],
            |r| r.get(0),
        )?;
        if has_old_summaries {
            self.conn.execute_batch(
                "ALTER TABLE memory_event_summaries RENAME TO audit_event_summaries",
            )?;
        }

        // 4. Add trace_id columns (idempotent — ALTER TABLE ADD COLUMN is a no-op if exists)
        // We check column existence via pragma to avoid "duplicate column" errors on re-run.
        if !self.column_exists("messages", "trace_id")? {
            self.conn
                .execute_batch("ALTER TABLE messages ADD COLUMN trace_id TEXT")?;
        }
        if !self.column_exists("tasks", "created_trace_id")? {
            self.conn
                .execute_batch("ALTER TABLE tasks ADD COLUMN created_trace_id TEXT")?;
        }
        if !self.column_exists("audit_events", "trace_id")? {
            self.conn
                .execute_batch("ALTER TABLE audit_events ADD COLUMN trace_id TEXT")?;
        }
        if !self.column_exists("team_workspace", "trace_id")? {
            self.conn
                .execute_batch("ALTER TABLE team_workspace ADD COLUMN trace_id TEXT")?;
        }

        // 5. Create partial indexes on trace_id columns
        self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_msg_trace ON messages(trace_id) WHERE trace_id IS NOT NULL;
             CREATE INDEX IF NOT EXISTS idx_audit_trace ON audit_events(trace_id) WHERE trace_id IS NOT NULL;
             CREATE INDEX IF NOT EXISTS idx_tasks_trace ON tasks(created_trace_id) WHERE created_trace_id IS NOT NULL;",
        )?;

        // 6. Create unified_timeline VIEW (uses shared constant)
        self.conn
            .execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;
        self.conn.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        // 7. Update schema version
        self.conn
            .execute("INSERT INTO schema_version (version) VALUES (5)", [])?;

        Ok(())
    }

    fn migrate_v5_to_v6(&self) -> Result<()> {
        info!("migrating database schema v5 → v6 (people mention_count)");

        if !self.column_exists("people", "mention_count")? {
            self.conn.execute_batch(
                "ALTER TABLE people ADD COLUMN mention_count INTEGER NOT NULL DEFAULT 1",
            )?;
        }

        self.conn
            .execute("INSERT INTO schema_version (version) VALUES (6)", [])?;

        Ok(())
    }

    fn migrate_v6_to_v7(&self) -> Result<()> {
        info!("migrating database schema v6 → v7 (skill_overrides table)");

        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS skill_overrides (
                agent_id   TEXT NOT NULL COLLATE NOCASE,
                skill_name TEXT NOT NULL COLLATE NOCASE,
                always_on  INTEGER,
                PRIMARY KEY (agent_id, skill_name)
            )",
        )?;

        self.conn
            .execute("INSERT INTO schema_version (version) VALUES (7)", [])?;

        Ok(())
    }

    fn migrate_v7_to_v8(&mut self) -> Result<()> {
        info!(
            "migrating database schema v7 → v8 (tasks: manual trigger_type, blocked status, none action_type, reference_url, source)"
        );

        // SQLite cannot ALTER CHECK constraints, so we must rebuild the tasks table.
        // Entire migration wrapped in a transaction to prevent partial state on crash.
        //
        // PRAGMA foreign_keys must be OFF during the table rebuild because the INSERT
        // copies self-referencing parent_task_id rows, and ALTER TABLE RENAME validates
        // FK references. Also disable FK checks to avoid issues with the temporary table.
        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Drop the unified_timeline VIEW first — it references the `tasks` table.
        // SQLite 3.25+ validates all views/triggers during ALTER TABLE RENAME,
        // so the view must not exist when we rename tasks_new → tasks.
        tx.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;

        tx.execute_batch(
            "CREATE TABLE tasks_new (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                team_run_id TEXT REFERENCES team_runs(id) ON DELETE SET NULL,
                parent_task_id TEXT REFERENCES tasks_new(id) ON DELETE SET NULL,
                depth INTEGER NOT NULL DEFAULT 0 CHECK (depth BETWEEN 0 AND 3),
                label TEXT NOT NULL,
                trigger_type TEXT NOT NULL CHECK (
                    trigger_type IN ('time','recurring','callback','user_reply','event','condition','manual')
                ),
                cron_expr TEXT,
                event_source TEXT,
                event_offset_secs INTEGER,
                condition_expr TEXT,
                next_fire_at INTEGER,
                timeout_at INTEGER,
                action_type TEXT NOT NULL CHECK (
                    action_type IN (
                        'send_message','resume_agent','inject_context',
                        'run_skill','invoke_orchestrator','none'
                    )
                ),
                action_config TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'pending' CHECK (
                    status IN ('pending','in_progress','completed','failed',
                               'cancelled','expired','recurring_active','delivered','blocked')
                ),
                process_id INTEGER,
                input_context TEXT,
                result TEXT,
                reference_url TEXT,
                source TEXT,
                created_by_session TEXT,
                created_trace_id TEXT,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
                fired_at INTEGER,
                completed_at INTEGER
            );

            INSERT INTO tasks_new (
                id, agent_id, team_run_id, parent_task_id, depth, label,
                trigger_type, cron_expr, event_source, event_offset_secs, condition_expr,
                next_fire_at, timeout_at, action_type, action_config,
                status, process_id, input_context, result,
                created_by_session, created_trace_id,
                created_at, updated_at, fired_at, completed_at
            )
            SELECT
                id, agent_id, team_run_id, parent_task_id, depth, label,
                trigger_type, cron_expr, event_source, event_offset_secs, condition_expr,
                next_fire_at, timeout_at, action_type, action_config,
                status, process_id, input_context, result,
                created_by_session, created_trace_id,
                created_at, updated_at, fired_at, completed_at
            FROM tasks;

            DROP TABLE tasks;
            ALTER TABLE tasks_new RENAME TO tasks;",
        )?;

        // Recreate all indexes (still within the transaction)
        tx.execute_batch(
            "CREATE INDEX idx_tasks_agent_status ON tasks(agent_id, status);
             CREATE INDEX idx_tasks_next_fire ON tasks(next_fire_at)
                WHERE status IN ('pending','recurring_active');
             CREATE INDEX idx_tasks_schedulable
                ON tasks(agent_id, next_fire_at ASC)
                WHERE status IN ('pending','recurring_active');
             CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_task_id, agent_id) WHERE parent_task_id IS NOT NULL;
             CREATE INDEX IF NOT EXISTS idx_tasks_session ON tasks(created_by_session) WHERE created_by_session IS NOT NULL;
             CREATE INDEX IF NOT EXISTS idx_tasks_trace ON tasks(created_trace_id) WHERE created_trace_id IS NOT NULL;
             CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_recurring
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE trigger_type = 'recurring'
                AND status NOT IN ('cancelled', 'failed', 'expired', 'delivered');
             CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_reminder
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE status IN ('pending', 'in_progress', 'recurring_active')
                AND (action_type = 'send_message' OR action_type = 'resume_agent')
                AND trigger_type NOT IN ('callback');
             CREATE INDEX IF NOT EXISTS idx_tasks_callback_delivery
                ON tasks(agent_id, completed_at)
                WHERE trigger_type='callback' AND action_type='resume_agent' AND status IN ('completed','failed');
             CREATE INDEX IF NOT EXISTS idx_tasks_manual_active
                ON tasks(agent_id, created_at DESC)
                WHERE trigger_type = 'manual'
                AND status IN ('pending', 'in_progress', 'blocked');",
        )?;

        // Recreate unified_timeline VIEW (was dropped before table rebuild)
        tx.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        tx.execute("INSERT INTO schema_version (version) VALUES (8)", [])?;

        tx.commit()?;
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(())
    }

    fn migrate_v8_to_v9(&mut self) -> Result<()> {
        info!(
            "migrating database schema v8 → v9 (rewind: nullable after_value, rewound_by_trace_id)"
        );

        // Rebuild audit_events to make after_value nullable and add rewound_by_trace_id.
        // SQLite cannot ALTER a NOT NULL constraint, so we must rebuild the table.
        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Drop the unified_timeline VIEW — it references audit_events.
        tx.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;

        tx.execute_batch(
            "CREATE TABLE audit_events_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                session_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                target_key TEXT NOT NULL,
                before_value TEXT,
                after_value TEXT,
                reasoning TEXT,
                trace_id TEXT,
                rewound_by_trace_id TEXT,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );

            INSERT INTO audit_events_new (id, agent_id, session_id, tool_name, target_key,
                before_value, after_value, reasoning, trace_id, created_at)
            SELECT id, agent_id, session_id, tool_name, target_key,
                before_value, after_value, reasoning, trace_id, created_at
            FROM audit_events;

            DROP TABLE audit_events;
            ALTER TABLE audit_events_new RENAME TO audit_events;",
        )?;

        // Recreate existing indexes + new rewound index
        tx.execute_batch(
            "CREATE INDEX idx_audit_agent_created ON audit_events(agent_id, created_at);
             CREATE INDEX idx_audit_session ON audit_events(session_id);
             CREATE INDEX idx_audit_trace ON audit_events(trace_id)
                 WHERE trace_id IS NOT NULL;
             CREATE INDEX idx_audit_rewound ON audit_events(rewound_by_trace_id)
                 WHERE rewound_by_trace_id IS NOT NULL;",
        )?;

        // Recreate unified_timeline VIEW
        tx.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        tx.execute("INSERT INTO schema_version (version) VALUES (9)", [])?;

        tx.commit()?;
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(())
    }

    fn migrate_v9_to_v10(&mut self) -> Result<()> {
        info!(
            "migrating database schema v9 → v10 (team_runs.trace_id, unified_timeline + team_workspace)"
        );

        // Hoist column_exists check before creating the transaction (borrow checker constraint).
        let has_trace_id = self.column_exists("team_runs", "trace_id")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Add trace_id column to team_runs (idempotent guard for crash recovery)
        if !has_trace_id {
            tx.execute_batch("ALTER TABLE team_runs ADD COLUMN trace_id TEXT;")?;
        }

        // Recreate unified_timeline VIEW with team_workspace union
        tx.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;
        tx.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        // Add partial index on team_workspace.trace_id (matches other timeline tables)
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_team_ws_trace ON team_workspace(trace_id)
                 WHERE trace_id IS NOT NULL;",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (10)", [])?;

        tx.commit()?;
        Ok(())
    }

    fn migrate_v10_to_v11(&mut self) -> Result<()> {
        info!(
            "migrating database schema v10 → v11 (tasks.execution_trace_id, sessions.parent_session_id)"
        );

        // Hoist column_exists checks before creating the transaction (borrow checker constraint).
        let has_exec_trace = self.column_exists("tasks", "execution_trace_id")?;
        let has_parent_session = self.column_exists("sessions", "parent_session_id")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Add execution_trace_id column to tasks (idempotent guard)
        if !has_exec_trace {
            tx.execute_batch("ALTER TABLE tasks ADD COLUMN execution_trace_id TEXT;")?;
        }

        // Add parent_session_id column to sessions (idempotent guard)
        if !has_parent_session {
            tx.execute_batch("ALTER TABLE sessions ADD COLUMN parent_session_id TEXT;")?;
        }

        // Partial indexes for new columns
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_tasks_exec_trace ON tasks(execution_trace_id) WHERE execution_trace_id IS NOT NULL;",
        )?;
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id) WHERE parent_session_id IS NOT NULL;",
        )?;

        // Recreate unified_timeline VIEW with COALESCE for execution_trace_id
        tx.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;
        tx.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        tx.execute("INSERT INTO schema_version (version) VALUES (11)", [])?;

        tx.commit()?;
        Ok(())
    }

    fn migrate_v11_to_v12(&mut self) -> Result<()> {
        info!("migrating database schema v11 → v12 (INTEGER timestamps → ISO 8601 TEXT)");

        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Drop views that reference tables we're rebuilding
        tx.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;

        // --- agents ---
        tx.execute_batch(
            "CREATE TABLE agents_new (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL COLLATE NOCASE,
                home_dir TEXT NOT NULL DEFAULT '',
                active BOOLEAN NOT NULL DEFAULT 1,
                last_seen TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO agents_new SELECT id, name, home_dir, active,
                CASE WHEN last_seen IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', last_seen, 'unixepoch') ELSE NULL END,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM agents;
            DROP TABLE agents;
            ALTER TABLE agents_new RENAME TO agents;")?;

        // --- teams ---
        tx.execute_batch(
            "CREATE TABLE teams_new (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL COLLATE NOCASE,
                config_path TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO teams_new SELECT id, name, config_path,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM teams;
            DROP TABLE teams;
            ALTER TABLE teams_new RENAME TO teams;",
        )?;

        // --- team_runs ---
        tx.execute_batch(
            "CREATE TABLE team_runs_new (
                id TEXT PRIMARY KEY,
                team_id TEXT NOT NULL REFERENCES teams(id),
                goal TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'running'
                    CHECK (status IN ('running','completed','failed','cancelled','suspended')),
                failure_reason TEXT,
                iteration INTEGER NOT NULL DEFAULT 1,
                max_iterations INTEGER NOT NULL DEFAULT 3,
                deliverable TEXT,
                checkpoint TEXT,
                trace_id TEXT,
                started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                ended_at TEXT
            );
            INSERT INTO team_runs_new SELECT id, team_id, goal, status, failure_reason,
                iteration, max_iterations, deliverable, checkpoint, trace_id,
                strftime('%Y-%m-%dT%H:%M:%SZ', started_at, 'unixepoch'),
                CASE WHEN ended_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', ended_at, 'unixepoch') ELSE NULL END
            FROM team_runs;
            DROP TABLE team_runs;
            ALTER TABLE team_runs_new RENAME TO team_runs;
            CREATE INDEX idx_team_runs_team ON team_runs(team_id, started_at DESC);")?;

        // --- sessions (must be before messages due to FK) ---
        tx.execute_batch(
            "CREATE TABLE sessions_new (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                channel_type TEXT NOT NULL DEFAULT 'cli',
                started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                ended_at TEXT,
                metadata TEXT,
                parent_session_id TEXT
            );
            INSERT INTO sessions_new SELECT id, agent_id, channel_type,
                strftime('%Y-%m-%dT%H:%M:%SZ', started_at, 'unixepoch'),
                CASE WHEN ended_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', ended_at, 'unixepoch') ELSE NULL END,
                metadata, parent_session_id
            FROM sessions;
            DROP TABLE sessions;
            ALTER TABLE sessions_new RENAME TO sessions;
            CREATE INDEX idx_sessions_agent ON sessions(agent_id, started_at DESC);
            CREATE INDEX idx_sessions_parent ON sessions(parent_session_id) WHERE parent_session_id IS NOT NULL;")?;

        // --- tasks ---
        tx.execute_batch(
            "CREATE TABLE tasks_new (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                team_run_id TEXT REFERENCES team_runs(id) ON DELETE SET NULL,
                parent_task_id TEXT REFERENCES tasks_new(id) ON DELETE SET NULL,
                depth INTEGER NOT NULL DEFAULT 0 CHECK (depth BETWEEN 0 AND 3),
                label TEXT NOT NULL,
                trigger_type TEXT NOT NULL CHECK (
                    trigger_type IN ('time','recurring','callback','user_reply','event','condition','manual')
                ),
                cron_expr TEXT,
                event_source TEXT,
                event_offset_secs INTEGER,
                condition_expr TEXT,
                next_fire_at TEXT,
                timeout_at TEXT,
                action_type TEXT NOT NULL CHECK (
                    action_type IN (
                        'send_message','resume_agent','inject_context',
                        'run_skill','invoke_orchestrator','none'
                    )
                ),
                action_config TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'pending' CHECK (
                    status IN ('pending','in_progress','completed','failed',
                               'cancelled','expired','recurring_active','delivered','blocked')
                ),
                process_id INTEGER,
                input_context TEXT,
                result TEXT,
                reference_url TEXT,
                source TEXT,
                created_by_session TEXT,
                created_trace_id TEXT,
                execution_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                fired_at TEXT,
                completed_at TEXT
            );
            INSERT INTO tasks_new SELECT id, agent_id, team_run_id, parent_task_id, depth, label,
                trigger_type, cron_expr, event_source, event_offset_secs, condition_expr,
                CASE WHEN next_fire_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', next_fire_at, 'unixepoch') ELSE NULL END,
                CASE WHEN timeout_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', timeout_at, 'unixepoch') ELSE NULL END,
                action_type, action_config, status, process_id, input_context, result,
                reference_url, source, created_by_session, created_trace_id, execution_trace_id,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch'),
                strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, 'unixepoch'),
                CASE WHEN fired_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', fired_at, 'unixepoch') ELSE NULL END,
                CASE WHEN completed_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', completed_at, 'unixepoch') ELSE NULL END
            FROM tasks;
            DROP TABLE tasks;
            ALTER TABLE tasks_new RENAME TO tasks;")?;

        // Recreate task indexes
        tx.execute_batch(
            "CREATE INDEX idx_tasks_agent_status ON tasks(agent_id, status);
             CREATE INDEX idx_tasks_next_fire ON tasks(next_fire_at) WHERE status IN ('pending','recurring_active');
             CREATE INDEX idx_tasks_schedulable ON tasks(agent_id, next_fire_at ASC) WHERE status IN ('pending','recurring_active');
             CREATE INDEX idx_tasks_parent ON tasks(parent_task_id, agent_id) WHERE parent_task_id IS NOT NULL;
             CREATE INDEX idx_tasks_session ON tasks(created_by_session) WHERE created_by_session IS NOT NULL;
             CREATE INDEX idx_tasks_trace ON tasks(created_trace_id) WHERE created_trace_id IS NOT NULL;
             CREATE INDEX idx_tasks_exec_trace ON tasks(execution_trace_id) WHERE execution_trace_id IS NOT NULL;
             CREATE INDEX idx_tasks_manual_active ON tasks(agent_id, created_at DESC) WHERE trigger_type = 'manual' AND status IN ('pending', 'in_progress', 'blocked');
             CREATE UNIQUE INDEX idx_tasks_unique_recurring ON tasks(agent_id, label COLLATE NOCASE) WHERE trigger_type = 'recurring' AND status NOT IN ('cancelled', 'failed', 'expired', 'delivered');
             CREATE UNIQUE INDEX idx_tasks_unique_reminder ON tasks(agent_id, label COLLATE NOCASE) WHERE status IN ('pending', 'in_progress', 'recurring_active') AND (action_type = 'send_message' OR action_type = 'resume_agent') AND trigger_type NOT IN ('callback');
             CREATE INDEX idx_tasks_callback_delivery ON tasks(agent_id, completed_at) WHERE trigger_type='callback' AND action_type='resume_agent' AND status IN ('completed','failed');")?;

        // --- messages ---
        tx.execute_batch(
            "CREATE TABLE messages_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                role TEXT NOT NULL CHECK (role IN ('user','assistant','system','summary','tool_result')),
                content TEXT NOT NULL,
                metadata TEXT,
                trace_id TEXT,
                compacted_through_id INTEGER,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO messages_new SELECT id, session_id, agent_id, role, content, metadata,
                trace_id, compacted_through_id,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM messages;
            DROP TABLE messages;
            ALTER TABLE messages_new RENAME TO messages;
            CREATE INDEX idx_msg_session ON messages(session_id, created_at ASC);
            CREATE INDEX idx_msg_agent_created ON messages(agent_id, created_at DESC);
            CREATE INDEX idx_msg_trace ON messages(trace_id) WHERE trace_id IS NOT NULL;")?;

        // --- core_memory ---
        tx.execute_batch(
            "CREATE TABLE core_memory_new (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                key TEXT NOT NULL COLLATE NOCASE,
                value TEXT NOT NULL,
                token_count INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (agent_id, key)
            );
            INSERT INTO core_memory_new SELECT agent_id, key, value, token_count,
                strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, 'unixepoch')
            FROM core_memory;
            DROP TABLE core_memory;
            ALTER TABLE core_memory_new RENAME TO core_memory;",
        )?;

        // --- people ---
        tx.execute_batch(
            "CREATE TABLE people_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                canonical_name TEXT NOT NULL COLLATE NOCASE,
                relationship TEXT,
                notes TEXT,
                first_mentioned TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                last_mentioned TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                mention_count INTEGER NOT NULL DEFAULT 1,
                UNIQUE (agent_id, canonical_name)
            );
            INSERT INTO people_new SELECT id, agent_id, canonical_name, relationship, notes,
                strftime('%Y-%m-%dT%H:%M:%SZ', first_mentioned, 'unixepoch'),
                strftime('%Y-%m-%dT%H:%M:%SZ', last_mentioned, 'unixepoch'),
                mention_count
            FROM people;
            DROP TABLE people;
            ALTER TABLE people_new RENAME TO people;",
        )?;

        // --- commitments ---
        tx.execute_batch(
            "CREATE TABLE commitments_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                description TEXT NOT NULL COLLATE NOCASE,
                status TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending','completed','cancelled')),
                due_date TEXT,
                person_id INTEGER REFERENCES people(id) ON DELETE SET NULL,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                completed_at TEXT
            );
            INSERT INTO commitments_new SELECT id, agent_id, description, status, due_date, person_id,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch'),
                CASE WHEN completed_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', completed_at, 'unixepoch') ELSE NULL END
            FROM commitments;
            DROP TABLE commitments;
            ALTER TABLE commitments_new RENAME TO commitments;
            CREATE INDEX idx_commit_agent_status ON commitments(agent_id, status);
            CREATE UNIQUE INDEX idx_commitments_unique_pending ON commitments(agent_id, description COLLATE NOCASE, due_date) WHERE status = 'pending';")?;

        // --- preferences ---
        tx.execute_batch(
            "CREATE TABLE preferences_new (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                category TEXT NOT NULL COLLATE NOCASE,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (agent_id, category)
            );
            INSERT INTO preferences_new SELECT agent_id, category, value,
                strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, 'unixepoch')
            FROM preferences;
            DROP TABLE preferences;
            ALTER TABLE preferences_new RENAME TO preferences;",
        )?;

        // --- events ---
        tx.execute_batch(
            "CREATE TABLE events_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                description TEXT NOT NULL,
                event_date TEXT,
                context TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO events_new SELECT id, agent_id, description, event_date, context,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM events;
            DROP TABLE events;
            ALTER TABLE events_new RENAME TO events;
            CREATE UNIQUE INDEX idx_events_unique_description ON events(agent_id, description COLLATE NOCASE, event_date) WHERE event_date IS NOT NULL;")?;

        // --- audit_events ---
        tx.execute_batch(
            "CREATE TABLE audit_events_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                session_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                target_key TEXT NOT NULL,
                before_value TEXT,
                after_value TEXT,
                reasoning TEXT,
                trace_id TEXT,
                rewound_by_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO audit_events_new SELECT id, agent_id, session_id, tool_name,
                target_key, before_value, after_value, reasoning, trace_id, rewound_by_trace_id,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM audit_events;
            DROP TABLE audit_events;
            ALTER TABLE audit_events_new RENAME TO audit_events;
            CREATE INDEX idx_audit_agent_created ON audit_events(agent_id, created_at DESC);
            CREATE INDEX idx_audit_session ON audit_events(session_id);
            CREATE INDEX idx_audit_trace ON audit_events(trace_id) WHERE trace_id IS NOT NULL;
            CREATE INDEX idx_audit_rewound ON audit_events(rewound_by_trace_id) WHERE rewound_by_trace_id IS NOT NULL;")?;

        // --- audit_event_summaries ---
        tx.execute_batch(
            "CREATE TABLE audit_event_summaries_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                year INTEGER NOT NULL,
                month INTEGER NOT NULL,
                summary TEXT NOT NULL,
                event_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                UNIQUE (agent_id, year, month)
            );
            INSERT INTO audit_event_summaries_new SELECT id, agent_id, year, month,
                summary, event_count,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM audit_event_summaries;
            DROP TABLE audit_event_summaries;
            ALTER TABLE audit_event_summaries_new RENAME TO audit_event_summaries;",
        )?;

        // --- search_content ---
        tx.execute_batch(
            "CREATE TABLE search_content_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                source_type TEXT NOT NULL,
                source_id INTEGER,
                content TEXT NOT NULL,
                embedding_json TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO search_content_new SELECT id, agent_id, source_type, source_id, content,
                embedding_json,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch'),
                strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, 'unixepoch')
            FROM search_content;
            DROP TABLE search_content;
            ALTER TABLE search_content_new RENAME TO search_content;
            CREATE INDEX idx_search_agent ON search_content(agent_id, source_type);",
        )?;

        // --- team_workspace ---
        tx.execute_batch(
            "CREATE TABLE team_workspace_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id TEXT NOT NULL REFERENCES team_runs(id) ON DELETE CASCADE,
                parent_id INTEGER REFERENCES team_workspace_new(id),
                agent_name TEXT,
                entry_type TEXT NOT NULL,
                content TEXT NOT NULL,
                trace_id TEXT,
                iteration INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO team_workspace_new SELECT id, run_id, parent_id, agent_name,
                entry_type, content, trace_id, iteration,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM team_workspace;
            DROP TABLE team_workspace;
            ALTER TABLE team_workspace_new RENAME TO team_workspace;
            CREATE INDEX idx_team_ws_run ON team_workspace(run_id, created_at);
            CREATE INDEX idx_team_ws_trace ON team_workspace(trace_id) WHERE trace_id IS NOT NULL;",
        )?;

        // --- heartbeat_sends ---
        tx.execute_batch(
            "CREATE TABLE heartbeat_sends_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                sent_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO heartbeat_sends_new SELECT id, agent_id,
                strftime('%Y-%m-%dT%H:%M:%SZ', sent_at, 'unixepoch')
            FROM heartbeat_sends;
            DROP TABLE heartbeat_sends;
            ALTER TABLE heartbeat_sends_new RENAME TO heartbeat_sends;
            CREATE INDEX idx_heartbeat_agent ON heartbeat_sends(agent_id, sent_at DESC);",
        )?;

        // --- reflection_runs ---
        tx.execute_batch(
            "CREATE TABLE reflection_runs_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                status TEXT NOT NULL,
                changes_made INTEGER NOT NULL DEFAULT 0,
                summary TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO reflection_runs_new SELECT id, agent_id, status, changes_made, summary,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM reflection_runs;
            DROP TABLE reflection_runs;
            ALTER TABLE reflection_runs_new RENAME TO reflection_runs;
            CREATE INDEX idx_reflect_agent ON reflection_runs(agent_id, created_at DESC);",
        )?;

        // --- customer_config ---
        tx.execute_batch(
            "CREATE TABLE customer_config_new (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                key TEXT NOT NULL COLLATE NOCASE,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (agent_id, key)
            );
            INSERT INTO customer_config_new SELECT agent_id, key, value,
                strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, 'unixepoch')
            FROM customer_config;
            DROP TABLE customer_config;
            ALTER TABLE customer_config_new RENAME TO customer_config;",
        )?;

        // --- failed_sends ---
        tx.execute_batch(
            "CREATE TABLE failed_sends_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                text TEXT NOT NULL,
                request_id TEXT,
                retry_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO failed_sends_new SELECT id, agent_id, text, request_id, retry_count,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM failed_sends;
            DROP TABLE failed_sends;
            ALTER TABLE failed_sends_new RENAME TO failed_sends;",
        )?;

        // --- schema_version ---
        tx.execute_batch(
            "CREATE TABLE schema_version_new (
                version INTEGER NOT NULL,
                applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO schema_version_new SELECT version,
                strftime('%Y-%m-%dT%H:%M:%SZ', applied_at, 'unixepoch')
            FROM schema_version;
            DROP TABLE schema_version;
            ALTER TABLE schema_version_new RENAME TO schema_version;",
        )?;

        // Recreate unified_timeline VIEW
        tx.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        // Record migration
        tx.execute("INSERT INTO schema_version (version) VALUES (12)", [])?;

        tx.commit()?;
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(())
    }

    fn migrate_v12_to_v13(&mut self) -> Result<()> {
        info!("migrating database schema v12 → v13 (A2A orthogonal persistence)");

        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Drop view first — it references the tasks table we're about to rebuild
        tx.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;

        // Rebuild tasks table to add 'a2a' to trigger_type CHECK constraint
        tx.execute_batch(
            "CREATE TABLE tasks_new (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                team_run_id TEXT REFERENCES team_runs(id) ON DELETE SET NULL,
                parent_task_id TEXT REFERENCES tasks_new(id) ON DELETE SET NULL,
                depth INTEGER NOT NULL DEFAULT 0 CHECK (depth BETWEEN 0 AND 3),
                label TEXT NOT NULL,
                trigger_type TEXT NOT NULL CHECK (
                    trigger_type IN ('time','recurring','callback','user_reply','event','condition','manual','a2a')
                ),
                cron_expr TEXT,
                event_source TEXT,
                event_offset_secs INTEGER,
                condition_expr TEXT,
                next_fire_at TEXT,
                timeout_at TEXT,
                action_type TEXT NOT NULL CHECK (
                    action_type IN (
                        'send_message','resume_agent','inject_context',
                        'run_skill','invoke_orchestrator','none'
                    )
                ),
                action_config TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'pending' CHECK (
                    status IN ('pending','in_progress','completed','failed',
                               'cancelled','expired','recurring_active','delivered','blocked')
                ),
                process_id INTEGER,
                input_context TEXT,
                result TEXT,
                reference_url TEXT,
                source TEXT,
                created_by_session TEXT,
                created_trace_id TEXT,
                execution_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                fired_at TEXT,
                completed_at TEXT
            );
            INSERT INTO tasks_new SELECT * FROM tasks;
            DROP TABLE tasks;
            ALTER TABLE tasks_new RENAME TO tasks;
            CREATE INDEX idx_tasks_agent_status ON tasks(agent_id, status);
            CREATE INDEX idx_tasks_next_fire ON tasks(next_fire_at)
                WHERE status IN ('pending','recurring_active');
            CREATE INDEX idx_tasks_schedulable
                ON tasks(agent_id, next_fire_at ASC)
                WHERE status IN ('pending','recurring_active');
            CREATE INDEX idx_tasks_parent ON tasks(parent_task_id, agent_id) WHERE parent_task_id IS NOT NULL;
            CREATE INDEX idx_tasks_session ON tasks(created_by_session) WHERE created_by_session IS NOT NULL;
            CREATE INDEX idx_tasks_trace ON tasks(created_trace_id) WHERE created_trace_id IS NOT NULL;
            CREATE INDEX idx_tasks_exec_trace ON tasks(execution_trace_id) WHERE execution_trace_id IS NOT NULL;
            CREATE INDEX idx_tasks_manual_active
                ON tasks(agent_id, created_at DESC)
                WHERE trigger_type = 'manual'
                AND status IN ('pending', 'in_progress', 'blocked');
            CREATE UNIQUE INDEX idx_tasks_unique_recurring
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE trigger_type = 'recurring'
                AND status NOT IN ('cancelled', 'failed', 'expired', 'delivered');
            CREATE UNIQUE INDEX idx_tasks_unique_reminder
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE status IN ('pending', 'in_progress', 'recurring_active')
                AND (action_type = 'send_message' OR action_type = 'resume_agent')
                AND trigger_type NOT IN ('callback');")?;

        // Create thin mapping table
        tx.execute_batch(
            "CREATE TABLE a2a_task_map (
                a2a_task_id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                context_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_a2a_task_map_task ON a2a_task_map(task_id);",
        )?;

        // Create A2A tables with FK to mapping table
        tx.execute_batch(
            "CREATE TABLE a2a_artifacts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL REFERENCES a2a_task_map(a2a_task_id) ON DELETE CASCADE,
                artifact_id TEXT NOT NULL,
                name TEXT,
                description TEXT,
                parts TEXT NOT NULL,
                metadata TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE TABLE a2a_push_notification_configs (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES a2a_task_map(a2a_task_id) ON DELETE CASCADE,
                url TEXT NOT NULL,
                token TEXT,
                auth_scheme TEXT,
                auth_credentials TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );",
        )?;

        // Recreate unified_timeline VIEW (tasks table was rebuilt)
        tx.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;
        tx.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        tx.execute("INSERT INTO schema_version (version) VALUES (13)", [])?;

        tx.commit()?;
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(())
    }

    fn migrate_v13_to_v14(&self) -> Result<()> {
        info!("migrating database schema v13 → v14 (task metadata column)");

        // Simple ALTER TABLE — SQLite supports adding nullable columns without table rebuild.
        // Idempotent: check if column already exists before adding.
        if !self.column_exists("tasks", "metadata")? {
            self.conn
                .execute_batch("ALTER TABLE tasks ADD COLUMN metadata TEXT;")?;
        }

        self.conn
            .execute("INSERT INTO schema_version (version) VALUES (14)", [])?;

        Ok(())
    }

    fn migrate_v14_to_v15(&self) -> Result<()> {
        info!("migrating database schema v14 → v15 (llm_calls + tool_calls tables)");

        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS llm_calls (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                trace_id TEXT,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER,
                cache_write_tokens INTEGER,
                latency_ms INTEGER NOT NULL DEFAULT 0,
                stop_reason TEXT,
                status TEXT NOT NULL DEFAULT 'success',
                error_message TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX IF NOT EXISTS idx_llm_calls_trace ON llm_calls(trace_id);
            CREATE INDEX IF NOT EXISTS idx_llm_calls_session ON llm_calls(session_id);
            CREATE INDEX IF NOT EXISTS idx_llm_calls_agent_created ON llm_calls(agent_id, created_at);

            CREATE TABLE IF NOT EXISTS tool_calls (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                trace_id TEXT,
                llm_call_id TEXT,
                step INTEGER NOT NULL DEFAULT 0,
                tool_name TEXT NOT NULL,
                tool_source TEXT NOT NULL DEFAULT 'builtin',
                skill_name TEXT,
                input TEXT,
                output TEXT,
                success INTEGER NOT NULL DEFAULT 1,
                non_zero_exit INTEGER NOT NULL DEFAULT 0,
                latency_ms INTEGER NOT NULL DEFAULT 0,
                error_message TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX IF NOT EXISTS idx_tool_calls_trace ON tool_calls(trace_id);
            CREATE INDEX IF NOT EXISTS idx_tool_calls_session ON tool_calls(session_id);
            CREATE INDEX IF NOT EXISTS idx_tool_calls_llm_call ON tool_calls(llm_call_id);
            CREATE INDEX IF NOT EXISTS idx_tool_calls_agent_created ON tool_calls(agent_id, created_at);",
        )?;

        // Recreate unified_timeline VIEW with new UNION ALL legs
        self.conn
            .execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;
        self.conn.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        self.conn
            .execute("INSERT INTO schema_version (version) VALUES (15)", [])?;

        Ok(())
    }

    fn migrate_v15_to_v16(&self) -> Result<()> {
        info!("migrating database schema v15 → v16 (add step column to llm_calls)");

        if !self.column_exists("llm_calls", "step")? {
            self.conn.execute_batch(
                "ALTER TABLE llm_calls ADD COLUMN step INTEGER NOT NULL DEFAULT 0;",
            )?;
        }

        self.conn
            .execute("INSERT INTO schema_version (version) VALUES (16)", [])?;

        Ok(())
    }

    fn migrate_v16_to_v17(&mut self) -> Result<()> {
        info!("migrating database schema v16 → v17 (add task dedup index on reference_url)");

        // Wrap in transaction for atomicity (matches pattern in migrate_v7_to_v8, etc.)
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "-- Step 1: Cancel duplicate active tasks with the same (agent_id, reference_url).
             -- Keep the earliest-created item per group (by rowid for deterministic tiebreaking
             -- when created_at timestamps collide), cancel the rest with a metadata breadcrumb.
             UPDATE tasks SET status = 'cancelled',
                 metadata = json_set(COALESCE(metadata, '{}'), '$.cancelled_reason', 'dedup_migration_v17')
             WHERE rowid IN (
                 SELECT t.rowid FROM tasks t
                 INNER JOIN (
                     SELECT agent_id, reference_url, MIN(rowid) as keeper_rowid
                     FROM tasks
                     WHERE trigger_type = 'manual'
                       AND reference_url IS NOT NULL
                       AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')
                     GROUP BY agent_id, reference_url
                     HAVING COUNT(*) > 1
                 ) dups ON t.agent_id = dups.agent_id
                        AND t.reference_url = dups.reference_url
                        AND t.rowid != dups.keeper_rowid
                 WHERE t.trigger_type = 'manual'
                   AND t.status NOT IN ('completed', 'cancelled', 'failed', 'delivered')
             );

             -- Step 2: Create partial unique index. NULLs are exempt (SQLite skips NULL in
             -- unique indexes), so label-only dedup is handled at the tool level.
             CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_manual_active_ref_url
             ON tasks(agent_id, reference_url)
             WHERE trigger_type = 'manual'
               AND reference_url IS NOT NULL
               AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered');

             INSERT INTO schema_version (version) VALUES (17);",
        )?;
        tx.commit()?;

        Ok(())
    }

    fn migrate_v17_to_v18(&mut self) -> Result<()> {
        info!("migrating database schema v17 → v18 (widen reminder dedup index for resume_agent)");

        // The old index only covered action_type = 'send_message'. The new index covers
        // both 'send_message' and 'resume_agent' to prevent duplicate reminders regardless
        // of action type. See #363.
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "DROP INDEX IF EXISTS idx_tasks_unique_reminder;
             CREATE UNIQUE INDEX idx_tasks_unique_reminder
             ON tasks(agent_id, label COLLATE NOCASE)
             WHERE status IN ('pending', 'in_progress', 'recurring_active')
               AND (action_type = 'send_message' OR action_type = 'resume_agent')
               AND trigger_type NOT IN ('callback');

             INSERT INTO schema_version (version) VALUES (18);",
        )?;
        tx.commit()?;

        Ok(())
    }

    fn migrate_v18_to_v19(&mut self) -> Result<()> {
        info!("migrating database schema v18 → v19 (add task_id to sessions for reverse lookup)");

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "ALTER TABLE sessions ADD COLUMN task_id TEXT;

             CREATE INDEX idx_sessions_task_id ON sessions(task_id) WHERE task_id IS NOT NULL;

             -- Backfill from existing metadata JSON (cli --task-id sessions)
             UPDATE sessions SET task_id = json_extract(metadata, '$.task_id')
               WHERE json_extract(metadata, '$.task_id') IS NOT NULL AND task_id IS NULL;

             INSERT INTO schema_version (version) VALUES (19);",
        )?;
        tx.commit()?;

        Ok(())
    }

    fn migrate_v19_to_v20(&mut self) -> Result<()> {
        info!("migrating database schema v19 → v20 (skill_overrides: llm_provider, llm_model)");

        // Idempotent: skip ALTER if columns already exist (defensive — re-runs).
        let has_provider = self.column_exists("skill_overrides", "llm_provider")?;
        let has_model = self.column_exists("skill_overrides", "llm_model")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut sql = String::new();
        if !has_provider {
            sql.push_str("ALTER TABLE skill_overrides ADD COLUMN llm_provider TEXT;\n");
        }
        if !has_model {
            sql.push_str("ALTER TABLE skill_overrides ADD COLUMN llm_model TEXT;\n");
        }
        sql.push_str("INSERT INTO schema_version (version) VALUES (20);");

        tx.execute_batch(&sql)?;
        tx.commit()?;
        Ok(())
    }

    fn migrate_v20_to_v21(&mut self) -> Result<()> {
        info!("migrating database schema v20 → v21 (llm_calls: prompt_variant)");

        let has_col = self.column_exists("llm_calls", "prompt_variant")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut sql = String::new();
        if !has_col {
            sql.push_str("ALTER TABLE llm_calls ADD COLUMN prompt_variant TEXT;\n");
        }
        sql.push_str("INSERT INTO schema_version (version) VALUES (21);");

        tx.execute_batch(&sql)?;
        tx.commit()?;
        Ok(())
    }

    fn migrate_v21_to_v22(&mut self) -> Result<()> {
        info!("migrating database schema v21 → v22 (messages: internal flag)");

        let has_col = self.column_exists("messages", "internal")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut sql = String::new();
        if !has_col {
            sql.push_str("ALTER TABLE messages ADD COLUMN internal INTEGER NOT NULL DEFAULT 0;\n");
        }
        sql.push_str("INSERT INTO schema_version (version) VALUES (22);");

        tx.execute_batch(&sql)?;
        tx.commit()?;
        Ok(())
    }

    fn migrate_v22_to_v23(&mut self) -> Result<()> {
        info!("migrating database schema v22 → v23 (tasks: type column)");

        let has_col = self.column_exists("tasks", "type")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut sql = String::new();
        if !has_col {
            // SQLite 3.37+ supports CHECK constraints in ALTER TABLE ADD COLUMN.
            // The DEFAULT backfills all existing rows to 'issue', preserving behavior.
            sql.push_str(
                "ALTER TABLE tasks ADD COLUMN type TEXT NOT NULL DEFAULT 'issue' \
                 CHECK (type IN ('issue', 'milestone', 'project'));\n",
            );
        }
        sql.push_str("INSERT INTO schema_version (version) VALUES (23);");

        tx.execute_batch(&sql)?;
        tx.commit()?;
        Ok(())
    }

    fn migrate_v23_to_v24(&mut self) -> Result<()> {
        info!("migrating database schema v23 → v24 (skill_overrides: enabled column)");

        let has_col = self.column_exists("skill_overrides", "enabled")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut sql = String::new();
        if !has_col {
            sql.push_str("ALTER TABLE skill_overrides ADD COLUMN enabled INTEGER;\n");
        }
        sql.push_str("INSERT INTO schema_version (version) VALUES (24);");

        tx.execute_batch(&sql)?;
        tx.commit()?;
        Ok(())
    }

    /// Migration v24 -> v25: Knowledge Graph schema tables.
    ///
    /// Adds 10 tables for the three-layer KG:
    /// - Domain layer: `kg_entities`, `kg_relationships`
    /// - Lexical layer: `kg_chunks`
    /// - Subject layer: `kg_subject_entities`, `kg_subject_resolutions`,
    ///   `kg_subject_relationships`
    /// - Provenance: `kg_chunk_subjects`, `kg_chunk_subject_relationships`
    /// - Tracking: `kg_extractions`, `kg_resolutions_log`
    fn migrate_v24_to_v25(&mut self) -> Result<()> {
        info!("migrating database schema v24 -> v25 (knowledge graph tables)");

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
                "-- KG domain layer (global, no agent_id)
                CREATE TABLE IF NOT EXISTS kg_entities (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    entity_key TEXT NOT NULL UNIQUE,
                    type TEXT NOT NULL,
                    name TEXT NOT NULL,
                    properties_json TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    CHECK (entity_key = type || ':' || name)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_entities_type ON kg_entities(type);

                CREATE TABLE IF NOT EXISTS kg_relationships (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    from_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
                    to_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
                    type TEXT NOT NULL,
                    properties_json TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
                );
                CREATE INDEX IF NOT EXISTS idx_kg_rel_from ON kg_relationships(from_entity_id, type);
                CREATE INDEX IF NOT EXISTS idx_kg_rel_to ON kg_relationships(to_entity_id, type);

                -- KG lexical layer (per-agent)
                CREATE TABLE IF NOT EXISTS kg_chunks (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    seq_id INTEGER NOT NULL,
                    source_doc_path TEXT NOT NULL,
                    source_doc_hash TEXT NOT NULL,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    trace_id TEXT,
                    UNIQUE (agent_id, source_doc_path, seq_id)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_chunks_agent_doc ON kg_chunks(agent_id, source_doc_path);

                -- KG subject layer (per-agent)
                CREATE TABLE IF NOT EXISTS kg_subject_entities (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    entity_key TEXT NOT NULL,
                    type TEXT NOT NULL,
                    name TEXT NOT NULL,
                    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                    properties_json TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    trace_id TEXT,
                    CHECK (entity_key = type || ':' || name),
                    UNIQUE (agent_id, entity_key)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_subj_entities_agent_type ON kg_subject_entities(agent_id, type);

                CREATE TABLE IF NOT EXISTS kg_subject_resolutions (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                    domain_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
                    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    trace_id TEXT,
                    UNIQUE (agent_id, subject_entity_id, domain_entity_id)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_resolutions_agent_subj ON kg_subject_resolutions(agent_id, subject_entity_id);
                CREATE INDEX IF NOT EXISTS idx_kg_resolutions_agent_dom ON kg_subject_resolutions(agent_id, domain_entity_id);

                -- KG subject-to-subject edges / fact triples (per-agent)
                CREATE TABLE IF NOT EXISTS kg_subject_relationships (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    from_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                    to_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                    type TEXT NOT NULL,
                    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                    properties_json TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    trace_id TEXT,
                    UNIQUE (agent_id, from_entity_id, to_entity_id, type)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_subj_rel_from ON kg_subject_relationships(agent_id, from_entity_id, type);
                CREATE INDEX IF NOT EXISTS idx_kg_subj_rel_to ON kg_subject_relationships(agent_id, to_entity_id, type);
                CREATE INDEX IF NOT EXISTS idx_kg_subj_rel_type ON kg_subject_relationships(agent_id, type);

                -- KG entity provenance: chunk -> subject entity (many-to-many)
                CREATE TABLE IF NOT EXISTS kg_chunk_subjects (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
                    subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                    extraction_trace_id TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    UNIQUE (agent_id, chunk_id, subject_entity_id)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_cs_chunk ON kg_chunk_subjects(agent_id, chunk_id);
                CREATE INDEX IF NOT EXISTS idx_kg_cs_entity ON kg_chunk_subjects(agent_id, subject_entity_id);
                CREATE INDEX IF NOT EXISTS idx_kg_cs_trace ON kg_chunk_subjects(agent_id, extraction_trace_id);

                -- KG relationship provenance: chunk -> subject relationship (many-to-many)
                CREATE TABLE IF NOT EXISTS kg_chunk_subject_relationships (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
                    subject_relationship_id INTEGER NOT NULL REFERENCES kg_subject_relationships(id) ON DELETE CASCADE,
                    extraction_trace_id TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    UNIQUE (agent_id, chunk_id, subject_relationship_id)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_csr_chunk ON kg_chunk_subject_relationships(agent_id, chunk_id);
                CREATE INDEX IF NOT EXISTS idx_kg_csr_rel ON kg_chunk_subject_relationships(agent_id, subject_relationship_id);

                -- KG extraction tracking.
                -- Historical shape as shipped at v25. `source_doc_hash` is
                -- added at v26 via ALTER TABLE in migrate_v25_to_v26 (#757);
                -- keeping migrate_v24_to_v25 as the record of v25's actual
                -- schema preserves migration immutability and means the
                -- convergence test exercises the ALTER path rather than
                -- skipping it.
                CREATE TABLE IF NOT EXISTS kg_extractions (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    source_doc_path TEXT NOT NULL,
                    extraction_model TEXT NOT NULL,
                    entities_extracted INTEGER NOT NULL DEFAULT 0,
                    relationships_extracted INTEGER NOT NULL DEFAULT 0,
                    extraction_trace_id TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    UNIQUE (agent_id, source_doc_path)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_extractions_agent ON kg_extractions(agent_id);

                -- KG resolution tracking
                CREATE TABLE IF NOT EXISTS kg_resolutions_log (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                    outcome TEXT NOT NULL CHECK (outcome IN (
                        'matched_exact', 'matched_llm', 'no_match', 'skipped_discovered_type', 'skipped_no_llm', 'error'
                    )),
                    resolution_trace_id TEXT NOT NULL,
                    source_extraction_trace_id TEXT,
                    model TEXT,
                    duration_ms INTEGER,
                    resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    UNIQUE (agent_id, subject_entity_id)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);

                INSERT INTO schema_version (version) VALUES (25);",
            )
            .context("failed to migrate v24 -> v25 (knowledge graph tables)")?;
        tx.commit()?;

        Ok(())
    }

    /// v25 -> v26: add nullable `source_doc_hash` column to `kg_extractions`
    /// so the pending-doc query can skip re-extraction when chunk content is
    /// unchanged (#757). Pre-existing rows get NULL and will re-extract once
    /// on the next run (bounded by MIKA_KG_BATCH_BUDGET), then populate the
    /// hash on success so subsequent runs are no-ops.
    ///
    /// Idempotent at the migration-chain level (the version gate in
    /// `run_migrations` prevents re-run); `column_exists` guards the inner
    /// ALTER against manual invocation.
    fn migrate_v25_to_v26(&mut self) -> Result<()> {
        info!("migrating database schema v25 -> v26 (kg_extractions.source_doc_hash)");

        if !self.column_exists("kg_extractions", "source_doc_hash")? {
            let tx = self
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(
                "ALTER TABLE kg_extractions ADD COLUMN source_doc_hash TEXT;
                 INSERT INTO schema_version (version) VALUES (26);",
            )
            .context("failed to migrate v25 -> v26 (add kg_extractions.source_doc_hash)")?;
            tx.commit()?;
        } else {
            // Column already exists (manual re-run in a test / recovery scenario).
            // Wrap in TransactionBehavior::Immediate to match the true-branch envelope
            // (mika#1391): bare INSERT could leave column-exists + schema_version-not-bumped
            // inconsistent state on failure mid-recovery.
            let tx = self
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute("INSERT INTO schema_version (version) VALUES (26)", [])?;
            tx.commit()?;
        }

        Ok(())
    }

    /// v26 -> v27: Schema v27 — docs_root_hash as shared-corpus primary key (#786 + #787).
    ///
    /// Two-phase migration in a single transaction:
    /// 1. **DDL** (#786): Renames six shared-layer KG tables to `*_v26_backup`,
    ///    creates fresh v27 tables keyed by `docs_root_hash` instead of `agent_id`.
    ///    Rebuilds per-agent tables to fix FK refs.
    /// 2. **Coalesce** (#787): Reads from backup tables, deduplicates across agents
    ///    via majority-vote (normalized entity_key, agent-count tiebreak), rewires
    ///    FKs via temp lookup tables, drops backups, writes `v27_coalesce_complete`
    ///    marker to `schema_meta`. `docs_root` resolved from `MIKA_KG_DOCS_ROOT`
    ///    env var or CWD fallback.
    fn migrate_v26_to_v27(&mut self) -> Result<()> {
        info!("migrating database schema v26 -> v27 (docs_root_hash shared-corpus)");

        // Idempotency guard: if kg_chunks already has docs_root_hash, we've run.
        if self.column_exists("kg_chunks", "docs_root_hash")? {
            // Already upgraded — just record the version bump.
            self.conn
                .execute("INSERT INTO schema_version (version) VALUES (27)", [])?;
            return Ok(());
        }

        // Resolve docs_root for v26 data coalescing.
        let docs_root_path = std::env::var("MIKA_KG_DOCS_ROOT")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .join("docs")
                    .join("solutions")
            });
        let docs_root_escaped = docs_root_path.to_string_lossy().replace('\'', "''");
        let docs_root_hash = crate::kg::config::hash_docs_root(&docs_root_path);

        info!(
            docs_root = %docs_root_path.display(),
            docs_root_hash = %docs_root_hash,
            "v26->v27 coalesce: resolved docs_root for migration"
        );

        // Log pre-coalesce counts from v26 tables (before DDL renames them).
        let pre_chunks: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_chunks", [], |r| r.get(0))
            .unwrap_or(0);
        let pre_entities: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_subject_entities", [], |r| r.get(0))
            .unwrap_or(0);
        let pre_relationships: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_subject_relationships", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        let pre_extractions: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_extractions", [], |r| r.get(0))
            .unwrap_or(0);
        let pre_resolutions: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_subject_resolutions", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        let pre_res_log: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        info!(
            pre_chunks,
            pre_entities,
            pre_relationships,
            pre_extractions,
            pre_resolutions,
            pre_res_log,
            "v26->v27 coalesce: pre-migration row counts"
        );

        // Generate the coalesce SQL for the resolved docs_root.
        let coalesce = v27_coalesce_sql(&docs_root_escaped, &docs_root_hash);

        // Build the full migration: DDL (rename to backup + create v27 tables) +
        // coalesce (read from backups, dedup, write to v27, drop backups) + finalize.
        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let sql = format!(
            "-- Create schema_meta table for migration state tracking.
                 CREATE TABLE IF NOT EXISTS schema_meta (
                     key TEXT PRIMARY KEY,
                     value TEXT NOT NULL
                 );

                 -- Drop v25/v26 indexes that will be recreated (SQLite keeps
                 -- index names across ALTER TABLE RENAME, so they'd conflict).
                 DROP INDEX IF EXISTS idx_kg_chunks_agent_doc;
                 DROP INDEX IF EXISTS idx_kg_subj_entities_agent_type;
                 DROP INDEX IF EXISTS idx_kg_subj_rel_from;
                 DROP INDEX IF EXISTS idx_kg_subj_rel_to;
                 DROP INDEX IF EXISTS idx_kg_subj_rel_type;
                 DROP INDEX IF EXISTS idx_kg_cs_chunk;
                 DROP INDEX IF EXISTS idx_kg_cs_entity;
                 DROP INDEX IF EXISTS idx_kg_cs_trace;
                 DROP INDEX IF EXISTS idx_kg_csr_chunk;
                 DROP INDEX IF EXISTS idx_kg_csr_rel;
                 DROP INDEX IF EXISTS idx_kg_extractions_agent;
                 DROP INDEX IF EXISTS idx_kg_resolutions_agent_subj;
                 DROP INDEX IF EXISTS idx_kg_resolutions_agent_dom;
                 DROP INDEX IF EXISTS idx_kg_res_log_pending;

                 -- 1. kg_chunks: rename to backup, create v27 table.
                 ALTER TABLE kg_chunks RENAME TO kg_chunks_v26_backup;
                 CREATE TABLE kg_chunks (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     docs_root_hash TEXT NOT NULL,
                     docs_root TEXT NOT NULL,
                     seq_id INTEGER NOT NULL,
                     source_doc_path TEXT NOT NULL,
                     source_doc_hash TEXT NOT NULL,
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     trace_id TEXT,
                     UNIQUE (docs_root_hash, source_doc_path, seq_id)
                 );
                 CREATE INDEX idx_kg_chunks_docs_root_hash_doc ON kg_chunks(docs_root_hash, source_doc_path);

                 -- 2. kg_subject_entities: rename to backup, create v27 table.
                 ALTER TABLE kg_subject_entities RENAME TO kg_subject_entities_v26_backup;
                 CREATE TABLE kg_subject_entities (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     docs_root_hash TEXT NOT NULL,
                     docs_root TEXT NOT NULL,
                     entity_key TEXT NOT NULL,
                     type TEXT NOT NULL,
                     name TEXT NOT NULL,
                     confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                     properties_json TEXT,
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     trace_id TEXT,
                     CHECK (entity_key = type || ':' || name),
                     UNIQUE (docs_root_hash, entity_key)
                 );
                 CREATE INDEX idx_kg_subj_entities_drh_type ON kg_subject_entities(docs_root_hash, type);

                 -- 3. kg_subject_relationships: rename to backup, create v27 table.
                 ALTER TABLE kg_subject_relationships RENAME TO kg_subject_relationships_v26_backup;
                 CREATE TABLE kg_subject_relationships (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     docs_root_hash TEXT NOT NULL,
                     docs_root TEXT NOT NULL,
                     from_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                     to_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                     type TEXT NOT NULL,
                     confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                     properties_json TEXT,
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     trace_id TEXT,
                     UNIQUE (docs_root_hash, from_entity_id, to_entity_id, type)
                 );
                 CREATE INDEX idx_kg_subj_rel_from ON kg_subject_relationships(docs_root_hash, from_entity_id, type);
                 CREATE INDEX idx_kg_subj_rel_to ON kg_subject_relationships(docs_root_hash, to_entity_id, type);
                 CREATE INDEX idx_kg_subj_rel_type ON kg_subject_relationships(docs_root_hash, type);

                 -- 4. kg_chunk_subjects: rename to backup, create v27 table.
                 ALTER TABLE kg_chunk_subjects RENAME TO kg_chunk_subjects_v26_backup;
                 CREATE TABLE kg_chunk_subjects (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     docs_root_hash TEXT NOT NULL,
                     docs_root TEXT NOT NULL,
                     chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
                     subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                     extraction_trace_id TEXT,
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     UNIQUE (docs_root_hash, chunk_id, subject_entity_id)
                 );
                 CREATE INDEX idx_kg_cs_chunk ON kg_chunk_subjects(docs_root_hash, chunk_id);
                 CREATE INDEX idx_kg_cs_entity ON kg_chunk_subjects(docs_root_hash, subject_entity_id);
                 CREATE INDEX idx_kg_cs_trace ON kg_chunk_subjects(docs_root_hash, extraction_trace_id);

                 -- 5. kg_chunk_subject_relationships: rename to backup, create v27 table.
                 ALTER TABLE kg_chunk_subject_relationships RENAME TO kg_chunk_subject_relationships_v26_backup;
                 CREATE TABLE kg_chunk_subject_relationships (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     docs_root_hash TEXT NOT NULL,
                     docs_root TEXT NOT NULL,
                     chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
                     subject_relationship_id INTEGER NOT NULL REFERENCES kg_subject_relationships(id) ON DELETE CASCADE,
                     extraction_trace_id TEXT,
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     UNIQUE (docs_root_hash, chunk_id, subject_relationship_id)
                 );
                 CREATE INDEX idx_kg_csr_chunk ON kg_chunk_subject_relationships(docs_root_hash, chunk_id);
                 CREATE INDEX idx_kg_csr_rel ON kg_chunk_subject_relationships(docs_root_hash, subject_relationship_id);

                 -- 6. kg_extractions: rename to backup, create v27 table.
                 ALTER TABLE kg_extractions RENAME TO kg_extractions_v26_backup;
                 CREATE TABLE kg_extractions (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     docs_root_hash TEXT NOT NULL,
                     docs_root TEXT NOT NULL,
                     source_doc_path TEXT NOT NULL,
                     source_doc_hash TEXT,
                     extraction_model TEXT NOT NULL,
                     entities_extracted INTEGER NOT NULL DEFAULT 0,
                     relationships_extracted INTEGER NOT NULL DEFAULT 0,
                     extraction_trace_id TEXT,
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     UNIQUE (docs_root_hash, source_doc_path)
                 );
                 CREATE INDEX idx_kg_extractions_drh ON kg_extractions(docs_root_hash);

                 -- 7. kg_subject_resolutions: rebuild to fix FK refs broken by
                 -- kg_subject_entities rename (SQLite rewrites FK targets on
                 -- ALTER TABLE RENAME).
                 ALTER TABLE kg_subject_resolutions RENAME TO kg_subject_resolutions_v26_backup;
                 CREATE TABLE kg_subject_resolutions (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                     subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                     domain_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
                     confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     trace_id TEXT,
                     UNIQUE (agent_id, subject_entity_id, domain_entity_id)
                 );
                 CREATE INDEX idx_kg_resolutions_agent_subj ON kg_subject_resolutions(agent_id, subject_entity_id);
                 CREATE INDEX idx_kg_resolutions_agent_dom ON kg_subject_resolutions(agent_id, domain_entity_id);

                 -- 8. kg_resolutions_log: rebuild to fix FK refs broken by
                 -- kg_subject_entities rename.
                 ALTER TABLE kg_resolutions_log RENAME TO kg_resolutions_log_v26_backup;
                 CREATE TABLE kg_resolutions_log (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                     subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                     outcome TEXT NOT NULL CHECK (outcome IN (
                         'matched_exact', 'matched_llm', 'no_match', 'skipped_discovered_type', 'skipped_no_llm', 'error'
                     )),
                     resolution_trace_id TEXT NOT NULL,
                     source_extraction_trace_id TEXT,
                     model TEXT,
                     duration_ms INTEGER,
                     resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     UNIQUE (agent_id, subject_entity_id)
                 );
                 CREATE INDEX idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);

                 -- v27 coalesce: read from backup tables, dedup, write to v27 tables.
                 {coalesce}

                 INSERT INTO schema_version (version) VALUES (27);"
        );

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(&sql)
            .context("failed to migrate v26 -> v27 (docs_root_hash shared-corpus)")?;
        tx.commit()?;
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;

        // Log post-coalesce counts from the new v27 tables.
        let post_chunks: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_chunks", [], |r| r.get(0))
            .unwrap_or(0);
        let post_entities: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_subject_entities", [], |r| r.get(0))
            .unwrap_or(0);
        let post_relationships: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_subject_relationships", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        let post_extractions: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_extractions", [], |r| r.get(0))
            .unwrap_or(0);
        let post_resolutions: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_subject_resolutions", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        let post_res_log: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        info!(
            post_chunks,
            post_entities,
            post_relationships,
            post_extractions,
            post_resolutions,
            post_res_log,
            chunks_deduped = pre_chunks - post_chunks,
            entities_deduped = pre_entities - post_entities,
            relationships_deduped = pre_relationships - post_relationships,
            extractions_deduped = pre_extractions - post_extractions,
            "v26->v27 coalesce: migration complete"
        );

        if pre_chunks > 0 && post_chunks == 0 {
            warn!("v26->v27 coalesce: all chunks were lost — this may indicate a migration bug");
        }

        Ok(())
    }

    /// v27→v28: Add `agent_kg_corpora` table for multi-corpus per-agent KG (#798).
    /// Maps `agent_id → {docs_root_hash, docs_root_path}` so the query path knows
    /// which corpora to fan out across without re-deriving from identity.
    /// Backfills from existing `kg_subject_resolutions → kg_subject_entities` joins.
    fn migrate_v27_to_v28(&mut self) -> Result<()> {
        // Idempotency: skip if table already exists.
        let exists: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='agent_kg_corpora'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);
        if exists {
            self.conn.execute(
                "UPDATE schema_version SET version = 28 WHERE version < 28",
                [],
            )?;
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "CREATE TABLE agent_kg_corpora (
                 agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                 docs_root_hash TEXT NOT NULL,
                 docs_root_path TEXT NOT NULL,
                 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 PRIMARY KEY (agent_id, docs_root_hash)
             );

             CREATE INDEX idx_agent_kg_corpora_hash ON agent_kg_corpora(docs_root_hash);

             INSERT OR IGNORE INTO agent_kg_corpora (agent_id, docs_root_hash, docs_root_path)
                 SELECT DISTINCT r.agent_id, e.docs_root_hash, e.docs_root
                 FROM kg_subject_resolutions r
                 JOIN kg_subject_entities e ON e.id = r.subject_entity_id
                 WHERE e.docs_root_hash IS NOT NULL AND e.docs_root IS NOT NULL;

             UPDATE schema_version SET version = 28;",
        )?;
        tx.commit()?;

        Ok(())
    }

    /// Backfill migration: scrub secret-shaped values from existing tool_calls
    /// rows (#908). Data-only — no DDL changes. Applies `scrub_secrets()` to
    /// `input` and `output` columns, updating only rows that change.
    fn migrate_v28_to_v29(&mut self) -> Result<()> {
        use crate::secret_scrubber::scrub_secrets;

        let version = self.schema_version()?;
        if version >= 29 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Collect IDs and content of rows that have non-NULL text fields.
        let mut stmt = tx.prepare(
            "SELECT id, input, output, error_message FROM tool_calls
             WHERE input IS NOT NULL OR output IS NOT NULL OR error_message IS NOT NULL",
        )?;
        #[allow(clippy::type_complexity)]
        let rows: Vec<(String, Option<String>, Option<String>, Option<String>)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);

        let mut updated = 0u64;
        for (id, input, output, error_msg) in &rows {
            let scrubbed_input = input.as_deref().map(scrub_secrets);
            let scrubbed_output = output.as_deref().map(scrub_secrets);
            let scrubbed_error = error_msg.as_deref().map(scrub_secrets);

            // Only UPDATE if scrubbing changed at least one field.
            let input_changed = matches!(&scrubbed_input, Some(std::borrow::Cow::Owned(_)));
            let output_changed = matches!(&scrubbed_output, Some(std::borrow::Cow::Owned(_)));
            let error_changed = matches!(&scrubbed_error, Some(std::borrow::Cow::Owned(_)));

            if input_changed || output_changed || error_changed {
                tx.execute(
                    "UPDATE tool_calls SET input = ?1, output = ?2, error_message = ?3 WHERE id = ?4",
                    params![
                        scrubbed_input.as_deref(),
                        scrubbed_output.as_deref(),
                        scrubbed_error.as_deref(),
                        id,
                    ],
                )?;
                updated += 1;
            }
        }

        tx.execute("UPDATE schema_version SET version = 29", [])?;
        tx.commit()?;

        if updated > 0 {
            info!(
                updated_rows = updated,
                total_rows = rows.len(),
                "v28→v29: scrubbed secrets from existing tool_calls rows"
            );
        }

        Ok(())
    }

    /// v29→v30: Expand `kg_resolutions_log.outcome` CHECK constraint to include
    /// `'matched_llm_db_fallback'` (#874). Table rebuild mirroring the v26→v27
    /// shape: RENAME → CREATE → INSERT INTO ... SELECT → DROP → recreate index.
    fn migrate_v29_to_v30(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 30 {
            return Ok(());
        }

        let count_before: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "PRAGMA foreign_keys = OFF;

             ALTER TABLE kg_resolutions_log RENAME TO kg_resolutions_log_v29_backup;

             CREATE TABLE kg_resolutions_log (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                 subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                 outcome TEXT NOT NULL CHECK (outcome IN (
                     'matched_exact', 'matched_llm', 'matched_llm_db_fallback',
                     'no_match', 'skipped_discovered_type', 'skipped_no_llm', 'error'
                 )),
                 resolution_trace_id TEXT NOT NULL,
                 source_extraction_trace_id TEXT,
                 model TEXT,
                 duration_ms INTEGER,
                 resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 UNIQUE (agent_id, subject_entity_id)
             );

             INSERT INTO kg_resolutions_log
                 (id, agent_id, subject_entity_id, outcome, resolution_trace_id,
                  source_extraction_trace_id, model, duration_ms, resolved_at)
             SELECT id, agent_id, subject_entity_id, outcome, resolution_trace_id,
                    source_extraction_trace_id, model, duration_ms, resolved_at
             FROM kg_resolutions_log_v29_backup;

             DROP TABLE kg_resolutions_log_v29_backup;

             CREATE INDEX idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);

             PRAGMA foreign_keys = ON;

             UPDATE schema_version SET version = 30;",
        )?;
        tx.commit()?;

        let count_after: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        info!(
            count_before = count_before,
            count_after = count_after,
            "v29→v30: expanded kg_resolutions_log outcome CHECK constraint (#874)"
        );

        Ok(())
    }

    fn migrate_v30_to_v31(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 31 {
            return Ok(());
        }

        // Columns may already exist in clean-slate DBs; check each independently
        // to handle partial-crash recovery scenarios.
        let has_response_text = self.column_exists("llm_calls", "response_text")?;
        let has_reasoning = self.column_exists("llm_calls", "reasoning")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !has_response_text {
            tx.execute_batch("ALTER TABLE llm_calls ADD COLUMN response_text TEXT;")?;
        }
        if !has_reasoning {
            tx.execute_batch("ALTER TABLE llm_calls ADD COLUMN reasoning TEXT;")?;
        }
        tx.execute("INSERT INTO schema_version (version) VALUES (31)", [])?;
        tx.commit()?;

        info!("v30→v31: added response_text and reasoning columns to llm_calls (#653)");

        Ok(())
    }

    /// v31→v32: Add `kg_invalidated_no_match` sidecar table (#961).
    ///
    /// Ephemeral marker table for tracking entities whose `no_match` resolution
    /// log rows were deleted by domain-graph rebuild invalidation (#960).
    /// The resolver reads and cleans up these markers during resolution.
    fn migrate_v31_to_v32(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 32 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS kg_invalidated_no_match (
                subject_entity_id INTEGER NOT NULL,
                agent_id TEXT NOT NULL,
                invalidated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (subject_entity_id, agent_id)
            );",
        )?;
        tx.execute("INSERT INTO schema_version (version) VALUES (32)", [])?;
        tx.commit()?;

        info!("v31→v32: added kg_invalidated_no_match sidecar table (#961)");

        Ok(())
    }

    fn migrate_v32_to_v33(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 33 {
            return Ok(());
        }

        // #1052: Delete kg_extractions rows with NULL source_doc_hash.
        // These are pre-v26 rows that escaped the v27 backfill due to
        // NULL = NULL being falsy in SQL. They create a deadlock: the
        // pending query says "extract me" but INSERT OR IGNORE (now
        // replaced with upsert in #1052) would skip them. Deleting makes
        // them cleanly pending for re-extraction with the new upsert.
        // Safe because kg_extractions is an idempotency marker, not data.
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let deleted: usize = tx.execute(
            "DELETE FROM kg_extractions WHERE source_doc_hash IS NULL",
            [],
        )?;
        tx.execute("INSERT INTO schema_version (version) VALUES (33)", [])?;
        tx.commit()?;

        if deleted > 0 {
            info!(
                deleted = deleted,
                "v32→v33: deleted {deleted} NULL-hash kg_extractions rows (#1052)"
            );
        } else {
            info!("v32→v33: no NULL-hash kg_extractions rows to clean up (#1052)");
        }

        Ok(())
    }

    fn migrate_v33_to_v34(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 34 {
            return Ok(());
        }

        // #1001: Add dispatch_class column for per-class dispatch slot split.
        // Nullable — pre-v34 rows stay NULL, treated as 'implement' via COALESCE
        // in the dispatch guard query. CHECK constraint limits values.
        // Column-exists guard for crash-recovery and convergence-test safety.
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let has_col: bool = tx
            .prepare("PRAGMA table_info(tasks)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .any(|name| name == "dispatch_class");
        if !has_col {
            tx.execute_batch(
                "ALTER TABLE tasks ADD COLUMN dispatch_class TEXT
                   CHECK (dispatch_class IS NULL OR dispatch_class IN ('implement', 'groom'));",
            )?;
        }
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_tasks_dispatch_class
               ON tasks(agent_id, dispatch_class, status)
               WHERE dispatch_class IS NOT NULL;",
        )?;
        tx.execute("INSERT INTO schema_version (version) VALUES (34)", [])?;
        tx.commit()?;

        info!("v33→v34: added dispatch_class column to tasks (#1001)");
        Ok(())
    }

    /// v34→v35: Expand `kg_resolutions_log.outcome` CHECK constraint to include
    /// `'no_candidate_of_type'` (#1154). Table rebuild mirroring the v29→v30
    /// shape: RENAME → CREATE → INSERT INTO ... SELECT → DROP → recreate index.
    fn migrate_v34_to_v35(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 35 {
            return Ok(());
        }

        let count_before: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "PRAGMA foreign_keys = OFF;

             ALTER TABLE kg_resolutions_log RENAME TO kg_resolutions_log_v34_backup;

             CREATE TABLE kg_resolutions_log (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                 subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                 outcome TEXT NOT NULL CHECK (outcome IN (
                     'matched_exact', 'matched_llm', 'matched_llm_db_fallback',
                     'no_match', 'no_candidate_of_type',
                     'skipped_discovered_type', 'skipped_no_llm', 'error'
                 )),
                 resolution_trace_id TEXT NOT NULL,
                 source_extraction_trace_id TEXT,
                 model TEXT,
                 duration_ms INTEGER,
                 resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 UNIQUE (agent_id, subject_entity_id)
             );

             INSERT INTO kg_resolutions_log
                 (id, agent_id, subject_entity_id, outcome, resolution_trace_id,
                  source_extraction_trace_id, model, duration_ms, resolved_at)
             SELECT id, agent_id, subject_entity_id, outcome, resolution_trace_id,
                    source_extraction_trace_id, model, duration_ms, resolved_at
             FROM kg_resolutions_log_v34_backup;

             DROP TABLE kg_resolutions_log_v34_backup;

             CREATE INDEX idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);

             PRAGMA foreign_keys = ON;

             UPDATE schema_version SET version = 35;",
        )?;
        tx.commit()?;

        let count_after: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        info!(
            count_before = count_before,
            count_after = count_after,
            "v34→v35: expanded kg_resolutions_log outcome CHECK to include 'no_candidate_of_type' (#1154)"
        );

        Ok(())
    }

    /// v35→v36: Add `discovered` and `discovery_reason` columns to
    /// `kg_subject_entities` for roster-grounding (#1158).
    fn migrate_v35_to_v36(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 36 {
            return Ok(());
        }

        // Column-exists guards for crash-recovery safety (per v30→v31 precedent).
        let has_discovered = self.column_exists("kg_subject_entities", "discovered")?;
        let has_discovery_reason = self.column_exists("kg_subject_entities", "discovery_reason")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        if !has_discovered {
            tx.execute(
                "ALTER TABLE kg_subject_entities ADD COLUMN discovered INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !has_discovery_reason {
            tx.execute(
                "ALTER TABLE kg_subject_entities ADD COLUMN discovery_reason TEXT",
                [],
            )?;
        }

        tx.execute("UPDATE schema_version SET version = 36", [])?;
        tx.commit()?;

        info!(
            "v35→v36: added discovered + discovery_reason columns to kg_subject_entities (#1158)"
        );

        Ok(())
    }

    /// v36→v37: Expand `kg_resolutions_log.outcome` CHECK constraint to include
    /// `'skipped_discovered_subject'` (#1158). Table rebuild.
    fn migrate_v36_to_v37(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 37 {
            return Ok(());
        }

        let count_before: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "PRAGMA foreign_keys = OFF;

             ALTER TABLE kg_resolutions_log RENAME TO kg_resolutions_log_v36_backup;

             CREATE TABLE kg_resolutions_log (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                 subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                 outcome TEXT NOT NULL CHECK (outcome IN (
                     'matched_exact', 'matched_llm', 'matched_llm_db_fallback',
                     'no_match', 'no_candidate_of_type',
                     'skipped_discovered_type', 'skipped_discovered_subject',
                     'skipped_no_llm', 'error'
                 )),
                 resolution_trace_id TEXT NOT NULL,
                 source_extraction_trace_id TEXT,
                 model TEXT,
                 duration_ms INTEGER,
                 resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 UNIQUE (agent_id, subject_entity_id)
             );

             INSERT INTO kg_resolutions_log
                 (id, agent_id, subject_entity_id, outcome, resolution_trace_id,
                  source_extraction_trace_id, model, duration_ms, resolved_at)
             SELECT id, agent_id, subject_entity_id, outcome, resolution_trace_id,
                    source_extraction_trace_id, model, duration_ms, resolved_at
             FROM kg_resolutions_log_v36_backup;

             DROP TABLE kg_resolutions_log_v36_backup;

             CREATE INDEX idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);

             PRAGMA foreign_keys = ON;

             UPDATE schema_version SET version = 37;",
        )?;
        tx.commit()?;

        let count_after: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        info!(
            count_before = count_before,
            count_after = count_after,
            "v36→v37: expanded kg_resolutions_log outcome CHECK to include 'skipped_discovered_subject' (#1158)"
        );

        Ok(())
    }

    /// v37→v38: Add `system_prompt_bytes` column to `llm_calls` (mika#1217).
    ///
    /// Per-call assembled-system-prompt byte count for context-budget
    /// observability. Nullable; pre-v38 rows stay NULL. Mirrors v30→v31's
    /// additive-nullable shape and the `column_exists` guard pattern.
    fn migrate_v37_to_v38(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 38 {
            return Ok(());
        }

        let has_column = self.column_exists("llm_calls", "system_prompt_bytes")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !has_column {
            tx.execute_batch("ALTER TABLE llm_calls ADD COLUMN system_prompt_bytes INTEGER;")?;
        }
        tx.execute("INSERT INTO schema_version (version) VALUES (38)", [])?;
        tx.commit()?;

        info!("v37→v38: added system_prompt_bytes column to llm_calls (mika#1217)");

        Ok(())
    }

    /// v38→v39: Add `operational_items` table (mika#1262).
    ///
    /// Canonical operational-item ledger for the What's Next engine.
    /// New table with indexes, no existing table changes.
    fn migrate_v38_to_v39(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 39 {
            return Ok(());
        }

        let has_table: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='operational_items'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        if !has_table {
            tx.execute_batch(
                "CREATE TABLE operational_items (
                    id TEXT PRIMARY KEY,
                    kind TEXT NOT NULL CHECK (kind IN ('goal', 'task', 'commitment', 'decision', 'blocker', 'evidence', 'next_action')),
                    title TEXT NOT NULL,
                    status TEXT NOT NULL CHECK (status IN ('now', 'waiting', 'delegated', 'scheduled', 'at_risk', 'done')),
                    owner_type TEXT NOT NULL CHECK (owner_type IN ('user', 'mika', 'person', 'agent')),
                    owner_name TEXT,
                    priority REAL NOT NULL DEFAULT 0.0,
                    user_importance REAL NOT NULL DEFAULT 0.0,
                    due_at TEXT,
                    blocked_by TEXT,
                    next_action TEXT,
                    evidence_refs TEXT NOT NULL DEFAULT '[]',
                    confidence REAL NOT NULL DEFAULT 1.0,
                    source_table TEXT,
                    source_id TEXT,
                    agent_id TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );

                CREATE INDEX idx_operational_items_agent_status ON operational_items(agent_id, status);
                CREATE INDEX idx_operational_items_agent_kind ON operational_items(agent_id, kind);
                CREATE INDEX idx_operational_items_agent_priority ON operational_items(agent_id, priority DESC);
                CREATE INDEX idx_operational_items_source ON operational_items(source_table, source_id);
                CREATE UNIQUE INDEX idx_operational_items_source_unique
                    ON operational_items(agent_id, source_table, source_id)
                    WHERE source_table IS NOT NULL AND source_id IS NOT NULL;",
            )?;
        }

        tx.execute("INSERT INTO schema_version (version) VALUES (39)", [])?;
        tx.commit()?;

        info!("v38→v39: added operational_items table (mika#1262)");

        Ok(())
    }

    /// v39→v40: Delete all mika-relay agent data (mika#1193).
    ///
    /// Self-contained: explicit deletes in reverse-dependency order. Correctness
    /// does NOT depend on PRAGMA foreign_keys being ON.
    fn migrate_v39_to_v40(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 40 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Delete in reverse-dependency order: descendants before ancestors.
        // Each statement is idempotent (no-op if rows are already gone).
        tx.execute_batch(
            "-- v40: mika#1193 retire mika-relay agent.
            DELETE FROM tool_calls
              WHERE session_id IN (SELECT id FROM sessions WHERE agent_id = 'mika-relay');

            DELETE FROM llm_calls
              WHERE session_id IN (SELECT id FROM sessions WHERE agent_id = 'mika-relay');

            DELETE FROM messages
              WHERE session_id IN (SELECT id FROM sessions WHERE agent_id = 'mika-relay');

            DELETE FROM skill_overrides
              WHERE agent_id = 'mika-relay';

            DELETE FROM audit_events
              WHERE session_id IN (SELECT id FROM sessions WHERE agent_id = 'mika-relay');

            DELETE FROM operational_items
              WHERE agent_id = 'mika-relay';

            DELETE FROM tasks
              WHERE agent_id = 'mika-relay';

            DELETE FROM sessions
              WHERE agent_id = 'mika-relay';

            DELETE FROM agents
              WHERE id = 'mika-relay';",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (40)", [])?;
        tx.commit()?;

        info!("v39→v40: deleted mika-relay agent data (mika#1193)");

        Ok(())
    }

    /// v40→v41: Add `task_messages` parallel narrative table (mika#974).
    ///
    /// Additive — no existing table altered, no data touched. Safe on live DB.
    fn migrate_v40_to_v41(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 41 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute_batch(
            "-- v41: mika#974 task_messages parallel narrative table.
            CREATE TABLE IF NOT EXISTS task_messages (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id    TEXT NOT NULL,
                agent_id   TEXT NOT NULL,
                session_id TEXT NOT NULL,
                role       TEXT NOT NULL,
                content    TEXT NOT NULL,
                metadata   TEXT,
                trace_id   TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE INDEX IF NOT EXISTS idx_task_messages_task_created
                ON task_messages (task_id, created_at);

            CREATE INDEX IF NOT EXISTS idx_task_messages_agent_created
                ON task_messages (agent_id, created_at);

            INSERT INTO schema_version (version) VALUES (41);",
        )?;

        tx.commit()?;

        info!("v40→v41: added task_messages table (mika#974)");

        Ok(())
    }

    /// v41→v42: Add `auto_pull_stats` circuit-breaker tracking table (mika#1363).
    fn migrate_v41_to_v42(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 42 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute_batch(
            "-- v42: mika#1363 auto-pull circuit-breaker stats table.
            CREATE TABLE IF NOT EXISTS auto_pull_stats (
                repo_full_name TEXT NOT NULL,
                issue_number INTEGER NOT NULL,
                failure_count INTEGER NOT NULL DEFAULT 0,
                last_auto_pull_at TEXT,
                last_failure_at TEXT,
                PRIMARY KEY (repo_full_name, issue_number)
            );",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (42)", [])?;
        tx.commit()?;

        info!("v41→v42: created auto_pull_stats table (mika#1363)");

        Ok(())
    }

    /// v42→v43: Add `lifecycle_state`, `use_count`, `last_used_at` columns to
    /// `skill_overrides` for curator background task (mika#1584).
    fn migrate_v42_to_v43(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 43 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Additive ALTER TABLE with column_exists guard for crash-recovery safety.
        if !Self::column_exists_tx(&tx, "skill_overrides", "lifecycle_state")? {
            tx.execute_batch(
                "ALTER TABLE skill_overrides ADD COLUMN lifecycle_state TEXT
                 CHECK (lifecycle_state IN ('staged', 'active', 'archived'));",
            )?;
        }
        if !Self::column_exists_tx(&tx, "skill_overrides", "use_count")? {
            tx.execute_batch(
                "ALTER TABLE skill_overrides ADD COLUMN use_count INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
        if !Self::column_exists_tx(&tx, "skill_overrides", "last_used_at")? {
            tx.execute_batch("ALTER TABLE skill_overrides ADD COLUMN last_used_at TEXT;")?;
        }

        tx.execute("INSERT INTO schema_version (version) VALUES (43)", [])?;
        tx.commit()?;

        info!(
            "v42→v43: added lifecycle_state, use_count, last_used_at to skill_overrides (mika#1584)"
        );

        Ok(())
    }

    /// v43→v44: additive `permission_decisions` provenance ledger (mika#1733 AC4).
    ///
    /// Records every operator permission decision routed through
    /// `PermissionsChannel::resolve_decision`, including the classifier
    /// verdict, operator ratification, derived `override_used` flag, and the
    /// scope (tenant/agent) at decision time. Additive-only — no rebuild of
    /// existing tables. Two indexes support the two expected query shapes:
    /// per-request lookup and time-window scans.
    fn migrate_v43_to_v44(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 44 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS permission_decisions (
                id TEXT PRIMARY KEY,
                request_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                args_summary TEXT,
                classifier_verdict TEXT NOT NULL
                    CHECK (classifier_verdict IN ('approved', 'denied', 'held')),
                operator_decision TEXT
                    CHECK (operator_decision IN ('approve', 'deny')),
                override_used INTEGER NOT NULL DEFAULT 0
                    CHECK (override_used IN (0, 1)),
                decision_authority TEXT NOT NULL
                    CHECK (decision_authority IN ('strict', 'override')),
                tenant_id TEXT,
                agent_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX IF NOT EXISTS idx_permission_decisions_request_id
                ON permission_decisions(request_id);
            CREATE INDEX IF NOT EXISTS idx_permission_decisions_created_at
                ON permission_decisions(created_at DESC);",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (44)", [])?;
        tx.commit()?;

        info!("v43→v44: added permission_decisions provenance table (mika#1733 AC4)");

        Ok(())
    }

    /// v48→v49: expand the `team_runs.status` CHECK constraint to include
    /// `'failed_transport'` (mika#1671 D3). Composes on top of v46→v47's
    /// `'failed_no_delegation'` addition (mika#1676) — both terminal states
    /// cover different failure classes (all-transport-failed short-circuit vs
    /// zero-delegation gate) and coexist in the CHECK; the D3 architect pin
    /// explicitly says they're orthogonal.
    ///
    /// SQLite cannot alter a CHECK in place, so this is a table rebuild.
    /// `team_runs` is FK-referenced by `tasks.team_run_id`, so the rebuild uses
    /// the **build-new-then-swap** shape (CREATE `team_runs_new` →
    /// INSERT SELECT → DROP `team_runs` → RENAME `team_runs_new` → `team_runs`),
    /// NOT the rename-to-backup shape used by v34→v35. Renaming the *referenced*
    /// table first would make SQLite (with the default `legacy_alter_table = OFF`)
    /// rewrite `tasks.team_run_id`'s FK target to the backup name, leaving a
    /// dangling reference after the backup is dropped (caught by
    /// `test_v1_and_incremental_schemas_converge`). Renaming the *new* table into
    /// place instead leaves `tasks`'s existing `team_runs` reference untouched.
    /// Symmetric to v46→v47's shape. `foreign_keys` is toggled OFF around the
    /// rebuild so the DROP/RENAME does not trip FK enforcement. Carries forward
    /// the v46→v47 columns (`delegation_count`, `solo_absorption`,
    /// `failure_context`) untouched. Row count is preserved.
    fn migrate_v48_to_v49(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 49 {
            return Ok(());
        }

        let count_before: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM team_runs", [], |r| r.get(0))
            .unwrap_or(0);

        // PRAGMA foreign_keys is a no-op inside a transaction, so it is set on
        // the connection before BEGIN and restored after COMMIT.
        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "CREATE TABLE team_runs_new (
                 id TEXT PRIMARY KEY,
                 team_id TEXT NOT NULL REFERENCES teams(id),
                 goal TEXT NOT NULL,
                 status TEXT NOT NULL DEFAULT 'running'
                     CHECK (status IN (
                         'running','completed','failed','cancelled','suspended',
                         'failed_no_delegation','failed_transport'
                     )),
                 failure_reason TEXT,
                 iteration INTEGER NOT NULL DEFAULT 1,
                 max_iterations INTEGER NOT NULL DEFAULT 3,
                 deliverable TEXT,
                 checkpoint TEXT,
                 trace_id TEXT,
                 started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 ended_at TEXT,
                 delegation_count INTEGER NOT NULL DEFAULT 0,
                 solo_absorption INTEGER NOT NULL DEFAULT 0,
                 failure_context TEXT
             );

             INSERT INTO team_runs_new
                 (id, team_id, goal, status, failure_reason,
                  iteration, max_iterations, deliverable, checkpoint,
                  trace_id, started_at, ended_at,
                  delegation_count, solo_absorption, failure_context)
             SELECT id, team_id, goal, status, failure_reason,
                    iteration, max_iterations, deliverable, checkpoint,
                    trace_id, started_at, ended_at,
                    delegation_count, solo_absorption, failure_context
             FROM team_runs;

             DROP TABLE team_runs;

             ALTER TABLE team_runs_new RENAME TO team_runs;

             CREATE INDEX idx_team_runs_team ON team_runs(team_id, started_at DESC);",
        )?;
        tx.execute("INSERT INTO schema_version (version) VALUES (49)", [])?;
        tx.commit()?;

        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;

        let count_after: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM team_runs", [], |r| r.get(0))
            .unwrap_or(0);

        info!(
            count_before = count_before,
            count_after = count_after,
            "v48→v49: expanded team_runs.status CHECK to include 'failed_transport' (mika#1671)"
        );

        Ok(())
    }

    /// v49→v50: additive re-drive accounting on `auto_pull_stats` (mika#2020).
    ///
    /// `failure_count` means "the `gh` call failed" and is reset on **every**
    /// successful Phase 2 rescue and Phase 1 promotion. A re-drive counter has
    /// to increment on exactly that event. The two semantics are opposed at the
    /// same point in the code, which is why mika#1901 could be re-driven 16
    /// times in 19 h without the circuit breaker ever seeing it: each rescue
    /// succeeded at the API, so each rescue zeroed the only counter that
    /// existed. Three additive columns, no table rebuild:
    ///
    /// - `redrive_count` — successful Phase 2 re-drives since the last observed
    ///   progress (an open PR closing the ticket, or an in-flight self_dev task).
    /// - `last_redrive_at` — timestamp of the most recent re-drive.
    /// - `redrive_abandoned_at` — set when the budget is exhausted and the
    ///   ticket is handed to the operator. Its presence is what distinguishes
    ///   "the budget just ran out, the label is not posted yet" from "the budget
    ///   ran out earlier and the operator has since removed the label", which is
    ///   the re-entry gesture.
    fn migrate_v49_to_v50(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 50 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute_batch(
            "-- v50: mika#2020 per-ticket re-drive budget + named abandonment.
            ALTER TABLE auto_pull_stats ADD COLUMN redrive_count INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE auto_pull_stats ADD COLUMN last_redrive_at TEXT;
            ALTER TABLE auto_pull_stats ADD COLUMN redrive_abandoned_at TEXT;",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (50)", [])?;
        tx.commit()?;

        info!("v49→v50: added re-drive accounting to auto_pull_stats (mika#2020)");

        Ok(())
    }

    /// v50 → v51 (mika#1948, Porte 2) — three-dispatcher exec-slot arbitration.
    ///
    /// Two additions, one axis each:
    ///
    /// 1. `tasks.dispatcher_source` — WHICH dispatcher inside this engine
    ///    initiated a task (`mika_dev` | `mika_manager` | `operator`). Nullable:
    ///    pre-v51 rows are all mika-dev by definition (the autonomous loop was
    ///    the only dispatcher), so NULL is an unambiguous "pre-v51, therefore
    ///    mika_dev" sentinel read through `COALESCE(dispatcher_source,
    ///    'mika_dev')`. That keeps the migration O(1) instead of rewriting every
    ///    existing row, and preserves the forensic distinction between "pre-v51
    ///    row" and "post-v51 row explicitly written by mika-dev".
    ///
    /// 2. `dispatch_slot_leases` — makes an assigned exec slot an OBSERVABLE
    ///    FACT rather than a convention. See `try_acquire_dispatch_slot` for the
    ///    full reasoning; the short version is that
    ///    `has_active_callback_tasks_excluding` only ever *checked* the slot, and
    ///    the row that makes it held is written seconds later, so two dispatchers
    ///    could both read "free" and both proceed.
    ///
    /// NOTE: this is a DIFFERENT axis from the `dispatch:*` seat label (mika#2084,
    /// `webhook_dispatch::CURRENT_DISPATCH_SEAT`). The seat says which *engine*
    /// owns a TICKET and is carried on the GitHub issue; `dispatcher_source` says
    /// which *role inside this engine* initiated a TASK. The seat gate refuses a
    /// ticket that belongs to another engine; this arbitrates the exec slot among
    /// the dispatchers of one engine. Neither subsumes the other, and they are
    /// deliberately not merged into one column.
    // FIXME(mika#1948-AC10): once this PR merges, update
    // `mika-platform/docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md`
    // § 3 Porte 2 to `**Statut : DISCHARGED**`, naming this ticket and PR. That
    // file lives in the meta-repo, outside this worktree, so it cannot be
    // touched here — this marker is the searchable reminder that survives the
    // squash. Remove it in the follow-up commit that lands the doc update.
    fn migrate_v50_to_v51(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 51 {
            return Ok(());
        }
        // Fail fast on an unexpected baseline. The idempotency guard above
        // already returned for >=51, so reaching here with anything but v50
        // means the migration-order assumption is broken — applying anyway
        // would corrupt the chain silently.
        if version != 50 {
            anyhow::bail!(
                "migrate_v50_to_v51 called with unexpected baseline version {version} \
                 (expected 50) — refusing to apply migration; investigate migration order"
            );
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // The column may already exist if a previous run was interrupted
        // between the ALTER and the version bump; PRAGMA-detect, do not assume.
        let has_col: bool = {
            let mut stmt = tx.prepare("PRAGMA table_info(tasks)")?;
            let names: Vec<String> = stmt
                .query_map([], |r| r.get::<_, String>(1))?
                .filter_map(|r| r.ok())
                .collect();
            names.iter().any(|n| n == "dispatcher_source")
        };
        if !has_col {
            tx.execute_batch(
                "ALTER TABLE tasks ADD COLUMN dispatcher_source TEXT CHECK (
                     dispatcher_source IS NULL
                     OR dispatcher_source IN ('mika_dev', 'mika_manager', 'operator')
                 );",
            )?;
        }

        tx.execute_batch(
            "-- v51: mika#1948 Porte 2 — 3-dispatcher exec-slot arbitration.
             CREATE INDEX IF NOT EXISTS idx_tasks_dispatcher_source
                 ON tasks(agent_id, dispatcher_source, status)
                 WHERE dispatcher_source IS NOT NULL;

             -- One row per (agent, class) = one exec slot. The PRIMARY KEY is
             -- what makes the claim atomic: a second claimant's INSERT collides
             -- instead of racing a SELECT.
             CREATE TABLE IF NOT EXISTS dispatch_slot_leases (
                 agent_id TEXT NOT NULL,
                 dispatch_class TEXT NOT NULL,
                 holder_task_id TEXT NOT NULL,
                 dispatcher_source TEXT CHECK (
                     dispatcher_source IS NULL
                     OR dispatcher_source IN ('mika_dev', 'mika_manager', 'operator')
                 ),
                 acquired_at TEXT NOT NULL,
                 expires_at TEXT NOT NULL,
                 PRIMARY KEY (agent_id, dispatch_class)
             );",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (51)", [])?;
        tx.commit()?;

        info!("v50→v51: added tasks.dispatcher_source + dispatch_slot_leases (mika#1948 Porte 2)");

        Ok(())
    }

    /// v51 → v52 (mika#2160) — the exec-slot lease stops being a hard cap of one.
    ///
    /// `dispatch_slot_leases` carried `PRIMARY KEY (agent_id, dispatch_class)`,
    /// which is itself a cap of one independent of any predicate: a class
    /// cannot hold two live leases whatever the TTL. Adding `slot_index` to the
    /// key is what makes a configurable cap real rather than decorative — see
    /// KTD1 in the mika#2160 plan, and the test that asserts two acquisitions.
    ///
    /// The migration is a table rebuild (SQLite cannot extend a PRIMARY KEY in
    /// place). Existing rows land at `slot_index = 0`, so a database migrated
    /// mid-dispatch keeps its live lease and the holder that owns it.
    ///
    /// At the default cap of 1 exactly one index is ever written and the
    /// behaviour is the pre-v52 behaviour, bit for bit.
    fn migrate_v51_to_v52(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 52 {
            return Ok(());
        }
        if version != 51 {
            anyhow::bail!(
                "migrate_v51_to_v52 called with unexpected baseline version {version} \
                 (expected 51) — refusing to apply migration; investigate migration order"
            );
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // The column may already exist if a previous run was interrupted
        // between the rebuild and the version bump; PRAGMA-detect, do not assume.
        let has_col: bool = {
            let mut stmt = tx.prepare("PRAGMA table_info(dispatch_slot_leases)")?;
            let names: Vec<String> = stmt
                .query_map([], |r| r.get::<_, String>(1))?
                .filter_map(|r| r.ok())
                .collect();
            names.iter().any(|n| n == "slot_index")
        };

        if !has_col {
            tx.execute_batch(
                "-- v52: mika#2160 — slot_index joins the lease key.
                 CREATE TABLE dispatch_slot_leases_v52 (
                     agent_id TEXT NOT NULL,
                     dispatch_class TEXT NOT NULL,
                     slot_index INTEGER NOT NULL DEFAULT 0,
                     holder_task_id TEXT NOT NULL,
                     dispatcher_source TEXT CHECK (
                         dispatcher_source IS NULL
                         OR dispatcher_source IN ('mika_dev', 'mika_manager', 'operator')
                     ),
                     acquired_at TEXT NOT NULL,
                     expires_at TEXT NOT NULL,
                     PRIMARY KEY (agent_id, dispatch_class, slot_index)
                 );

                 INSERT INTO dispatch_slot_leases_v52
                     (agent_id, dispatch_class, slot_index, holder_task_id,
                      dispatcher_source, acquired_at, expires_at)
                 SELECT agent_id, dispatch_class, 0, holder_task_id,
                        dispatcher_source, acquired_at, expires_at
                 FROM dispatch_slot_leases;

                 DROP TABLE dispatch_slot_leases;
                 ALTER TABLE dispatch_slot_leases_v52 RENAME TO dispatch_slot_leases;",
            )?;
        }

        tx.execute("INSERT INTO schema_version (version) VALUES (52)", [])?;
        tx.commit()?;

        info!("v51→v52: dispatch_slot_leases gained slot_index in its key (mika#2160)");

        Ok(())
    }

    /// v52→v53: Add `request_bytes` column to `llm_calls` (mika#2189 D5/Q2).
    ///
    /// # The hole this closes
    ///
    /// mika#2189's AC1 asks for the expiry distribution "by brief size". On the
    /// **error** path `agent_loop` writes `input_tokens = 0` and
    /// `output_tokens = 0` as literals — a failed call carries no token count at
    /// all — so the size of the request that timed out was not recoverable after
    /// the fact. The measurement had to fall back to `system_prompt_bytes`
    /// (mika#1217), which discriminates well but is only half the payload.
    ///
    /// An estimated `input_tokens` was rejected in Q2: the axis exists to
    /// *correlate* size with expiry, and a correlation built on an estimate
    /// cannot settle anything. This is the measured byte length of the
    /// serialized request, written on **both** paths — write it on the success
    /// path only and the hole reopens on exactly the side that matters.
    ///
    /// Nullable and non-retroactive: pre-v53 rows stay NULL, and this does not
    /// recover the 209 failures that motivated the ticket. It makes the *next*
    /// measurement whole. Additive shape and `column_exists` guard mirror
    /// v37→v38, which added `system_prompt_bytes` for the same family of reason.
    fn migrate_v52_to_v53(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 53 {
            return Ok(());
        }

        let has_column = self.column_exists("llm_calls", "request_bytes")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !has_column {
            tx.execute_batch("ALTER TABLE llm_calls ADD COLUMN request_bytes INTEGER;")?;
        }
        tx.execute("INSERT INTO schema_version (version) VALUES (53)", [])?;
        tx.commit()?;

        info!("v52→v53: added request_bytes column to llm_calls (mika#2189)");

        Ok(())
    }

    /// v53 → v54 (mika#2192) — a worktree stops being an unclaimed shared
    /// resource.
    ///
    /// `_set_up_worktree` derives `WORKTREE_DIR` from the branch and walks in.
    /// Between the derivation and the entry nothing reads any state: the path
    /// is a FUNCTION of the branch, never an allocation. Two actors on the same
    /// ticket therefore get the same directory by construction, and the first
    /// gesture on the resume path is `git stash push --include-untracked` — so
    /// a file someone else has just written disappears between two tool calls,
    /// silently, into a stash stack shared by every worktree of the repo.
    ///
    /// `dispatch_slot_leases` arbitrates the exec SLOT and does it correctly.
    /// It never claimed to arbitrate a DIRECTORY, and an orchestrator session
    /// does not appear in it at all — it is not a dispatch.
    ///
    /// Additive `CREATE TABLE`, nothing FK-references it, no existing row is
    /// read or rewritten. Renuméroté de v52→v53 à v53→v54 (mika#2202) : #2189
    /// a pris le slot v53 (colonne request_bytes) entre le grooming et le merge.
    ///
    /// **No `pid`, no `pgrep`, no `/proc`.** Liveness is `expires_at > now`, the
    /// exact property `try_acquire_dispatch_slot` already documents: a claimant
    /// that dies mid-flight blocks its ticket for at most one TTL, not forever.
    fn migrate_v53_to_v54(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 54 {
            return Ok(());
        }
        if version != 53 {
            anyhow::bail!(
                "migrate_v53_to_v54 called with unexpected baseline version {version} \
                 (expected 53) — refusing to apply migration; investigate migration order"
            );
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute_batch(
            "-- v54: mika#2192 — worktree ownership registry.
             CREATE TABLE IF NOT EXISTS worktree_claims (
                 repo TEXT NOT NULL,
                 issue_number INTEGER NOT NULL,
                 owner_kind TEXT NOT NULL CHECK (
                     owner_kind IN ('pilot', 'orchestrator', 'spawn')
                 ),
                 owner_id TEXT NOT NULL,
                 owner_label TEXT,
                 claimed_at TEXT NOT NULL,
                 expires_at TEXT NOT NULL,
                 PRIMARY KEY (repo, issue_number)
             );",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (54)", [])?;
        tx.commit()?;

        info!("v53→v54: added worktree_claims (mika#2192)");

        Ok(())
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

    /// v44→v45: additive `pilot_transcripts` table (mika#1705).
    ///
    /// Captures the LLM-call transcripts emitted by claude-pilot subprocesses
    /// (the implementation-reasoning corpus, ~90% of the trajectory that the
    /// in-process `llm_calls` table never sees). Rows are ingested by the
    /// engine tick from `~/.mika/data/pilot-transcripts/<task-id>.jsonl` files
    /// written by claude-pilot-py and linked back to the dispatching callback
    /// `task_id`. Additive-only — no rebuild of existing tables. Two indexes
    /// support the two query shapes: per-task correlation and time-window
    /// retention scans.
    fn migrate_v44_to_v45(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 45 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS pilot_transcripts (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL,
                timestamp TEXT,
                provider TEXT,
                model TEXT,
                request_body TEXT,
                response_body TEXT,
                tokens_in INTEGER,
                tokens_out INTEGER,
                latency_ms INTEGER,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX IF NOT EXISTS idx_pilot_transcripts_task_id
                ON pilot_transcripts(task_id);
            CREATE INDEX IF NOT EXISTS idx_pilot_transcripts_created_at
                ON pilot_transcripts(created_at DESC);",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (45)", [])?;
        tx.commit()?;

        info!("v44→v45: added pilot_transcripts table (mika#1705)");

        Ok(())
    }

    /// v45→v46: additive `served_content` table (mika#1867).
    ///
    /// Per-(agent, person, category) ledger of content Mika has served —
    /// proverb, quote, joke, poem, recommendation, story, fact — so that
    /// re-generation on future turns can dedup against exact-match hashes.
    /// Founding incident: Al (Vietnam tester) 2026-07-28 — same zen proverb
    /// served twice, 6 days apart, because history-fetch is global-recency
    /// (not per-user) and there was no content-serve ledger.
    ///
    /// Backward compat: reads return empty for rows pre-migration (agent that
    /// has never served anything = no dedup, safe direction per AC1).
    fn migrate_v45_to_v46(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 46 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS served_content (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                person_id INTEGER NOT NULL REFERENCES people(id) ON DELETE CASCADE,
                category TEXT NOT NULL CHECK (category IN (
                    'proverb','quote','joke','poem','recommendation','story','fact'
                )),
                content_text TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                -- Reserved for v2 fuzzy dedup (embedding cosine similarity per AC6).
                -- Format TBD — likely 384-dim float array as BLOB or hex string.
                content_signature TEXT,
                served_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
                UNIQUE(agent_id, person_id, content_hash)
            );
            CREATE INDEX IF NOT EXISTS idx_served_content_person_cat
                ON served_content(agent_id, person_id, category, served_at DESC);
            CREATE INDEX IF NOT EXISTS idx_served_content_hash
                ON served_content(agent_id, person_id, content_hash);",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (46)", [])?;
        tx.commit()?;

        info!("v45→v46: added served_content table (mika#1867)");

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

    /// v46→v47: team_runs delegation-visibility (mika#1676).
    ///
    /// Two coupled schema changes, one atomic table-rebuild:
    ///
    /// 1. **CHECK expansion** on `team_runs.status` to add
    ///    `'failed_no_delegation'` — the terminal state Unit A's delegation
    ///    gate transitions to when the orchestrator returns Conversational
    ///    for an actionable goal (after one reinforced retry). The v1 DDL
    ///    already declares the expanded set for fresh installs; this
    ///    migration lifts existing databases into parity.
    /// 2. **Additive columns** for Unit B observability:
    ///    - `delegation_count INTEGER NOT NULL DEFAULT 0` — incremented per
    ///      spawned member session in `execute_tasks()`.
    ///    - `solo_absorption INTEGER NOT NULL DEFAULT 0` — flag set by
    ///      `finalize_and_shutdown()` when the run completed with zero
    ///      delegations.
    ///    - `failure_context TEXT` — nullable JSON `{"phase": "…"}` carrying
    ///      the phase in which the delegation gate fired
    ///      (`first_decompose` / `revision_after_critic`).
    ///
    /// Table-rebuild is mandatory here because SQLite does not support
    /// `ALTER TABLE … MODIFY CONSTRAINT` and the CHECK constraint must widen.
    /// Follows the exact shape of `migrate_v34_to_v35` (kg_resolutions_log
    /// outcome CHECK expansion, #1154): rename existing table to `_v46_backup`,
    /// create new table with expanded CHECK + new columns, INSERT SELECT
    /// (backfilling new columns with their DEFAULTs), DROP backup, recreate
    /// index. `PRAGMA foreign_keys = OFF` for the rebuild window because
    /// `tasks.team_run_id` FKs into `team_runs(id)`; the RENAME/DROP would
    /// otherwise violate FK enforcement even though row IDs are preserved.
    fn migrate_v46_to_v47(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 47 {
            return Ok(());
        }

        let count_before: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM team_runs", [], |r| r.get(0))
            .unwrap_or(0);

        // CREATE-INSERT-DROP-RENAME sequence (per the v11→v12 rebuild
        // precedent at `crates/mika-agent/src/db.rs:2696`), NOT the
        // RENAME-CREATE-INSERT-DROP sequence used by the v34→v35
        // kg_resolutions_log rebuild.
        //
        // Rationale — child-FK preservation:
        // Since SQLite 3.26 (2018), a plain `ALTER TABLE … RENAME TO …`
        // rewrites referring FK metadata to track the new name. That is
        // safe for `kg_resolutions_log` (no child table FKs into it), but
        // catastrophic here because `tasks.team_run_id REFERENCES team_runs(id)`.
        // A RENAME-first sequence would silently retarget the FK to
        // `team_runs_v46_backup`, then leave it pointing at a dropped
        // table — every subsequent `INSERT INTO tasks` would fail
        // "no such table: team_runs_v46_backup". `PRAGMA legacy_alter_table = ON`
        // does NOT prevent this rewrite reliably across sqlite versions.
        //
        // The CREATE-INSERT-DROP-RENAME sequence sidesteps the trap:
        // the OLD `team_runs` is dropped BEFORE the new table exists under
        // that name, and the RENAME then creates the target name fresh —
        // so `tasks.team_run_id` continues to name `team_runs` throughout
        // and resolves against the new table when FK checks re-enable.
        // (Symmetric bug would have corrupted every FK-user of team_runs
        // at first write post-deploy — caught by the
        // `test_migrate_v46_to_v47_*` tests and `test_v1_and_incremental_schemas_converge`.)
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "PRAGMA foreign_keys = OFF;

             CREATE TABLE team_runs_new (
                 id TEXT PRIMARY KEY,
                 team_id TEXT NOT NULL REFERENCES teams(id),
                 goal TEXT NOT NULL,
                 status TEXT NOT NULL DEFAULT 'running'
                     CHECK (status IN (
                         'running','completed','failed','cancelled','suspended',
                         'failed_no_delegation'
                     )),
                 failure_reason TEXT,
                 iteration INTEGER NOT NULL DEFAULT 1,
                 max_iterations INTEGER NOT NULL DEFAULT 3,
                 deliverable TEXT,
                 checkpoint TEXT,
                 trace_id TEXT,
                 started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 ended_at TEXT,
                 delegation_count INTEGER NOT NULL DEFAULT 0,
                 solo_absorption INTEGER NOT NULL DEFAULT 0,
                 failure_context TEXT
             );

             INSERT INTO team_runs_new
                 (id, team_id, goal, status, failure_reason,
                  iteration, max_iterations, deliverable, checkpoint,
                  trace_id, started_at, ended_at,
                  delegation_count, solo_absorption, failure_context)
             SELECT id, team_id, goal, status, failure_reason,
                    iteration, max_iterations, deliverable, checkpoint,
                    trace_id, started_at, ended_at,
                    0, 0, NULL
             FROM team_runs;

             DROP TABLE team_runs;

             ALTER TABLE team_runs_new RENAME TO team_runs;

             CREATE INDEX idx_team_runs_team ON team_runs(team_id, started_at DESC);

             PRAGMA foreign_keys = ON;",
        )?;
        tx.execute("INSERT INTO schema_version (version) VALUES (47)", [])?;
        tx.commit()?;

        let count_after: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM team_runs", [], |r| r.get(0))
            .unwrap_or(0);

        info!(
            count_before = count_before,
            count_after = count_after,
            "v46→v47: expanded team_runs.status CHECK to include 'failed_no_delegation' + added delegation_count/solo_absorption/failure_context columns (mika#1676)"
        );

        Ok(())
    }

    /// v47→v48 (mika#1712): behavioral marker only — no DDL. Anchors the
    /// phantom NULL-PID sweep semantics added in
    /// [`super::task_engine::engine::TaskEngine::sweep_null_pid_phantoms`] so
    /// operators reading the migration ledger can pinpoint the schema head at
    /// which sweep telemetry began. Reserved for future DDL if write-time
    /// enforcement (mika#1934 cause-racine) ever lands.
    fn migrate_v47_to_v48(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 48 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("INSERT INTO schema_version (version) VALUES (48)", [])?;
        tx.commit()?;

        info!("v47→v48: no DDL; behavioral marker for mika#1712 phantom sweep");

        Ok(())
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

    // ===== Task CRUD =====

    pub fn create_task(&self, task: &NewTask) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        // `type` defaults to "issue" when the caller passes None or an empty string.
        // The DB CHECK constraint enforces the same allowlist as VALID_TASK_TYPES.
        let task_type = task
            .r#type
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(TASK_TYPE_ISSUE);
        self.conn.execute(
            "INSERT INTO tasks (
                id, agent_id, team_run_id, parent_task_id, depth, label,
                trigger_type, cron_expr, event_source, event_offset_secs, condition_expr,
                next_fire_at, timeout_at, action_type, action_config,
                input_context, created_by_session, created_trace_id,
                reference_url, source, metadata, type, dispatch_class
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6,
                ?7, ?8, ?9, ?10, ?11,
                ?12, ?13, ?14, ?15,
                ?16, ?17, ?18,
                ?19, ?20, ?21, ?22, ?23
             )",
            params![
                id,
                task.agent_id,
                task.team_run_id,
                task.parent_task_id,
                task.depth,
                task.label,
                task.trigger_type,
                task.cron_expr,
                task.event_source,
                task.event_offset_secs,
                task.condition_expr,
                task.next_fire_at,
                task.timeout_at,
                task.action_type,
                task.action_config,
                task.input_context,
                task.created_by_session,
                task.created_trace_id,
                task.reference_url,
                task.source,
                task.metadata,
                task_type,
                task.dispatch_class,
            ],
        )?;
        Ok(id)
    }

    /// Insert a recurring task only if no task with the same (agent_id, label) and
    /// trigger_type='recurring' exists. Returns the task ID if created, None if
    /// already existed or if a recent dead sibling refuses re-registration.
    ///
    /// **mika#1742 Problem B — refuse-to-zombie guard.** The partial unique index
    /// `idx_tasks_unique_recurring` explicitly excludes `'cancelled' | 'failed' |
    /// 'expired' | 'delivered'` statuses. So a bare `INSERT OR IGNORE` silently
    /// creates a fresh row whenever every prior instance died — and every
    /// mika-spirit restart re-triggers the fresh registration. That's the
    /// curator_review zombie root cause (8 dead Mika rows across 8 days per
    /// root-claude's forensic).
    ///
    /// This guard adds a pre-insert query: if any recurring row for the same
    /// `(agent_id, label)` was `failed`/`cancelled` within the last
    /// [`RECURRING_ZOMBIE_GRACE_HOURS`] window, refuse to re-register and log a
    /// `warn!` so the operator sees the surface. The operator can:
    ///
    /// - Wait for the grace window to elapse (fresh registration then proceeds).
    /// - Investigate the previous instance's failure via `mika tasks get <id>`.
    /// - Manually clear the dead row and let the next startup re-register.
    ///
    /// **mika#2271 — config-cancel exemption.** A `cancelled` row is not always
    /// a death: a knob (`MIKA_DEV_AUTO_PULL=0`) or an `identity.toml` toggle
    /// cancels the recurring row on purpose, and removing the knob is meant to
    /// bring the task back. Rows whose `metadata` carries
    /// [`RECURRING_CONFIG_CANCEL_REVERTED_PATH`] — written by
    /// [`Database::revert_config_cancel_recurring_task`] when the boot path
    /// re-declares the task as wanted — are therefore skipped by this guard.
    /// `failed` / `expired` rows are never exempted.
    ///
    /// **mika#2337 — unknown-trigger exemption, single-use.** A death caused by
    /// [`crate::task_engine::DispatchError::UnknownTrigger`] carries
    /// [`RECURRING_UNKNOWN_TRIGGER_PATH`], and this guard skips it: the binary
    /// that re-registers a trigger name is the binary that carries its match
    /// arm, so that death predicts nothing about the next one and the veto only
    /// prolongs the outage. The exemption is **spent** when honoured — the
    /// marker is removed and [`RECURRING_UNKNOWN_TRIGGER_LIFT_CONSUMED_PATH`]
    /// is written in its place — so a *second* unknown-trigger death on the same
    /// label inside the window meets a fully armed veto. The lift buys one
    /// restart, not immunity.
    ///
    /// Non-goal here: fixing the *underlying* dispatch failure for Mika's
    /// specific `curator_review` (Problem A in the ticket). Root-claude's
    /// diagnosis notes PR#1726 (RouteFuture/dashmap wedge) likely already
    /// resolves it. Verification is a Phase-2 follow-up under the Problem-A
    /// investigation.
    pub fn create_recurring_task_if_absent(&self, task: NewTask) -> Result<Option<String>> {
        // mika#2337 — has a lift already been spent on this (agent, label)
        // inside the grace window? The marker rides on the row it was spent on,
        // so it ages out of the window together with the death it absolved:
        // past the grace, the exemption re-arms rather than being lost for good.
        let lift_already_spent: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks
               WHERE agent_id = ?1 AND label = ?2 COLLATE NOCASE
                 AND trigger_type = 'recurring'
                 AND updated_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?3)
                 AND json_valid(metadata)
                 AND COALESCE(json_extract(metadata, ?4), 0) = 1)",
            params![
                task.agent_id,
                task.label,
                RECURRING_ZOMBIE_GRACE_SQL,
                RECURRING_UNKNOWN_TRIGGER_LIFT_CONSUMED_PATH
            ],
            |r| r.get::<_, i64>(0).map(|n| n == 1),
        )?;

        // Zombie guard — refuse to re-register a recurring label whose most
        // recent instance died in the grace window.
        let dead_sibling: Option<(String, String, String)> = self
            .conn
            .query_row(
                "SELECT id, status, updated_at FROM tasks
                 WHERE agent_id = ?1 AND label = ?2 COLLATE NOCASE
                   AND trigger_type = 'recurring'
                   AND status IN ('failed', 'cancelled', 'expired')
                   AND updated_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?3)
                   AND NOT (json_valid(metadata)
                            AND COALESCE(json_extract(metadata, ?4), 0) = 1)
                   AND NOT (?5 = 0
                            AND json_valid(metadata)
                            AND COALESCE(json_extract(metadata, ?6), 0) = 1)
                 ORDER BY updated_at DESC LIMIT 1",
                params![
                    task.agent_id,
                    task.label,
                    RECURRING_ZOMBIE_GRACE_SQL,
                    RECURRING_CONFIG_CANCEL_REVERTED_PATH,
                    i64::from(lift_already_spent),
                    RECURRING_UNKNOWN_TRIGGER_PATH
                ],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;

        if let Some((prev_id, prev_status, prev_updated)) = dead_sibling {
            tracing::warn!(
                agent_id = %task.agent_id,
                label = %task.label,
                previous_task_id = %prev_id,
                previous_status = %prev_status,
                previous_updated_at = %prev_updated,
                grace_hours = RECURRING_ZOMBIE_GRACE_HOURS,
                "mika#1742: refusing to re-register recurring task — recent same-label \
                 instance ended in a terminal-failure state. Investigate root cause \
                 (`mika tasks get {prev_id}`) before re-enabling; re-registration \
                 automatically re-attempts after the grace window elapses."
            );
            return Ok(None);
        }

        let id = uuid::Uuid::new_v4().to_string();
        let task_type = task
            .r#type
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(TASK_TYPE_ISSUE);
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO tasks
             (id, agent_id, team_run_id, parent_task_id, depth, label,
              trigger_type, cron_expr, event_source, event_offset_secs,
              condition_expr, next_fire_at, timeout_at, action_type,
              action_config, status, input_context, created_by_session, created_trace_id,
              reference_url, source, metadata, type)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,'recurring_active',?16,?17,?18,?19,?20,?21,?22)",
            params![
                id, task.agent_id, task.team_run_id, task.parent_task_id,
                task.depth, task.label, task.trigger_type, task.cron_expr,
                task.event_source, task.event_offset_secs, task.condition_expr,
                task.next_fire_at, task.timeout_at, task.action_type,
                task.action_config, task.input_context, task.created_by_session,
                task.created_trace_id, task.reference_url, task.source,
                task.metadata, task_type
            ],
        )?;
        if n > 0 {
            // mika#2337 — the registration went through; if an unknown-trigger
            // marker is what let it through, spend it now. Conditioned on
            // `!lift_already_spent` so a re-registration that never needed the
            // exemption cannot silently burn a fresh one.
            if !lift_already_spent
                && let Err(e) = self.spend_unknown_trigger_lift(&task.agent_id, &task.label)
            {
                // Fail-open on the *bookkeeping*, never on the guard: the row is
                // already registered and the engine is running again. An unspent
                // marker costs at most one extra lift on the next death;
                // refusing the registration here would restore the outage this
                // whole path exists to end.
                tracing::warn!(
                    agent_id = %task.agent_id,
                    label = %task.label,
                    error = %e,
                    "mika#2337: failed to spend the unknown-trigger veto lift — \
                     the next unknown-trigger death on this label may be \
                     forgiven a second time"
                );
            }
            Ok(Some(id))
        } else {
            Ok(None) // already existed (unique-index conflict on an active row)
        }
    }

    /// mika#2337 — consume the unknown-trigger exemption for `(agent_id, label)`.
    ///
    /// Removes [`RECURRING_UNKNOWN_TRIGGER_PATH`] from every row that carries it
    /// and writes [`RECURRING_UNKNOWN_TRIGGER_LIFT_CONSUMED_PATH`] in the same
    /// statement. Returns the number of rows whose marker was spent.
    ///
    /// **Why both halves.** Removing the marker alone does not bound anything:
    /// a second death writes a *fresh* marker on a *fresh* row and would absolve
    /// itself. The `lift_consumed` marker is what the next call reads to find
    /// the exemption already spent — and because it is never given a new
    /// `updated_at`, it ages out of the grace window together with the death it
    /// absolved, so the exemption re-arms rather than being lost permanently.
    fn spend_unknown_trigger_lift(&self, agent_id: &str, label: &str) -> Result<usize> {
        let n = self.conn.execute(
            "UPDATE tasks
             SET metadata = json_set(
                     json_remove(
                         CASE WHEN json_valid(metadata) THEN metadata ELSE '{}' END,
                         ?3),
                     ?4, 1)
             WHERE agent_id = ?1 AND label = ?2 COLLATE NOCASE
               AND trigger_type = 'recurring'
               AND json_valid(metadata)
               AND COALESCE(json_extract(metadata, ?3), 0) = 1",
            params![
                agent_id,
                label,
                RECURRING_UNKNOWN_TRIGGER_PATH,
                RECURRING_UNKNOWN_TRIGGER_LIFT_CONSUMED_PATH
            ],
        )?;
        Ok(n)
    }

    /// Get the cron expression for an existing recurring task by label.
    pub fn get_recurring_task_cron(&self, agent_id: &str, label: &str) -> Result<Option<String>> {
        let cron: Option<Option<String>> = self.conn.query_row(
            "SELECT cron_expr FROM tasks WHERE agent_id = ?1 AND label = ?2 AND trigger_type = 'recurring' AND status IN ('recurring_active', 'pending', 'in_progress') LIMIT 1",
            params![agent_id, label],
            |r| r.get(0),
        ).optional()?;
        Ok(cron.flatten())
    }

    /// Update the cron expression and next_fire_at for an existing recurring task.
    pub fn update_recurring_task_cron(
        &self,
        agent_id: &str,
        label: &str,
        new_cron: &str,
        next_fire_at: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET cron_expr = ?1, next_fire_at = ?2
             WHERE agent_id = ?3 AND label = ?4 AND trigger_type = 'recurring'
               AND status IN ('recurring_active', 'pending', 'in_progress')",
            params![new_cron, next_fire_at, agent_id, label],
        )?;
        Ok(())
    }

    /// Mark every `cancelled` recurring row for `(agent_id, label)` as a
    /// *reverted config cancel* (mika#2271). Returns the number of rows marked.
    ///
    /// Called from `ensure_recurring_task` — whose invocation *is* the config
    /// declaring the task must run — right before re-registration. Without it,
    /// the knob-off → knob-on cycle leaves the feeder dead: the knob-off boot
    /// cancels the row, and the knob-on boot hits the mika#1742 refuse-to-zombie
    /// guard, which cannot tell a deliberate config cancel from a terminal
    /// failure.
    ///
    /// Deliberately does **not** touch `updated_at`: the row keeps the timestamp
    /// of its actual cancel so the audit trail stays truthful. Exemption is
    /// carried by the metadata marker, not by ageing the row out of the grace
    /// window. Non-JSON `metadata` is replaced by a fresh object rather than
    /// erroring — the marker matters more than a malformed legacy blob.
    pub fn revert_config_cancel_recurring_task(
        &self,
        agent_id: &str,
        label: &str,
    ) -> Result<usize> {
        let n = self.conn.execute(
            "UPDATE tasks
             SET metadata = json_set(
                     CASE WHEN json_valid(metadata) THEN metadata ELSE '{}' END,
                     ?3, 1)
             WHERE agent_id = ?1 AND label = ?2 COLLATE NOCASE
               AND trigger_type = 'recurring'
               AND status = 'cancelled'
               AND NOT (json_valid(metadata)
                        AND COALESCE(json_extract(metadata, ?3), 0) = 1)",
            params![agent_id, label, RECURRING_CONFIG_CANCEL_REVERTED_PATH],
        )?;
        Ok(n)
    }

    /// Mark a recurring task's row as having died on an **unknown trigger**
    /// (mika#2337). Returns the number of rows marked (0 or 1).
    ///
    /// Called from `TaskEngine::fire_task` on the
    /// [`crate::task_engine::DispatchError::UnknownTrigger`] arm, *before*
    /// `update_task_failed`, so the row is never a plain unmarked corpse — not
    /// even for the instant between the two writes.
    ///
    /// Writes the integer `1`, not the string `"1"`: the reader compares with
    /// `COALESCE(json_extract(metadata, ?), 0) = 1`, and SQLite orders INTEGER
    /// before TEXT, so `'1' = 1` is false. A string would produce a marker that
    /// is visible in the row and invisible to the guard — the worst shape.
    ///
    /// Deliberately does **not** touch `updated_at`, mirroring
    /// [`Database::revert_config_cancel_recurring_task`]: the row keeps the
    /// timestamp of its actual death so the audit trail stays truthful, and the
    /// exemption is carried by the marker rather than by ageing the row out of
    /// the grace window. Non-JSON `metadata` is replaced by a fresh object
    /// rather than erroring — the marker matters more than a malformed legacy
    /// blob.
    pub fn mark_recurring_unknown_trigger(&self, task_id: &str) -> Result<usize> {
        let n = self.conn.execute(
            "UPDATE tasks
             SET metadata = json_set(
                     CASE WHEN json_valid(metadata) THEN metadata ELSE '{}' END,
                     ?2, 1)
             WHERE id = ?1 AND trigger_type = 'recurring'",
            params![task_id, RECURRING_UNKNOWN_TRIGGER_PATH],
        )?;
        Ok(n)
    }

    /// Cancel a recurring task by label (e.g. when reflection is disabled in identity.toml).
    pub fn cancel_recurring_task_by_label(&self, agent_id: &str, label: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET status = 'cancelled', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE agent_id = ?1 AND label = ?2 AND trigger_type = 'recurring'
               AND status NOT IN ('completed','failed','cancelled','expired')",
            params![agent_id, label],
        )?;
        Ok(())
    }

    /// Cancel active recurring tasks for agents no longer on disk (mika#1436).
    ///
    /// Called once at startup after the filesystem walk populates the agent set.
    /// Cancels (not deletes) so the audit trail via audit_events is preserved.
    /// Returns the list of (task_id, agent_id) pairs for operator observability.
    ///
    /// Agent-unscoped: operates across all agents in the DB, not filtered by
    /// this `Database` instance's implicit agent context.
    pub fn cancel_orphan_recurring_tasks(
        &self,
        known_agent_ids: &[String],
    ) -> Result<Vec<(String, String)>> {
        if known_agent_ids.is_empty() {
            return Ok(vec![]);
        }

        let tx = self.conn.unchecked_transaction()?;

        // Build a parameterized placeholder list for the NOT IN clause.
        let placeholders: Vec<String> = (1..=known_agent_ids.len())
            .map(|i| format!("?{}", i))
            .collect();
        let placeholders_str = placeholders.join(", ");

        // SELECT orphan tasks first for logging.
        let select_sql = format!(
            "SELECT id, agent_id FROM tasks
             WHERE trigger_type = 'recurring'
               AND status IN ('pending', 'recurring_active', 'in_progress')
               AND agent_id NOT IN ({placeholders_str})"
        );

        let params: Vec<&dyn rusqlite::types::ToSql> = known_agent_ids
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        let orphans: Vec<(String, String)> = {
            let mut stmt = tx.prepare(&select_sql)?;
            stmt.query_map(params.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        if orphans.is_empty() {
            tx.commit()?;
            return Ok(vec![]);
        }

        // UPDATE to cancelled.
        let update_sql = format!(
            "UPDATE tasks SET status = 'cancelled', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE trigger_type = 'recurring'
               AND status IN ('pending', 'recurring_active', 'in_progress')
               AND agent_id NOT IN ({placeholders_str})"
        );
        tx.execute(&update_sql, params.as_slice())?;

        tx.commit()?;
        Ok(orphans)
    }

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

    pub fn get_task(&self, id: &str, agent_id: &str) -> Result<Option<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks WHERE id = ?1 AND agent_id = ?2",
            Self::TASK_COLUMNS
        );
        self.conn
            .query_row(&sql, params![id, agent_id], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// Resolve a task ID prefix to matching full task IDs, scoped to the given agent.
    /// Returns up to 10 matching IDs for ambiguity reporting.
    pub fn resolve_task_id_by_prefix(&self, prefix: &str, agent_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM tasks WHERE id LIKE ?1 || '%' AND agent_id = ?2 ORDER BY id LIMIT 10",
        )?;
        let ids: Vec<String> = stmt
            .query_map(params![prefix, agent_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Get a manual (task) task by ID, scoped to the given agent.
    pub fn get_manual_task(&self, id: &str, agent_id: &str) -> Result<Option<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks WHERE id = ?1 AND agent_id = ?2 AND trigger_type = 'manual'",
            Self::TASK_COLUMNS
        );
        self.conn
            .query_row(&sql, params![id, agent_id], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// Walk the `parent_task_id` chain to the nearest scope root — a task with
    /// `type IN ('issue', 'milestone', 'project')`. Returns `None` if no scope
    /// ancestor exists, the starting task is not found, or the chain exceeds the
    /// depth limit (mika#974).
    ///
    /// The depth limit of 20 gives 6× headroom above the deployed maximum of
    /// N=3 (project → milestone → issue) while bounding worst-case walk cost.
    pub fn resolve_scope_root_task_id(&self, task_id: &str) -> Result<Option<String>> {
        /// Maximum parent-chain hops before giving up. Deployed task hierarchies
        /// never exceed N=3 today (project → milestone → issue). 20 gives 6×
        /// headroom against pathological chains.
        const SCOPE_ROOT_WALK_DEPTH_LIMIT: usize = 20;
        const SCOPE_TYPES: &[&str] = &[TASK_TYPE_ISSUE, TASK_TYPE_MILESTONE, TASK_TYPE_PROJECT];

        let mut current_id = task_id.to_owned();

        for _ in 0..SCOPE_ROOT_WALK_DEPTH_LIMIT {
            let row: Option<(String, String, Option<String>)> = self
                .conn
                .query_row(
                    "SELECT type, trigger_type, parent_task_id FROM tasks WHERE id = ?1",
                    params![current_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;

            match row {
                // A manual task with a scope type is the scope root.
                // Callback/recurring tasks are not scope roots even if typed as 'issue'.
                Some((task_type, trigger_type, _))
                    if SCOPE_TYPES.contains(&task_type.as_str()) && trigger_type == "manual" =>
                {
                    return Ok(Some(current_id));
                }
                Some((_, _, Some(parent_id))) => {
                    current_id = parent_id;
                }
                // Task not a scope root and no parent — chain exhausted.
                Some((_, _, None)) | None => return Ok(None),
            }
        }

        // Depth limit exceeded — likely a circular chain.
        warn!(
            task_id = task_id,
            limit = SCOPE_ROOT_WALK_DEPTH_LIMIT,
            "scope_root_walk_depth_limit_exceeded"
        );
        Ok(None)
    }

    /// Get IDs of pending callback tasks for a given session.
    ///
    /// Used by `mika ask` to detect background tasks that were spawned during the
    /// agent loop but won't be consumed until TUI or server starts. See #265.
    pub fn get_pending_callbacks_for_session(&self, session_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM tasks
             WHERE created_by_session = ?1
               AND trigger_type = 'callback'
               AND status = 'pending'
             ORDER BY created_at ASC",
        )?;
        let ids = stmt
            .query_map(params![session_id], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(ids)
    }

    /// Count child tasks for a given parent task (manual tasks only).
    pub fn count_child_tasks(
        &self,
        parent_task_id: &str,
        agent_id: &str,
    ) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT status, COUNT(*) FROM tasks
             WHERE parent_task_id = ?1 AND agent_id = ?2 AND trigger_type = 'manual'
             GROUP BY status",
        )?;
        let rows = stmt.query_map(params![parent_task_id, agent_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn get_schedulable_tasks(&self, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1 AND status IN ('pending','recurring_active')
               AND trigger_type NOT IN ('callback', 'manual')
             ORDER BY next_fire_at ASC NULLS LAST",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn update_task_status(&self, id: &str, status: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET status = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
            params![status, id],
        )?;
        Ok(())
    }

    /// Record the trace_id of the execution that ran this task.
    /// Deliberately does NOT scope by agent_id — the dispatcher may write
    /// execution_trace_id for tasks owned by different agents (cross-agent team tasks).
    pub fn update_task_execution_trace_id(&self, id: &str, trace_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET execution_trace_id = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
            params![trace_id, id],
        )?;
        Ok(())
    }

    /// Atomically claim a task and record its fired_at in a single UPDATE.
    /// Returns true if the task was claimed (was in 'pending' or 'recurring_active' state).
    /// Returns false if the task was already claimed, cancelled, or completed.
    pub fn claim_and_fire_task(&self, id: &str, agent_id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE tasks SET status = 'in_progress',
                              fired_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                              updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1 AND agent_id = ?2 AND status IN ('pending', 'recurring_active')",
            params![id, agent_id],
        )?;
        Ok(n > 0)
    }

    pub fn update_task_completed(
        &self,
        id: &str,
        agent_id: &str,
        result: Option<&str>,
    ) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE tasks SET status = 'completed', result = ?1,
             completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2 AND agent_id = ?3 AND status IN ('pending', 'in_progress')",
            params![result, id, agent_id],
        )?;
        Ok(rows > 0)
    }

    pub fn update_task_failed(&self, id: &str, agent_id: &str, error: &str) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE tasks SET status = 'failed', result = ?1,
             completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2 AND agent_id = ?3
             AND status NOT IN ('completed', 'failed', 'cancelled', 'expired', 'delivered')",
            params![error, id, agent_id],
        )?;
        Ok(rows > 0)
    }

    /// Update the dispatch class of a task (#1001).
    ///
    /// Used when a task transitions between grooming and implementation phases
    /// (e.g., after dev-groom completes and dev-pilot is about to dispatch on
    /// the same task_id per mika#996's task-reuse pattern). Idempotent —
    /// setting the same class is a no-op (updated_at still advances).
    pub fn update_task_dispatch_class(
        &self,
        id: &str,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE tasks SET dispatch_class = ?1,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2 AND agent_id = ?3",
            params![dispatch_class, id, agent_id],
        )?;
        Ok(rows > 0)
    }

    /// Write a dispatch-rejection reason to `tasks.result` without changing status (#1108).
    ///
    /// Used by `validate_dispatch_readiness()` to surface rejection reasons to
    /// operator-visible surfaces (`tasks.result` column). The task's status is
    /// preserved — only `result` and `updated_at` are modified. Returns `true`
    /// if the row was updated. Agent-unscoped because the caller may not know
    /// the agent_id (e.g., the unauthorized-webhook check fires before task fetch).
    pub fn write_task_dispatch_rejection(&self, id: &str, reason_json: &str) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE tasks SET result = ?1,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2 AND trigger_type = 'manual'",
            params![reason_json, id],
        )?;
        Ok(rows > 0)
    }

    /// Promote a task from `failed` → `completed` (#958).
    ///
    /// Symmetric to `update_task_failed()`. Only transitions tasks currently
    /// in `failed` status — guarded WHERE clause prevents promotion from any
    /// other state. Returns `true` if the transition happened.
    pub fn promote_task_completed(&self, id: &str, agent_id: &str, reason: &str) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE tasks SET status = 'completed', result = ?1,
             completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2 AND agent_id = ?3
             AND status = 'failed'",
            params![reason, id, agent_id],
        )?;
        Ok(rows > 0)
    }

    pub fn update_task_next_fire_at(&self, id: &str, next_fire_at: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET next_fire_at = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
            params![next_fire_at, id],
        )?;
        Ok(())
    }

    /// Atomically reschedule a recurring task: set next_fire_at and status = 'recurring_active'
    /// in a single UPDATE, replacing two sequential writes.
    pub fn update_task_rescheduled(&self, id: &str, next_fire_at: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET next_fire_at = ?1, status = 'recurring_active', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
            params![next_fire_at, id],
        )?;
        Ok(())
    }

    pub fn cancel_task(&self, id: &str, agent_id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE tasks SET status = 'cancelled', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1 AND agent_id = ?2 AND status NOT IN ('completed','failed','cancelled','expired','delivered')",
            params![id, agent_id],
        )?;
        if n > 0 {
            // Cascade: cancel active callback children (mika#1011 Phase 0.7).
            // Prevents orphan-pending deferred-dispatch callbacks (and closes a
            // latent gap for immediate callbacks too). Non-callback children
            // (e.g., manual sub-tasks) are intentionally left untouched.
            self.conn.execute(
                "UPDATE tasks SET status = 'cancelled', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                 WHERE parent_task_id = ?1 AND agent_id = ?2
                   AND trigger_type = 'callback'
                   AND status IN ('pending', 'in_progress')",
                params![id, agent_id],
            )?;
        }
        Ok(n > 0)
    }

    /// Update the status of a manual (task) task. Free transitions allowed.
    /// Sets `completed_at` when transitioning to `completed`.
    /// Returns the old status for audit logging.
    pub fn update_manual_task_status(
        &self,
        task_id: &str,
        agent_id: &str,
        new_status: &str,
    ) -> Result<Option<String>> {
        let old_status: Option<String> = self
            .conn
            .query_row(
                "SELECT status FROM tasks WHERE id = ?1 AND agent_id = ?2 AND trigger_type = 'manual'",
                params![task_id, agent_id],
                |r| r.get(0),
            )
            .optional()?;

        let Some(old) = &old_status else {
            return Ok(None);
        };

        if old == new_status {
            return Ok(Some(old.clone()));
        }

        self.conn.execute(
            "UPDATE tasks SET status = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                    completed_at = CASE WHEN ?1 = 'completed' THEN strftime('%Y-%m-%dT%H:%M:%SZ', 'now') ELSE NULL END
             WHERE id = ?2 AND agent_id = ?3 AND trigger_type = 'manual'",
            params![new_status, task_id, agent_id],
        )?;

        Ok(old_status)
    }

    /// Transition a **parent tracking row** to `in_progress` on dispatch **and**
    /// stamp `fired_at` (mika#2335, facette 2).
    ///
    /// SOLE WRITER of the parent-side dispatch transition. The three production
    /// dispatch paths — `skills::executor::execute_long_running` (#525, the
    /// original), `server::ready_label_handler` and `server::verdict_handler`
    /// (both of which say in their own comments that they *mirror* the first) —
    /// call this instead of `update_manual_task_status(…, "in_progress")`.
    ///
    /// **The defect this closes.** `set_task_process_id` stamps `fired_at`
    /// (mika#2263 défaut (b)), and it is the sole writer of `process_id` — but
    /// its only production caller writes the **callback child**. The parent
    /// tracking row, the one every operator surface reads (`mika tasks`, the
    /// dashboard, the health probes), went through `update_manual_task_status`,
    /// which writes `status`, `updated_at` and `completed_at` and nothing else.
    /// So a live dispatch's parent read `fired_at = NULL` — "never fired" — for
    /// the whole life of the pilot. On 2026-09-15 an operator read exactly that
    /// and cancelled a pilot that had been running for 38 minutes.
    ///
    /// **Why a dedicated writer and not a `CASE` added to
    /// [`Self::update_manual_task_status`].** That method also serves
    /// `rewind.rs`, which *restores a prior status* and must stamp nothing — a
    /// rewind is not a dispatch. Naming the dispatch transition separates the
    /// two intents at the call site instead of relying on a conditional that a
    /// future caller would inherit silently.
    ///
    /// **An existing `fired_at` is never overwritten**, same clause and same
    /// reason as `set_task_process_id`: the reapers measure a dispatch's age
    /// from it, and a re-stamp would reset that age under them.
    ///
    /// Returns the prior status (`None` when no such manual row exists), so
    /// callers keep the non-fatal `warn!`-and-continue shape they already had —
    /// the stamp is observability and must never fail a dispatch.
    pub fn mark_parent_dispatched(&self, task_id: &str, agent_id: &str) -> Result<Option<String>> {
        let old_status: Option<String> = self
            .conn
            .query_row(
                "SELECT status FROM tasks WHERE id = ?1 AND agent_id = ?2 AND trigger_type = 'manual'",
                params![task_id, agent_id],
                |r| r.get(0),
            )
            .optional()?;

        if old_status.is_none() {
            return Ok(None);
        }

        // Unlike `update_manual_task_status`, an unchanged status is NOT an
        // early return: a row already `in_progress` whose dispatch is only now
        // firing still needs its `fired_at`. Skipping the write there would
        // reproduce the defect on every path that transitions before it
        // dispatches.
        //
        // **The `status IN ('pending','in_progress')` guard is load-bearing,
        // and it is the one place this method is STRICTER than the free
        // transition it replaces.** Two of the three call sites observe the
        // row's status and then do real work before stamping —
        // `ready_label_handler` creates the callback child and stats the
        // handler script in between, `verdict_handler` likewise — and a
        // concurrent supersession for the same `reference_url` is a designed-
        // for event on exactly that path (`supersede_prior_tracking_rows` runs
        // at the top of the same handler). Without the guard, a dispatch could
        // resurrect a parent another dispatch had just cancelled, putting two
        // live dispatches back on one ticket through the bookkeeping instead of
        // through the missing kill. A refusal writes nothing and is silent by
        // design: the callers already treat this whole call as non-fatal, and
        // the prior status is returned either way.
        self.conn.execute(
            "UPDATE tasks
                SET status = 'in_progress',
                    fired_at = CASE
                                 WHEN fired_at IS NULL
                                 THEN strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                                 ELSE fired_at
                               END,
                    completed_at = NULL,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
              WHERE id = ?1 AND agent_id = ?2 AND trigger_type = 'manual'
                AND status IN ('pending', 'in_progress')",
            params![task_id, agent_id],
        )?;

        Ok(old_status)
    }

    /// Number of columns in TASK_COLUMNS (used for child_count ordinal in list_manual_tasks).
    /// Bumped to 32 in mika#1948 for `dispatcher_source`.
    const TASK_COLUMN_COUNT: usize = 32;

    /// List manual (task) tasks for an agent with optional filters.
    /// Uses parameterized NULL checks to avoid dynamic SQL construction.
    pub fn list_manual_tasks(
        &self,
        agent_id: &str,
        status_filter: Option<&str>,
        source_filter: Option<&str>,
        include_children: bool,
    ) -> Result<Vec<(Task, Option<i64>)>> {
        let child_expr = if include_children {
            "(SELECT COUNT(*) FROM tasks c WHERE c.parent_task_id = t.id)"
        } else {
            "NULL"
        };

        let sql = format!(
            "SELECT {columns}, {child_expr} AS child_count
             FROM tasks t
             WHERE t.agent_id = ?1 AND t.trigger_type = 'manual'
               AND (?2 IS NULL OR t.status = ?2)
               AND (?3 IS NULL OR t.source = ?3)
             ORDER BY t.created_at DESC LIMIT 50",
            columns = Self::TASK_COLUMNS
                .split(", ")
                .map(|c| format!("t.{c}"))
                .collect::<Vec<_>>()
                .join(", "),
        );

        let status_param: Option<String> = status_filter.map(|s| s.to_string());
        let source_param: Option<String> = source_filter.map(|s| s.to_string());

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id, status_param, source_param], |r| {
                let task = Self::row_to_task(r)?;
                // child_count is at ordinal Self::TASK_COLUMN_COUNT (one past last task column)
                let child_count: Option<i64> = r.get(Self::TASK_COLUMN_COUNT)?;
                Ok((task, child_count))
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

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

    /// Count **active** agent-created tasks in a session (for per-session cap enforcement).
    /// Only pending/in_progress/blocked items count — completed/cancelled/failed/delivered
    /// items are terminal and should not block new task creation (sprint mode).
    /// Scoped to agent_id for defense-in-depth.
    pub fn count_session_tasks(&self, agent_id: &str, session_id: &str) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1 AND created_by_session = ?2 AND trigger_type = 'manual'
               AND (source IS NULL OR source != 'user_request')
               AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')",
            params![agent_id, session_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    /// Find an active manual task by agent_id and reference_url.
    /// Used for dedup when `create_task` is called with a reference_url that
    /// already has an active (non-terminal) task.
    pub fn find_active_task_by_ref_url(
        &self,
        agent_id: &str,
        reference_url: &str,
    ) -> Result<Option<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1 AND reference_url = ?2
               AND trigger_type = 'manual'
               AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')
             LIMIT 1",
            Self::TASK_COLUMNS
        );
        self.conn
            .query_row(&sql, params![agent_id, reference_url], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// Find an active manual task by agent_id and PR URL stored in metadata.
    /// Looks up `json_extract(metadata, '$.claude_pilot.pr_url')` for matching.
    /// Used to locate the parent task when a PR review verdict arrives.
    pub fn find_active_task_by_pr_url(&self, agent_id: &str, pr_url: &str) -> Result<Option<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND json_extract(metadata, '$.claude_pilot.pr_url') = ?2
               AND trigger_type = 'manual'
               AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')
             LIMIT 1",
            Self::TASK_COLUMNS
        );
        self.conn
            .query_row(&sql, params![agent_id, pr_url], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// Find an active manual task by agent_id and branch stored in metadata.
    /// Looks up `json_extract(metadata, '$.claude_pilot.branch')` for matching.
    /// Used to locate the parent task when a PR webhook arrives before
    /// the PR URL has been recorded (in-flight tasks only have a branch).
    pub fn find_active_task_by_branch(&self, agent_id: &str, branch: &str) -> Result<Option<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND json_extract(metadata, '$.claude_pilot.branch') = ?2
               AND trigger_type = 'manual'
               AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')
             LIMIT 1",
            Self::TASK_COLUMNS
        );
        self.conn
            .query_row(&sql, params![agent_id, branch], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// Find an active manual task by agent_id and label (case-insensitive).
    /// Used as a fallback dedup path when `create_task` is called without a reference_url.
    pub fn find_active_task_by_label(&self, agent_id: &str, label: &str) -> Result<Option<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1 AND label = ?2 COLLATE NOCASE
               AND trigger_type = 'manual'
               AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')
             LIMIT 1",
            Self::TASK_COLUMNS
        );
        self.conn
            .query_row(&sql, params![agent_id, label], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// Find active phantom-shape tracking rows for an issue's base URL AND its
    /// `?phase=groom` variant (mika#1934 AC2.2 / AC4.b).
    ///
    /// Returns rows in the phantom shape — `trigger_type='manual'`,
    /// `action_type='none'`, `process_id IS NULL`, `status IN ('blocked',
    /// 'in_progress')` — whose `reference_url` is either the exact `base_url` or
    /// `<base_url>?phase=groom`. `base_url` MUST be canonical (no `?phase=groom`
    /// suffix); callers strip it via
    /// [`crate::task_state::tasks::strip_groom_phase_suffix`].
    ///
    /// A single helper serves both cleanup surfaces: supersede-on-new-dispatch
    /// (AC2) and complete-on-upstream-close (AC4) both need the same
    /// exact-URL + groom-variant fan-out. Reuses the partial unique index
    /// `idx_tasks_manual_active_ref_url` (`blocked`/`in_progress` are inside the
    /// index's "active" set) — no new migration (AC2.1).
    pub fn find_active_tracking_rows_by_reference_url_and_variants(
        &self,
        agent_id: &str,
        base_url: &str,
    ) -> Result<Vec<Task>> {
        let groom_url = format!("{base_url}{}", crate::task_state::tasks::GROOM_PHASE_SUFFIX);
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'manual'
               AND action_type = 'none'
               AND process_id IS NULL
               AND status IN ('blocked', 'in_progress')
               AND reference_url IN (?2, ?3)
             ORDER BY id",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id, base_url, groom_url], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // mika#2335 — `find_live_dispatch_rows_by_reference_url_and_variants` lived
    // here and is deleted, not left in place. Its WHERE clause was
    // `process_id IS NOT NULL AND reference_url IN (?2, ?3)`, a conjunction
    // **empty on the topology production writes**: the `reference_url` is on
    // the parent tracking row, which never carries a `process_id`; the pgid is
    // on the callback child, which never carries a `reference_url`. It could
    // not return a row for any normal dispatch — not in the mika#2263 incident,
    // in none. A query that can return nothing and stays in the file will one
    // day be read as a guarantee. The traversal that does work already existed
    // and is tested: [`Self::find_dispatch_children_with_pid`].

    /// Guarded transition of a phantom tracking row → `cancelled` with the
    /// canonical supersede reason (mika#1934 AC2).
    ///
    /// SOLE WRITER of `result = 'superseded_by_new_dispatch'`
    /// ([`crate::task_state::tasks::SUPERSEDED_BY_NEW_DISPATCH`]). Only
    /// transitions from the phantom shape (blocked/in_progress,
    /// `action_type='none'`, `process_id IS NULL`), so a caller passing a
    /// genuine dispatched row or a `pending` row gets `false` and no write.
    /// Returns `true` iff a row transitioned.
    pub fn cancel_task_superseded(&self, id: &str, agent_id: &str) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE tasks SET status = 'cancelled', result = ?1,
             completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2 AND agent_id = ?3
               AND action_type = 'none'
               AND process_id IS NULL
               AND status IN ('blocked', 'in_progress')",
            params![
                crate::task_state::tasks::SUPERSEDED_BY_NEW_DISPATCH,
                id,
                agent_id
            ],
        )?;
        Ok(rows > 0)
    }

    /// Guarded terminal transition of a phantom tracking row on upstream close
    /// (mika#1934 AC4).
    ///
    /// `new_status` is `'cancelled'` or `'completed'`; `result` is one of the
    /// upstream-close reason constants
    /// ([`crate::task_state::tasks::ISSUE_CLOSED_UPSTREAM`],
    /// [`crate::task_state::tasks::UPSTREAM_PR_MERGED`],
    /// [`crate::task_state::tasks::UPSTREAM_PR_CLOSED_UNMERGED`]). Same
    /// phantom-shape guard as [`Self::cancel_task_superseded`]. Idempotent: an
    /// already-terminal row matches nothing (the `status IN (...)` guard
    /// short-circuits on re-delivery) and returns `false` (AC4.c). Returns
    /// `true` iff transitioned.
    pub fn terminal_mark_tracking_row_upstream_closed(
        &self,
        id: &str,
        agent_id: &str,
        new_status: &str,
        result: &str,
    ) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE tasks SET status = ?1, result = ?2,
             completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?3 AND agent_id = ?4
               AND action_type = 'none'
               AND process_id IS NULL
               AND status IN ('blocked', 'in_progress')",
            params![new_status, result, id, agent_id],
        )?;
        Ok(rows > 0)
    }

    /// Get the depth of a task by ID (for computing child depth).
    /// Scoped to agent_id to prevent cross-agent parent linking.
    pub fn get_task_depth(&self, task_id: &str, agent_id: &str) -> Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT depth FROM tasks WHERE id = ?1 AND agent_id = ?2",
                params![task_id, agent_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// List pending/in_progress/blocked manual tasks for prompt injection (heartbeat awareness).
    pub fn list_active_tasks(&self, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1 AND trigger_type = 'manual'
               AND status IN ('pending', 'in_progress', 'blocked')
             ORDER BY created_at DESC LIMIT 10",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Build a task health summary for heartbeat prompt injection.
    ///
    /// Returns active manual tasks plus anomalous task states across all trigger types.
    /// Anomalies are capped at [`crate::task_engine::types::health_thresholds::MAX_ANOMALIES`].
    pub fn get_task_health_summary(&self, agent_id: &str) -> Result<TaskHealthSummary> {
        use crate::task_engine::types::health_thresholds;

        let active_tasks = self.list_active_tasks(agent_id)?;
        let now = Utc::now();
        let mut anomalies: Vec<TaskHealthAnomaly> = Vec::new();

        // Helper: query anomaly rows and return them as TaskHealthAnomaly values.
        // `describe_age` receives the 5th SELECT column (a timestamp or ignored)
        // and returns the human-readable age_description for each anomaly.
        let query_anomalies = |sql: &str,
                               sql_params: &[&dyn rusqlite::types::ToSql],
                               anomaly_type: &str,
                               describe_age: &dyn Fn(&str) -> String|
         -> Result<Vec<TaskHealthAnomaly>> {
            let mut stmt = self.conn.prepare(sql)?;
            let rows = stmt.query_map(sql_params, |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            })?;
            let mut result = Vec::new();
            for row in rows {
                let (id, label, trigger_type, status, ts_col, reference_url) = row?;
                result.push(TaskHealthAnomaly {
                    task_id: id,
                    label,
                    trigger_type,
                    status,
                    anomaly_type: anomaly_type.to_string(),
                    age_description: describe_age(&ts_col),
                    reference_url,
                });
            }
            Ok(result)
        };

        // 1. Stuck callbacks: completed but not delivered for > threshold
        {
            let threshold = timestamp::format(
                &(now - Duration::seconds(health_thresholds::STUCK_CALLBACK_SECS)),
            );
            let limit = health_thresholds::MAX_ANOMALIES as i64;
            anomalies.extend(query_anomalies(
                "SELECT id, label, trigger_type, status, updated_at, reference_url
                 FROM tasks
                 WHERE agent_id = ?1
                   AND trigger_type = 'callback'
                   AND status = 'completed'
                   AND updated_at < ?2
                 ORDER BY updated_at ASC
                 LIMIT ?3",
                &[&agent_id, &threshold as &dyn rusqlite::types::ToSql, &limit],
                "stuck_callback",
                &|ts| format!("stuck {}", format_age(ts, now)),
            )?);
        }

        // 2. Failed recurring tasks
        {
            let since = timestamp::format(&(now - Duration::seconds(86_400)));
            let remaining = health_thresholds::MAX_ANOMALIES.saturating_sub(anomalies.len()) as i64;
            if remaining > 0 {
                anomalies.extend(query_anomalies(
                    "SELECT id, label, trigger_type, status, updated_at, reference_url
                     FROM tasks
                     WHERE agent_id = ?1
                       AND trigger_type = 'recurring'
                       AND status = 'failed'
                       AND updated_at > ?2
                     ORDER BY updated_at DESC
                     LIMIT ?3",
                    &[&agent_id, &since as &dyn rusqlite::types::ToSql, &remaining],
                    "failed_recurring",
                    &|ts| format!("failed {} ago", format_age(ts, now)),
                )?);
            }
        }

        // 3. Long-running in_progress tasks
        {
            let threshold = timestamp::format(
                &(now - Duration::seconds(health_thresholds::LONG_RUNNING_DEFAULT_SECS)),
            );
            let remaining = health_thresholds::MAX_ANOMALIES.saturating_sub(anomalies.len()) as i64;
            if remaining > 0 {
                anomalies.extend(query_anomalies(
                    "SELECT id, label, trigger_type, status, fired_at, reference_url
                     FROM tasks
                     WHERE agent_id = ?1
                       AND status = 'in_progress'
                       AND trigger_type != 'manual'
                       AND fired_at IS NOT NULL
                       AND fired_at < ?2
                     ORDER BY fired_at ASC
                     LIMIT ?3",
                    &[
                        &agent_id,
                        &threshold as &dyn rusqlite::types::ToSql,
                        &remaining,
                    ],
                    "long_running",
                    &|ts| format!("running for {}", format_age(ts, now)),
                )?);
            }
        }

        // 4. Stale blocked manual tasks
        {
            let threshold = timestamp::format(
                &(now - Duration::seconds(health_thresholds::STALE_BLOCKED_SECS)),
            );
            let remaining = health_thresholds::MAX_ANOMALIES.saturating_sub(anomalies.len()) as i64;
            if remaining > 0 {
                anomalies.extend(query_anomalies(
                    "SELECT id, label, trigger_type, status, updated_at, reference_url
                     FROM tasks
                     WHERE agent_id = ?1
                       AND trigger_type = 'manual'
                       AND status = 'blocked'
                       AND updated_at < ?2
                     ORDER BY updated_at ASC
                     LIMIT ?3",
                    &[
                        &agent_id,
                        &threshold as &dyn rusqlite::types::ToSql,
                        &remaining,
                    ],
                    "stale_blocked",
                    &|ts| format!("blocked for {}", format_age(ts, now)),
                )?);
            }
        }

        // 5. Stale pending manual tasks with no callback child (#583)
        {
            let threshold = timestamp::format(
                &(now - Duration::seconds(health_thresholds::STALE_PENDING_SECS)),
            );
            let remaining = health_thresholds::MAX_ANOMALIES.saturating_sub(anomalies.len()) as i64;
            if remaining > 0 {
                anomalies.extend(query_anomalies(
                    "SELECT t.id, t.label, t.trigger_type, t.status, t.created_at, t.reference_url
                     FROM tasks t
                     WHERE t.agent_id = ?1
                       AND t.trigger_type = 'manual'
                       AND t.status = 'pending'
                       AND t.created_at < ?2
                       AND NOT EXISTS (
                           SELECT 1 FROM tasks c
                           WHERE c.parent_task_id = t.id
                             AND c.trigger_type = 'callback'
                       )
                     ORDER BY t.created_at ASC
                     LIMIT ?3",
                    &[
                        &agent_id,
                        &threshold as &dyn rusqlite::types::ToSql,
                        &remaining,
                    ],
                    "stale_pending",
                    &|ts| format!("pending for {}", format_age(ts, now)),
                )?);
            }
        }

        // 6. GitHub-linked manual tasks (active, with reference_url containing github.com)
        {
            let remaining = health_thresholds::MAX_ANOMALIES.saturating_sub(anomalies.len()) as i64;
            if remaining > 0 {
                anomalies.extend(query_anomalies(
                    "SELECT id, label, trigger_type, status, created_at, reference_url
                     FROM tasks
                     WHERE agent_id = ?1
                       AND trigger_type = 'manual'
                       AND status IN ('pending', 'in_progress')
                       AND reference_url LIKE '%github.com%'
                     ORDER BY created_at DESC
                     LIMIT ?2",
                    &[&agent_id, &remaining as &dyn rusqlite::types::ToSql],
                    "github_linked",
                    &|_| "has linked GitHub PR".to_string(),
                )?);
            }
        }

        // 7. Dispatch failures: dual-signal wedge detection (#980)
        //    Signal A: >= THRESHOLD recent run_claude_pilot failures in the sliding window
        //    Signal B: stale dispatch — no run_claude_pilot attempt in > 1h while task is in_progress
        {
            let remaining = health_thresholds::MAX_ANOMALIES.saturating_sub(anomalies.len());
            if remaining > 0 {
                let window_start = timestamp::format(
                    &(now - Duration::seconds(health_thresholds::DISPATCH_FAILURE_WINDOW_SECS)),
                );
                let stale_threshold = timestamp::format(
                    &(now - Duration::seconds(health_thresholds::LONG_RUNNING_DEFAULT_SECS)),
                );

                // Signal A: Count recent failures with session→task JOIN for correlation
                let signal_a: Option<(u32, Option<String>, Option<String>)> = self
                    .conn
                    .prepare(
                        "SELECT
                            COUNT(*) as failure_count,
                            t.id as task_id,
                            t.label as task_label
                         FROM tool_calls tc
                         LEFT JOIN sessions s ON tc.session_id = s.id
                         LEFT JOIN tasks t ON s.task_id = t.id AND t.status = 'in_progress'
                         WHERE tc.agent_id = ?1
                           AND tc.tool_name = 'run_claude_pilot'
                           AND tc.success = 0
                           AND tc.created_at >= ?2
                         GROUP BY t.id
                         ORDER BY failure_count DESC
                         LIMIT 1",
                    )?
                    .query_row(params![agent_id, &window_start], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })
                    .ok();

                // Signal B: Stale dispatch — most recent run_claude_pilot attempt is older than 1h
                // while an in_progress manual task exists
                let signal_b: Option<(String, String)> = self
                    .conn
                    .prepare(
                        "SELECT t.id, t.label
                         FROM tasks t
                         WHERE t.agent_id = ?1
                           AND t.status = 'in_progress'
                           AND t.trigger_type = 'manual'
                           AND NOT EXISTS (
                               SELECT 1 FROM tool_calls tc2
                               WHERE tc2.agent_id = ?1
                                 AND tc2.tool_name = 'run_claude_pilot'
                                 AND tc2.created_at >= ?2
                           )
                         ORDER BY t.updated_at DESC
                         LIMIT 1",
                    )?
                    .query_row(params![agent_id, &stale_threshold], |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })
                    .ok();

                let mut dispatch_anomaly_fired = false;

                // Emit anomaly from Signal A (threshold met)
                if let Some((count, task_id, task_label)) = signal_a
                    && count >= health_thresholds::DISPATCH_FAILURE_THRESHOLD
                {
                    let (tid, tlabel) = match (task_id, task_label) {
                        (Some(id), Some(label)) => (id, label),
                        _ => (
                            agent_id.to_string(),
                            "run_claude_pilot dispatch".to_string(),
                        ),
                    };
                    anomalies.push(TaskHealthAnomaly {
                        task_id: tid,
                        label: tlabel,
                        trigger_type: "manual".to_string(),
                        status: "in_progress".to_string(),
                        anomaly_type: "dispatch_failures".to_string(),
                        age_description: format!("{} failures in last 2h", count),
                        reference_url: None,
                    });
                    dispatch_anomaly_fired = true;
                }

                // Emit anomaly from Signal B (stale dispatch) — only if Signal A didn't fire
                if !dispatch_anomaly_fired && let Some((task_id, task_label)) = signal_b {
                    anomalies.push(TaskHealthAnomaly {
                        task_id,
                        label: task_label,
                        trigger_type: "manual".to_string(),
                        status: "in_progress".to_string(),
                        anomaly_type: "dispatch_stale".to_string(),
                        age_description: "no dispatch attempt in >1h".to_string(),
                        reference_url: None,
                    });
                }
            }
        }

        // Cap total anomalies
        anomalies.truncate(health_thresholds::MAX_ANOMALIES); // safe-byte-slice: Vec<TaskHealthAnomaly> — element count, no char boundary

        Ok(TaskHealthSummary {
            active_tasks,
            anomalies,
        })
    }

    pub fn mark_tasks_expired(&self, now: &str, agent_id: &str) -> Result<usize> {
        let n = self.conn.execute(
            "UPDATE tasks SET status = 'expired', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE agent_id = ?2
               AND timeout_at IS NOT NULL AND timeout_at < ?1
               AND status NOT IN ('completed','failed','cancelled','expired','delivered')",
            params![now, agent_id],
        )?;
        Ok(n)
    }

    /// Get IDs of expired tasks whose parent is still pending (for sibling completion checks).
    pub fn get_expired_child_task_ids(&self, agent_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id FROM tasks t
             JOIN tasks p ON t.parent_task_id = p.id
             WHERE t.agent_id = ?1
               AND t.status = 'expired'
               AND t.parent_task_id IS NOT NULL
               AND p.status = 'pending'",
        )?;
        let ids = stmt
            .query_map(params![agent_id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    pub fn count_pending_tasks(&self, agent_id: &str) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1 AND status IN ('pending','in_progress','recurring_active')",
            params![agent_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    /// Count active self_dev tasks for mika-dev (mika#1363 F2).
    ///
    /// Returns the number of tasks that indicate mika-dev is not idle:
    /// - status='in_progress' (currently running) or status='pending' (awaiting dispatch)
    /// - source='self_dev' (excludes system tasks: heartbeat, reflection, recurring auto_pull)
    /// - Excludes 'completed', 'failed', 'blocked', 'cancelled' (terminal states)
    pub fn count_active_self_dev_tasks(&self, agent_id: &str) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1
               AND source = 'self_dev'
               AND status IN ('in_progress', 'pending')",
            params![agent_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    /// True if an active (pending/in_progress) self_dev task references this issue
    /// (mika#1824 D6). Used by the Phase 2 stuck-ready reconciler to skip tickets
    /// that already have in-flight work of their own.
    ///
    /// `issue_url` is the canonical issue URL (e.g.
    /// `https://github.com/senara-solutions/mika/issues/123`). The match is a
    /// prefix `LIKE` so the `?phase=groom` suffix variant is covered.
    pub fn has_active_self_dev_task_for_issue(
        &self,
        agent_id: &str,
        issue_url: &str,
    ) -> Result<bool> {
        let prefix = format!("{}%", issue_url);
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1
               AND source = 'self_dev'
               AND status IN ('pending', 'in_progress')
               AND reference_url LIKE ?2",
            params![agent_id, prefix],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    /// Get all pending user-visible tasks (reminders and callbacks, excludes heartbeat/reflection).
    /// Returns user-visible reminder tasks (both `send_message` and `resume_agent`).
    ///
    /// Intentionally excludes `trigger_type = 'callback'` tasks: those are system-internal
    /// tasks created by long-running exec handlers, not user-created reminders. Callback
    /// delivery is handled separately by `get_undelivered_callback_tasks()` (server mode)
    /// and `poll_callback_tasks()` (CLI mode). See #363.
    pub fn get_user_visible_tasks(&self, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND action_type IN ('send_message', 'resume_agent')
               AND trigger_type NOT IN ('callback')
               AND status IN ('pending', 'in_progress', 'recurring_active')
             ORDER BY next_fire_at ASC",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Split counts of active background tasks: executing (subprocess alive) vs queued (waiting).
    /// Uses `process_id IS NOT NULL` as the discriminator for executing tasks.
    /// Used by TUI footer badge to show `[1 running, 2 queued]` instead of `[3 running]`.
    pub fn get_background_task_counts(&self, agent_id: &str) -> Result<BackgroundTaskCounts> {
        let (executing, queued): (i64, i64) = self.conn.query_row(
            "SELECT
               COALESCE(SUM(CASE WHEN process_id IS NOT NULL THEN 1 ELSE 0 END), 0),
               COALESCE(SUM(CASE WHEN process_id IS NULL THEN 1 ELSE 0 END), 0)
             FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND action_type = 'resume_agent'
               AND status IN ('pending', 'in_progress')",
            params![agent_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(BackgroundTaskCounts {
            executing: executing as usize,
            queued: queued as usize,
        })
    }

    /// Count active background tasks (long-running callback tasks that are pending or in-progress).
    /// Convenience wrapper that sums executing + queued counts.
    pub fn get_active_background_task_count(&self, agent_id: &str) -> Result<usize> {
        let counts = self.get_background_task_counts(agent_id)?;
        Ok(counts.executing + counts.queued)
    }

    pub fn get_inject_context_tasks(&self, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1 AND action_type = 'inject_context'
               AND status IN ('pending', 'in_progress')
             ORDER BY next_fire_at ASC",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Get callback tasks that completed or failed but have not yet been delivered to the user.
    /// Bounded by `since` (ISO 8601) to avoid processing stale callbacks.
    pub fn get_undelivered_callback_tasks(&self, agent_id: &str, since: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND action_type = 'resume_agent'
               AND status IN ('completed', 'failed')
               AND completed_at IS NOT NULL
               AND completed_at > ?2
             ORDER BY completed_at ASC",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id, since], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Get callback tasks that completed or failed but have not yet been delivered,
    /// scoped to a specific session. Used by TUI to avoid cross-session leakage.
    pub fn get_undelivered_callback_tasks_for_session(
        &self,
        agent_id: &str,
        since: &str,
        session_id: &str,
    ) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND action_type = 'resume_agent'
               AND status IN ('completed', 'failed')
               AND completed_at IS NOT NULL
               AND completed_at > ?2
               AND created_by_session = ?3
             ORDER BY completed_at ASC",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id, since, session_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Atomically mark a completed or failed callback task as delivered.
    /// Returns `false` if the task was already claimed (not in 'completed'/'failed' status).
    pub fn mark_task_delivered(&self, id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE tasks SET status = 'delivered', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1 AND status IN ('completed', 'failed')",
            params![id],
        )?;
        Ok(n > 0)
    }

    /// Terminal record for a deferred wrapper consumed without dispatching
    /// (mika#2169).
    ///
    /// `expired` is deliberate: it is already in the `status` CHECK constraint,
    /// it is terminal, and it is NOT in the `('completed','failed')` set that
    /// `get_undelivered_callback_tasks` scans — so the wrapper leaves the
    /// delivery queue instead of re-entering it. Marking it `failed` would
    /// livelock the delivery scan.
    ///
    /// Three fins de vie, trois mots distincts: `delivered` = the turn
    /// dispatched; `expired` + reason = the turn happened and dispatched
    /// nothing; `completed` with no successor = the promotion never fired —
    /// which is exactly the exclusivity `count_promoted_undelivered_wrappers`
    /// relies on.
    ///
    /// The guard on `label` and on the departure status makes the write
    /// idempotent and impossible to aim at anything but a deferred wrapper.
    pub fn mark_deferred_wrapper_noop(&self, id: &str, reason: &str) -> Result<bool> {
        let now = crate::timestamp::now();
        let n = self.conn.execute(
            "UPDATE tasks
             SET status = 'expired',
                 result = ?2,
                 completed_at = ?3,
                 updated_at = ?3
             WHERE id = ?1
               AND label = ?4
               AND status IN ('completed', 'delivered')",
            params![id, reason, now, crate::agent::DEFERRED_DISPATCH_LABEL],
        )?;
        Ok(n > 0)
    }

    /// Count deferred wrappers promoted (`completed`) but never taken by the
    /// engine, older than `stale_seconds` and born at or after `epoch`
    /// (mika#2169, L2b).
    ///
    /// After L1 and L2a, `status = 'completed'` on this label is **exclusively**
    /// "promoted, not yet taken": delivery writes `delivered`, and a sterile
    /// consumption writes `expired`. That exclusivity is what makes the count
    /// readable — without it the number mixes promotion starvation with the very
    /// defect this ticket repairs.
    ///
    /// `epoch` is the instant L1 first ran in production (`schema_meta` key
    /// `deferred_promotion_epoch`). Rows older than it predate the exclusivity
    /// and would fire the indicator permanently, for a reason unrelated to
    /// starvation. Nothing mutates them — the bound only narrows the count.
    pub fn count_promoted_undelivered_wrappers(
        &self,
        agent_id: &str,
        stale_seconds: i64,
        epoch: &str,
    ) -> Result<(i64, i64)> {
        let stale_modifier = format!("-{stale_seconds} seconds");
        let row = self.conn.query_row(
            "SELECT COUNT(*),
                    COALESCE(
                      MAX(CAST(strftime('%s','now') - strftime('%s', completed_at) AS INTEGER)),
                      0)
             FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND label = ?2
               AND status = 'completed'
               AND completed_at IS NOT NULL
               AND completed_at >= ?4
               AND completed_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?3)",
            params![
                agent_id,
                crate::agent::DEFERRED_DISPATCH_LABEL,
                stale_modifier,
                epoch
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(row)
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

    /// Find implement-class parent self_dev tasks left `in_progress` whose
    /// callback subtask delivered without producing a PR (#871).
    ///
    /// A parent is "orphaned" when:
    /// - `status = 'in_progress'`, `source = 'self_dev'`, `trigger_type = 'manual'`
    /// - Its latest callback subtask is `status = 'delivered'`
    /// - **The callback's** `dispatch_class = 'implement'` (or NULL — pre-v34 via COALESCE).
    ///   The class is keyed off the child (per-dispatch) rather than the parent
    ///   because reused parents (mika#920 pattern) carry stale class data from
    ///   their original dispatch.
    /// - The callback's `updated_at` is older than `grace_seconds` ago
    /// - Parent metadata does NOT contain `$.claude_pilot.pr_url`
    /// - No other active callback child exists (defers to #870's retry loop)
    ///
    /// Groom-class callbacks (mika#1001) are NOT reaped here — their expected
    /// artifact is a plan commit pushed to the branch, not a PR url. Groom-class
    /// leak detection is a separate follow-up (mika#1118 Option B).
    ///
    /// **Coupled pair:** `find_completable_parent_tasks_on_pr_url` is the
    /// success-side sibling (mika#1162). Any filter change here (agent_id,
    /// status, source, trigger_type, dispatch_class, sibling guard, grace
    /// window) MUST be applied symmetrically there. The two queries differ
    /// only on the `pr_url` predicate (`IS NULL` here vs `IS NOT NULL` there).
    pub fn find_orphaned_parent_tasks(
        &self,
        agent_id: &str,
        grace_seconds: i64,
    ) -> Result<Vec<OrphanedParentTask>> {
        let grace_modifier = format!("-{grace_seconds} seconds");
        let mut stmt = self.conn.prepare(
            "SELECT parent.id, parent.agent_id, parent.created_at,
                    MIN(child.id) AS callback_task_id
             FROM tasks parent
             JOIN tasks child ON parent.id = child.parent_task_id
             WHERE parent.agent_id = ?1
               AND parent.status = 'in_progress'
               AND parent.source = 'self_dev'
               AND parent.trigger_type = 'manual'
               AND COALESCE(child.dispatch_class, 'implement') = 'implement'
               AND child.trigger_type = 'callback'
               AND child.action_type = 'resume_agent'
               AND child.status = 'delivered'
               AND child.updated_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
               AND (parent.metadata IS NULL
                    OR json_extract(parent.metadata, '$.claude_pilot.pr_url') IS NULL)
               AND NOT EXISTS (
                 SELECT 1 FROM tasks sibling
                 WHERE sibling.parent_task_id = parent.id
                   AND sibling.id != child.id
                   AND sibling.status IN ('pending', 'in_progress')
               )
             GROUP BY parent.id
             ORDER BY parent.id",
        )?;
        let rows = stmt
            .query_map(params![agent_id, grace_modifier], |row| {
                Ok(OrphanedParentTask {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    created_at: row.get(2)?,
                    callback_task_id: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Find phantom tracking rows: `action_type='none'`, `process_id IS NULL`,
    /// `status IN ('in_progress','blocked')`, `updated_at` older than
    /// `age_seconds` ago. These are the NULL-PID tracking-task orphans the
    /// callback watchdog cannot see (its very first predicate is
    /// `process_id IS NOT NULL`) and the orphaned-parent reaper does not
    /// select (no `resume_agent` callback child). See mika#1712.
    ///
    /// **Scope narrower than the ticket's Problem statement (ADV-2, 2026-08-21).**
    /// Plan §3 (Problem) lists `status IN ('in_progress','blocked','pending')` as
    /// the observed leak class, and one of the two cited leaked samples
    /// (`613a996d ... | pending`) is a `pending` phantom. The mika#1712 AC
    /// narrows to `('in_progress','blocked')` only — `pending` is deferred to
    /// the mika#1934 cause-racine investigation. Rationale: `pending` is the
    /// default initial state of every tracking task from `create_task`, and
    /// sweeping it at the default grace would false-positive on any newly-created
    /// tracking row the agent hasn't yet transitioned. `in_progress` and
    /// `blocked` are the "started but abandoned" states where the phantom
    /// signal is unambiguous. If mika#1934's telemetry shows `pending` phantoms
    /// remain a meaningful leak class, the predicate can be widened there.
    ///
    /// `age_seconds = 0` matches every candidate row regardless of freshness.
    /// The comparison uses `<=` (not `<`) so a row inserted in the same
    /// second the WHERE clause evaluates is still selected — SQLite's
    /// `strftime('%Y-%m-%dT%H:%M:%SZ', ...)` truncates to seconds, and a `<`
    /// would silently miss same-second injections. `<=` is safe for the AC3
    /// path too: the default grace has 1-second slack to spare, and a
    /// row updated exactly at "now - grace" being caught 1s early is
    /// behaviorally identical to being caught 1s later.
    ///
    /// SOLE READER — this query is the sole source of candidates for the
    /// `phantom_aged_out` audit-event transition emitted by
    /// [`super::task_engine::engine::TaskEngine::sweep_null_pid_phantoms`]
    /// (AC3 watchdog) and the equivalent step inside
    /// [`super::task_engine::engine::TaskEngine::startup_recovery`] (AC5).
    ///
    /// Candidates, not verdicts (mika#2156). Both callers run each row past a
    /// liveness guard — `TaskEngine::live_dispatch_child`, backed by
    /// [`Self::find_dispatch_children_with_pid`] — before writing the failure,
    /// because a PID's liveness is not expressible in SQL. Age alone cannot
    /// tell an orphaned tracking row from one whose dispatch is still running:
    /// the row's `updated_at` is never bumped while the work proceeds.
    pub fn find_phantom_tracking_tasks(
        &self,
        agent_id: &str,
        age_seconds: i64,
    ) -> Result<Vec<PhantomTrackingTask>> {
        let age_modifier = format!("-{age_seconds} seconds");
        let mut stmt = self.conn.prepare(
            "SELECT id, agent_id, label, status, created_at, updated_at
             FROM tasks
             WHERE agent_id = ?1
               AND action_type = 'none'
               AND process_id IS NULL
               AND status IN ('in_progress', 'blocked')
               AND updated_at <= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
             ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![agent_id, age_modifier], |row| {
                Ok(PhantomTrackingTask {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    label: row.get(2)?,
                    status: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Read `metadata.process_start_time` out of the value the two dispatch-child
    /// queries below select for it.
    ///
    /// The executor writes it as a JSON **string** (`skills/executor.rs`), but an
    /// integer is accepted too so a hand-written or future-shaped `metadata` row
    /// is not silently treated as missing. Anything else — NULL, malformed, a
    /// negative integer — degrades to `None`.
    ///
    /// **`None` never means "dead" and never means "alive"; it means the pair
    /// that identifies a process *instance* is incomplete.** Each caller decides
    /// what to do about that, and they deliberately decide differently: the
    /// phantom sweep sweeps (mika#2156 D-3), the supersession declines to signal
    /// (mika#2335), the live-pilot predicate answers `Unreadable` (mika#2279).
    ///
    /// Shared rather than written twice: the two queries below carry the same
    /// rule, and a rule written twice is a rule that can disagree with itself —
    /// which is the sentence [`is_terminal_task_status`] already had to have
    /// engraved on it one file over.
    fn parse_process_start_time(value: rusqlite::types::Value) -> Option<u64> {
        match value {
            rusqlite::types::Value::Text(t) => t.parse::<u64>().ok(),
            rusqlite::types::Value::Integer(i) => u64::try_from(i).ok(),
            _ => None,
        }
    }

    /// The dispatch children of a tracking row that carry a `process_id`.
    ///
    /// Companion to [`Self::find_phantom_tracking_tasks`] (mika#2156), placed
    /// next to it so a reader meets the guard where they meet the query it
    /// guards. The sweep query cannot tell "orphaned tracking row" from
    /// "tracking row whose dispatch is still running": its three non-temporal
    /// criteria (`action_type='none'`, `process_id IS NULL`,
    /// `status IN ('in_progress','blocked')`) are satisfied *by construction*
    /// for every healthy tracking row, because the real process lives on a
    /// separate `long_running:*` recall row. That recall row already points
    /// back via `parent_task_id` — the missing link is read here, not added.
    ///
    /// Deliberately does NOT filter on the child's status. Per plan mika#2156
    /// D-2: 1146 of the 1147 PID-carrying children measured in production are
    /// `delivered`, and nothing in the code proves `delivered` implies a dead
    /// process. Treating the child's status as the discriminator would disarm
    /// the sweeper on 98% of its historical population (measure M2). Liveness
    /// is the discriminator; the caller applies it.
    ///
    /// `process_start_time` is extracted from `metadata` JSON, where the
    /// executor writes it as a *string* (see `skills/executor.rs`). Rows
    /// without it come back as `None` — the caller sweeps them (D-3).
    pub fn find_dispatch_children_with_pid(
        &self,
        parent_task_id: &str,
    ) -> Result<Vec<DispatchChild>> {
        let mut stmt = self.conn.prepare(
            // `json_valid` guard is load-bearing, not belt-and-braces:
            // SQLite's `json_extract` raises a hard "malformed JSON" error
            // (not NULL) on a non-JSON `metadata`, and that error propagates
            // out of the whole `query_map`. Without the guard, ONE bad sibling
            // row would fail the lookup for the entire parent — and the caller
            // would then have no answer about a parent whose OTHER child may
            // be alive. Wrapped, a malformed row degrades to NULL and only
            // that child stops being able to spare (plan mika#2156 D-3).
            "SELECT id,
                    process_id,
                    CASE WHEN json_valid(metadata)
                         THEN json_extract(metadata, '$.process_start_time')
                    END,
                    status
             FROM tasks
             WHERE parent_task_id = ?1
               AND process_id IS NOT NULL
             ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![parent_task_id], |row| {
                Ok(DispatchChild {
                    id: row.get(0)?,
                    process_id: row.get(1)?,
                    process_start_time: Self::parse_process_start_time(row.get(2)?),
                    status: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The dispatch children reachable from an **issue URL**, whatever the
    /// status of the tracking row that carries it (mika#2279).
    ///
    /// Sibling of [`Self::find_dispatch_children_with_pid`], which starts from a
    /// known parent id. This one starts from the URL, because the caller —
    /// "is a pilot alive for this ticket?" — holds an issue, not a task.
    ///
    /// **The absent predicate is the fix.** There is deliberately no condition
    /// on `parent.status`. `has_active_self_dev_task_for_issue` conjoins
    /// `reference_url LIKE …` with `status IN ('pending','in_progress')` on one
    /// row, and that conjunction goes false the instant a supersession cancels
    /// the parent — while the pilot on the child keeps running. That false
    /// answer is what let the mika#2279 loop re-drive a ticket every 20 minutes
    /// with its pilot working: a cancelled parent is not the absence of a
    /// dispatch, it is exactly the state where the question needs asking.
    ///
    /// `issue_url` is matched as a prefix `LIKE` so the `?phase=groom` variant
    /// is covered — same rule, same reason, as
    /// [`Self::has_active_self_dev_task_for_issue`].
    ///
    /// Two things this query does **not** decide, both left to the caller:
    /// liveness (only `/proc` can answer that) and the child's terminal status
    /// (`is_terminal_task_status` owns that vocabulary; spelling it here would
    /// be a third copy in a dialect that cannot express its "unknown is not
    /// terminal" rule).
    ///
    /// **One predicate it carries that the sibling does not**, named here so the
    /// asymmetry is not read later as an accident:
    /// `child.trigger_type = 'callback'`. The sibling narrows by its
    /// `parent_task_id` argument, which already scopes it to one tracking row's
    /// offspring; this one starts from a URL and so must say which of a parent's
    /// children is a dispatch. It is redundant *today* — `set_task_process_id`
    /// has exactly one production call site and it writes a callback child
    /// (`skills/executor.rs`) — and it is kept because the day something else
    /// records a `process_id`, a URL-keyed query that did not say
    /// "callback" would start answering about a process that is not a pilot.
    ///
    /// The `json_valid` guard is carried over verbatim from the sibling, and is
    /// load-bearing for the same reason: `json_extract` raises a hard error on a
    /// non-JSON `metadata`, and that error propagates out of the whole
    /// `query_map` — so one malformed sibling row would make the answer for the
    /// entire ticket "no pilot", which is the fail-open direction *and* the
    /// wrong one here.
    pub fn find_dispatch_children_for_issue_url(
        &self,
        agent_id: &str,
        issue_url: &str,
    ) -> Result<Vec<IssueDispatchChild>> {
        let prefix = format!("{}%", issue_url);
        let mut stmt = self.conn.prepare(
            "SELECT child.id,
                    child.process_id,
                    CASE WHEN json_valid(child.metadata)
                         THEN json_extract(child.metadata, '$.process_start_time')
                    END,
                    child.status,
                    parent.id
             FROM tasks child
             JOIN tasks parent ON child.parent_task_id = parent.id
             WHERE parent.agent_id = ?1
               AND parent.reference_url LIKE ?2
               AND child.trigger_type = 'callback'
               AND child.process_id IS NOT NULL
             ORDER BY child.id",
        )?;
        let rows = stmt
            .query_map(params![agent_id, prefix], |row| {
                Ok(IssueDispatchChild {
                    parent_task_id: row.get(4)?,
                    child: DispatchChild {
                        id: row.get(0)?,
                        process_id: row.get(1)?,
                        process_start_time: Self::parse_process_start_time(row.get(2)?),
                        status: row.get(3)?,
                    },
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
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

    /// Test-only helper: backdate a task's `updated_at` by `seconds_ago`
    /// seconds relative to now. Used by mika#1712 integration tests to inject
    /// phantom-shape rows aged past the sweep grace window without waiting on
    /// wall-clock time. Not intended for production use — timestamps are
    /// otherwise engine-owned.
    #[doc(hidden)]
    pub fn backdate_task_updated_at(&self, task_id: &str, seconds_ago: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
             WHERE id = ?1",
            params![task_id, format!("-{seconds_ago} seconds")],
        )?;
        Ok(())
    }

    /// Test-only helper: backdate a task's `completed_at` by `seconds_ago`
    /// seconds relative to now. Sibling of [`Self::backdate_task_updated_at`],
    /// and needed for the same reason a different column needs it:
    /// `update_task_completed` writes `completed_at = now`, so a mika#2179 test
    /// cannot otherwise seed the incident's `2026-09-03T22:03:24Z` completion
    /// and measure the 5 h 06 wait that followed. Deliberately leaves
    /// `updated_at` alone — the delivery latency under test is
    /// `completed_at → delivery`, and moving both would hide a regression in
    /// which column the measurement reads. Not intended for production use —
    /// timestamps are otherwise engine-owned.
    #[doc(hidden)]
    pub fn backdate_task_completed_at(&self, task_id: &str, seconds_ago: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
             WHERE id = ?1",
            params![task_id, format!("-{seconds_ago} seconds")],
        )?;
        Ok(())
    }

    /// Test-only helper: rewrite a task's primary key.
    ///
    /// `find_dispatch_children_with_pid` orders by `id`, and `create_task`
    /// generates a UUIDv4 — so a mika#2156 test that needs a *specific*
    /// examination order (a dead child examined before a live one) cannot get
    /// it from insertion order. Forcing the ids is what makes that test
    /// deterministic, and therefore what makes its injection check mean
    /// something. Safe only for a leaf row: nothing here rewrites the
    /// `parent_task_id` of any grandchild. Not for production use.
    #[doc(hidden)]
    pub fn set_task_id_for_test(&self, old_id: &str, new_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET id = ?2 WHERE id = ?1",
            params![old_id, new_id],
        )?;
        Ok(())
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

    /// Find parent self_dev tasks left `in_progress` after their callback
    /// subtask delivered WITH a `pr_url` (success indicator). Sibling to
    /// `find_orphaned_parent_tasks` — same JOIN shape, same guards, but
    /// inverted on the `pr_url` predicate. Used by the success-side engine
    /// backstop (mika#1162).
    ///
    /// A parent is `completable` when:
    /// - Parent is `status='in_progress'`, `source='self_dev'`, `trigger_type='manual'`
    /// - Its latest callback subtask is `status='delivered'`
    /// - **The callback's** `dispatch_class='implement'` (or NULL — pre-v34 via COALESCE)
    /// - The callback's `updated_at` is older than `grace_seconds` ago
    /// - Parent metadata HAS a non-empty `$.claude_pilot.pr_url`
    /// - No other active callback child exists (mirrors the reaper's guard)
    ///
    /// Groom-class callbacks (mika#1001) cannot trip this path because they
    /// never emit `PR:` lines — the `dispatch_class` filter is defense-in-depth.
    ///
    /// SOLE WRITER warning: this method is the only DB query that selects
    /// candidates for the `parent_completed_from_callback` audit transition.
    pub fn find_completable_parent_tasks_on_pr_url(
        &self,
        agent_id: &str,
        grace_seconds: i64,
    ) -> Result<Vec<CompletableParentTask>> {
        let grace_modifier = format!("-{grace_seconds} seconds");
        let mut stmt = self.conn.prepare(
            "SELECT parent.id, parent.agent_id, parent.created_at,
                    MIN(child.id) AS callback_task_id,
                    json_extract(parent.metadata, '$.claude_pilot.pr_url') AS pr_url
             FROM tasks parent
             JOIN tasks child ON parent.id = child.parent_task_id
             WHERE parent.agent_id = ?1
               AND parent.status = 'in_progress'
               AND parent.source = 'self_dev'
               AND parent.trigger_type = 'manual'
               AND COALESCE(child.dispatch_class, 'implement') = 'implement'
               AND child.trigger_type = 'callback'
               AND child.action_type = 'resume_agent'
               AND child.status = 'delivered'
               AND child.updated_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
               AND json_extract(parent.metadata, '$.claude_pilot.pr_url') IS NOT NULL
               AND json_extract(parent.metadata, '$.claude_pilot.pr_url') != ''
               AND NOT EXISTS (
                 SELECT 1 FROM tasks sibling
                 WHERE sibling.parent_task_id = parent.id
                   AND sibling.id != child.id
                   AND sibling.status IN ('pending', 'in_progress')
               )
             GROUP BY parent.id
             ORDER BY parent.id",
        )?;
        let rows = stmt
            .query_map(params![agent_id, grace_modifier], |row| {
                Ok(CompletableParentTask {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    created_at: row.get(2)?,
                    callback_task_id: row.get(3)?,
                    pr_url: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Find parent self_dev **issue** tasks left `in_progress` with **zero**
    /// callback children, aged past `grace_seconds` (mika#1687).
    ///
    /// This is the zero-child complement of `find_orphaned_parent_tasks` and
    /// `find_completable_parent_tasks_on_pr_url`: both of those INNER-JOIN a
    /// delivered callback child, so a parent that reached `in_progress` without
    /// ever recording a callback child (silent pilot death — hypothesis 3 of
    /// mika#1687) produces zero rows in either and is never reaped. The
    /// `NOT EXISTS (SELECT 1 FROM tasks child …)` predicate here is the exact
    /// complement of their JOIN, so the three selection sets are disjoint by
    /// construction (they require a child; this requires none).
    ///
    /// Staleness keys on `parent.updated_at` (there is no delivered-child
    /// `updated_at` to key on). Scoped to `type='issue'` in v1 — milestone and
    /// project parents legitimately sit childless between child dispatches and
    /// carry their own advancement backstops (mika#991, #1218); their
    /// childless-stuck detection is a deferred follow-up.
    ///
    /// SOLE WRITER context: candidates selected here are transitioned to
    /// `failed` with the distinct `stuck_in_progress_no_callback_child` reason
    /// by `TaskEngine::reap_childless_stuck_parent_tasks`.
    pub fn find_childless_stuck_parent_tasks(
        &self,
        agent_id: &str,
        grace_seconds: i64,
    ) -> Result<Vec<ChildlessStuckParent>> {
        let grace_modifier = format!("-{grace_seconds} seconds");
        let mut stmt = self.conn.prepare(
            "SELECT parent.id, parent.agent_id, parent.created_at, parent.updated_at
             FROM tasks parent
             WHERE parent.agent_id = ?1
               AND parent.status = 'in_progress'
               AND parent.source = 'self_dev'
               AND parent.trigger_type = 'manual'
               AND parent.type = 'issue'
               AND parent.updated_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
               AND NOT EXISTS (
                 SELECT 1 FROM tasks child
                 WHERE child.parent_task_id = parent.id
               )
             ORDER BY parent.id",
        )?;
        let rows = stmt
            .query_map(params![agent_id, grace_modifier], |row| {
                Ok(ChildlessStuckParent {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    created_at: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// True when `parent_task_id` still has a **live** deferred-dispatch wrapper
    /// representing it (mika#2045, widened by mika#2181).
    ///
    /// This is the task-scoped answer to the question `mika tasks
    /// promote-deferred` answers per dispatch class. The class-scoped answer
    /// cannot distinguish "this task is queued" from "some other task is
    /// queued"; `register_deferred_callback` sets `parent_task_id` on the
    /// wrapper, so the task-scoped predicate is exact.
    ///
    /// It asks the same question as clause (1) of
    /// [`Self::find_orphaned_pending_issue_tasks`] — *is this parent
    /// represented?* — and must therefore answer it with the same predicate.
    /// The old name said `pending`, which described the implementation rather
    /// than the question, and the narrow predicate it named was the mika#2181
    /// defect. It has no production caller today; that is not a stable property,
    /// and two functions named as equivalents that disagree are a trap armed for
    /// the next caller. Renamed so the compiler catches every site.
    pub fn has_live_deferred_wrapper_child(
        &self,
        agent_id: &str,
        parent_task_id: &str,
        promoted_liveness_seconds: i64,
    ) -> Result<bool> {
        let liveness_modifier = format!("-{promoted_liveness_seconds} seconds");
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1
               AND parent_task_id = ?2
               AND trigger_type = 'callback'
               AND label = ?3
               AND (
                 status = 'pending'
                 OR (status = 'completed'
                     AND completed_at IS NOT NULL
                     AND completed_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?4))
               )",
            params![
                agent_id,
                parent_task_id,
                crate::agent::DEFERRED_DISPATCH_LABEL,
                liveness_modifier
            ],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Find `pending` self_dev issue parents older than `grace_seconds` that no
    /// callback child represents any more (mika#2045).
    ///
    /// Two `NOT EXISTS` clauses, and both are load-bearing:
    /// 1. no **live** deferred wrapper — `pending` (queued behind a busy
    ///    dispatch slot), or `completed` within `promoted_liveness_seconds`
    ///    (promoted, and the silent turn it feeds has not returned yet);
    /// 2. no active non-deferred callback — the task's real dispatch is not in
    ///    flight.
    ///
    /// Clause (1) counts promoted wrappers on purpose (mika#2181). Promotion
    /// writes `status = 'completed'`; `delivered` only arrives when the silent
    /// turn *returns*, minutes later. Counting `pending` alone made the reaper
    /// call a healthy parent unrepresented a few milliseconds after promotion —
    /// the three engine steps share one 60s pass, in the order promote ->
    /// dispatch -> reap — and burn `MAX_STUCK_REARMS` in two ticks. The founding
    /// trace: promoted 15:31:03Z, expired 15:33:03Z, turn answered 15:34:43Z.
    ///
    /// The window is bounded because `completed`-without-`delivered` is not
    /// guaranteed transient: on the silent-turn error path the wrapper is
    /// re-armed but never marked `delivered`, so it stays `completed` forever.
    /// An unbounded clause would make that corpse a permanent shield against
    /// repair. `completed_at IS NULL` reads as *not* live for the same reason —
    /// a wrapper that cannot prove it is recent does not get to hide its parent.
    /// `completed` is named explicitly rather than `status NOT IN (...)`:
    /// `delivered`, `failed` and `cancelled` are spent wrappers, never live.
    ///
    /// Dropping (2) would classify a parent whose pilot is actually running as
    /// orphaned during the window before the parent reaches `in_progress`.
    ///
    /// Age is measured on `created_at`, not `updated_at`: a re-armed parent
    /// regains a `pending` wrapper, so clause (1) already protects it and the
    /// grace window has no reason to restart.
    ///
    /// `rearm_count` reads `metadata.stuck_rearm_count` through a `json_valid`
    /// guard so unreadable metadata yields 0 instead of raising.
    pub fn find_orphaned_pending_issue_tasks(
        &self,
        agent_id: &str,
        grace_seconds: i64,
        promoted_liveness_seconds: i64,
    ) -> Result<Vec<OrphanedPendingTask>> {
        let grace_modifier = format!("-{grace_seconds} seconds");
        let liveness_modifier = format!("-{promoted_liveness_seconds} seconds");
        let mut stmt = self.conn.prepare(
            "SELECT parent.id,
                    parent.reference_url,
                    parent.created_at,
                    CAST(strftime('%s', 'now') - strftime('%s', parent.created_at) AS INTEGER),
                    COALESCE(
                      CASE WHEN json_valid(parent.metadata)
                           THEN CAST(json_extract(parent.metadata, '$.stuck_rearm_count') AS INTEGER)
                      END, 0),
                    COALESCE(parent.dispatch_class, 'implement')
             FROM tasks parent
             WHERE parent.agent_id = ?1
               AND parent.status = 'pending'
               AND parent.source = 'self_dev'
               AND parent.trigger_type = 'manual'
               AND parent.type = 'issue'
               AND parent.reference_url IS NOT NULL
               AND parent.created_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
               AND NOT EXISTS (
                 SELECT 1 FROM tasks w
                 WHERE w.parent_task_id = parent.id
                   AND w.agent_id = ?1
                   AND w.trigger_type = 'callback'
                   AND w.label = ?3
                   AND (
                     w.status = 'pending'
                     OR (w.status = 'completed'
                         AND w.completed_at IS NOT NULL
                         AND w.completed_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?4))
                   )
               )
               AND NOT EXISTS (
                 SELECT 1 FROM tasks c
                 WHERE c.parent_task_id = parent.id
                   AND c.trigger_type = 'callback'
                   AND c.status IN ('pending', 'in_progress')
                   AND c.label != ?3
               )
             ORDER BY parent.id",
        )?;
        let rows = stmt
            .query_map(
                params![
                    agent_id,
                    grace_modifier,
                    crate::agent::DEFERRED_DISPATCH_LABEL,
                    liveness_modifier
                ],
                |row| {
                    Ok(OrphanedPendingTask {
                        id: row.get(0)?,
                        reference_url: row.get(1)?,
                        created_at: row.get(2)?,
                        age_seconds: row.get(3)?,
                        rearm_count: row.get(4)?,
                        dispatch_class: row.get(5)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Find `blocked` self_dev issue parents refused on a busy dispatch slot,
    /// older than `grace_seconds` and with nothing left representing them
    /// (mika#2169, L3b).
    ///
    /// **Discriminated, not widened.** The population is separated from the
    /// deliberate operator gates by `json_extract(result, '$.error') =
    /// 'global_dispatch_active'` — the only value the slot-refusal path writes.
    /// A `blocked` written by an auto-merge refusal
    /// (`server/verdict_handler.rs`) or a QA escalation does not carry it, so
    /// this sweep can never re-drive a dispatch an operator deliberately
    /// stopped. Measured negative control: task `662d9752` carries
    /// `unauthorized_webhook_dispatch` and is excluded.
    ///
    /// The two `NOT EXISTS` clauses are taken verbatim from
    /// `find_orphaned_pending_issue_tasks` and carry the same meaning: no
    /// `pending` wrapper still queued, and no live real dispatch.
    pub fn find_stale_blocked_dispatch_tasks(
        &self,
        agent_id: &str,
        grace_seconds: i64,
    ) -> Result<Vec<StaleBlockedTask>> {
        let grace_modifier = format!("-{grace_seconds} seconds");
        let mut stmt = self.conn.prepare(
            "SELECT parent.id,
                    parent.reference_url,
                    parent.created_at,
                    CAST(strftime('%s', 'now') - strftime('%s', parent.created_at) AS INTEGER),
                    COALESCE(
                      CASE WHEN json_valid(parent.metadata)
                           THEN CAST(json_extract(parent.metadata, '$.stuck_rearm_count') AS INTEGER)
                      END, 0),
                    COALESCE(parent.dispatch_class, 'implement'),
                    json_extract(parent.result, '$.blocking_callback_id')
             FROM tasks parent
             WHERE parent.agent_id = ?1
               AND parent.status = 'blocked'
               AND parent.source = 'self_dev'
               AND parent.trigger_type = 'manual'
               AND parent.type = 'issue'
               AND parent.reference_url IS NOT NULL
               AND json_valid(parent.result)
               AND json_extract(parent.result, '$.error') = 'global_dispatch_active'
               AND parent.created_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
               AND NOT EXISTS (
                 SELECT 1 FROM tasks w
                 WHERE w.parent_task_id = parent.id
                   AND w.trigger_type = 'callback'
                   AND w.status = 'pending'
                   AND w.label = ?3
               )
               AND NOT EXISTS (
                 SELECT 1 FROM tasks c
                 WHERE c.parent_task_id = parent.id
                   AND c.trigger_type = 'callback'
                   AND c.status IN ('pending', 'in_progress')
                   AND c.label != ?3
               )
             ORDER BY parent.id",
        )?;
        let rows = stmt
            .query_map(
                params![
                    agent_id,
                    grace_modifier,
                    crate::agent::DEFERRED_DISPATCH_LABEL
                ],
                |row| {
                    Ok(StaleBlockedTask {
                        id: row.get(0)?,
                        reference_url: row.get(1)?,
                        created_at: row.get(2)?,
                        age_seconds: row.get(3)?,
                        rearm_count: row.get(4)?,
                        dispatch_class: row.get(5)?,
                        blocking_callback_id: row.get(6)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Read `metadata.stuck_rearm_count` for a task, tolerating absent or
    /// unreadable metadata as 0 (mika#2045).
    ///
    /// This counter records *repairs* only — the deferred wrappers this task
    /// needed because an earlier one was consumed without dispatching. It is
    /// deliberately not the count of all wrappers ever registered for the task:
    /// the ordinary rejection path registers those, and conflating the two
    /// would spend the repair budget on healthy queueing.
    pub fn get_stuck_rearm_count(&self, task_id: &str) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COALESCE(
                      CASE WHEN json_valid(metadata)
                           THEN CAST(json_extract(metadata, '$.stuck_rearm_count') AS INTEGER)
                      END, 0)
             FROM tasks WHERE id = ?1",
            params![task_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Increment `metadata.stuck_rearm_count`, returning the new value
    /// (mika#2045). Unreadable metadata is replaced by a fresh object rather
    /// than raising, so a corrupt row still repairs and still terminates.
    pub fn increment_stuck_rearm_count(&self, task_id: &str) -> Result<i64> {
        self.conn.execute(
            "UPDATE tasks SET
                metadata = json_set(
                  CASE WHEN json_valid(metadata) THEN metadata ELSE '{}' END,
                  '$.stuck_rearm_count',
                  COALESCE(
                    CASE WHEN json_valid(metadata)
                         THEN CAST(json_extract(metadata, '$.stuck_rearm_count') AS INTEGER)
                    END, 0) + 1),
                updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            params![task_id],
        )?;
        self.get_stuck_rearm_count(task_id)
    }

    /// Parents past the grace that the reaper is sheltering **only** because a
    /// promoted wrapper is still inside the liveness window (mika#2181).
    ///
    /// The sheltering happens inside a SQL `NOT EXISTS`, so a spared parent
    /// never reaches Rust: it produces no candidate, no log line, no audit
    /// event, and `mika tasks stuck` does not print it. If the widened predicate
    /// ever disarmed the reaper systematically, its counters would fall to zero
    /// — indistinguishable from a healthy loop. That silence is the failure mode
    /// `docs/solutions/best-practices/a-reaper-that-reads-a-proxy-instead-of-the-process-2026-09-04.md`
    /// names ("faucher, **et le compter**") and fixed for the sibling sweep with
    /// `phantom_sweep_spared`. This is that counter for this reaper.
    ///
    /// It is the exact complement of clause (1): same parent filters, same
    /// grace, but requiring a live *promoted* wrapper rather than the absence of
    /// any live one. A parent held by a plain `pending` wrapper is ordinary
    /// queueing and is not counted — only the new shelter this fix introduced.
    pub fn find_parents_sheltered_by_promoted_wrapper(
        &self,
        agent_id: &str,
        grace_seconds: i64,
        promoted_liveness_seconds: i64,
    ) -> Result<Vec<String>> {
        let grace_modifier = format!("-{grace_seconds} seconds");
        let liveness_modifier = format!("-{promoted_liveness_seconds} seconds");
        let mut stmt = self.conn.prepare(
            "SELECT parent.reference_url
             FROM tasks parent
             WHERE parent.agent_id = ?1
               AND parent.status = 'pending'
               AND parent.source = 'self_dev'
               AND parent.trigger_type = 'manual'
               AND parent.type = 'issue'
               AND parent.reference_url IS NOT NULL
               AND parent.created_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
               AND EXISTS (
                 SELECT 1 FROM tasks w
                 WHERE w.parent_task_id = parent.id
                   AND w.agent_id = ?1
                   AND w.trigger_type = 'callback'
                   AND w.label = ?3
                   AND w.status = 'completed'
                   AND w.completed_at IS NOT NULL
                   AND w.completed_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?4)
               )
               AND NOT EXISTS (
                 SELECT 1 FROM tasks w2
                 WHERE w2.parent_task_id = parent.id
                   AND w2.agent_id = ?1
                   AND w2.trigger_type = 'callback'
                   AND w2.label = ?3
                   AND w2.status = 'pending'
               )
             ORDER BY parent.id",
        )?;
        let rows = stmt
            .query_map(
                params![
                    agent_id,
                    grace_modifier,
                    crate::agent::DEFERRED_DISPATCH_LABEL,
                    liveness_modifier
                ],
                |row| row.get(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Every deferred wrapper of a parent with the fields the stuck-pending
    /// reaper's verdict turns on (mika#2181 AC4), oldest first.
    ///
    /// Read once per candidate, before the re-arm decision, and written into the
    /// `details` of both terminal reaper events. Unfiltered by status on
    /// purpose: the point is to show what was there, including the statuses that
    /// did *not* count as live.
    pub fn summarize_deferred_wrappers_of_parent(
        &self,
        agent_id: &str,
        parent_task_id: &str,
    ) -> Result<Vec<DeferredWrapperSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, status, completed_at FROM tasks
             WHERE agent_id = ?1
               AND parent_task_id = ?2
               AND trigger_type = 'callback'
               AND label = ?3
             -- `created_at` has one-second resolution, so it is not a total
             -- order: two wrappers minted in the same second would render in an
             -- arbitrary order in the audit line. `rowid` breaks the tie by
             -- insertion, which is the order the reaper's reader expects.
             ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt
            .query_map(
                params![
                    agent_id,
                    parent_task_id,
                    crate::agent::DEFERRED_DISPATCH_LABEL
                ],
                |row| {
                    Ok(DeferredWrapperSummary {
                        id: row.get(0)?,
                        status: row.get(1)?,
                        completed_at: row.get(2)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Cancel every deferred wrapper of a parent that is not already terminal
    /// (mika#2045). Called immediately before expiring the parent: a wrapper
    /// that survives the expiry can still be promoted, and would then replay a
    /// dispatch against a dead parent while the `ready` sweep has already
    /// created a live replacement for the same issue — a double dispatch.
    /// Returns the number of wrappers cancelled.
    pub fn cancel_deferred_wrappers_of_parent(
        &self,
        agent_id: &str,
        parent_task_id: &str,
    ) -> Result<usize> {
        let n = self.conn.execute(
            "UPDATE tasks
             SET status = 'cancelled',
                 result = 'parent expired by the stuck-pending reaper (mika#2045)',
                 completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE agent_id = ?1
               AND parent_task_id = ?2
               AND label = ?3
               AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')",
            params![
                agent_id,
                parent_task_id,
                crate::agent::DEFERRED_DISPATCH_LABEL
            ],
        )?;
        Ok(n)
    }

    /// The `action_config` of a parent's most recent deferred wrapper, whatever
    /// its status (mika#2045). The reaper replays this so a repaired dispatch is
    /// byte-identical to the one that was refused. `None` when the parent never
    /// had a wrapper — the reaper then rebuilds the call from the parent's own
    /// columns.
    pub fn latest_deferred_wrapper_action_config(
        &self,
        agent_id: &str,
        parent_task_id: &str,
    ) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT action_config FROM tasks
                 WHERE agent_id = ?1
                   AND parent_task_id = ?2
                   AND label = ?3
                 ORDER BY created_at DESC
                 LIMIT 1",
                params![
                    agent_id,
                    parent_task_id,
                    crate::agent::DEFERRED_DISPATCH_LABEL
                ],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// Return ALL children of a parent task for the reaper's structured log event
    /// (`task_engine_reaper.evaluated`). Captures a point-in-time snapshot at kill
    /// time so post-incident diagnosis can see what the reaper saw (mika#1126).
    pub fn get_reaper_child_snapshot(
        &self,
        parent_task_id: &str,
    ) -> Result<Vec<ReaperChildSnapshot>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, dispatch_class, status, trigger_type, action_type, updated_at, label
             FROM tasks
             WHERE parent_task_id = ?1
             ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![parent_task_id], |row| {
                Ok(ReaperChildSnapshot {
                    id: row.get(0)?,
                    dispatch_class: row.get(1)?,
                    status: row.get(2)?,
                    trigger_type: row.get(3)?,
                    action_type: row.get(4)?,
                    updated_at: row.get(5)?,
                    label: row.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Record (or clear) the OS process running under a task.
    ///
    /// **Also stamps `fired_at` when a process is recorded and the row has none
    /// (mika#2263 défaut (b)).** Registering a PID is the moment the engine
    /// learns a pilot is alive under this task — `skills/executor.rs` calls it
    /// immediately after the spawn — so a row that leaves this function with a
    /// `process_id` and no `fired_at` is a live dispatch that reads as *never
    /// dispatched*. That is precisely what `b429a658` (#2252) and `4e867d85`
    /// (#2212) looked like on 2026-09-09 while their bwrap pilots ran for 69
    /// and 45 minutes: `pending`, `fired_at` NULL, invisible to every probe
    /// that uses `fired_at` to tell *not yet dispatched* from *zombie*.
    ///
    /// The stamp lives here, at the single chokepoint every spawn path passes
    /// through, rather than in each caller — one site cannot drift from
    /// another the way a per-caller convention does.
    ///
    /// Two deliberate non-effects, both pinned by tests:
    /// - clearing (`process_id = None`, what a disposal does after a kill)
    ///   stamps nothing — that is not a dispatch;
    /// - an existing `fired_at` is never overwritten, so a re-record cannot
    ///   reset a dispatch's age under the reapers that measure it.
    pub fn set_task_process_id(&self, id: &str, process_id: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks
                SET process_id = ?1,
                    fired_at = CASE
                                 WHEN ?1 IS NOT NULL AND fired_at IS NULL
                                 THEN strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                                 ELSE fired_at
                               END,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
              WHERE id = ?2",
            params![process_id, id],
        )?;
        Ok(())
    }

    /// Returns (task_id, process_id) pairs for expired tasks that still have a process_id set.
    pub fn get_expired_tasks_with_process_id(&self, agent_id: &str) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, process_id FROM tasks
             WHERE agent_id = ?1 AND status = 'expired' AND process_id IS NOT NULL",
        )?;
        let rows = stmt
            .query_map(params![agent_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Clear the process_id after killing an orphan process to prevent repeated kill attempts.
    pub fn clear_task_process_id(&self, id: &str, agent_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET process_id = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1 AND agent_id = ?2",
            params![id, agent_id],
        )?;
        Ok(())
    }

    /// Get active callback tasks that have a process_id set (#959).
    ///
    /// Returns callback tasks in `in_progress` status with a non-null process_id,
    /// used by the callback watchdog to detect dead subprocesses.
    pub fn get_active_callback_tasks_with_pid(&self, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND status = 'in_progress'
               AND process_id IS NOT NULL",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id], Self::row_to_task)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Callback tasks whose dispatch may still be **alive**, with a
    /// `process_id` set (mika#2272).
    ///
    /// # Why this is not [`Self::get_active_callback_tasks_with_pid`]
    ///
    /// That method filters `status = 'in_progress'`, and on this row that
    /// status **never occurs**. A dispatch's callback child is created by
    /// `build_callback_task` with no status at all, so `create_task` writes
    /// `pending`; the auto-transition to `in_progress` (`executor.rs`, #525)
    /// applies to the **parent** manual task, not to the child. The child
    /// carries the PID (`set_task_process_id`, right after the spawn) and stays
    /// `pending` until the callback lands and moves it to `delivered`.
    ///
    /// Measured on the production database on 2026-09-09, over every row that
    /// has ever carried a `process_id`: 876 `delivered`, 19 `cancelled`, 1
    /// `failed`, 1 `pending` — and **zero** `in_progress`. A reaper reading the
    /// other method is not looking at a small population, it is looking at an
    /// empty one, which is exactly why mika#2261 shipped and never fired.
    ///
    /// So this scans both surfaces on which a live dispatch can sit: `pending`,
    /// which is where it actually sits, and `in_progress`, kept because nothing
    /// guarantees the state machine keeps this shape and a reaper that silently
    /// narrows again is the defect this method exists to close.
    ///
    /// The #959 watchdog deliberately keeps the narrower query: widening the
    /// population of *that* mechanism changes who marks a task `failed` when a
    /// process dies, which races the spawn monitor. Separate blast radius,
    /// separate ticket.
    pub fn get_live_dispatch_callback_tasks_with_pid(&self, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND status IN ('pending', 'in_progress')
               AND process_id IS NOT NULL",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id], Self::row_to_task)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Set a single field in the task's metadata JSON (#959).
    ///
    /// Uses SQLite's `json_set()` to merge the field into existing metadata,
    /// initializing with `'{}'` if metadata is currently NULL.
    pub fn set_task_metadata_field(&self, task_id: &str, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET
                metadata = json_set(COALESCE(metadata, '{}'), '$.' || ?1, ?2),
                updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?3",
            params![key, value, task_id],
        )?;
        Ok(())
    }

    /// Remove a single field from the task's metadata JSON (#959).
    ///
    /// Uses SQLite's `json_remove()` to delete the key. No-op if the key doesn't exist.
    pub fn remove_task_metadata_field(&self, task_id: &str, key: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET
                metadata = json_remove(COALESCE(metadata, '{}'), '$.' || ?1),
                updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2",
            params![key, task_id],
        )?;
        Ok(())
    }

    /// Get a single metadata field from a task's metadata JSON as a string.
    pub fn get_task_metadata_field(&self, task_id: &str, key: &str) -> Result<Option<String>> {
        let result: Option<String> = self.conn.query_row(
            "SELECT json_extract(metadata, '$.' || ?1) FROM tasks WHERE id = ?2",
            params![key, task_id],
            |row| row.get(0),
        )?;
        Ok(result)
    }

    /// Check if all siblings of a completed task are done. If so, atomically
    /// claim the parent task for dispatch.
    ///
    /// Returns `Some(parent_id)` when the parent was claimed, `None` otherwise.
    /// Uses a single SQLite transaction to prevent races.
    pub fn try_complete_parent_on_sibling_done(&self, task_id: &str) -> Result<Option<String>> {
        let tx = self.conn.unchecked_transaction()?;

        // 1. Get parent_task_id for this task (no agent_id filter — parent-child
        //    relationships are structural, and team task trees have mixed agent_ids)
        let parent_id: Option<String> = tx
            .query_row(
                "SELECT parent_task_id FROM tasks WHERE id = ?1",
                params![task_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();

        let parent_id = match parent_id {
            Some(id) => id,
            None => return Ok(None),
        };

        // 2. Count incomplete siblings (same parent, not in terminal state).
        // No agent_id filter — siblings in a team task tree have different agent_ids.
        let incomplete: i64 = tx.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE parent_task_id = ?1
             AND status NOT IN ('completed','failed','cancelled','expired','delivered')",
            params![&parent_id],
            |row| row.get(0),
        )?;

        if incomplete > 0 {
            tx.commit()?;
            return Ok(None);
        }

        // 3. Atomically claim parent task (only if still pending).
        // No agent_id filter — the parent may have a different agent_id than children.
        let changed = tx.execute(
            "UPDATE tasks SET status = 'in_progress', fired_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1 AND status = 'pending'",
            params![&parent_id],
        )?;

        tx.commit()?;

        if changed > 0 {
            Ok(Some(parent_id))
        } else {
            Ok(None) // already claimed by another thread
        }
    }

    /// Get all child tasks for a given parent task.
    /// No agent_id filter — team task trees have children with different agent_ids.
    pub fn get_child_tasks(&self, parent_task_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE parent_task_id = ?1
             ORDER BY created_at ASC",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![parent_task_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Get all descendant tasks for a given root task (recursive).
    /// Returns all tasks in the subtree below root_task_id, excluding the root itself.
    /// No agent_id filter — team task trees have children with different agent_ids.
    /// Depth guard (depth <= 3) mirrors the CHECK constraint as defense-in-depth.
    pub fn get_task_descendants(&self, root_task_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "WITH RECURSIVE descendant_ids(id) AS (
                 SELECT id FROM tasks WHERE parent_task_id = ?1
                 UNION ALL
                 SELECT t.id FROM tasks t
                 JOIN descendant_ids d ON t.parent_task_id = d.id
                 WHERE t.depth <= 3
             )
             SELECT {cols} FROM tasks
             WHERE id IN (SELECT id FROM descendant_ids)
             ORDER BY created_at ASC",
            cols = Self::TASK_COLUMNS,
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![root_task_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Check if any active callback tasks exist for a task OTHER than the excluded one,
    /// filtered by dispatch class.
    ///
    /// Returns `Some((parent_task_id, callback_task_id))` if an active callback task exists
    /// whose parent differs from `excluded_parent_id` and whose dispatch class matches.
    /// Used by the per-class dispatch guard to enforce one-slot-per-class (#583, #1001).
    ///
    /// Pre-v34 rows with `dispatch_class IS NULL` are treated as `'implement'` via
    /// `COALESCE` — no application-layer NULL coercion needed (architect NF1).
    ///
    /// mika#1163: Excludes `:deferred` wrappers via `label NOT LIKE '%:deferred'`.
    /// Deferred wrappers are pending markers waiting for promotion, NOT active
    /// dispatches occupying a slot. Without this exclusion, two parents each
    /// holding a pending wrapper deadlock — every dispatch attempt from one
    /// wrapper sees the OTHER as slot-occupied and registers yet another
    /// wrapper. Mirrors the equivalent clause in `has_any_active_callback`
    /// (mika#1070), which the engine-level promotion backstop uses.
    /// Returns `(parent_task_id, callback_id, callback_label)` of the blocking
    /// callback, or `None` if no conflicting dispatch exists. The label enables
    /// callers to derive `blocker_kind` for rejection JSON (#1172 W3).
    pub fn has_active_callback_tasks_excluding(
        &self,
        excluded_parent_id: &str,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<Option<BlockingDispatch>> {
        let mut stmt = self.conn.prepare(
            // mika#1948: the blocking row's `dispatcher_source` rides along so a
            // rejection can name WHO holds the slot, not just that it is held.
            // Deliberately NOT wrapped in COALESCE here — the on-disk NULL of a
            // pre-v51 row is a real distinction ("unknown, predates the column")
            // and collapsing it to 'mika_dev' at this depth would manufacture a
            // certainty the row does not carry. Callers that need a default
            // apply it themselves.
            "SELECT t.parent_task_id, t.id, t.label, p.dispatcher_source
             FROM tasks t
             LEFT JOIN tasks p ON p.id = t.parent_task_id
             WHERE t.trigger_type = 'callback'
               AND t.status IN ('pending', 'in_progress')
               AND t.parent_task_id IS NOT NULL
               AND t.parent_task_id != ?1
               AND t.agent_id = ?2
               AND COALESCE(t.dispatch_class, 'implement') = ?3
               AND t.label NOT LIKE '%:deferred'
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![excluded_parent_id, agent_id, dispatch_class])?;
        if let Some(row) = rows.next()? {
            Ok(Some(BlockingDispatch {
                parent_task_id: row.get(0)?,
                callback_task_id: row.get(1)?,
                label: row.get(2)?,
                dispatcher_source: row.get(3)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// How many *distinct* dispatches of this class are active for this agent,
    /// excluding `excluded_parent_id` (mika#2160).
    ///
    /// The companion to [`Database::has_active_callback_tasks_excluding`], with
    /// the identical WHERE clause. It exists because a configurable cap needs a
    /// number and the predicate only ever answered "is there at least one" —
    /// the shape of a cap of exactly 1. KTD5: the existing signature is left
    /// alone, it has callers in production, in tests, and an async twin.
    ///
    /// `COUNT(DISTINCT parent_task_id)`, not `COUNT(*)`: a dispatch is a
    /// parent, and a parent that happens to carry two callback rows of the same
    /// class is still one dispatch. Counting rows would fill the cap with a
    /// single dispatch and re-serialize the class behind the operator's back.
    pub fn count_active_callback_tasks_excluding(
        &self,
        excluded_parent_id: &str,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT t.parent_task_id)
             FROM tasks t
             WHERE t.trigger_type = 'callback'
               AND t.status IN ('pending', 'in_progress')
               AND t.parent_task_id IS NOT NULL
               AND t.parent_task_id != ?1
               AND t.agent_id = ?2
               AND COALESCE(t.dispatch_class, 'implement') = ?3
               AND t.label NOT LIKE '%:deferred'",
            params![excluded_parent_id, agent_id, dispatch_class],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Record which dispatcher initiated a task (mika#1948).
    ///
    /// A separate write rather than a `NewTask` field on purpose: `NewTask` has
    /// 175 construction sites, and all but a handful are the autonomous loop,
    /// whose correct value is exactly the NULL default. Making every one of them
    /// restate "mika_dev" would be churn that buries the few sites where the
    /// answer is genuinely something else.
    ///
    /// Callers pass the value they mean; leaving it unset keeps the row NULL,
    /// which reads as `mika_dev`. Rejects unknown values rather than storing
    /// them — the CHECK constraint would too, but failing here names the
    /// offending value.
    pub fn set_task_dispatcher_source(
        &self,
        task_id: &str,
        agent_id: &str,
        dispatcher_source: &str,
    ) -> Result<bool> {
        if !matches!(dispatcher_source, "mika_dev" | "mika_manager" | "operator") {
            anyhow::bail!(
                "unknown dispatcher_source '{dispatcher_source}' — \
                 expected one of: mika_dev, mika_manager, operator"
            );
        }
        let n = self.conn.execute(
            "UPDATE tasks SET dispatcher_source = ?3 WHERE id = ?1 AND agent_id = ?2",
            params![task_id, agent_id, dispatcher_source],
        )?;
        Ok(n > 0)
    }

    /// Whether an operator-sourced task is waiting to run in this dispatch class
    /// (mika#1948 AC3).
    ///
    /// Backs the operator-priority rule in `promote_pending_deferred_if_idle`:
    /// when the operator has drafted work in a class, an automatic
    /// deferred-wrapper promotion must not take the slot out from under it.
    /// Scoped to `pending` — an operator task that is already `in_progress`
    /// holds the slot through the ordinary active-callback check instead.
    pub fn has_pending_operator_task_for_class(
        &self,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1
               AND status = 'pending'
               AND dispatcher_source = 'operator'
               AND COALESCE(dispatch_class, 'implement') = ?2",
            params![agent_id, dispatch_class],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Atomically claim the exec slot for `(agent_id, dispatch_class)`
    /// (mika#1948 — Porte 2).
    ///
    /// # Why a lease exists at all
    ///
    /// `has_active_callback_tasks_excluding` only ever *checked* the slot. It is
    /// a bare SELECT, and on `None` the dispatch path simply proceeds — the row
    /// that makes the slot observably held (the callback task) is written much
    /// later, by the caller. In between, `validate_dispatch_readiness` performs
    /// several GitHub round-trips (issue body, open-PR check, grooming markers),
    /// each with a 10s timeout. So the gap between "the slot looked free" and
    /// "the slot is held" is measured in seconds, and there are four production
    /// entry points into that function.
    ///
    /// Two dispatchers could therefore both read "free" and both proceed. A slot
    /// that two claimants can simultaneously believe they hold is not
    /// arbitration — it is a convention. This makes the claim a FACT: the
    /// PRIMARY KEY on `(agent_id, dispatch_class)` means a second claimant's
    /// INSERT collides rather than races.
    ///
    /// # Semantics
    ///
    /// One statement, inside an IMMEDIATE transaction. The lease is taken when
    /// no row exists, when the existing row has expired, or when the caller is
    /// re-entering with its own `holder_task_id` (a retry refreshes its own
    /// lease instead of deadlocking against itself). Otherwise the current
    /// holder is returned and the caller must not dispatch.
    ///
    /// # On the TTL, and why it is short
    ///
    /// The lease guards one narrow window: from the moment validation completes
    /// to the moment the callback row exists. That is a process spawn — seconds.
    /// The TTL must exceed that, and must stay far below the duration of a real
    /// dispatch (minutes), so that a lease can never outlive the work it
    /// guarded and block a slot that has legitimately freed. Once the callback
    /// row exists, the ordinary active-callback check is the durable holder and
    /// the lease is redundant; letting it lapse is the intended end of life.
    ///
    /// Expiry is what keeps this fail-closed without being loop-breaking: a
    /// dispatcher that dies mid-claim stalls its class for at most one TTL
    /// rather than forever.
    pub fn try_acquire_dispatch_slot(
        &mut self,
        agent_id: &str,
        dispatch_class: &str,
        holder_task_id: &str,
        dispatcher_source: Option<&str>,
        ttl_secs: i64,
        max_slots: i64,
    ) -> Result<SlotClaim> {
        let now = Utc::now();
        let now_s = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let expires_s = (now + Duration::seconds(ttl_secs))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // 1. Idempotent re-claim. Before v52 this was the `OR holder_task_id =
        //    ?3` arm of the upsert; with several slots it has to come first, or
        //    a holder that already owns slot 1 would take a free slot 0 as well
        //    and spend two slots on one dispatch.
        let refreshed = tx.execute(
            "UPDATE dispatch_slot_leases
                SET dispatcher_source = ?4, acquired_at = ?5, expires_at = ?6
              WHERE agent_id = ?1 AND dispatch_class = ?2 AND holder_task_id = ?3",
            params![
                agent_id,
                dispatch_class,
                holder_task_id,
                dispatcher_source,
                now_s,
                expires_s
            ],
        )?;
        if refreshed > 0 {
            tx.commit()?;
            return Ok(SlotClaim::Acquired);
        }

        // 2. Take the first index whose lease is free or expired. `max_slots <=
        //    0` is the explicit disable sentinel (KTD3, the grammar of
        //    `MIKA_AUTO_PULL_MAX_BEHIND` and `MAX_REDRIVES_ENV`): the cap is
        //    lifted, so a fresh index is appended rather than contended for.
        if max_slots <= 0 {
            // Reclaim an expired index before minting a new one. Without this,
            // every claim under the disable sentinel appends a row that nothing
            // ever removes: `release_dispatch_slot` has no production caller,
            // and before v52 the two-column PRIMARY KEY forced each claim to
            // overwrite the single row. `slot_index` is what makes unbounded
            // accumulation representable, so the reclaim has to be explicit.
            // The table is then bounded by the high-water mark of *live*
            // leases, not by the total number of dispatches ever made.
            let reclaimed: Option<i64> = tx
                .query_row(
                    "SELECT MIN(slot_index) FROM dispatch_slot_leases
                      WHERE agent_id = ?1 AND dispatch_class = ?2
                        AND expires_at <= ?3",
                    params![agent_id, dispatch_class, now_s],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();

            if let Some(idx) = reclaimed {
                tx.execute(
                    "UPDATE dispatch_slot_leases
                        SET holder_task_id = ?4, dispatcher_source = ?5,
                            acquired_at = ?6, expires_at = ?7
                      WHERE agent_id = ?1 AND dispatch_class = ?2 AND slot_index = ?3",
                    params![
                        agent_id,
                        dispatch_class,
                        idx,
                        holder_task_id,
                        dispatcher_source,
                        now_s,
                        expires_s
                    ],
                )?;
                tx.commit()?;
                return Ok(SlotClaim::Acquired);
            }

            let next: i64 = tx.query_row(
                "SELECT COALESCE(MAX(slot_index), -1) + 1 FROM dispatch_slot_leases
                  WHERE agent_id = ?1 AND dispatch_class = ?2",
                params![agent_id, dispatch_class],
                |row| row.get(0),
            )?;
            tx.execute(
                "INSERT INTO dispatch_slot_leases
                     (agent_id, dispatch_class, slot_index, holder_task_id,
                      dispatcher_source, acquired_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    agent_id,
                    dispatch_class,
                    next,
                    holder_task_id,
                    dispatcher_source,
                    now_s,
                    expires_s
                ],
            )?;
            tx.commit()?;
            return Ok(SlotClaim::Acquired);
        }

        // A cap lowered while leases are live can leave rows ABOVE the new
        // ceiling. Scanning `0..max_slots` alone would not see them: at cap 2
        // slots 0 and 1 are taken, the operator reverts to 1, slot 0 expires —
        // and a fresh claim would win slot 0 while slot 1 is still live, so two
        // leases exist under a cap of one. Count what is live first; the index
        // scan below then only has to find a free slot, not to police the cap.
        let live: i64 = tx.query_row(
            "SELECT COUNT(*) FROM dispatch_slot_leases
              WHERE agent_id = ?1 AND dispatch_class = ?2 AND expires_at > ?3",
            params![agent_id, dispatch_class, now_s],
            |row| row.get(0),
        )?;
        if live < max_slots {
            for slot_index in 0..max_slots {
                let changed = tx.execute(
                    "INSERT INTO dispatch_slot_leases
                     (agent_id, dispatch_class, slot_index, holder_task_id,
                      dispatcher_source, acquired_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(agent_id, dispatch_class, slot_index) DO UPDATE SET
                     holder_task_id    = excluded.holder_task_id,
                     dispatcher_source = excluded.dispatcher_source,
                     acquired_at       = excluded.acquired_at,
                     expires_at        = excluded.expires_at
                 WHERE dispatch_slot_leases.expires_at <= ?6",
                    params![
                        agent_id,
                        dispatch_class,
                        slot_index,
                        holder_task_id,
                        dispatcher_source,
                        now_s,
                        expires_s
                    ],
                )?;
                if changed > 0 {
                    tx.commit()?;
                    return Ok(SlotClaim::Acquired);
                }
            }
        }

        // Every slot collided with a live lease, or the cap is already met. Read a holder inside the same
        // transaction so the reported blocker is one we actually lost to; the
        // lowest live index is chosen so the answer is deterministic.
        let held = tx
            .query_row(
                "SELECT holder_task_id, dispatcher_source, expires_at
                 FROM dispatch_slot_leases
                 WHERE agent_id = ?1 AND dispatch_class = ?2 AND expires_at > ?3
                 ORDER BY slot_index ASC
                 LIMIT 1",
                params![agent_id, dispatch_class, now_s],
                |row| {
                    Ok(SlotClaim::Held {
                        holder_task_id: row.get(0)?,
                        dispatcher_source: row.get(1)?,
                        expires_at: row.get(2)?,
                    })
                },
            )
            .optional()?;
        tx.commit()?;

        // A row that vanished between the failed INSERT and this SELECT cannot
        // happen inside one IMMEDIATE transaction, but the query is fallible in
        // principle. Report contention rather than inventing a free slot —
        // fail-closed is the whole point of the lease.
        Ok(held.unwrap_or(SlotClaim::Held {
            holder_task_id: "<unknown>".to_string(),
            dispatcher_source: None,
            expires_at: expires_s,
        }))
    }

    /// Release a slot lease, but only if `holder_task_id` still owns it
    /// (mika#1948).
    ///
    /// The holder predicate matters: without it, a late release from a
    /// dispatcher whose lease already expired and was re-taken by someone else
    /// would free a slot it no longer owns. Returns whether a row was removed.
    pub fn release_dispatch_slot(
        &self,
        agent_id: &str,
        dispatch_class: &str,
        holder_task_id: &str,
    ) -> Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM dispatch_slot_leases
             WHERE agent_id = ?1 AND dispatch_class = ?2 AND holder_task_id = ?3",
            params![agent_id, dispatch_class, holder_task_id],
        )?;
        Ok(n > 0)
    }

    /// A live holder of a slot lease for this class, if any (mika#1948).
    /// Expired leases read as free — they are reclaimable by definition.
    ///
    /// Since mika#2160 a class may hold several live leases, so this returns
    /// the **lowest-indexed** one. The ordering is explicit rather than left to
    /// SQLite's scan order: before v52 the PRIMARY KEY made "the holder"
    /// singular and the question could not arise, and a caller that inherited
    /// that assumption would otherwise get a different answer run to run.
    pub fn dispatch_slot_lease_holder(
        &self,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<Option<(String, Option<String>)>> {
        let now_s = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let row = self
            .conn
            .query_row(
                "SELECT holder_task_id, dispatcher_source
                 FROM dispatch_slot_leases
                 WHERE agent_id = ?1 AND dispatch_class = ?2 AND expires_at > ?3
                 ORDER BY slot_index ASC
                 LIMIT 1",
                params![agent_id, dispatch_class, now_s],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(row)
    }

    /// Claim the worktree of `(repo, issue_number)` (mika#2192).
    ///
    /// # Why a claim exists at all
    ///
    /// `dispatch_slot_leases` says WHO may run. It says nothing about WHERE,
    /// and the where is a shared directory: `derive-worktree-path` is a pure
    /// function of the branch, so two actors on one ticket land on the same
    /// path by construction. An orchestrator session is not a dispatch and
    /// holds no lease, so the lease can never see it — yet it is precisely the
    /// actor of the founding incident.
    ///
    /// # Semantics
    ///
    /// One IMMEDIATE transaction. The claim is taken when no row exists, when
    /// the existing row has expired, or when the caller re-enters with its own
    /// `owner_id` (idempotent refresh — a resumed dispatch must not deadlock
    /// against the claim it took on its first pass). Otherwise the live holder
    /// is returned and the caller must not enter the directory.
    ///
    /// # On liveness
    ///
    /// `expires_at > now`, and nothing else. No `pid`, no `pgrep -f` (vacuous
    /// in this harness), no `/proc` (the agent may be containerised, so the
    /// host's process table is not a surface to build on). The TTL is what
    /// keeps fail-closed from becoming loop-breaking: a claimant that dies
    /// mid-run blocks its own ticket for one TTL, never forever, and the CLI
    /// `mika worktree release` is the manual escape.
    pub fn try_claim_worktree(
        &mut self,
        repo: &str,
        issue_number: i64,
        owner_kind: &str,
        owner_id: &str,
        owner_label: Option<&str>,
        ttl_secs: i64,
    ) -> Result<WorktreeClaimOutcome> {
        let now = Utc::now();
        let now_s = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let expires_s = (now + Duration::seconds(ttl_secs))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Take it, refresh it, or lose it — in one statement, so a second
        // claimant collides on the PRIMARY KEY instead of racing a SELECT.
        // The `WHERE` is what makes losing possible: without it the upsert
        // would silently steal a live claim, which is the defect, not the fix.
        let changed = tx.execute(
            "INSERT INTO worktree_claims
                 (repo, issue_number, owner_kind, owner_id, owner_label,
                  claimed_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(repo, issue_number) DO UPDATE SET
                 owner_kind  = excluded.owner_kind,
                 owner_id    = excluded.owner_id,
                 owner_label = excluded.owner_label,
                 claimed_at  = excluded.claimed_at,
                 expires_at  = excluded.expires_at
             WHERE worktree_claims.expires_at <= ?6
                OR worktree_claims.owner_id = ?4",
            params![
                repo,
                issue_number,
                owner_kind,
                owner_id,
                owner_label,
                now_s,
                expires_s
            ],
        )?;

        // Read the row back inside the same transaction either way: on success
        // it is the claim we just wrote, on failure it is the holder we
        // actually lost to — never a second read that could see a third state.
        let row = tx
            .query_row(
                "SELECT repo, issue_number, owner_kind, owner_id, owner_label,
                        claimed_at, expires_at
                 FROM worktree_claims
                 WHERE repo = ?1 AND issue_number = ?2",
                params![repo, issue_number],
                |row| {
                    Ok(WorktreeClaim {
                        repo: row.get(0)?,
                        issue_number: row.get(1)?,
                        owner_kind: row.get(2)?,
                        owner_id: row.get(3)?,
                        owner_label: row.get(4)?,
                        claimed_at: row.get(5)?,
                        expires_at: row.get(6)?,
                    })
                },
            )
            .optional()?;
        tx.commit()?;

        match row {
            Some(claim) if changed > 0 => Ok(WorktreeClaimOutcome::Claimed(claim)),
            Some(claim) => Ok(WorktreeClaimOutcome::HeldByOther(claim)),
            // A row that vanished between the write and the read cannot happen
            // inside one IMMEDIATE transaction. Report contention rather than
            // inventing a free worktree — fail-closed is the whole point.
            None => Ok(WorktreeClaimOutcome::HeldByOther(WorktreeClaim {
                repo: repo.to_string(),
                issue_number,
                owner_kind: "unknown".to_string(),
                owner_id: "<unknown>".to_string(),
                owner_label: None,
                claimed_at: now_s,
                expires_at: expires_s,
            })),
        }
    }

    /// The live owner of `(repo, issue_number)`'s worktree, if any (mika#2192).
    ///
    /// Filters on `expires_at > now`, so `None` **is** the answer "expired or
    /// absent" — a caller never has to re-derive liveness and never has two
    /// ways to get it wrong.
    pub fn worktree_claim_holder(
        &self,
        repo: &str,
        issue_number: i64,
    ) -> Result<Option<WorktreeClaim>> {
        let now_s = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let row = self
            .conn
            .query_row(
                "SELECT repo, issue_number, owner_kind, owner_id, owner_label,
                        claimed_at, expires_at
                 FROM worktree_claims
                 WHERE repo = ?1 AND issue_number = ?2 AND expires_at > ?3",
                params![repo, issue_number, now_s],
                |row| {
                    Ok(WorktreeClaim {
                        repo: row.get(0)?,
                        issue_number: row.get(1)?,
                        owner_kind: row.get(2)?,
                        owner_id: row.get(3)?,
                        owner_label: row.get(4)?,
                        claimed_at: row.get(5)?,
                        expires_at: row.get(6)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// Release a worktree claim, but only if `owner_id` still owns it
    /// (mika#2192).
    ///
    /// The owner predicate is the same one `release_dispatch_slot` carries and
    /// for the same reason: a late release from an actor whose claim already
    /// expired and was re-taken by someone else would free a directory it no
    /// longer owns — handing a live writer's worktree to the next dispatch.
    pub fn release_worktree_claim(
        &self,
        repo: &str,
        issue_number: i64,
        owner_id: &str,
    ) -> Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM worktree_claims
             WHERE repo = ?1 AND issue_number = ?2 AND owner_id = ?3",
            params![repo, issue_number, owner_id],
        )?;
        Ok(n > 0)
    }

    /// Drop lapsed claims (mika#2192). Returns how many rows went.
    ///
    /// Purely hygienic: `worktree_claim_holder` and `try_claim_worktree`
    /// already treat an expired row as absent, so nothing depends on this
    /// having run. It keeps the table bounded by live claims rather than by
    /// every ticket ever dispatched.
    pub fn purge_expired_worktree_claims(&self) -> Result<usize> {
        let now_s = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let n = self.conn.execute(
            "DELETE FROM worktree_claims WHERE expires_at <= ?1",
            params![now_s],
        )?;
        Ok(n)
    }

    /// Check whether the given parent task has any active (pending/in_progress)
    /// non-deferred callback child. Used by the R9 no-op wrapper detection (#1172)
    /// to determine if a deferred wrapper completed without spawning a real dispatch.
    pub fn has_non_deferred_active_callback_child(&self, parent_task_id: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE parent_task_id = ?1
               AND trigger_type = 'callback'
               AND status IN ('pending', 'in_progress')
               AND label NOT LIKE '%:deferred'",
            params![parent_task_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Check whether the autonomous `dev-groom` loop has really groomed a
    /// GitHub issue. Used by the dispatch-classification gate (#1620) so that
    /// grooming markers in an issue body are only trusted when a groom actually
    /// ran and converged — not when they were pre-stamped by hand.
    ///
    /// **Proof = the groom callback row (mika#2287).** The parent dispatch row
    /// is not durable evidence: the structural producers write the bare issue
    /// URL (mika#1572), and the engine flips the parent's `dispatch_class`
    /// groom→implement before it ever reaches a terminal status (mika#1614
    /// task-reuse). The callback child survives both: it keeps
    /// `dispatch_class='groom'` (derived from the `skill` input), reaches
    /// `completed` then `delivered`, and its `result` is the dispatch-lib RESULT
    /// written by `POST /tasks/{id}/complete`, which carries
    /// [`crate::task_state::tasks::GROOM_SUCCESS_MARKER`] on convergence — the
    /// same marker `try_dispatch_pilot_after_groom_success` already trusts.
    ///
    /// One query serves every groom producer: the parent `reference_url` is
    /// accepted in bare form or with the legacy
    /// [`crate::task_state::tasks::GROOM_PHASE_SUFFIX`] the LLM-driven path
    /// appends. Nothing is appended to `issue_url` by the caller.
    ///
    /// Read-only, fail-closed: a single `SELECT`; a DB error propagates as
    /// `Err` and the caller refuses the dispatch. A proof pruned by
    /// `prune_completed_tasks` (30-day retention, either row) is a refusal.
    pub fn has_completed_groom_for_issue(&self, agent_id: &str, issue_url: &str) -> Result<bool> {
        let legacy_groom_url = format!(
            "{}{}",
            issue_url,
            crate::task_state::tasks::GROOM_PHASE_SUFFIX
        );
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks child
             JOIN tasks parent ON child.parent_task_id = parent.id
             WHERE child.agent_id = ?1
               AND child.trigger_type = 'callback'
               AND child.dispatch_class = 'groom'
               AND child.status IN ('completed', 'delivered')
               AND instr(child.result, ?4) > 0
               AND parent.reference_url IN (?2, ?3)",
            params![
                agent_id,
                issue_url,
                legacy_groom_url,
                crate::task_state::tasks::GROOM_SUCCESS_MARKER
            ],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Count pending callback tasks for a given team run with depth > 1.
    /// Used to detect grandchild long-running tasks spawned during a team run.
    pub fn count_pending_callback_tasks_by_team_run(&self, team_run_id: &str) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE team_run_id = ?1
               AND trigger_type = 'callback'
               AND status = 'pending'
               AND depth > 1",
            params![team_run_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Count pending deferred-dispatch callback tasks for this agent (mika#1011).
    /// Used by the executor flood-cap check before registering a new deferred callback.
    pub fn count_pending_deferred_callbacks(&self, agent_id: &str) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND status = 'pending'
               AND label = 'long_running:run_claude_pilot:deferred'",
            params![agent_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Promote the next pending deferred-dispatch callback for dispatch (FIFO).
    ///
    /// Sets `next_fire_at` to now and marks with a synthetic result so the task
    /// engine's periodic scan picks it up and routes through `dispatch_resume_agent`
    /// within one tick (~1 second). Returns `Some(task_id)` if a task was promoted,
    /// `None` if no pending deferred callback existed. Called by the dispatcher
    /// after a blocking callback completes (mika#1011).
    pub fn promote_next_deferred_callback(&self, agent_id: &str) -> Result<Option<String>> {
        // First, find the candidate task ID
        let candidate_id: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM tasks
                 WHERE agent_id = ?1
                   AND trigger_type = 'callback'
                   AND status = 'pending'
                   AND label = 'long_running:run_claude_pilot:deferred'
                 ORDER BY created_at ASC
                 LIMIT 1",
                params![agent_id],
                |row| row.get(0),
            )
            .optional()?;

        let Some(task_id) = candidate_id else {
            return Ok(None);
        };

        let now = crate::timestamp::now();
        let n = self.conn.execute(
            "UPDATE tasks
             SET status = 'completed',
                 result = 'deferred dispatch slot freed',
                 completed_at = ?2,
                 next_fire_at = ?2,
                 updated_at = ?2
             WHERE id = ?1
               AND status = 'pending'",
            params![task_id, now],
        )?;
        Ok(if n > 0 { Some(task_id) } else { None })
    }

    /// Class-scoped sibling of `promote_next_deferred_callback`. Promotes the
    /// oldest pending deferred wrapper matching the given `dispatch_class`.
    /// Returns `Some(task_id)` if a task was promoted, `None` otherwise. Used by
    /// the periodic backstop's per-class iteration (mika#1175). Pre-v34 NULL
    /// rows treated as 'implement' via COALESCE (matches
    /// `has_active_callback_tasks_excluding` semantics).
    pub fn promote_next_deferred_callback_for_class(
        &self,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<Option<String>> {
        // First, find the candidate task ID
        let candidate_id: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM tasks
                 WHERE agent_id = ?1
                   AND trigger_type = 'callback'
                   AND status = 'pending'
                   AND label = 'long_running:run_claude_pilot:deferred'
                   AND COALESCE(dispatch_class, 'implement') = ?2
                 ORDER BY created_at ASC
                 LIMIT 1",
                params![agent_id, dispatch_class],
                |row| row.get(0),
            )
            .optional()?;

        let Some(task_id) = candidate_id else {
            return Ok(None);
        };

        let now = crate::timestamp::now();
        let n = self.conn.execute(
            "UPDATE tasks
             SET status = 'completed',
                 result = 'deferred dispatch slot freed',
                 completed_at = ?2,
                 next_fire_at = ?2,
                 updated_at = ?2
             WHERE id = ?1
               AND status = 'pending'",
            params![task_id, now],
        )?;
        Ok(if n > 0 { Some(task_id) } else { None })
    }

    /// Returns true if any non-deferred callback task is in pending or in_progress
    /// status (i.e., a dispatch slot is occupied). Was used by the engine-level
    /// deferred-dispatch backstop (mika#1070). Post-mika#1175, the engine
    /// backstop calls `has_any_active_callback_for_class` per-class; this
    /// agent-wide form has no remaining production callers and is retained as
    /// a regression-test baseline + as a sibling reference for the class-scoped
    /// shape. See `has_any_active_callback_for_class` for production usage.
    pub fn has_any_active_callback(&self, agent_id: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND action_type = 'resume_agent'
               AND status IN ('pending', 'in_progress')
               AND label NOT LIKE '%:deferred'",
            params![agent_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Class-scoped sibling of `has_any_active_callback`. Returns `true` if any
    /// non-deferred callback task in the given `dispatch_class` is `pending` or
    /// `in_progress` (i.e., the per-class dispatch slot is occupied). Used by
    /// the periodic backstop's per-class slot check (mika#1175). Excludes
    /// `:deferred` wrappers (parity with mika#1163's symmetric exclusion).
    /// Pre-v34 NULL rows treated as 'implement' via COALESCE.
    pub fn has_any_active_callback_for_class(
        &self,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND action_type = 'resume_agent'
               AND status IN ('pending', 'in_progress')
               AND label NOT LIKE '%:deferred'
               AND COALESCE(dispatch_class, 'implement') = ?2",
            params![agent_id, dispatch_class],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// How many *dispatches* of this class occupy the per-class slots
    /// (mika#2160). Counting companion to
    /// [`Database::has_any_active_callback_for_class`], with the identical
    /// WHERE clause.
    ///
    /// It exists because that predicate is a boolean — the shape of a cap of
    /// exactly one — and it gates the deferred-promotion backstop
    /// (`engine.rs::promote_pending_deferred_if_idle`) and the force-promote
    /// override. Leaving them boolean while the dispatch guard learned to count
    /// would make a cap above 1 half-effective: new dispatches would be
    /// admitted, but a *deferred* one would wait for the class to fall back to
    /// **zero** rather than below the cap — the asymmetric-predicate drift
    /// mika#1163 names, rebuilt.
    ///
    /// `COUNT(DISTINCT COALESCE(parent_task_id, id))`: a dispatch is a parent,
    /// so two callback rows under one parent are one occupant. The COALESCE
    /// keeps a parentless row counting as itself, preserving the boolean
    /// predicate's answer for that shape rather than collapsing several such
    /// rows into one.
    pub fn count_active_callbacks_for_class(
        &self,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT COALESCE(parent_task_id, id)) FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND action_type = 'resume_agent'
               AND status IN ('pending', 'in_progress')
               AND label NOT LIKE '%:deferred'
               AND COALESCE(dispatch_class, 'implement') = ?2",
            params![agent_id, dispatch_class],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Force-promote the next pending deferred wrapper for a dispatch class,
    /// with fail-closed slot-availability semantics. Checks
    /// `has_any_active_callback_for_class()` first; if the slot is occupied,
    /// returns `RejectedSlotBusy` without mutating state. If free, promotes
    /// via `promote_next_deferred_callback_for_class()` and returns `Promoted`
    /// or `NoPendingWrapper`. Used by both the CLI verb and agent tool
    /// (mika#1453).
    ///
    /// Paired predicates: the slot check shares the same SQL predicate as
    /// `has_any_active_callback_for_class` — see mika#1163 for the
    /// asymmetric-predicate-drift failure class.
    pub fn force_promote_deferred_for_class(
        &self,
        agent_id: &str,
        dispatch_class: &str,
        max_slots: i64,
    ) -> Result<ForcePromoteResult> {
        // Slot-availability check (fail-closed) — same predicate as the
        // periodic backstop in engine.rs (mika#1163 parity). Since mika#2160
        // that predicate counts and compares against the class cap; `max_slots`
        // arrives as a parameter, the way `ttl_secs` does for the lease, so the
        // layer that owns the setting stays the one that reads the environment.
        let active = self.count_active_callbacks_for_class(agent_id, dispatch_class)?;
        if max_slots > 0 && active >= max_slots {
            // Fetch the blocker's label for the rejection message.
            let blocking_label: String = self
                .conn
                .query_row(
                    "SELECT label FROM tasks
                     WHERE agent_id = ?1
                       AND trigger_type = 'callback'
                       AND action_type = 'resume_agent'
                       AND status IN ('pending', 'in_progress')
                       AND label NOT LIKE '%:deferred'
                       AND COALESCE(dispatch_class, 'implement') = ?2
                     LIMIT 1",
                    params![agent_id, dispatch_class],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| "<unknown>".to_string());

            return Ok(ForcePromoteResult::RejectedSlotBusy { blocking_label });
        }

        match self.promote_next_deferred_callback_for_class(agent_id, dispatch_class)? {
            Some(task_id) => Ok(ForcePromoteResult::Promoted { task_id }),
            None => Ok(ForcePromoteResult::NoPendingWrapper),
        }
    }

    /// Returns the task ID of the active non-deferred callback occupying the
    /// per-class dispatch slot. Same SQL predicate as
    /// `has_any_active_callback_for_class` but `SELECT id LIMIT 1` instead of
    /// `SELECT COUNT(*)`. Used by the CLI override path to identify the blocker
    /// before cancellation (mika#1453).
    ///
    /// Paired predicate: shares the `:deferred` exclusion and COALESCE
    /// semantics with `has_any_active_callback_for_class` — keep in sync
    /// (mika#1163).
    pub fn find_active_callback_for_class(
        &self,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<Option<String>> {
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM tasks
                 WHERE agent_id = ?1
                   AND trigger_type = 'callback'
                   AND action_type = 'resume_agent'
                   AND status IN ('pending', 'in_progress')
                   AND label NOT LIKE '%:deferred'
                   AND COALESCE(dispatch_class, 'implement') = ?2
                 LIMIT 1",
                params![agent_id, dispatch_class],
                |row| row.get(0),
            )
            .optional()?;
        Ok(id)
    }

    pub fn prune_completed_tasks(&self, older_than_secs: i64) -> Result<usize> {
        let cutoff = timestamp::now_minus(Duration::seconds(older_than_secs));
        let n = self.conn.execute(
            "DELETE FROM tasks WHERE status IN ('completed','cancelled','expired','failed','delivered')
             AND completed_at IS NOT NULL AND completed_at < ?1",
            params![cutoff],
        )?;
        Ok(n)
    }

    pub fn get_tasks_by_status(&self, agent_id: &str, statuses: &[&str]) -> Result<Vec<Task>> {
        self.get_tasks_by_status_and_label(agent_id, statuses, None)
    }

    /// Like `get_tasks_by_status`, but with an optional `label_contains` substring filter.
    /// When `label_contains` is `Some`, only tasks whose label contains the substring
    /// (case-sensitive) are returned. The substring is parameterized (no SQL injection).
    pub fn get_tasks_by_status_and_label(
        &self,
        agent_id: &str,
        statuses: &[&str],
        label_contains: Option<&str>,
    ) -> Result<Vec<Task>> {
        if statuses.is_empty() {
            return Ok(vec![]);
        }
        let placeholders: String = (1..=statuses.len())
            .map(|i| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let label_param_idx = statuses.len() + 2; // next param index after agent_id + statuses
        let label_clause = if label_contains.is_some() {
            format!(" AND label LIKE '%' || ?{label_param_idx} || '%'")
        } else {
            String::new()
        };
        let sql = format!(
            "SELECT {} FROM tasks WHERE agent_id = ?1 AND status IN ({}){} ORDER BY created_at DESC",
            Self::TASK_COLUMNS,
            placeholders,
            label_clause,
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut bind: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(agent_id.to_string())];
        for s in statuses {
            bind.push(Box::new(s.to_string()));
        }
        if let Some(lc) = label_contains {
            bind.push(Box::new(lc.to_string()));
        }
        let refs: Vec<&dyn rusqlite::types::ToSql> = bind.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(refs.as_slice(), Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
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
                        ),
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

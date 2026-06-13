//! Task lifecycle types and Database query/write methods.

/// Default value for the `tasks.type` column. New tasks default to `"issue"`
/// unless the caller explicitly requests `"milestone"` or `"project"`.
pub const TASK_TYPE_ISSUE: &str = "issue";
pub const TASK_TYPE_MILESTONE: &str = "milestone";
pub const TASK_TYPE_PROJECT: &str = "project";

/// All valid values for the `tasks.type` column. Enforced by SQLite CHECK and by
/// the `create_task` tool boundary. Order is the documented enum order.
pub const VALID_TASK_TYPES: &[&str] = &[TASK_TYPE_ISSUE, TASK_TYPE_MILESTONE, TASK_TYPE_PROJECT];

#[derive(Debug, Clone)]
pub struct Task {
    pub id: String,
    pub agent_id: String,
    pub team_run_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub depth: i64,
    pub label: String,
    pub trigger_type: String,
    pub cron_expr: Option<String>,
    pub event_source: Option<String>,
    pub event_offset_secs: Option<i64>,
    pub condition_expr: Option<String>,
    pub next_fire_at: Option<String>,
    pub timeout_at: Option<String>,
    pub action_type: String,
    pub action_config: String,
    pub status: String,
    pub process_id: Option<i64>,
    pub input_context: Option<String>,
    pub result: Option<String>,
    pub created_by_session: Option<String>,
    pub created_trace_id: Option<String>,
    pub execution_trace_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub fired_at: Option<String>,
    pub completed_at: Option<String>,
    pub reference_url: Option<String>,
    pub source: Option<String>,
    pub metadata: Option<String>,
    /// Task kind: `"issue"`, `"milestone"`, or `"project"`. NOT NULL in the DB,
    /// defaulted to `"issue"` for backward compatibility (added in schema v23). See
    /// [`VALID_TASK_TYPES`].
    pub r#type: String,
    /// Dispatch class for long-running tasks: `"implement"` or `"groom"`. Nullable —
    /// pre-v34 rows have `NULL` (treated as `"implement"` by the per-class dispatch
    /// guard via `COALESCE`). Set on callback task creation based on the dispatched
    /// skill (#1001).
    pub dispatch_class: Option<String>,
}

/// Result of a force-promote attempt on a deferred dispatch wrapper (mika#1453).
/// Used by both the CLI verb and agent tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForcePromoteResult {
    /// The next pending deferred wrapper was promoted for dispatch.
    Promoted { task_id: String },
    /// The per-class dispatch slot is occupied by a non-deferred callback;
    /// promotion is refused (fail-closed). The `blocking_label` identifies
    /// the occupying task for operator diagnostics.
    RejectedSlotBusy { blocking_label: String },
    /// No pending deferred wrapper exists for the given dispatch class.
    NoPendingWrapper,
}

/// Split counts of active background callback tasks. `executing` = subprocess alive
/// (`process_id IS NOT NULL`), `queued` = waiting for dispatch slot (`process_id IS NULL`).
/// Used by TUI footer badge to distinguish `[1 running, 2 queued]` (#1057).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundTaskCounts {
    pub executing: usize,
    pub queued: usize,
}

/// A parent self_dev task left `in_progress` after its callback subtask
/// delivered without producing a PR. Used by the task engine reaper (#871).
#[derive(Debug, Clone)]
pub struct OrphanedParentTask {
    pub id: String,
    pub agent_id: String,
    pub callback_task_id: String,
    pub created_at: String,
}

/// A parent self_dev task left `in_progress` after its callback subtask
/// delivered WITH a `pr_url` (success indicator). Used by the success-side
/// engine backstop (mika#1162) — sibling shape to `OrphanedParentTask`.
/// Returned by `find_completable_parent_tasks_on_pr_url`.
///
/// Why this is a separate type from `OrphanedParentTask` despite near-identical
/// fields: the `pr_url` field is included in the SELECT so the completer can
/// build its audit-event reason string without a second DB round-trip (see plan
/// docs/plans/2026-05-17-001-fix-1162-...md, decision D1).
#[derive(Debug, Clone)]
pub struct CompletableParentTask {
    pub id: String,
    pub agent_id: String,
    pub callback_task_id: String,
    pub created_at: String,
    /// pr_url extracted from `parent.metadata.claude_pilot.pr_url`. Embedded
    /// in the SELECT so the completer doesn't need a second round-trip to
    /// build its audit-event reason string.
    pub pr_url: String,
}

/// Snapshot of a child task for the orphaned-parent reaper's structured log
/// event (`task_engine_reaper.evaluated`). Captures all children of a candidate
/// parent at kill time for post-incident diagnosis (mika#1126).
#[derive(Debug, Clone)]
pub struct ReaperChildSnapshot {
    pub id: String,
    pub dispatch_class: Option<String>,
    pub status: String,
    pub trigger_type: String,
    pub action_type: String,
    pub updated_at: String,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct NewTask {
    pub agent_id: String,
    pub team_run_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub depth: i64,
    pub label: String,
    pub trigger_type: String,
    pub cron_expr: Option<String>,
    pub event_source: Option<String>,
    pub event_offset_secs: Option<i64>,
    pub condition_expr: Option<String>,
    pub next_fire_at: Option<String>,
    pub timeout_at: Option<String>,
    pub action_type: String,
    pub action_config: String,
    pub input_context: Option<String>,
    pub created_by_session: Option<String>,
    pub created_trace_id: Option<String>,
    pub reference_url: Option<String>,
    pub source: Option<String>,
    pub metadata: Option<String>,
    /// Task kind. `None` (or an empty string) means "use the SQL default"
    /// (`"issue"`), preserving backward compatibility for existing callers. Values
    /// other than those in [`VALID_TASK_TYPES`] are rejected by the DB CHECK
    /// constraint; prefer validating at the tool boundary before INSERT.
    pub r#type: Option<String>,
    /// Dispatch class for per-class slot split (#1001). `None` means NULL in
    /// the DB (treated as `"implement"` via `COALESCE` by the dispatch guard).
    /// Set to `Some("groom")` for grooming dispatches.
    pub dispatch_class: Option<String>,
}

/// A single anomalous task state detected by the health check.
#[derive(Debug, Clone)]
pub struct TaskHealthAnomaly {
    pub task_id: String,
    pub label: String,
    pub trigger_type: String,
    pub status: String,
    /// One of: "stuck_callback", "stale_blocked", "failed_recurring", "long_running", "github_linked", "dispatch_failures", "dispatch_stale"
    pub anomaly_type: String,
    /// Human-readable age description (e.g., "3h 22m", "5 days").
    pub age_description: String,
    pub reference_url: Option<String>,
}

/// Aggregated task health summary for heartbeat prompt injection.
#[derive(Debug, Clone, Default)]
pub struct TaskHealthSummary {
    /// Active manual tasks (pending/in_progress/blocked).
    pub active_tasks: Vec<Task>,
    /// Anomalous task states across all trigger types, capped at [`health_thresholds::MAX_ANOMALIES`].
    pub anomalies: Vec<TaskHealthAnomaly>,
}

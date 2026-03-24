/// String constants for task status values stored in the `tasks.status` column.
///
/// Using constants instead of bare string literals prevents silent typos that
/// would compile but produce incorrect DB queries or match arms.
pub mod task_status {
    pub const PENDING: &str = "pending";
    pub const IN_PROGRESS: &str = "in_progress";
    pub const COMPLETED: &str = "completed";
    pub const FAILED: &str = "failed";
    pub const CANCELLED: &str = "cancelled";
    pub const EXPIRED: &str = "expired";
    pub const RECURRING_ACTIVE: &str = "recurring_active";
    pub const DELIVERED: &str = "delivered";
    pub const BLOCKED: &str = "blocked";
}

/// String constants for task action type values stored in the `tasks.action_type` column.
///
/// Using constants instead of bare string literals prevents silent typos that
/// would compile but produce incorrect DB queries or match arms.
pub mod action_type {
    pub const SEND_MESSAGE: &str = "send_message";
    pub const RUN_SKILL: &str = "run_skill";
    pub const INJECT_CONTEXT: &str = "inject_context";
    pub const RESUME_AGENT: &str = "resume_agent";
    pub const INVOKE_ORCHESTRATOR: &str = "invoke_orchestrator";
    pub const NONE: &str = "none";
}

/// Anomaly detection thresholds for task health checks (heartbeat prompt injection).
///
/// These are intentionally hardcoded constants. If per-customer configurability is
/// needed in the future, extract them into `Settings` or the preferences table.
pub mod health_thresholds {
    /// Callback task stuck at `completed` (not delivered) for longer than this is anomalous.
    pub const STUCK_CALLBACK_SECS: i64 = 600; // 10 minutes

    /// Manual work item `blocked` with no `updated_at` change for longer than this is anomalous.
    pub const STALE_BLOCKED_SECS: i64 = 86_400; // 24 hours

    /// Task `in_progress` for longer than this (when no `estimated_duration_secs`) is anomalous.
    pub const LONG_RUNNING_DEFAULT_SECS: i64 = 3_600; // 1 hour

    /// Maximum number of anomalies to include in the heartbeat prompt.
    pub const MAX_ANOMALIES: usize = 10;
}

/// String constants for task trigger type values stored in the `tasks.trigger_type` column.
pub mod trigger_type {
    pub const TIME: &str = "time";
    pub const RECURRING: &str = "recurring";
    pub const CALLBACK: &str = "callback";
    pub const USER_REPLY: &str = "user_reply";
    pub const EVENT: &str = "event";
    pub const CONDITION: &str = "condition";
    pub const MANUAL: &str = "manual";
    pub const A2A: &str = "a2a";
}

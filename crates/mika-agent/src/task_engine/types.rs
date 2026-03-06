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
}

/// String constants for task action type values stored in the `tasks.action_type` column.
///
/// Using constants instead of bare string literals prevents silent typos that
/// would compile but produce incorrect DB queries or match arms.
pub mod action_type {
    pub const SEND_MESSAGE: &str = "send_message";
    pub const RUN_SKILL: &str = "run_skill";
    pub const INJECT_CONTEXT: &str = "inject_context";
}

//! Task lifecycle, status transitions.
//!
//! Owns the `Task` struct family, `Database` query/write methods for task
//! CRUD, status-transition logic, discovery/lookup helpers, callback/orphan
//! management, and the `merge_metadata` shallow-merge utility.
//!
//! Per Foundation §6: task_state/ is the canonical owner of task lifecycle
//! and status-transition domain code.

pub mod metadata;
pub mod tasks;

pub use metadata::merge_metadata;
pub use tasks::{
    BackgroundTaskCounts, CompletableParentTask, ForcePromoteResult, NewTask, OrphanedParentTask,
    ReaperChildSnapshot, TASK_TYPE_ISSUE, TASK_TYPE_MILESTONE, TASK_TYPE_PROJECT, Task,
    TaskHealthAnomaly, TaskHealthSummary, VALID_TASK_TYPES,
};

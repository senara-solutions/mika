pub mod cron;
pub mod dispatcher;
pub mod engine;
pub mod queue;
pub mod types;

pub use dispatcher::TaskDispatcher;
pub use engine::TaskEngine;
pub use queue::QueuedTask;
pub use types::{action_type, task_status};

use crate::async_db::AsyncDatabase;
use crate::db::NewTask;
use tracing::{debug, info, warn};

/// Register a recurring task in the DB if one with the same label doesn't already exist.
///
/// Idempotent: no-op if a recurring task with `label` is already scheduled.
/// Used at startup to ensure built-in tasks (heartbeat, reflection) are always registered.
pub async fn ensure_recurring_task(
    db: &AsyncDatabase,
    label: &str,
    cron_expr: &str,
    action_config: &str,
) {
    let agent_id = db.agent_id.clone();
    let task = NewTask {
        agent_id,
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: label.to_string(),
        trigger_type: "recurring".to_string(),
        cron_expr: Some(cron_expr.to_string()),
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: None,
        timeout_at: None,
        action_type: action_type::RUN_SKILL.to_string(),
        action_config: action_config.to_string(),
        input_context: None,
        created_by_session: None,
    };

    match db.create_recurring_task_if_absent(task).await {
        Ok(Some(id)) => info!(label, task_id = %id, cron = cron_expr, "registered recurring task"),
        Ok(None) => debug!(label, "recurring task already registered, skipping"),
        Err(e) => warn!(label, error = %e, "failed to register recurring task"),
    }
}

pub mod cron;
pub mod dispatcher;
pub mod engine;
pub mod queue;
pub mod types;

pub use dispatcher::{DispatchError, TaskDispatcher};
pub use engine::TaskEngine;
pub use queue::QueuedTask;
pub use types::{action_type, task_status, trigger_type};

use crate::async_db::AsyncDatabase;
use crate::db::NewTask;
use chrono::Timelike;
use std::path::Path;
use tracing::{debug, info, warn};

/// Prune completed/failed/cancelled/expired tasks older than 30 days at startup
/// to prevent unbounded DB growth.
pub async fn prune_old_tasks(db: &AsyncDatabase) {
    // 30 days in seconds
    const THIRTY_DAYS_SECS: i64 = 30 * 24 * 60 * 60;
    if let Err(e) = db.prune_completed_tasks(THIRTY_DAYS_SECS).await {
        warn!("Failed to prune completed tasks: {}", e);
    }
}

/// Register a recurring task in the DB if one with the same label doesn't already exist.
/// If it already exists but the cron expression differs, update the cron and recompute
/// the next fire time.
///
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
        created_trace_id: None,
        reference_url: None,
        source: None,
    };

    match db.create_recurring_task_if_absent(task).await {
        Ok(Some(id)) => info!(label, task_id = %id, cron = cron_expr, "registered recurring task"),
        Ok(None) => {
            // Task already exists — check if the cron expression changed.
            if let Ok(Some(existing_cron)) = db.get_recurring_task_cron(label).await {
                if existing_cron != cron_expr {
                    let now = chrono::Utc::now().timestamp();
                    match cron::next_fire_from_cron(cron_expr, now) {
                        Ok(next_fire) => {
                            match db
                                .update_recurring_task_cron(label, cron_expr, next_fire)
                                .await
                            {
                                Ok(_) => {
                                    info!(label, old_cron = %existing_cron, new_cron = cron_expr, "updated recurring task cron")
                                }
                                Err(e) => {
                                    warn!(label, error = %e, "failed to update recurring task cron")
                                }
                            }
                        }
                        Err(e) => {
                            warn!(label, cron = cron_expr, error = %e, "failed to compute next fire time for updated cron")
                        }
                    }
                } else {
                    debug!(label, "recurring task already registered, skipping");
                }
            }
        }
        Err(e) => warn!(label, error = %e, "failed to register recurring task"),
    }
}

/// Build a UTC cron expression for reflection from identity.toml config + customer timezone.
/// Returns `None` if reflection is disabled or not configured.
pub async fn reflection_cron_for_agent(home_dir: &Path, db: &AsyncDatabase) -> Option<String> {
    let identity = crate::prompt::load_identity_async(home_dir).await;
    let config = identity.reflection.as_ref().filter(|c| c.enabled)?;
    let local_time = config.parse_time()?;

    let tz_str = if let Some(ref tz) = config.timezone {
        tz.clone()
    } else {
        db.get_customer_config("timezone")
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "UTC".to_string())
    };
    let tz: chrono_tz::Tz = match tz_str.parse() {
        Ok(tz) => tz,
        Err(_) => {
            warn!(timezone = %tz_str, "invalid timezone in customer config, skipping reflection registration");
            return None;
        }
    };

    // Convert local time to UTC: pick today's date, attach the local time,
    // convert to UTC, extract hour/minute.
    // NOTE: DST drift — the UTC offset is computed from today's date. For timezones with
    // daylight saving time, the reflection may fire ~1 hour early or late after a DST
    // transition until the next restart. This is acceptable for daily reflections.
    let today = chrono::Utc::now().with_timezone(&tz).date_naive();
    let local_dt = today.and_time(local_time);
    let utc_dt = local_dt.and_local_timezone(tz).earliest()?;
    let utc_time = utc_dt.with_timezone(&chrono::Utc).time();

    Some(format!("0 {} {} * * *", utc_time.minute(), utc_time.hour()))
}

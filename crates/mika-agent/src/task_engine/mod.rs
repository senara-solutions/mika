pub mod cron;
pub mod dispatcher;
pub mod engine;
pub mod liveness;
pub mod pilot_transcript;
pub mod process_kill;
pub mod process_liveness;
pub mod queue;
pub mod types;
pub mod worktree_activity;

pub use dispatcher::{DispatchError, TaskDispatcher};
pub use engine::{TaskEngine, promoted_wrapper_liveness_secs, stuck_pending_reaper_grace_secs};
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
///
/// **Calling this function *is* the config declaring the task must run** — the
/// callers are the boot paths that already evaluated the knob or the
/// `identity.toml` toggle. So a prior *config-driven* cancel of the same label
/// (the knob-off boot cancelled the row) must not survive as a veto: mika#2271
/// reverts it before re-registering. Terminal failures (`failed` / `expired`)
/// keep blocking through the mika#1742 refuse-to-zombie guard — only the
/// deliberate `cancelled` state is cleared here.
pub async fn ensure_recurring_task(
    db: &AsyncDatabase,
    label: &str,
    cron_expr: &str,
    action_config: &str,
) {
    // mika#2271: knob-off cancelled this label; the caller now says it must run.
    // Clear the config-cancel veto so the mika#1742 guard doesn't refuse the
    // re-registration below.
    match db.revert_config_cancel_recurring_task(label).await {
        Ok(0) => {}
        Ok(n) => {
            info!(
                label,
                rows = n,
                "reverted config cancel on recurring task (mika#2271)"
            )
        }
        Err(e) => warn!(label, error = %e, "failed to revert config cancel on recurring task"),
    }

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
        metadata: None,
        r#type: None,
        dispatch_class: None,
    };

    match db.create_recurring_task_if_absent(task).await {
        Ok(Some(id)) => info!(label, task_id = %id, cron = cron_expr, "registered recurring task"),
        Ok(None) => {
            // Task already exists — check if the cron expression changed.
            if let Ok(Some(existing_cron)) = db.get_recurring_task_cron(label).await {
                if existing_cron != cron_expr {
                    let now = crate::timestamp::now();
                    match cron::next_fire_from_cron(cron_expr, &now) {
                        Ok(next_fire) => {
                            match db
                                .update_recurring_task_cron(label, cron_expr, &next_fire)
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

/// Check if heartbeat is enabled for the agent from identity.toml config.
/// Returns `true` (default) unless `[heartbeat] enabled = false`.
pub async fn heartbeat_enabled_for_agent(home_dir: &Path) -> bool {
    let identity = crate::prompt::load_identity_async(home_dir).await;
    identity
        .heartbeat
        .as_ref()
        .map(|c| c.enabled)
        .unwrap_or(true)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    const FEEDER_LABEL: &str = "auto_pull_groomed";
    const FEEDER_CRON: &str = "0 */20 * * * *";
    const FEEDER_CONFIG: &str = r#"{"trigger":"auto_pull_groomed"}"#;

    fn test_async_db() -> AsyncDatabase {
        AsyncDatabase::new(Database::open_in_memory().unwrap())
    }

    async fn statuses_for(db: &AsyncDatabase, label: &str) -> Vec<String> {
        db.get_tasks_by_status(vec![
            "recurring_active".to_string(),
            "pending".to_string(),
            "in_progress".to_string(),
            "cancelled".to_string(),
            "failed".to_string(),
            "expired".to_string(),
        ])
        .await
        .unwrap()
        .into_iter()
        .filter(|t| t.label == label)
        .map(|t| t.status)
        .collect()
    }

    /// **Porte mika#2271 — test négatif.** Invariant : *un cycle knob-off →
    /// knob-on ré-inscrit le feeder*. Le knob-off boot annule la task
    /// récurrente ; le knob-on boot rappelle `ensure_recurring_task`, ce qui
    /// **est** la config déclarant que la task doit tourner. La garde
    /// refuse-to-zombie (mika#1742) ne doit pas transformer ce cancel
    /// délibéré en veto permanent.
    ///
    /// Sans le fix, le second `ensure_recurring_task` est refusé par la garde
    /// et le seul statut restant est `cancelled` — la boucle n'est plus
    /// réalimentée (symptôme mesuré le 2026-09-09).
    #[tokio::test]
    async fn knob_off_then_on_reregisters_the_feeder() {
        let db = test_async_db();

        // Boot 1 — knob absent : le feeder s'inscrit.
        ensure_recurring_task(&db, FEEDER_LABEL, FEEDER_CRON, FEEDER_CONFIG).await;
        assert_eq!(
            statuses_for(&db, FEEDER_LABEL).await,
            vec!["recurring_active".to_string()],
            "boot initial : le feeder doit être inscrit"
        );

        // Boot 2 — MIKA_DEV_AUTO_PULL=0 : la branche knob-off annule la row.
        db.cancel_recurring_task_by_label(FEEDER_LABEL)
            .await
            .unwrap();
        assert_eq!(
            statuses_for(&db, FEEDER_LABEL).await,
            vec!["cancelled".to_string()],
            "knob-off : la row doit être annulée"
        );

        // Boot 3 — knob retiré : le feeder doit revenir.
        ensure_recurring_task(&db, FEEDER_LABEL, FEEDER_CRON, FEEDER_CONFIG).await;

        let statuses = statuses_for(&db, FEEDER_LABEL).await;
        assert!(
            statuses.iter().any(|s| s == "recurring_active"),
            "knob-on : le feeder doit être RÉ-INSCRIT (recurring_active), \
             pas laissé cancelled — statuts observés : {statuses:?}"
        );
    }

    /// Contrôle positif de la garde : un `failed` récent bloque toujours la
    /// ré-inscription. L'exemption mika#2271 ne vise que le cancel de config —
    /// elle ne doit pas désarmer la protection anti-zombie de mika#1742.
    #[tokio::test]
    async fn recent_failed_still_blocks_reregistration() {
        let db = test_async_db();
        ensure_recurring_task(&db, FEEDER_LABEL, FEEDER_CRON, FEEDER_CONFIG).await;

        let label = FEEDER_LABEL.to_string();
        db.with_db(move |d| {
            d.conn.execute(
                "UPDATE tasks SET status = 'failed',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hour')
                 WHERE label = ?1",
                rusqlite::params![label],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        ensure_recurring_task(&db, FEEDER_LABEL, FEEDER_CRON, FEEDER_CONFIG).await;

        let statuses = statuses_for(&db, FEEDER_LABEL).await;
        assert_eq!(
            statuses,
            vec!["failed".to_string()],
            "un échec terminal récent doit toujours bloquer la ré-inscription \
             (mika#1742) — statuts observés : {statuses:?}"
        );
    }
}

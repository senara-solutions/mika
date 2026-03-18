use anyhow::Result;
use mika_agent::db::format_ts;

use crate::cli::{ReminderArgs, ReminderCommand};
use crate::init;

pub async fn run(args: ReminderArgs, agent_name: &str) -> Result<()> {
    let ctx = init::init_db_only_for_agent(agent_name)?;
    let db = &ctx.async_db;

    match args.command {
        None => {
            let reminders = db.get_user_visible_tasks().await?;
            if reminders.is_empty() {
                println!("\n  No pending reminders.\n");
            } else {
                println!("\n  Pending Reminders ({}):", reminders.len());
                for t in &reminders {
                    let short_id = &t.id[..12.min(t.id.len())];
                    let fire_at = t
                        .next_fire_at
                        .as_ref()
                        .map(|s| format_ts(s))
                        .unwrap_or_else(|| "unknown".to_string());
                    println!("    {}: \"{}\" at {}", short_id, t.label, fire_at);
                }
                println!();
            }
        }
        Some(ReminderCommand::Cancel { id }) => {
            let cancelled = db.cancel_task(&id).await?;
            if cancelled {
                println!("\n  Cancelled reminder {id}.\n");
            } else {
                println!("\n  Reminder {id} not found or already completed.\n");
            }
        }
    }

    // Database shutdown happens automatically via Drop on ctx
    Ok(())
}

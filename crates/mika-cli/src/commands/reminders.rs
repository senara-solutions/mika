use anyhow::Result;

use crate::cli::{ReminderArgs, ReminderCommand};
use crate::init;

pub async fn run(args: ReminderArgs, agent_name: &str) -> Result<()> {
    let ctx = init::init_db_only_for_agent(agent_name)?;
    let db = &ctx.async_db;

    match args.command {
        None => {
            let reminders = db.get_pending_reminders().await?;
            if reminders.is_empty() {
                println!("\n  No pending reminders.\n");
            } else {
                println!("\n  Pending Reminders ({}):", reminders.len());
                for r in &reminders {
                    println!(
                        "    #{}: \"{}\" at {} (created: {})",
                        r.id,
                        r.message,
                        r.display_fire_at(),
                        r.created_at
                    );
                }
                println!();
            }
        }
        Some(ReminderCommand::Cancel { id }) => {
            let cancelled = db.cancel_reminder(id).await?;
            if cancelled {
                println!("\n  Cancelled reminder #{id}.\n");
            } else {
                println!("\n  Reminder #{id} not found or already completed.\n");
            }
        }
    }

    // Database shutdown happens automatically via Drop on ctx
    Ok(())
}

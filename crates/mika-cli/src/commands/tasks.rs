use anyhow::Result;
use mika_agent::db::format_ts;

use crate::cli::{TaskArgs, TaskCommand};
use crate::init;

pub async fn run(args: TaskArgs, agent_name: &str) -> Result<()> {
    let ctx = init::init_db_only_for_agent(agent_name)?;
    let db = &ctx.async_db;

    match args.command {
        None => {
            let tasks = db
                .get_tasks_by_status(vec![
                    "pending".to_string(),
                    "in_progress".to_string(),
                    "recurring_active".to_string(),
                ])
                .await?;

            if tasks.is_empty() {
                println!("\n  No active tasks.\n");
            } else {
                println!("\n  Active Tasks ({}):", tasks.len());
                for t in &tasks {
                    let short_id = &t.id[..12.min(t.id.len())];
                    let when = t
                        .next_fire_at
                        .as_ref()
                        .map(|s| format_ts(s))
                        .unwrap_or_else(|| t.trigger_type.clone());
                    println!(
                        "    {}: [{}] [{}] \"{}\" ({})",
                        short_id, t.status, t.action_type, t.label, when
                    );
                }
                println!();
            }
        }
        Some(TaskCommand::Cancel { id }) => {
            let cancelled = db.cancel_task(&id).await?;
            if cancelled {
                println!("\n  Cancelled task {id}.\n");
            } else {
                println!("\n  Task {id} not found or already completed.\n");
            }
        }
    }

    Ok(())
}

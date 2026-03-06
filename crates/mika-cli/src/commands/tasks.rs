use anyhow::Result;
use mika_agent::db::format_unix_ts;

use crate::cli::{TaskArgs, TaskCommand};
use crate::init;

pub async fn run(args: TaskArgs, agent_name: &str) -> Result<()> {
    let ctx = init::init_db_only_for_agent(agent_name)?;
    let db = &ctx.async_db;

    match args.command {
        None => {
            let tasks = db.get_schedulable_tasks().await?;
            let pending: Vec<_> = tasks
                .iter()
                .filter(|t| t.status == "pending" || t.status == "in_progress")
                .collect();

            if pending.is_empty() {
                println!("\n  No pending tasks.\n");
            } else {
                println!("\n  Pending Tasks ({}):", pending.len());
                for t in &pending {
                    let short_id = &t.id[..12.min(t.id.len())];
                    let when = t
                        .next_fire_at
                        .map(format_unix_ts)
                        .unwrap_or_else(|| t.trigger_type.clone());
                    println!(
                        "    {}: [{}] \"{}\" ({})",
                        short_id, t.action_type, t.label, when
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

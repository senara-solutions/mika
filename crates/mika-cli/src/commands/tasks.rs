use anyhow::Result;
use mika_agent::db::{Task, format_ts};
use serde_json::{Value, json};

use crate::cli::{OutputFormat, TaskArgs, TaskCommand};
use crate::init;

pub async fn run(args: TaskArgs, agent_name: &str) -> Result<()> {
    let ctx = init::init_db_only_for_agent(agent_name)?;
    let db = &ctx.async_db;

    match args.command {
        None | Some(TaskCommand::List { .. }) => {
            let format = match &args.command {
                Some(TaskCommand::List { format }) => format.clone(),
                _ => OutputFormat::Text,
            };
            let tasks = db
                .get_tasks_by_status(vec![
                    "pending".to_string(),
                    "in_progress".to_string(),
                    "recurring_active".to_string(),
                ])
                .await?;

            match format {
                OutputFormat::Text => {
                    if tasks.is_empty() {
                        println!("\n  No active tasks.\n");
                    } else {
                        println!("\n  Active Tasks ({}):", tasks.len());
                        for t in &tasks {
                            print_task_summary(t);
                        }
                        println!();
                    }
                }
                OutputFormat::Json => {
                    let json_tasks: Vec<Value> = tasks.iter().map(task_to_json).collect();
                    println!("{}", serde_json::to_string_pretty(&json_tasks)?);
                }
            }
        }
        Some(TaskCommand::Get { id, format }) => {
            let task = db.get_task(&id).await?;
            match task {
                Some(t) => match format {
                    OutputFormat::Text => print_task_detail(&t),
                    OutputFormat::Json => {
                        println!("{}", serde_json::to_string_pretty(&task_to_json(&t))?);
                    }
                },
                None => {
                    println!("\n  Task {id} not found.\n");
                }
            }
        }
        Some(TaskCommand::Cancel { id }) => {
            match mika_agent::task_engine::process_kill::cancel_task_and_kill(db, &id).await? {
                Some(outcome) => match (outcome.process_killed, outcome.pid) {
                    (Some(true), Some(pid)) => {
                        println!(
                            "\n  Cancelled task {id} (\"{}\") — process (PID {pid}) terminated.\n",
                            outcome.label
                        );
                    }
                    (Some(false), Some(pid)) => {
                        println!(
                            "\n  Cancelled task {id} (\"{}\") — warning: process (PID {pid}) may still be running.\n",
                            outcome.label
                        );
                    }
                    _ => {
                        println!("\n  Cancelled task {id} (\"{}\").\n", outcome.label);
                    }
                },
                None => {
                    println!("\n  Task {id} not found or already completed.\n");
                }
            }
        }
    }

    Ok(())
}

fn print_task_summary(t: &Task) {
    let short_id = &t.id[..12.min(t.id.len())];
    let when = t
        .next_fire_at
        .as_ref()
        .map(|s| format_ts(s))
        .unwrap_or_else(|| t.trigger_type.clone());
    let pid_info = t
        .process_id
        .map(|pid| format!(" [PID {pid}]"))
        .unwrap_or_default();
    println!(
        "    {}: [{}] [{}] \"{}\" ({}){pid_info}",
        short_id, t.status, t.action_type, t.label, when
    );
}

fn print_task_detail(t: &Task) {
    println!();
    println!("  Task Detail");
    println!("  ───────────────────────────────────");
    println!("  ID:            {}", t.id);
    println!("  Label:         {}", t.label);
    println!("  Status:        {}", t.status);
    println!("  Type:          {}", t.r#type);
    println!("  Trigger:       {}", t.trigger_type);
    println!("  Action:        {}", t.action_type);
    println!("  Agent:         {}", t.agent_id);
    println!("  Created:       {}", format_ts(&t.created_at));
    println!("  Updated:       {}", format_ts(&t.updated_at));
    if let Some(ref v) = t.next_fire_at {
        println!("  Next fire at:  {}", format_ts(v));
    }
    if let Some(ref v) = t.fired_at {
        println!("  Fired at:      {}", format_ts(v));
    }
    if let Some(ref v) = t.completed_at {
        println!("  Completed at:  {}", format_ts(v));
    }
    if let Some(ref v) = t.cron_expr {
        println!("  Cron:          {v}");
    }
    if let Some(ref v) = t.parent_task_id {
        println!("  Parent task:   {v}");
    }
    if let Some(ref v) = t.reference_url {
        println!("  Reference:     {v}");
    }
    if let Some(ref v) = t.source {
        println!("  Source:        {v}");
    }
    if let Some(pid) = t.process_id {
        println!("  Process ID:    {pid}");
    }
    if let Some(ref v) = t.result {
        let display = match v.char_indices().nth(200) {
            Some((i, _)) => &v[..i],
            None => v,
        };
        println!("  Result:        {display}");
    }
    println!();
}

fn task_to_json(t: &Task) -> Value {
    json!({
        "id": t.id,
        "agent_id": t.agent_id,
        "label": t.label,
        "status": t.status,
        "type": t.r#type,
        "trigger_type": t.trigger_type,
        "action_type": t.action_type,
        "cron_expr": t.cron_expr,
        "next_fire_at": t.next_fire_at,
        "timeout_at": t.timeout_at,
        "fired_at": t.fired_at,
        "completed_at": t.completed_at,
        "created_at": t.created_at,
        "updated_at": t.updated_at,
        "parent_task_id": t.parent_task_id,
        "reference_url": t.reference_url,
        "source": t.source,
        "process_id": t.process_id,
        "result": t.result,
        "metadata": t.metadata,
    })
}

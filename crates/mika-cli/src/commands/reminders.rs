use anyhow::Result;
use mika_agent::db::{Task, format_ts};
use serde_json::{Value, json};

use crate::cli::{OutputFormat, ReminderArgs, ReminderCommand};
use crate::init;

pub async fn run(args: ReminderArgs, agent_name: &str) -> Result<()> {
    let ctx = init::init_db_only_for_agent(agent_name)?;
    let db = &ctx.async_db;

    match args.command {
        None | Some(ReminderCommand::List { .. }) => {
            let format = match &args.command {
                Some(ReminderCommand::List { format }) => format.clone(),
                _ => OutputFormat::Text,
            };
            let reminders = db.get_user_visible_tasks().await?;

            match format {
                OutputFormat::Text => {
                    if reminders.is_empty() {
                        println!("\n  No pending reminders.\n");
                    } else {
                        println!("\n  Pending Reminders ({}):", reminders.len());
                        for t in &reminders {
                            print_reminder_summary(t);
                        }
                        println!();
                    }
                }
                OutputFormat::Json => {
                    let json_reminders: Vec<Value> =
                        reminders.iter().map(reminder_to_json).collect();
                    println!("{}", serde_json::to_string_pretty(&json_reminders)?);
                }
            }
        }
        Some(ReminderCommand::Get { id, format }) => {
            let task = db.get_task(&id).await?;
            match task {
                Some(t) => {
                    // Verify it's actually a reminder (time or recurring trigger)
                    if t.trigger_type != "time" && t.trigger_type != "recurring" {
                        println!(
                            "\n  ID {id} is not a reminder (trigger_type: {}).\n",
                            t.trigger_type
                        );
                        return Ok(());
                    }
                    match format {
                        OutputFormat::Text => print_reminder_detail(&t),
                        OutputFormat::Json => {
                            println!("{}", serde_json::to_string_pretty(&reminder_to_json(&t))?);
                        }
                    }
                }
                None => {
                    println!("\n  Reminder {id} not found.\n");
                }
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

    Ok(())
}

fn print_reminder_summary(t: &Task) {
    let short_id = &t.id[..12.min(t.id.len())];
    let fire_at = t
        .next_fire_at
        .as_ref()
        .map(|s| format_ts(s))
        .unwrap_or_else(|| "unknown".to_string());
    println!("    {}: \"{}\" at {}", short_id, t.label, fire_at);
}

fn print_reminder_detail(t: &Task) {
    println!();
    println!("  Reminder Detail");
    println!("  ───────────────────────────────────");
    println!("  ID:            {}", t.id);
    println!("  Label:         {}", t.label);
    println!("  Status:        {}", t.status);
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
    // Show the reminder message text from action_config
    if !t.action_config.is_empty() {
        println!("  Message:       {}", t.action_config);
    }
    if let Some(ref v) = t.metadata {
        println!("  Metadata:      {v}");
    }
    println!();
}

fn reminder_to_json(t: &Task) -> Value {
    json!({
        "id": t.id,
        "agent_id": t.agent_id,
        "label": t.label,
        "status": t.status,
        "trigger_type": t.trigger_type,
        "action_type": t.action_type,
        "action_config": t.action_config,
        "cron_expr": t.cron_expr,
        "next_fire_at": t.next_fire_at,
        "fired_at": t.fired_at,
        "completed_at": t.completed_at,
        "created_at": t.created_at,
        "updated_at": t.updated_at,
        "metadata": t.metadata,
    })
}

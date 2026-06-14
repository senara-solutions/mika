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
                OutputFormat::Yaml => {
                    let json_reminders: Vec<Value> =
                        reminders.iter().map(reminder_to_json).collect();
                    print!("{}", serde_yaml::to_string(&json_reminders)?);
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
                        OutputFormat::Yaml => {
                            print!("{}", serde_yaml::to_string(&reminder_to_json(&t))?);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_reminder(id: &str, label: &str, trigger_type: &str) -> Task {
        Task {
            id: id.to_string(),
            agent_id: "test-agent".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: label.to_string(),
            trigger_type: trigger_type.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: Some("2026-05-06T15:00:00Z".to_string()),
            timeout_at: None,
            action_type: "send_message".to_string(),
            action_config: "Don't forget the meeting".to_string(),
            status: "pending".to_string(),
            process_id: None,
            input_context: None,
            result: None,
            created_by_session: None,
            created_trace_id: None,
            execution_trace_id: None,
            created_at: "2026-05-06T09:00:00Z".to_string(),
            updated_at: "2026-05-06T09:00:00Z".to_string(),
            fired_at: None,
            completed_at: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: "issue".to_string(),
            dispatch_class: None,
        }
    }

    #[test]
    fn test_reminder_to_json_happy_get() {
        let reminder = make_reminder("rem-001-aaaa", "Team standup", "time");
        let json = reminder_to_json(&reminder);

        assert_eq!(json["id"], "rem-001-aaaa");
        assert_eq!(json["label"], "Team standup");
        assert_eq!(json["status"], "pending");
        assert_eq!(json["trigger_type"], "time");
        assert_eq!(json["action_type"], "send_message");
        assert_eq!(json["action_config"], "Don't forget the meeting");
        assert_eq!(json["next_fire_at"], "2026-05-06T15:00:00Z");
    }

    #[test]
    fn test_reminder_to_json_not_found() {
        // Verify null optional fields serialize correctly.
        let reminder = make_reminder("rem-missing-00", "Orphan reminder", "recurring");
        let json = reminder_to_json(&reminder);

        assert_eq!(json["id"], "rem-missing-00");
        assert_eq!(json["trigger_type"], "recurring");
        assert!(json["cron_expr"].is_null());
        assert!(json["fired_at"].is_null());
        assert!(json["completed_at"].is_null());
        assert!(json["metadata"].is_null());
    }

    #[test]
    fn test_reminder_to_json_list_serialization() {
        let reminders = [
            make_reminder("rem-001-aaaa", "Morning standup", "time"),
            make_reminder("rem-002-bbbb", "Weekly review", "recurring"),
        ];
        let json_reminders: Vec<Value> = reminders.iter().map(reminder_to_json).collect();
        let output = serde_json::to_string_pretty(&json_reminders).unwrap();

        assert!(output.contains("\"Morning standup\""));
        assert!(output.contains("\"Weekly review\""));
        assert!(output.contains("\"time\""));
        assert!(output.contains("\"recurring\""));
    }

    #[test]
    fn test_reminder_to_json_empty_list() {
        let reminders: Vec<Task> = vec![];
        let json_reminders: Vec<Value> = reminders.iter().map(reminder_to_json).collect();
        let output = serde_json::to_string_pretty(&json_reminders).unwrap();

        assert_eq!(output, "[]");
    }
}

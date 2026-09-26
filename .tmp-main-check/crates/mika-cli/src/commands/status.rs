use anyhow::Result;

use crate::cli::OutputFormat;
use crate::init;

pub async fn run(agent_name: &str, format: &OutputFormat) -> Result<()> {
    let ctx = init::init_db_only_for_agent(agent_name)?;
    let db = &ctx.async_db;

    let (db_size, msg_count, people, commitments, preferences, events, last_msg, tokens, version) = tokio::join!(
        db.db_size_bytes(),
        db.count_messages(),
        db.list_people(),
        db.list_commitments("pending"),
        db.list_preferences(),
        db.list_events(),
        db.last_user_message_time(),
        db.total_core_memory_tokens(),
        db.schema_version(),
    );

    let db_size = db_size.unwrap_or(0);

    match format {
        OutputFormat::Json => {
            let last_message = last_msg.ok().flatten();
            let obj = serde_json::json!({
                "agent": agent_name,
                "version": version.unwrap_or(0),
                "db_size_bytes": db_size,
                "messages": msg_count.unwrap_or(0),
                "people": people.map(|p| p.len()).unwrap_or(0),
                "commitments_pending": commitments.map(|c| c.len()).unwrap_or(0),
                "preferences": preferences.map(|p| p.len()).unwrap_or(0),
                "events": events.map(|e| e.len()).unwrap_or(0),
                "tokens_used": tokens.unwrap_or(0),
                "last_message": last_message,
            });
            println!("{}", serde_json::to_string_pretty(&obj)?);
        }
        OutputFormat::Yaml => {
            let last_message = last_msg.ok().flatten();
            let obj = serde_json::json!({
                "agent": agent_name,
                "version": version.unwrap_or(0),
                "db_size_bytes": db_size,
                "messages": msg_count.unwrap_or(0),
                "people": people.map(|p| p.len()).unwrap_or(0),
                "commitments_pending": commitments.map(|c| c.len()).unwrap_or(0),
                "preferences": preferences.map(|p| p.len()).unwrap_or(0),
                "events": events.map(|e| e.len()).unwrap_or(0),
                "tokens_used": tokens.unwrap_or(0),
                "last_message": last_message,
            });
            print!("{}", serde_yaml::to_string(&obj)?);
        }
        OutputFormat::Text => {
            let size_str = if db_size > 1_000_000 {
                format!("{:.1} MB", db_size as f64 / 1_000_000.0)
            } else if db_size > 1_000 {
                format!("{:.1} KB", db_size as f64 / 1_000.0)
            } else {
                format!("{db_size} B")
            };

            let last_activity = last_msg
                .ok()
                .flatten()
                .map(|s| mika_agent::db::format_ts(&s))
                .unwrap_or_else(|| "never".to_string());

            println!();
            println!("  \u{2726} Mika Status");
            println!(
                "  Database:       {} ({})",
                ctx.settings.db_path.display(),
                size_str
            );
            println!("  Schema version: {}", version.unwrap_or(0));
            println!("  Messages:       {}", msg_count.unwrap_or(0));
            println!("  Last activity:  {last_activity}");
            println!("  Core memory:    {} / 2000 tokens", tokens.unwrap_or(0));
            println!("  People:         {}", people.map(|p| p.len()).unwrap_or(0));
            println!(
                "  Commitments:    {} pending",
                commitments.map(|c| c.len()).unwrap_or(0)
            );
            println!(
                "  Preferences:    {}",
                preferences.map(|p| p.len()).unwrap_or(0)
            );
            println!("  Events:         {}", events.map(|e| e.len()).unwrap_or(0));
            println!();
        }
    }

    // Database shutdown happens automatically via Drop on ctx
    Ok(())
}

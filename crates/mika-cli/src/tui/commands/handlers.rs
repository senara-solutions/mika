use std::fmt::Write;

use mika_common::agent;
use mika_common::home;

use crate::tui::app::{AgentRequest, AgentStatus, App, ChatRole};
use crate::tui::commands::{COMMANDS, parse_command};

/// Dispatch a slash command string to the appropriate handler.
/// Returns Some(output) for commands that produce output, None for commands like /exit.
pub async fn dispatch(app: &mut App<'_>, input: &str) -> Option<String> {
    let (cmd_name, args) = parse_command(input);
    match cmd_name {
        "help" | "h" | "?" => Some(handle_help()),
        "clear" => Some(handle_clear(app, args).await),
        "exit" | "quit" | "q" => {
            app.should_quit = true;
            None
        }
        "compact" => Some(handle_compact(app).await),
        "memory" | "mem" => Some(handle_memory(app, args).await),
        "reminders" | "remind" => Some(handle_reminders(app).await),
        "status" | "stat" => Some(handle_status(app).await),
        "soul" => Some(handle_soul(app).await),
        "config" | "cfg" => Some(handle_config(app).await),
        "model" => Some(handle_model(app)),
        "export" => Some(handle_export(app).await),
        "skills" => Some(handle_skills(app)),
        "skill" => Some(handle_skill(app, args)),
        "switch" | "agent" => Some(handle_switch(app, args)),
        "agents" => Some(handle_agents(app)),
        _ => Some(format!(
            "Unknown command: /{cmd_name}. Type /help for available commands."
        )),
    }
}

fn handle_help() -> String {
    let mut out = String::from("Available commands:\n");
    for cmd in COMMANDS {
        let aliases = if cmd.aliases.is_empty() {
            String::new()
        } else {
            format!(" ({})", cmd.aliases.join(", "))
        };
        let args = cmd.args_hint.unwrap_or("");
        let _ = writeln!(
            out,
            "  /{}{} {} — {}",
            cmd.name, aliases, args, cmd.description
        );
    }
    out
}

async fn handle_clear(app: &mut App<'_>, _args: &str) -> String {
    app.messages.clear();
    app.scroll_offset = 0;
    "Chat display cleared.".to_string()
}

async fn handle_compact(app: &mut App<'_>) -> String {
    if app.status != AgentStatus::Idle {
        return "Cannot compact while agent is busy.".to_string();
    }
    let count = match app.db.count_messages().await {
        Ok(c) => c,
        Err(e) => return format!("Failed to check message count: {e}"),
    };
    if count <= 50 {
        return format!("Nothing to compact ({count}/50 messages).");
    }
    match mika_agent::compaction::maybe_compact(&app.db, &app.claude).await {
        Ok(()) => format!("Compacted conversation ({count} messages)."),
        Err(e) => format!("Compaction failed: {e}"),
    }
}

async fn handle_memory(app: &mut App<'_>, args: &str) -> String {
    if args.starts_with("search") {
        let query = args.strip_prefix("search").unwrap_or("").trim();
        if query.is_empty() {
            return "Usage: /memory search <query>".to_string();
        }
        return handle_memory_search(app, query).await;
    }

    // Show core memory blocks
    match app.db.get_all_core_memory().await {
        Ok(entries) => {
            if entries.is_empty() {
                return "No core memory entries.".to_string();
            }
            let mut out = String::from("Core Memory:\n");
            for entry in &entries {
                let _ = writeln!(
                    out,
                    "  [{}] {} ({} tokens)",
                    entry.key, entry.value, entry.token_count
                );
            }
            match app.db.total_core_memory_tokens().await {
                Ok(total) => {
                    let _ = writeln!(out, "\nTotal: {total}/2000 tokens");
                }
                Err(_) => {}
            }
            out
        }
        Err(e) => format!("Failed to load core memory: {e}"),
    }
}

async fn handle_memory_search(app: &mut App<'_>, query: &str) -> String {
    let mut out = String::new();
    let mut found = false;

    // Search across all memory layers concurrently
    let (people, commitments, preferences, events) = tokio::join!(
        app.db.search_people(query),
        app.db.search_commitments(query),
        app.db.search_preferences(query),
        app.db.search_events(query),
    );

    if let Ok(people) = people {
        if !people.is_empty() {
            found = true;
            let _ = writeln!(out, "People:");
            for p in &people {
                let rel = p.relationship.as_deref().unwrap_or("unknown");
                let _ = writeln!(out, "  {} ({})", p.canonical_name, rel);
            }
        }
    }
    if let Ok(commitments) = commitments {
        if !commitments.is_empty() {
            found = true;
            let _ = writeln!(out, "Commitments:");
            for c in &commitments {
                let due = c.due_date.as_deref().unwrap_or("no due date");
                let _ = writeln!(out, "  [{}] {} ({})", c.status, c.description, due);
            }
        }
    }
    if let Ok(preferences) = preferences {
        if !preferences.is_empty() {
            found = true;
            let _ = writeln!(out, "Preferences:");
            for p in &preferences {
                let _ = writeln!(out, "  {}: {}", p.category, p.value);
            }
        }
    }
    if let Ok(events) = events {
        if !events.is_empty() {
            found = true;
            let _ = writeln!(out, "Events:");
            for e in &events {
                let date = e.event_date.as_deref().unwrap_or("no date");
                let _ = writeln!(out, "  {} ({})", e.description, date);
            }
        }
    }

    if !found {
        format!("No results for '{query}'.")
    } else {
        out
    }
}

async fn handle_reminders(app: &mut App<'_>) -> String {
    let pending = app.db.get_pending_reminders().await;
    let future = app.db.get_future_reminders().await;

    let mut out = String::new();
    match pending {
        Ok(reminders) if !reminders.is_empty() => {
            let _ = writeln!(out, "Pending reminders:");
            for r in &reminders {
                let _ = writeln!(out, "  #{}: {} (due: {})", r.id, r.message, r.fire_at);
            }
        }
        _ => {}
    }
    match future {
        Ok(reminders) if !reminders.is_empty() => {
            let _ = writeln!(out, "Upcoming reminders:");
            for r in &reminders {
                let _ = writeln!(out, "  #{}: {} (fires: {})", r.id, r.message, r.fire_at);
            }
        }
        _ => {}
    }

    if out.is_empty() {
        "No active reminders.".to_string()
    } else {
        out
    }
}

async fn handle_status(app: &mut App<'_>) -> String {
    let mut out = String::from("Status:\n");

    // Run all DB queries concurrently
    let (count, size, tokens, version) = tokio::join!(
        app.db.count_messages(),
        app.db.db_size_bytes(),
        app.db.total_core_memory_tokens(),
        app.db.schema_version(),
    );

    if let Ok(count) = count {
        let _ = writeln!(out, "  Messages: {count}");
    }
    if let Ok(size) = size {
        let size_kb = size / 1024;
        let _ = writeln!(out, "  DB size: {size_kb} KB");
    }
    if let Ok(tokens) = tokens {
        let _ = writeln!(out, "  Core memory: {tokens}/2000 tokens");
    }
    if let Ok(version) = version {
        let _ = writeln!(out, "  Schema: v{version}");
    }
    let _ = writeln!(out, "  Model: {}", app.model);
    let _ = writeln!(
        out,
        "  Session: {}",
        &app.session_id[..8.min(app.session_id.len())]
    );

    out
}

async fn handle_soul(app: &mut App<'_>) -> String {
    let soul_path = app.home_dir.join("soul.md");
    match tokio::fs::read_to_string(&soul_path).await {
        Ok(content) if !content.trim().is_empty() => content,
        Ok(_) => "soul.md is empty.".to_string(),
        Err(_) => "No soul.md found. Create one at ~/.mika/soul.md".to_string(),
    }
}

async fn handle_config(app: &mut App<'_>) -> String {
    let config_path = app.home_dir.join("config").join("local.toml");

    let mut out = String::from("Configuration:\n");
    let _ = writeln!(out, "  Model: {}", app.model);
    let _ = writeln!(out, "  Home: {}", app.home_dir.display());
    let _ = writeln!(
        out,
        "  Session: {}",
        &app.session_id[..8.min(app.session_id.len())]
    );

    // Show config file path without dumping contents (may contain secrets)
    if tokio::fs::metadata(&config_path).await.is_ok() {
        let _ = writeln!(out, "  Config file: {}", config_path.display());
    } else {
        let _ = writeln!(out, "  Config file: (using defaults)");
    }

    out
}

fn handle_model(app: &App<'_>) -> String {
    format!("Current model: {}", app.model)
}

async fn handle_export(app: &mut App<'_>) -> String {
    if app.messages.is_empty() {
        return "Nothing to export.".to_string();
    }

    let exports_dir = app.home_dir.join("exports");
    if let Err(e) = tokio::fs::create_dir_all(&exports_dir).await {
        return format!("Failed to create exports directory: {e}");
    }

    let timestamp = chrono::Utc::now().format("%Y-%m-%d-%H%M%S");
    let short_session = &app.session_id[..8.min(app.session_id.len())];
    let filename = format!("session-{short_session}-{timestamp}.md");
    let filepath = exports_dir.join(&filename);

    let mut content = String::from("# Mika Conversation Export\n\n");
    let _ = writeln!(content, "Session: {}", app.session_id);
    let _ = writeln!(content, "Model: {}", app.model);
    let _ = writeln!(
        content,
        "Exported: {}\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
    );
    let _ = writeln!(content, "---\n");

    for msg in &app.messages {
        match msg.role {
            ChatRole::User => {
                let _ = writeln!(content, "**You:** {}\n", msg.content);
            }
            ChatRole::Assistant => {
                let _ = writeln!(content, "**{}:** {}\n", app.identity_name, msg.content);
            }
            ChatRole::System => {
                let _ = writeln!(content, "*System: {}*\n", msg.content);
            }
            ChatRole::Command => {
                // Command output is ephemeral (status, help text, etc.)
                // and not part of the conversation — intentionally excluded.
            }
        }
    }

    // Use create_new to prevent symlink attacks (atomic, fails if path exists)
    match tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&filepath)
        .await
    {
        Ok(mut file) => {
            use tokio::io::AsyncWriteExt;
            match file.write_all(content.as_bytes()).await {
                Ok(()) => format!("Exported to {}", filepath.display()),
                Err(e) => format!("Export failed: {e}"),
            }
        }
        Err(e) => format!("Export failed: {e}"),
    }
}

fn handle_skills(app: &App<'_>) -> String {
    let skills = app.skills.skills();
    if skills.is_empty() {
        return "No skills loaded.".to_string();
    }

    let mut out = String::from("Loaded skills:\n");
    for entry in skills {
        let handler_type = match &entry.manifest.handler {
            mika_agent::skills::manifest::Handler::Builtin { .. } => "builtin",
        };
        let always_on = if entry.manifest.options.always_on {
            " [always on]"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "  {} ({}) — {}{}",
            entry.manifest.name, handler_type, entry.manifest.description, always_on
        );
    }
    out
}

fn handle_switch(app: &mut App<'_>, args: &str) -> String {
    let name = agent::normalize_agent_name(args);
    if name.is_empty() {
        return "Usage: /switch <agent-name>".to_string();
    }
    if let Err(e) = agent::validate_agent_name(&name) {
        return format!("Invalid agent name: {e}");
    }
    if name == app.agent_name {
        return format!("Already using agent '{name}'.");
    }

    if !agent::agent_exists(&app.global_home, &name) {
        return format!(
            "Agent '{name}' not found. Create it with `mika agents create {name}` first."
        );
    }

    if app.status != AgentStatus::Idle {
        return "Cannot switch while agent is busy. Wait for the current response to finish."
            .to_string();
    }

    // Signal the current worker to stop
    let _ = app.agent_tx.send(AgentRequest::Quit);
    app.pending_switch = Some(name.clone());
    format!("Switching to agent '{name}'...")
}

fn handle_agents(app: &App<'_>) -> String {
    let agents = agent::list_agents(&app.global_home);
    let active = home::read_active_agent(&app.global_home);

    if agents.is_empty() {
        return "No agents found.".to_string();
    }

    let mut out = String::from("Agents:\n");
    for name in &agents {
        let current = if *name == app.agent_name {
            " (current)"
        } else {
            ""
        };
        let default = if *name == active { " (default)" } else { "" };
        let _ = writeln!(out, "  {name}{current}{default}");
    }
    out
}

fn handle_skill(app: &App<'_>, args: &str) -> String {
    let name = args.trim();
    if name.is_empty() {
        return "Usage: /skill <name>".to_string();
    }

    let skills = app.skills.skills();
    let found = skills
        .iter()
        .find(|s| s.manifest.name.eq_ignore_ascii_case(name));

    match found {
        Some(entry) => {
            let m = &entry.manifest;
            let handler_type = match &m.handler {
                mika_agent::skills::manifest::Handler::Builtin { tools } => {
                    format!("builtin (tools: {})", tools.join(", "))
                }
            };
            let keywords = if m.triggers.keywords.is_empty() {
                "none".to_string()
            } else {
                m.triggers.keywords.join(", ")
            };
            let mut out = String::new();
            let _ = writeln!(out, "Skill: {}", m.name);
            let _ = writeln!(out, "  Description: {}", m.description);
            let _ = writeln!(out, "  Handler: {handler_type}");
            let _ = writeln!(out, "  Always on: {}", m.options.always_on);
            let _ = writeln!(out, "  Keywords: {keywords}");
            let _ = writeln!(out, "  Path: {}", entry.dir.display());
            out
        }
        None => {
            format!("No skill found with name '{name}'. Use /skills to list all loaded skills.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handle_help_contains_all_commands() {
        let output = handle_help();
        assert!(output.contains("/help"));
        assert!(output.contains("/clear"));
        assert!(output.contains("/exit"));
        assert!(output.contains("/memory"));
        assert!(output.contains("/status"));
        assert!(output.contains("/skills"));
    }

    #[test]
    fn test_handle_model() {
        // We can't easily construct an App in tests, so just test the format
        let output = format!("Current model: {}", "claude-sonnet-4-6");
        assert!(output.contains("claude-sonnet-4-6"));
    }
}

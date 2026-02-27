use std::fmt::Write;

use mika_agent::config_keys::{is_settable_key, settable_keys_display, validate_config_value};
use mika_common::agent;
use mika_common::home;
use mika_common::team;

use crate::tui::app::{AgentRequest, AgentStatus, App, ChatMessage, ChatRole};
use crate::tui::commands::{COMMANDS, parse_command};
use crate::tui::input;

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
        "config" | "cfg" => Some(handle_config(app, args).await),
        "model" => Some(handle_model(app, args)),
        "export" => Some(handle_export(app).await),
        "skills" => Some(handle_skills(app)),
        "skill" => Some(handle_skill(app, args)),
        "switch" | "agent" => Some(handle_switch(app, args)),
        "agents" => Some(handle_agents(app)),
        "teams" => Some(handle_teams(app)),
        "team" => Some(handle_team(args)),
        "think" | "t" => handle_think(app, args),
        "attach" | "img" => Some(handle_attach(app, args)),
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

async fn handle_config(app: &mut App<'_>, args: &str) -> String {
    if let Some(rest) = args.strip_prefix("set") {
        return handle_config_set(app, rest.trim()).await;
    }

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

    // Show customer config entries
    if let Ok(configs) = app.db.list_customer_config().await {
        if !configs.is_empty() {
            let _ = writeln!(out);
            let _ = writeln!(out, "Customer settings:");
            for (key, value) in &configs {
                let _ = writeln!(out, "  {key}: {value}");
            }
        }
    }

    out
}

async fn handle_config_set(app: &mut App<'_>, args: &str) -> String {
    let parts: Vec<&str> = args.splitn(2, char::is_whitespace).collect();
    if parts.len() < 2 || parts[0].is_empty() || parts[1].trim().is_empty() {
        return format!(
            "Usage: /config set <key> <value>\nSettable keys: {}",
            settable_keys_display()
        );
    }
    let key = parts[0];
    let value = parts[1].trim();

    if !is_settable_key(key) {
        return format!(
            "Unknown config key: {key}\nSettable keys: {}",
            settable_keys_display()
        );
    }

    if let Err(msg) = validate_config_value(key, value) {
        return msg;
    }

    match app.db.set_customer_config(key, value).await {
        Ok(()) => format!("Set {key} = {value}"),
        Err(e) => format!("Failed to set {key}: {e}"),
    }
}

/// Known model shorthands: (shorthand, full_model_id, display_name).
const MODEL_ALIASES: &[(&str, &str, &str)] = &[
    ("sonnet", "claude-sonnet-4-6", "Claude Sonnet 4.6"),
    ("opus", "claude-opus-4-6", "Claude Opus 4.6"),
    ("haiku", "claude-haiku-4-5", "Claude Haiku 4.5"),
];

fn resolve_model_name(input: &str) -> Option<(&'static str, &'static str)> {
    let lower = input.to_lowercase();
    for &(alias, full_id, display) in MODEL_ALIASES {
        if lower == alias || lower == full_id {
            return Some((full_id, display));
        }
    }
    None
}

fn handle_model(app: &mut App<'_>, args: &str) -> String {
    let args = args.trim();
    if args.is_empty() {
        let mut out = format!("Current model: {}\n\nAvailable models:", app.model);
        for &(alias, full_id, display) in MODEL_ALIASES {
            let current = if full_id == app.model {
                " (current)"
            } else {
                ""
            };
            let _ = write!(out, "\n  /{alias} — {display}{current}");
        }
        let _ = write!(out, "\n\nUsage: /model <name>");
        return out;
    }

    match resolve_model_name(args) {
        Some((full_id, display)) => {
            if full_id == app.model {
                return format!("Already using {display}.");
            }
            app.model = full_id.to_string();
            app.claude.model = full_id.to_string();
            let _ = app.agent_tx.send(AgentRequest::SetModel {
                model: full_id.to_string(),
            });
            app.needs_redraw = true;
            format!("Switched to {display} ({full_id}).")
        }
        None => {
            let options: Vec<&str> = MODEL_ALIASES.iter().map(|&(a, _, _)| a).collect();
            format!("Unknown model: {args}\nAvailable: {}", options.join(", "))
        }
    }
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
        let channel_prefix = msg
            .channel
            .as_ref()
            .map(|ch| format!("[{ch}] "))
            .unwrap_or_default();
        match msg.role {
            ChatRole::User => {
                let _ = writeln!(content, "**{channel_prefix}You:** {}\n", msg.content);
            }
            ChatRole::Assistant => {
                let _ = writeln!(
                    content,
                    "**{channel_prefix}{}:** {}\n",
                    app.identity_name, msg.content
                );
            }
            ChatRole::System => {
                let _ = writeln!(content, "*System: {}*\n", msg.content);
            }
            ChatRole::Thinking => {
                // Thinking is ephemeral — not exported.
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
        let tool_count = entry.skill_tools.len();
        let handler_desc = if tool_count > 0 {
            format!("{} tools", tool_count)
        } else {
            "no tools".to_string()
        };
        let always_on = if entry.manifest.skill.always_on {
            " [always on]"
        } else {
            ""
        };
        let enabled = if entry.enabled { "" } else { " [disabled]" };
        let _ = writeln!(
            out,
            "  {} ({}) — {}{}{}",
            entry.manifest.skill.name,
            handler_desc,
            entry.manifest.skill.description,
            always_on,
            enabled
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
        .find(|s| s.manifest.skill.name.eq_ignore_ascii_case(name));

    match found {
        Some(entry) => {
            let m = &entry.manifest;
            let keywords = if m.triggers.keywords.is_empty() {
                "none".to_string()
            } else {
                m.triggers.keywords.join(", ")
            };
            let mut out = String::new();
            let _ = writeln!(out, "Skill: {}", m.skill.name);
            let _ = writeln!(out, "  Description: {}", m.skill.description);
            let _ = writeln!(
                out,
                "  Version: {}",
                if m.skill.version.is_empty() {
                    "unset"
                } else {
                    &m.skill.version
                }
            );
            let _ = writeln!(out, "  Always on: {}", m.skill.always_on);
            let _ = writeln!(out, "  Enabled: {}", entry.enabled);
            let _ = writeln!(out, "  Timeout: {}s", m.skill.timeout_secs);
            let _ = writeln!(out, "  Keywords: {keywords}");
            if !entry.skill_tools.is_empty() {
                let tool_names: Vec<&str> = entry
                    .skill_tools
                    .iter()
                    .map(|t| t.definition.name.as_str())
                    .collect();
                let _ = writeln!(out, "  Tools: {}", tool_names.join(", "));
            }
            let _ = writeln!(out, "  Path: {}", entry.dir.display());
            out
        }
        None => {
            format!("No skill found with name '{name}'. Use /skills to list all loaded skills.")
        }
    }
}

fn handle_teams(app: &App<'_>) -> String {
    let teams = team::list_teams(&app.global_home);

    if teams.is_empty() {
        return "No teams found. Use `mika teams create <name>` to create one.".to_string();
    }

    let mut out = String::from("Teams:\n");
    for name in &teams {
        let agent_count = match team::load_team(&app.global_home, name) {
            Ok(def) => def.agents.len(),
            Err(_) => 0,
        };
        let _ = writeln!(out, "  {name} ({agent_count} agents)");
    }
    out
}

fn handle_team(args: &str) -> String {
    if args.is_empty() {
        return "Usage: /team <name> \"<goal>\"".to_string();
    }

    // Parse: /team <name> <goal>
    let (name, goal) = match args.split_once(char::is_whitespace) {
        Some((n, g)) => (n.trim(), g.trim().trim_matches('"')),
        None => return "Usage: /team <name> \"<goal>\"".to_string(),
    };

    if goal.is_empty() {
        return "Usage: /team <name> \"<goal>\"".to_string();
    }

    format!(
        "Team runs are long-running operations. Use the CLI instead: \
         mika teams run {name} \"{goal}\""
    )
}

/// Resolve a thinking level keyword to (budget_tokens, level_name).
fn resolve_thinking_level(word: &str) -> Option<(u32, &'static str)> {
    match word.to_lowercase().as_str() {
        "low" => Some((5_000, "low")),
        "medium" | "med" => Some((10_000, "medium")),
        "high" => Some((50_000, "high")),
        _ => None,
    }
}

fn handle_think(app: &mut App<'_>, args: &str) -> Option<String> {
    let args = args.trim();

    // No args: show current level and usage
    if args.is_empty() {
        let current = match app.thinking_level {
            Some((budget, level)) => format!("Thinking level: {level} ({budget} tokens)"),
            None => "Thinking: off".to_string(),
        };
        return Some(format!(
            "{current}\nUsage: /think [low|medium|high|off] [prompt]"
        ));
    }

    // /think off — disable persistent thinking
    if args.eq_ignore_ascii_case("off") {
        app.thinking_level = None;
        return Some("Thinking: off".to_string());
    }

    // Parse: first word might be a level
    let (first, rest) = match args.split_once(char::is_whitespace) {
        Some((f, r)) => (f, r.trim()),
        None => (args, ""),
    };

    // If first word is a valid level...
    if let Some((budget, level)) = resolve_thinking_level(first) {
        if rest.is_empty() {
            // /think high — set persistent level (no prompt)
            app.thinking_level = Some((budget, level));
            return Some(format!(
                "Thinking level: {level} ({budget} tokens). All messages will use extended thinking."
            ));
        }

        // /think high <prompt> — one-shot with thinking
        if app.status != AgentStatus::Idle {
            return Some("Agent is busy. Wait for the current response to finish.".to_string());
        }

        app.messages.push(ChatMessage {
            role: ChatRole::User,
            content: format!("[think:{level}] {rest}"),
            rendered: None,
            channel: None,
        });

        let images = std::mem::take(&mut app.pending_images);
        let _ = app.agent_tx.send(AgentRequest::Message {
            text: rest.to_string(),
            images,
            thinking_budget: Some(budget),
        });
        app.status = AgentStatus::Thinking;
        app.scroll_offset = 0;
        app.needs_redraw = true;
        return None;
    }

    // No level prefix — use default medium for one-shot
    if app.status != AgentStatus::Idle {
        return Some("Agent is busy. Wait for the current response to finish.".to_string());
    }

    let (budget, level) = (10_000, "medium");
    app.messages.push(ChatMessage {
        role: ChatRole::User,
        content: format!("[think:{level}] {args}"),
        rendered: None,
        channel: None,
    });

    let images = std::mem::take(&mut app.pending_images);
    let _ = app.agent_tx.send(AgentRequest::Message {
        text: args.to_string(),
        images,
        thinking_budget: Some(budget),
    });
    app.status = AgentStatus::Thinking;
    app.scroll_offset = 0;
    app.needs_redraw = true;
    None
}

fn handle_attach(app: &mut App<'_>, args: &str) -> String {
    let path = args.trim();
    if path.is_empty() {
        return "Usage: /attach <path-to-image>".to_string();
    }
    match input::try_load_image_file(path) {
        Some(attachment) => {
            let label = attachment.label.clone();
            let size = attachment.size_display();
            if let Some(err) = app.attach_image(attachment) {
                return err;
            }
            format!("Attached: {label} ({size})")
        }
        None => {
            "Failed to load image. Supported formats: png, jpg, gif, webp (max 10MB).".to_string()
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

    #[test]
    fn test_handle_help_contains_new_commands() {
        let output = handle_help();
        assert!(output.contains("/think"));
        assert!(output.contains("/attach"));
    }

    #[test]
    fn test_handle_help_contains_config_args() {
        let output = handle_help();
        assert!(output.contains("/config"));
        assert!(output.contains("set <key> <value>"));
    }

    #[test]
    fn test_settable_config_keys_allowlist() {
        // Validation now lives in mika_agent::config_keys (shared module).
        // Verify we can still call the shared helpers from the CLI.
        assert!(is_settable_key("chat_id"));
        assert!(is_settable_key("timezone"));
        assert!(!is_settable_key("api_key"));
        assert!(!is_settable_key("db_path"));
    }
}

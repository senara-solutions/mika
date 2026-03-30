use std::fmt::Write;

use mika_agent::config_keys::{is_settable_key, settable_keys_display, validate_config_value};
use mika_common::agent;
use mika_common::home;
use mika_common::team;

use crate::tui::app::{
    AgentRequest, AgentStatus, App, ChatMessage, ChatRole, MessagesLayout,
    callback_label_from_metadata,
};
use crate::tui::commands::{COMMANDS, parse_command, resolve_thinking_level};
use crate::tui::input;

/// Commands allowed in team mode. New commands are blocked by default (safe failure mode).
const TEAM_MODE_ALLOWED_COMMANDS: &[&str] = &[
    "help", "h", "?", "clear", "exit", "quit", "q", "export", "agents", "teams", "team", "status",
    "stat", "verbose", "v",
];

/// Dispatch a slash command string to the appropriate handler.
/// Returns Some(output) for commands that produce output, None for commands like /exit.
pub async fn dispatch(app: &mut App<'_>, input: &str) -> Option<String> {
    let (cmd_name, args) = parse_command(input);

    // Gate agent-specific commands in team mode (allowlist — new commands blocked by default)
    if app.is_team_mode() && !TEAM_MODE_ALLOWED_COMMANDS.contains(&cmd_name) {
        return Some(format!("/{cmd_name} is not available in team mode."));
    }

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
        "tasks" => Some(handle_tasks(app, args).await),
        "status" | "stat" => {
            if app.is_team_mode() {
                Some(handle_team_info(app))
            } else {
                Some(handle_status(app).await)
            }
        }
        "soul" => Some(handle_soul(app).await),
        "config" | "cfg" => Some(handle_config(app, args).await),
        "model" => Some(handle_model(app, args).await),
        "provider" => Some(handle_provider(app, args).await),
        "export" => Some(handle_export(app).await),
        "skills" => Some(handle_skills(app)),
        "skill" => Some(handle_skill(app, args)),
        "switch" | "agent" => Some(handle_switch(app, args)),
        "agents" => Some(handle_agents(app)),
        "teams" => Some(handle_teams(app)),
        "team" => {
            if app.is_team_mode() {
                Some(handle_team_info(app))
            } else {
                Some(handle_team(args))
            }
        }
        "verbose" | "v" => Some(handle_verbose(app)),
        "think" | "t" => handle_think(app, args).await,
        "attach" | "img" => Some(handle_attach(app, args)),
        "undo" => Some(handle_undo(app).await),
        "rewind" => Some(handle_rewind(app, args).await),
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
    // End the current session
    let old_session = app.session_id.clone();
    if let Err(e) = app.db.end_session(&old_session).await {
        tracing::warn!(error = %e, "failed to end session on /clear");
    }

    // Create a new session
    let new_session = uuid::Uuid::new_v4().to_string();
    let agent_id = app.db.agent_id().to_owned();
    if let Err(e) = app.db.create_session(&new_session, &agent_id, "cli").await {
        tracing::warn!(error = %e, "failed to create new session on /clear");
        // Continue anyway — the display clear still works
    }

    // Notify the agent worker of the new session
    if app
        .agent_tx
        .send(AgentRequest::NewSession {
            session_id: new_session.clone(),
        })
        .is_err()
    {
        return "Error: Agent worker is not responding. Try /exit and restart.".to_string();
    }

    // Update app state
    app.session_id = new_session;
    app.messages.clear();
    app.scroll_offset = 0;
    app.last_seen_msg_id = 0;
    app.context_tokens = None;
    app.messages_layout = MessagesLayout::default();
    app.needs_redraw = true;

    "Chat cleared and session reset.".to_string()
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
    match mika_agent::compaction::maybe_compact(&app.db, app.llm.as_ref()).await {
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
            if let Ok(total) = app.db.total_core_memory_tokens().await {
                let _ = writeln!(out, "\nTotal: {total}/2000 tokens");
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

    if let Ok(people) = people
        && !people.is_empty()
    {
        found = true;
        let _ = writeln!(out, "People:");
        for p in &people {
            let rel = p.relationship.as_deref().unwrap_or("unknown");
            let _ = writeln!(out, "  {} ({})", p.canonical_name, rel);
        }
    }
    if let Ok(commitments) = commitments
        && !commitments.is_empty()
    {
        found = true;
        let _ = writeln!(out, "Commitments:");
        for c in &commitments {
            let due = c.due_date.as_deref().unwrap_or("no due date");
            let _ = writeln!(out, "  [{}] {} ({})", c.status, c.description, due);
        }
    }
    if let Ok(preferences) = preferences
        && !preferences.is_empty()
    {
        found = true;
        let _ = writeln!(out, "Preferences:");
        for p in &preferences {
            let _ = writeln!(out, "  {}: {}", p.category, p.value);
        }
    }
    if let Ok(events) = events
        && !events.is_empty()
    {
        found = true;
        let _ = writeln!(out, "Events:");
        for e in &events {
            let date = e.event_date.as_deref().unwrap_or("no date");
            let _ = writeln!(out, "  {} ({})", e.description, date);
        }
    }

    if !found {
        format!("No results for '{query}'.")
    } else {
        out
    }
}

async fn handle_reminders(app: &mut App<'_>) -> String {
    let tasks = match app.db.get_user_visible_tasks().await {
        Ok(t) => t,
        Err(_) => return "Failed to load reminders.".to_string(),
    };

    if tasks.is_empty() {
        return "No active reminders.".to_string();
    }

    let mut out = String::new();
    let _ = writeln!(out, "Active reminders:");
    for t in &tasks {
        let short_id = &t.id[..12.min(t.id.len())];
        let fire_at = t
            .next_fire_at
            .as_ref()
            .map(|s| mika_agent::db::format_ts(s))
            .unwrap_or_else(|| "unknown".to_string());
        let _ = writeln!(out, "  {}: {} (fires: {})", short_id, t.label, fire_at);
    }
    out
}

async fn handle_tasks(app: &mut App<'_>, args: &str) -> String {
    if let Some(id) = args.strip_prefix("cancel").map(|s| s.trim()) {
        if id.is_empty() {
            return "Usage: /tasks cancel <id>".to_string();
        }
        return match app.db.cancel_task(id).await {
            Ok(true) => format!("Cancelled task {id}."),
            Ok(false) => format!("Task {id} not found or already completed."),
            Err(e) => format!("Failed to cancel task: {e}"),
        };
    }

    let tasks = match app.db.get_schedulable_tasks().await {
        Ok(t) => t,
        Err(e) => return format!("Failed to load tasks: {e}"),
    };

    let pending: Vec<_> = tasks
        .iter()
        .filter(|t| t.status == "pending" || t.status == "in_progress")
        .collect();

    if pending.is_empty() {
        return "No pending tasks.".to_string();
    }

    let mut out = String::new();
    let _ = writeln!(out, "Pending tasks ({}):", pending.len());
    for t in &pending {
        let short_id = &t.id[..12.min(t.id.len())];
        let when = t
            .next_fire_at
            .as_ref()
            .map(|s| mika_agent::db::format_ts(s))
            .unwrap_or_else(|| t.trigger_type.clone());
        let _ = writeln!(
            out,
            "  {}: [{}] \"{}\" ({})",
            short_id, t.action_type, t.label, when
        );
    }
    out.push_str("  Use /tasks cancel <id> to cancel.");
    out
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
    if let Ok(configs) = app.db.list_customer_config().await
        && !configs.is_empty()
    {
        let _ = writeln!(out);
        let _ = writeln!(out, "Customer settings:");
        for (key, value) in &configs {
            let _ = writeln!(out, "  {key}: {value}");
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

use crate::cli::MODEL_ALIASES;

fn resolve_model_name(input: &str) -> Option<(&'static str, &'static str)> {
    let lower = input.to_lowercase();
    for &(alias, full_id, display) in MODEL_ALIASES {
        if lower == alias || lower == full_id {
            return Some((full_id, display));
        }
    }
    None
}

async fn handle_model(app: &mut App<'_>, args: &str) -> String {
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

            // Persist to config.toml first so validation reads the updated value.
            let config_path = app.home_dir.join("config.toml");
            // Parse provider/model if present to determine the right config key
            let (provider_prefix, model_name) = if let Some(slash_pos) = full_id.find('/') {
                let prefix = &full_id[..slash_pos];
                if prefix.parse::<mika_common::llm::ProviderKind>().is_ok() {
                    (prefix, &full_id[slash_pos + 1..])
                } else {
                    ("anthropic", full_id)
                }
            } else {
                ("anthropic", full_id)
            };
            let model_key = format!("{provider_prefix}_model");
            let persist_warning = match crate::commands::config::write_config_toml(
                &config_path,
                &model_key,
                model_name,
            ) {
                Ok(()) => String::new(),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to persist model to config.toml");
                    " (warning: failed to save — change won't survive restart)".to_string()
                }
            };

            // Pre-validate: ensure the new model works with the provider
            if let Err(e) = validate_provider_switch(&app.home_dir, &app.global_home) {
                return format!("Cannot switch to {display}: {e}");
            }

            app.model = full_id.to_string();
            if app
                .agent_tx
                .send(AgentRequest::SetModel {
                    model: full_id.to_string(),
                })
                .is_err()
            {
                return "Error: Agent worker is not responding. Try /exit and restart.".to_string();
            }
            app.needs_redraw = true;

            format!("Switched to {display} ({full_id}).{persist_warning}")
        }
        None => {
            let options: Vec<&str> = MODEL_ALIASES.iter().map(|&(a, _, _)| a).collect();
            format!("Unknown model: {args}\nAvailable: {}", options.join(", "))
        }
    }
}

async fn handle_provider(app: &mut App<'_>, args: &str) -> String {
    use mika_common::llm::ProviderKind;

    let args = args.trim();
    if args.is_empty() {
        // Show current provider and list all available
        let current = app.provider.to_string();
        let mut out = format!("Current provider: {current}\n\nAvailable providers:");
        for &p in ProviderKind::ALL {
            let marker = if p.to_string() == current {
                " (current)"
            } else {
                ""
            };
            let _ = write!(out, "\n  {} — {}{}", p, p.default_model(), marker);
        }
        let _ = write!(out, "\n\nUsage: /provider <name>");
        return out;
    }

    // Handle subcommands: "set model <name>", "set api_key <value>", "set base_url <value>"
    if args == "set" {
        return "Usage: /provider set <model|api_key|base_url> <value>".to_string();
    }
    if let Some(rest) = args.strip_prefix("set ") {
        let rest = rest.trim();
        if let Some((field, value)) = rest.split_once(' ') {
            let value = value.trim();
            let prefix = app.provider.config_prefix();
            let config_path = app.home_dir.join("config.toml");
            let key = match field {
                "model" => format!("{prefix}_model"),
                "base_url" => format!("{prefix}_base_url"),
                "api_key" => {
                    // API keys go to .env, not config.toml
                    let env_key = format!("MIKA_{}_API_KEY", prefix.to_uppercase());
                    let global_home = &app.global_home;
                    match mika_common::dotenv::set_env_var(global_home, &env_key, value) {
                        Ok(()) => {
                            return format!(
                                "Set {env_key} in .env (restart required for changes to take effect)"
                            );
                        }
                        Err(e) => return format!("Error: {e}"),
                    }
                }
                _ => return format!("Unknown field: {field}. Use: model, api_key, base_url"),
            };
            match crate::commands::config::write_config_toml(&config_path, &key, value) {
                Ok(()) => {
                    // If setting model, also update the display and agent worker
                    if field == "model" {
                        let full_id = if app.provider == ProviderKind::Anthropic {
                            value.to_string()
                        } else {
                            format!("{}/{}", app.provider, value)
                        };

                        // Pre-validate: load settings and try to build the provider
                        if let Err(e) = validate_provider_switch(&app.home_dir, &app.global_home) {
                            return format!("Error: cannot use model {value}: {e}");
                        }

                        app.model = full_id.clone();
                        if app
                            .agent_tx
                            .send(AgentRequest::SetModel { model: full_id })
                            .is_err()
                        {
                            return "Error: Agent worker is not responding. Try /exit and restart."
                                .to_string();
                        }
                        app.needs_redraw = true;
                    }
                    format!("Set {key} = \"{value}\"")
                }
                Err(e) => format!("Error writing config: {e}"),
            }
        } else {
            "Usage: /provider set <model|api_key|base_url> <value>".to_string()
        }
    } else {
        // Switch provider
        match args.parse::<ProviderKind>() {
            Ok(new_provider) => {
                if new_provider == app.provider {
                    return format!("Already using {new_provider}.");
                }

                // Pre-validate: load settings with the new provider and try to build it
                let config_path = app.home_dir.join("config.toml");
                match validate_provider_switch_for(&app.home_dir, &app.global_home, new_provider) {
                    Ok(()) => {}
                    Err(e) => return format!("Cannot switch to {new_provider}: {e}"),
                }

                let model = new_provider.default_model();
                let full_id = if new_provider == ProviderKind::Anthropic {
                    model.to_string()
                } else {
                    format!("{new_provider}/{model}")
                };
                app.model = full_id.clone();
                app.provider = new_provider;
                if app
                    .agent_tx
                    .send(AgentRequest::SetModel { model: full_id })
                    .is_err()
                {
                    return "Error: Agent worker is not responding. Try /exit and restart."
                        .to_string();
                }
                app.needs_redraw = true;

                // Persist provider switch to config.toml
                let persist_warning = match crate::commands::config::write_config_toml(
                    &config_path,
                    "llm_provider",
                    &new_provider.to_string(),
                ) {
                    Ok(()) => String::new(),
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to persist provider to config.toml");
                        " (warning: failed to save — change won't survive restart)".to_string()
                    }
                };

                format!("Switched to {new_provider} (model: {model}).{persist_warning}")
            }
            Err(e) => e,
        }
    }
}

/// Pre-validate that the current config can build an LLM provider.
/// Used after writing a config change to verify it works before updating the UI.
fn validate_provider_switch(
    home_dir: &std::path::Path,
    global_home: &std::path::Path,
) -> Result<(), String> {
    let settings = mika_common::config::Settings::load_for_agent(global_home, home_dir)
        .map_err(|e| format!("failed to load settings: {e}"))?;
    settings.make_llm_provider().map_err(|e| e.to_string())?;
    Ok(())
}

/// Pre-validate that switching to a specific provider will work (API key present, etc.).
fn validate_provider_switch_for(
    home_dir: &std::path::Path,
    global_home: &std::path::Path,
    provider: mika_common::llm::ProviderKind,
) -> Result<(), String> {
    let mut settings = mika_common::config::Settings::load_for_agent(global_home, home_dir)
        .map_err(|e| format!("failed to load settings: {e}"))?;
    settings.llm_provider = provider;
    settings.make_llm_provider().map_err(|e| e.to_string())?;
    Ok(())
}

async fn handle_export(app: &mut App<'_>) -> String {
    if app.messages.is_empty() {
        return "Nothing to export.".to_string();
    }

    // Team mode exports to the team directory instead of agent home
    let exports_dir = if let Some(ref team_dir) = app.team_dir {
        team_dir.join("exports")
    } else {
        app.home_dir.join("exports")
    };
    if let Err(e) = tokio::fs::create_dir_all(&exports_dir).await {
        return format!("Failed to create exports directory: {e}");
    }

    let timestamp = chrono::Utc::now().format("%Y-%m-%d-%H%M%S");
    let (label, filename) = if app.is_team_mode() {
        let name = app.team_name.as_deref().unwrap_or("team");
        (name.to_string(), format!("team-{name}-{timestamp}.md"))
    } else {
        let short_session = &app.session_id[..8.min(app.session_id.len())];
        (
            app.session_id.clone(),
            format!("session-{short_session}-{timestamp}.md"),
        )
    };
    let filepath = exports_dir.join(&filename);

    let mut content = String::from("# Mika Conversation Export\n\n");
    if app.is_team_mode() {
        let _ = writeln!(content, "Team: {label}");
    } else {
        let _ = writeln!(content, "Session: {label}");
        let _ = writeln!(content, "Model: {}", app.model);
    }
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

    let max_name_width = skills
        .iter()
        .map(|e| e.manifest.skill.name.len())
        .max()
        .unwrap_or(0);

    let mut always_on: Vec<&_> = skills
        .iter()
        .filter(|e| e.manifest.skill.always_on)
        .collect();
    let mut on_demand: Vec<&_> = skills
        .iter()
        .filter(|e| !e.manifest.skill.always_on)
        .collect();

    // Within each group: enabled first, disabled last
    always_on.sort_by_key(|e| !e.enabled);
    on_demand.sort_by_key(|e| !e.enabled);

    let mut out = format!("Loaded skills ({}):\n", skills.len());

    let format_entry = |out: &mut String, entry: &mika_agent::skills::index::SkillEntry| {
        let tool_count = entry.skill_tools.len();
        let tool_info = if tool_count == 0 {
            "—".to_string()
        } else if tool_count == 1 {
            "1 tool".to_string()
        } else {
            format!("{tool_count} tools")
        };
        let disabled = if entry.enabled { "" } else { " [disabled]" };
        let overridden = if entry.has_override {
            " [override]"
        } else {
            ""
        };
        let variants = {
            let count = entry.variant_count();
            if count > 0 {
                format!(" [variants: {count}]")
            } else {
                String::new()
            }
        };
        let _ = writeln!(
            out,
            "  ● {:<width$}  {:<9}  {}{}{}{}",
            entry.manifest.skill.name,
            tool_info,
            entry.manifest.skill.description,
            disabled,
            overridden,
            variants,
            width = max_name_width
        );
    };

    if !always_on.is_empty() {
        let _ = writeln!(out, "\n  ALWAYS ON");
        for entry in &always_on {
            format_entry(&mut out, entry);
        }
    }

    if !on_demand.is_empty() {
        let _ = writeln!(out, "\n  ON DEMAND");
        for entry in &on_demand {
            format_entry(&mut out, entry);
        }
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
            let variant_total = entry.variant_count();
            if variant_total > 0 {
                let providers = entry.variant_providers();
                let provider_list: Vec<_> = providers.iter().copied().collect();
                let mut variant_details = Vec::new();
                for p in &provider_list {
                    let model_count = entry.variant_models(p).len();
                    if model_count > 0 {
                        variant_details.push(format!("{p} ({model_count} models)"));
                    } else {
                        variant_details.push(p.to_string());
                    }
                }
                let _ = writeln!(
                    out,
                    "  Variants: {} total ({})",
                    variant_total,
                    variant_details.join(", ")
                );
            }
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

/// Show team info for both `/team` and `/status` in team mode.
fn handle_verbose(app: &mut App<'_>) -> String {
    if !app.is_team_mode() {
        return "Verbose mode is only available in team mode.".to_string();
    }
    app.verbose_mode = !app.verbose_mode;
    let state = if app.verbose_mode { "on" } else { "off" };
    format!("Verbose mode: {state}")
}

fn handle_team_info(app: &App<'_>) -> String {
    let team_name = app.team_name.as_deref().unwrap_or("unknown");
    let mut out = String::new();
    let _ = writeln!(out, "Team: {team_name}");
    match team::load_team(&app.global_home, team_name) {
        Ok(def) => {
            let _ = writeln!(out, "  Orchestrator: {}", def.team.orchestrator);
            let _ = writeln!(out, "  Max iterations: {}", def.flow.max_iterations);
            let _ = writeln!(out, "  Agents:");
            for a in &def.agents {
                let _ = writeln!(out, "    {} — {}", a.name, a.role);
            }
        }
        Err(e) => {
            let _ = writeln!(out, "  (failed to load team definition: {e})");
        }
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
         mika ask --team {name} \"{goal}\""
    )
}

async fn handle_think(app: &mut App<'_>, args: &str) -> Option<String> {
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
        if let Err(e) = app.db.set_customer_config("thinking_level", "off").await {
            tracing::warn!(error = %e, "failed to persist thinking level");
        }
        return Some("Thinking: off".to_string());
    }

    // Parse: first word might be a level
    let (first, rest) = match args.split_once(char::is_whitespace) {
        Some((f, r)) => (f, r.trim()),
        None => (args, ""),
    };

    // Resolve budget, level, and prompt text in one step
    let (budget, level, prompt) = match resolve_thinking_level(first) {
        Some((b, l)) if rest.is_empty() => {
            // /think high — set persistent level, no prompt
            app.thinking_level = Some((b, l));
            if let Err(e) = app.db.set_customer_config("thinking_level", l).await {
                tracing::warn!(error = %e, "failed to persist thinking level");
            }
            return Some(format!(
                "Thinking level: {l} ({b} tokens). All messages will use extended thinking."
            ));
        }
        Some((b, l)) => (b, l, rest),
        None => (10_000, "medium", args),
    };

    if app.status != AgentStatus::Idle {
        return Some("Agent is busy. Wait for the current response to finish.".to_string());
    }

    app.messages.push(ChatMessage {
        role: ChatRole::User,
        content: format!("[think:{level}] {prompt}"),
        rendered: None,
        channel: None,
    });

    let images = std::mem::take(&mut app.pending_images);
    let _ = app.agent_tx.send(AgentRequest::Message {
        text: prompt.to_string(),
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

async fn handle_undo(app: &mut App<'_>) -> String {
    handle_rewind_impl(app, 1, None).await
}

async fn handle_rewind(app: &mut App<'_>, args: &str) -> String {
    let args = args.trim();

    if args.is_empty() {
        return "Usage: /rewind <count> or /rewind to <message_id>".to_string();
    }

    // /rewind to <message_id>
    if let Some(id_str) = args
        .strip_prefix("to ")
        .or_else(|| args.strip_prefix("to\t"))
    {
        let id_str = id_str.trim();
        match id_str.parse::<i64>() {
            Ok(msg_id) if msg_id > 0 => return handle_rewind_impl(app, 0, Some(msg_id)).await,
            _ => return format!("Invalid message ID: '{id_str}'. Must be a positive integer."),
        }
    }

    // /rewind <count>
    match args.parse::<usize>() {
        Ok(count) if count > 0 => handle_rewind_impl(app, count, None).await,
        Ok(_) => "Rewind count must be at least 1.".to_string(),
        Err(_) => {
            format!("Invalid argument: '{args}'. Usage: /rewind <count> or /rewind to <message_id>")
        }
    }
}

async fn handle_rewind_impl(
    app: &mut App<'_>,
    exchange_count: usize,
    after_message_id: Option<i64>,
) -> String {
    use mika_agent::rewind;

    if app.status != AgentStatus::Idle {
        return "Cannot rewind while agent is busy.".to_string();
    }

    // Determine the anchor message ID and the session to rewind
    let (rewind_session, anchor_id) = if let Some(id) = after_message_id {
        (app.session_id.clone(), id)
    } else {
        // Find N recent exchanges (may fall back to a previous session)
        match rewind::find_recent_exchanges(&app.db, &app.session_id, exchange_count).await {
            Ok(Some(m)) => (m.session_id, m.anchor_message_id),
            Ok(None) => return "No exchanges found to rewind.".to_string(),
            Err(e) => return format!("Error finding exchanges: {e}"),
        }
    };

    // Preview
    let preview = match rewind::preview_rewind(&app.db, &rewind_session, anchor_id).await {
        Ok(p) => p,
        Err(e) => return format!("Error previewing rewind: {e}"),
    };

    if preview.blocked {
        return format!(
            "Rewind blocked: {}",
            preview.block_reason.unwrap_or_default()
        );
    }

    // Show preview and execute (no confirmation in TUI — the preview is shown as output)
    let preview_text = rewind::format_preview(&preview);

    // Execute
    let originating = if rewind_session != app.session_id {
        Some(app.session_id.as_str())
    } else {
        None
    };
    match rewind::execute_rewind(&app.db, &rewind_session, anchor_id, originating).await {
        Ok(result) => {
            // Remove rewound messages from the TUI display — but only if we
            // rewound the current session. Cross-session rewinds affect a prior
            // session whose messages were loaded at startup; the TUI display
            // should be fully refreshed instead.
            // Reload messages from DB for both same-session and cross-session
            // rewinds. The DB may contain intermediate assistant messages (tool_use)
            // that are never in app.messages, so counting-based truncation is unreliable.
            // This mirrors the startup loader in chat.rs.
            app.messages.clear();
            if let Ok(recent) = app.db.load_recent_messages(20).await {
                for msg in &recent {
                    let role = match msg.role.as_str() {
                        "user" => {
                            if msg.content.starts_with("A background task has completed.") {
                                continue;
                            }
                            ChatRole::User
                        }
                        "assistant" => ChatRole::Assistant,
                        "tool_result" => ChatRole::System,
                        _ => continue,
                    };
                    let channel = if msg.channel_type == "cli" {
                        None
                    } else {
                        Some(msg.channel_type.clone())
                    };
                    let content = if msg.role == "tool_result" {
                        let label = callback_label_from_metadata(&msg.metadata);
                        format!("[Task: {}] Result received", label)
                    } else {
                        msg.content.clone()
                    };
                    app.messages.push(ChatMessage {
                        role,
                        content,
                        rendered: None,
                        channel,
                    });
                }
            }
            app.scroll_offset = 0;
            app.messages_layout = MessagesLayout::default();

            let mut output = preview_text;
            output.push_str(&format!(
                "\n\nRewind complete: {} messages deleted, {} reversals applied.",
                result.messages_deleted, result.reversals_applied
            ));
            if result.tasks_deleted > 0 || result.tasks_cancelled > 0 {
                output.push_str(&format!(
                    "\nTasks: {} deleted, {} cancelled.",
                    result.tasks_deleted, result.tasks_cancelled
                ));
            }
            if !result.warnings.is_empty() {
                output.push_str("\n\nWarnings:");
                for w in &result.warnings {
                    output.push_str(&format!("\n  - {w}"));
                }
            }
            output
        }
        Err(e) => format!("Rewind failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use mika_agent::async_db::AsyncDatabase;
    use mika_agent::db::Database;
    use mika_agent::skills::SkillRegistry;
    use mika_common::llm::{LlmError, LlmProvider, LlmRequest, LlmResponse, ProviderKind};
    use tokio::sync::mpsc;

    /// Minimal no-op LLM provider for TUI handler tests.
    /// We never call send_message in handler tests — this just satisfies the type.
    struct NoopLlmProvider;

    #[async_trait::async_trait]
    impl LlmProvider for NoopLlmProvider {
        async fn send_message(&self, _request: &LlmRequest) -> Result<LlmResponse, LlmError> {
            Err(LlmError::ProviderError("noop provider".to_string()))
        }

        fn model_name(&self) -> &str {
            "noop"
        }

        fn provider_name(&self) -> &str {
            "noop"
        }

        fn max_tokens(&self) -> u32 {
            4096
        }

        fn supports_tool_calling(&self) -> bool {
            false
        }

        async fn check_health(&self) -> Result<(), LlmError> {
            Ok(())
        }
    }

    /// Test helper: construct a minimal App with captured agent_tx receiver.
    /// Returns (App, agent_rx, temp_dir) — temp_dir must be kept alive for the test duration.
    async fn test_app() -> (
        App<'static>,
        mpsc::UnboundedReceiver<AgentRequest>,
        tempfile::TempDir,
    ) {
        let temp_dir = tempfile::tempdir().unwrap();
        let home_dir = temp_dir.path().to_path_buf();
        let global_home = temp_dir.path().to_path_buf();

        let db = Database::open_in_memory().unwrap();
        let async_db = AsyncDatabase::new(db);

        // Create a session for the app
        let session_id = uuid::Uuid::new_v4().to_string();
        async_db
            .create_session(&session_id, "mika", "cli")
            .await
            .unwrap();

        let (agent_tx, agent_rx) = mpsc::unbounded_channel();
        let (_response_tx, response_rx) = mpsc::unbounded_channel();

        let llm: Arc<dyn LlmProvider> = Arc::new(NoopLlmProvider);
        let skills = Arc::new(SkillRegistry::empty());

        let app = App::new(
            agent_tx,
            response_rx,
            session_id,
            "claude-sonnet-4-6".to_string(),
            "Mika".to_string(),
            async_db,
            llm,
            home_dir,
            skills,
            "mika".to_string(),
            global_home,
            ProviderKind::Anthropic,
        );

        (app, agent_rx, temp_dir)
    }

    /// Drain all pending requests from the agent channel.
    fn drain_requests(rx: &mut mpsc::UnboundedReceiver<AgentRequest>) -> Vec<AgentRequest> {
        let mut requests = Vec::new();
        while let Ok(req) = rx.try_recv() {
            requests.push(req);
        }
        requests
    }

    // --- Existing tests ---

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
    fn test_handle_help_contains_new_commands() {
        let output = handle_help();
        assert!(output.contains("/think"));
        assert!(output.contains("/attach"));
        assert!(output.contains("/undo"));
        assert!(output.contains("/rewind"));
    }

    #[test]
    fn test_handle_help_contains_config_args() {
        let output = handle_help();
        assert!(output.contains("/config"));
        assert!(output.contains("set <key> <value>"));
    }

    #[test]
    fn test_settable_config_keys_allowlist() {
        assert!(is_settable_key("chat_id"));
        assert!(is_settable_key("timezone"));
        assert!(!is_settable_key("api_key"));
        assert!(!is_settable_key("db_path"));
    }

    #[test]
    fn test_team_mode_allowed_commands() {
        for cmd in &[
            "help", "h", "?", "clear", "exit", "quit", "q", "export", "agents", "teams", "team",
            "status", "stat",
        ] {
            assert!(
                TEAM_MODE_ALLOWED_COMMANDS.contains(cmd),
                "Expected '{cmd}' to be allowed in team mode"
            );
        }
    }

    #[test]
    fn test_team_mode_blocks_agent_commands() {
        for cmd in &[
            "compact", "memory", "model", "provider", "think", "soul", "config", "skills", "agent",
            "attach", "undo", "rewind",
        ] {
            assert!(
                !TEAM_MODE_ALLOWED_COMMANDS.contains(cmd),
                "Expected '{cmd}' to be blocked in team mode"
            );
        }
    }

    // --- /clear tests ---

    #[tokio::test]
    async fn test_clear_creates_new_session() {
        let (mut app, _rx, _tmp) = test_app().await;
        let old_session = app.session_id.clone();

        // Add a message to verify it gets cleared
        app.messages.push(ChatMessage {
            role: ChatRole::User,
            content: "hello".to_string(),
            rendered: None,
            channel: None,
        });

        let output = handle_clear(&mut app, "").await;

        assert!(output.contains("session reset"));
        assert_ne!(app.session_id, old_session, "session_id should change");
        assert!(app.messages.is_empty(), "messages should be cleared");
        assert_eq!(app.scroll_offset, 0);
        assert_eq!(app.last_seen_msg_id, 0);
        assert!(app.context_tokens.is_none());
        assert!(app.needs_redraw);
    }

    #[tokio::test]
    async fn test_clear_sends_new_session_request() {
        let (mut app, mut rx, _tmp) = test_app().await;

        handle_clear(&mut app, "").await;

        let requests = drain_requests(&mut rx);
        assert_eq!(requests.len(), 1, "should send exactly one request");
        match &requests[0] {
            AgentRequest::NewSession { session_id } => {
                assert_eq!(
                    session_id, &app.session_id,
                    "session_id should match app's new session"
                );
            }
            _ => panic!("expected NewSession request"),
        }
    }

    #[tokio::test]
    async fn test_clear_ends_old_session() {
        let (mut app, _rx, _tmp) = test_app().await;
        let old_session = app.session_id.clone();

        handle_clear(&mut app, "").await;

        // Verify old session is ended (has ended_at set)
        let old = app.db.get_session(&old_session).await.unwrap();
        assert!(
            old.map(|s| s.ended_at.is_some()).unwrap_or(false),
            "old session should have ended_at set"
        );
    }

    // --- /model tests ---

    #[test]
    fn test_model_alias_resolves() {
        let result = resolve_model_name("opus");
        assert!(result.is_some());
        let (full_id, _display) = result.unwrap();
        assert!(full_id.contains("opus"), "should resolve to an opus model");
    }

    #[test]
    fn test_model_unknown_returns_none() {
        let result = resolve_model_name("nonexistent-model-xyz");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_model_no_args_shows_current() {
        let (mut app, _rx, _tmp) = test_app().await;

        let output = handle_model(&mut app, "").await;

        assert!(output.contains("Current model:"));
        assert!(output.contains("claude-sonnet-4-6"));
        assert!(output.contains("Usage: /model"));
    }

    #[tokio::test]
    async fn test_model_unknown_returns_error() {
        let (mut app, _rx, _tmp) = test_app().await;
        let original_model = app.model.clone();

        let output = handle_model(&mut app, "nonexistent-xyz").await;

        assert!(output.contains("Unknown model"));
        assert_eq!(
            app.model, original_model,
            "model should not change on error"
        );
    }

    // --- /provider tests ---

    #[tokio::test]
    async fn test_provider_no_args_shows_list() {
        let (mut app, _rx, _tmp) = test_app().await;

        let output = handle_provider(&mut app, "").await;

        assert!(output.contains("Current provider:"));
        assert!(output.contains("anthropic"));
        assert!(output.contains("openai"));
        assert!(output.contains("Usage: /provider"));
    }

    #[tokio::test]
    async fn test_provider_already_using() {
        let (mut app, _rx, _tmp) = test_app().await;

        let output = handle_provider(&mut app, "anthropic").await;

        assert!(output.contains("Already using"));
    }

    #[tokio::test]
    async fn test_provider_invalid_name() {
        let (mut app, _rx, _tmp) = test_app().await;
        let original_provider = app.provider;

        let output = handle_provider(&mut app, "nonexistent-provider").await;

        // Should return an error message (from ProviderKind parse error)
        assert!(
            output.contains("Unknown")
                || output.contains("unknown")
                || output.contains("Invalid")
                || output.contains("nknown"),
            "expected error for invalid provider, got: {output}"
        );
        assert_eq!(
            app.provider, original_provider,
            "provider should not change"
        );
    }

    #[tokio::test]
    async fn test_provider_set_subcommand_usage() {
        let (mut app, _rx, _tmp) = test_app().await;

        let output = handle_provider(&mut app, "set").await;

        assert!(
            output.contains("Usage:"),
            "expected usage message, got: {output}"
        );
    }

    #[tokio::test]
    async fn test_provider_set_unknown_field() {
        let (mut app, _rx, _tmp) = test_app().await;

        let output = handle_provider(&mut app, "set unknown_field value").await;

        assert!(output.contains("Unknown field"));
    }

    // --- Channel send failure tests ---

    #[tokio::test]
    async fn test_clear_with_closed_channel_returns_error() {
        let (mut app, rx, _tmp) = test_app().await;
        // Drop the receiver to close the channel
        drop(rx);

        let output = handle_clear(&mut app, "").await;

        assert!(
            output.contains("not responding"),
            "should report worker error: {output}"
        );
    }
}

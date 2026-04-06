use std::fmt::Write;

use mika_agent::config_keys::{is_settable_key, settable_keys_display, validate_config_value};
use mika_common::agent;
use mika_common::home;
use mika_common::team;

use crate::tui::app::{
    AgentRequest, AgentStatus, App, ChatMessage, ChatRole, MessagesLayout, SelectionState,
    callback_label_from_metadata,
};
use crate::tui::commands::{COMMANDS, parse_command, resolve_thinking_level};
use crate::tui::input;

/// Error message shown when the agent worker channel is closed.
const WORKER_NOT_RESPONDING: &str = "Error: Agent worker is not responding. Try /exit and restart.";

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
        return WORKER_NOT_RESPONDING.to_string();
    }

    // Drain any pending responses from the old session to prevent ghost messages
    while app.agent_rx.try_recv().is_ok() {}

    // Update app state — session
    app.session_id = new_session;
    app.messages.clear();
    app.scroll_offset = 0;
    app.last_seen_msg_id = app.db.max_message_id().await.unwrap_or(0);
    app.context_tokens = None;
    app.messages_layout = MessagesLayout::default();

    // Response state
    app.pending_response = None;
    app.reveal_index = 0;
    app.status = AgentStatus::Idle;

    // Input state
    app.pending_images.clear();
    app.pending_command = None;

    // UI state
    app.has_new_message = false;
    app.selection_state = SelectionState::None;
    app.pending_task_count = 0;

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

#[cfg(test)]
fn resolve_model_name(input: &str) -> Option<(&'static str, &'static str)> {
    let lower = input.to_lowercase();
    for &(alias, full_id, display) in crate::cli::MODEL_ALIASES {
        if lower == alias || lower == full_id {
            return Some((full_id, display));
        }
    }
    None
}

async fn handle_model(app: &mut App<'_>, args: &str) -> String {
    let args = args.trim();
    if args.is_empty() {
        match crate::commands::model::list_models(&app.global_home, &app.home_dir).await {
            Ok(result) => {
                let mut out = format!(
                    "Current model: {}\n\nAvailable models for {}:",
                    result.current_model, result.provider
                );
                if result.models.is_empty() {
                    let _ = write!(
                        out,
                        "\n  (no models available \u{2014} type a model name directly)"
                    );
                } else {
                    for m in &result.models {
                        let marker = if m.current { " (current)" } else { "" };
                        let _ = write!(out, "\n  {}{}", m.id, marker);
                    }
                }
                let _ = write!(out, "\n\nAliases:");
                for a in &result.aliases {
                    let _ = write!(out, "\n  {} \u{2014} {}", a.alias, a.display);
                }
                let _ = write!(out, "\n\nUsage: /model <name> or /model <alias>");
                out
            }
            Err(e) => format!("Error: {e}"),
        }
    } else {
        match crate::commands::model::switch_model(&app.global_home, &app.home_dir, args) {
            Ok(result) => {
                let target_provider: mika_common::llm::ProviderKind =
                    result.provider.parse().unwrap_or(app.provider);
                if result.provider_changed {
                    app.provider = target_provider;
                }
                let display_id =
                    crate::commands::provider::format_display_model(target_provider, &result.model);
                let worker_model =
                    crate::commands::provider::format_worker_model(target_provider, &result.model);
                app.model = display_id.clone();
                if app
                    .agent_tx
                    .send(AgentRequest::SetModel {
                        model: worker_model,
                    })
                    .is_err()
                {
                    return WORKER_NOT_RESPONDING.to_string();
                }
                app.needs_redraw = true;

                let mut out = format!("Switched to {display_id}.");
                if result.provider_changed {
                    out.push_str(&format!(" (switched provider to {})", result.provider));
                }
                for w in &result.warnings {
                    out.push_str(&format!(" (warning: {w})"));
                }
                out
            }
            Err(e) => format!("{e}"),
        }
    }
}

async fn handle_provider(app: &mut App<'_>, args: &str) -> String {
    use mika_common::llm::ProviderKind;

    let args = args.trim();
    if args.is_empty() {
        // List providers using shared logic
        let providers = crate::commands::provider::list_providers(
            &app.global_home,
            &app.home_dir,
            app.provider,
        );
        let mut out = format!("Current provider: {}\n\nAvailable providers:", app.provider);
        for p in &providers {
            let marker = if p.current { " (current)" } else { "" };
            let _ = write!(out, "\n  {} — {}{}", p.name, p.model, marker);
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
            match crate::commands::provider::set_provider_field(
                &app.global_home,
                &app.home_dir,
                field,
                value,
            ) {
                Ok(result) => {
                    // If setting model, also update the display and agent worker
                    if field == "model" {
                        let display_id =
                            crate::commands::provider::format_display_model(app.provider, value);
                        let worker_model =
                            crate::commands::provider::format_worker_model(app.provider, value);
                        app.model = display_id;
                        if app
                            .agent_tx
                            .send(AgentRequest::SetModel {
                                model: worker_model,
                            })
                            .is_err()
                        {
                            return WORKER_NOT_RESPONDING.to_string();
                        }
                        app.needs_redraw = true;
                    }
                    if field == "api_key" || field == "api-key" {
                        format!(
                            "Set {} in .env (restart required for changes to take effect)",
                            result.field
                        )
                    } else {
                        format!("Set {} = \"{}\"", result.field, result.value)
                    }
                }
                Err(e) => format!("Error: {e}"),
            }
        } else {
            "Usage: /provider set <model|api_key|base_url> <value>".to_string()
        }
    } else {
        // Switch provider using shared logic (prefetch_models=false, TUI does fire-and-forget)
        match crate::commands::provider::switch_provider(
            &app.global_home,
            &app.home_dir,
            match args.parse::<ProviderKind>() {
                Ok(p) => p,
                Err(e) => return e,
            },
            false,
        )
        .await
        {
            Ok(result) => {
                let new_provider: ProviderKind = result.provider.parse().unwrap_or(app.provider);
                let display_id =
                    crate::commands::provider::format_display_model(new_provider, &result.model);
                let worker_model =
                    crate::commands::provider::format_worker_model(new_provider, &result.model);
                app.model = display_id;
                app.provider = new_provider;
                if app
                    .agent_tx
                    .send(AgentRequest::SetModel {
                        model: worker_model,
                    })
                    .is_err()
                {
                    return WORKER_NOT_RESPONDING.to_string();
                }
                app.needs_redraw = true;

                // Fire-and-forget model cache warm (TUI keeps running, unlike CLI)
                let home = app.home_dir.clone();
                let global = app.global_home.clone();
                tokio::spawn(async move {
                    let _ =
                        crate::commands::provider::fetch_models(&global, &home, new_provider).await;
                });

                let mut out = format!("Switched to {new_provider} (model: {}).", result.model);
                for w in &result.warnings {
                    let _ = write!(out, "\n{w}");
                }
                out
            }
            Err(e) => format!("{e}"),
        }
    }
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
    let skipped = app.skills.skipped();

    if skills.is_empty() && skipped.is_empty() {
        return "No skills loaded.".to_string();
    }

    let max_name_width = skills
        .iter()
        .map(|e| e.manifest.skill.name.len())
        .chain(skipped.iter().map(|s| s.name.len()))
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

    let mut out = if skipped.is_empty() {
        format!("Loaded skills ({}):\n", skills.len())
    } else {
        format!(
            "Loaded skills ({}, {} skipped):\n",
            skills.len(),
            skipped.len()
        )
    };

    let format_entry = |out: &mut String, entry: &mika_agent::skills::index::SkillEntry| {
        let tool_count = entry.skill_tools.len();
        let tool_info = if tool_count == 0 {
            "\u{2014}".to_string()
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
            "  \u{25cf} {:<width$}  {:<9}  {}{}{}{}",
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

    if !skipped.is_empty() {
        let _ = writeln!(out, "\n  SKIPPED");
        for entry in skipped {
            let _ = writeln!(
                out,
                "  \u{2717} {:<width$}  {}",
                entry.name,
                entry.reason,
                width = max_name_width
            );
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
    async fn test_model_direct_name_sets_model() {
        let (mut app, mut rx, tmp) = test_app().await;

        // Switch to DeepSeek first (easier to test with fake API key)
        let env_path = tmp.path().join(".env");
        std::fs::write(&env_path, "MIKA_DEEPSEEK_API_KEY=sk-fake-key-for-test\n").unwrap();
        let _ = handle_provider(&mut app, "deepseek").await;
        drain_requests(&mut rx); // clear SetModel from provider switch

        // Use a direct model name (not an alias)
        let output = handle_model(&mut app, "deepseek-reasoner").await;

        assert!(
            output.contains("Switched to deepseek/deepseek-reasoner"),
            "should accept direct model name, got: {output}"
        );
        assert_eq!(app.model, "deepseek/deepseek-reasoner");

        // Should have sent SetModel to the worker
        let requests = drain_requests(&mut rx);
        assert!(requests.iter().any(
            |r| matches!(r, AgentRequest::SetModel { model } if model == "deepseek/deepseek-reasoner")
        ));
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
        // Should return an error, not a success
        assert!(
            !output.contains("Switched"),
            "should not switch to invalid provider, got: {output}"
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

    // --- /clear comprehensive state reset tests ---

    #[tokio::test]
    async fn test_clear_resets_all_state_fields() {
        let (mut app, _rx, _tmp) = test_app().await;

        // Set all fields to non-default values
        app.pending_response = Some("partial response text".to_string());
        app.reveal_index = 42;
        app.has_new_message = true;
        app.selection_state = SelectionState::Selected {
            start: crate::tui::app::MessagePosition {
                message_idx: 0,
                text_pos: crate::tui::app::TextPosition {
                    line: 0,
                    char_offset: 0,
                },
            },
            end: crate::tui::app::MessagePosition {
                message_idx: 0,
                text_pos: crate::tui::app::TextPosition {
                    line: 0,
                    char_offset: 5,
                },
            },
        };
        app.pending_command = Some("/model opus".to_string());
        app.pending_task_count = 3;
        app.active_background_task_count = 2;
        app.status = AgentStatus::Thinking;

        handle_clear(&mut app, "").await;

        assert!(
            app.pending_response.is_none(),
            "pending_response should be None"
        );
        assert_eq!(app.reveal_index, 0, "reveal_index should be 0");
        assert!(!app.has_new_message, "has_new_message should be false");
        assert!(
            matches!(app.selection_state, SelectionState::None),
            "selection_state should be None"
        );
        assert!(
            app.pending_command.is_none(),
            "pending_command should be None"
        );
        assert_eq!(app.pending_task_count, 0, "pending_task_count should be 0");
        assert_eq!(
            app.active_background_task_count, 2,
            "active_background_task_count should be preserved (agent-scoped, not session-scoped)"
        );
        assert_eq!(app.status, AgentStatus::Idle, "status should be Idle");
    }

    #[tokio::test]
    async fn test_clear_while_thinking() {
        let (mut app, _rx, _tmp) = test_app().await;
        app.status = AgentStatus::Thinking;

        handle_clear(&mut app, "").await;

        assert_eq!(app.status, AgentStatus::Idle);
    }

    #[tokio::test]
    async fn test_clear_while_responding() {
        let (mut app, _rx, _tmp) = test_app().await;
        app.status = AgentStatus::Responding(42);
        app.pending_response = Some("partial output being revealed".to_string());
        app.reveal_index = 10;

        handle_clear(&mut app, "").await;

        assert_eq!(app.status, AgentStatus::Idle);
        assert!(app.pending_response.is_none());
        assert_eq!(app.reveal_index, 0);
    }

    #[tokio::test]
    async fn test_clear_with_pending_images() {
        use crate::tui::attachment::ImageAttachment;

        let (mut app, _rx, _tmp) = test_app().await;
        app.pending_images.push(ImageAttachment {
            base64_data: "dGVzdA==".to_string(),
            media_type: "image/png".to_string(),
            size_bytes: 4,
            label: "test.png".to_string(),
        });

        handle_clear(&mut app, "").await;

        assert!(
            app.pending_images.is_empty(),
            "pending_images should be cleared"
        );
    }

    #[tokio::test]
    async fn test_clear_preserves_preferences() {
        let (mut app, _rx, _tmp) = test_app().await;
        app.thinking_level = Some((8192, "medium"));
        let original_model = app.model.clone();
        let original_provider = app.provider;

        handle_clear(&mut app, "").await;

        assert_eq!(
            app.thinking_level,
            Some((8192, "medium")),
            "thinking_level should be preserved"
        );
        assert_eq!(app.model, original_model, "model should be preserved");
        assert_eq!(
            app.provider, original_provider,
            "provider should be preserved"
        );
    }

    // --- /skills skipped section tests ---

    #[tokio::test]
    async fn test_skills_shows_skipped_section() {
        use mika_agent::skills::index::SkippedSkill;

        let (mut app, _rx, _tmp) = test_app().await;
        app.skills = Arc::new(SkillRegistry::with_skipped(vec![
            SkippedSkill {
                name: "broken-skill".to_string(),
                reason: "broken symlink".to_string(),
            },
            SkippedSkill {
                name: "bad-manifest".to_string(),
                reason: "invalid TOML: unexpected key".to_string(),
            },
        ]));

        let output = handle_skills(&app);

        assert!(output.contains("SKIPPED"), "should have SKIPPED section");
        assert!(
            output.contains("broken-skill"),
            "should show broken skill name"
        );
        assert!(
            output.contains("broken symlink"),
            "should show broken skill reason"
        );
        assert!(
            output.contains("bad-manifest"),
            "should show bad manifest name"
        );
        assert!(
            output.contains("invalid TOML"),
            "should show parse error reason"
        );
        assert!(
            output.contains("2 skipped"),
            "header should show skipped count"
        );
    }

    #[tokio::test]
    async fn test_skills_hides_skipped_when_none() {
        let (app, _rx, _tmp) = test_app().await;

        let output = handle_skills(&app);

        // Empty registry — no skills, no skipped
        assert!(
            !output.contains("SKIPPED"),
            "should not have SKIPPED section"
        );
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

    // --- #442: provider/model config state fixes ---

    #[tokio::test]
    async fn test_provider_switch_reads_configured_model() {
        let (mut app, _rx, tmp) = test_app().await;

        // Pre-configure deepseek_model in config.toml
        let config_path = tmp.path().join("config.toml");
        std::fs::write(
            &config_path,
            "llm_provider = \"anthropic\"\ndeepseek_model = \"deepseek-reasoner\"\n",
        )
        .unwrap();

        // Also need a fake API key for validation to pass
        let env_path = tmp.path().join(".env");
        std::fs::write(&env_path, "MIKA_DEEPSEEK_API_KEY=sk-fake-key-for-test\n").unwrap();

        let output = handle_provider(&mut app, "deepseek").await;

        // Should use the configured model, not the default
        assert!(
            output.contains("deepseek-reasoner"),
            "should show configured model, got: {output}"
        );
        assert_eq!(app.provider, ProviderKind::DeepSeek);
    }

    #[tokio::test]
    async fn test_provider_switch_persists_default_model() {
        let (mut app, _rx, tmp) = test_app().await;

        // Set up API key but no deepseek_model
        let config_path = tmp.path().join("config.toml");
        std::fs::write(&config_path, "llm_provider = \"anthropic\"\n").unwrap();
        let env_path = tmp.path().join(".env");
        std::fs::write(&env_path, "MIKA_DEEPSEEK_API_KEY=sk-fake-key-for-test\n").unwrap();

        let _output = handle_provider(&mut app, "deepseek").await;

        // Config should now have deepseek_model persisted
        let config_content = std::fs::read_to_string(&config_path).unwrap();
        assert!(
            config_content.contains("deepseek_model"),
            "should persist default model, config: {config_content}"
        );
    }

    #[tokio::test]
    async fn test_provider_switch_warns_max_tokens() {
        let (mut app, _rx, tmp) = test_app().await;

        // Set up: llm_max_tokens exceeds DeepSeek's 8192 limit
        let config_path = tmp.path().join("config.toml");
        std::fs::write(
            &config_path,
            "llm_provider = \"anthropic\"\nllm_max_tokens = 16384\n",
        )
        .unwrap();
        let env_path = tmp.path().join(".env");
        std::fs::write(&env_path, "MIKA_DEEPSEEK_API_KEY=sk-fake-key-for-test\n").unwrap();

        let output = handle_provider(&mut app, "deepseek").await;

        assert!(
            output.contains("exceeds deepseek's limit"),
            "should warn about max_tokens, got: {output}"
        );
    }

    #[tokio::test]
    async fn test_model_cross_provider_alias_switches_provider() {
        let (mut app, mut rx, tmp) = test_app().await;

        // Set up: start on Anthropic, switch via deepseek alias
        let env_path = tmp.path().join(".env");
        std::fs::write(&env_path, "MIKA_DEEPSEEK_API_KEY=sk-fake-key-for-test\n").unwrap();

        assert_eq!(app.provider, ProviderKind::Anthropic);
        let output = handle_model(&mut app, "deepseek").await;

        // Should switch provider to DeepSeek
        assert_eq!(
            app.provider,
            ProviderKind::DeepSeek,
            "should switch provider for cross-provider alias"
        );
        assert!(
            output.contains("switched provider to deepseek"),
            "should note provider switch, got: {output}"
        );

        // Verify SetModel was sent
        let requests = drain_requests(&mut rx);
        assert!(
            requests
                .iter()
                .any(|r| matches!(r, AgentRequest::SetModel { .. }))
        );
    }

    #[tokio::test]
    async fn test_model_direct_with_provider_prefix() {
        let (mut app, _rx, tmp) = test_app().await;

        let env_path = tmp.path().join(".env");
        std::fs::write(&env_path, "MIKA_DEEPSEEK_API_KEY=sk-fake-key-for-test\n").unwrap();

        let output = handle_model(&mut app, "deepseek/deepseek-reasoner").await;

        assert_eq!(app.provider, ProviderKind::DeepSeek);
        assert!(
            output.contains("deepseek/deepseek-reasoner"),
            "should set full model id, got: {output}"
        );
    }

    #[tokio::test]
    async fn test_model_no_args_shows_provider_models() {
        let (mut app, _rx, _tmp) = test_app().await;

        let output = handle_model(&mut app, "").await;

        // Anthropic uses hardcoded models
        assert!(
            output.contains("Available models for anthropic"),
            "should show provider models, got: {output}"
        );
        assert!(
            output.contains("claude-sonnet-4-6"),
            "should list Anthropic models, got: {output}"
        );
        assert!(output.contains("Aliases:"), "should show aliases section");
    }

    #[test]
    fn test_max_output_tokens_known_providers() {
        // Verify key providers have sensible limits
        assert_eq!(ProviderKind::DeepSeek.max_output_tokens(), 8_192);
        assert_eq!(ProviderKind::Anthropic.max_output_tokens(), 128_000);
        assert_eq!(ProviderKind::OpenAi.max_output_tokens(), 16_384);
        assert_eq!(ProviderKind::Groq.max_output_tokens(), 8_192);
    }

    // --- #451: provider switch worker propagation ---

    #[tokio::test]
    async fn test_provider_switch_worker_gets_provider_prefix() {
        let (mut app, mut rx, tmp) = test_app().await;

        // Setup: start on Anthropic, switch to DeepSeek
        let env_path = tmp.path().join(".env");
        std::fs::write(&env_path, "MIKA_DEEPSEEK_API_KEY=sk-fake-key-for-test\n").unwrap();

        let _output = handle_provider(&mut app, "deepseek").await;

        // Worker message must contain "deepseek/" prefix so worker can switch provider
        let requests = drain_requests(&mut rx);
        assert!(
            requests.iter().any(
                |r| matches!(r, AgentRequest::SetModel { model } if model.starts_with("deepseek/"))
            ),
            "SetModel should contain provider prefix"
        );
    }

    #[tokio::test]
    async fn test_provider_switch_to_anthropic_worker_gets_prefix() {
        let (mut app, mut rx, tmp) = test_app().await;

        // Set API keys via process env (test_app uses global_home == agent_home,
        // so .env file isn't loaded by load_for_agent).
        let env_path = tmp.path().join(".env");
        std::fs::write(&env_path, "MIKA_DEEPSEEK_API_KEY=sk-fake-key-for-test\n").unwrap();

        // Switch to DeepSeek first
        let _ = handle_provider(&mut app, "deepseek").await;
        drain_requests(&mut rx);

        // Now switch back to Anthropic — need API key in config since .env isn't read
        let config_path = tmp.path().join("config.toml");
        std::fs::write(
            &config_path,
            "llm_provider = \"deepseek\"\nanthropic_model = \"claude-sonnet-4-6\"\nanthropic_api_key = \"sk-ant-fake-key\"\n",
        )
        .unwrap();
        let _output = handle_provider(&mut app, "anthropic").await;

        // Worker message must contain "anthropic/" prefix even for Anthropic
        let requests = drain_requests(&mut rx);
        assert!(
            requests.iter().any(
                |r| matches!(r, AgentRequest::SetModel { model } if model.starts_with("anthropic/"))
            ),
            "SetModel for Anthropic should contain provider prefix"
        );
        // Display value should NOT have the prefix
        assert_eq!(app.model, "claude-sonnet-4-6");
    }

    #[tokio::test]
    async fn test_model_switch_worker_gets_provider_prefix() {
        let (mut app, mut rx, tmp) = test_app().await;

        // Need API key in config (test_app uses global_home == agent_home, .env not read)
        let config_path = tmp.path().join("config.toml");
        std::fs::write(
            &config_path,
            "llm_provider = \"anthropic\"\nanthropic_api_key = \"sk-ant-fake-key\"\n",
        )
        .unwrap();

        // Direct model name on Anthropic — worker should get "anthropic/" prefix
        let output = handle_model(&mut app, "claude-opus-4-6").await;

        assert!(
            output.contains("Switched to"),
            "should switch model, got: {output}"
        );

        let requests = drain_requests(&mut rx);
        assert!(
            requests.iter().any(
                |r| matches!(r, AgentRequest::SetModel { model } if model == "anthropic/claude-opus-4-6")
            ),
            "SetModel should have anthropic/ prefix"
        );
        // Display should be bare for Anthropic
        assert_eq!(app.model, "claude-opus-4-6");
    }

    #[tokio::test]
    async fn test_provider_listing_shows_configured_model() {
        let (mut app, _rx, tmp) = test_app().await;

        // Configure a non-default model for DeepSeek
        let config_path = tmp.path().join("config.toml");
        std::fs::write(
            &config_path,
            "llm_provider = \"anthropic\"\ndeepseek_model = \"deepseek-reasoner\"\n",
        )
        .unwrap();

        let output = handle_provider(&mut app, "").await;

        // Should show the configured model, not the default "deepseek-chat"
        assert!(
            output.contains("deepseek-reasoner"),
            "should show configured model for deepseek, got: {output}"
        );
    }

    #[test]
    fn test_format_helpers() {
        use crate::commands::provider::{format_display_model, format_worker_model};
        use mika_common::llm::ProviderKind;

        // format_worker_model always includes prefix
        assert_eq!(
            format_worker_model(ProviderKind::Anthropic, "claude-sonnet-4-6"),
            "anthropic/claude-sonnet-4-6"
        );
        assert_eq!(
            format_worker_model(ProviderKind::DeepSeek, "deepseek-chat"),
            "deepseek/deepseek-chat"
        );

        // format_display_model: no prefix for Anthropic, prefix for others
        assert_eq!(
            format_display_model(ProviderKind::Anthropic, "claude-sonnet-4-6"),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            format_display_model(ProviderKind::DeepSeek, "deepseek-chat"),
            "deepseek/deepseek-chat"
        );
    }

    #[tokio::test]
    async fn test_model_alias_on_openrouter_uses_provider_prefix() {
        let (mut app, mut rx, tmp) = test_app().await;

        // Set up: user is on OpenRouter
        app.provider = ProviderKind::OpenRouter;
        let config_path = tmp.path().join("config.toml");
        std::fs::write(
            &config_path,
            "llm_provider = \"openrouter\"\nopenrouter_model = \"anthropic/claude-sonnet-4\"\n",
        )
        .unwrap();
        let env_path = tmp.path().join(".env");
        std::fs::write(
            &env_path,
            "MIKA_OPENROUTER_API_KEY=sk-or-fake-key-for-test\n",
        )
        .unwrap();

        // Using "sonnet" alias on OpenRouter should set the model to
        // "anthropic/claude-sonnet-4-6" (valid OpenRouter model name),
        // NOT "claude-sonnet-4-6" (invalid bare name).
        let output = handle_model(&mut app, "sonnet").await;

        // Should stay on OpenRouter (not switch to Anthropic)
        assert_eq!(
            app.provider,
            ProviderKind::OpenRouter,
            "should stay on OpenRouter, got: {:?}",
            app.provider
        );
        assert!(
            output.contains("openrouter/anthropic/claude-sonnet-4-6"),
            "should show openrouter model with anthropic prefix, got: {output}"
        );

        // Worker should receive the full prefixed model
        let requests = drain_requests(&mut rx);
        assert!(
            requests.iter().any(
                |r| matches!(r, AgentRequest::SetModel { model } if model == "openrouter/anthropic/claude-sonnet-4-6")
            ),
            "SetModel should have openrouter/anthropic/ prefix"
        );

        // Config should have openrouter_model with anthropic prefix
        let config_content = std::fs::read_to_string(&config_path).unwrap();
        assert!(
            config_content.contains("anthropic/claude-sonnet-4-6"),
            "should persist model with anthropic prefix, config: {config_content}"
        );
    }

    #[tokio::test]
    async fn test_model_on_openrouter_keeps_slash_model_name() {
        let (mut app, mut rx, tmp) = test_app().await;

        // Set up: user is on OpenRouter
        app.provider = ProviderKind::OpenRouter;
        let config_path = tmp.path().join("config.toml");
        std::fs::write(&config_path, "llm_provider = \"openrouter\"\n").unwrap();
        let env_path = tmp.path().join(".env");
        std::fs::write(
            &env_path,
            "MIKA_OPENROUTER_API_KEY=sk-or-fake-key-for-test\n",
        )
        .unwrap();

        // When on OpenRouter, "qwen/qwen-plus" should be treated as an OpenRouter model
        // name, NOT as a cross-provider switch to the Qwen provider.
        let output = handle_model(&mut app, "qwen/qwen-plus").await;

        // Should stay on OpenRouter, not switch to Qwen
        assert_eq!(
            app.provider,
            ProviderKind::OpenRouter,
            "should stay on OpenRouter, got: {:?}",
            app.provider
        );
        assert!(
            output.contains("openrouter/qwen/qwen-plus"),
            "should show openrouter model, got: {output}"
        );

        // Worker should receive openrouter/qwen/qwen-plus (not qwen/qwen-plus)
        let requests = drain_requests(&mut rx);
        assert!(
            requests.iter().any(
                |r| matches!(r, AgentRequest::SetModel { model } if model == "openrouter/qwen/qwen-plus")
            ),
            "SetModel should have openrouter/ prefix"
        );

        // Config should have openrouter_model, not qwen_model
        let config_content = std::fs::read_to_string(&config_path).unwrap();
        assert!(
            config_content.contains("openrouter_model"),
            "should persist to openrouter_model, config: {config_content}"
        );
        assert!(
            !config_content.contains("qwen_model"),
            "should NOT create qwen_model, config: {config_content}"
        );
    }
}

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use chrono::{Local, Utc};
use serde::Serialize;

use crate::cli::{LogsActivityArgs, OutputFormat};
use crate::init;

/// Print the resolved log file paths for the given agent.
///
/// Two sinks exist:
/// - **Server log:** `MIKA_SERVER_LOG_FILE` (or `/var/log/mika/server.log` fallback) — contains
///   all runtime events from the long-running mika-server daemon, filterable by `agent_id`.
/// - **Per-agent CLI log:** `~/.mika/agents/<name>/logs/mika.log.YYYY-MM-DD` — contains events
///   from discrete CLI invocations (`mika ask`, `mika chat`, etc.) for that agent only.
pub fn run(agent_name: &str, agent_home: &Path, format: &OutputFormat) -> Result<()> {
    let today = Local::now().format("%Y-%m-%d").to_string();

    // Resolve server log path: env var > fallback
    let server_log = std::env::var("MIKA_SERVER_LOG_FILE")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/log/mika/server.log"));

    // Per-agent CLI log path (daily rotation)
    let cli_log = agent_home.join("logs").join(format!("mika.log.{today}"));

    let server_log_exists = server_log.exists();
    let cli_log_exists = cli_log.exists();

    let server_log_size = if server_log_exists {
        std::fs::metadata(&server_log).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };
    let cli_log_size = if cli_log_exists {
        std::fs::metadata(&cli_log).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };

    match format {
        OutputFormat::Json => {
            let obj = serde_json::json!({
                "agent": agent_name,
                "server_log": {
                    "path": server_log.to_string_lossy(),
                    "exists": server_log_exists,
                    "size_bytes": server_log_size,
                    "filter_command": format!("jq 'select(.agent_id == \"{agent_name}\")' {}", server_log.display()),
                },
                "cli_log": {
                    "path": cli_log.to_string_lossy(),
                    "exists": cli_log_exists,
                    "size_bytes": cli_log_size,
                    "date": today,
                },
            });
            println!("{}", serde_json::to_string_pretty(&obj)?);
        }
        OutputFormat::Yaml => {
            let obj = serde_json::json!({
                "agent": agent_name,
                "server_log": {
                    "path": server_log.to_string_lossy(),
                    "exists": server_log_exists,
                    "size_bytes": server_log_size,
                    "filter_command": format!("jq 'select(.agent_id == \"{agent_name}\")' {}", server_log.display()),
                },
                "cli_log": {
                    "path": cli_log.to_string_lossy(),
                    "exists": cli_log_exists,
                    "size_bytes": cli_log_size,
                    "date": today,
                },
            });
            print!("{}", serde_yaml::to_string(&obj)?);
        }
        OutputFormat::Text => {
            println!();
            println!("  \u{2726} Log Sinks for agent: {agent_name}");
            println!();
            println!("  Server log (mika-server runtime events):");
            println!("    Path:   {}", server_log.display());
            println!(
                "    Status: {}",
                if server_log_exists {
                    format!("exists ({})", format_size(server_log_size))
                } else {
                    "not found".to_string()
                }
            );
            println!(
                "    Filter: jq 'select(.agent_id == \"{agent_name}\")' {}",
                server_log.display()
            );
            println!();
            println!("  Per-agent CLI log (mika ask/chat invocations):");
            println!("    Path:   {}", cli_log.display());
            println!(
                "    Status: {}",
                if cli_log_exists {
                    format!("exists ({})", format_size(cli_log_size))
                } else {
                    "not found (no CLI invocations today)".to_string()
                }
            );
            println!();
            println!("  Tip: For server-mode runtime events (skill execution, task engine,");
            println!("  callback lifecycle), always use the server log filtered by agent_id.");
            println!("  The per-agent CLI log only contains events from discrete CLI calls.");
            println!();
        }
    }

    Ok(())
}

fn format_size(bytes: u64) -> String {
    if bytes > 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes > 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else if bytes > 1_000 {
        format!("{:.1} KB", bytes as f64 / 1_000.0)
    } else {
        format!("{bytes} B")
    }
}

// ── Activity subcommand ──────────────────────────────────────────────

const KNOWN_SURFACES: &[&str] = &["messages", "llm_calls", "tool_calls", "tasks", "server_log"];

/// Unified event type for chronological interleaving of DB surfaces.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
enum LogEvent {
    #[serde(rename = "message")]
    Message {
        timestamp: String,
        session_id: String,
        trace_id: Option<String>,
        role: String,
        content: String,
    },
    #[serde(rename = "llm_call")]
    LlmCall {
        timestamp: String,
        session_id: String,
        trace_id: Option<String>,
        provider: String,
        model: String,
        status: String,
        input_tokens: u64,
        output_tokens: u64,
        latency_ms: u64,
        error_message: Option<String>,
    },
    #[serde(rename = "tool_call")]
    ToolCall {
        timestamp: String,
        session_id: String,
        trace_id: Option<String>,
        tool_name: String,
        success: bool,
        latency_ms: u64,
        error_message: Option<String>,
    },
    #[serde(rename = "task")]
    Task {
        timestamp: String,
        session_id: Option<String>,
        trace_id: Option<String>,
        task_id: String,
        status: String,
        label: Option<String>,
        action_type: Option<String>,
    },
}

impl LogEvent {
    fn timestamp(&self) -> &str {
        match self {
            LogEvent::Message { timestamp, .. }
            | LogEvent::LlmCall { timestamp, .. }
            | LogEvent::ToolCall { timestamp, .. }
            | LogEvent::Task { timestamp, .. } => timestamp,
        }
    }

    /// Check if any searchable text field contains the pattern (case-insensitive).
    fn matches_grep(&self, pattern: &str) -> bool {
        let lower = pattern.to_lowercase();
        match self {
            LogEvent::Message { role, content, .. } => {
                role.to_lowercase().contains(&lower) || content.to_lowercase().contains(&lower)
            }
            LogEvent::LlmCall {
                provider,
                model,
                status,
                error_message,
                ..
            } => {
                provider.to_lowercase().contains(&lower)
                    || model.to_lowercase().contains(&lower)
                    || status.to_lowercase().contains(&lower)
                    || error_message
                        .as_deref()
                        .is_some_and(|e| e.to_lowercase().contains(&lower))
            }
            LogEvent::ToolCall {
                tool_name,
                error_message,
                ..
            } => {
                tool_name.to_lowercase().contains(&lower)
                    || error_message
                        .as_deref()
                        .is_some_and(|e| e.to_lowercase().contains(&lower))
            }
            LogEvent::Task {
                status,
                label,
                action_type,
                ..
            } => {
                status.to_lowercase().contains(&lower)
                    || label
                        .as_deref()
                        .is_some_and(|l| l.to_lowercase().contains(&lower))
                    || action_type
                        .as_deref()
                        .is_some_and(|a| a.to_lowercase().contains(&lower))
            }
        }
    }
}

/// Parse a human-readable time expression into an ISO 8601 timestamp.
///
/// Supported formats:
/// - `Nm` — N minutes ago
/// - `Nh` — N hours ago
/// - `Nd` — N days ago
/// - `today` — midnight UTC of the current day
/// - ISO 8601 timestamp — passed through unchanged
pub fn parse_time_expr(input: &str) -> Result<String> {
    let input = input.trim();
    if input.eq_ignore_ascii_case("today") {
        let today = Utc::now().date_naive().and_hms_opt(0, 0, 0).unwrap();
        return Ok(today
            .and_utc()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    }

    // Try Nm, Nh, Nd patterns
    if input.len() >= 2 {
        let (num_part, suffix) = input.split_at(input.len() - 1);
        if let Ok(n) = num_part.parse::<i64>() {
            let duration = match suffix {
                "m" => chrono::Duration::minutes(n),
                "h" => chrono::Duration::hours(n),
                "d" => chrono::Duration::days(n),
                _ => bail!(
                    "Unknown time expression: '{input}'. Use Nm, Nh, Nd, 'today', or ISO 8601."
                ),
            };
            let ts = Utc::now() - duration;
            return Ok(ts.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        }
    }

    // Assume ISO 8601 passthrough if it contains 'T' or starts with a digit and has dashes
    if input.contains('T')
        || (input.starts_with(|c: char| c.is_ascii_digit()) && input.contains('-'))
    {
        return Ok(input.to_string());
    }

    bail!("Unknown time expression: '{input}'. Use Nm, Nh, Nd, 'today', or ISO 8601.")
}

/// Parse `--include` into a validated set of surface names.
fn parse_include_surfaces(include: &str) -> Result<HashSet<String>> {
    let mut surfaces = HashSet::new();
    for s in include.split(',') {
        let s = s.trim();
        if s.is_empty() {
            continue;
        }
        if !KNOWN_SURFACES.contains(&s) {
            bail!(
                "Unknown surface: '{s}'. Valid surfaces: {}",
                KNOWN_SURFACES.join(", ")
            );
        }
        surfaces.insert(s.to_string());
    }
    Ok(surfaces)
}

/// Query cross-surface activity and render as a unified timeline.
pub async fn run_activity(args: &LogsActivityArgs, agent_name: &str) -> Result<()> {
    let ctx = init::init_db_only_for_agent(agent_name)?;
    let db = &ctx.async_db;

    // Parse and validate surfaces
    let surfaces = parse_include_surfaces(&args.include)?;
    if surfaces.contains("server_log") {
        bail!("server_log surface is not yet implemented — see mika#851 U3");
    }

    // Parse time bounds
    let since = parse_time_expr(&args.since)?;
    let until = args
        .until
        .as_deref()
        .map(parse_time_expr)
        .transpose()?
        .unwrap_or_else(|| Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));

    // Resolve --task to session IDs
    let task_session_ids: Option<Vec<String>> = if let Some(ref task_id) = args.task {
        let sessions = db.get_sessions_for_task_tree(task_id).await?;
        if sessions.is_empty() {
            bail!("No sessions found for task '{task_id}'.");
        }
        Some(sessions.into_iter().map(|s| s.id).collect())
    } else {
        None
    };

    let mut events: Vec<LogEvent> = Vec::new();

    // Helper: check if a session_id passes session/task filters
    let session_matches = |session_id: &str| -> bool {
        if let Some(ref prefix) = args.session
            && !session_id.starts_with(prefix)
        {
            return false;
        }
        if let Some(ref sessions) = task_session_ids
            && !sessions.contains(&session_id.to_string())
        {
            return false;
        }
        true
    };

    // Query messages
    if surfaces.contains("messages") {
        let messages = db.get_messages_since(&since).await?;
        for m in messages {
            if m.created_at > until {
                continue;
            }
            if !session_matches(&m.session_id) {
                continue;
            }
            if let Some(ref trace) = args.trace
                && m.trace_id.as_deref() != Some(trace)
            {
                continue;
            }
            events.push(LogEvent::Message {
                timestamp: m.created_at,
                session_id: m.session_id,
                trace_id: m.trace_id,
                role: m.role,
                content: m.content,
            });
        }
    }

    // Query LLM calls
    if surfaces.contains("llm_calls") {
        let filters = mika_agent::db::LlmCallFilters {
            agent_id: Some(db.agent_id().to_string()),
            session_id: task_session_ids.as_ref().and_then(|s| s.first().cloned()),
            trace_id: args.trace.clone(),
            from: Some(since.clone()),
            to: Some(until.clone()),
            ..Default::default()
        };
        let (rows, _total) = db.query_llm_calls(filters, 1, args.limit as u32).await?;
        for r in rows {
            if !session_matches(&r.session_id) {
                continue;
            }
            events.push(LogEvent::LlmCall {
                timestamp: r.created_at,
                session_id: r.session_id,
                trace_id: r.trace_id,
                provider: r.provider,
                model: r.model,
                status: r.status,
                input_tokens: r.input_tokens,
                output_tokens: r.output_tokens,
                latency_ms: r.latency_ms,
                error_message: r.error_message,
            });
        }
    }

    // Query tool calls
    if surfaces.contains("tool_calls") {
        let filters = mika_agent::db::ToolCallFilters {
            agent_id: Some(db.agent_id().to_string()),
            session_id: task_session_ids.as_ref().and_then(|s| s.first().cloned()),
            trace_id: args.trace.clone(),
            from: Some(since.clone()),
            to: Some(until.clone()),
            ..Default::default()
        };
        let (rows, _total) = db.query_tool_calls(filters, 1, args.limit as u32).await?;
        for r in rows {
            if !session_matches(&r.session_id) {
                continue;
            }
            events.push(LogEvent::ToolCall {
                timestamp: r.created_at,
                session_id: r.session_id,
                trace_id: r.trace_id,
                tool_name: r.tool_name,
                success: r.success,
                latency_ms: r.latency_ms,
                error_message: r.error_message,
            });
        }
    }

    // Query tasks
    if surfaces.contains("tasks") {
        let task_filters = mika_agent::db::TaskFilters {
            agent_id: Some(db.agent_id().to_string()),
            from: Some(since.clone()),
            to: Some(until.clone()),
            ..Default::default()
        };
        let (tasks, _total) = db
            .list_tasks_paginated_with_count(task_filters, args.limit as u32, 0)
            .await?;
        for t in tasks {
            if let Some(ref prefix) = args.session
                && !t
                    .created_by_session
                    .as_deref()
                    .is_some_and(|s| s.starts_with(prefix))
            {
                continue;
            }
            if let Some(ref sessions) = task_session_ids
                && !t
                    .created_by_session
                    .as_deref()
                    .is_some_and(|s| sessions.contains(&s.to_string()))
            {
                continue;
            }
            if let Some(ref trace) = args.trace {
                let t_trace = t
                    .execution_trace_id
                    .as_deref()
                    .or(t.created_trace_id.as_deref());
                if t_trace != Some(trace) {
                    continue;
                }
            }

            let task_id = t.reference_url.as_deref().unwrap_or(&t.id);
            events.push(LogEvent::Task {
                timestamp: t.updated_at,
                session_id: t.created_by_session,
                trace_id: t.execution_trace_id.or(t.created_trace_id),
                task_id: task_id.to_string(),
                status: t.status,
                label: Some(t.label),
                action_type: Some(t.action_type),
            });
        }
    }

    // Apply --grep filter
    if let Some(ref pattern) = args.grep {
        events.retain(|e| e.matches_grep(pattern));
    }

    // Sort chronologically
    events.sort_by(|a, b| a.timestamp().cmp(b.timestamp()));

    // Truncate to --limit
    events.truncate(args.limit);

    // Render
    match args.format {
        OutputFormat::Json => render_json(agent_name, &since, &until, &events)?,
        OutputFormat::Yaml => render_yaml(agent_name, &since, &until, &events)?,
        OutputFormat::Text => render_text(agent_name, &since, &until, &events),
    }

    Ok(())
}

fn render_json(agent: &str, since: &str, until: &str, events: &[LogEvent]) -> Result<()> {
    let envelope = serde_json::json!({
        "agent": agent,
        "since": since,
        "until": until,
        "total": events.len(),
        "events": events,
    });
    println!("{}", serde_json::to_string_pretty(&envelope)?);
    Ok(())
}

fn render_yaml(agent: &str, since: &str, until: &str, events: &[LogEvent]) -> Result<()> {
    let envelope = serde_json::json!({
        "agent": agent,
        "since": since,
        "until": until,
        "total": events.len(),
        "events": events,
    });
    print!("{}", serde_yaml::to_string(&envelope)?);
    Ok(())
}

fn render_text(agent: &str, since: &str, until: &str, events: &[LogEvent]) {
    println!();
    println!("  Activity for agent: {agent} (since {since} to {until})");
    println!();

    for event in events {
        println!("  {}", format_event_line(event));
    }

    println!();
    println!("  Total: {} events", events.len());
    println!();
}

/// Format a single event as one line for text output.
fn format_event_line(event: &LogEvent) -> String {
    let time = extract_local_time(event.timestamp());
    match event {
        LogEvent::Message {
            session_id,
            role,
            content,
            ..
        } => {
            let preview = truncate_content(content, 120);
            format!(
                "[{time}] msg.{role:<13} session={} {preview}",
                truncate_id(session_id),
            )
        }
        LogEvent::LlmCall {
            session_id,
            provider,
            model,
            status,
            input_tokens,
            output_tokens,
            latency_ms,
            ..
        } => {
            format!(
                "[{time}] llm.call        session={} provider={provider} model={model} status={status}  in={input_tokens} out={output_tokens}  {latency_ms}ms",
                truncate_id(session_id),
            )
        }
        LogEvent::ToolCall {
            session_id,
            tool_name,
            success,
            latency_ms,
            ..
        } => {
            let status = if *success { "success" } else { "error" };
            format!(
                "[{time}] tool.call       session={} tool={tool_name} status={status}  {latency_ms}ms",
                truncate_id(session_id),
            )
        }
        LogEvent::Task {
            task_id,
            status,
            label,
            action_type,
            ..
        } => {
            let label_str = label.as_deref().unwrap_or("");
            let action_str = action_type
                .as_deref()
                .map(|a| format!(" action={a}"))
                .unwrap_or_default();
            format!(
                "[{time}] task.{status:<10} task={} {label_str}{action_str}",
                truncate_id(task_id),
            )
        }
    }
}

/// Extract HH:MM:SS in local time from an ISO 8601 timestamp.
fn extract_local_time(iso: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(iso)
        .or_else(|_| {
            // Handle timestamps without timezone (assume UTC)
            chrono::NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%SZ")
                .or_else(|_| chrono::NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%S"))
                .map(|n| n.and_utc().fixed_offset())
        })
        .map(|dt| dt.with_timezone(&Local).format("%H:%M:%S").to_string())
        .unwrap_or_else(|_| iso.to_string())
}

fn truncate_id(id: &str) -> &str {
    if id.len() > 8 { &id[..8] } else { id }
}

fn truncate_content(content: &str, max_chars: usize) -> String {
    // Take first line, trim
    let first_line = content.lines().next().unwrap_or("").trim();
    if first_line.chars().count() > max_chars {
        let truncated: String = first_line.chars().take(max_chars).collect();
        format!("{truncated}…")
    } else {
        first_line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_time_expr_minutes() {
        let result = parse_time_expr("30m").unwrap();
        let parsed = chrono::DateTime::parse_from_rfc3339(&result).unwrap();
        let diff = Utc::now() - parsed.with_timezone(&Utc);
        // Should be ~30 minutes ago (allow 5s tolerance)
        assert!(diff.num_seconds() >= 1795 && diff.num_seconds() <= 1805);
    }

    #[test]
    fn test_parse_time_expr_hours() {
        let result = parse_time_expr("2h").unwrap();
        let parsed = chrono::DateTime::parse_from_rfc3339(&result).unwrap();
        let diff = Utc::now() - parsed.with_timezone(&Utc);
        assert!(diff.num_seconds() >= 7195 && diff.num_seconds() <= 7205);
    }

    #[test]
    fn test_parse_time_expr_days() {
        let result = parse_time_expr("1d").unwrap();
        let parsed = chrono::DateTime::parse_from_rfc3339(&result).unwrap();
        let diff = Utc::now() - parsed.with_timezone(&Utc);
        assert!(diff.num_seconds() >= 86395 && diff.num_seconds() <= 86405);
    }

    #[test]
    fn test_parse_time_expr_today() {
        let result = parse_time_expr("today").unwrap();
        let parsed = chrono::DateTime::parse_from_rfc3339(&result).unwrap();
        let today = Utc::now().date_naive();
        assert_eq!(parsed.date_naive(), today);
        assert_eq!(
            parsed.time(),
            chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()
        );
    }

    #[test]
    fn test_parse_time_expr_iso_passthrough() {
        let iso = "2026-06-14T12:00:00Z";
        let result = parse_time_expr(iso).unwrap();
        assert_eq!(result, iso);
    }

    #[test]
    fn test_parse_time_expr_invalid() {
        assert!(parse_time_expr("xyz").is_err());
        assert!(parse_time_expr("").is_err());
    }

    #[test]
    fn test_log_event_sorting() {
        let events = vec![
            LogEvent::ToolCall {
                timestamp: "2026-06-14T12:02:00Z".to_string(),
                session_id: "s1".to_string(),
                trace_id: None,
                tool_name: "run_gh".to_string(),
                success: true,
                latency_ms: 100,
                error_message: None,
            },
            LogEvent::Message {
                timestamp: "2026-06-14T12:00:00Z".to_string(),
                session_id: "s1".to_string(),
                trace_id: None,
                role: "user".to_string(),
                content: "hello".to_string(),
            },
            LogEvent::LlmCall {
                timestamp: "2026-06-14T12:01:00Z".to_string(),
                session_id: "s1".to_string(),
                trace_id: None,
                provider: "anthropic".to_string(),
                model: "claude-sonnet-4-6".to_string(),
                status: "success".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                latency_ms: 500,
                error_message: None,
            },
            LogEvent::Task {
                timestamp: "2026-06-14T12:03:00Z".to_string(),
                session_id: None,
                trace_id: None,
                task_id: "t1".to_string(),
                status: "completed".to_string(),
                label: Some("fix bug".to_string()),
                action_type: Some("none".to_string()),
            },
        ];

        let mut sorted = events;
        sorted.sort_by(|a, b| a.timestamp().cmp(b.timestamp()));

        assert!(matches!(sorted[0], LogEvent::Message { .. }));
        assert!(matches!(sorted[1], LogEvent::LlmCall { .. }));
        assert!(matches!(sorted[2], LogEvent::ToolCall { .. }));
        assert!(matches!(sorted[3], LogEvent::Task { .. }));
    }

    #[test]
    fn test_text_format_message() {
        let event = LogEvent::Message {
            timestamp: "2026-06-14T12:00:00Z".to_string(),
            session_id: "abc12345-long-id".to_string(),
            trace_id: None,
            role: "user".to_string(),
            content: "hello world".to_string(),
        };
        let line = format_event_line(&event);
        assert!(line.contains("msg.user"));
        assert!(line.contains("session=abc12345"));
        assert!(line.contains("hello world"));
    }

    #[test]
    fn test_text_format_llm_call() {
        let event = LogEvent::LlmCall {
            timestamp: "2026-06-14T12:01:00Z".to_string(),
            session_id: "abc12345-x".to_string(),
            trace_id: None,
            provider: "anthropic".to_string(),
            model: "sonnet".to_string(),
            status: "success".to_string(),
            input_tokens: 100,
            output_tokens: 50,
            latency_ms: 500,
            error_message: None,
        };
        let line = format_event_line(&event);
        assert!(line.contains("llm.call"));
        assert!(line.contains("provider=anthropic"));
        assert!(line.contains("in=100"));
        assert!(line.contains("out=50"));
    }

    #[test]
    fn test_text_format_tool_call() {
        let event = LogEvent::ToolCall {
            timestamp: "2026-06-14T12:02:00Z".to_string(),
            session_id: "abc12345-x".to_string(),
            trace_id: None,
            tool_name: "run_gh".to_string(),
            success: false,
            latency_ms: 200,
            error_message: Some("failed".to_string()),
        };
        let line = format_event_line(&event);
        assert!(line.contains("tool.call"));
        assert!(line.contains("tool=run_gh"));
        assert!(line.contains("status=error"));
    }

    #[test]
    fn test_text_format_task() {
        let event = LogEvent::Task {
            timestamp: "2026-06-14T12:03:00Z".to_string(),
            session_id: None,
            trace_id: None,
            task_id: "task-123456789".to_string(),
            status: "completed".to_string(),
            label: Some("fix bug".to_string()),
            action_type: Some("none".to_string()),
        };
        let line = format_event_line(&event);
        assert!(line.contains("task.completed"));
        assert!(line.contains("task=task-123"));
        assert!(line.contains("fix bug"));
        assert!(line.contains("action=none"));
    }

    #[test]
    fn test_include_parse_valid() {
        let surfaces = parse_include_surfaces("messages,llm_calls,tool_calls").unwrap();
        assert_eq!(surfaces.len(), 3);
        assert!(surfaces.contains("messages"));
        assert!(surfaces.contains("llm_calls"));
        assert!(surfaces.contains("tool_calls"));
    }

    #[test]
    fn test_include_parse_all_defaults() {
        let surfaces = parse_include_surfaces("messages,llm_calls,tool_calls,tasks").unwrap();
        assert_eq!(surfaces.len(), 4);
    }

    #[test]
    fn test_include_server_log_errors() {
        // server_log parses fine but run_activity should reject it — we test the parse here
        let surfaces = parse_include_surfaces("server_log").unwrap();
        assert!(surfaces.contains("server_log"));
    }

    #[test]
    fn test_include_unknown_surface_errors() {
        let result = parse_include_surfaces("messages,unknown_surface");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Unknown surface"));
        assert!(err_msg.contains("unknown_surface"));
    }

    #[test]
    fn test_grep_matches() {
        let event = LogEvent::Message {
            timestamp: "2026-06-14T12:00:00Z".to_string(),
            session_id: "s1".to_string(),
            trace_id: None,
            role: "user".to_string(),
            content: "hello world".to_string(),
        };
        assert!(event.matches_grep("hello"));
        assert!(event.matches_grep("HELLO")); // case-insensitive
        assert!(!event.matches_grep("goodbye"));
    }

    #[test]
    fn test_truncate_content() {
        assert_eq!(truncate_content("short", 120), "short");
        let long = "a".repeat(200);
        let truncated = truncate_content(&long, 120);
        assert!(truncated.chars().count() == 121); // 120 chars + "…"
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn test_truncate_id() {
        assert_eq!(truncate_id("abc12345-long-uuid"), "abc12345");
        assert_eq!(truncate_id("short"), "short");
    }

    #[test]
    fn test_extract_local_time() {
        // This test is timezone-dependent but should at least produce HH:MM:SS format
        let result = extract_local_time("2026-06-14T12:30:45Z");
        assert_eq!(result.len(), 8); // HH:MM:SS
        assert!(result.contains(':'));
    }
}

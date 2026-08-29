use anyhow::{Context, Result};
use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{ForcePromoteResult, Task, format_ts};
use serde_json::{Value, json};

use crate::cli::{OutputFormat, TaskArgs, TaskCommand};
use crate::init;

const VALID_DISPATCH_CLASSES: &[&str] = &["implement", "groom"];

/// Resolve a potentially truncated task ID to its full UUID.
/// Returns `Some(full_id)` on exact or unique prefix match, `None` on no match.
/// Exits with code 1 on ambiguous prefix (multiple matches).
async fn resolve_task_id(db: &AsyncDatabase, input: &str) -> Result<Option<String>> {
    // Try exact match first (fast path, no regression)
    if db.get_task(input).await?.is_some() {
        return Ok(Some(input.to_string()));
    }

    // Minimum prefix length guard
    if input.len() < 4 {
        return Ok(None);
    }

    // Prefix expansion
    let matches = db.resolve_task_id_by_prefix(input).await?;
    match matches.len() {
        0 => Ok(None),
        1 => Ok(Some(matches.into_iter().next().unwrap())),
        _ => {
            eprintln!("\n  Ambiguous task ID prefix '{input}'. Matches:");
            for id in &matches {
                eprintln!("    {id}");
            }
            eprintln!();
            std::process::exit(1);
        }
    }
}

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
                OutputFormat::Yaml => {
                    let json_tasks: Vec<Value> = tasks.iter().map(task_to_json).collect();
                    print!("{}", serde_yaml::to_string(&json_tasks)?);
                }
            }
        }
        Some(TaskCommand::Get { id, format }) => {
            let resolved_id = match resolve_task_id(db, &id).await? {
                Some(id) => id,
                None => {
                    println!("\n  Task {id} not found.\n");
                    return Ok(());
                }
            };
            let task = db.get_task(&resolved_id).await?;
            match task {
                Some(t) => match format {
                    OutputFormat::Text => print_task_detail(&t),
                    OutputFormat::Json => {
                        println!("{}", serde_json::to_string_pretty(&task_to_json(&t))?);
                    }
                    OutputFormat::Yaml => {
                        print!("{}", serde_yaml::to_string(&task_to_json(&t))?);
                    }
                },
                None => {
                    println!("\n  Task {id} not found.\n");
                }
            }
        }
        Some(TaskCommand::Cancel { id }) => {
            let resolved_id = match resolve_task_id(db, &id).await? {
                Some(id) => id,
                None => {
                    println!("\n  Task {id} not found or already completed.\n");
                    return Ok(());
                }
            };
            match mika_agent::task_engine::process_kill::cancel_task_and_kill(db, &resolved_id)
                .await?
            {
                Some(outcome) => match (outcome.process_killed, outcome.pid) {
                    (Some(true), Some(pid)) => {
                        println!(
                            "\n  Cancelled task {resolved_id} (\"{}\") — process (PID {pid}) terminated.\n",
                            outcome.label
                        );
                    }
                    (Some(false), Some(pid)) => {
                        println!(
                            "\n  Cancelled task {resolved_id} (\"{}\") — warning: process (PID {pid}) may still be running.\n",
                            outcome.label
                        );
                    }
                    _ => {
                        println!(
                            "\n  Cancelled task {resolved_id} (\"{}\").\n",
                            outcome.label
                        );
                    }
                },
                None => {
                    println!("\n  Task {id} not found or already completed.\n");
                }
            }
        }
        Some(TaskCommand::Stuck { format }) => {
            let grace = mika_agent::task_engine::stuck_pending_reaper_grace_secs();
            let stuck = db.find_orphaned_pending_issue_tasks(grace).await?;

            match format {
                OutputFormat::Text => {
                    if stuck.is_empty() {
                        println!(
                            "\n  No stuck pending tasks (grace {} min). Every `ready` issue is \
                             either working or queued.\n",
                            grace / 60
                        );
                    } else {
                        println!(
                            "\n  Stuck pending tasks ({}, grace {} min):",
                            stuck.len(),
                            grace / 60
                        );
                        for s in &stuck {
                            println!(
                                "    {}: {} — pending {} min, {} repair(s) attempted",
                                &s.id[..12.min(s.id.len())],
                                s.reference_url,
                                s.age_seconds / 60,
                                s.rearm_count
                            );
                        }
                        println!();
                    }
                }
                OutputFormat::Json => {
                    let rows: Vec<Value> = stuck.iter().map(stuck_task_to_json).collect();
                    println!("{}", serde_json::to_string_pretty(&rows)?);
                }
                OutputFormat::Yaml => {
                    let rows: Vec<Value> = stuck.iter().map(stuck_task_to_json).collect();
                    print!("{}", serde_yaml::to_string(&rows)?);
                }
            }
        }
        Some(TaskCommand::PromoteDeferred { class, r#override }) => {
            if !VALID_DISPATCH_CLASSES.contains(&class.as_str()) {
                eprintln!(
                    "\n  Invalid dispatch class '{class}'. Must be one of: {}\n",
                    VALID_DISPATCH_CLASSES.join(", ")
                );
                std::process::exit(1);
            }

            match db.force_promote_deferred_for_class(&class).await? {
                ForcePromoteResult::Promoted { task_id } => {
                    db.log_audit_event(
                        "cli",
                        "deferred_dispatch_force_promote_succeeded",
                        &format!("dispatch_class:{class}"),
                        None,
                        Some(&format!("promoted:{task_id}")),
                        None,
                        None,
                    )
                    .await?;
                    println!(
                        "\n  Promoted deferred wrapper for class '{class}'. Task ID: {task_id}\n"
                    );
                }
                ForcePromoteResult::RejectedSlotBusy { blocking_label } => {
                    if r#override {
                        // Override path: cancel the blocker, then retry promotion.
                        let blocker_id = db.find_active_callback_for_class(&class).await?;
                        let Some(blocker_id) = blocker_id else {
                            eprintln!(
                                "\n  Slot reported busy but could not find the blocker task. Try again.\n"
                            );
                            std::process::exit(1);
                        };

                        // Cancel the blocker via the shared cancel+kill path.
                        match mika_agent::task_engine::process_kill::cancel_task_and_kill(
                            db,
                            &blocker_id,
                        )
                        .await?
                        {
                            Some(outcome) => {
                                let kill_msg = match (outcome.process_killed, outcome.pid) {
                                    (Some(true), Some(pid)) => {
                                        format!(" Process (PID {pid}) terminated.")
                                    }
                                    (Some(false), Some(pid)) => {
                                        format!(
                                            " Warning: process (PID {pid}) may still be running."
                                        )
                                    }
                                    _ => String::new(),
                                };
                                println!(
                                    "\n  Override: cancelled blocker {blocker_id} (\"{}\").{kill_msg}",
                                    outcome.label
                                );
                            }
                            None => {
                                eprintln!(
                                    "\n  Override: blocker {blocker_id} not found or already completed.\n"
                                );
                                std::process::exit(1);
                            }
                        }

                        // Emit override audit event.
                        db.log_audit_event(
                            "cli",
                            "deferred_dispatch_force_promote_override",
                            &format!("dispatch_class:{class}"),
                            Some(&format!("cancelled:{blocker_id}")),
                            Some("override"),
                            None,
                            None,
                        )
                        .await?;

                        // Retry promotion after cancel.
                        match db.force_promote_deferred_for_class(&class).await? {
                            ForcePromoteResult::Promoted { task_id } => {
                                db.log_audit_event(
                                    "cli",
                                    "deferred_dispatch_force_promote_succeeded",
                                    &format!("dispatch_class:{class}"),
                                    None,
                                    Some(&format!("promoted:{task_id}")),
                                    None,
                                    None,
                                )
                                .await?;
                                println!(
                                    "  Promoted deferred wrapper for class '{class}'. Task ID: {task_id}\n"
                                );
                            }
                            ForcePromoteResult::NoPendingWrapper => {
                                eprintln!(
                                    "  No pending deferred wrapper for class '{class}' after override.\n"
                                );
                                std::process::exit(1);
                            }
                            ForcePromoteResult::RejectedSlotBusy { blocking_label } => {
                                eprintln!(
                                    "  Slot still busy after override (blocker: '{blocking_label}'). Try again.\n"
                                );
                                std::process::exit(1);
                            }
                        }
                    } else {
                        eprintln!(
                            "\n  Cannot promote: dispatch slot for class '{class}' is occupied by '{blocking_label}'."
                        );
                        eprintln!("  Use --override to cancel the blocker and force promotion.\n");
                        std::process::exit(1);
                    }
                }
                ForcePromoteResult::NoPendingWrapper => {
                    eprintln!("\n  No pending deferred wrapper for class '{class}'.\n");
                    std::process::exit(1);
                }
            }
        }
        Some(TaskCommand::Stream { url }) => {
            stream_task_events(url).await?;
        }
    }

    Ok(())
}

/// mika#1758 stub consumer: open the mika-spirit task-events SSE stream and
/// print each `TaskEventFrame` JSON payload to stdout (one line per frame).
///
/// Diagnostic — not integrated into the TUI. mika#1727 handles TUI consumption.
async fn stream_task_events(url_override: Option<String>) -> Result<()> {
    use std::io::Write;

    let base_url = url_override.unwrap_or_else(crate::commands::dashboard::spirit_url);
    let stream_url = format!(
        "{}/api/v1/dashboard/tasks/stream",
        base_url.trim_end_matches('/')
    );
    let token = crate::commands::dashboard::auth_token()?;

    eprintln!("mika#1758 task-event stream: connecting to {stream_url}");
    let client = reqwest::Client::new();
    let mut resp = client
        .get(&stream_url)
        .header("authorization", format!("Bearer {token}"))
        .header("accept", "text/event-stream")
        .send()
        .await
        .with_context(|| format!("failed to open SSE stream at {stream_url}"))?;

    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!(
            "mika-spirit returned {} for {}: {}",
            status,
            stream_url,
            resp.text().await.unwrap_or_default()
        );
    }
    eprintln!("mika#1758 task-event stream: connected. Streaming frames (Ctrl-C to stop).");

    // Minimal SSE frame parser: buffer bytes, split on double-newline event
    // separator, extract `data: <payload>` lines. Using `Response::chunk()`
    // avoids pulling the `futures-util` `Stream` extension (workspace dep is
    // `default-features = false`).
    let mut buffer: Vec<u8> = Vec::with_capacity(4096);
    let stdout = std::io::stdout();

    while let Some(chunk) = resp.chunk().await.context("SSE stream read failed")? {
        buffer.extend_from_slice(&chunk);

        // Split on SSE event separator (`\r\n\r\n` / `\n\n` / `\r\r`, spec
        // § 9.2). Keep the trailing partial event in the buffer for the
        // next iteration.
        while let Some((sep_idx, sep_len)) = find_event_separator(&buffer) {
            let event_bytes: Vec<u8> = buffer.drain(..sep_idx + sep_len).collect();
            // Drop the trailing separator; parse the remainder line-by-line.
            let event_str = String::from_utf8_lossy(&event_bytes[..event_bytes.len() - sep_len]);
            for line in event_str.split(['\n', '\r']) {
                if let Some(data) = line.strip_prefix("data:") {
                    let data = data.trim_start();
                    if data.is_empty() {
                        continue;
                    }
                    let mut handle = stdout.lock();
                    writeln!(handle, "{data}").ok();
                    handle.flush().ok();
                }
            }
        }
    }
    Ok(())
}

/// Locate the first SSE event separator in the buffer, and return
/// `(start_index, separator_length)`.
///
/// Per the SSE spec (WHATWG § Server-sent events), the field-block terminator
/// is either `\r\n\r\n` (4 bytes), `\n\n` (2 bytes), or `\r\r` (2 bytes).
/// axum's `Sse` currently emits `\n\n`, but any reverse proxy or TLS
/// terminator on the path may normalise line endings — matching only `\n\n`
/// would make the stream appear to hang indefinitely on those deployments.
fn find_event_separator(buf: &[u8]) -> Option<(usize, usize)> {
    // Check the 4-byte separator first so a `\r\n\r\n` is not matched as an
    // earlier `\n\n` at offset+1.
    if let Some(idx) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
        return Some((idx, 4));
    }
    if let Some(idx) = buf.windows(2).position(|w| w == b"\n\n") {
        return Some((idx, 2));
    }
    if let Some(idx) = buf.windows(2).position(|w| w == b"\r\r") {
        return Some((idx, 2));
    }
    None
}

fn print_task_summary(t: &Task) {
    let short_id = &t.id[..12.min(t.id.len())];
    let when = t
        .next_fire_at
        .as_ref()
        .map(|s| format_ts(s))
        .unwrap_or_else(|| t.trigger_type.clone());
    // For callback tasks, annotate executing vs queued (#1057)
    let status_info = if t.trigger_type == "callback" {
        if let Some(pid) = t.process_id {
            format!(" [executing, PID {pid}]")
        } else if t.status == "pending" || t.status == "in_progress" {
            " [queued]".to_string()
        } else {
            String::new()
        }
    } else {
        t.process_id
            .map(|pid| format!(" [PID {pid}]"))
            .unwrap_or_default()
    };
    println!(
        "    {}: [{}] [{}] \"{}\" ({}){status_info}",
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

/// Machine-readable shape for `mika tasks stuck` (mika#2045). Kept flat so a
/// watcher can count rows and read `age_seconds` without walking a nested task.
fn stuck_task_to_json(t: &mika_agent::db::OrphanedPendingTask) -> Value {
    json!({
        "id": t.id,
        "reference_url": t.reference_url,
        "created_at": t.created_at,
        "age_seconds": t.age_seconds,
        "rearm_count": t.rearm_count,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use mika_agent::db::OrphanedPendingTask;

    // -- `mika tasks stuck` probe shape (mika#2045) --

    fn stuck_row(issue: u32, age_seconds: i64, rearm_count: i64) -> OrphanedPendingTask {
        OrphanedPendingTask {
            id: format!("ad399d69-0000-0000-0000-{issue:012}"),
            reference_url: format!("https://github.com/senara-solutions/mika/issues/{issue}"),
            created_at: "2026-08-29T09:12:22Z".to_string(),
            age_seconds,
            rearm_count,
            dispatch_class: "implement".to_string(),
        }
    }

    /// A watcher counts rows and reads `age_seconds` — both must be present and
    /// flat, with no nested task to walk.
    #[test]
    fn test_stuck_task_to_json_shape() {
        let row = stuck_row(2013, 2280, 1);
        let json = stuck_task_to_json(&row);

        assert_eq!(
            json["reference_url"],
            "https://github.com/senara-solutions/mika/issues/2013"
        );
        assert_eq!(json["age_seconds"], 2280);
        assert_eq!(json["rearm_count"], 1);
        assert_eq!(json["created_at"], "2026-08-29T09:12:22Z");
        assert!(json["id"].as_str().unwrap().starts_with("ad399d69"));
    }

    /// Anti-vacuity for the probe: with nothing stuck it must render an empty
    /// array, not a placeholder row. A probe that always says something is a
    /// probe nobody reads.
    #[test]
    fn test_stuck_probe_renders_empty_array_when_nothing_is_stuck() {
        let rows: Vec<Value> = Vec::<OrphanedPendingTask>::new()
            .iter()
            .map(stuck_task_to_json)
            .collect();
        assert_eq!(serde_json::to_string(&rows).unwrap(), "[]");
    }

    #[test]
    fn test_stuck_probe_renders_one_row_per_issue() {
        let rows: Vec<Value> = [stuck_row(2013, 2280, 0), stuck_row(1887, 2340, 2)]
            .iter()
            .map(stuck_task_to_json)
            .collect();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1]["rearm_count"], 2);
        assert_ne!(rows[0]["reference_url"], rows[1]["reference_url"]);
    }

    fn make_task(id: &str, label: &str, status: &str) -> Task {
        Task {
            id: id.to_string(),
            agent_id: "test-agent".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: label.to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: Some("2026-05-06T10:00:00Z".to_string()),
            timeout_at: None,
            action_type: "send_message".to_string(),
            action_config: "{}".to_string(),
            status: status.to_string(),
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
    fn test_task_to_json_happy_get() {
        let task = make_task("abc123def456", "Deploy service", "pending");
        let json = task_to_json(&task);

        assert_eq!(json["id"], "abc123def456");
        assert_eq!(json["label"], "Deploy service");
        assert_eq!(json["status"], "pending");
        assert_eq!(json["type"], "issue");
        assert_eq!(json["trigger_type"], "manual");
        assert_eq!(json["action_type"], "send_message");
        assert_eq!(json["agent_id"], "test-agent");
        assert_eq!(json["next_fire_at"], "2026-05-06T10:00:00Z");
    }

    #[test]
    fn test_task_to_json_not_found() {
        // When a task is not found, the handler prints a message.
        // Verify that task_to_json correctly serializes null optional fields.
        let task = make_task("missing-id-0000", "Orphan task", "in_progress");
        let json = task_to_json(&task);

        assert_eq!(json["id"], "missing-id-0000");
        assert!(json["cron_expr"].is_null());
        assert!(json["fired_at"].is_null());
        assert!(json["completed_at"].is_null());
        assert!(json["parent_task_id"].is_null());
        assert!(json["reference_url"].is_null());
        assert!(json["result"].is_null());
        assert!(json["metadata"].is_null());
    }

    #[test]
    fn test_task_to_json_list_serialization() {
        let tasks = [
            make_task("task-001-aaaa", "First task", "pending"),
            make_task("task-002-bbbb", "Second task", "in_progress"),
        ];
        let json_tasks: Vec<Value> = tasks.iter().map(task_to_json).collect();
        let output = serde_json::to_string_pretty(&json_tasks).unwrap();

        assert!(output.contains("\"First task\""));
        assert!(output.contains("\"Second task\""));
        assert!(output.contains("\"pending\""));
        assert!(output.contains("\"in_progress\""));
    }

    #[test]
    fn test_task_to_json_empty_list() {
        let tasks: Vec<Task> = vec![];
        let json_tasks: Vec<Value> = tasks.iter().map(task_to_json).collect();
        let output = serde_json::to_string_pretty(&json_tasks).unwrap();

        assert_eq!(output, "[]");
    }
}

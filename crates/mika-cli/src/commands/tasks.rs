use anyhow::{Context, Result};
use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{ForcePromoteResult, Task, format_ts};
use mika_agent::task_engine::engine::{HANDLER_FAILURE_METADATA_KEY, HandlerFailure};
use mika_common::config::Settings;
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
                            let liveness = probe_pilot_liveness(db, &ctx.settings, t).await;
                            print_task_summary(t, &liveness);
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
                    OutputFormat::Text => {
                        let liveness = probe_pilot_liveness(db, &ctx.settings, &t).await;
                        print_task_detail(&t, &liveness);
                    }
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
        Some(TaskCommand::Cancel { id, yes }) => {
            let resolved_id = match resolve_task_id(db, &id).await? {
                Some(id) => id,
                None => {
                    println!("\n  Task {id} not found or already completed.\n");
                    return Ok(());
                }
            };

            // mika#2335 F2c — the gesture that caused the 2026-09-15 incident.
            // Warn and ask; do NOT refuse: cancelling a live pilot is sometimes
            // exactly what you want. What must never happen again is doing it
            // without being told there is one.
            if let Some(task) = db.get_task(&resolved_id).await?
                && let PilotLiveness::Alive {
                    pid,
                    child_id,
                    idle_secs,
                } = probe_pilot_liveness(db, &ctx.settings, &task).await
            {
                let idle = match idle_secs {
                    Some(s) => format!("its session log was written {s}s ago"),
                    None => "its session log is not readable from here".to_string(),
                };
                let child = child_id
                    .as_ref()
                    .map(|c| format!(" (dispatch child {})", short(c)))
                    .unwrap_or_default();
                eprintln!(
                    "\n  ⚠  A pilot is RUNNING under this task{child}: PID {pid}, {idle}.\n  \
                     Cancelling kills it. A task row reading `pending` / `fired_at=null` is \
                     not evidence\n  that a dispatch is inert — that misreading cost 30 \
                     minutes of work and left three files\n  dirty under a second pilot on \
                     2026-09-15.\n"
                );
                if !yes {
                    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                        eprintln!(
                            "  Refusing to kill a live pilot without confirmation. \
                             Re-run with --yes if that is what you mean.\n"
                        );
                        std::process::exit(1);
                    }
                    let proceed = dialoguer::Confirm::new()
                        .with_prompt(format!("  Kill the running pilot (PID {pid})?"))
                        .default(false)
                        .interact()
                        .unwrap_or(false);
                    if !proceed {
                        println!("\n  Cancelled nothing — the pilot keeps running.\n");
                        return Ok(());
                    }
                }
            }

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
        Some(TaskCommand::Rearm { label }) => {
            match mika_agent::task_engine::rearm_recurring_task(db, &label).await {
                Ok(outcome) => {
                    println!(
                        "\n  Re-armed recurring task \"{}\" (trigger {}, cron {}).\n  \
                         Lifted the zombie veto on {} dead row(s); most recent: {} ({}).\n  \
                         Recorded in audit_events as recurring_operator_rearm.\n",
                        outcome.label,
                        outcome.trigger,
                        outcome.cron_expr,
                        outcome.rows_marked,
                        outcome.dead_task_id,
                        outcome.dead_status,
                    );
                }
                Err(e) => {
                    eprintln!("\n  Refused: {e}\n");
                    std::process::exit(1);
                }
            }
        }
        Some(TaskCommand::Stuck { format }) => {
            let grace = mika_agent::task_engine::stuck_pending_reaper_grace_secs();
            // The probe must see exactly the population the reaper acts on
            // (mika#2181) — a probe with its own predicate is a probe that lies.
            let promoted_liveness = mika_agent::task_engine::promoted_wrapper_liveness_secs();
            let stuck = db
                .find_orphaned_pending_issue_tasks(grace, promoted_liveness)
                .await?;

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

            match db
                .force_promote_deferred_for_class(
                    &class,
                    mika_agent::skills::executor::max_concurrent_for_class(&class),
                )
                .await?
            {
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
                        match db
                            .force_promote_deferred_for_class(
                                &class,
                                mika_agent::skills::executor::max_concurrent_for_class(&class),
                            )
                            .await?
                        {
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

/// What a task's dispatch actually looks like **on the machine** (mika#2335).
///
/// The founding incident is a reading error the tool made possible: the row of
/// the live pilot `590a06c0` carried `status=pending, fired_at=null` — the
/// shape of "inert, never launched" — while its PID was active and its log had
/// just been written to. An operator read the row, concluded orphan, and
/// cancelled a pilot that had been working for 38 minutes. `fired_at` is now
/// stamped (F2a), but the authoritative signal of life was never the row: it is
/// the PID and the log mtime. This type is that signal, read where the operator
/// looks instead of left as a gesture they must remember to perform.
#[derive(Debug)]
enum PilotLiveness {
    /// No PID-carrying dispatch to speak of.
    NoPilot,
    /// A PID is recorded and the process at it is the instance we spawned.
    Alive {
        pid: i64,
        child_id: Option<String>,
        /// Seconds since the claude-pilot session log was last written.
        /// `None` when the log is absent or unreadable — which says nothing
        /// about the pilot, only about the log.
        idle_secs: Option<u64>,
    },
    /// A PID is recorded and nothing is running under it.
    Dead { pid: i64, child_id: Option<String> },
    /// A PID is recorded but carries no readable `process_start_time`, so a
    /// recycled PID cannot be told from the pilot. We claim neither.
    Unverifiable { pid: i64, child_id: Option<String> },
    /// The lookup failed. Not evidence of anything.
    Unknown,
}

impl PilotLiveness {
    /// How much this answer outranks another when a parent carries several
    /// PID-bearing children. Higher wins. `Unverifiable` sits **above** `Dead`
    /// on purpose: a dead child was checked and is gone, an unverifiable one
    /// might be running and we cannot tell — and "might be running" is the
    /// answer an operator about to cancel needs to see.
    fn rank(&self) -> u8 {
        match self {
            PilotLiveness::Alive { .. } => 3,
            PilotLiveness::Unverifiable { .. } => 2,
            PilotLiveness::Dead { .. } => 1,
            PilotLiveness::NoPilot | PilotLiveness::Unknown => 0,
        }
    }
}

fn short(id: &str) -> &str {
    &id[..12.min(id.len())]
}

/// `process_start_time` as the executor writes it into task metadata: a JSON
/// *string*. An integer is accepted too, matching `find_dispatch_children_with_pid`.
fn start_time_from_metadata(metadata: Option<&str>) -> Option<u64> {
    let v: Value = serde_json::from_str(metadata?).ok()?;
    let field = v.get("process_start_time")?;
    match field {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_u64(),
        _ => None,
    }
}

/// Seconds since `<pilot_log_dir>/<task-id>.log` was last written.
///
/// Same derivation as the silent-stall reaper (mika#2277) and the same
/// discipline: an unreadable signal is reported as absent, never as silence.
fn pilot_log_idle_secs(settings: &Settings, log_id: &str) -> Option<u64> {
    let path = settings
        .effective_pilot_log_dir()
        .join(format!("{log_id}.log"));
    mika_agent::task_engine::worktree_activity::seconds_since_file_write(&path)
}

/// Measure the liveness of the pilot behind `t`, whichever of the two rows of a
/// dispatch `t` happens to be.
///
/// A **callback** row carries the pgid itself. A **parent** tracking row never
/// does — its pilot hangs off a `parent_task_id`-linked child, which is exactly
/// the traversal `find_dispatch_children_with_pid` performs (mika#2156). The
/// parent is the row an operator consults, so before mika#2335 the one surface
/// that could have contradicted the misleading status showed nothing at all.
async fn probe_pilot_liveness(db: &AsyncDatabase, settings: &Settings, t: &Task) -> PilotLiveness {
    if t.trigger_type == "callback" {
        let Some(pid) = t.process_id else {
            return PilotLiveness::NoPilot;
        };
        let Some(start_time) = start_time_from_metadata(t.metadata.as_deref()) else {
            return PilotLiveness::Unverifiable {
                pid,
                child_id: None,
            };
        };
        return classify(pid, None, start_time, settings, &t.id);
    }

    let children = match db.find_dispatch_children_with_pid(&t.id).await {
        Ok(c) => c,
        Err(_) => return PilotLiveness::Unknown,
    };

    // A parent can carry several PID-bearing children across retries, so the
    // answers must be ranked rather than overwritten: **alive > unverifiable >
    // dead > none**. A live child is the answer outright. Failing that, an
    // *unverifiable* child outranks a dead one — we checked the dead one and it
    // is gone, whereas the unverifiable one might be running and we cannot tell.
    // Letting a later dead child overwrite an earlier unverifiable one would
    // print "no live pilot" over a genuine unknown, which is the fail-safe
    // inversion this whole ticket exists to remove.
    let mut fallback = PilotLiveness::NoPilot;
    for child in children {
        let candidate = match child.process_start_time {
            Some(start_time) => classify(
                child.process_id,
                Some(child.id.clone()),
                start_time,
                settings,
                &child.id,
            ),
            None => PilotLiveness::Unverifiable {
                pid: child.process_id,
                child_id: Some(child.id),
            },
        };
        if matches!(candidate, PilotLiveness::Alive { .. }) {
            return candidate;
        }
        if candidate.rank() > fallback.rank() {
            fallback = candidate;
        }
    }
    fallback
}

fn classify(
    pid: i64,
    child_id: Option<String>,
    start_time: u64,
    settings: &Settings,
    log_id: &str,
) -> PilotLiveness {
    let Ok(pid_u32) = u32::try_from(pid) else {
        return PilotLiveness::Unverifiable { pid, child_id };
    };
    if mika_agent::task_engine::process_liveness::is_same_process_alive(pid_u32, start_time) {
        PilotLiveness::Alive {
            pid,
            child_id,
            idle_secs: pilot_log_idle_secs(settings, log_id),
        }
    } else {
        PilotLiveness::Dead { pid, child_id }
    }
}

fn print_task_summary(t: &Task, liveness: &PilotLiveness) {
    let short_id = short(&t.id);
    let when = t
        .next_fire_at
        .as_ref()
        .map(|s| format_ts(s))
        .unwrap_or_else(|| t.trigger_type.clone());
    let status_info = summary_annotation(&t.trigger_type, &t.status, liveness);
    println!(
        "    {}: [{}] [{}] \"{}\" ({}){status_info}",
        short_id, t.status, t.action_type, t.label, when
    );
}

/// The bracketed tail of a summary line (#1057, **measured** since mika#2335).
///
/// The `[executing, PID n]` this replaces was inferred from the presence of a
/// PID column, so it stayed on the screen long after the process was gone — and
/// said nothing at all on a parent tracking row, which is the row an operator
/// consults. Pure, so the branch deciding what an operator reads is testable
/// without capturing stdout. Takes the two fields it reads rather than the
/// whole `Task`: `Task` has no `Default`, and a test that had to spell out
/// thirty-two columns to assert on one bracket would not get written.
fn summary_annotation(trigger_type: &str, status: &str, liveness: &PilotLiveness) -> String {
    match liveness {
        PilotLiveness::Alive { pid, .. } => format!(" [pilot alive, PID {pid}]"),
        PilotLiveness::Dead { pid, .. } => format!(" [PID {pid} — no live pilot]"),
        PilotLiveness::Unverifiable { pid, .. } => format!(" [PID {pid} — liveness unverifiable]"),
        PilotLiveness::NoPilot | PilotLiveness::Unknown => {
            if trigger_type == "callback" && (status == "pending" || status == "in_progress") {
                " [queued]".to_string()
            } else {
                String::new()
            }
        }
    }
}

/// The `Dispatch pilot:` line of `mika tasks get` — the measured answer to
/// "is anything actually running under this task?".
fn format_pilot_detail(liveness: &PilotLiveness) -> String {
    let child = |c: &Option<String>| {
        c.as_ref()
            .map(|id| format!(", child {}", short(id)))
            .unwrap_or_default()
    };
    match liveness {
        PilotLiveness::Alive {
            pid,
            child_id,
            idle_secs,
        } => {
            let idle = match idle_secs {
                Some(s) => format!(", last log write {s}s ago"),
                None => ", no readable session log".to_string(),
            };
            format!("alive — PID {pid}{}{idle}", child(child_id))
        }
        PilotLiveness::Dead { pid, child_id } => format!(
            "none — PID {pid} is not running{} (dead or reused)",
            child(child_id)
        ),
        PilotLiveness::Unverifiable { pid, child_id } => format!(
            "unverifiable — PID {pid}{} has no recorded start time, so a \
             recycled PID cannot be ruled out",
            child(child_id)
        ),
        PilotLiveness::NoPilot => "none — no dispatch carrying a PID".to_string(),
        PilotLiveness::Unknown => "unknown — the dispatch-child lookup failed".to_string(),
    }
}

fn print_task_detail(t: &Task, liveness: &PilotLiveness) {
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
    // mika#2335 — the measured answer, not the one inferred from the columns
    // above. `status` and `fired_at` describe bookkeeping; this describes the
    // machine.
    println!("  Dispatch pilot: {}", format_pilot_detail(liveness));
    if let Some(ref v) = t.result {
        let display = match v.char_indices().nth(200) {
            Some((i, _)) => &v[..i],
            None => v,
        };
        println!("  Result:        {display}");
    }
    print_handler_failure(t);
    println!();
}

/// Render `$.handler_failure` when the row carries one (mika#2532 R1).
///
/// The engine writes this whenever a long-running handler exits non-zero —
/// **including when the task is already terminal**, which is the case the
/// ticket was filed for: the handler's EXIT trap delivers its callback, the
/// row goes `completed`, `update_task_failed` matches nothing, and the one
/// string naming the cause used to be dropped on the floor.
///
/// Fail-open on every read: unparseable metadata, an absent key, an unexpected
/// shape — all render nothing. A crash whose record cannot be read is not
/// worth breaking `mika tasks get` over, and the JSON output carries the raw
/// `metadata` for anyone who wants to look closer.
fn print_handler_failure(t: &Task) {
    let Some(failure) = read_handler_failure(t.metadata.as_deref()) else {
        return;
    };

    println!(
        "  Handler failure: {} (captured {})",
        failure.exit,
        format_ts(&failure.captured_at)
    );
    // Absence is a fact, not an empty string: the handler wrote nothing on
    // fd 2. Saying so beats rendering a blank line that reads like a bug.
    match failure.stderr.as_deref() {
        Some(stderr) => {
            println!("  Handler stderr:");
            for line in stderr.lines() {
                println!("    {line}");
            }
        }
        None => println!("  Handler stderr: (the handler wrote nothing)"),
    }
}

/// Extract the engine's failure record from a task's raw `metadata` JSON.
///
/// Both the key and the payload's shape come from the engine
/// ([`HandlerFailure`]), never from literals retyped here: two spellings of one
/// name is the `grooming_marker` class (mika#2158), and this renderer lives in
/// a different crate from the writer.
///
/// Fail-open on every read — unparseable metadata, an absent key, an
/// unexpected shape all yield `None`. A crash whose record cannot be read is
/// not worth breaking `mika tasks get` over, and `--format json` carries the
/// raw `metadata` for anyone who wants to look closer.
fn read_handler_failure(raw: Option<&str>) -> Option<HandlerFailure> {
    let map = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(raw?).ok()?;
    serde_json::from_value(map.get(HANDLER_FAILURE_METADATA_KEY)?.clone()).ok()
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

    // -- pilot liveness surfacing (mika#2335 F2b/F2c) --

    /// The line an operator reads must **name the pilot**, not merely echo the
    /// PID column. The 2026-09-15 cancel happened because every surface agreed
    /// with the row (`pending`, `fired_at=null`) and none contradicted it.
    #[test]
    fn alive_detail_names_the_pid_the_child_and_the_log_age() {
        let line = format_pilot_detail(&PilotLiveness::Alive {
            pid: 890_205,
            child_id: Some("590a06c0-0000-0000-0000-000000000000".to_string()),
            idle_secs: Some(12),
        });
        assert!(line.contains("alive"), "{line}");
        assert!(line.contains("890205"), "le PID doit être nommé — {line}");
        assert!(
            line.contains("590a06c0"),
            "l'enfant doit être nommé — {line}"
        );
        assert!(
            line.contains("12s ago"),
            "l'âge de la dernière écriture est la moitié de la leçon opérateur — {line}"
        );
    }

    /// An unreadable log says something about the log, never about the pilot.
    /// Reporting it as silence is exactly the fail-safe inversion the reaper
    /// family (mika#2277) exists to refuse.
    #[test]
    fn alive_without_a_readable_log_still_reads_alive() {
        let line = format_pilot_detail(&PilotLiveness::Alive {
            pid: 42,
            child_id: None,
            idle_secs: None,
        });
        assert!(line.starts_with("alive"), "{line}");
        assert!(line.contains("no readable session log"), "{line}");
    }

    /// A recorded PID with nothing running under it must NOT read as a pilot —
    /// the failure the old `[executing, PID n]` annotation produced, since it
    /// was inferred from the column rather than measured.
    #[test]
    fn a_dead_pid_never_reads_as_a_running_pilot() {
        let line = format_pilot_detail(&PilotLiveness::Dead {
            pid: 890_205,
            child_id: None,
        });
        assert!(!line.contains("alive"), "{line}");
        assert!(line.contains("not running"), "{line}");
    }

    /// Unverifiable is its own answer, distinct from both "alive" and "dead":
    /// without a start time the pair identifying a process *instance* is
    /// incomplete, and claiming either way is what a recycled PID exploits.
    #[test]
    fn unverifiable_claims_neither_life_nor_death() {
        let line = format_pilot_detail(&PilotLiveness::Unverifiable {
            pid: 7,
            child_id: None,
        });
        assert!(line.contains("unverifiable"), "{line}");
        assert!(!line.contains("alive —"), "{line}");
        assert!(!line.contains("not running"), "{line}");
    }

    /// `process_start_time` is written by the executor as a JSON **string**; an
    /// integer is accepted too, and anything else degrades to `None` rather
    /// than to a wrong number.
    #[test]
    fn start_time_is_read_from_metadata_in_both_shapes() {
        assert_eq!(
            start_time_from_metadata(Some(r#"{"process_start_time":"12345"}"#)),
            Some(12345)
        );
        assert_eq!(
            start_time_from_metadata(Some(r#"{"process_start_time":12345}"#)),
            Some(12345)
        );
        assert_eq!(start_time_from_metadata(Some(r#"{"other":1}"#)), None);
        assert_eq!(start_time_from_metadata(Some("not json")), None);
        assert_eq!(start_time_from_metadata(None), None);
    }

    /// The summary line must never say a pilot is alive about a dead PID — the
    /// exact misreading the old annotation licensed.
    #[test]
    fn summary_annotation_separates_alive_from_dead() {
        assert!(
            summary_annotation(
                "manual",
                "in_progress",
                &PilotLiveness::Alive {
                    pid: 890_205,
                    child_id: None,
                    idle_secs: None,
                },
            )
            .contains("pilot alive, PID 890205")
        );
        let dead = summary_annotation(
            "manual",
            "in_progress",
            &PilotLiveness::Dead {
                pid: 890_205,
                child_id: None,
            },
        );
        assert!(dead.contains("no live pilot"), "{dead}");
        assert!(!dead.contains("alive,"), "{dead}");
    }

    /// A parent with no dispatch child stays silent; a queued callback keeps
    /// its `[queued]` (#1057 behaviour, unchanged). Annotating every row would
    /// make the annotation stop meaning anything.
    #[test]
    fn no_pilot_is_silent_on_a_parent_and_queued_on_a_callback() {
        assert_eq!(
            summary_annotation("manual", "in_progress", &PilotLiveness::NoPilot),
            ""
        );
        assert_eq!(
            summary_annotation("callback", "pending", &PilotLiveness::NoPilot),
            " [queued]"
        );
    }

    /// A parent carrying several PID-bearing children must report the SAFEST
    /// answer, not the last one scanned. `Unverifiable` outranks `Dead`: a dead
    /// child was checked and is gone, an unverifiable one might be running —
    /// and printing "no live pilot" over a genuine unknown is exactly the
    /// fail-safe inversion that cost a working pilot on 2026-09-15.
    #[test]
    fn an_unverifiable_child_outranks_a_dead_one() {
        let dead = PilotLiveness::Dead {
            pid: 1,
            child_id: None,
        };
        let unverifiable = PilotLiveness::Unverifiable {
            pid: 2,
            child_id: None,
        };
        let alive = PilotLiveness::Alive {
            pid: 3,
            child_id: None,
            idle_secs: None,
        };
        assert!(unverifiable.rank() > dead.rank());
        assert!(alive.rank() > unverifiable.rank());
        assert!(dead.rank() > PilotLiveness::NoPilot.rank());
        assert_eq!(
            PilotLiveness::Unknown.rank(),
            PilotLiveness::NoPilot.rank(),
            "ni l'un ni l'autre ne doit jamais écraser une réponse mesurée"
        );
    }

    /// A failed lookup is not evidence of absence, so it must not be dressed as
    /// a verdict in the detail view.
    #[test]
    fn nopilot_and_unknown_are_distinguishable_in_the_detail_view() {
        assert!(format_pilot_detail(&PilotLiveness::NoPilot).contains("no dispatch"));
        assert!(format_pilot_detail(&PilotLiveness::Unknown).contains("unknown"));
    }

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
            dispatcher_source: None,
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

    // -- handler-failure rendering (mika#2532 R1) --

    /// Build the metadata the engine really writes, by going through
    /// `Database::set_task_handler_failure` on an in-memory row.
    ///
    /// The point is the **crossing**: the writer lives in `mika-agent` and this
    /// reader in `mika-cli`. Asserting the reader against a payload this file
    /// hand-rolled would prove only that it can read itself. Going through the
    /// DB is what makes "the operator sees the cause" a fact rather than a
    /// convention.
    fn metadata_after_engine_write(stderr: Option<&str>) -> Option<String> {
        use mika_agent::db::{Database, NewTask};

        let db = Database::open_in_memory().unwrap();
        let id = db
            .create_task(&NewTask {
                agent_id: "mika".to_string(),
                team_run_id: None,
                parent_task_id: None,
                depth: 0,
                label: "long_running:build_mika".to_string(),
                trigger_type: "callback".to_string(),
                cron_expr: None,
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: None,
                timeout_at: None,
                action_type: "resume_agent".to_string(),
                action_config: "{}".to_string(),
                input_context: None,
                created_by_session: None,
                created_trace_id: None,
                reference_url: None,
                source: None,
                metadata: None,
                r#type: None,
                dispatch_class: None,
            })
            .unwrap();
        db.set_task_handler_failure(&id, "Exit code: 1", stderr)
            .unwrap();
        db.get_task(&id, "mika").unwrap().unwrap().metadata
    }

    #[test]
    fn mika2532_the_cli_reads_what_the_engine_wrote() {
        let metadata = metadata_after_engine_write(Some("ERROR: could not cd to /nope/mika"));
        let failure = read_handler_failure(metadata.as_deref())
            .expect("the renderer must read the record the engine persisted");

        assert_eq!(failure.exit, "Exit code: 1");
        assert_eq!(
            failure.stderr.as_deref(),
            Some("ERROR: could not cd to /nope/mika")
        );
        assert!(!failure.captured_at.is_empty());
    }

    /// A mute handler leaves `exit` and no `stderr` key — the renderer must say
    /// so rather than print a blank line that reads like a defect.
    #[test]
    fn mika2532_a_mute_handler_reads_as_an_absence_not_an_empty_string() {
        let metadata = metadata_after_engine_write(None);
        let failure = read_handler_failure(metadata.as_deref()).expect("record present");

        assert_eq!(failure.exit, "Exit code: 1");
        assert!(
            failure.stderr.is_none(),
            "fd 2 stayed mute: the key must be absent, not empty"
        );
    }

    /// Fail-open, four ways. `mika tasks get` must never die on a row whose
    /// metadata it cannot make sense of.
    #[test]
    fn mika2532_an_unreadable_record_renders_nothing_rather_than_failing() {
        assert!(read_handler_failure(None).is_none(), "no metadata at all");
        assert!(
            read_handler_failure(Some("not json")).is_none(),
            "metadata that is not JSON"
        );
        assert!(
            read_handler_failure(Some(r#"{"process_start_time":"123"}"#)).is_none(),
            "valid metadata carrying no failure record — the nominal case"
        );
        assert!(
            read_handler_failure(Some(r#"{"handler_failure":"a string"}"#)).is_none(),
            "the key present but not the expected shape"
        );
    }
}

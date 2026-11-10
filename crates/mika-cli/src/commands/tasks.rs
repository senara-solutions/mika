use anyhow::{Context, Result};
use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{ForcePromoteResult, Task, format_ts};
use mika_agent::live_pilot::TaskPilot;
use mika_agent::server::tasks_stream::TaskEventFrame;
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
                    print!("{}", yaml_serde::to_string(&json_tasks)?);
                }
            }
        }
        Some(TaskCommand::Get { id, format, spirit }) => {
            // `--spirit` sort **avant** le chemin local plutôt que dans un `else` :
            // ré-indenter le corps d'aujourd'hui en ferait des lignes modifiées, là
            // où le critère 3 le veut identique à l'octet près. Cette porte ne
            // résout pas de préfixe — le serveur lit par identifiant exact — et ne
            // sonde pas la vivacité du pilote, qui est locale.
            if spirit {
                let url = super::dashboard::spirit_url();
                let jeton = super::dashboard::auth_token()?;
                let mut sortie = std::io::stdout();
                return get_task_detail_from_spirit_into(&url, &jeton, &id, format, &mut sortie)
                    .await;
            }
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
                        print!("{}", yaml_serde::to_string(&task_to_json(&t))?);
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
            match mika_agent::task_engine::rearm_recurring_task(db, &ctx.home_dir, &label).await {
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
                    print!("{}", yaml_serde::to_string(&rows)?);
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
        Some(TaskCommand::Stream { url, agent }) => {
            stream_task_events(url, agent).await?;
        }
    }

    Ok(())
}

/// mika#1758 stub consumer: open the mika-spirit task-events SSE stream and
/// print each `TaskEventFrame` JSON payload to stdout (one line per frame).
///
/// Diagnostic — not integrated into the TUI. mika#1727 handles TUI consumption.
///
/// Resolving the base URL and the bearer token lives **here**, never under
/// [`TaskEventStream::open`]: `dashboard::auth_token` reads the process
/// environment and fails without it, so a consumer resolving its own
/// credentials could not be driven from a test at all (mika#1727 vol V2b-1b).
async fn stream_task_events(url_override: Option<String>, agent: Option<String>) -> Result<()> {
    let base_url = url_override.unwrap_or_else(crate::commands::dashboard::spirit_url);
    let token = crate::commands::dashboard::auth_token()?;

    eprintln!(
        "mika#1758 task-event stream: connecting to {}",
        stream_url(&base_url)
    );
    stream_task_events_into(&base_url, &token, agent.as_deref(), &mut std::io::stdout()).await
}

/// `mika tasks stream`, with its output sink as a parameter.
///
/// The sink is a parameter because the property to hold is "one line per `data:`
/// payload, **and nothing else**", and that is only measurable by comparing the
/// whole stream to an expected string — which `std::io::stdout()` does not allow
/// from a Rust test without a subprocess. The two `eprintln!` stay on stderr and
/// are therefore absent from the sink: that absence *is* the measurement of
/// "nothing else", not a restatement of it.
///
/// `agent_filter` is `--agent` as [`TaskEventStream::consume`] takes it. The
/// sink gets the **raw** payload either way — the decode travels so the filter
/// can read it, not so the output changes shape — which is why `None` yields the
/// output of before the flag existed, to the byte.
async fn stream_task_events_into<W: std::io::Write>(
    base_url: &str,
    token: &str,
    agent_filter: Option<&str>,
    out: &mut W,
) -> Result<()> {
    let stream = TaskEventStream::open(base_url, token).await?;
    eprintln!("mika#1758 task-event stream: connected. Streaming frames (Ctrl-C to stop).");

    stream
        .consume(agent_filter, |payload, _frame| {
            let _ = writeln!(out, "{payload}");
            let _ = out.flush();
        })
        .await
}

/// The task-events stream URL for a mika-spirit base URL.
fn stream_url(base_url: &str) -> String {
    format!(
        "{}/api/v1/dashboard/tasks/stream",
        base_url.trim_end_matches('/')
    )
}

/// L'agent qu'une trame du fil nomme, quand elle en nomme un.
///
/// Le `match` **énumère les huit variantes**, sans bras `_`, et c'est une garde
/// de structure et non un goût de style : le fil est additif par contrat — « new
/// variants may ship additively » dit le doc de [`TaskEventFrame`], qui nomme
/// déjà mika#1734 et mika#1736. Sous un bras `_ => None`, une variante neuve
/// portant un `agent_id` tomberait **silencieusement** du bon côté du filtre
/// d'un autre agent. Énumérées, elle ne compile plus ici, et le compilateur
/// nomme le site qui doit décider.
///
/// `None` pour `OverflowMarker` et `Unknown` : ces deux-là ne portent aucun
/// agent. Ce que le filtre en fait est la décision de
/// [`TaskEventStream::consume`] — il n'écarte que ce qu'il a **compris** comme
/// étranger.
fn frame_agent_id(frame: &TaskEventFrame) -> Option<&str> {
    match frame {
        TaskEventFrame::TaskCreated { agent_id, .. }
        | TaskEventFrame::TaskClaimed { agent_id, .. }
        | TaskEventFrame::TaskCompleted { agent_id, .. }
        | TaskEventFrame::TaskFailed { agent_id, .. }
        | TaskEventFrame::TaskDelivered { agent_id, .. }
        | TaskEventFrame::TaskCancelled { agent_id, .. } => Some(agent_id),
        TaskEventFrame::OverflowMarker { .. } | TaskEventFrame::Unknown => None,
    }
}

/// La tâche et l'`original_status` qu'une trame **terminale** nomme.
///
/// Jumelle de [`frame_agent_id`] : une lecture de variante, rien d'autre. Elle
/// vit donc ici et non dans `tui/app.rs`, ce qui laisse la TUI sans aucune
/// connaissance du format du fil — la même frontière que « aucun client HTTP
/// dans `tui/app.rs` » trace. La doc d'exhaustivité ci-dessus vaut mot pour mot
/// pour ce `match`-ci : **pas de bras `_ => None`**, parce qu'une variante neuve
/// portant un `task_id` tomberait silencieusement hors du chemin des callbacks.
///
/// **L'`original_status` vient de la variante, et de rien d'autre.**
/// `mark_task_delivered` passe la ligne à `delivered` avant que le
/// `CallbackResult` ne se bâtisse, donc le `status` du détail dirait `delivered`
/// et serait faux. Le miroir **porte** ce champ — les trois formats le rendent —
/// si bien que l'interdit est dit ici et attesté par un test, pas structurel.
pub(crate) fn terminal_callback_frame(frame: &TaskEventFrame) -> Option<(&str, &'static str)> {
    match frame {
        TaskEventFrame::TaskCompleted { task_id, .. } => Some((task_id, "completed")),
        TaskEventFrame::TaskFailed { task_id, .. } => Some((task_id, "failed")),
        TaskEventFrame::TaskCreated { .. }
        | TaskEventFrame::TaskClaimed { .. }
        | TaskEventFrame::TaskDelivered { .. }
        | TaskEventFrame::TaskCancelled { .. }
        | TaskEventFrame::OverflowMarker { .. }
        | TaskEventFrame::Unknown => None,
    }
}

/// Ouvrir le fil des tâches **une fois** et pousser sur un canal les callbacks
/// terminés de `agent_id`, en couples `(task_id, original_status)`.
///
/// Le canal porte exactement ce que la variante sait dire, et rien de plus : le
/// détail se lit du côté du drainage, parce que la fermeture de
/// [`TaskEventStream::consume`] est **synchrone** et qu'aucun des quatre gestes
/// de la livraison — lire le détail, appliquer le prédicat, réclamer, livrer —
/// n'est autre que `async`. Les trames d'un autre agent sont écartées par
/// `consume`, les non-terminales par [`terminal_callback_frame`] : le canal ne
/// reçoit donc que des candidats, jamais le fil brut.
///
/// **Une seule ouverture, aucun recul, aucune reconnexion** (vol V2b-2b-3). Un
/// échec d'ouverture ou une fin de flux arrête la tâche et rien ne la relance.
/// Ce que ça coûte, nommé plutôt que découvert : à partir de cet instant les
/// callbacks de cette conversation n'arrivent plus que par `poll_callback_tasks`,
/// toutes les `POLL_INTERVAL_TICKS`. C'est une dégradation de latence, pas une
/// perte — et c'est pourquoi le poll reste le filet de ce vol.
pub(crate) fn spawn_callback_subscription(
    agent_id: String,
    base_url: String,
    token: String,
) -> tokio::sync::mpsc::UnboundedReceiver<(String, &'static str)> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        let flux = match TaskEventStream::open(&base_url, &token).await {
            Ok(flux) => flux,
            Err(e) => {
                tracing::warn!("task-events subscription failed to open: {e:#}");
                return;
            }
        };
        let issue = flux
            .consume(Some(&agent_id), |_, trame| {
                if let Some((task_id, original_status)) = trame.and_then(terminal_callback_frame) {
                    let _ = tx.send((task_id.to_string(), original_status));
                }
            })
            .await;
        tracing::debug!("task-events subscription ended: {issue:?}");
    });
    rx
}

/// An open task-events stream.
///
/// Opening and reading are two gestures rather than one call, because the
/// callers tell them apart: `mika tasks stream` prints "connected" **between**
/// the two — a connected-but-idle wire must not read as a hung one — and the
/// TUI's subscription (vol V2b-2) backs off on a failed **open** while an ended
/// stream is only a reconnect. A single call would make both the same `Err`.
pub(crate) struct TaskEventStream {
    resp: reqwest::Response,
}

impl TaskEventStream {
    /// Open the stream against `base_url`, authenticating with `token`.
    ///
    /// Both travel by argument and nothing is resolved under here, so a caller
    /// — a test against a throwaway server included — can drive the consumer
    /// without touching the process environment the two `dashboard` resolvers
    /// read.
    ///
    /// A non-success status fails explicitly, body included: a stream that
    /// silently yields nothing is indistinguishable from an agent with no task.
    pub(crate) async fn open(base_url: &str, token: &str) -> Result<Self> {
        let url = stream_url(base_url);
        let client = reqwest::Client::new();
        let resp = client
            .get(&url)
            .header("authorization", format!("Bearer {token}"))
            .header("accept", "text/event-stream")
            .send()
            .await
            .with_context(|| format!("failed to open SSE stream at {url}"))?;

        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!(
                "mika-spirit returned {} for {}: {}",
                status,
                url,
                resp.text().await.unwrap_or_default()
            );
        }
        Ok(Self { resp })
    }

    /// Read until the server closes the stream, handing every `data:` payload to
    /// `on_event` — the bytes as they came off the wire, **and** their decode
    /// into a [`TaskEventFrame`] when it succeeds — one at a time and in order.
    ///
    /// `agent_filter` is a **first, free stage**: under `Some(id)`, a payload
    /// that decodes to a frame naming another agent never reaches `on_event`.
    /// What the filter has *not* understood goes through — an `OverflowMarker`
    /// (the only thing that says information is missing, including possibly the
    /// caller's own), an `Unknown`, and an undecodable payload. Under `None`
    /// nothing is dropped and the raw sequence is the one this consumer yielded
    /// before the filter existed.
    ///
    /// The second closure parameter is an `Option<&TaskEventFrame>` rather than
    /// a `&TaskEventFrame` because the two callers read disjoint halves:
    /// `mika tasks stream` relays the raw payload and must keep showing what it
    /// cannot decode — an instrument blind to a malformed frame measures
    /// nothing — while the TUI's subscription reads the frame and ignores the
    /// bytes. Returns `Ok(())` on a clean close; the caller decides whether to
    /// reconnect.
    pub(crate) async fn consume(
        mut self,
        agent_filter: Option<&str>,
        mut on_event: impl FnMut(&str, Option<&TaskEventFrame>),
    ) -> Result<()> {
        // Minimal SSE frame parser: buffer bytes, split on the event separator,
        // extract `data: <payload>` lines. Using `Response::chunk()` avoids
        // pulling the `futures-util` `Stream` extension (workspace dep is
        // `default-features = false`).
        let mut buffer: Vec<u8> = Vec::with_capacity(4096);

        // The decode and the filter live in this relay, and not in
        // `drain_events`, whose one responsibility is re-splicing an event cut
        // across two packets — a property of the buffer alone, attested with no
        // decode in the way.
        let mut relay = |payload: &str| {
            let frame = serde_json::from_str::<TaskEventFrame>(payload).ok();
            let etrangere = match (agent_filter, frame.as_ref().and_then(frame_agent_id)) {
                (Some(voulu), Some(porte)) => porte != voulu,
                _ => false,
            };
            if !etrangere {
                on_event(payload, frame.as_ref());
            }
        };

        while let Some(chunk) = self.resp.chunk().await.context("SSE stream read failed")? {
            buffer.extend_from_slice(&chunk);
            drain_events(&mut buffer, &mut relay);
        }
        Ok(())
    }
}

/// Le détail d'une tâche de mika-spirit, réduit aux champs que ce vol lit.
///
/// **Miroir local faute d'autre forme disponible** :
/// `mika_agent::server::dashboard::TaskDetailResponse` ne dérive que
/// `Serialize`, et ce vol ne modifie pas `crates/mika-agent/`. Ses autres champs
/// sont ignorés par serde — tolérance voulue, attestée par `corps_de_detail`.
///
/// Dix champs, et **chacun a son lecteur dans ce vol** : `Serialize` en rend dix
/// en `json` et `yaml`, l'écrivain texte en rend les six que l'opérateur lit, et
/// les quatre neufs — `trigger_type`, `action_type`, `created_trace_id` et
/// `parent_task_id` — sont lus par la livraison d'un callback (vol V2b-2b-2).
/// Le miroir ne prend **aucun champ « pour plus tard »** : c'est l'arrivée d'un
/// lecteur, pas la largeur du serveur, qui fait entrer un champ ici.
///
/// La visibilité le dit champ par champ. `label`, `result`, `created_trace_id` et
/// `parent_task_id` sont `pub(crate)` parce que le drainage les lit depuis
/// `tui/app.rs`. Les six autres restent **privés** : leur seul lecteur hors des
/// trois formats est [`TaskDetailMirror::is_resume_callback_for`], qui est une
/// méthode d'ici.
#[derive(serde::Deserialize, serde::Serialize)]
pub(crate) struct TaskDetailMirror {
    id: String,
    agent_id: String,
    pub(crate) label: String,
    status: String,
    trigger_type: String,
    action_type: String,
    created_by_session: Option<String>,
    pub(crate) result: Option<String>,
    pub(crate) created_trace_id: Option<String>,
    pub(crate) parent_task_id: Option<String>,
}

impl TaskDetailMirror {
    /// Le détail est-il un callback de reprise **de cette conversation** ?
    ///
    /// Les quatre colonnes de `get_undelivered_callback_tasks_for_session`
    /// (`crates/mika-agent/src/db/tasks.rs`), dans l'ordre où son SQL les écrit :
    /// la livraison par le fil applique **le prédicat du poll**, et non un
    /// prédicat voisin qui divergerait au premier changement de l'un des deux.
    ///
    /// Une méthode du miroir et non une fonction de `tui/app.rs`, parce que
    /// quatre des cinq champs qu'elle lit n'ont aucun autre lecteur hors de ce
    /// module : posée dehors, elle les rendrait tous `pub(crate)` et le miroir
    /// perdrait son énoncé « chaque champ a son lecteur, et pas un de plus ».
    ///
    /// **Les deux colonnes du poll qui n'y sont pas** ne manquent pas, elles
    /// sont dites ailleurs : `status IN ('completed','failed')` l'est par la
    /// variante de la trame, puis une seconde fois en SQL par
    /// `mark_task_delivered` ; `completed_at NOT NULL` l'est par construction,
    /// une trame terminale étant émise **à la transition**.
    pub(crate) fn is_resume_callback_for(&self, agent_id: &str, session_id: &str) -> bool {
        self.agent_id == agent_id
            && self.trigger_type == "callback"
            && self.action_type == "resume_agent"
            && self.created_by_session.as_deref() == Some(session_id)
    }
}

/// Lire `GET /api/v1/tasks/{task_id}` sur mika-spirit.
///
/// URL de base et jeton voyagent **par argument**, pour la raison déjà écrite
/// sur [`TaskEventStream::open`] : un consommateur qui résout ses propres accès
/// ne se pilote pas depuis un test.
///
/// **Échoue, là où le matériau de #3062 rendait `None`.** Ce `None` était juste
/// dans `tui/app.rs` — la ligne restait non réclamée et le tour suivant la
/// reprenait. Ici il n'y a aucun repli, et un échec avalé rendrait un détail vide
/// comme un succès ; au drainage (vol V2b-2b-2), c'est **l'appelant** qui traduira
/// ce `Err` en « je passe et je ne réclame pas ». Le `timeout` de 5 s vient du
/// matériau : un flux a le droit de se taire, une requête courte non.
pub(crate) async fn fetch_task_detail(
    base_url: &str,
    token: &str,
    task_id: &str,
) -> Result<TaskDetailMirror> {
    let url = format!("{}/api/v1/tasks/{task_id}", base_url.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .get(&url)
        .header("authorization", format!("Bearer {token}"))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .with_context(|| format!("failed to fetch task detail at {url}"))?;

    let statut = resp.status();
    if !statut.is_success() {
        let corps = resp.text().await.unwrap_or_default();
        anyhow::bail!("mika-spirit returned {statut} for {url}: {corps}");
    }
    resp.json::<TaskDetailMirror>()
        .await
        .with_context(|| format!("unreadable task detail for '{task_id}' from {url}"))
}

/// Rendre le miroir en texte, sur le modèle visuel de [`print_task_detail`].
///
/// Un écrivain **neuf** pour trois raisons dont aucune n'est de goût. Un critère
/// de ce vol : `print_task_detail` n'est pas modifiée. Son type : elle prend un
/// `&Task` de vingt champs que le miroir n'a pas. Son second argument : la
/// vivacité du pilote est **locale**, donc sans réponse sous `--spirit`, et
/// l'absence de ligne `Dispatch pilot:` est exacte, pas incomplète.
fn write_task_detail_text<W: std::io::Write>(detail: &TaskDetailMirror, out: &mut W) -> Result<()> {
    let session = detail.created_by_session.as_deref().unwrap_or("—");
    let resultat = detail.result.as_deref().unwrap_or("—");
    writeln!(out, "\n  Task Detail (mika-spirit)")?;
    writeln!(out, "  ───────────────────────────────────")?;
    for (etiquette, valeur) in [
        ("ID:", detail.id.as_str()),
        ("Label:", &detail.label),
        ("Status:", &detail.status),
        ("Agent:", &detail.agent_id),
        ("Session:", session),
        ("Result:", resultat),
    ] {
        writeln!(out, "  {etiquette:<15}{valeur}")?;
    }
    writeln!(out)?;
    Ok(())
}

/// Ce que `mika tasks get <id> --spirit` rend, avec son puits en paramètre.
///
/// Le puits est un paramètre pour la raison déjà écrite sur
/// [`stream_task_events_into`] : la propriété à tenir est « le détail, **et rien
/// d'autre** », et elle ne se mesure qu'en comparant tout le puits. Et **l'ordre
/// des gestes porte la moitié du critère 2** : lecture, puis `?`, puis écriture —
/// un échec laisse donc le puits vide par conséquence, non par politique.
async fn get_task_detail_from_spirit_into<W: std::io::Write>(
    base_url: &str,
    token: &str,
    task_id: &str,
    format: OutputFormat,
    out: &mut W,
) -> Result<()> {
    let detail = fetch_task_detail(base_url, token, task_id).await?;
    match format {
        OutputFormat::Text => write_task_detail_text(&detail, out)?,
        OutputFormat::Json => writeln!(out, "{}", serde_json::to_string_pretty(&detail)?)?,
        OutputFormat::Yaml => write!(out, "{}", yaml_serde::to_string(&detail)?)?,
    }
    Ok(())
}

/// Hand every **complete** event sitting in `buffer` to `on_payload`, and leave
/// a trailing partial one in place for the next read.
///
/// A free function rather than the body of the read loop, because that is the
/// only way to attest the re-splicing without depending on a timing: an event
/// cut across two packets is a property of the **buffer**, and a test making the
/// server emit in two `sleep`-separated writes would go green without measuring
/// anything on the day a layer coalesced them.
fn drain_events(buffer: &mut Vec<u8>, on_payload: &mut impl FnMut(&str)) {
    // Split on SSE event separator (`\r\n\r\n` / `\n\n` / `\r\r`, spec § 9.2).
    // Keep the trailing partial event in the buffer for the next read.
    while let Some((sep_idx, sep_len)) = find_event_separator(buffer) {
        let event_bytes: Vec<u8> = buffer.drain(..sep_idx + sep_len).collect();
        // Drop the trailing separator; parse the remainder line-by-line.
        let event_str = String::from_utf8_lossy(&event_bytes[..event_bytes.len() - sep_len]);
        for line in event_str.split(['\n', '\r']) {
            if let Some(data) = line.strip_prefix("data:") {
                let data = data.trim_start();
                if data.is_empty() {
                    continue;
                }
                on_payload(data);
            }
        }
    }
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
/// **The traversal is not performed here** (mika#2670).
/// [`mika_agent::live_pilot::resolve_task_pilot`] is the sole reader of *which
/// pilot does this task carry?* since mika#2653 phase B, and this function only
/// classifies what it hands back. Written a second time — as it was until
/// mika#2670 — the two readings diverge in silence, and they had: this one
/// applied the **terminal filter in neither branch**, so a terminal callback row
/// still carrying a live pgid printed "pilot alive" to the operator while the
/// engine held it out of population. The discrimination between the two rows of
/// a dispatch (mika#2156) is now the sole reader's — a non-terminal
/// `process_id` on the row itself — and no longer `trigger_type == "callback"`.
///
/// What stays here is the **classification**, because the operator's question is
/// not the engine's: the verdict side of `live_pilot` has three fail-safe arms,
/// whereas an operator needs a dead PID told apart from no pilot at all, and a
/// PID that cannot be verified printed *with* its PID.
async fn probe_pilot_liveness(db: &AsyncDatabase, settings: &Settings, t: &Task) -> PilotLiveness {
    match mika_agent::live_pilot::resolve_task_pilot(db, &t.id, Some(t)).await {
        // The row carries the pgid itself. Without a readable
        // `process_start_time` a recycled PID cannot be told from the pilot, so
        // we claim neither.
        TaskPilot::Named { pid, start_time } => match start_time {
            Some(start_time) => classify(pid, None, start_time, settings, &t.id),
            None => PilotLiveness::Unverifiable {
                pid,
                child_id: None,
            },
        },
        // A parent can carry several PID-bearing children across retries, so the
        // answers must be ranked rather than overwritten: **alive > unverifiable >
        // dead > none**. A live child is the answer outright. Failing that, an
        // *unverifiable* child outranks a dead one — we checked the dead one and it
        // is gone, whereas the unverifiable one might be running and we cannot tell.
        // Letting a later dead child overwrite an earlier unverifiable one would
        // print "no live pilot" over a genuine unknown, which is the fail-safe
        // inversion this whole ticket exists to remove.
        TaskPilot::Children(children) => {
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
        TaskPilot::ChildrenUnreadable(_) => PilotLiveness::Unknown,
    }
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

    // -- la découpe d'événements SSE est attestée (mika#1727, vol V2b-1b) --
    //
    // `find_event_separator` est arrivée ici avec le consommateur de diagnostic
    // (mika#1758) et **rien ne la mesurait** : son doc-comment nomme l'ordre
    // d'essai des trois terminateurs, et c'était une affirmation sur du code.
    // Ces trois tests atterrissent **avant** que le consommateur bouge — écrits
    // après, ils n'attesteraient que l'état d'arrivée du refactor.

    #[test]
    fn mika1727_v2b1b_les_trois_separateurs_sont_reconnus() {
        assert_eq!(find_event_separator(b"data: x\n\n"), Some((7, 2)));
        assert_eq!(find_event_separator(b"data: x\r\n\r\n"), Some((7, 4)));
        assert_eq!(find_event_separator(b"data: x\r\r"), Some((7, 2)));
        assert_eq!(find_event_separator(b"data: x\n"), None);
    }

    /// L'ordre d'essai est la moitié porteuse : un `\r\n\r\n` doit se lire comme
    /// un seul terminateur de 4 octets, jamais comme le `\n\n` qui siège à
    /// l'offset+1 — ce qui laisserait un `\r` orphelin en tête de l'événement
    /// suivant.
    #[test]
    fn mika1727_v2b1b_un_crlfcrlf_nest_pas_lu_comme_un_lflf_decale() {
        let (idx, len) = find_event_separator(b"a\r\n\r\nb").unwrap();
        assert_eq!((idx, len), (1, 4));
        assert_eq!(&b"a\r\n\r\nb"[idx + len..], b"b");
    }

    #[test]
    fn mika1727_v2b1b_le_premier_separateur_gagne() {
        assert_eq!(find_event_separator(b"a\n\nb\n\nc"), Some((1, 2)));
    }

    /// Le recollage se mesure sur le **tampon**, et pas sur une socket : un
    /// serveur factice qui écrirait en deux paquets séparés d'un `sleep`
    /// passerait au vert le jour où une couche les fusionne, sans plus rien
    /// mesurer. Ici les deux moitiés sont choisies à la main, donc la coupure
    /// est la propriété tenue et non un effet de cadence.
    #[test]
    fn mika1727_v2b1b_un_evenement_coupe_en_deux_paquets_est_rendu_entier_une_fois() {
        let mut rendues: Vec<String> = Vec::new();
        let mut buffer: Vec<u8> = Vec::new();

        // Premier paquet : la charge est tronquée en plein milieu du JSON, et
        // aucun séparateur n'est encore arrivé.
        buffer.extend_from_slice(b"data: {\"task_id\":\"t-1\",\"sta");
        drain_events(&mut buffer, &mut |charge: &str| {
            rendues.push(charge.to_string())
        });
        assert!(
            rendues.is_empty(),
            "un événement incomplet ne doit rien rendre : {rendues:?}"
        );

        // Second paquet : la fin de la charge, son séparateur, puis le début
        // d'un troisième événement qui doit rester en attente.
        buffer.extend_from_slice(b"tus\":\"done\"}\n\ndata: {\"task_id\":\"t-2\"");
        drain_events(&mut buffer, &mut |charge: &str| {
            rendues.push(charge.to_string())
        });
        assert_eq!(rendues, vec![r#"{"task_id":"t-1","status":"done"}"#]);

        // Et le résidu est conservé tel quel pour la lecture suivante.
        assert_eq!(buffer, b"data: {\"task_id\":\"t-2\"");
    }

    /// Le consommateur s'ouvre sur une URL et un jeton **donnés**, et rend ses
    /// trames une fois chacune, dans l'ordre : une trame perdue ou doublée fait
    /// rougir l'égalité.
    #[tokio::test]
    async fn mika1727_v2b1b_le_consommateur_rend_ses_trames_dans_lordre() {
        let charges = [
            r#"{"task_id":"t-1","status":"pending"}"#,
            r#"{"task_id":"t-2","status":"in_progress"}"#,
            r#"{"task_id":"t-3","status":"completed"}"#,
        ];
        let base = fake_spirit::spawn(fake_spirit::sse_payload(&charges)).await;

        let flux = TaskEventStream::open(&base, "jeton-de-test").await.unwrap();
        let mut rendues: Vec<String> = Vec::new();
        flux.consume(None, |charge, _| rendues.push(charge.to_string()))
            .await
            .unwrap();

        assert_eq!(rendues, charges);
    }

    /// `mika tasks stream` écrit une ligne par charge `data:` et **rien
    /// d'autre** : l'assertion est une égalité sur tout le puits, jamais un
    /// `contains`. Les deux lignes de connexion partent sur stderr, donc leur
    /// absence d'ici *est* la mesure de « rien d'autre » — pas une redite.
    #[tokio::test]
    async fn mika1727_v2b1b_mika_tasks_stream_ecrit_une_ligne_par_charge_et_rien_dautre() {
        let charges = [
            r#"{"task_id":"t-1"}"#,
            r#"{"task_id":"t-2"}"#,
            r#"{"task_id":"t-3"}"#,
        ];
        let base = fake_spirit::spawn(fake_spirit::sse_payload(&charges)).await;

        let mut puits: Vec<u8> = Vec::new();
        stream_task_events_into(&base, "jeton-de-test", None, &mut puits)
            .await
            .unwrap();

        assert_eq!(
            String::from_utf8(puits).unwrap(),
            "{\"task_id\":\"t-1\"}\n{\"task_id\":\"t-2\"}\n{\"task_id\":\"t-3\"}\n"
        );
    }

    /// `mika tasks stream --agent <id>` n'écrit que les lignes de cet agent.
    /// C'est ce test qui fait du flag un **lecteur mesuré** du filtre : l'égalité
    /// porte sur tout le puits, donc une ligne fuitée rougit autant qu'une ligne
    /// perdue, et la sortie reste la charge **brute** — le décodage voyage pour
    /// le filtre, pas pour changer la forme de l'écriture.
    #[tokio::test]
    async fn mika1727_v2b2a_mika_tasks_stream_agent_necrit_que_les_lignes_de_lagent_demande() {
        let charges = [
            r#"{"event":"task_completed","task_id":"t-1","agent_id":"mika-dev","completed_at":"2026-11-09T10:00:00Z","result_preview":null}"#,
            r#"{"event":"task_completed","task_id":"t-2","agent_id":"mika-qa","completed_at":"2026-11-09T10:00:01Z","result_preview":null}"#,
        ];
        let base = fake_spirit::spawn(fake_spirit::sse_payload(&charges)).await;

        let mut puits: Vec<u8> = Vec::new();
        stream_task_events_into(&base, "jeton-de-test", Some("mika-dev"), &mut puits)
            .await
            .unwrap();

        assert_eq!(
            String::from_utf8(puits).unwrap(),
            format!("{}\n", charges[0])
        );
    }

    // -- le filtre par agent du fil des tâches (mika#1727, vol V2b-2a) --

    /// Chaque variante du fil dit, ou non, son agent. Ce test mesure le
    /// **choix** — six variantes en portent un, `OverflowMarker` et `Unknown`
    /// n'en portent pas — et non l'exhaustivité du `match`, qui est tenue par le
    /// compilateur et n'a pas besoin d'un test pour l'être.
    #[test]
    fn mika1727_v2b2a_chaque_variante_du_fil_dit_ou_non_son_agent() {
        let quand = "2026-11-09T10:00:00Z".to_string();
        let porteuses = [
            TaskEventFrame::TaskCreated {
                task_id: "t-1".to_string(),
                agent_id: "mika-dev".to_string(),
                kind: "callback".to_string(),
                action_type: "resume_agent".to_string(),
                label: None,
                parent_task_id: None,
                created_at: quand.clone(),
            },
            TaskEventFrame::TaskClaimed {
                task_id: "t-2".to_string(),
                agent_id: "mika-dev".to_string(),
                claimed_at: quand.clone(),
            },
            TaskEventFrame::TaskCompleted {
                task_id: "t-3".to_string(),
                agent_id: "mika-dev".to_string(),
                completed_at: quand.clone(),
                result_preview: None,
            },
            TaskEventFrame::TaskFailed {
                task_id: "t-4".to_string(),
                agent_id: "mika-dev".to_string(),
                failed_at: quand.clone(),
                error_preview: None,
            },
            TaskEventFrame::TaskDelivered {
                task_id: "t-5".to_string(),
                agent_id: "mika-dev".to_string(),
                delivered_at: quand.clone(),
            },
            TaskEventFrame::TaskCancelled {
                task_id: "t-6".to_string(),
                agent_id: "mika-dev".to_string(),
                cancelled_at: quand,
                reason: None,
            },
        ];
        for trame in &porteuses {
            assert_eq!(frame_agent_id(trame), Some("mika-dev"), "{trame:?}");
        }

        assert_eq!(
            frame_agent_id(&TaskEventFrame::OverflowMarker { dropped_count: 7 }),
            None
        );
        assert_eq!(frame_agent_id(&TaskEventFrame::Unknown), None);
    }

    /// Les charges de test sont des JSON **littéraux**, et non des
    /// `TaskEventFrame` sérialisés : un aller-retour `to_string` → `from_str` du
    /// même type ne mesurerait que la symétrie de serde avec elle-même, et
    /// resterait vert le jour où le tag cesserait d'être `event` — c'est-à-dire
    /// le jour où le consommateur cesserait de comprendre ce que le serveur
    /// émet. Le littéral épingle le format du **fil**, qui est le contrat réel
    /// entre `mika-agent` et `mika-cli`.
    ///
    /// Le contrôle négatif vit dans ce test : sous `None`, les trois trames
    /// passent. Sans lui, un filtre qui écarterait *tout* serait vert.
    #[tokio::test]
    async fn mika1727_v2b2a_une_trame_dun_autre_agent_est_ecartee_du_fil() {
        let charges = [
            r#"{"event":"task_claimed","task_id":"t-1","agent_id":"mika-dev","claimed_at":"2026-11-09T10:00:00Z"}"#,
            r#"{"event":"task_claimed","task_id":"t-2","agent_id":"mika-qa","claimed_at":"2026-11-09T10:00:01Z"}"#,
            r#"{"event":"task_completed","task_id":"t-3","agent_id":"mika-dev","completed_at":"2026-11-09T10:00:02Z","result_preview":"fait"}"#,
        ];

        let base = fake_spirit::spawn(fake_spirit::sse_payload(&charges)).await;
        let mut filtrees: Vec<String> = Vec::new();
        TaskEventStream::open(&base, "jeton-de-test")
            .await
            .unwrap()
            .consume(Some("mika-dev"), |charge, _| {
                filtrees.push(charge.to_string())
            })
            .await
            .unwrap();
        assert_eq!(filtrees, vec![charges[0], charges[2]]);

        let mut toutes: Vec<String> = Vec::new();
        TaskEventStream::open(&base, "jeton-de-test")
            .await
            .unwrap()
            .consume(None, |charge, _| toutes.push(charge.to_string()))
            .await
            .unwrap();
        assert_eq!(toutes, charges);
    }

    /// Le filtre n'écarte que ce qu'il a **compris** comme étranger. Un
    /// `overflow_marker` dit à un consommateur filtré qu'il a perdu des trames,
    /// dont peut-être les siennes : l'écarter serait perdre la seule information
    /// qui dit que l'information manque.
    ///
    /// L'égalité porte sur toute la séquence, ce qui mesure du même coup que
    /// `consume` est allée **jusqu'au bout du corps** : une charge indécodable
    /// qui interromprait la lecture tronquerait la séquence au lieu de la
    /// changer.
    #[tokio::test]
    async fn mika1727_v2b2a_le_filtre_laisse_passer_ce_quil_ne_comprend_pas() {
        let charges = [
            r#"{"event":"overflow_marker","dropped_count":3}"#,
            r#"{"event":"trame_du_futur","task_id":"t-9","agent_id":"mika-qa"}"#,
            r#"ceci n'est pas du JSON"#,
            r#"{"event":"task_failed","task_id":"t-2","agent_id":"mika-qa","failed_at":"2026-11-09T10:00:01Z","error_preview":null}"#,
        ];
        let base = fake_spirit::spawn(fake_spirit::sse_payload(&charges)).await;

        let mut rendues: Vec<String> = Vec::new();
        TaskEventStream::open(&base, "jeton-de-test")
            .await
            .unwrap()
            .consume(Some("mika-dev"), |charge, _| {
                rendues.push(charge.to_string())
            })
            .await
            .unwrap();

        assert_eq!(rendues, vec![charges[0], charges[1], charges[2]]);
    }

    // -- le détail se lit sur mika-spirit (mika#1727, vol V2b-2b-1) --

    /// Un corps tel que `dashboard::handle_task_detail` le rend : les dix champs du
    /// miroir, **plus deux** qu'il laisse tomber — et qui mesurent la tolérance,
    /// sans quoi elle resterait une affirmation du commentaire.
    fn corps_de_detail() -> String {
        json!({"id": "t-42", "agent_id": "mika-qa", "label": "sonde:detail",
               "status": "completed", "trigger_type": "callback",
               "action_type": "resume_agent", "created_by_session": "s-7",
               "result": "ok", "created_trace_id": "tr-9", "parent_task_id": "t-41",
               "depth": 1, "action_config": "{}"})
        .to_string()
    }

    /// Lire le détail sur `base` et rendre les deux moitiés que ces tests mesurent.
    async fn lire_detail(base: &str, id: &str, format: OutputFormat) -> (Vec<u8>, Result<()>) {
        let mut puits: Vec<u8> = Vec::new();
        let issue = get_task_detail_from_spirit_into(base, "jeton", id, format, &mut puits).await;
        (puits, issue)
    }

    /// **Critère 1** — `mika tasks get <id> --spirit` rend le détail lu sur
    /// mika-spirit, et le jeton part en `Bearer`. Deux mesures dans un test parce
    /// qu'elles portent sur **la même requête** : ce qui en est revenu, et ce qui y
    /// est parti. L'égalité porte sur tout le puits, donc elle mesure aussi
    /// l'absence de la ligne `Dispatch pilot:`.
    #[tokio::test]
    async fn mika1727_v2b2b1_mika_tasks_get_spirit_rend_le_detail_lu_sur_mika_spirit() {
        let (base, recu) = fake_spirit::spawn_detail(200, corps_de_detail()).await;

        let (puits, issue) = lire_detail(&base, "t-42", OutputFormat::Text).await;

        issue.unwrap();
        let attendu = concat!(
            "\n  Task Detail (mika-spirit)\n",
            "  ───────────────────────────────────\n",
            "  ID:            t-42\n  Label:         sonde:detail\n",
            "  Status:        completed\n  Agent:         mika-qa\n",
            "  Session:       s-7\n  Result:        ok\n\n",
        );
        assert_eq!(String::from_utf8(puits).unwrap(), attendu);
        let entete = recu.lock().unwrap().clone();
        assert_eq!(entete.as_deref(), Some("Bearer jeton"));
    }

    /// **Critère 1** — `json` et `yaml` rendent le même miroir, ni un champ de plus
    /// ni un de moins : les deux attentes se dérivent d'**une** liste de dix paires,
    /// donc un onzième champ rendu ne peut pas être vert ici.
    ///
    /// La liste est à dix depuis le vol V2b-2b-2, et l'attente du **texte** ne l'a
    /// pas suivie : elle reste les six paires du test précédent, parce que
    /// l'écrivain texte ne rend pas `trigger_type` ni `action_type` — des colonnes
    /// de machine, pas une lecture d'opérateur. L'invariant « ni un de plus ni un de
    /// moins » survit donc sur chaque format, pris séparément.
    #[tokio::test]
    async fn mika1727_v2b2b1_les_trois_formats_rendent_le_meme_miroir() {
        let (base, _) = fake_spirit::spawn_detail(200, corps_de_detail()).await;
        let champs = [
            ("id", "t-42"),
            ("agent_id", "mika-qa"),
            ("label", "sonde:detail"),
            ("status", "completed"),
            ("trigger_type", "callback"),
            ("action_type", "resume_agent"),
            ("created_by_session", "s-7"),
            ("result", "ok"),
            ("created_trace_id", "tr-9"),
            ("parent_task_id", "t-41"),
        ];

        let (rendu, issue) = lire_detail(&base, "t-42", OutputFormat::Json).await;
        issue.unwrap();
        let lignes = champs.map(|(c, v)| format!("  \"{c}\": \"{v}\""));
        let attendu = format!("{{\n{}\n}}\n", lignes.join(",\n"));
        assert_eq!(String::from_utf8(rendu).unwrap(), attendu);

        let (rendu, issue) = lire_detail(&base, "t-42", OutputFormat::Yaml).await;
        issue.unwrap();
        let corps: String = champs.iter().map(|(c, v)| format!("{c}: {v}\n")).collect();
        assert_eq!(String::from_utf8(rendu).unwrap(), corps);
    }

    /// **Critère 2** — un 404 échoue explicitement et n'écrit rien. Le corps est
    /// celui que `dashboard::handle_task_detail` rend vraiment. « Explicitement » se
    /// mesure par le statut présent dans le message ; « n'écrit rien » par le puits
    /// vide, qui tient à l'ordre des gestes et non à une politique d'écriture.
    #[tokio::test]
    async fn mika1727_v2b2b1_un_404_echoue_explicitement_et_necrit_rien() {
        let (base, _) =
            fake_spirit::spawn_detail(404, r#"{"error":"task 't-absente' not found"}"#.to_string())
                .await;

        let (puits, issue) = lire_detail(&base, "t-absente", OutputFormat::Text).await;

        let message = format!("{:#}", issue.expect_err("un 404 doit échouer"));
        assert!(message.contains("404"), "le statut est nommé : {message}");
        assert!(puits.is_empty(), "rien n'est écrit : {puits:?}");
    }

    /// **Critère 2** — un 500 échoue explicitement et n'écrit rien. Deux tests et non
    /// un seul paramétré : le critère demande « un test pour chacun des trois cas »,
    /// et 404 et 500 ne diront pas la même chose au vol V2b-2b-2 qui lira l'erreur.
    #[tokio::test]
    async fn mika1727_v2b2b1_un_500_echoue_explicitement_et_necrit_rien() {
        let (base, _) =
            fake_spirit::spawn_detail(500, r#"{"error":"database is locked"}"#.to_string()).await;

        let (puits, issue) = lire_detail(&base, "t-1", OutputFormat::Json).await;

        let message = format!("{:#}", issue.expect_err("un 500 doit échouer"));
        assert!(message.contains("500"), "le statut est nommé : {message}");
        assert!(puits.is_empty(), "rien n'est écrit : {puits:?}");
    }

    /// **Critère 2** — un corps illisible échoue explicitement et n'écrit rien. Un
    /// **200**, un JSON valide, et `agent_id` absent : c'est le cas qui compte, car
    /// c'est la forme qu'un serveur ayant dérivé rendrait, et elle ne doit jamais
    /// passer pour un succès.
    #[tokio::test]
    async fn mika1727_v2b2b1_un_corps_illisible_echoue_explicitement_et_necrit_rien() {
        let (base, _) = fake_spirit::spawn_detail(200, r#"{"id":"t-1"}"#.to_string()).await;

        let (puits, issue) = lire_detail(&base, "t-1", OutputFormat::Text).await;

        let message = format!("{:#}", issue.expect_err("un corps illisible doit échouer"));
        assert!(message.contains("t-1"), "la tâche est nommée : {message}");
        assert!(puits.is_empty(), "rien n'est écrit : {puits:?}");
    }

    // -- the terminal filter reaches the operator's surface (mika#2670) --

    /// A callback row, as `dispatch-lib` leaves it: the pgid on the row itself,
    /// `process_start_time` in metadata, `status` as the parameter says.
    ///
    /// One parameterized helper rather than two: `NewTask` carries ~22 fields,
    /// and the only thing the negative control and its positive twin disagree
    /// about is `status`. Two helpers would let that one difference drift.
    #[cfg(target_os = "linux")]
    async fn seed_callback_row(status: &str, pid: i64, start_time: u64) -> (AsyncDatabase, Task) {
        use mika_agent::db::{Database, NewTask};

        const AGENT: &str = "mika";
        let db = AsyncDatabase::new_with_agent(
            Database::open_in_memory().expect("open in-memory DB"),
            AGENT,
        );
        let id = db
            .create_task(NewTask {
                agent_id: AGENT.to_string(),
                team_run_id: None,
                parent_task_id: None,
                depth: 1,
                label: "long_running:run_claude_pilot".to_string(),
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
                created_by_session: Some("s".to_string()),
                created_trace_id: None,
                reference_url: None,
                source: Some("self_dev".to_string()),
                metadata: None,
                r#type: None,
                dispatch_class: Some("implement".to_string()),
            })
            .await
            .expect("create callback row");
        if status != "pending" {
            db.update_task_status(&id, status)
                .await
                .expect("set status");
        }
        db.set_task_process_id(&id, Some(pid))
            .await
            .expect("record pgid");
        db.set_task_metadata_field(&id, "process_start_time", &start_time.to_string())
            .await
            .expect("record start time");
        let t = db
            .get_task(&id)
            .await
            .expect("read back")
            .expect("row exists");
        (db, t)
    }

    /// Our own process: alive, with a start time that matches by construction.
    /// Same device as `live_pilot.rs`'s `self_pid_and_start()` — a *real* live
    /// PID, because a synthetic one would make the negative control vacant.
    #[cfg(target_os = "linux")]
    fn self_pid_and_start() -> (i64, u64) {
        let pid = std::process::id();
        let st = mika_agent::task_engine::process_liveness::read_process_start_time(pid)
            .expect("read own start time");
        (i64::from(pid), st)
    }

    /// **AC3 of mika#2670** — a terminal row carrying a live PID is not shown
    /// alive.
    ///
    /// This is the defect, in one assertion. `dispatch-lib`'s EXIT trap delivers
    /// the callback from **inside the still-running wrapper**, so `delivered`
    /// precedes process exit by a real window: a terminal row whose pgid is
    /// still alive is the nominal state during that window, not a laboratory
    /// curiosity. The engine holds such a row out of population; until mika#2670
    /// this surface printed "pilot alive" over it, because it redid the
    /// traversal without the terminal filter.
    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn mika2670_une_ligne_terminale_a_pid_vivant_nest_pas_affichee_vivante() {
        let (pid, start_time) = self_pid_and_start();
        let (db, t) = seed_callback_row("delivered", pid, start_time).await;

        let liveness = probe_pilot_liveness(&db, &Settings::test_defaults(), &t).await;

        assert!(
            !matches!(liveness, PilotLiveness::Alive { .. }),
            "mika#2670 — une ligne `delivered` portant un pid vivant est lue \
             {liveness:?} : le filtre terminal du lecteur unique n'atteint pas \
             la surface opérateur"
        );
        assert!(
            !summary_annotation(&t.trigger_type, &t.status, &liveness).contains("alive"),
            "la ligne de résumé ne doit pas dire « pilot alive » sur une ligne \
             terminale"
        );
    }

    /// **Le jumeau positif, et il est dû.** Même ligne, statut non terminal ⇒
    /// `Alive`.
    ///
    /// Sans lui, le contrôle négatif ci-dessus ne distingue pas « le filtre
    /// mord » de « le semis n'a jamais produit de pid vivant » : un test qui
    /// vise un cadavre se lit exactement comme un arbre propre (mika#2103 /
    /// mika#2205). C'est aussi lui qui rougit le jour où `read_process_start_time`
    /// ou le semis cessent de fonctionner, plutôt que le négatif qui verdirait
    /// en silence.
    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn mika2670_la_meme_ligne_non_terminale_est_affichee_vivante() {
        let (pid, start_time) = self_pid_and_start();
        let (db, t) = seed_callback_row("in_progress", pid, start_time).await;

        let liveness = probe_pilot_liveness(&db, &Settings::test_defaults(), &t).await;

        assert!(
            matches!(liveness, PilotLiveness::Alive { pid: p, .. } if p == pid),
            "le semis doit produire un pilote réellement vivant, sinon le \
             contrôle négatif de mika#2670 est vacant — lu {liveness:?}"
        );
    }

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

/// Un serveur SSE factice pour le flux task-events de mika-spirit, sur le patron
/// `axum::serve` du crate (`commands/chat_turn.rs`, `commands/agents.rs`,
/// `budget_floor.rs`).
///
/// **`pub(crate)` pour que le vol V2b-2 le reprenne** depuis les tests de la
/// TUI, au lieu d'en écrire un second.
///
/// Deux choses sur son emplacement, qui ne sont pas des détails de goût :
///
/// - c'est un **bloc** `#[cfg(test)] pub(crate) mod …`, et non un `#[cfg(test)]`
///   posé sur des `fn` isolées : le scanner egress
///   (`scripts/lib/egress_manifest_lint.py`) n'écarte le code de test que sous
///   cette forme, et une `fn` annotée seule serait lue comme de la production ;
/// - il est posé **après** le module `tests`, en fin de fichier, là où la coupure
///   du scanner a déjà eu lieu. Le placer plus haut déplacerait cette coupure
///   vers lui et rendrait invisibles au lint toutes les lignes qui suivent.
#[cfg(test)]
pub(crate) mod fake_spirit {
    use axum::Router;
    use axum::routing::get;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    /// Mettre `router` en écoute sur un port libre et rendre son URL de base.
    ///
    /// La queue des trois constructeurs — `bind`, `local_addr`, `tokio::spawn`,
    /// `format!` — était écrite trois fois à l'identique. Trois lignes dupliquées
    /// valent mieux qu'une abstraction, neuf non.
    async fn servir(router: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router.into_make_service()).await;
        });
        format!("http://{addr}")
    }

    /// Encode une suite de charges en un corps SSE : un événement par charge,
    /// terminé par `\n\n`.
    pub(crate) fn sse_payload(charges: &[&str]) -> String {
        charges
            .iter()
            .map(|charge| format!("data: {charge}\n\n"))
            .collect()
    }

    /// Sert `payload` sur `/api/v1/dashboard/tasks/stream`, puis ferme.
    ///
    /// Le corps est **fini** : à la fermeture `consume` rend `Ok(())`, donc un
    /// test assertant toute la séquence s'arrête de lui-même, sans timeout ni
    /// `abort`. Un flux qui reste ouvert est ce dont la souscription de la TUI
    /// aura besoin (vol V2b-2) ; il arrivera avec son appelant, pas avant.
    ///
    /// Rend l'URL de **base**, celle que [`super::TaskEventStream::open`]
    /// complète.
    pub(crate) async fn spawn(payload: String) -> String {
        let router = Router::new().route(
            "/api/v1/dashboard/tasks/stream",
            get(move || {
                let corps = payload.clone();
                async move {
                    axum::response::Response::builder()
                        .header("content-type", "text/event-stream")
                        .body(axum::body::Body::from(corps))
                        .expect("corps SSE factice")
                }
            }),
        );
        servir(router).await
    }

    /// L'en-tête `authorization` tel que la socket l'a reçu, `None` avant la requête.
    pub(crate) type EnTeteRecu = Arc<Mutex<Option<String>>>;

    /// Sert `/api/v1/tasks/{task_id}` avec le `statut` et le `corps` que le test
    /// choisit, et rend l'URL de base avec le témoin de l'en-tête reçu.
    ///
    /// Le chemin est **paramétré** et le corps cloné par le gestionnaire : un seul
    /// serveur répond donc à plusieurs identifiants et à plusieurs requêtes, ce dont
    /// le test des trois formats a besoin. Le témoin fait de « le jeton part en
    /// `Bearer` » une mesure prise **sur la socket**, non une relecture du code.
    pub(crate) async fn spawn_detail(statut: u16, corps: String) -> (String, EnTeteRecu) {
        let recu: EnTeteRecu = Arc::new(Mutex::new(None));
        let temoin = Arc::clone(&recu);
        let router = Router::new().route(
            "/api/v1/tasks/{task_id}",
            get(move |en_tetes: axum::http::HeaderMap| {
                let corps = corps.clone();
                let temoin = Arc::clone(&temoin);
                async move {
                    *temoin.lock().expect("témoin d'en-tête") = en_tetes
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string);
                    axum::response::Response::builder()
                        .status(statut)
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(corps))
                        .expect("corps de détail factice")
                }
            }),
        );
        (servir(router).await, recu)
    }

    /// Sert le flux **et** les détails sur une **même** URL de base, parce que la
    /// TUI n'en connaît qu'une : `flux` sur `/api/v1/dashboard/tasks/stream`, et
    /// `details` — des triplets `(task_id, statut, corps)` — sur
    /// `/api/v1/tasks/{task_id}`, résolus par identifiant dans une table
    /// partagée. Un identifiant hors table rend un 404, comme le vrai serveur.
    ///
    /// Le corps SSE reste **fini** : sans reconnexion, un test assertant toute la
    /// séquence s'arrête de lui-même, sans timeout ni `abort`. La variante au flux
    /// jamais fermé arrive au vol qui la sert (V2b-2b-3).
    pub(crate) async fn spawn_stream_and_details(
        flux: String,
        details: Vec<(String, u16, String)>,
    ) -> String {
        let table: Arc<std::collections::HashMap<String, (u16, String)>> = Arc::new(
            details
                .into_iter()
                .map(|(id, statut, corps)| (id, (statut, corps)))
                .collect(),
        );
        let router = Router::new()
            .route(
                "/api/v1/dashboard/tasks/stream",
                get(move || {
                    let corps = flux.clone();
                    async move {
                        axum::response::Response::builder()
                            .header("content-type", "text/event-stream")
                            .body(axum::body::Body::from(corps))
                            .expect("corps SSE factice")
                    }
                }),
            )
            .route(
                "/api/v1/tasks/{task_id}",
                get(
                    move |axum::extract::Path(id): axum::extract::Path<String>| {
                        let table = Arc::clone(&table);
                        async move {
                            let (statut, corps) = table.get(&id).cloned().unwrap_or((
                                404,
                                format!(r#"{{"error":"task '{id}' not found"}}"#),
                            ));
                            axum::response::Response::builder()
                                .status(statut)
                                .header("content-type", "application/json")
                                .body(axum::body::Body::from(corps))
                                .expect("corps de détail factice")
                        }
                    },
                ),
            );
        servir(router).await
    }
}

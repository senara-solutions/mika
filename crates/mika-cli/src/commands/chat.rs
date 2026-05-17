use anyhow::Result;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use secrecy::ExposeSecret;
use std::io;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinHandle};
use uuid::Uuid;

use crate::init::{self, AppContext};
use crate::tui::app::{
    AgentRequest, AgentResponse, AgentStatus, App, ChatMessage, ChatRole, TeamRequest,
    session_message_to_chat_message,
};
use crate::tui::event::{AppEvent, EventReader};
use crate::tui::input;
use crate::tui::ui;
use mika_agent::agent::{self, AgentParams, check_onboarding};
use mika_agent::prompt;
use mika_agent::skills::SkillRegistry;
use mika_agent::task_engine::{self, TaskDispatcher, TaskEngine};
use mika_agent::teams::types::{RunStatus, TeamEvent};
use mika_agent::tools;
use mika_common::claude::ThinkingConfig;
use mika_common::config::Settings;
use mika_common::llm::LlmImage;
use mika_common::team;
use std::path::Path;

/// Holds the agent worker task handle, poller handle, and the AppContext (for DB shutdown).
///
/// `handle` is the supervisor task's JoinHandle. The supervisor owns the worker's
/// JoinHandle internally and forwards crash notifications as
/// `AgentResponse::WorkerCrashed` events on `agent_rx`. `worker_abort` is an
/// AbortHandle to the *worker* task — used during agent switch and app exit to
/// signal an intentional shutdown (so the supervisor sees `Err(JoinError::is_cancelled)`
/// and exits silently rather than emitting a spurious crash event).
struct AgentWorker {
    handle: JoinHandle<()>,
    worker_abort: AbortHandle,
    poller_handle: JoinHandle<()>,
    _ctx: AppContext,
}

/// Spawn the agent worker task. Returns the worker, channels, and context info needed for App.
async fn spawn_agent_worker(
    ctx: AppContext,
    _agent_name: &str,
    http_client: &reqwest::Client,
    session_id: Option<&str>,
) -> Result<(
    AgentWorker,
    mpsc::UnboundedSender<AgentRequest>,
    mpsc::UnboundedReceiver<AgentResponse>,
    String, // session_id
    String, // model
    String, // identity_name
    Arc<SkillRegistry>,
)> {
    let identity = prompt::load_identity(&ctx.home_dir);
    if let Some(s) = session_id
        && s.is_empty()
    {
        anyhow::bail!("--session-id value must not be empty");
    }
    let reusing_session = session_id.is_some();
    let session_id = session_id
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    // Validate session ownership if reusing an existing session
    if reusing_session
        && let Ok(Some(existing)) = ctx.async_db.get_session(&session_id).await
        && existing.agent_id != ctx.async_db.agent_id()
    {
        anyhow::bail!(
            "Session '{}' belongs to agent '{}', not '{}'",
            session_id,
            existing.agent_id,
            ctx.async_db.agent_id()
        );
    }
    if let Err(e) = ctx
        .async_db
        .create_session(&session_id, ctx.async_db.agent_id(), "cli")
        .await
    {
        tracing::warn!(error = %e, "failed to create session");
    }
    let mut tool_registry = tools::default_tools();
    for tool in tools::management_tools_if_needed(
        &ctx.global_home,
        &ctx.settings,
        reqwest::Client::new(),
        ctx.github_app.clone(),
    ) {
        tool_registry.register(tool);
    }
    let tool_registry = Arc::new(tool_registry);
    let skills_dir = ctx.home_dir.join("skills");
    // Migrate legacy .disabled marker files to DB (one-shot, idempotent).
    if let Err(e) = mika_agent::skills::migrate_disabled_markers_async(
        &skills_dir,
        &ctx.async_db,
        &ctx.async_db.agent_id,
    )
    .await
    {
        tracing::warn!(error = %e, "failed to migrate .disabled markers");
    }
    let mut skill_registry = SkillRegistry::from_dir(&skills_dir);
    if let Some(ref allowlist) = identity.skills.allowlist {
        skill_registry.apply_identity_allowlist(allowlist);
    }
    if let Ok(overrides) = ctx
        .async_db
        .get_skill_overrides(&ctx.async_db.agent_id)
        .await
    {
        skill_registry.apply_overrides(&overrides);
    }
    skill_registry.validate_loaded();
    skill_registry.log_summary();
    let skill_registry = Arc::new(skill_registry);
    let skills_dirty = Arc::new(AtomicBool::new(false));
    let embedding_client = ctx.settings.make_embedding_client();
    let message_sender = crate::init::make_message_sender(
        &ctx.settings,
        &ctx.async_db,
        http_client,
        ctx.async_db.agent_id(),
    );

    let brave_api_key = ctx
        .settings
        .brave_api_key
        .as_ref()
        .map(|s| s.expose_secret().to_string());
    let github_token = ctx.settings.agent_github_token().map(String::from);

    // Connect to MCP servers
    let mcp_manager = crate::init::connect_mcp(&ctx.home_dir).await;

    // Set up task engine for background tasks (reminders, reflection)
    let dispatcher = Arc::new(TaskDispatcher {
        db: ctx.async_db.clone(),
        llm: ctx.llm.clone(),
        tools: tool_registry.clone(),
        skills: skill_registry.clone(),
        home_dir: ctx.home_dir.clone(),
        message_sender: message_sender.clone(),
        embedding_client: embedding_client.clone(),
        brave_api_key: brave_api_key.clone(),
        github_token: github_token.clone(),
        github_app: ctx.github_app.clone(),
        skills_dirty: skills_dirty.clone(),
        // CLI mode: no agent_lock passed. The task engine may run heartbeat/reflection
        // concurrently with user message processing (both use the same AsyncDatabase,
        // which serializes at the DB thread level). Concurrent Claude API calls are
        // accepted in CLI mode; rate-limit errors are handled by the Claude client.
        agent_lock: None,
        // CLI mode: the TUI's poll_callback_tasks() handles callback delivery with
        // session-scoped queries and atomic claiming. Skip engine dispatch to prevent
        // the engine from stealing callbacks. See #264.
        cli_mode: true,
        settings: ctx.settings.clone(),
        pr_reviews_posted: None, // CLI mode: no session-scoped dedup needed
    });
    let task_engine = Arc::new(tokio::sync::Mutex::new(TaskEngine::new(
        ctx.async_db.clone(),
        dispatcher,
    )));

    // Register recurring built-in tasks and run startup recovery
    task_engine::ensure_recurring_task(
        &ctx.async_db,
        "heartbeat",
        "0 0 * * * *",
        r#"{"trigger":"heartbeat"}"#,
    )
    .await;
    if let Some(cron) = task_engine::reflection_cron_for_agent(&ctx.home_dir, &ctx.async_db).await {
        task_engine::ensure_recurring_task(
            &ctx.async_db,
            "reflection",
            &cron,
            r#"{"trigger":"reflection"}"#,
        )
        .await;
    } else if let Err(e) = ctx
        .async_db
        .cancel_recurring_task_by_label("reflection")
        .await
    {
        tracing::warn!(error = %e, "failed to cancel stale reflection task");
    }
    {
        let mut eng = task_engine.lock().await;
        if let Err(e) = eng.startup_recovery().await {
            tracing::warn!(error = %e, "task engine startup recovery failed");
        }
    }
    // Prune completed tasks older than 30 days to prevent unbounded DB growth
    task_engine::prune_old_tasks(&ctx.async_db).await;
    let poller_handle = TaskEngine::spawn_tick_loop(task_engine);

    let (user_tx, mut user_rx) = mpsc::unbounded_channel::<AgentRequest>();
    let (agent_tx, agent_rx) = mpsc::unbounded_channel::<AgentResponse>();

    // Supervisor plumbing (mika#850). The supervisor task awaits the worker
    // JoinHandle and emits `AgentResponse::WorkerCrashed` on panic or unexpected
    // exit. `quit_received` distinguishes clean Quit-induced shutdown from
    // worker_loop_exited (e.g., user_rx closed without a Quit), so the
    // supervisor stays silent on intentional teardown.
    let supervisor_tx = agent_tx.clone();
    let quit_received = Arc::new(AtomicBool::new(false));
    let worker_quit_flag = quit_received.clone();
    let last_message_at: Arc<StdMutex<Option<Instant>>> = Arc::new(StdMutex::new(None));
    let worker_last_message_at = last_message_at.clone();
    let supervisor_agent_id = ctx.async_db.agent_id().to_string();
    let supervisor_session_id = session_id.clone();

    let worker_db = ctx.async_db.clone();
    let mut worker_llm = ctx.llm.clone();
    let mut worker_settings = ctx.settings.clone();
    let worker_tools = tool_registry.clone();
    let mut worker_skills = skill_registry.clone();
    let worker_home = ctx.home_dir.clone();
    let mut worker_session = session_id.clone();
    let worker_embedding = embedding_client;
    let worker_brave_key = brave_api_key;
    let worker_github_token = github_token;
    let worker_github_app = ctx.github_app.clone();
    let worker_sender = message_sender;
    let worker_dirty = skills_dirty.clone();
    let mut worker_mcp = mcp_manager;
    let worker_global_home = ctx.global_home.clone();
    let worker_handle = tokio::spawn(async move {
        while let Some(request) = user_rx.recv().await {
            match request {
                AgentRequest::Message {
                    text,
                    images,
                    thinking_budget,
                } => {
                    // Record the dispatch timestamp so the supervisor can report
                    // seconds_since_message_persist on crash (mika#850 S5).
                    if let Ok(mut guard) = worker_last_message_at.lock() {
                        *guard = Some(Instant::now());
                    }
                    // Hot-reload skills if the dirty flag was set by a previous turn
                    let mut skills_reloaded = false;
                    if worker_dirty.load(Ordering::Acquire) {
                        worker_dirty.store(false, Ordering::Release);
                        let mut registry = SkillRegistry::from_dir(&worker_home.join("skills"));
                        let reload_identity = mika_agent::prompt::load_identity(&worker_home);
                        if let Some(ref allowlist) = reload_identity.skills.allowlist {
                            registry.apply_identity_allowlist(allowlist);
                        }
                        if let Ok(overrides) =
                            worker_db.get_skill_overrides(&worker_db.agent_id).await
                        {
                            registry.apply_overrides(&overrides);
                        }
                        registry.log_summary();
                        worker_skills = Arc::new(registry);
                        skills_reloaded = true;
                    }

                    let thinking = thinking_budget.map(|budget| ThinkingConfig::Enabled {
                        budget_tokens: budget,
                    });

                    // Convert ImageAttachments to provider-agnostic LlmImage
                    let image_sources: Vec<LlmImage> = images
                        .into_iter()
                        .map(|img| LlmImage {
                            media_type: img.media_type,
                            data: img.base64_data,
                        })
                        .collect();

                    let is_onboarding = check_onboarding(&worker_db).await;
                    let result = agent::run_agent(&AgentParams {
                        db: &worker_db,
                        llm: worker_llm.as_ref(),
                        tools: &worker_tools,
                        skills: &worker_skills,
                        user_message: &text,
                        channel_type: "cli",
                        session_id: &worker_session,
                        home_dir: &worker_home,
                        is_onboarding,
                        message_sender: worker_sender.clone(),
                        skip_compaction: false,
                        embedding_client: worker_embedding.as_ref(),
                        thinking,
                        user_images: &image_sources,
                        brave_api_key: worker_brave_key.as_deref(),
                        github_token: worker_github_token.as_deref(),
                        github_app: worker_github_app.as_deref(),
                        skills_dirty: &worker_dirty,
                        mcp_manager: worker_mcp.as_ref(),
                        global_home_dir: Some(&worker_global_home),
                        is_callback_turn: false,
                        settings: Some(&worker_settings),
                        trace_id: None,
                        correlated_task_id: None,
                        internal: false,
                        pr_reviews_posted: None, // CLI mode: no session-scoped dedup needed
                    })
                    .await;

                    // If the dirty flag is set after this turn, rebuild now so we
                    // can send the updated registry to the TUI for autocomplete.
                    if worker_dirty.load(Ordering::Acquire) {
                        worker_dirty.store(false, Ordering::Release);
                        let mut registry = SkillRegistry::from_dir(&worker_home.join("skills"));
                        let reload_identity2 = mika_agent::prompt::load_identity(&worker_home);
                        if let Some(ref allowlist) = reload_identity2.skills.allowlist {
                            registry.apply_identity_allowlist(allowlist);
                        }
                        if let Ok(overrides) =
                            worker_db.get_skill_overrides(&worker_db.agent_id).await
                        {
                            registry.apply_overrides(&overrides);
                        }
                        registry.log_summary();
                        worker_skills = Arc::new(registry);
                        skills_reloaded = true;
                    }

                    let updated_skills = if skills_reloaded {
                        Some(worker_skills.clone())
                    } else {
                        None
                    };

                    let response = match result {
                        Ok(output) => AgentResponse::Reply {
                            content: output.text.unwrap_or_default(),
                            is_error: false,
                            thinking: output.thinking,
                            input_tokens: output.usage.as_ref().map(|u| u.input_tokens as u32),
                            updated_skills,
                        },
                        Err(e) => AgentResponse::Reply {
                            content: format!("{e}"),
                            is_error: true,
                            thinking: None,
                            input_tokens: None,
                            updated_skills,
                        },
                    };
                    if agent_tx.send(response).is_err() {
                        break;
                    }
                }
                AgentRequest::CallbackResult {
                    task_id,
                    label,
                    result,
                    trace_id,
                    original_status,
                    parent_task_id,
                } => {
                    if let Ok(mut guard) = worker_last_message_at.lock() {
                        *guard = Some(Instant::now());
                    }
                    // Save the callback result as a tool_result message
                    let metadata = serde_json::json!({
                        "callback_task_id": task_id,
                        "label": label,
                        "parent_task_id": parent_task_id,
                    })
                    .to_string();
                    let _ = worker_db
                        .save_message_with_metadata(
                            &worker_session,
                            "tool_result",
                            &result,
                            Some(&metadata),
                            trace_id.as_deref(),
                            false,
                        )
                        .await;

                    // Build framing message for the agent
                    let is_failed = original_status == "failed";
                    let framing = agent::build_callback_trigger_context(
                        &label,
                        &task_id,
                        parent_task_id.as_deref(),
                        &result,
                        is_failed,
                    );

                    let is_onboarding = check_onboarding(&worker_db).await;
                    let callback_result = agent::run_agent(&AgentParams {
                        db: &worker_db,
                        llm: worker_llm.as_ref(),
                        tools: &worker_tools,
                        skills: &worker_skills,
                        user_message: &framing,
                        channel_type: "cli",
                        session_id: &worker_session,
                        home_dir: &worker_home,
                        is_onboarding,
                        message_sender: worker_sender.clone(),
                        skip_compaction: false,
                        embedding_client: worker_embedding.as_ref(),
                        thinking: None,
                        user_images: &[],
                        brave_api_key: worker_brave_key.as_deref(),
                        github_token: worker_github_token.as_deref(),
                        github_app: worker_github_app.as_deref(),
                        skills_dirty: &worker_dirty,
                        mcp_manager: worker_mcp.as_ref(),
                        global_home_dir: Some(&worker_global_home),
                        is_callback_turn: true,
                        settings: Some(&worker_settings),
                        trace_id: trace_id.clone(),
                        correlated_task_id: None,
                        internal: false,
                        pr_reviews_posted: None, // CLI mode: no session-scoped dedup needed
                    })
                    .await;

                    let response = match callback_result {
                        Ok(output) => AgentResponse::Reply {
                            content: output.text.unwrap_or_default(),
                            is_error: false,
                            thinking: output.thinking,
                            input_tokens: output.usage.as_ref().map(|u| u.input_tokens as u32),
                            updated_skills: None,
                        },
                        Err(e) => {
                            // Unclaim the task so the next poll cycle can retry it.
                            // mark_task_delivered set status to 'delivered'; reset to original
                            // status so get_undelivered_callback_tasks will pick it up again.
                            if let Err(reset_err) = worker_db
                                .update_task_status(&task_id, &original_status)
                                .await
                            {
                                tracing::warn!(
                                    task_id = %task_id,
                                    error = %reset_err,
                                    "failed to unclaim callback task after agent error"
                                );
                            }
                            AgentResponse::Reply {
                                content: format!("Failed to process callback result: {e}"),
                                is_error: true,
                                thinking: None,
                                input_tokens: None,
                                updated_skills: None,
                            }
                        }
                    };
                    if agent_tx.send(response).is_err() {
                        break;
                    }
                }
                AgentRequest::SetModel { model } => {
                    // Recreate the LLM provider with the new model.
                    // Parse provider/model format if present, otherwise set on active provider.
                    let mut updated_settings = worker_settings.clone();
                    if let Some(slash_pos) = model.find('/') {
                        let prefix = &model[..slash_pos];
                        if let Ok(provider) = prefix.parse::<mika_common::llm::ProviderKind>() {
                            let model_name = model[slash_pos + 1..].to_string();
                            updated_settings.llm_provider = provider;
                            updated_settings.set_provider_model(provider, Some(model_name));
                        } else {
                            let provider = updated_settings.llm_provider;
                            updated_settings.set_provider_model(provider, Some(model.clone()));
                        }
                    } else {
                        let provider = updated_settings.llm_provider;
                        updated_settings.set_provider_model(provider, Some(model.clone()));
                    }
                    if let Ok(new_llm) = updated_settings.make_llm_provider() {
                        worker_llm = new_llm;
                        worker_settings = updated_settings;
                    }
                }
                AgentRequest::NewSession { session_id } => {
                    worker_session = session_id;
                }
                AgentRequest::Quit => {
                    worker_quit_flag.store(true, Ordering::Release);
                    break;
                }
            }
        }
        // Gracefully shut down MCP server connections
        if let Some(mcp) = worker_mcp.take() {
            mcp.shutdown().await;
        }
    });

    let worker_abort = worker_handle.abort_handle();

    let handle = spawn_worker_supervisor(
        worker_handle,
        supervisor_tx,
        quit_received,
        last_message_at,
        supervisor_agent_id,
        supervisor_session_id,
    );

    let model = ctx.settings.active_model_display();
    let identity_name = identity.name.clone();

    let worker = AgentWorker {
        handle,
        worker_abort,
        poller_handle,
        _ctx: ctx,
    };

    Ok((
        worker,
        user_tx,
        agent_rx,
        session_id,
        model,
        identity_name,
        skill_registry,
    ))
}

pub async fn run(
    agent_name: &str,
    session: Option<&str>,
    model_override: Option<&str>,
) -> Result<()> {
    let mut ctx = init::init_for_agent(agent_name)?;

    if let Some(model) = model_override {
        ctx.override_model(model)?;
    }

    let http_client = reqwest::Client::new();
    let (mut worker, user_tx, agent_rx, session_id, model, identity_name, skill_registry) =
        spawn_agent_worker(ctx, agent_name, &http_client, session).await?;

    // Build app with shared resources
    let mut app = App::new(
        user_tx,
        agent_rx,
        session_id,
        model,
        identity_name,
        worker._ctx.async_db.clone(),
        worker._ctx.llm.clone(),
        worker._ctx.home_dir.clone(),
        skill_registry,
        agent_name.to_string(),
        worker._ctx.global_home.clone(),
        worker._ctx.settings.llm_provider,
    );

    // Load recent conversation history so the user sees prior messages on restart.
    // In inbox mode (default), internal messages are filtered at the DB level.
    if let Ok(history) = worker
        ._ctx
        .async_db
        .load_recent_messages_filtered(20, app.inbox_mode)
        .await
    {
        for msg in history {
            if let Some(chat_msg) = session_message_to_chat_message(&msg) {
                app.messages.push(chat_msg);
            }
        }
    }

    // Load persisted thinking level ("off" resolves to None → stays default)
    app.load_thinking_level().await;

    // Surface skipped skills as a startup warning so the user knows
    let skipped_skills = app.skills.skipped();
    if !skipped_skills.is_empty() {
        use std::fmt::Write;
        let count = skipped_skills.len();
        let mut warning = format!("\u{26a0} {count} skill(s) skipped at startup:\n");
        let max_display = 5;
        for entry in skipped_skills.iter().take(max_display) {
            let _ = writeln!(warning, "  \u{2022} {}: {}", entry.name, entry.reason);
        }
        if count > max_display {
            let _ = writeln!(
                warning,
                "  ... and {} more. Run `mika skills validate` for details.",
                count - max_display
            );
        }
        app.messages.push(ChatMessage {
            role: ChatRole::System,
            content: warning.trim_end().to_string(),
            rendered: None,
            channel: None,
        });
    }

    // Surface skills with validation warnings (#530)
    let validation_warnings = app.skills.validated_warnings();
    if !validation_warnings.is_empty() {
        use std::fmt::Write;
        let count = validation_warnings.len();
        let mut warning = format!("\u{26a0} {count} skill(s) loaded with validation warnings:\n");
        let max_display = 5;
        for entry in validation_warnings.iter().take(max_display) {
            // Show skill name + first diagnostic message for brevity
            let first_msg = entry
                .diagnostics
                .first()
                .map(|d| d.message.as_str())
                .unwrap_or("unknown issue");
            let _ = writeln!(warning, "  \u{2022} {}: {}", entry.skill_name, first_msg);
        }
        if count > max_display {
            let _ = writeln!(
                warning,
                "  ... and {} more. Run `mika skills validate` for details.",
                count - max_display
            );
        }
        app.messages.push(ChatMessage {
            role: ChatRole::System,
            content: warning.trim_end().to_string(),
            rendered: None,
            channel: None,
        });
    }

    // Initialize cross-channel polling watermark
    app.last_seen_msg_id = worker._ctx.async_db.max_message_id().await.unwrap_or(0);

    // One-time dashboard status check at startup (no periodic polling).
    app.dashboard_running = crate::commands::dashboard::is_dashboard_running().await;

    // Install panic hook that restores terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        original_hook(info);
    }));

    // Enter TUI mode
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Event reader (30ms tick rate for responsive progressive reveal)
    let mut events = EventReader::new(Duration::from_millis(30));

    // Main loop
    loop {
        if app.needs_redraw {
            terminal.draw(|f| ui::draw(f, &mut app))?;
            app.needs_redraw = false;
        }

        match events.next().await {
            Some(AppEvent::Key(key)) => {
                input::handle_key(&mut app, key);
                app.needs_redraw = true;
            }
            Some(AppEvent::Mouse(mouse)) => {
                input::handle_mouse(&mut app, mouse);
                app.needs_redraw = true;
            }
            Some(AppEvent::Paste(text)) => {
                input::handle_paste(&mut app, &text);
                app.needs_redraw = true;
            }
            Some(AppEvent::Tick) => {
                app.tick().await;
            }
            Some(AppEvent::Resize) => {
                app.selection_state.clear();
                app.needs_redraw = true;
            }
            None => break,
        }

        // Handle agent switch
        if let Some(target_name) = app.pending_switch.take() {
            // End the old session before switching agents
            if let Err(e) = worker._ctx.async_db.end_session(&app.session_id).await {
                tracing::warn!(error = %e, "failed to end session on agent switch");
            }

            // /agent-switch already sent AgentRequest::Quit via handle_switch;
            // give the worker a short window to exit cleanly so the post-loop
            // `mcp.shutdown().await` block runs (and the supervisor sees
            // `Ok(()) + quit_received=true` and stays silent). Abort only as a
            // fallback if the worker is wedged.
            worker.poller_handle.abort();
            let old_handle = std::mem::replace(
                &mut worker.handle,
                tokio::spawn(async {}), // placeholder
            );
            if tokio::time::timeout(Duration::from_secs(2), old_handle)
                .await
                .is_err()
            {
                tracing::warn!(
                    event = "supervisor_drain_timeout",
                    site = "agent_switch",
                    "old worker did not exit within 2s of Quit; aborting"
                );
                worker.worker_abort.abort();
            }

            // Initialize the new agent
            match init::init_for_agent(&target_name) {
                Ok(new_ctx) => {
                    match spawn_agent_worker(new_ctx, &target_name, &http_client, None).await {
                        Ok((
                            new_worker,
                            new_tx,
                            new_rx,
                            new_session,
                            new_model,
                            new_identity,
                            new_skills,
                        )) => {
                            // Update app fields
                            app.agent_tx = new_tx;
                            app.agent_rx = new_rx;
                            app.session_id = new_session;
                            app.model = new_model.clone();
                            app.identity_name = new_identity;
                            app.db = new_worker._ctx.async_db.clone();
                            app.llm = new_worker._ctx.llm.clone();
                            app.home_dir = new_worker._ctx.home_dir.clone();
                            app.skills = new_skills;
                            app.agent_name = target_name.clone();
                            app.history = crate::tui::app::InputHistory::load(&app.home_dir);
                            // The new agent is healthy; drop any residual flags
                            // from a crash on the agent we just left.
                            app.worker_crashed = None;
                            app.pending_restart = false;
                            app.status = AgentStatus::Idle;

                            worker = new_worker;

                            // Load persisted thinking level from new agent's DB
                            app.load_thinking_level().await;

                            app.messages.push(ChatMessage {
                                role: ChatRole::System,
                                content: format!(
                                    "Switched to agent '{target_name}' ({new_model})."
                                ),
                                rendered: None,
                                channel: None,
                            });
                        }
                        Err(e) => {
                            app.messages.push(ChatMessage {
                                role: ChatRole::System,
                                content: format!("Failed to switch agent: {e}"),
                                rendered: None,
                                channel: None,
                            });
                        }
                    }
                }
                Err(e) => {
                    app.messages.push(ChatMessage {
                        role: ChatRole::System,
                        content: format!("Failed to switch agent: {e}"),
                        rendered: None,
                        channel: None,
                    });
                }
            }
            app.scroll_offset = 0;
            app.needs_redraw = true;
        }

        // Handle /restart (mika#850): respawn the agent worker in-place after
        // a WorkerCrashed event. Reuses the current session_id so the operator
        // can keep the conversation context; the lost in-flight prompt must be
        // re-typed by the operator (no replay).
        if app.pending_restart {
            app.pending_restart = false;
            let agent_name = app.agent_name.clone();
            let session_id_for_restart = app.session_id.clone();

            // Tear down the crashed worker (idempotent on already-finished tasks).
            worker.poller_handle.abort();
            worker.worker_abort.abort();
            let old_handle = std::mem::replace(
                &mut worker.handle,
                tokio::spawn(async {}), // placeholder
            );
            let _ = tokio::time::timeout(Duration::from_secs(2), old_handle).await;

            match init::init_for_agent(&agent_name) {
                Ok(new_ctx) => {
                    match spawn_agent_worker(
                        new_ctx,
                        &agent_name,
                        &http_client,
                        Some(&session_id_for_restart),
                    )
                    .await
                    {
                        Ok((
                            new_worker,
                            new_tx,
                            new_rx,
                            new_session,
                            new_model,
                            new_identity,
                            new_skills,
                        )) => {
                            app.agent_tx = new_tx;
                            app.agent_rx = new_rx;
                            app.session_id = new_session;
                            app.model = new_model;
                            app.identity_name = new_identity;
                            app.db = new_worker._ctx.async_db.clone();
                            app.llm = new_worker._ctx.llm.clone();
                            app.home_dir = new_worker._ctx.home_dir.clone();
                            app.skills = new_skills;
                            worker = new_worker;
                            app.worker_crashed = None;
                            app.status = AgentStatus::Idle;
                            app.messages.push(ChatMessage {
                                role: ChatRole::System,
                                content:
                                    "Agent worker restarted. Re-send the last prompt if needed."
                                        .to_string(),
                                rendered: None,
                                channel: None,
                            });
                        }
                        Err(e) => {
                            app.messages.push(ChatMessage {
                                role: ChatRole::System,
                                content: format!("Failed to restart agent worker: {e}"),
                                rendered: None,
                                channel: None,
                            });
                        }
                    }
                }
                Err(e) => {
                    app.messages.push(ChatMessage {
                        role: ChatRole::System,
                        content: format!("Failed to restart agent worker: {e}"),
                        rendered: None,
                        channel: None,
                    });
                }
            }
            app.needs_redraw = true;
        }

        if app.should_quit {
            let _ = app.agent_tx.send(AgentRequest::Quit);
            break;
        }
    }

    // Restore terminal
    restore_terminal()?;

    // Shut down event reader thread
    events.shutdown();

    // End the session so the dashboard shows duration instead of "ongoing"
    if let Err(e) = worker._ctx.async_db.end_session(&app.session_id).await {
        tracing::warn!(error = %e, "failed to end session");
    }

    // Stop the reminder poller
    worker.poller_handle.abort();

    // App-exit already sent AgentRequest::Quit; await the supervisor briefly so
    // the worker's post-loop `mcp.shutdown()` block runs. Abort only on timeout
    // so a wedged worker can't hang the CLI on exit.
    match tokio::time::timeout(Duration::from_secs(2), worker.handle).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            eprintln!("Agent worker supervisor error: {e}");
        }
        Err(_) => {
            tracing::warn!(
                event = "supervisor_drain_timeout",
                site = "app_exit",
                "worker did not exit within 2s of Quit; aborting"
            );
            worker.worker_abort.abort();
        }
    }

    // Database shutdown happens automatically via Drop on worker._ctx
    Ok(())
}

/// Run the TUI in team mode. Each user message is sent as a goal to `run_team()`.
pub async fn run_team(team_name: &str, global_home: &Path, run_id: Option<&str>) -> Result<()> {
    let team_dir = team::team_dir(global_home, team_name);

    let (team_tx, mut team_rx_worker) = mpsc::unbounded_channel::<TeamRequest>();
    let (response_tx, response_rx) = mpsc::unbounded_channel::<TeamEvent>();

    // Use the shared container DB for team conversation persistence
    let db_path = mika_common::home::container_db_path(global_home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    let team_db = mika_agent::async_db::AsyncDatabase::open(&db_path)?;

    // Spawn team worker
    let worker_home = global_home.to_path_buf();
    let worker_name = team_name.to_string();
    let worker_team_db = team_db.clone();
    let worker_run_id = run_id.map(|s| s.to_string());
    let team_worker_handle = tokio::spawn(async move {
        let settings = match Settings::load(&worker_home) {
            Ok(s) => s,
            Err(e) => {
                let _ = response_tx.send(TeamEvent::RunFailed(format!(
                    "Failed to load settings: {e}"
                )));
                return;
            }
        };

        let github_app = mika_common::github_app::GitHubApp::from_settings(&settings);
        while let Some(request) = team_rx_worker.recv().await {
            match request {
                TeamRequest::Goal(goal) => {
                    let tx = response_tx.clone();
                    let callback = move |event: TeamEvent| {
                        let _ = tx.send(event);
                    };

                    match mika_agent::teams::run_team(
                        &worker_name,
                        &goal,
                        &worker_home,
                        &settings,
                        Some(Box::new(callback)),
                        worker_team_db.clone(),
                        worker_run_id.as_deref(),
                        github_app.clone(),
                        None, // CLI: no AppState for session-scoped dedup (#821)
                    )
                    .await
                    {
                        Ok(run) => {
                            if let RunStatus::Failed(reason) = run.status {
                                let _ = response_tx.send(TeamEvent::RunFailed(reason));
                            } else {
                                let deliverable = run.deliverable.unwrap_or_default();
                                let _ = response_tx.send(TeamEvent::Deliverable(deliverable));
                            }
                        }
                        Err(e) => {
                            let _ = response_tx.send(TeamEvent::RunFailed(format!("{e}")));
                        }
                    }
                }
                TeamRequest::Quit => break,
            }
        }
    });

    // Load previous run context if a reference run was provided
    let (previous_run, run_warning) = if let Some(ref_id) = run_id {
        match team_db.get_team_run_summary(ref_id).await {
            Ok(Some(summary)) => {
                let warning = if summary.run.team_name != team_name {
                    tracing::warn!(
                        run_id = ref_id,
                        expected_team = team_name,
                        actual_team = summary.run.team_name,
                        "Referenced run belongs to a different team"
                    );
                    Some(format!(
                        "Warning: Referenced run belongs to team '{}', not '{}'",
                        summary.run.team_name, team_name
                    ))
                } else {
                    None
                };
                (Some(summary), warning)
            }
            Ok(None) => {
                tracing::warn!(run_id = ref_id, "Referenced team run not found in database");
                (
                    None,
                    Some(format!(
                        "Warning: Referenced run {} not found. Starting without previous run context.",
                        ref_id
                    )),
                )
            }
            Err(e) => {
                tracing::warn!(run_id = ref_id, error = %e, "Failed to load previous run summary");
                (None, None)
            }
        }
    } else {
        (None, None)
    };

    // Build app in team mode
    let mut app = App::new_team(
        team_tx,
        response_rx,
        team_name,
        team_dir.clone(),
        global_home.to_path_buf(),
        team_db,
        previous_run,
    );

    // Inject warning message if there was a run reference issue
    if let Some(warning) = run_warning {
        app.messages.push(ChatMessage {
            role: ChatRole::System,
            content: warning,
            rendered: None,
            channel: None,
        });
    }

    // Team mode: skip history loading. Team conversations are goal-driven —
    // each goal triggers a full team run. The shared container DB contains
    // regular agent messages that should not appear in the team TUI.

    // One-time dashboard status check at startup (no periodic polling).
    app.dashboard_running = crate::commands::dashboard::is_dashboard_running().await;

    // Install panic hook that restores terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        original_hook(info);
    }));

    // Enter TUI mode
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Event reader (30ms tick rate for responsive progressive reveal)
    let mut events = EventReader::new(Duration::from_millis(30));

    // Main loop
    loop {
        if app.needs_redraw {
            terminal.draw(|f| ui::draw(f, &mut app))?;
            app.needs_redraw = false;
        }

        match events.next().await {
            Some(AppEvent::Key(key)) => {
                input::handle_key(&mut app, key);
                app.needs_redraw = true;
            }
            Some(AppEvent::Mouse(mouse)) => {
                input::handle_mouse(&mut app, mouse);
                app.needs_redraw = true;
            }
            Some(AppEvent::Paste(text)) => {
                input::handle_paste(&mut app, &text);
                app.needs_redraw = true;
            }
            Some(AppEvent::Tick) => {
                app.tick().await;
            }
            Some(AppEvent::Resize) => {
                app.selection_state.clear();
                app.needs_redraw = true;
            }
            None => break,
        }

        if app.should_quit {
            if let Some(ref tx) = app.team_tx {
                let _ = tx.send(TeamRequest::Quit);
            }
            break;
        }
    }

    // Shut down team DB (ensures pending writes complete)
    app.db.shutdown();

    // Restore terminal
    restore_terminal()?;

    // Shut down event reader thread
    events.shutdown();

    // Give team worker a brief window to shut down gracefully before aborting
    let _ = tokio::time::timeout(Duration::from_secs(2), team_worker_handle).await;

    Ok(())
}

fn restore_terminal() -> Result<()> {
    terminal::disable_raw_mode()?;
    execute!(
        io::stdout(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste
    )?;
    Ok(())
}

/// Spawn a supervisor task that awaits the agent worker `JoinHandle` and forwards
/// crash signals as `AgentResponse::WorkerCrashed` on `supervisor_tx`.
///
/// Stays silent on intentional shutdown (Quit-induced clean exit, or
/// `AbortHandle::abort` from the run loop). Emits on panic, join error, or
/// `Ok(())` exits that weren't preceded by an `AgentRequest::Quit` (which would
/// indicate a worker-loop bug — the load-bearing failure shape from mika#850).
fn spawn_worker_supervisor(
    worker_handle: JoinHandle<()>,
    supervisor_tx: mpsc::UnboundedSender<AgentResponse>,
    quit_received: Arc<AtomicBool>,
    last_message_at: Arc<StdMutex<Option<Instant>>>,
    agent_id: String,
    session_id: String,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let outcome = worker_handle.await;
        let reason = match outcome {
            Ok(()) => {
                if quit_received.load(Ordering::Acquire) {
                    return;
                }
                "worker_loop_exited_without_quit".to_string()
            }
            Err(e) if e.is_cancelled() => return,
            Err(e) if e.is_panic() => {
                let payload = e.into_panic();
                let msg = payload
                    .downcast_ref::<&str>()
                    .map(|s| (*s).to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "<non-string panic>".to_string());
                format!("panic: {msg}")
            }
            Err(e) => format!("join_error: {e}"),
        };

        let elapsed_secs = last_message_at
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|t| t.elapsed().as_secs()));

        tracing::error!(
            target: "mika::otel",
            event = "agent_worker_silenced",
            agent_id = %agent_id,
            session_id = %session_id,
            reason = %reason,
            seconds_since_dispatch_start = ?elapsed_secs,
            "TUI agent worker entered failure state; pending prompt dropped"
        );

        let _ = supervisor_tx.send(AgentResponse::WorkerCrashed { reason });
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a no-op `quit_received` and timestamp store for tests
    /// that don't care about them.
    fn supervisor_test_state() -> (Arc<AtomicBool>, Arc<StdMutex<Option<Instant>>>) {
        (
            Arc::new(AtomicBool::new(false)),
            Arc::new(StdMutex::new(None)),
        )
    }

    /// Drive the supervisor with a worker that panics. Expect a `WorkerCrashed`
    /// event on `agent_rx` within 2 seconds, carrying the panic message in `reason`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn supervisor_emits_worker_crashed_on_panic() {
        let (tx, mut rx) = mpsc::unbounded_channel::<AgentResponse>();
        let (quit, last) = supervisor_test_state();
        let worker = tokio::spawn(async {
            panic!("synthetic panic for mika#850 supervision test");
        });
        let _sup = spawn_worker_supervisor(
            worker,
            tx,
            quit,
            last,
            "test-agent".to_string(),
            "test-session".to_string(),
        );

        let recv = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("supervisor did not emit within 2s window");
        let response = recv.expect("agent_rx closed before receiving any event");
        match response {
            AgentResponse::WorkerCrashed { reason } => {
                assert!(
                    reason.starts_with("panic:"),
                    "expected panic-classified reason, got {reason:?}"
                );
                assert!(
                    reason.contains("synthetic panic for mika#850"),
                    "expected panic message in reason, got {reason:?}"
                );
            }
            AgentResponse::Reply { .. } => {
                panic!("expected WorkerCrashed, got Reply");
            }
        }
    }

    /// Worker that exits `Ok(())` without `quit_received = true` is treated as
    /// an unexpected loop exit and surfaces a distinct, non-panic reason.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn supervisor_emits_loop_exit_when_no_quit_signal() {
        let (tx, mut rx) = mpsc::unbounded_channel::<AgentResponse>();
        let (quit, last) = supervisor_test_state();
        let worker = tokio::spawn(async {
            // Exit cleanly without anyone setting quit_received.
        });
        let _sup = spawn_worker_supervisor(
            worker,
            tx,
            quit,
            last,
            "test-agent".to_string(),
            "test-session".to_string(),
        );

        let response = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("supervisor did not emit within 2s window")
            .expect("agent_rx closed before receiving any event");
        match response {
            AgentResponse::WorkerCrashed { reason } => {
                assert_eq!(reason, "worker_loop_exited_without_quit");
            }
            AgentResponse::Reply { .. } => panic!("expected WorkerCrashed, got Reply"),
        }
    }

    /// Clean Quit-induced shutdown: supervisor must stay silent so the TUI
    /// doesn't show a crash banner on `/exit` or agent switch.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn supervisor_stays_silent_on_clean_quit() {
        let (tx, mut rx) = mpsc::unbounded_channel::<AgentResponse>();
        let (quit, last) = supervisor_test_state();
        quit.store(true, Ordering::Release);
        let worker = tokio::spawn(async {});
        let sup = spawn_worker_supervisor(
            worker,
            tx,
            quit,
            last,
            "test-agent".to_string(),
            "test-session".to_string(),
        );
        sup.await.expect("supervisor task should complete cleanly");
        assert!(
            rx.try_recv().is_err(),
            "supervisor must not emit on clean Quit"
        );
    }

    /// Worker aborted via its `AbortHandle` (the agent-switch / app-exit path):
    /// supervisor must stay silent (cancelled JoinError is intentional teardown).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn supervisor_stays_silent_on_abort() {
        let (tx, mut rx) = mpsc::unbounded_channel::<AgentResponse>();
        let (quit, last) = supervisor_test_state();
        let worker = tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        });
        let abort = worker.abort_handle();
        let sup = spawn_worker_supervisor(
            worker,
            tx,
            quit,
            last,
            "test-agent".to_string(),
            "test-session".to_string(),
        );
        abort.abort();
        sup.await.expect("supervisor task should complete cleanly");
        assert!(
            rx.try_recv().is_err(),
            "supervisor must not emit on aborted worker"
        );
    }
}

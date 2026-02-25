use anyhow::Result;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{event::DisableMouseCapture, execute};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::init::{self, AppContext};
use crate::tui::app::{AgentRequest, AgentResponse, App, ChatMessage, ChatRole};
use crate::tui::event::{AppEvent, EventReader};
use crate::tui::input;
use crate::tui::ui;
use mika_agent::agent::{self, AgentParams, check_onboarding};
use mika_agent::prompt;
use mika_agent::scheduler::ReminderScheduler;
use mika_agent::skills::SkillRegistry;
use mika_agent::tools;

/// Holds the agent worker task handle and the AppContext (for DB shutdown).
struct AgentWorker {
    handle: JoinHandle<()>,
    _ctx: AppContext,
}

/// Spawn the agent worker task. Returns the worker, channels, and context info needed for App.
async fn spawn_agent_worker(
    ctx: AppContext,
    _agent_name: &str,
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
    let session_id = Uuid::new_v4().to_string();
    let tool_registry = Arc::new(tools::default_tools());
    let skill_registry = Arc::new(SkillRegistry::from_dir(&ctx.home_dir.join("skills")));
    let embedding_client = ctx.settings.make_embedding_client();

    // Recover reminders on startup
    {
        let scheduler = ReminderScheduler {
            db: ctx.async_db.clone(),
            claude: ctx.claude.clone(),
            tools: tool_registry.clone(),
            skills: skill_registry.clone(),
            home_dir: ctx.home_dir.clone(),
            message_sender: None,
            embedding_client: embedding_client.clone(),
        };
        if let Err(e) = scheduler.recover().await {
            tracing::warn!(error = %e, "reminder recovery failed");
        }
    }

    let (user_tx, mut user_rx) = mpsc::unbounded_channel::<AgentRequest>();
    let (agent_tx, agent_rx) = mpsc::unbounded_channel::<AgentResponse>();

    let worker_db = ctx.async_db.clone();
    let worker_claude = ctx.claude.clone();
    let worker_tools = tool_registry.clone();
    let worker_skills = skill_registry.clone();
    let worker_home = ctx.home_dir.clone();
    let worker_session = session_id.clone();
    let worker_embedding = embedding_client;
    let handle = tokio::spawn(async move {
        while let Some(request) = user_rx.recv().await {
            match request {
                AgentRequest::Message(text) => {
                    let is_onboarding = check_onboarding(&worker_db).await;
                    let result = agent::run_agent(&AgentParams {
                        db: &worker_db,
                        claude: &worker_claude,
                        tools: &worker_tools,
                        skills: &worker_skills,
                        user_message: &text,
                        channel_type: "cli",
                        session_id: &worker_session,
                        home_dir: &worker_home,
                        is_onboarding,
                        message_sender: None,
                        skip_compaction: false,
                        embedding_client: worker_embedding.as_ref(),
                    })
                    .await;

                    let response = match result {
                        Ok(content) => AgentResponse {
                            content,
                            is_error: false,
                        },
                        Err(e) => AgentResponse {
                            content: format!("{e}"),
                            is_error: true,
                        },
                    };
                    if agent_tx.send(response).is_err() {
                        break;
                    }
                }
                AgentRequest::Quit => break,
            }
        }
    });

    let model = ctx.settings.claude_model.clone();
    let identity_name = identity.name.clone();

    let worker = AgentWorker { handle, _ctx: ctx };

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

pub async fn run(agent_name: &str) -> Result<()> {
    let ctx = init::init_for_agent(agent_name)?;
    let (mut worker, user_tx, agent_rx, session_id, model, identity_name, skill_registry) =
        spawn_agent_worker(ctx, agent_name).await?;

    // Build app with shared resources
    let mut app = App::new(
        user_tx,
        agent_rx,
        session_id,
        model,
        identity_name,
        worker._ctx.async_db.clone(),
        worker._ctx.claude.clone(),
        worker._ctx.home_dir.clone(),
        skill_registry,
        agent_name.to_string(),
        worker._ctx.global_home.clone(),
    );

    // Install panic hook that restores terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        original_hook(info);
    }));

    // Enter TUI mode
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
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
            Some(AppEvent::Tick) => {
                app.tick().await;
            }
            Some(AppEvent::Resize) => {
                app.needs_redraw = true;
            }
            None => break,
        }

        // Handle agent switch
        if let Some(target_name) = app.pending_switch.take() {
            // Wait for the old worker to stop (with timeout)
            let old_handle = std::mem::replace(
                &mut worker.handle,
                tokio::spawn(async {}), // placeholder
            );
            let _ = tokio::time::timeout(Duration::from_secs(2), old_handle).await;

            // Initialize the new agent
            match init::init_for_agent(&target_name) {
                Ok(new_ctx) => {
                    match spawn_agent_worker(new_ctx, &target_name).await {
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
                            app.claude = new_worker._ctx.claude.clone();
                            app.home_dir = new_worker._ctx.home_dir.clone();
                            app.skills = new_skills;
                            app.agent_name = target_name.clone();

                            worker = new_worker;

                            app.messages.push(ChatMessage {
                                role: ChatRole::System,
                                content: format!(
                                    "Switched to agent '{target_name}' ({new_model})."
                                ),
                                rendered: None,
                            });
                        }
                        Err(e) => {
                            app.messages.push(ChatMessage {
                                role: ChatRole::System,
                                content: format!("Failed to switch agent: {e}"),
                                rendered: None,
                            });
                        }
                    }
                }
                Err(e) => {
                    app.messages.push(ChatMessage {
                        role: ChatRole::System,
                        content: format!("Failed to switch agent: {e}"),
                        rendered: None,
                    });
                }
            }
            app.scroll_offset = 0;
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

    // Check agent worker for panics
    if worker.handle.is_finished() {
        if let Err(e) = worker.handle.await {
            eprintln!("Agent worker error: {e}");
        }
    } else {
        worker.handle.abort();
    }

    // Database shutdown happens automatically via Drop on worker._ctx
    Ok(())
}

fn restore_terminal() -> Result<()> {
    terminal::disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
    Ok(())
}

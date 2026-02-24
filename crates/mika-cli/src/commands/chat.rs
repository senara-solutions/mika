use anyhow::Result;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{event::DisableMouseCapture, execute};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

use mika_agent::agent::{self, AgentParams, check_onboarding};
use mika_agent::prompt;
use mika_agent::scheduler::ReminderScheduler;
use mika_agent::skills::SkillRegistry;
use mika_agent::tools;

use crate::init;
use crate::tui::app::{AgentRequest, AgentResponse, App};
use crate::tui::event::{AppEvent, EventReader};
use crate::tui::input;
use crate::tui::ui;

pub async fn run() -> Result<()> {
    let ctx = init::init()?;
    let identity = prompt::load_identity(&ctx.home_dir);
    let session_id = Uuid::new_v4().to_string();
    let tool_registry = Arc::new(tools::default_tools());
    let skill_registry = Arc::new(SkillRegistry::from_dir(&ctx.home_dir.join("skills")));

    // Recover reminders on startup
    let scheduler = ReminderScheduler {
        db: ctx.async_db.clone(),
        claude: ctx.claude.clone(),
        tools: tool_registry.clone(),
        skills: skill_registry.clone(),
        home_dir: ctx.home_dir.clone(),
        message_sender: None,
    };
    if let Err(e) = scheduler.recover().await {
        tracing::warn!(error = %e, "reminder recovery failed");
    }

    // Channels between TUI and agent worker
    let (user_tx, mut user_rx) = mpsc::unbounded_channel::<AgentRequest>();
    let (agent_tx, agent_rx) = mpsc::unbounded_channel::<AgentResponse>();

    // Spawn agent worker task
    let worker_db = ctx.async_db.clone();
    let worker_claude = ctx.claude.clone();
    let worker_tools = tool_registry.clone();
    let worker_skills = skill_registry.clone();
    let worker_home = ctx.home_dir.clone();
    let worker_session = session_id.clone();
    let agent_handle = tokio::spawn(async move {
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
                    })
                    .await;

                    let response = match result {
                        Ok(content) => AgentResponse {
                            content,
                            is_error: false,
                        },
                        Err(e) => AgentResponse {
                            content: format!("{e:#}"),
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

    // Build app
    let mut app = App::new(
        user_tx,
        agent_rx,
        session_id,
        ctx.settings.claude_model.clone(),
        identity.name.clone(),
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
                app.tick();
            }
            Some(AppEvent::Resize) => {
                app.needs_redraw = true;
            }
            None => break,
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
    if agent_handle.is_finished() {
        if let Err(e) = agent_handle.await {
            eprintln!("Agent worker error: {e}");
        }
    } else {
        agent_handle.abort();
    }

    // Database shutdown happens automatically via Drop on ctx
    Ok(())
}

fn restore_terminal() -> Result<()> {
    terminal::disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
    Ok(())
}

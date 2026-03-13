use anyhow::Result;
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use uuid::Uuid;

use mika_agent::agent::{self, AgentParams, check_onboarding};
use mika_agent::skills::SkillRegistry;
use mika_agent::tools;

use crate::init;

pub async fn run(
    message: &str,
    agent_name: &str,
    task_id: Option<&str>,
    session: Option<&str>,
    parent_task: Option<&str>,
) -> Result<()> {
    let ctx = init::init_for_agent(agent_name)?;

    // Use provided session ID or generate a new one.
    // When --session is passed (e.g., from claude-asked-relay), messages from the
    // same Claude Code run share a session for grouping and introspection.
    if let Some(s) = session
        && s.is_empty()
    {
        anyhow::bail!("--session value must not be empty");
    }
    let session_id = session
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    // Validate session ownership if reusing an existing session
    if session.is_some()
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
    let http_client = reqwest::Client::new();
    let message_sender =
        crate::init::make_message_sender(&ctx.settings, &ctx.async_db, &http_client);
    let embedding_client = ctx.settings.make_embedding_client();

    // Read message from arg, or from stdin if "-"
    let user_message = if message == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf.trim().to_string()
    } else {
        message.to_string()
    };

    if user_message.is_empty() {
        anyhow::bail!("Empty message. Provide a message argument or pipe via stdin with \"-\".");
    }

    const MAX_CALLBACK_RESULT: usize = 100_000; // 100KB, matches server limit
    if task_id.is_some() && user_message.len() > MAX_CALLBACK_RESULT {
        anyhow::bail!(
            "Callback result too large: {} bytes (max: {MAX_CALLBACK_RESULT})",
            user_message.len()
        );
    }

    // If --task-id is provided, mark the task as completed and exit.
    // The TUI tick loop (or server dispatcher) handles delivery to the user.
    if let Some(tid) = task_id {
        let task = match ctx.async_db.get_task(tid).await {
            Ok(Some(task)) => task,
            Ok(None) => {
                anyhow::bail!("Task '{}' not found.", tid);
            }
            Err(e) => {
                anyhow::bail!("Failed to load task '{}': {}", tid, e);
            }
        };

        if task.trigger_type != "callback" {
            anyhow::bail!(
                "Task '{}' has trigger_type '{}', not 'callback'. \
                 --task-id is only for callback tasks.",
                tid,
                task.trigger_type
            );
        }
        if !matches!(task.status.as_str(), "pending" | "in_progress") {
            anyhow::bail!(
                "Task '{}' has status '{}' and cannot be completed.",
                tid,
                task.status
            );
        }
        if !ctx
            .async_db
            .update_task_completed(tid, Some(&user_message))
            .await?
        {
            anyhow::bail!(
                "Task '{}' could not be completed: already in a terminal state.",
                tid
            );
        }

        // Check if all siblings are done and parent task should be dispatched.
        // In CLI one-shot mode we can't run the dispatcher, but we mark the
        // parent ready so the next TaskEngine tick (in TUI or server) picks it up.
        if let Ok(Some(parent_id)) = ctx.async_db.try_complete_parent_on_sibling_done(tid).await {
            tracing::info!(
                task_id = tid,
                parent_id = %parent_id,
                "All sibling tasks complete; parent task ready for dispatch"
            );
        }

        return Ok(());
    }

    // Prepend work item context if --parent-task is provided
    let user_message = if let Some(pt) = parent_task {
        format!("[work-item:{pt}] {user_message}")
    } else {
        user_message
    };

    // Normal ask mode — full conversation agent
    let mut tool_registry = tools::default_tools();
    for tool in tools::management_tools_if_needed(&ctx.global_home, &ctx.settings) {
        tool_registry.register(tool);
    }
    let tool_registry = Arc::new(tool_registry);
    let mut skill_registry = SkillRegistry::from_dir(&ctx.home_dir.join("skills"));
    if let Ok(overrides) = ctx
        .async_db
        .get_skill_overrides(&ctx.async_db.agent_id)
        .await
    {
        skill_registry.apply_overrides(&overrides);
    }
    skill_registry.validate_dependencies();
    let skill_registry = Arc::new(skill_registry);
    let is_onboarding = check_onboarding(&ctx.async_db).await;
    let skills_dirty = AtomicBool::new(false);
    let mcp_manager = init::connect_mcp(&ctx.home_dir).await;

    let output = agent::run_agent(&AgentParams {
        db: &ctx.async_db,
        claude: &ctx.claude,
        tools: &tool_registry,
        skills: &skill_registry,
        user_message: &user_message,
        channel_type: "cli",
        session_id: &session_id,
        home_dir: &ctx.home_dir,
        is_onboarding,
        message_sender,
        skip_compaction: false,
        embedding_client: embedding_client.as_ref(),
        thinking: None,
        user_images: &[],
        brave_api_key: ctx.settings.brave_api_key.as_deref(),
        skills_dirty: &skills_dirty,
        mcp_manager: mcp_manager.as_ref(),
        global_home_dir: Some(&ctx.global_home),
        is_callback_turn: false,
    })
    .await?;

    match output.text {
        Some(text) => println!("{text}"),
        None => eprintln!("{}", mika_agent::agent::EMPTY_RESPONSE_FALLBACK),
    }

    // Gracefully shut down MCP server connections
    if let Some(mcp) = mcp_manager {
        mcp.shutdown().await;
    }

    // Database shutdown happens automatically via Drop on ctx
    Ok(())
}

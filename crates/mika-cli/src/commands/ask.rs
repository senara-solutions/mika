use anyhow::Result;
use secrecy::ExposeSecret;
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use uuid::Uuid;

use mika_agent::agent::{self, AgentParams, check_onboarding};
use mika_agent::skills::SkillRegistry;
use mika_agent::tools;

use crate::cli::OutputFormat;
use crate::init;

#[derive(serde::Serialize)]
struct AskJsonResponse {
    role: &'static str,
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pending_tasks: Vec<String>,
    /// Runtime metadata envelope. Populated when `--verbose` is set; omitted
    /// otherwise (preserves byte-identical output for existing JSON consumers).
    /// Per-field gating: not every future field needs to be `--verbose`-gated;
    /// the envelope shape supports unconditional fields landing alongside
    /// gated ones without semantics churn.
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<MetadataEnvelope>,
}

/// Runtime metadata for `mika ask` invocations under `--format json`.
///
/// Mirrors the text-mode trailer's role: separates the assistant message
/// (`role`/`content`) from CLI/runtime concerns. Fields here may be
/// individually gated by `--verbose` (e.g., `session_id`) or unconditional
/// (e.g., `task_id` when the CLI flag is provided).
#[derive(Default, serde::Serialize)]
struct MetadataEnvelope {
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tokens: Option<TokensMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_task_id: Option<String>,
}

#[derive(serde::Serialize)]
struct TokensMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    input: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_read: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_write: Option<u64>,
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    message: &str,
    agent_name: &str,
    task_id: Option<&str>,
    task_complete: bool,
    session_id: Option<&str>,
    parent_task_id: Option<&str>,
    format: &OutputFormat,
    model_override: Option<&str>,
    enable_skill: &[String],
    disable_skill: &[String],
    verbose: bool,
) -> Result<()> {
    let mut ctx = init::init_for_agent(agent_name)?;

    if let Some(model) = model_override {
        ctx.override_model(model)?;
    }

    // Validate task_id format — reject empty or excessively long values
    if let Some(tid) = task_id {
        if tid.is_empty() {
            anyhow::bail!("--task-id value must not be empty");
        }
        if tid.len() > 128 {
            anyhow::bail!("--task-id value too long: {} bytes (max: 128)", tid.len());
        }
    }

    // Use provided session ID or generate a new one.
    // When --session-id is passed (e.g., from claude-asked-relay), messages from the
    // same Claude Code run share a session for grouping and introspection.
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
    // Store task_id in session metadata and as a first-class column for observability correlation
    let session_metadata = task_id.map(|tid| serde_json::json!({"task_id": tid}).to_string());
    if let Err(e) = ctx
        .async_db
        .create_session_with_metadata(
            &session_id,
            ctx.async_db.agent_id(),
            "cli",
            session_metadata.as_deref(),
            task_id,
        )
        .await
    {
        tracing::warn!(error = %e, "failed to create session");
    }
    let http_client = reqwest::Client::new();
    let message_sender = crate::init::make_message_sender(
        &ctx.settings,
        &ctx.async_db,
        &http_client,
        ctx.async_db.agent_id(),
    );
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

    // --task-complete path: validate and complete the callback task, then exit.
    // --task-id without --task-complete: correlation only — validate existence, then
    // fall through to the normal agent loop with task_id in session/trace metadata.
    if let Some(tid) = task_id {
        if task_complete {
            // Completion path: size limit + existing completion logic
            const MAX_CALLBACK_RESULT: usize = 100_000; // 100KB, matches server limit
            if user_message.len() > MAX_CALLBACK_RESULT {
                anyhow::bail!(
                    "Callback result too large: {} bytes (max: {MAX_CALLBACK_RESULT})",
                    user_message.len()
                );
            }

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
                     --task-id --task-complete is only for callback tasks.",
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
            if let Ok(Some(parent_id)) = ctx.async_db.try_complete_parent_on_sibling_done(tid).await
            {
                tracing::info!(
                    task_id = tid,
                    parent_id = %parent_id,
                    "All sibling tasks complete; parent task ready for dispatch"
                );
            }

            // End the session so the dashboard doesn't show it as "ongoing"
            if let Err(e) = ctx.async_db.end_session(&session_id).await {
                tracing::warn!(error = %e, "failed to end session");
            }
            return Ok(());
        }

        // Correlation-only path: validate task exists, emit deprecation warning if needed.
        // Uses unscoped lookup because the caller may not own the task.
        // See Database::get_task_unscoped for the ownership-vs-correlation distinction.
        match ctx.async_db.get_task_unscoped(tid).await {
            Ok(Some(task)) => {
                // Deprecation bridge: warn if this looks like an old-style completion call
                if task.trigger_type == "callback"
                    && matches!(task.status.as_str(), "pending" | "in_progress")
                {
                    tracing::warn!(
                        task_id = tid,
                        "DEPRECATED: --task-id without --task-complete no longer completes \
                         callback tasks. Add --task-complete to preserve completion behavior."
                    );
                    eprintln!(
                        "[mika] WARNING: --task-id without --task-complete no longer completes \
                         callback tasks. Add --task-complete to preserve completion behavior."
                    );
                }
            }
            Ok(None) => {
                anyhow::bail!("Task '{}' not found.", tid);
            }
            Err(e) => {
                tracing::warn!(error = %e, task_id = tid, "failed to validate task existence");
            }
        }
        // Fall through to normal agent loop with task_id in session metadata + trace
    }

    // Prepend task context if --parent-task-id is provided
    let user_message = if let Some(pt) = parent_task_id {
        format!("[work-item:{pt}] {user_message}")
    } else {
        user_message
    };

    // Normal ask mode — full conversation agent
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
    let identity = mika_agent::prompt::load_identity(&ctx.home_dir);
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
    // Validate --enable-skill / --disable-skill don't conflict on the same skill name.
    for enable_name in enable_skill {
        if disable_skill
            .iter()
            .any(|d| d.eq_ignore_ascii_case(enable_name))
        {
            anyhow::bail!(
                "Cannot both enable and disable skill '{enable_name}' in the same invocation"
            );
        }
    }

    // Apply transient overrides from --enable-skill / --disable-skill CLI flags.
    // Runs after apply_overrides() so it stacks on top of DB state.
    // Order: disable first, enable second (matches apply_overrides Phase 0/1 pattern).
    if !disable_skill.is_empty() {
        let result = skill_registry.apply_transient_disable(disable_skill);
        for name in &result.not_found {
            eprintln!("[mika] warning: --disable-skill '{name}' did not match any loaded skill");
        }
    }
    if !enable_skill.is_empty() {
        let result = skill_registry.apply_transient_always_on(enable_skill);
        for name in &result.disabled {
            eprintln!(
                "[mika] warning: --enable-skill '{name}' has no effect — \
                 skill is disabled. Run 'mika skills enable {name}' first."
            );
        }
        for name in &result.not_found {
            eprintln!("[mika] warning: --enable-skill '{name}' did not match any loaded skill");
        }
    }
    skill_registry.apply_load_safety_check();
    skill_registry.log_summary();

    // Surface validation warnings on stderr (Unit 4: #530)
    let warn_count = skill_registry.validated_warnings().len();
    if warn_count > 0 {
        eprintln!(
            "[mika] {warn_count} skill(s) loaded with validation warnings. \
             Run 'mika skills validate' for details."
        );
    }

    let skill_registry = Arc::new(skill_registry);
    let is_onboarding = check_onboarding(&ctx.async_db).await;
    let skills_dirty = AtomicBool::new(false);
    let mcp_manager = init::connect_mcp(&ctx.home_dir).await;

    let started = std::time::Instant::now();
    let output = agent::run_agent(&AgentParams {
        db: &ctx.async_db,
        llm: ctx.llm.as_ref(),
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
        brave_api_key: ctx
            .settings
            .brave_api_key
            .as_ref()
            .map(|s| s.expose_secret()),
        github_token: ctx.settings.agent_github_token(),
        github_app: ctx.github_app.as_deref(),
        skills_dirty: &skills_dirty,
        mcp_manager: mcp_manager.as_ref(),
        global_home_dir: Some(&ctx.global_home),
        is_callback_turn: false,
        settings: Some(&ctx.settings),
        trace_id: None,
        correlated_task_id: task_id.map(|s| s.to_string()),
        internal: task_id.is_some() && !task_complete,
        pr_reviews_posted: None, // CLI mode: no session-scoped dedup needed
    })
    .await;

    // End the session regardless of agent result so the dashboard shows duration
    if let Err(e) = ctx.async_db.end_session(&session_id).await {
        tracing::warn!(error = %e, "failed to end session");
    }

    let output = output?;

    // Check for pending callback tasks spawned during the agent loop (#265).
    // In `mika ask` there is no TaskEngine to poll for callbacks — the user needs
    // TUI or server to receive results from long-running background tasks.
    let pending_callbacks = match ctx
        .async_db
        .get_pending_callbacks_for_session(&session_id)
        .await
    {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!(error = %e, "failed to check pending callbacks");
            vec![]
        }
    };

    // Build the metadata envelope. Verbose-gated fields are populated only
    // when `--verbose`; unconditional fields (`task_id`, `parent_task_id`)
    // are populated whenever their CLI flag was provided.
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let model_string = {
        let provider = ctx.settings.llm_provider;
        let (model_name, _, _) = ctx.settings.provider_fields(provider);
        model_name.map(|m| format!("{provider}/{m}"))
    };

    let envelope = MetadataEnvelope {
        // Unconditional fields — present whenever the CLI flag was provided
        task_id: task_id.map(|s| s.to_string()),
        parent_task_id: parent_task_id.map(|s| s.to_string()),
        // Verbose-gated fields
        session_id: if verbose {
            Some(session_id.clone())
        } else {
            None
        },
        model: if verbose { model_string } else { None },
        agent_id: if verbose {
            Some(agent_name.to_string())
        } else {
            None
        },
        latency_ms: if verbose { Some(elapsed_ms) } else { None },
        tokens: if verbose {
            output.usage.as_ref().map(|u| TokensMetadata {
                input: Some(u.input_tokens),
                output: Some(u.output_tokens),
                cache_read: u.cache_read_input_tokens,
                cache_write: u.cache_creation_input_tokens,
            })
        } else {
            None
        },
    };

    // Emit None when all fields are absent so the top-level `metadata` key
    // is omitted from JSON (preserves byte-identical output for non-verbose,
    // non-task invocations).
    let has_any_field = envelope.session_id.is_some()
        || envelope.model.is_some()
        || envelope.agent_id.is_some()
        || envelope.latency_ms.is_some()
        || envelope.tokens.is_some()
        || envelope.task_id.is_some()
        || envelope.parent_task_id.is_some();
    let metadata = if has_any_field { Some(envelope) } else { None };

    match format {
        OutputFormat::Text => {
            match output.text {
                Some(text) => println!("{text}"),
                None => eprintln!("{}", mika_agent::agent::EMPTY_RESPONSE_FALLBACK),
            }
            if !pending_callbacks.is_empty() {
                eprintln!(
                    "\n[mika] {} background task(s) started. \
                     Open TUI (`mika`) or start server to receive results.",
                    pending_callbacks.len()
                );
            }
            // Text-mode trailer: one key: value per populated field.
            // Blank line separates response body from metadata trailer.
            if let Some(ref meta) = metadata {
                println!();
                if let Some(ref v) = meta.session_id {
                    println!("session_id: {v}");
                }
                if let Some(ref v) = meta.model {
                    println!("model: {v}");
                }
                if let Some(ref v) = meta.agent_id {
                    println!("agent_id: {v}");
                }
                if let Some(v) = meta.latency_ms {
                    println!("latency_ms: {v}");
                }
                if let Some(ref t) = meta.tokens {
                    if let Some(v) = t.input {
                        println!("tokens.input: {v}");
                    }
                    if let Some(v) = t.output {
                        println!("tokens.output: {v}");
                    }
                    if let Some(v) = t.cache_read {
                        println!("tokens.cache_read: {v}");
                    }
                    if let Some(v) = t.cache_write {
                        println!("tokens.cache_write: {v}");
                    }
                }
                if let Some(ref v) = meta.task_id {
                    println!("task_id: {v}");
                }
                if let Some(ref v) = meta.parent_task_id {
                    println!("parent_task_id: {v}");
                }
            }
        }
        OutputFormat::Json => {
            let response = AskJsonResponse {
                role: "assistant",
                content: output.text,
                task_id: task_id.map(|s| s.to_string()),
                pending_tasks: pending_callbacks,
                metadata,
            };
            println!("{}", serde_json::to_string(&response)?);
        }
    }

    // Gracefully shut down MCP server connections
    if let Some(mcp) = mcp_manager {
        mcp.shutdown().await;
    }

    // Database shutdown happens automatically via Drop on ctx
    Ok(())
}

/// Extended JSON response for team runs.
#[derive(serde::Serialize)]
struct AskTeamJsonResponse {
    role: &'static str,
    content: Option<String>,
    team_run: TeamRunMeta,
}

#[derive(serde::Serialize)]
struct TeamRunMeta {
    run_id: String,
    status: String,
    iterations: u32,
}

/// Run a team workflow in non-interactive mode (mika ask --team).
///
/// Runs the full team cycle (decompose → execute → review → deliver),
/// prints progress to stderr and the deliverable to stdout.
pub async fn run_team_ask(
    team_name: &str,
    message: &str,
    run_id: Option<&str>,
    format: &OutputFormat,
    global_home: &std::path::Path,
) -> Result<()> {
    use mika_agent::teams::types::{RunStatus, TeamEvent};
    use mika_common::config::Settings;

    // Read message from stdin if "-"
    let goal = if message == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf.trim().to_string()
    } else {
        message.to_string()
    };

    if goal.is_empty() {
        anyhow::bail!("Empty message. Provide a goal for the team.");
    }

    // Validate --run-id format before any filesystem/DB use (defense-in-depth)
    if let Some(ref_id) = run_id {
        if uuid::Uuid::parse_str(ref_id).is_err() {
            anyhow::bail!(
                "Invalid --run-id format. Expected a UUID (e.g., from a previous team run)."
            );
        }
        let db_path = mika_common::home::container_db_path(global_home);
        if db_path.exists() {
            let db = mika_agent::db::Database::open(&db_path)?;
            match db.load_team_run_by_id(ref_id) {
                Ok(Some(run)) => {
                    if run.team_name != team_name {
                        anyhow::bail!(
                            "Run '{}' belongs to team '{}', not '{}'.",
                            ref_id,
                            run.team_name,
                            team_name
                        );
                    }
                    if run.status == "running" {
                        anyhow::bail!(
                            "Run '{}' is still running. Cannot reference a running run.",
                            ref_id
                        );
                    }
                }
                Ok(None) => {
                    anyhow::bail!("Run '{}' not found.", ref_id);
                }
                Err(e) => {
                    anyhow::bail!("Failed to look up run '{}': {}", ref_id, e);
                }
            }
        } else {
            anyhow::bail!("No database found. Run `mika` first to initialize.");
        }
    }

    let settings = Settings::load(global_home)?;

    let callback = |event: TeamEvent| match event {
        TeamEvent::Progress(msg) => {
            eprintln!("  > {msg}");
        }
        TeamEvent::PhaseChanged { phase, iteration } => {
            eprintln!("  > Phase: {phase} (iteration {iteration})");
        }
        TeamEvent::AgentStarted { agent, role } => {
            eprintln!("  > Agent {agent} ({role}) started");
        }
        TeamEvent::AgentCompleted { agent, .. } => eprintln!("  > {agent} completed"),
        TeamEvent::AgentFailed { agent, error } => {
            eprintln!("  > {agent} failed: {error}");
        }
        TeamEvent::TasksAssigned { tasks, iteration } => {
            let names: Vec<_> = tasks.iter().map(|t| t.agent.as_str()).collect();
            eprintln!(
                "  > Iteration {iteration}: assigned tasks to {}",
                names.join(", ")
            );
        }
        TeamEvent::CriticReview {
            approved,
            feedback,
            iteration,
        } => {
            let verdict = if approved { "approved" } else { "rejected" };
            eprintln!("  > Critic (iteration {iteration}): {verdict}. {feedback}");
        }
        TeamEvent::Deliverable(_) => {} // handled below
        TeamEvent::RunFailed(_) => {}   // handled below
    };

    let team_db = crate::commands::teams::open_container_db_async(global_home)?;

    let github_app = mika_common::github_app::GitHubApp::from_settings(&settings);
    let run = mika_agent::teams::run_team(
        team_name,
        &goal,
        global_home,
        &settings,
        Some(Box::new(callback)),
        team_db.clone(),
        run_id,
        github_app,
        None, // CLI: no AppState for session-scoped dedup (#821)
    )
    .await?;
    team_db.shutdown();

    let is_failure = matches!(&run.status, RunStatus::Failed(_));

    match format {
        OutputFormat::Text => {
            if let Some(ref deliverable) = run.deliverable {
                println!("{deliverable}");
            } else if let RunStatus::Failed(ref msg) = run.status {
                eprintln!("Error: {msg}");
            }
        }
        OutputFormat::Json => {
            let response = AskTeamJsonResponse {
                role: "assistant",
                content: run.deliverable.clone(),
                team_run: TeamRunMeta {
                    run_id: run.run_id.clone(),
                    status: format!("{}", run.status),
                    iterations: run.iteration,
                },
            };
            println!("{}", serde_json::to_string(&response)?);
        }
    }

    if is_failure {
        std::process::exit(1);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_response_with_content() {
        let response = AskJsonResponse {
            role: "assistant",
            content: Some("Hello, world!".to_string()),
            task_id: None,
            pending_tasks: vec![],
            metadata: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert_eq!(json, r#"{"role":"assistant","content":"Hello, world!"}"#);
    }

    #[test]
    fn test_json_response_with_null_content() {
        let response = AskJsonResponse {
            role: "assistant",
            content: None,
            task_id: None,
            pending_tasks: vec![],
            metadata: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert_eq!(json, r#"{"role":"assistant","content":null}"#);
    }

    #[test]
    fn test_json_response_with_special_characters() {
        let response = AskJsonResponse {
            role: "assistant",
            content: Some("Line 1\nLine 2\t\"quoted\"".to_string()),
            task_id: None,
            pending_tasks: vec![],
            metadata: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["role"], "assistant");
        assert_eq!(parsed["content"], "Line 1\nLine 2\t\"quoted\"");
    }

    #[test]
    fn test_json_response_with_pending_tasks() {
        let response = AskJsonResponse {
            role: "assistant",
            content: Some("I've started the implementation.".to_string()),
            task_id: None,
            pending_tasks: vec!["task-abc-123".to_string(), "task-def-456".to_string()],
            metadata: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["role"], "assistant");
        assert!(parsed["pending_tasks"].is_array());
        assert_eq!(parsed["pending_tasks"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_json_response_omits_empty_pending_tasks() {
        let response = AskJsonResponse {
            role: "assistant",
            content: Some("Done.".to_string()),
            task_id: None,
            pending_tasks: vec![],
            metadata: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        // pending_tasks should be omitted entirely when empty
        assert!(!json.contains("pending_tasks"));
    }

    #[test]
    fn test_json_response_with_task_id() {
        let response = AskJsonResponse {
            role: "assistant",
            content: Some("Permission granted.".to_string()),
            task_id: Some("abc-123-def".to_string()),
            pending_tasks: vec![],
            metadata: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["role"], "assistant");
        assert_eq!(parsed["task_id"], "abc-123-def");
        // pending_tasks should be omitted when empty
        assert!(parsed.get("pending_tasks").is_none());
    }

    #[test]
    fn test_verbose_trailer_format() {
        // The verbose trailer must be a standalone `session_id: <uuid>` line
        // that downstream parsers can match by key name.
        let session_id = uuid::Uuid::new_v4().to_string();
        let trailer = format!("session_id: {session_id}");

        // Must start with the key name
        assert!(trailer.starts_with("session_id: "));

        // The value after "session_id: " must be a valid UUID
        let value = trailer.strip_prefix("session_id: ").unwrap();
        assert!(uuid::Uuid::parse_str(value).is_ok());
    }

    #[test]
    fn test_json_response_omits_none_task_id() {
        let response = AskJsonResponse {
            role: "assistant",
            content: Some("Hello.".to_string()),
            task_id: None,
            pending_tasks: vec![],
            metadata: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        // task_id should be omitted when None
        assert!(!json.contains("task_id"));
    }

    #[test]
    fn test_json_response_omits_metadata_when_none() {
        // Existing JSON consumers (no --verbose) must see byte-identical
        // output to pre-#829: the `metadata` key is skipped entirely when
        // None, not emitted as `"metadata":null`.
        let response = AskJsonResponse {
            role: "assistant",
            content: Some("Hello, world!".to_string()),
            task_id: None,
            pending_tasks: vec![],
            metadata: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert_eq!(json, r#"{"role":"assistant","content":"Hello, world!"}"#);
        assert!(!json.contains("metadata"));
    }

    #[test]
    fn test_json_response_includes_metadata_session_id_when_verbose() {
        // When --verbose is set, the JSON envelope must carry session_id
        // inside a nested `metadata` object — separating runtime metadata
        // from the assistant message shape (mirrors the text-mode trailer's
        // conceptual separation).
        let session_id = uuid::Uuid::new_v4().to_string();
        let response = AskJsonResponse {
            role: "assistant",
            content: Some("Here.".to_string()),
            task_id: None,
            pending_tasks: vec![],
            metadata: Some(MetadataEnvelope {
                session_id: Some(session_id.clone()),
                ..Default::default()
            }),
        };
        let json = serde_json::to_string(&response).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Metadata must be a nested object, not a top-level field.
        assert!(parsed["metadata"].is_object());
        assert_eq!(parsed["metadata"]["session_id"], session_id);

        // The session_id value must round-trip as a valid UUID.
        let value = parsed["metadata"]["session_id"].as_str().unwrap();
        assert!(uuid::Uuid::parse_str(value).is_ok());

        // session_id must NOT appear at the top level — that would entangle
        // the message shape with runtime metadata.
        assert!(parsed.get("session_id").is_none());
    }

    #[test]
    fn test_metadata_envelope_default_serializes_empty() {
        // Default envelope with all fields None serializes to `{}`.
        let envelope = MetadataEnvelope::default();
        let json = serde_json::to_string(&envelope).unwrap();
        assert_eq!(json, "{}");
    }

    #[test]
    fn test_metadata_envelope_partial_population() {
        // Only session_id set — other fields absent from JSON.
        let envelope = MetadataEnvelope {
            session_id: Some("abc-123".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["session_id"], "abc-123");
        assert!(parsed.get("model").is_none());
        assert!(parsed.get("agent_id").is_none());
        assert!(parsed.get("latency_ms").is_none());
        assert!(parsed.get("tokens").is_none());
        assert!(parsed.get("task_id").is_none());
        assert!(parsed.get("parent_task_id").is_none());
    }

    #[test]
    fn test_metadata_envelope_full_population() {
        // All fields populated — complete JSON shape.
        let envelope = MetadataEnvelope {
            session_id: Some("sess-1".to_string()),
            model: Some("anthropic/claude-sonnet-4-6".to_string()),
            agent_id: Some("mika-dev".to_string()),
            latency_ms: Some(1234),
            tokens: Some(TokensMetadata {
                input: Some(100),
                output: Some(50),
                cache_read: Some(80),
                cache_write: Some(20),
            }),
            task_id: Some("task-1".to_string()),
            parent_task_id: Some("parent-1".to_string()),
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["session_id"], "sess-1");
        assert_eq!(parsed["model"], "anthropic/claude-sonnet-4-6");
        assert_eq!(parsed["agent_id"], "mika-dev");
        assert_eq!(parsed["latency_ms"], 1234);
        assert_eq!(parsed["tokens"]["input"], 100);
        assert_eq!(parsed["tokens"]["output"], 50);
        assert_eq!(parsed["tokens"]["cache_read"], 80);
        assert_eq!(parsed["tokens"]["cache_write"], 20);
        assert_eq!(parsed["task_id"], "task-1");
        assert_eq!(parsed["parent_task_id"], "parent-1");
    }

    #[test]
    fn test_metadata_envelope_verbose_without_usage() {
        // Verbose with no LLM usage — tokens field absent.
        let envelope = MetadataEnvelope {
            session_id: Some("sess-2".to_string()),
            model: Some("openai/gpt-4o".to_string()),
            agent_id: Some("mika".to_string()),
            latency_ms: Some(500),
            tokens: None,
            ..Default::default()
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["model"], "openai/gpt-4o");
        assert_eq!(parsed["latency_ms"], 500);
        assert!(parsed.get("tokens").is_none());
    }

    #[test]
    fn test_metadata_envelope_unconditional_task_id_only() {
        // Non-verbose with --task-id: envelope has only task_id.
        let envelope = MetadataEnvelope {
            task_id: Some("abc".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["task_id"], "abc");
        // No verbose-gated fields
        assert!(parsed.get("session_id").is_none());
        assert!(parsed.get("model").is_none());
        assert!(parsed.get("agent_id").is_none());
        assert!(parsed.get("latency_ms").is_none());
        assert!(parsed.get("tokens").is_none());
    }

    #[test]
    fn test_tokens_metadata_partial_cache() {
        // Only input/output tokens, no cache fields.
        let tokens = TokensMetadata {
            input: Some(200),
            output: Some(100),
            cache_read: None,
            cache_write: None,
        };
        let json = serde_json::to_string(&tokens).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["input"], 200);
        assert_eq!(parsed["output"], 100);
        assert!(parsed.get("cache_read").is_none());
        assert!(parsed.get("cache_write").is_none());
    }
}

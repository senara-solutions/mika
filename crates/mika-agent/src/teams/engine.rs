use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tracing::{Instrument, debug, info, info_span, warn};

use mika_common::agent;
use mika_common::claude::ClaudeClient;
use mika_common::config::Settings;
use mika_common::embedding::EmbeddingClient;
use mika_common::team::TeamDefinition;

use crate::agent::TeamAgentParams;
use crate::async_db::AsyncDatabase;
use crate::db::Database;
use crate::skills::SkillRegistry;
use crate::startup;
use crate::tools;
use crate::tools::ToolRegistry;
use mika_common::home;

use super::prompt;
use super::types::*;

/// Maximum wall-clock time for the entire team run (all phases combined).
const TEAM_RUN_TIMEOUT_SECS: u64 = 900; // 15 minutes

/// Resources needed to run a specific agent.
struct AgentResources {
    db: AsyncDatabase,
    skills: SkillRegistry,
    home_dir: PathBuf,
    embedding_client: Option<EmbeddingClient>,
}

/// Shared initialization resources produced by [`TeamEngine::init_resources`].
struct EngineResources {
    agents: HashMap<String, AgentResources>,
    claude: ClaudeClient,
    tool_registry: ToolRegistry,
    workspace_dir: PathBuf,
}

/// The team orchestration engine.
pub struct TeamEngine {
    team: TeamDefinition,
    run: TeamRun,
    workspace_dir: PathBuf,
    agents: Arc<HashMap<String, AgentResources>>,
    claude: ClaudeClient,
    tool_registry: Arc<ToolRegistry>,
    callback: Option<Arc<TeamEventCallback>>,
    brave_api_key: Option<String>,
    /// Team-level database for persisting runs and messages.
    team_db: AsyncDatabase,
    /// Message ID of the root goal message (set after insert).
    goal_msg_id: Option<i64>,
    /// Trace ID for correlating all events within this team run.
    trace_id: String,
}

impl TeamEngine {
    /// Initialize the shared resources (agents, Claude client, tool registry, workspace)
    /// used by both [`Self::new`] and [`Self::new_for_resume`].
    fn init_resources(
        team: &TeamDefinition,
        global_home: &Path,
        settings: &Settings,
        run_id: &str,
        reference_workspace: Option<PathBuf>,
    ) -> Result<EngineResources> {
        let team_name = &team.team.name;
        let workspace_dir = mika_common::team::workspace_run_dir(global_home, team_name, run_id);

        // Initialize agent resources
        let mut agents = HashMap::new();
        let embedding_client = settings.make_embedding_client();

        // Use the single shared container database
        let db_path = home::container_db_path(global_home);

        // N+1 DB connections: each agent gets its own SQLite connection (+ 1 for team_db).
        // This is intentional — AsyncDatabase::shutdown() kills the inner OS thread, so we
        // cannot share a single connection across agent shutdown boundaries. WAL mode handles
        // concurrent readers correctly, and busy_timeout covers write contention.
        for ta in &team.agents {
            let home_dir = agent::agent_dir(global_home, &ta.name);
            let db = Database::open(&db_path)
                .with_context(|| format!("failed to open container DB for agent '{}'", ta.name))?;
            let identity = crate::prompt::load_identity(&home_dir);
            db.register_agent(&ta.name, &identity.name, home_dir.to_str().unwrap_or(""))?;
            startup::seed_core_memory_if_empty(&db, &home_dir, &ta.name)?;
            let mut skills = SkillRegistry::from_dir(&home_dir.join("skills"));
            if let Ok(overrides) = db.get_skill_overrides(&ta.name) {
                skills.apply_overrides(&overrides);
            }
            let async_db = AsyncDatabase::new_with_agent(db, &ta.name);

            agents.insert(
                ta.name.clone(),
                AgentResources {
                    db: async_db,
                    skills,
                    home_dir,
                    embedding_client: embedding_client.clone(),
                },
            );
        }

        let claude = ClaudeClient::new(
            settings.anthropic_api_key.clone(),
            settings.claude_model.clone(),
            settings.claude_max_tokens,
        )?;

        // Build tool registry once with base tools + workspace tools (#255)
        let mut tool_registry = tools::default_tools();
        for tool in tools::team_tools(&workspace_dir, reference_workspace.as_deref()) {
            tool_registry.register(tool);
        }

        Ok(EngineResources {
            agents,
            claude,
            tool_registry,
            workspace_dir,
        })
    }

    /// Create a new engine for a team run.
    pub fn new(
        team: TeamDefinition,
        goal: &str,
        global_home: &Path,
        settings: &Settings,
        callback: Option<TeamEventCallback>,
        team_db: AsyncDatabase,
        reference_run_id: Option<&str>,
    ) -> Result<Self> {
        let run_id = uuid::Uuid::new_v4().to_string();

        // Resolve reference workspace from a previous run
        let reference_workspace = reference_run_id.map(|ref_id| {
            mika_common::team::workspace_run_dir(global_home, &team.team.name, ref_id)
        });

        let res = Self::init_resources(&team, global_home, settings, &run_id, reference_workspace)?;

        // Ensure run-scoped workspace and .meta directory exist
        std::fs::create_dir_all(res.workspace_dir.join(".meta")).with_context(|| {
            format!("failed to create workspace {}", res.workspace_dir.display())
        })?;

        let run = TeamRun {
            run_id,
            team_name: team.team.name.clone(),
            goal: goal.to_string(),
            status: RunStatus::Running,
            iteration: 0,
            max_iterations: team.flow.max_iterations,
            tasks: Vec::new(),
            started_at: chrono::Utc::now().timestamp(),
            ended_at: None,
            deliverable: None,
        };

        Ok(Self {
            team,
            run,
            workspace_dir: res.workspace_dir,
            agents: Arc::new(res.agents),
            claude: res.claude,
            tool_registry: Arc::new(res.tool_registry),
            callback: callback.map(Arc::new),
            brave_api_key: settings.brave_api_key.clone(),
            team_db,
            goal_msg_id: None,
            trace_id: mika_common::trace::generate_trace_id(),
        })
    }

    /// Create an engine for resuming a suspended team run.
    pub async fn new_for_resume(
        team: TeamDefinition,
        run: TeamRun,
        global_home: &Path,
        settings: &Settings,
        team_db: AsyncDatabase,
    ) -> Result<Self> {
        let res = Self::init_resources(&team, global_home, settings, &run.run_id, None)?;

        // Restore trace_id from DB to maintain correlation across suspend/resume.
        // Fall back to generating fresh if not persisted (backward compat with pre-v10 runs).
        let trace_id = match team_db.load_team_run_trace_id(&run.run_id).await {
            Ok(Some(tid)) => tid,
            Ok(None) => {
                debug!(run_id = %run.run_id, "no trace_id in team_run (pre-v10), generating fresh");
                mika_common::trace::generate_trace_id()
            }
            Err(e) => {
                warn!(error = %e, run_id = %run.run_id, "failed to load trace_id, generating fresh");
                mika_common::trace::generate_trace_id()
            }
        };

        Ok(Self {
            team,
            run,
            workspace_dir: res.workspace_dir,
            agents: Arc::new(res.agents),
            claude: res.claude,
            tool_registry: Arc::new(res.tool_registry),
            callback: None, // No callback on resume — events go through task system
            brave_api_key: settings.brave_api_key.clone(),
            team_db,
            goal_msg_id: None,
            trace_id,
        })
    }

    /// Resume execution from a specific phase with child results injected.
    ///
    /// Called after a suspended team run's child tasks all complete.
    /// The child results are injected as if agents completed synchronously.
    pub async fn execute_from_phase(
        mut self,
        next_phase: &str,
        child_results: &str,
    ) -> Result<TeamRun> {
        info!(
            team = %self.run.team_name,
            run_id = %self.run.run_id,
            phase = next_phase,
            "resuming team run from phase"
        );

        // Mark run as running again
        self.run.status = RunStatus::Running;
        if let Err(e) = self.team_db.resume_team_run_status(&self.run.run_id).await {
            warn!(error = %e, "failed to resume team run status");
        }

        // Inject child results into the task assignments
        if let Ok(results) = serde_json::from_str::<Vec<serde_json::Value>>(child_results) {
            for result in &results {
                let agent = result["agent"].as_str().unwrap_or("");
                let status = result["status"].as_str().unwrap_or("");
                let output = result["result"].as_str().unwrap_or("");

                // Update the task assignment status based on child task status
                if let Some(task) = self.run.tasks.iter_mut().find(|t| t.agent == agent) {
                    if status == "completed" {
                        task.status = TaskStatus::Completed;
                    } else {
                        task.status = TaskStatus::Failed(format!("child task {status}: {output}"));
                    }
                }

                // Write agent output to the workspace output file
                if !output.is_empty()
                    && let Some(task) = self.run.tasks.iter().find(|t| t.agent == agent)
                {
                    let output_path = self.workspace_dir.join(&task.output_file);
                    if let Err(e) = tokio::fs::write(&output_path, output).await {
                        warn!(error = %e, agent, "failed to write resumed agent output");
                    }
                }
            }
        }

        // Determine starting phase based on next_phase parameter
        match next_phase {
            "execute" => {
                // Re-execute tasks, then review, then deliver
                self.transition_phase(TeamPhase::Execute);
                let suspended = self.execute_tasks().await?;
                if suspended {
                    // Team run suspended again, will resume via invoke_orchestrator
                    self.finalize_and_shutdown().await;
                    return Ok(self.run);
                }

                // Fall through to review
                self.transition_phase(TeamPhase::Review);
                let (approved, feedback) = self.review().await?;
                if approved {
                    info!(
                        iteration = self.run.iteration,
                        "critic approved (resumed from execute)"
                    );
                } else {
                    info!(
                        iteration = self.run.iteration,
                        feedback = %feedback,
                        "critic rejected on resume from execute, proceeding anyway"
                    );
                }
            }
            "deliver" => {
                // Skip review, go straight to deliver
                info!(
                    iteration = self.run.iteration,
                    "resuming directly to deliver phase"
                );
            }
            other => {
                if other != "review" {
                    warn!(
                        phase = other,
                        "unknown next_phase value, defaulting to review"
                    );
                }
                // Default: Review -> Deliver
                self.transition_phase(TeamPhase::Review);
                let (approved, feedback) = self.review().await?;
                if approved {
                    info!(iteration = self.run.iteration, "critic approved (resumed)");
                } else {
                    info!(
                        iteration = self.run.iteration,
                        feedback = %feedback,
                        "critic rejected on resume, proceeding anyway"
                    );
                }
            }
        }

        // Deliver
        self.deliver_phase().await?;

        self.run.status = RunStatus::Completed;
        self.finalize_and_shutdown().await;

        Ok(self.run)
    }

    /// Execute the full team orchestration flow.
    #[tracing::instrument(
        name = "team_run",
        skip_all,
        fields(
            team = %self.run.team_name,
            run_id = %self.run.run_id,
            goal_len = self.run.goal.len(),
        )
    )]
    pub async fn execute(mut self) -> Result<TeamRun> {
        info!("team_run started");

        // Persist the team run to DB
        if let Err(e) = self
            .team_db
            .insert_team_run(
                &self.run.run_id,
                &self.run.team_name,
                &self.run.goal,
                self.run.max_iterations,
                self.run.started_at,
                Some(&self.trace_id),
            )
            .await
        {
            warn!(error = %e, "failed to persist team run to DB");
        }

        // Create a team session for storing agent messages
        let orchestrator_agent_id = self.team.team.orchestrator.clone();
        let team_session_id = format!("team-{}", self.run.run_id);
        if let Err(e) = self
            .team_db
            .create_session(&team_session_id, &orchestrator_agent_id, "team")
            .await
        {
            warn!(error = %e, "failed to create team session");
        }

        // Insert the goal as the root workspace entry
        match self
            .team_db
            .insert_team_workspace_entry(
                &self.run.run_id,
                None,
                None,
                "goal",
                &self.run.goal,
                0,
                Some(&self.trace_id),
            )
            .await
        {
            Ok(id) => self.goal_msg_id = Some(id),
            Err(e) => warn!(error = %e, "failed to persist goal message"),
        }

        let result = match tokio::time::timeout(
            Duration::from_secs(TEAM_RUN_TIMEOUT_SECS),
            self.execute_inner(),
        )
        .await
        {
            Ok(r) => r,
            Err(_) => {
                warn!("team run timed out after {TEAM_RUN_TIMEOUT_SECS}s");
                Err(anyhow::anyhow!(
                    "Team run timed out after {} minutes. Check workspace for partial results.",
                    TEAM_RUN_TIMEOUT_SECS / 60,
                ))
            }
        };

        match &result {
            Ok(_) => {
                // Don't override Suspended status set by execute_tasks
                if self.run.status == RunStatus::Running {
                    self.run.status = RunStatus::Completed;
                }
            }
            Err(e) => {
                self.run.status = RunStatus::Failed(e.to_string());
                self.emit_event(TeamEvent::RunFailed(e.to_string()));

                // Persist error entry to DB (generic error, no agent)
                if let Some(goal_id) = self.goal_msg_id
                    && let Err(e) = self
                        .team_db
                        .insert_team_workspace_entry(
                            &self.run.run_id,
                            Some(goal_id),
                            None,
                            "error",
                            &e.to_string(),
                            self.run.iteration,
                            Some(&self.trace_id),
                        )
                        .await
                {
                    warn!(error = %e, "failed to persist team workspace entry");
                }
            }
        }

        self.finalize_and_shutdown().await;

        match result {
            Ok(_) => Ok(self.run),
            Err(e) => {
                // Return the run even on failure (it has status info)
                warn!(error = %e, "team execution failed");
                Ok(self.run)
            }
        }
    }

    /// Run the deliver phase: transition, emit progress, produce deliverable, emit result.
    ///
    /// Shared by `execute_inner` (normal flow) and `execute_from_phase` (resumed flow).
    async fn deliver_phase(&mut self) -> Result<()> {
        self.transition_phase(TeamPhase::Deliver);
        self.emit_event(TeamEvent::Progress(
            "Producing final deliverable...".to_string(),
        ));
        let deliverable = self.deliver().await?;
        self.run.deliverable = Some(deliverable.clone());

        // Write deliverable metadata
        self.write_metadata_file("deliverable.md", &deliverable);

        self.emit_event(TeamEvent::Deliverable(deliverable));
        Ok(())
    }

    /// Persist final run status to DB and shutdown all AsyncDatabase instances.
    ///
    /// Shared by `execute` (normal flow) and `execute_from_phase` (resumed flow).
    async fn finalize_and_shutdown(&mut self) {
        let is_suspended = self.run.status == RunStatus::Suspended;

        let ended_at = if is_suspended {
            None // Suspended runs haven't ended yet
        } else {
            let ts = chrono::Utc::now().timestamp();
            self.run.ended_at = Some(ts);
            Some(ts)
        };

        // Update run status in DB
        let (status_str, failure_reason) = match &self.run.status {
            RunStatus::Running => ("running", None),
            RunStatus::Completed => ("completed", None),
            RunStatus::Suspended => ("suspended", None),
            RunStatus::Failed(reason) => ("failed", Some(reason.as_str())),
        };
        if let Err(e) = self
            .team_db
            .update_team_run(
                &self.run.run_id,
                status_str,
                failure_reason,
                self.run.iteration,
                self.run.deliverable.as_deref(),
                ended_at,
            )
            .await
        {
            warn!(error = %e, "failed to update team run in DB");
        }

        // Shutdown all AsyncDatabase instances to avoid thread leaks (#256)
        for resources in self.agents.values() {
            resources.db.shutdown();
        }
        self.team_db.shutdown();
    }

    async fn execute_inner(&mut self) -> Result<()> {
        // Load recent conversation history for orchestrator context
        let history = self
            .team_db
            .load_team_runs_for_prompt(&self.run.team_name, 10, 500)
            .await
            .unwrap_or_default();

        // Load enriched summary for the most recent previous run
        let previous_run_summary = self
            .team_db
            .get_last_completed_team_run_summary(&self.run.team_name)
            .await
            .unwrap_or_else(|e| {
                debug!("Failed to load previous team run summary: {e}");
                None
            });
        if let Some(ref summary) = previous_run_summary {
            info!(
                previous_run_id = %summary.run.id,
                previous_run_status = %summary.run.status,
                agent_results_count = summary.agent_results.len(),
                "Injecting previous team run context"
            );
        }

        // Step 1: Decompose -- orchestrator produces task assignments
        self.run.iteration = 1;
        self.transition_phase(TeamPhase::Decompose);
        self.emit_event(TeamEvent::Progress("Decomposing goal...".to_string()));

        // Write goal metadata (once, at first decompose)
        self.write_metadata_file("goal.md", &self.run.goal);

        match self
            .decompose(None, &history, previous_run_summary.as_ref())
            .await?
        {
            DecomposeResult::Tasks(tasks) => {
                self.run.tasks = tasks;
                // Write assignments metadata
                let assignments = self.format_assignments();
                self.write_metadata_file("assignments.md", &assignments);
            }
            DecomposeResult::Conversational(reply) => {
                self.run.deliverable = Some(reply.clone());
                self.emit_event(TeamEvent::Deliverable(reply));
                return Ok(());
            }
        }

        // Emit structured event for task assignments
        self.emit_event(TeamEvent::TasksAssigned {
            tasks: self.run.tasks.clone(),
            iteration: 1,
        });

        // Iterate: execute -> review -> (retry if rejected)
        loop {
            // Step 2: Execute -- run each specialist
            self.transition_phase(TeamPhase::Execute);
            let suspended = self.execute_tasks().await?;
            if suspended {
                return Ok(()); // Team run suspended, will resume via invoke_orchestrator
            }

            // Step 3: Review -- critic evaluates outputs
            self.transition_phase(TeamPhase::Review);
            let (approved, feedback) = self.review().await?;

            if approved {
                info!(iteration = self.run.iteration, "critic approved");
                break;
            }

            if self.run.iteration >= self.run.max_iterations {
                info!(
                    iteration = self.run.iteration,
                    "max iterations reached, proceeding with current output"
                );
                break;
            }

            // Re-decompose with feedback
            self.transition_phase(TeamPhase::ReDecompose);
            self.emit_event(TeamEvent::Progress(format!(
                "Iteration {}: critic requested revisions...",
                self.run.iteration
            )));
            match self
                .decompose(Some(&feedback), &history, previous_run_summary.as_ref())
                .await?
            {
                DecomposeResult::Tasks(tasks) => {
                    self.run.tasks = tasks;
                    // Update assignments metadata on re-decompose
                    let assignments = self.format_assignments();
                    self.write_metadata_file("assignments.md", &assignments);
                }
                DecomposeResult::Conversational(reply) => {
                    self.run.deliverable = Some(reply.clone());
                    self.emit_event(TeamEvent::Deliverable(reply));
                    return Ok(());
                }
            }

            self.emit_event(TeamEvent::TasksAssigned {
                tasks: self.run.tasks.clone(),
                iteration: self.run.iteration + 1,
            });

            self.run.iteration += 1;
        }

        // Step 4: Deliver -- produce final output
        self.deliver_phase().await?;

        Ok(())
    }

    /// Ask the orchestrator to decompose the goal into tasks.
    /// Pass `None` for first decomposition, `Some(feedback)` for re-decomposition
    /// after critic rejection. (#261: merged decompose + decompose_with_feedback)
    async fn decompose(
        &self,
        feedback: Option<&str>,
        history: &[crate::db::TeamRunRow],
        previous_run: Option<&crate::db::TeamRunSummary>,
    ) -> Result<DecomposeResult> {
        let listing = prompt::workspace_listing(&self.workspace_dir);
        let context = prompt::build_orchestrator_context(
            &self.team,
            &listing,
            feedback,
            history,
            previous_run,
        );

        let orchestrator_name = &self.team.team.orchestrator;

        let message = if feedback.is_some() {
            format!(
                "The previous attempt was rejected by the critic. Revise the task assignments.\n\nOriginal goal: {}",
                self.run.goal
            )
        } else {
            self.run.goal.clone()
        };

        let response = self
            .run_agent(orchestrator_name, &message, &context)
            .await?;

        let result = parse_task_assignments(&response, &self.team)?;

        let iteration = if feedback.is_some() {
            self.run.iteration + 1
        } else {
            1
        };

        // For conversational replies, persist and return early
        let tasks = match result {
            DecomposeResult::Conversational(reply) => {
                // Persist orchestrator conversational response to DB
                if let Some(goal_id) = self.goal_msg_id
                    && let Err(e) = self
                        .team_db
                        .insert_team_workspace_entry(
                            &self.run.run_id,
                            Some(goal_id),
                            Some(orchestrator_name),
                            "orchestrator",
                            &response,
                            iteration,
                            Some(&self.trace_id),
                        )
                        .await
                {
                    warn!(error = %e, "failed to persist team workspace entry");
                }
                return Ok(DecomposeResult::Conversational(reply));
            }
            DecomposeResult::Tasks(tasks) => tasks,
        };

        // Persist orchestrator decomposition to DB
        if let Some(goal_id) = self.goal_msg_id {
            let orchestrator_entry_id = self
                .team_db
                .insert_team_workspace_entry(
                    &self.run.run_id,
                    Some(goal_id),
                    Some(orchestrator_name),
                    "orchestrator",
                    &response,
                    iteration,
                    Some(&self.trace_id),
                )
                .await
                .ok();

            // Insert assignment entries as children of orchestrator
            if let Some(orch_id) = orchestrator_entry_id {
                for task in &tasks {
                    if let Err(e) = self
                        .team_db
                        .insert_team_workspace_entry(
                            &self.run.run_id,
                            Some(orch_id),
                            Some(&task.agent),
                            "assignment",
                            &task.task,
                            iteration,
                            Some(&self.trace_id),
                        )
                        .await
                    {
                        warn!(error = %e, "failed to persist team workspace entry");
                    }
                }
            }
        }

        Ok(DecomposeResult::Tasks(tasks))
    }

    /// Execute all tasks by delegating to specialist agents concurrently.
    ///
    /// Uses `tokio::task::JoinSet` to run all specialist agents in parallel.
    /// Each agent has its own AsyncDatabase and the workspace is shared via
    /// the filesystem, so there are no shared mutable state concerns.
    async fn execute_tasks(&mut self) -> Result<bool> {
        // Prepare shared resources that will be moved into spawned tasks.
        let agents = Arc::clone(&self.agents);
        let claude = self.claude.clone();
        let tool_registry = Arc::clone(&self.tool_registry);
        let run_id = self.run.run_id.clone();
        let callback = self.callback.clone();
        let team_name = self.run.team_name.clone();
        let brave_api_key = self.brave_api_key.clone();
        let team_db = self.team_db.clone();

        // Build per-task parameters upfront from self (avoids borrowing self in spawned tasks).
        struct TaskInput {
            index: usize,
            agent_name: String,
            role: String,
            task_desc: String,
            context: String,
            child_task_id: Option<String>,
        }

        let mut inputs = Vec::with_capacity(self.run.tasks.len());
        for (i, task) in self.run.tasks.iter_mut().enumerate() {
            task.status = TaskStatus::Running;

            let mandate = self
                .team
                .agents
                .iter()
                .find(|a| a.name == task.agent)
                .map(|a| a.mandate.as_str())
                .unwrap_or("Complete the assigned task");

            let context = prompt::build_specialist_context(
                &task.role,
                mandate,
                &task.task,
                &task.output_file,
            );

            inputs.push(TaskInput {
                index: i,
                agent_name: task.agent.clone(),
                role: task.role.clone(),
                task_desc: task.task.clone(),
                context,
                child_task_id: None, // Populated below after task tree creation
            });
        }

        // Phase 4.2: Create parent/child task tree for sibling completion detection.
        // Parent task: invoke_orchestrator (fires when all children complete).
        // Child tasks: one per agent delegation.
        let team_state = serialize_checkpoint(&self.run);
        let parent_task = crate::db::NewTask {
            agent_id: self.team.team.orchestrator.clone(),
            team_run_id: Some(self.run.run_id.clone()),
            parent_task_id: None,
            depth: 0,
            label: format!("team-{}-orchestrator", self.run.team_name),
            trigger_type: crate::task_engine::types::trigger_type::CALLBACK.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: Some(chrono::Utc::now().timestamp() + TEAM_RUN_TIMEOUT_SECS as i64),
            action_type: crate::task_engine::types::action_type::INVOKE_ORCHESTRATOR.to_string(),
            action_config: serde_json::json!({
                "team_name": self.run.team_name,
                "next_phase": "review",
            })
            .to_string(),
            input_context: Some(team_state),
            created_by_session: Some(format!("team-{}", self.run.run_id)),
            created_trace_id: Some(self.trace_id.clone()),
            reference_url: None,
            source: None,
        };

        let parent_task_id = self
            .team_db
            .create_task(parent_task)
            .await
            .context("failed to create parent invoke_orchestrator task")?;

        for input in &mut inputs {
            let child_task = crate::db::NewTask {
                agent_id: input.agent_name.clone(),
                team_run_id: Some(self.run.run_id.clone()),
                parent_task_id: Some(parent_task_id.clone()),
                depth: 1,
                label: format!("team-agent-{}", input.agent_name),
                trigger_type: crate::task_engine::types::trigger_type::CALLBACK.to_string(),
                cron_expr: None,
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: None,
                timeout_at: Some(chrono::Utc::now().timestamp() + TEAM_RUN_TIMEOUT_SECS as i64),
                action_type: crate::task_engine::types::action_type::RESUME_AGENT.to_string(),
                action_config: serde_json::json!({
                    "agent": input.agent_name,
                })
                .to_string(),
                input_context: None,
                created_by_session: Some(format!("team-{}-{}", self.run.run_id, input.agent_name)),
                created_trace_id: Some(self.trace_id.clone()),
                reference_url: None,
                source: None,
            };

            match self.team_db.create_task(child_task).await {
                Ok(child_id) => {
                    input.child_task_id = Some(child_id);
                }
                Err(e) => {
                    warn!(agent = %input.agent_name, error = %e, "failed to create child task");
                }
            }
        }

        self.emit_event(TeamEvent::Progress(format!(
            "Running {} specialist agents in parallel...",
            inputs.len()
        )));

        // Spawn all tasks concurrently via JoinSet.
        let mut join_set = tokio::task::JoinSet::new();

        for input in inputs {
            self.emit_event(TeamEvent::AgentStarted {
                agent: input.agent_name.clone(),
                role: input.role.clone(),
            });

            let agents = Arc::clone(&agents);
            let claude = claude.clone();
            let tool_registry = Arc::clone(&tool_registry);
            let run_id = run_id.clone();
            let callback = callback.clone();
            let team_name = team_name.clone();
            let brave_api_key = brave_api_key.clone();
            let team_db = team_db.clone();
            let trace_id = self.trace_id.clone();

            let agent_span = info_span!("team_agent_task", agent = %input.agent_name);
            join_set.spawn(
                async move {
                    let agent_name = &input.agent_name;

                    // Report that this agent is starting.
                    info!(team = %team_name, "Running {agent_name}...");
                    if let Some(ref cb) = callback {
                        cb(TeamEvent::Progress(format!("Running {agent_name}...")));
                    }

                    let resources = agents.get(agent_name.as_str()).with_context(|| {
                        format!("agent '{}' not found in team resources", agent_name)
                    });

                    let result: Result<String> = match resources {
                        Ok(resources) => {
                            let session_id = format!("team-{}-{}", run_id, agent_name);
                            let skills_dirty = AtomicBool::new(false);
                            let child_task_id_ref = input.child_task_id.as_deref();
                            let params = TeamAgentParams {
                                db: &resources.db,
                                claude: &claude,
                                tools: &tool_registry,
                                skills: &resources.skills,
                                home_dir: &resources.home_dir,
                                task_message: &input.task_desc,
                                team_context: &input.context,
                                session_id: &session_id,
                                embedding_client: resources.embedding_client.as_ref(),
                                brave_api_key: brave_api_key.as_deref(),
                                skills_dirty: &skills_dirty,
                                mcp_manager: None,
                                agent_name,
                                child_task_id: child_task_id_ref,
                                // Team agents communicate through the orchestrator pipeline
                                // (workspace entries, deliverables, critic feedback), not
                                // directly to users via Telegram.
                                message_sender: None,
                            };
                            crate::agent::run_team_agent(&params)
                                .await
                                .map(|opt| opt.unwrap_or_default())
                        }
                        Err(e) => Err(e),
                    };

                    // Persist and report completion/failure for this agent.
                    match &result {
                        Ok(response) => {
                            info!(agent = %agent_name, "task completed");
                            if let Some(ref cb) = callback {
                                cb(TeamEvent::AgentCompleted {
                                    agent: agent_name.to_string(),
                                    response: response.clone(),
                                });
                            }
                            // Persist agent response to messages table
                            let team_session_id = format!("team-{}", run_id);
                            let metadata = serde_json::json!({
                                "team_run_id": run_id,
                                "agent_name": agent_name,
                            })
                            .to_string();
                            if let Err(e) = team_db
                                .save_message_as(
                                    agent_name,
                                    &team_session_id,
                                    "assistant",
                                    response,
                                    Some(&metadata),
                                    Some(&trace_id),
                                )
                                .await
                            {
                                warn!(error = %e, "failed to persist agent response message");
                            }
                        }
                        Err(e) => {
                            let error_str = e.to_string();
                            warn!(agent = %agent_name, error = %error_str, "task failed");
                            if let Some(ref cb) = callback {
                                cb(TeamEvent::AgentFailed {
                                    agent: agent_name.to_string(),
                                    error: error_str.clone(),
                                });
                            }
                            // Persist agent error to messages table
                            let team_session_id = format!("team-{}", run_id);
                            let metadata = serde_json::json!({
                                "team_run_id": run_id,
                                "agent_name": agent_name,
                            })
                            .to_string();
                            if let Err(e) = team_db
                                .save_message_as(
                                    agent_name,
                                    &team_session_id,
                                    "system",
                                    &error_str,
                                    Some(&metadata),
                                    Some(&trace_id),
                                )
                                .await
                            {
                                warn!(error = %e, "failed to persist agent error message");
                            }
                        }
                    }

                    (input.index, input.agent_name, result)
                }
                .instrument(agent_span),
            );
        }

        // Collect results as tasks complete, emitting periodic heartbeats so the
        // user knows the run is still alive.
        let mut completed_count = 0usize;
        let total_count = self.run.tasks.len();
        let start = tokio::time::Instant::now();
        let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
        heartbeat.tick().await; // skip the immediate first tick

        loop {
            tokio::select! {
                Some(join_result) = join_set.join_next() => {
                    completed_count += 1;
                    match join_result {
                        Ok((index, agent_name, result)) => {
                            match result {
                                Ok(_) => {
                                    self.run.tasks[index].status = TaskStatus::Completed;
                                }
                                Err(e) => {
                                    self.run.tasks[index].status =
                                        TaskStatus::Failed(e.to_string());
                                }
                            }
                            let status_label = match &self.run.tasks[index].status {
                                TaskStatus::Completed => "done",
                                TaskStatus::Failed(_) => "failed",
                                _ => "done",
                            };
                            self.emit_event(TeamEvent::Progress(format!(
                                "{agent_name} {status_label} ({completed_count}/{total_count})"
                            )));
                        }
                        Err(join_error) => {
                            // A JoinError means the task panicked or was cancelled.
                            warn!(error = %join_error, "spawned task failed unexpectedly");
                        }
                    }
                    if completed_count >= total_count {
                        break;
                    }
                }
                _ = heartbeat.tick() => {
                    let elapsed = start.elapsed().as_secs();
                    self.emit_event(TeamEvent::Progress(format!(
                        "Specialists working... ({completed_count}/{total_count} done, {elapsed}s elapsed)"
                    )));
                }
            }
        }

        self.emit_event(TeamEvent::Progress(format!(
            "All {total_count} specialists done."
        )));

        // Any tasks still Running after all joins completed must have hit a JoinError.
        for task in &mut self.run.tasks {
            if matches!(task.status, TaskStatus::Running) {
                task.status = TaskStatus::Failed("task panicked or was cancelled".to_string());
            }
        }

        // Phase 4.4: Check for pending grandchild callback tasks (long_running tools).
        // If any exist, the team run should suspend and wait for them to complete.
        let pending_grandchildren = self
            .team_db
            .count_pending_callback_tasks_by_team_run(&self.run.run_id)
            .await
            .unwrap_or(0);

        if pending_grandchildren > 0 {
            info!(
                count = pending_grandchildren,
                "pending grandchild callback tasks detected, suspending team run"
            );

            // Update checkpoint on parent task with current state
            let checkpoint = serialize_checkpoint(&self.run);
            if let Err(e) = self
                .team_db
                .suspend_team_run(&self.run.run_id, &checkpoint)
                .await
            {
                warn!(error = %e, "failed to suspend team run");
            }

            self.run.status = RunStatus::Suspended;
            self.emit_event(TeamEvent::Progress(format!(
                "Team run suspended — waiting for {pending_grandchildren} background task(s)"
            )));
            return Ok(true); // suspended
        }

        // Not suspending — cancel the parent invoke_orchestrator task to prevent it
        // from firing via sibling-completion detection. The synchronous flow will
        // continue directly to review/deliver, so the async resume path must not race.
        if let Err(e) = self
            .team_db
            .update_task_status(&parent_task_id, "cancelled")
            .await
        {
            warn!(
                parent_task_id = %parent_task_id,
                error = %e,
                "failed to cancel parent invoke_orchestrator task"
            );
        } else {
            debug!(
                parent_task_id = %parent_task_id,
                "cancelled parent invoke_orchestrator task (sync completion)"
            );
        }

        Ok(false) // not suspended
    }

    /// Ask the critic agent to review the outputs.
    async fn review(&mut self) -> Result<(bool, String)> {
        // Find the QA/critic agent
        let critic = self
            .team
            .agents
            .iter()
            .find(|a| a.role == "qa" || a.role == "critic");

        let critic_name = match critic {
            Some(a) => a.name.clone(),
            None => {
                // No critic -- auto-approve
                info!("no critic agent, auto-approving");
                return Ok((true, String::new()));
            }
        };

        self.emit_event(TeamEvent::Progress(format!(
            "Critic ({critic_name}) reviewing..."
        )));

        let context = prompt::build_critic_context(&self.team, &self.run);
        let response = self
            .run_agent(&critic_name, "Review the team's outputs.", &context)
            .await?;

        let (approved, feedback) = parse_review_response(&response)?;

        // Persist critic feedback to DB
        if let Some(goal_id) = self.goal_msg_id
            && let Err(e) = self
                .team_db
                .insert_team_workspace_entry(
                    &self.run.run_id,
                    Some(goal_id),
                    Some(&critic_name),
                    "critic",
                    &response,
                    self.run.iteration,
                    Some(&self.trace_id),
                )
                .await
        {
            warn!(error = %e, "failed to persist team workspace entry");
        }

        self.emit_event(TeamEvent::CriticReview {
            approved,
            feedback: feedback.clone(),
            iteration: self.run.iteration,
        });

        // Write critic feedback metadata (overwritten each iteration)
        let verdict = if approved { "APPROVED" } else { "REJECTED" };
        let meta = format!(
            "# Critic Feedback (Iteration {})\n\nVerdict: {}\n\n{}\n",
            self.run.iteration, verdict, feedback
        );
        self.write_metadata_file("critic_feedback.md", &meta);

        Ok((approved, feedback))
    }

    /// Produce the final deliverable.
    async fn deliver(&self) -> Result<String> {
        // Use a writer/communicator agent if one exists, otherwise the orchestrator
        let writer = self
            .team
            .agents
            .iter()
            .find(|a| a.role == "communicator" || a.role == "writer");

        let agent_name = match writer {
            Some(a) => a.name.clone(),
            None => self.team.team.orchestrator.clone(),
        };

        let context = prompt::build_deliverable_context(&self.run);
        let response = self
            .run_agent(&agent_name, "Produce the final deliverable.", &context)
            .await?;

        // Persist deliverable to messages table
        let team_session_id = format!("team-{}", self.run.run_id);
        let metadata = serde_json::json!({
            "team_run_id": self.run.run_id,
            "agent_name": agent_name,
        })
        .to_string();
        if let Err(e) = self
            .team_db
            .save_message_as(
                &agent_name,
                &team_session_id,
                "assistant",
                &response,
                Some(&metadata),
                Some(&self.trace_id),
            )
            .await
        {
            warn!(error = %e, "failed to persist deliverable message");
        }

        Ok(response)
    }

    /// Run an agent with team context and workspace tools.
    /// Returns the agent's text response, or an empty string if the agent
    /// produced no text (tool-use-only turn).
    async fn run_agent(
        &self,
        agent_name: &str,
        task_message: &str,
        team_context: &str,
    ) -> Result<String> {
        let resources = self
            .agents
            .get(agent_name)
            .with_context(|| format!("agent '{}' not found in team resources", agent_name))?;

        // Use the pre-built tool registry (#255)
        let session_id = format!("team-{}-{}", self.run.run_id, agent_name);
        let skills_dirty = AtomicBool::new(false);
        let params = TeamAgentParams {
            db: &resources.db,
            claude: &self.claude,
            tools: &self.tool_registry,
            skills: &resources.skills,
            home_dir: &resources.home_dir,
            task_message,
            team_context,
            session_id: &session_id,
            embedding_client: resources.embedding_client.as_ref(),
            brave_api_key: self.brave_api_key.as_deref(),
            skills_dirty: &skills_dirty,
            mcp_manager: None,
            agent_name,
            child_task_id: None,
            // Team agents communicate through the orchestrator pipeline
            // (workspace entries, deliverables, critic feedback), not
            // directly to users via Telegram.
            message_sender: None,
        };

        Ok(crate::agent::run_team_agent(&params)
            .await?
            .unwrap_or_default())
    }

    /// Format task assignments as markdown for the metadata file.
    fn format_assignments(&self) -> String {
        use std::fmt::Write;
        let mut buf = String::from("# Task Assignments\n\n");
        for task in &self.run.tasks {
            let _ = writeln!(buf, "- **{}** ({}): {}", task.agent, task.role, task.task);
            let _ = writeln!(buf, "  Output: `{}`\n", task.output_file);
        }
        buf
    }

    /// Write a metadata file to the `.meta/` subdirectory of the run workspace.
    fn write_metadata_file(&self, name: &str, content: &str) {
        debug_assert!(
            !name.contains('/') && !name.contains('\\') && !name.contains(".."),
            "metadata file name must be a simple filename: {name}"
        );
        let path = self.workspace_dir.join(".meta").join(name);
        if let Err(e) = std::fs::write(&path, content) {
            warn!(file = name, error = %e, "failed to write metadata file");
        }
    }

    /// Log and emit a phase transition event.
    fn transition_phase(&mut self, phase: TeamPhase) {
        info!(phase = %phase, iteration = self.run.iteration, "team_phase");
        self.emit_event(TeamEvent::PhaseChanged {
            phase,
            iteration: self.run.iteration,
        });
    }

    fn emit_event(&self, event: TeamEvent) {
        // Single log line per event (#435). AgentCompleted / AgentFailed are
        // logged in execute_tasks() spawned tasks (which bypass emit_event).
        // RunFailed is logged in execute() after calling emit_event.
        match &event {
            TeamEvent::Progress(s) => {
                info!(team = %self.run.team_name, "{s}");
            }
            TeamEvent::PhaseChanged { phase, iteration } => {
                info!(team = %self.run.team_name, %phase, iteration, "Phase changed");
            }
            TeamEvent::AgentStarted { agent, role } => {
                info!(team = %self.run.team_name, agent, role, "Agent started");
            }
            TeamEvent::TasksAssigned { .. } => {
                info!(team = %self.run.team_name, "Tasks assigned");
            }
            TeamEvent::AgentCompleted { .. }
            | TeamEvent::AgentFailed { .. }
            | TeamEvent::RunFailed(_) => {
                // Already logged at the call site; skip here to avoid duplicates.
            }
            TeamEvent::CriticReview { approved, .. } => {
                info!(team = %self.run.team_name, approved, "Critic review");
            }
            TeamEvent::Deliverable(_) => {
                info!(team = %self.run.team_name, "Deliverable ready");
            }
        }
        if let Some(cb) = &self.callback {
            cb(event);
        }
    }
}

/// Result of parsing the orchestrator's response.
enum DecomposeResult {
    /// Orchestrator produced valid task assignments.
    Tasks(Vec<TaskAssignment>),
    /// Orchestrator replied conversationally instead of producing tasks.
    Conversational(String),
}

/// Parse the orchestrator's JSON response into task assignments.
///
/// If the orchestrator replied conversationally (via `{"reply": "..."}`),
/// returns `DecomposeResult::Conversational` so the caller can handle it
/// as a deliverable without error-based control flow.
fn parse_task_assignments(response: &str, team: &TeamDefinition) -> Result<DecomposeResult> {
    // Check for conversational reply envelope before trying array parse
    if let Some(json_str) = extract_json(response, '{', '}')
        && let Ok(obj) = serde_json::from_str::<serde_json::Value>(json_str)
        && let Some(reply) = obj.get("reply").and_then(|v| v.as_str())
    {
        return Ok(DecomposeResult::Conversational(reply.to_string()));
    }

    // Try to extract JSON array from the response (#257: unified extract_json)
    let json_str = match extract_json(response, '[', ']') {
        Some(s) => s,
        None => {
            warn!("orchestrator response contains no JSON array, treating as conversational");
            return Ok(DecomposeResult::Conversational(response.to_string()));
        }
    };

    let parsed: Vec<serde_json::Value> = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "failed to parse orchestrator task array, treating as conversational");
            return Ok(DecomposeResult::Conversational(response.to_string()));
        }
    };

    let agent_names: Vec<&str> = team.agents.iter().map(|a| a.name.as_str()).collect();

    let mut tasks = Vec::new();
    for item in parsed {
        let agent_name = item["agent"].as_str().unwrap_or("").to_string();
        let task = item["task"].as_str().unwrap_or("").to_string();
        let output_file = item["output_file"]
            .as_str()
            .unwrap_or("output.md")
            .to_string();

        if agent_name.is_empty() || task.is_empty() {
            continue;
        }

        // #252: Validate agent exists in team
        if !agent_names.contains(&agent_name.as_str()) {
            warn!(agent = %agent_name, "orchestrator assigned task to unknown agent, skipping");
            continue;
        }

        // #252 + #448: Validate output_file (no path traversal, absolute paths, null bytes, or backslashes)
        if output_file.contains("..")
            || output_file.starts_with('/')
            || output_file.contains('\0')
            || output_file.contains('\\')
        {
            warn!(output_file = %output_file, agent = %agent_name, "invalid output_file path, skipping");
            continue;
        }

        // #252: Enforce task length limit (5000 chars)
        let task = if task.len() > 5000 {
            warn!(agent = %agent_name, len = task.len(), "task description exceeds 5000 chars, truncating");
            task[..task.floor_char_boundary(5000)].to_string()
        } else {
            task
        };

        // Look up the role from the team definition
        let role = team
            .agents
            .iter()
            .find(|a| a.name == agent_name)
            .map(|a| a.role.clone())
            .unwrap_or_else(|| "specialist".to_string());

        tasks.push(TaskAssignment {
            agent: agent_name,
            role,
            task,
            output_file,
            status: TaskStatus::Pending,
        });
    }

    if tasks.is_empty() {
        warn!("orchestrator produced no valid task assignments, treating as conversational");
        return Ok(DecomposeResult::Conversational(response.to_string()));
    }

    Ok(DecomposeResult::Tasks(tasks))
}

/// Parse the critic's JSON response.
fn parse_review_response(response: &str) -> Result<(bool, String)> {
    // Try to extract JSON object from the response (#257: unified extract_json)
    let json_str = extract_json(response, '{', '}').unwrap_or(response);

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(parsed) => {
            // #251: Default to false (reject) when approved field is missing
            let approved = parsed["approved"].as_bool().unwrap_or(false);
            let feedback = parsed["feedback"].as_str().unwrap_or("").to_string();
            Ok((approved, feedback))
        }
        Err(_) => {
            // #251: Auto-reject on parse failure instead of auto-approve
            warn!("could not parse critic JSON, auto-rejecting");
            Ok((
                false,
                format!("Critic response was not parseable JSON: {response}"),
            ))
        }
    }
}

/// Extract a JSON structure from text that may contain surrounding prose.
///
/// Uses smarter start patterns to avoid matching brackets in prose:
/// - For arrays (`[`/`]`): looks for `[{` as the start pattern
/// - For objects (`{`/`}`): looks for `{"` as the start pattern
///   Then searches backwards from the end for the matching close bracket. (#257)
fn extract_json(text: &str, open: char, close: char) -> Option<&str> {
    let expected_inner = if open == '[' { '{' } else { '"' };

    // Find the opening bracket where the next non-whitespace char is the expected inner.
    // Handles both compact (`[{`) and pretty-printed (`[\n  {`) JSON.
    let start = text.char_indices().find_map(|(i, c)| {
        if c == open {
            let rest = &text[i + c.len_utf8()..];
            if rest.trim_start().starts_with(expected_inner) {
                return Some(i);
            }
        }
        None
    })?;

    // Search backwards from end for the matching close bracket
    let end = text.rfind(close)?;
    if start < end {
        Some(&text[start..=end])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mika_common::team::*;

    fn test_team() -> TeamDefinition {
        TeamDefinition {
            team: TeamMeta {
                name: "test-team".to_string(),
                orchestrator: "planner".to_string(),
            },
            agents: vec![
                TeamAgent {
                    name: "planner".to_string(),
                    role: "orchestrator".to_string(),
                    mandate: "Plan".to_string(),
                },
                TeamAgent {
                    name: "worker".to_string(),
                    role: "specialist".to_string(),
                    mandate: "Work".to_string(),
                },
            ],
            flow: TeamFlow { max_iterations: 3 },
        }
    }

    #[test]
    fn test_parse_task_assignments_valid() {
        let response =
            r#"[{"agent": "worker", "task": "Do research", "output_file": "research.md"}]"#;
        let team = test_team();
        let result = parse_task_assignments(response, &team).unwrap();
        let tasks = match result {
            DecomposeResult::Tasks(tasks) => tasks,
            DecomposeResult::Conversational(_) => panic!("expected Tasks, got Conversational"),
        };
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].agent, "worker");
        assert_eq!(tasks[0].task, "Do research");
        assert_eq!(tasks[0].output_file, "research.md");
        assert_eq!(tasks[0].role, "specialist");
        assert_eq!(tasks[0].status, TaskStatus::Pending);
    }

    #[test]
    fn test_parse_task_assignments_with_surrounding_text() {
        let response = "Here are the tasks:\n\n[{\"agent\": \"worker\", \"task\": \"Research\", \"output_file\": \"out.md\"}]\n\nGood luck!";
        let team = test_team();
        let result = parse_task_assignments(response, &team).unwrap();
        let tasks = match result {
            DecomposeResult::Tasks(tasks) => tasks,
            DecomposeResult::Conversational(_) => panic!("expected Tasks, got Conversational"),
        };
        assert_eq!(tasks.len(), 1);
    }

    #[test]
    fn test_parse_task_assignments_empty() {
        let response = "[]";
        let team = test_team();
        let result = parse_task_assignments(response, &team).unwrap();
        match result {
            DecomposeResult::Conversational(text) => assert_eq!(text, response),
            DecomposeResult::Tasks(_) => panic!("expected Conversational, got Tasks"),
        }
    }

    #[test]
    fn test_parse_task_assignments_invalid_json() {
        let response = "not json at all";
        let team = test_team();
        let result = parse_task_assignments(response, &team).unwrap();
        match result {
            DecomposeResult::Conversational(text) => assert_eq!(text, response),
            DecomposeResult::Tasks(_) => panic!("expected Conversational, got Tasks"),
        }
    }

    #[test]
    fn test_parse_task_assignments_unknown_agent_skipped() {
        // #252: Tasks assigned to agents not in the team should be skipped
        let response = r#"[{"agent": "unknown", "task": "Do something", "output_file": "out.md"}]"#;
        let team = test_team();
        let result = parse_task_assignments(response, &team).unwrap();
        match result {
            DecomposeResult::Conversational(text) => assert_eq!(text, response),
            DecomposeResult::Tasks(_) => panic!("expected Conversational, got Tasks"),
        }
    }

    #[test]
    fn test_parse_task_assignments_path_traversal_skipped() {
        // #252: output_file with ".." should be skipped
        let response =
            r#"[{"agent": "worker", "task": "Do research", "output_file": "../etc/passwd"}]"#;
        let team = test_team();
        let result = parse_task_assignments(response, &team).unwrap();
        match result {
            DecomposeResult::Conversational(text) => assert_eq!(text, response),
            DecomposeResult::Tasks(_) => panic!("expected Conversational, got Tasks"),
        }
    }

    #[test]
    fn test_parse_task_assignments_absolute_path_skipped() {
        // #252: output_file starting with "/" should be skipped
        let response =
            r#"[{"agent": "worker", "task": "Do research", "output_file": "/tmp/evil.md"}]"#;
        let team = test_team();
        let result = parse_task_assignments(response, &team).unwrap();
        match result {
            DecomposeResult::Conversational(text) => assert_eq!(text, response),
            DecomposeResult::Tasks(_) => panic!("expected Conversational, got Tasks"),
        }
    }

    #[test]
    fn test_parse_task_assignments_pretty_printed() {
        let response = r#"[
  {
    "agent": "worker",
    "task": "Do research",
    "output_file": "research.md"
  }
]"#;
        let team = test_team();
        let result = parse_task_assignments(response, &team).unwrap();
        let tasks = match result {
            DecomposeResult::Tasks(tasks) => tasks,
            DecomposeResult::Conversational(_) => panic!("expected Tasks, got Conversational"),
        };
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].agent, "worker");
    }

    #[test]
    fn test_parse_task_assignments_long_task_truncated() {
        // #252: Tasks longer than 5000 chars get truncated
        let long_task = "x".repeat(6000);
        let response =
            format!(r#"[{{"agent": "worker", "task": "{long_task}", "output_file": "out.md"}}]"#);
        let team = test_team();
        let result = parse_task_assignments(&response, &team).unwrap();
        let tasks = match result {
            DecomposeResult::Tasks(tasks) => tasks,
            DecomposeResult::Conversational(_) => panic!("expected Tasks, got Conversational"),
        };
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].task.len(), 5000);
    }

    #[test]
    fn test_parse_review_approved() {
        let response = r#"{"approved": true, "feedback": "Looks great!"}"#;
        let (approved, feedback) = parse_review_response(response).unwrap();
        assert!(approved);
        assert_eq!(feedback, "Looks great!");
    }

    #[test]
    fn test_parse_review_rejected() {
        let response = r#"{"approved": false, "feedback": "Needs more detail"}"#;
        let (approved, feedback) = parse_review_response(response).unwrap();
        assert!(!approved);
        assert_eq!(feedback, "Needs more detail");
    }

    #[test]
    fn test_parse_review_with_surrounding_text() {
        let response = "After review:\n{\"approved\": true, \"feedback\": \"Good\"}\nDone.";
        let (approved, _) = parse_review_response(response).unwrap();
        assert!(approved);
    }

    #[test]
    fn test_parse_review_invalid_json_rejects() {
        // #251: Parse failure should auto-reject, not auto-approve
        let response = "This is just text, not JSON.";
        let (approved, feedback) = parse_review_response(response).unwrap();
        assert!(!approved);
        assert!(feedback.contains("not parseable JSON"));
    }

    #[test]
    fn test_parse_review_missing_approved_field_rejects() {
        // #251: Missing approved field should default to false
        let response = r#"{"feedback": "some feedback"}"#;
        let (approved, _) = parse_review_response(response).unwrap();
        assert!(!approved);
    }

    #[test]
    fn test_parse_task_assignments_conversational_reply() {
        // Conversational reply envelope should return DecomposeResult::Conversational
        let response = r#"{"reply": "Hey! The team is ready and waiting."}"#;
        let team = test_team();
        let result = parse_task_assignments(response, &team).unwrap();
        match result {
            DecomposeResult::Conversational(reply) => {
                assert!(reply.contains("The team is ready and waiting."));
            }
            DecomposeResult::Tasks(_) => panic!("expected Conversational, got Tasks"),
        }
    }

    #[test]
    fn test_parse_task_assignments_conversational_reply_with_prose() {
        // Conversational reply wrapped in prose should still be detected
        let response =
            "Sure thing!\n{\"reply\": \"Hello! Everything looks good.\"}\nHope that helps.";
        let team = test_team();
        let result = parse_task_assignments(response, &team).unwrap();
        match result {
            DecomposeResult::Conversational(reply) => {
                assert!(reply.contains("Everything looks good."));
            }
            DecomposeResult::Tasks(_) => panic!("expected Conversational, got Tasks"),
        }
    }

    #[test]
    fn test_parse_task_assignments_reply_without_reply_key_is_conversational() {
        // A JSON object without "reply" key and no array falls through to conversational
        let response = r#"{"approved": true, "feedback": "ok"}"#;
        let team = test_team();
        let result = parse_task_assignments(response, &team).unwrap();
        match result {
            DecomposeResult::Conversational(text) => assert_eq!(text, response),
            DecomposeResult::Tasks(_) => panic!("expected Conversational, got Tasks"),
        }
    }

    #[test]
    fn test_parse_task_assignments_prose_fallback() {
        // Orchestrator responds with prose task descriptions instead of JSON
        let response = "Both briefs are live in the workspace. Here's what I've dispatched:\n\n\
            **CTO Agent:** Analyze the technical architecture...\n\n\
            **Quant Agent:** Build the pricing model...";
        let team = test_team();
        let result = parse_task_assignments(response, &team).unwrap();
        match result {
            DecomposeResult::Conversational(text) => {
                assert!(text.contains("Both briefs are live"));
                assert!(text.contains("CTO Agent"));
            }
            DecomposeResult::Tasks(_) => panic!("expected Conversational, got Tasks"),
        }
    }

    #[test]
    fn test_extract_json_array() {
        assert_eq!(
            extract_json("prefix [{\"a\":1}] suffix", '[', ']'),
            Some("[{\"a\":1}]")
        );
        assert_eq!(extract_json("no array here", '[', ']'), None);
    }

    #[test]
    fn test_extract_json_object() {
        assert_eq!(
            extract_json("prefix {\"a\":1} suffix", '{', '}'),
            Some("{\"a\":1}")
        );
        assert_eq!(extract_json("no object here", '{', '}'), None);
    }

    #[test]
    fn test_extract_json_avoids_prose_brackets() {
        // The smarter patterns should skip brackets that don't look like JSON
        // e.g., a lone "[" in prose without a following "{"
        assert_eq!(extract_json("see [1] for details", '[', ']'), None);
        // But should still find actual JSON arrays
        assert_eq!(
            extract_json("result: [{\"x\":1}]", '[', ']'),
            Some("[{\"x\":1}]")
        );
    }
}

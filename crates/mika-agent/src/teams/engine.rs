use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tracing::{Instrument, info, info_span, warn};

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
}

impl TeamEngine {
    /// Create a new engine for a team run.
    pub fn new(
        team: TeamDefinition,
        goal: &str,
        global_home: &Path,
        settings: &Settings,
        callback: Option<TeamEventCallback>,
        team_db: AsyncDatabase,
    ) -> Result<Self> {
        let run_id = uuid::Uuid::new_v4().to_string();
        let team_name = &team.team.name;
        let workspace_dir = mika_common::team::workspace_dir(global_home, team_name);

        // Ensure workspace exists
        std::fs::create_dir_all(&workspace_dir)
            .with_context(|| format!("failed to create workspace {}", workspace_dir.display()))?;

        // Initialize agent resources
        let mut agents = HashMap::new();
        let embedding_client = settings.make_embedding_client();

        for ta in &team.agents {
            let home_dir = agent::agent_dir(global_home, &ta.name);
            let db_path = home_dir.join("data").join("mika.db");
            let db = Database::open(&db_path)
                .with_context(|| format!("failed to open DB for agent '{}'", ta.name))?;
            startup::seed_core_memory_if_empty(&db, &home_dir)?;
            let async_db = AsyncDatabase::new(db);
            let skills = SkillRegistry::from_dir(&home_dir.join("skills"));

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
        for tool in tools::team_tools(&workspace_dir) {
            tool_registry.register(tool);
        }

        let run = TeamRun {
            run_id,
            team_name: team_name.clone(),
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
            workspace_dir,
            agents: Arc::new(agents),
            claude,
            tool_registry: Arc::new(tool_registry),
            callback: callback.map(Arc::new),
            brave_api_key: settings.brave_api_key.clone(),
            team_db,
            goal_msg_id: None,
        })
    }

    /// Execute the full team orchestration flow.
    pub async fn execute(mut self) -> Result<TeamRun> {
        info!(
            team = %self.run.team_name,
            run_id = %self.run.run_id,
            "team_run started"
        );

        // Persist the team run to DB
        if let Err(e) = self
            .team_db
            .insert_team_run(
                &self.run.run_id,
                &self.run.team_name,
                &self.run.goal,
                self.run.max_iterations,
                self.run.started_at,
            )
            .await
        {
            warn!(error = %e, "failed to persist team run to DB");
        }

        // Insert the goal as the root message
        match self
            .team_db
            .insert_team_message(&self.run.run_id, None, None, "goal", &self.run.goal, 0)
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
                self.run.status = RunStatus::Completed;
            }
            Err(e) => {
                self.run.status = RunStatus::Failed(e.to_string());
                self.emit_event(TeamEvent::RunFailed(e.to_string()));

                // Persist error message to DB
                if let Some(goal_id) = self.goal_msg_id
                    && let Err(e) = self
                        .team_db
                        .insert_team_message(
                            &self.run.run_id,
                            Some(goal_id),
                            None,
                            "error",
                            &e.to_string(),
                            self.run.iteration,
                        )
                        .await
                {
                    warn!(error = %e, "failed to persist team message");
                }
            }
        }

        let ended_at = chrono::Utc::now().timestamp();
        self.run.ended_at = Some(ended_at);

        // Update run status in DB
        let (status_str, failure_reason) = match &self.run.status {
            RunStatus::Running => ("running", None),
            RunStatus::Completed => ("completed", None),
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
                Some(ended_at),
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

        match result {
            Ok(_) => Ok(self.run),
            Err(e) => {
                // Return the run even on failure (it has status info)
                warn!(error = %e, "team execution failed");
                Ok(self.run)
            }
        }
    }

    async fn execute_inner(&mut self) -> Result<()> {
        // Load recent conversation history for orchestrator context
        let history = self
            .team_db
            .load_team_runs_for_prompt(&self.run.team_name, 10, 500)
            .await
            .unwrap_or_default();

        // Step 1: Decompose -- orchestrator produces task assignments
        info!(phase = "decompose", iteration = 1, "team_phase");
        self.emit_event(TeamEvent::PhaseChanged {
            phase: TeamPhase::Decompose,
            iteration: 1,
        });
        self.emit_event(TeamEvent::Progress("Decomposing goal...".to_string()));
        match self.decompose(None, &history).await? {
            DecomposeResult::Tasks(tasks) => {
                self.run.tasks = tasks;
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
            self.run.iteration += 1;

            // Step 2: Execute -- run each specialist
            info!(
                phase = "execute",
                iteration = self.run.iteration,
                "team_phase"
            );
            self.emit_event(TeamEvent::PhaseChanged {
                phase: TeamPhase::Execute,
                iteration: self.run.iteration,
            });
            self.execute_tasks().await?;

            // Step 3: Review -- critic evaluates outputs
            info!(
                phase = "review",
                iteration = self.run.iteration,
                "team_phase"
            );
            self.emit_event(TeamEvent::PhaseChanged {
                phase: TeamPhase::Review,
                iteration: self.run.iteration,
            });
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
            info!(
                phase = "re-decompose",
                iteration = self.run.iteration,
                "team_phase"
            );
            self.emit_event(TeamEvent::PhaseChanged {
                phase: TeamPhase::ReDecompose,
                iteration: self.run.iteration,
            });
            self.emit_event(TeamEvent::Progress(format!(
                "Iteration {}: critic requested revisions...",
                self.run.iteration
            )));
            match self.decompose(Some(&feedback), &history).await? {
                DecomposeResult::Tasks(tasks) => {
                    self.run.tasks = tasks;
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
        }

        // Step 4: Deliver -- produce final output
        info!(
            phase = "deliver",
            iteration = self.run.iteration,
            "team_phase"
        );
        self.emit_event(TeamEvent::PhaseChanged {
            phase: TeamPhase::Deliver,
            iteration: self.run.iteration,
        });
        self.emit_event(TeamEvent::Progress(
            "Producing final deliverable...".to_string(),
        ));
        let deliverable = self.deliver().await?;
        self.run.deliverable = Some(deliverable.clone());

        self.emit_event(TeamEvent::Deliverable(deliverable));

        Ok(())
    }

    /// Ask the orchestrator to decompose the goal into tasks.
    /// Pass `None` for first decomposition, `Some(feedback)` for re-decomposition
    /// after critic rejection. (#261: merged decompose + decompose_with_feedback)
    async fn decompose(
        &self,
        feedback: Option<&str>,
        history: &[crate::db::TeamRunRow],
    ) -> Result<DecomposeResult> {
        let listing = prompt::workspace_listing(&self.workspace_dir);
        let context = prompt::build_orchestrator_context(&self.team, &listing, feedback, history);

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
                        .insert_team_message(
                            &self.run.run_id,
                            Some(goal_id),
                            Some(orchestrator_name),
                            "orchestrator",
                            &response,
                            iteration,
                        )
                        .await
                {
                    warn!(error = %e, "failed to persist team message");
                }
                return Ok(DecomposeResult::Conversational(reply));
            }
            DecomposeResult::Tasks(tasks) => tasks,
        };

        // Persist orchestrator decomposition to DB
        if let Some(goal_id) = self.goal_msg_id {
            let orchestrator_msg_id = self
                .team_db
                .insert_team_message(
                    &self.run.run_id,
                    Some(goal_id),
                    Some(orchestrator_name),
                    "orchestrator",
                    &response,
                    iteration,
                )
                .await
                .ok();

            // Insert assignment messages as children of orchestrator
            if let Some(orch_id) = orchestrator_msg_id {
                for task in &tasks {
                    if let Err(e) = self
                        .team_db
                        .insert_team_message(
                            &self.run.run_id,
                            Some(orch_id),
                            Some(&task.agent),
                            "assignment",
                            &task.task,
                            iteration,
                        )
                        .await
                    {
                        warn!(error = %e, "failed to persist team message");
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
    async fn execute_tasks(&mut self) -> Result<()> {
        // Prepare shared resources that will be moved into spawned tasks.
        let agents = Arc::clone(&self.agents);
        let claude = self.claude.clone();
        let tool_registry = Arc::clone(&self.tool_registry);
        let run_id = self.run.run_id.clone();
        let callback = self.callback.clone();
        let team_name = self.run.team_name.clone();
        let brave_api_key = self.brave_api_key.clone();
        let team_db = self.team_db.clone();
        let iteration = self.run.iteration;

        // Look up assignment message IDs from DB for parent linking.
        let assignment_msg_ids: HashMap<String, i64> = self
            .team_db
            .load_assignment_msg_ids(&self.run.run_id, iteration)
            .await
            .unwrap_or_default();
        let assignment_msg_ids = Arc::new(assignment_msg_ids);

        // Build per-task parameters upfront from self (avoids borrowing self in spawned tasks).
        struct TaskInput {
            index: usize,
            agent_name: String,
            role: String,
            task_desc: String,
            context: String,
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
            });
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
            let assignment_msg_ids = Arc::clone(&assignment_msg_ids);

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
                            };
                            crate::agent::run_team_agent(&params)
                                .await
                                .map(|opt| opt.unwrap_or_default())
                        }
                        Err(e) => Err(e),
                    };

                    // Persist and report completion/failure for this agent.
                    let parent_id = assignment_msg_ids.get(agent_name.as_str()).copied();

                    match &result {
                        Ok(response) => {
                            info!(agent = %agent_name, "task completed");
                            if let Some(ref cb) = callback {
                                cb(TeamEvent::AgentCompleted {
                                    agent: agent_name.to_string(),
                                    response: response.clone(),
                                });
                            }
                            // Persist agent response
                            if let Err(e) = team_db
                                .insert_team_message(
                                    &run_id,
                                    parent_id,
                                    Some(agent_name),
                                    "agent_response",
                                    response,
                                    iteration,
                                )
                                .await
                            {
                                warn!(error = %e, "failed to persist team message");
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
                            // Persist agent error
                            if let Err(e) = team_db
                                .insert_team_message(
                                    &run_id,
                                    parent_id,
                                    Some(agent_name),
                                    "error",
                                    &error_str,
                                    iteration,
                                )
                                .await
                            {
                                warn!(error = %e, "failed to persist team message");
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

        Ok(())
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
                .insert_team_message(
                    &self.run.run_id,
                    Some(goal_id),
                    Some(&critic_name),
                    "critic",
                    &response,
                    self.run.iteration,
                )
                .await
        {
            warn!(error = %e, "failed to persist team message");
        }

        self.emit_event(TeamEvent::CriticReview {
            approved,
            feedback: feedback.clone(),
            iteration: self.run.iteration,
        });

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

        // Persist deliverable to DB
        if let Some(goal_id) = self.goal_msg_id
            && let Err(e) = self
                .team_db
                .insert_team_message(
                    &self.run.run_id,
                    Some(goal_id),
                    Some(&agent_name),
                    "deliverable",
                    &response,
                    self.run.iteration,
                )
                .await
        {
            warn!(error = %e, "failed to persist team message");
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
        };

        Ok(crate::agent::run_team_agent(&params)
            .await?
            .unwrap_or_default())
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

use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{info, warn};

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

use super::history;
use super::prompt;
use super::types::*;

/// Resources needed to run a specific agent.
struct AgentResources {
    db: AsyncDatabase,
    skills: SkillRegistry,
    home_dir: PathBuf,
    embedding_client: Option<EmbeddingClient>,
}

/// Progress callback type for team execution reporting.
pub type ProgressCallback = Box<dyn Fn(&str) + Send + Sync>;

/// The team orchestration engine.
pub struct TeamEngine {
    team: TeamDefinition,
    run: TeamRun,
    workspace_dir: PathBuf,
    history_dir: PathBuf,
    agents: Arc<HashMap<String, AgentResources>>,
    claude: ClaudeClient,
    tool_registry: Arc<ToolRegistry>,
    progress: Option<Arc<ProgressCallback>>,
}

impl TeamEngine {
    /// Create a new engine for a team run.
    pub fn new(
        team: TeamDefinition,
        goal: &str,
        global_home: &Path,
        settings: &Settings,
        progress: Option<ProgressCallback>,
    ) -> Result<Self> {
        let run_id = uuid::Uuid::new_v4().to_string();
        let team_name = &team.team.name;
        let workspace_dir = mika_common::team::workspace_dir(global_home, team_name);
        let history_dir = mika_common::team::history_dir(global_home, team_name);

        // Ensure workspace exists
        std::fs::create_dir_all(&workspace_dir)
            .with_context(|| format!("failed to create workspace {}", workspace_dir.display()))?;

        // Initialize agent resources
        let mut agents = HashMap::new();
        let embedding_client = settings.make_embedding_client();

        crate::db::init_sqlite_vec();

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
            started_at: chrono::Utc::now().to_rfc3339(),
            ended_at: None,
            deliverable: None,
        };

        Ok(Self {
            team,
            run,
            workspace_dir,
            history_dir,
            agents: Arc::new(agents),
            claude,
            tool_registry: Arc::new(tool_registry),
            progress: progress.map(Arc::new),
        })
    }

    /// Execute the full team orchestration flow.
    pub async fn execute(mut self) -> Result<TeamRun> {
        let result = self.execute_inner().await;

        match &result {
            Ok(_) => {
                self.run.status = RunStatus::Completed;
            }
            Err(e) => {
                self.run.status = RunStatus::Failed(e.to_string());
            }
        }

        self.run.ended_at = Some(chrono::Utc::now().to_rfc3339());

        // Save to history regardless of outcome
        if let Err(e) = history::save_run(&self.history_dir, &self.run) {
            warn!(error = %e, "failed to save team run history");
        }

        // Shutdown all AsyncDatabase instances to avoid thread leaks (#256)
        for resources in self.agents.values() {
            resources.db.shutdown();
        }

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
        // Step 1: Decompose -- orchestrator produces task assignments
        self.report_progress("Decomposing goal...");
        let tasks = self.decompose(None).await?;
        self.run.tasks = tasks;

        // Iterate: execute -> review -> (retry if rejected)
        loop {
            self.run.iteration += 1;

            // Step 2: Execute -- run each specialist
            self.execute_tasks().await?;

            // Step 3: Review -- critic evaluates outputs
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
            self.report_progress(&format!(
                "Iteration {}: critic requested revisions...",
                self.run.iteration
            ));
            let tasks = self.decompose(Some(&feedback)).await?;
            self.run.tasks = tasks;
        }

        // Step 4: Deliver -- produce final output
        self.report_progress("Producing final deliverable...");
        let deliverable = self.deliver().await?;
        self.run.deliverable = Some(deliverable);

        Ok(())
    }

    /// Ask the orchestrator to decompose the goal into tasks.
    /// Pass `None` for first decomposition, `Some(feedback)` for re-decomposition
    /// after critic rejection. (#261: merged decompose + decompose_with_feedback)
    async fn decompose(&self, feedback: Option<&str>) -> Result<Vec<TaskAssignment>> {
        let listing = prompt::workspace_listing(&self.workspace_dir);
        let context = prompt::build_orchestrator_context(&self.team, &listing, feedback);

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

        parse_task_assignments(&response, &self.team)
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
        let progress = self.progress.clone();
        let team_name = self.run.team_name.clone();

        // Build per-task parameters upfront from self (avoids borrowing self in spawned tasks).
        struct TaskInput {
            index: usize,
            agent_name: String,
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
                task_desc: task.task.clone(),
                context,
            });
        }

        self.report_progress(&format!(
            "Running {} specialist agents in parallel...",
            inputs.len()
        ));

        // Spawn all tasks concurrently via JoinSet.
        let mut join_set = tokio::task::JoinSet::new();

        for input in inputs {
            let agents = Arc::clone(&agents);
            let claude = claude.clone();
            let tool_registry = Arc::clone(&tool_registry);
            let run_id = run_id.clone();
            let progress = progress.clone();
            let team_name = team_name.clone();

            join_set.spawn(async move {
                let agent_name = &input.agent_name;

                // Report that this agent is starting.
                info!(team = %team_name, "Running {agent_name}...");
                if let Some(ref cb) = progress {
                    cb(&format!("Running {agent_name}..."));
                }

                let resources = agents
                    .get(agent_name.as_str())
                    .with_context(|| format!("agent '{}' not found in team resources", agent_name));

                let result = match resources {
                    Ok(resources) => {
                        let session_id = format!("team-{}-{}", run_id, agent_name);
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
                        };
                        crate::agent::run_team_agent(&params).await
                    }
                    Err(e) => Err(e),
                };

                // Report completion/failure for this agent.
                match &result {
                    Ok(_) => {
                        info!(agent = %agent_name, "task completed");
                        if let Some(ref cb) = progress {
                            cb(&format!("{agent_name} completed"));
                        }
                    }
                    Err(e) => {
                        warn!(agent = %agent_name, error = %e, "task failed");
                        if let Some(ref cb) = progress {
                            cb(&format!("{agent_name} failed: {e}"));
                        }
                    }
                }

                (input.index, input.agent_name, result)
            });
        }

        // Collect results as tasks complete and update statuses.
        while let Some(join_result) = join_set.join_next().await {
            match join_result {
                Ok((index, _agent_name, result)) => match result {
                    Ok(_) => {
                        self.run.tasks[index].status = TaskStatus::Completed;
                    }
                    Err(e) => {
                        self.run.tasks[index].status = TaskStatus::Failed(e.to_string());
                    }
                },
                Err(join_error) => {
                    // A JoinError means the task panicked or was cancelled.
                    warn!(error = %join_error, "spawned task failed unexpectedly");
                }
            }
        }

        Ok(())
    }

    /// Ask the critic agent to review the outputs.
    async fn review(&self) -> Result<(bool, String)> {
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

        self.report_progress(&format!("Critic ({critic_name}) reviewing..."));

        let context = prompt::build_critic_context(&self.team, &self.run);
        let response = self
            .run_agent(&critic_name, "Review the team's outputs.", &context)
            .await?;

        parse_review_response(&response)
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

        Ok(response)
    }

    /// Run an agent with team context and workspace tools.
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
        };

        crate::agent::run_team_agent(&params).await
    }

    fn report_progress(&self, message: &str) {
        info!(team = %self.run.team_name, "{message}");
        if let Some(cb) = &self.progress {
            cb(message);
        }
    }
}

/// Parse the orchestrator's JSON response into task assignments.
fn parse_task_assignments(response: &str, team: &TeamDefinition) -> Result<Vec<TaskAssignment>> {
    // Try to extract JSON array from the response (#257: unified extract_json)
    let json_str = extract_json(response, '[', ']').unwrap_or(response);

    let parsed: Vec<serde_json::Value> = serde_json::from_str(json_str)
        .with_context(|| format!("failed to parse orchestrator task assignments: {response}"))?;

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

        // #252: Validate output_file (no path traversal or absolute paths)
        if output_file.contains("..") || output_file.starts_with('/') {
            warn!(output_file = %output_file, agent = %agent_name, "invalid output_file path, skipping");
            continue;
        }

        // #252: Enforce task length limit (5000 chars)
        let task = if task.len() > 5000 {
            warn!(agent = %agent_name, len = task.len(), "task description exceeds 5000 chars, truncating");
            task[..5000].to_string()
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
        bail!("Orchestrator produced no valid task assignments");
    }

    Ok(tasks)
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
/// Then searches backwards from the end for the matching close bracket. (#257)
fn extract_json(text: &str, open: char, close: char) -> Option<&str> {
    // Build a smarter start pattern to reduce false positives
    let start_pattern = if open == '[' { "[{" } else { "{\"" };

    let start = text.find(start_pattern)?;

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
        let tasks = parse_task_assignments(response, &team).unwrap();
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
        let tasks = parse_task_assignments(response, &team).unwrap();
        assert_eq!(tasks.len(), 1);
    }

    #[test]
    fn test_parse_task_assignments_empty() {
        let response = "[]";
        let team = test_team();
        let result = parse_task_assignments(response, &team);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_task_assignments_invalid_json() {
        let response = "not json at all";
        let team = test_team();
        let result = parse_task_assignments(response, &team);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_task_assignments_unknown_agent_skipped() {
        // #252: Tasks assigned to agents not in the team should be skipped
        let response = r#"[{"agent": "unknown", "task": "Do something", "output_file": "out.md"}]"#;
        let team = test_team();
        let result = parse_task_assignments(response, &team);
        assert!(result.is_err()); // No valid tasks remain
    }

    #[test]
    fn test_parse_task_assignments_path_traversal_skipped() {
        // #252: output_file with ".." should be skipped
        let response =
            r#"[{"agent": "worker", "task": "Do research", "output_file": "../etc/passwd"}]"#;
        let team = test_team();
        let result = parse_task_assignments(response, &team);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_task_assignments_absolute_path_skipped() {
        // #252: output_file starting with "/" should be skipped
        let response =
            r#"[{"agent": "worker", "task": "Do research", "output_file": "/tmp/evil.md"}]"#;
        let team = test_team();
        let result = parse_task_assignments(response, &team);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_task_assignments_long_task_truncated() {
        // #252: Tasks longer than 5000 chars get truncated
        let long_task = "x".repeat(6000);
        let response =
            format!(r#"[{{"agent": "worker", "task": "{long_task}", "output_file": "out.md"}}]"#);
        let team = test_team();
        let tasks = parse_task_assignments(&response, &team).unwrap();
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

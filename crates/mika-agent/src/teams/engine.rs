use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
    agents: HashMap<String, AgentResources>,
    claude: ClaudeClient,
    progress: Option<ProgressCallback>,
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

        let run = TeamRun {
            run_id,
            team_name: team_name.clone(),
            goal: goal.to_string(),
            status: RunStatus::Running,
            current_step: "initialize".to_string(),
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
            agents,
            claude,
            progress,
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
        // Step 1: Decompose — orchestrator produces task assignments
        self.report_progress("Decomposing goal...");
        self.run.current_step = "decompose".to_string();
        let tasks = self.decompose().await?;
        self.run.tasks = tasks;

        // Iterate: execute → review → (retry if rejected)
        loop {
            self.run.iteration += 1;

            // Step 2: Execute — run each specialist
            self.run.current_step = "execute".to_string();
            self.execute_tasks().await?;

            // Step 3: Review — critic evaluates outputs
            self.run.current_step = "review".to_string();
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
            let tasks = self.decompose_with_feedback(&feedback).await?;
            self.run.tasks = tasks;
        }

        // Step 4: Deliver — produce final output
        self.run.current_step = "deliver".to_string();
        self.report_progress("Producing final deliverable...");
        let deliverable = self.deliver().await?;
        self.run.deliverable = Some(deliverable);

        Ok(())
    }

    /// Ask the orchestrator to decompose the goal into tasks.
    async fn decompose(&self) -> Result<Vec<TaskAssignment>> {
        let listing = prompt::workspace_listing(&self.workspace_dir);
        let context = prompt::build_orchestrator_context(&self.team, "decompose", &listing, None);

        let orchestrator_name = &self.team.team.orchestrator;
        let response = self
            .run_agent(orchestrator_name, &self.run.goal, &context)
            .await?;

        parse_task_assignments(&response, &self.team)
    }

    /// Re-decompose with feedback from the critic.
    async fn decompose_with_feedback(&self, feedback: &str) -> Result<Vec<TaskAssignment>> {
        let listing = prompt::workspace_listing(&self.workspace_dir);
        let context =
            prompt::build_orchestrator_context(&self.team, "execute", &listing, Some(feedback));

        let orchestrator_name = &self.team.team.orchestrator;
        let msg = format!(
            "The previous attempt was rejected by the critic. Revise the task assignments.\n\nOriginal goal: {}",
            self.run.goal
        );
        let response = self.run_agent(orchestrator_name, &msg, &context).await?;

        parse_task_assignments(&response, &self.team)
    }

    /// Execute all tasks by delegating to specialist agents.
    async fn execute_tasks(&mut self) -> Result<()> {
        for i in 0..self.run.tasks.len() {
            // Clone values from task before mutating
            let agent_name = self.run.tasks[i].agent.clone();
            let role = self.run.tasks[i].role.clone();
            let task_desc = self.run.tasks[i].task.clone();
            let output_file = self.run.tasks[i].output_file.clone();

            self.report_progress(&format!("Running {agent_name}..."));
            self.run.tasks[i].status = TaskStatus::Running;

            let mandate = self
                .team
                .agents
                .iter()
                .find(|a| a.name == agent_name)
                .map(|a| a.mandate.as_str())
                .unwrap_or("Complete the assigned task");

            let context =
                prompt::build_specialist_context(&role, mandate, &task_desc, &output_file);

            match self.run_agent(&agent_name, &task_desc, &context).await {
                Ok(_response) => {
                    self.run.tasks[i].status = TaskStatus::Completed;
                    info!(agent = %agent_name, "task completed");
                }
                Err(e) => {
                    self.run.tasks[i].status = TaskStatus::Failed(e.to_string());
                    warn!(agent = %agent_name, error = %e, "task failed");
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
                // No critic — auto-approve
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

        // Build a tool registry with base tools + workspace tools
        let mut registry = tools::default_tools();
        for tool in tools::team_tools(&self.workspace_dir) {
            registry.register(tool);
        }

        let session_id = format!("team-{}-{}", self.run.run_id, agent_name);
        let params = TeamAgentParams {
            db: &resources.db,
            claude: &self.claude,
            tools: &registry,
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
    // Try to extract JSON array from the response
    let json_str = extract_json_array(response).unwrap_or(response);

    let parsed: Vec<serde_json::Value> = serde_json::from_str(json_str)
        .with_context(|| format!("failed to parse orchestrator task assignments: {response}"))?;

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
    // Try to extract JSON object from the response
    let json_str = extract_json_object(response).unwrap_or(response);

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(parsed) => {
            let approved = parsed["approved"].as_bool().unwrap_or(true);
            let feedback = parsed["feedback"].as_str().unwrap_or("").to_string();
            Ok((approved, feedback))
        }
        Err(_) => {
            // If we can't parse JSON, treat it as approved with the text as feedback
            warn!("could not parse critic JSON, auto-approving");
            Ok((true, response.to_string()))
        }
    }
}

/// Extract a JSON array from text that may contain surrounding prose.
fn extract_json_array(text: &str) -> Option<&str> {
    let start = text.find('[')?;
    let end = text.rfind(']')?;
    if start < end {
        Some(&text[start..=end])
    } else {
        None
    }
}

/// Extract a JSON object from text that may contain surrounding prose.
fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
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
            flow: TeamFlow {
                steps: vec!["decompose".to_string(), "execute".to_string()],
                max_iterations: 3,
            },
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
    fn test_parse_review_invalid_json_auto_approves() {
        let response = "This is just text, not JSON.";
        let (approved, _) = parse_review_response(response).unwrap();
        assert!(approved);
    }

    #[test]
    fn test_extract_json_array() {
        assert_eq!(
            extract_json_array("prefix [{\"a\":1}] suffix"),
            Some("[{\"a\":1}]")
        );
        assert_eq!(extract_json_array("no array here"), None);
    }

    #[test]
    fn test_extract_json_object() {
        assert_eq!(
            extract_json_object("prefix {\"a\":1} suffix"),
            Some("{\"a\":1}")
        );
        assert_eq!(extract_json_object("no object here"), None);
    }
}

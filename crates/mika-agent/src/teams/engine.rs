use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tracing::{Instrument, debug, info, info_span, warn};

use mika_common::agent;
use mika_common::config::Settings;
use mika_common::embedding::EmbeddingClient;
use mika_common::llm::LlmProvider;
use mika_common::team::TeamDefinition;
use secrecy::ExposeSecret;

use crate::agent::{
    DelegationOutcome, TeamAgentOutcome, TeamAgentParams, classify_delegation_from_error,
};
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

/// Team-run stuck-detection threshold (mika#1652). A run in `status='running'`
/// older than this with no child-session liveness is considered orphaned.
///
/// 20 min captures "genuinely wedged" without false-positing on slow-but-
/// legitimate multi-agent coordination — members may legitimately go quiet
/// while waiting on workspace file I/O. 10× the 300s `run_team` tool timeout
/// (50 min) would be too conservative for the founding-incident shape (team
/// `d68fdcaf`, 2026-06-29: a2a_call 503s left a row stuck `running` for 80+ min).
const TEAM_RUN_STUCK_THRESHOLD_SECS: i64 = 20 * 60;

/// Liveness window for team child sessions (mika#1652). Any `llm_calls` or
/// `tool_calls` activity on a `team-<id>%` session within this window proves
/// the run is alive. 5 min matches the engine's periodic scan cadence with
/// headroom.
const TEAM_RUN_LIVENESS_THRESHOLD_SECS: i64 = 5 * 60;

/// Resources needed to run a specific agent.
struct AgentResources {
    db: AsyncDatabase,
    skills: SkillRegistry,
    home_dir: PathBuf,
    embedding_client: Option<EmbeddingClient>,
    /// Per-agent settings loaded via `Settings::load_for_agent()` cascade:
    /// per-agent config.toml > global config.toml > defaults > env vars.
    settings: Settings,
    /// Per-agent LLM provider constructed from this agent's cascaded settings.
    llm: Arc<dyn LlmProvider>,
}

/// Build the team-mode tool registry: base tools + workspace tools, with the
/// team-mode suppression list applied (mika#1653).
///
/// `a2a_call` is removed because no agent inside a team run — orchestrator or
/// member — has a valid use for it: local team members are routed by name via
/// the decompose→spawn→resume cycle, and `a2a_call` is URL-addressed and
/// structurally 503s on local siblings (no gateway route exists for
/// `~/.mika/agents/<name>`). Suppression is scoped to this private registry, so
/// conversation-mode agents (which legitimately reach external A2A agents) are
/// unaffected. This is the structural enforcement behind the prompt guidance in
/// `teams::prompt::build_orchestrator_context` — with the tool absent from the
/// array, the orchestrator cannot mis-reach regardless of prompt drift.
///
/// The `TEAM_SUPPRESSED_TOOLS` invariant is regression-gated by
/// `test_team_registry_suppresses_a2a_call` below — a future `default_tools()`
/// change cannot silently re-introduce the affordance.
fn build_team_tool_registry(
    workspace_dir: &Path,
    reference_workspace: Option<&Path>,
) -> ToolRegistry {
    let mut registry = tools::default_tools();
    for tool in tools::team_tools(workspace_dir, reference_workspace) {
        registry.register(tool);
    }
    for name in TEAM_SUPPRESSED_TOOLS {
        registry.remove(name);
    }
    registry
}

/// Tools removed from the team-mode tool array (mika#1653). See
/// [`build_team_tool_registry`] for the rationale.
const TEAM_SUPPRESSED_TOOLS: &[&str] = &["a2a_call"];

/// Shared initialization resources produced by [`TeamEngine::init_resources`].
struct EngineResources {
    agents: HashMap<String, AgentResources>,
    tool_registry: ToolRegistry,
    workspace_dir: PathBuf,
}

/// The team orchestration engine.
pub struct TeamEngine {
    team: TeamDefinition,
    run: TeamRun,
    workspace_dir: PathBuf,
    agents: Arc<HashMap<String, AgentResources>>,
    tool_registry: Arc<ToolRegistry>,
    callback: Option<Arc<TeamEventCallback>>,
    brave_api_key: Option<String>,
    github_token: Option<String>,
    /// Gateway base URL for builtins that call substrate endpoints
    /// (mika#1969). Threaded into every team-member `TeamAgentParams`.
    gateway_url: Option<String>,
    /// Bearer token companion to `gateway_url` (mika#1969).
    internal_token: Option<String>,
    github_app: Option<Arc<mika_common::github_app::GitHubApp>>,
    /// Team-level database for persisting runs and messages.
    team_db: AsyncDatabase,
    /// Message ID of the root goal message (set after insert).
    goal_msg_id: Option<i64>,
    /// Trace ID for correlating all events within this team run.
    trace_id: String,
    /// If set, use this specific run's summary instead of auto-detecting the last completed run.
    reference_run_id: Option<String>,
    /// Session-scoped PR review dedup map (#821). Shared with `AppState`.
    /// Entries evicted at each `end_session()` callsite.
    pr_reviews_posted: Option<Arc<dashmap::DashMap<String, std::collections::HashSet<String>>>>,
}

/// Outcome of an `execute_tasks` iteration (mika#1671). Replaces the previous
/// `bool` (suspended) return so the caller can distinguish a normal proceed, a
/// grandchild-callback suspend, and the all-transport-failed short-circuit.
enum ExecuteOutcome {
    /// Proceed to the review/deliver phases (current default behavior).
    Proceed,
    /// Pending grandchild callbacks — the run suspended and will resume later.
    Suspended,
    /// Every completed delegation in this iteration failed at the transport layer;
    /// `run.status` has been set to `RunStatus::FailedTransport` and the caller must
    /// skip review/deliver and finalize (mika#1671 AC3/AC5).
    AllTransportFailed,
}

/// True iff the iteration is non-empty and every delegation classified as a
/// transport failure (mika#1671 AC3). A `None` slot (unclassified — e.g. a task
/// that panicked into a `JoinError`) or any non-transport outcome makes this
/// `false`, so a mixed iteration never short-circuits (AC4 invariant).
fn all_delegations_transport_failed(outcomes: &[Option<DelegationOutcome>]) -> bool {
    !outcomes.is_empty()
        && outcomes
            .iter()
            .all(|o| matches!(o, Some(DelegationOutcome::TransportError { .. })))
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
            let mut db = Database::open(&db_path)
                .with_context(|| format!("failed to open container DB for agent '{}'", ta.name))?;
            let identity = crate::prompt::load_identity(&home_dir);
            db.register_agent(&ta.name, &identity.name, home_dir.to_str().unwrap_or(""))?;
            startup::seed_core_memory_if_empty(&db, &home_dir, &ta.name)?;
            let skills_dir = home_dir.join("skills");
            // Migrate legacy .disabled marker files to DB (one-shot, idempotent).
            if let Err(e) = crate::skills::migrate_disabled_markers(&skills_dir, &mut db, &ta.name)
            {
                tracing::warn!(agent = %ta.name, error = %e, "failed to migrate .disabled markers");
            }
            let mut skills = SkillRegistry::from_dir(&skills_dir);
            if let Some(ref allowlist) = identity.skills.allowlist {
                skills.apply_identity_allowlist(allowlist);
            }
            if let Ok(overrides) = db.get_skill_overrides(&ta.name) {
                skills.apply_overrides(&overrides);
            }
            // Phase 2 (mika#1798): testimony-grade ban.
            skills.apply_testimony_grade_ban();
            skills.log_summary();
            let async_db = AsyncDatabase::new_with_agent(db, &ta.name);

            // Per-agent config cascade: per-agent config.toml > global config.toml > env vars.
            // This ensures each team agent uses its own LLM provider/model settings (#285).
            let agent_settings = Settings::load_for_agent(global_home, &home_dir)
                .with_context(|| format!("failed to load config for agent '{}'", ta.name))?;
            let agent_llm = agent_settings.make_llm_provider().with_context(|| {
                format!("failed to create LLM provider for agent '{}'", ta.name)
            })?;

            agents.insert(
                ta.name.clone(),
                AgentResources {
                    db: async_db,
                    skills,
                    home_dir,
                    embedding_client: embedding_client.clone(),
                    settings: agent_settings,
                    llm: agent_llm,
                },
            );
        }

        // Build tool registry once with base tools + workspace tools (#255),
        // with the team-mode suppression applied (mika#1653).
        let tool_registry =
            build_team_tool_registry(&workspace_dir, reference_workspace.as_deref());

        Ok(EngineResources {
            agents,
            tool_registry,
            workspace_dir,
        })
    }

    /// Create a new engine for a team run.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        team: TeamDefinition,
        goal: &str,
        global_home: &Path,
        settings: &Settings,
        callback: Option<TeamEventCallback>,
        team_db: AsyncDatabase,
        reference_run_id: Option<&str>,
        github_app: Option<Arc<mika_common::github_app::GitHubApp>>,
        pr_reviews_posted: Option<Arc<dashmap::DashMap<String, std::collections::HashSet<String>>>>,
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
            started_at: crate::timestamp::now(),
            ended_at: None,
            deliverable: None,
            coverage_retry_fired: false,
            conversational_retry_fired: false,
            delegation_count: 0,
            solo_absorption: false,
            failure_context: None,
        };

        Ok(Self {
            team,
            run,
            workspace_dir: res.workspace_dir,
            agents: Arc::new(res.agents),
            tool_registry: Arc::new(res.tool_registry),
            callback: callback.map(Arc::new),
            brave_api_key: settings
                .brave_api_key
                .as_ref()
                .map(|s| s.expose_secret().to_string()),
            github_token: settings.agent_github_token().map(String::from),
            gateway_url: settings.routing_url.clone(),
            internal_token: settings
                .internal_token
                .as_ref()
                .map(|s| s.expose_secret().to_string()),
            github_app,
            team_db,
            goal_msg_id: None,
            trace_id: mika_common::trace::generate_trace_id(),
            reference_run_id: reference_run_id.map(|s| s.to_string()),
            pr_reviews_posted,
        })
    }

    /// Create an engine for resuming a suspended team run.
    pub async fn new_for_resume(
        team: TeamDefinition,
        run: TeamRun,
        global_home: &Path,
        settings: &Settings,
        team_db: AsyncDatabase,
        github_app: Option<Arc<mika_common::github_app::GitHubApp>>,
        pr_reviews_posted: Option<Arc<dashmap::DashMap<String, std::collections::HashSet<String>>>>,
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
            tool_registry: Arc::new(res.tool_registry),
            callback: None, // No callback on resume — events go through task system
            brave_api_key: settings
                .brave_api_key
                .as_ref()
                .map(|s| s.expose_secret().to_string()),
            github_token: settings.agent_github_token().map(String::from),
            gateway_url: settings.routing_url.clone(),
            internal_token: settings
                .internal_token
                .as_ref()
                .map(|s| s.expose_secret().to_string()),
            github_app,
            team_db,
            goal_msg_id: None,
            trace_id,
            reference_run_id: None,
            pr_reviews_posted,
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
                match self.execute_tasks().await? {
                    ExecuteOutcome::Suspended => {
                        // Team run suspended again, will resume via invoke_orchestrator
                        self.finalize_and_shutdown().await;
                        return Ok(self.run);
                    }
                    ExecuteOutcome::AllTransportFailed => {
                        // mika#1671: status set to FailedTransport inside execute_tasks;
                        // skip review/deliver and finalize the terminal state.
                        self.finalize_and_shutdown().await;
                        return Ok(self.run);
                    }
                    ExecuteOutcome::Proceed => {}
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
                &self.run.started_at,
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
            let ts = crate::timestamp::now();
            self.run.ended_at = Some(ts.clone());
            Some(ts)
        };

        // mika#1676 Unit B: a Completed run that delegated to zero members is a
        // solo-absorption — the orchestrator did the work itself. Flag it so the
        // failure class is queryable without a manual DB audit. (Trivial
        // conversational goals also land here; that's intentional — the column is
        // a backstop, the gate in Unit A is the prevention.)
        self.run.solo_absorption =
            self.run.status == RunStatus::Completed && self.run.delegation_count == 0;

        // Update run status in DB
        let no_delegation_reason = "orchestrator returned a conversational reply for an actionable goal and \
             delegated to zero members (after one reinforced retry)";
        let (status_str, failure_reason) = match &self.run.status {
            RunStatus::Running => ("running", None),
            RunStatus::Completed => ("completed", None),
            RunStatus::Suspended => ("suspended", None),
            RunStatus::Failed(reason) => ("failed", Some(reason.as_str())),
            RunStatus::FailedNoDelegation => ("failed_no_delegation", Some(no_delegation_reason)),
            RunStatus::FailedTransport(reason) => ("failed_transport", Some(reason.as_str())),
        };
        if let Err(e) = self
            .team_db
            .update_team_run(
                &self.run.run_id,
                status_str,
                failure_reason,
                self.run.iteration,
                self.run.deliverable.as_deref(),
                ended_at.as_deref(),
                self.run.delegation_count,
                self.run.solo_absorption,
                self.run.failure_context.as_deref(),
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

        // Load enriched summary — prefer explicitly referenced run, fall back to last completed
        let previous_run_summary = match &self.reference_run_id {
            Some(ref_id) => self.team_db.get_team_run_summary(ref_id).await,
            None => {
                self.team_db
                    .get_last_completed_team_run_summary(&self.run.team_name)
                    .await
            }
        }
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

        let first_result = self
            .decompose(None, &history, previous_run_summary.as_ref(), None)
            .await?;
        match self
            .apply_delegation_gate(
                first_result,
                None,
                &history,
                previous_run_summary.as_ref(),
                DecomposePhase::First,
            )
            .await?
        {
            GateOutcome::Tasks(tasks) => {
                self.run.tasks = tasks;
                // Write assignments metadata
                let assignments = self.format_assignments();
                self.write_metadata_file("assignments.md", &assignments);
            }
            GateOutcome::Conversational(reply) => {
                self.run.deliverable = Some(reply.clone());
                self.emit_event(TeamEvent::Deliverable(reply));
                return Ok(());
            }
            GateOutcome::NoDelegation => {
                // Status + failure_context already set on self.run by the gate.
                // Return Ok so execute()'s Ok arm runs, but it won't override the
                // terminal status (guarded by `status == Running`).
                self.emit_event(TeamEvent::RunFailed(
                    "team run did not delegate the goal to any member".to_string(),
                ));
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
            match self.execute_tasks().await? {
                ExecuteOutcome::Suspended => {
                    return Ok(()); // Team run suspended, will resume via invoke_orchestrator
                }
                ExecuteOutcome::AllTransportFailed => {
                    // mika#1671: status already set to FailedTransport inside
                    // execute_tasks. Skip review/deliver; `execute()` sees the
                    // non-Running status and finalizes without overriding it.
                    return Ok(());
                }
                ExecuteOutcome::Proceed => {}
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
            let redecompose_result = self
                .decompose(
                    Some(&feedback),
                    &history,
                    previous_run_summary.as_ref(),
                    None,
                )
                .await?;
            match self
                .apply_delegation_gate(
                    redecompose_result,
                    Some(&feedback),
                    &history,
                    previous_run_summary.as_ref(),
                    DecomposePhase::RevisionAfterCritic,
                )
                .await?
            {
                GateOutcome::Tasks(tasks) => {
                    self.run.tasks = tasks;
                    // Update assignments metadata on re-decompose
                    let assignments = self.format_assignments();
                    self.write_metadata_file("assignments.md", &assignments);
                }
                GateOutcome::Conversational(reply) => {
                    self.run.deliverable = Some(reply.clone());
                    self.emit_event(TeamEvent::Deliverable(reply));
                    return Ok(());
                }
                GateOutcome::NoDelegation => {
                    self.emit_event(TeamEvent::RunFailed(
                        "team run did not delegate the goal to any member".to_string(),
                    ));
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
        &mut self,
        feedback: Option<&str>,
        history: &[crate::db::TeamRunRow],
        previous_run: Option<&crate::db::TeamRunSummary>,
        reinforcement: Option<&str>,
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

        let mut message = if feedback.is_some() {
            format!(
                "The previous attempt was rejected by the critic. Revise the task assignments.\n\nOriginal goal: {}",
                self.run.goal
            )
        } else {
            self.run.goal.clone()
        };

        // mika#1676 Unit A: delegation-gate retry reinforcement. Appended only on
        // the gated re-prompt after a Conversational fallthrough for an actionable
        // goal. Fresh context (architect U3) — the failed first attempt is NOT
        // included as history, to avoid biasing the model toward repeating the
        // prose-around-JSON pattern.
        if let Some(reinforce) = reinforcement {
            message.push_str("\n\n");
            message.push_str(reinforce);
        }

        let response = self
            .run_agent(orchestrator_name, &message, &context)
            .await?
            .into_text();

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

        // Coverage check: detect silently omitted members and retry once (#286)
        // Track which response text to persist — the original or the retry's.
        let mut persist_response = response;
        let tasks = {
            let initial_missing = missing_members(&tasks, &self.team);
            if initial_missing.is_empty() {
                tasks
            } else {
                let missing_names = initial_missing
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let nudge = format!(
                    "You did not assign tasks to: {missing_names}. Either include them or \
                     add a brief note in a `reply` explaining why they're not relevant to \
                     this goal. It's fine to skip members whose mandate doesn't fit, but \
                     the response must reflect that you considered them."
                );
                let retry_response = self
                    .run_agent(orchestrator_name, &nudge, &context)
                    .await?
                    .into_text();
                let retry_result = parse_task_assignments(&retry_response, &self.team)?;
                match retry_result {
                    DecomposeResult::Conversational(reply) => {
                        // Orchestrator pivoted to a conversational reply on retry —
                        // respect the decision but log coverage gap.
                        warn!(
                            team_run_id = %self.run.run_id,
                            team_id = %self.team.team.name,
                            missing_members = ?initial_missing,
                            retry_recovered = false,
                            initial_missing_count = initial_missing.len(),
                            final_missing_count = initial_missing.len(),
                            "team_coverage_gap"
                        );
                        self.run.coverage_retry_fired = true;
                        // Persist conversational reply and return
                        if let Some(goal_id) = self.goal_msg_id
                            && let Err(e) = self
                                .team_db
                                .insert_team_workspace_entry(
                                    &self.run.run_id,
                                    Some(goal_id),
                                    Some(orchestrator_name),
                                    "orchestrator",
                                    &retry_response,
                                    iteration,
                                    Some(&self.trace_id),
                                )
                                .await
                        {
                            warn!(error = %e, "failed to persist team workspace entry");
                        }
                        return Ok(DecomposeResult::Conversational(reply));
                    }
                    DecomposeResult::Tasks(retry_tasks) => {
                        let final_missing = missing_members(&retry_tasks, &self.team);
                        let retry_recovered = final_missing.is_empty();
                        warn!(
                            team_run_id = %self.run.run_id,
                            team_id = %self.team.team.name,
                            missing_members = ?final_missing,
                            retry_recovered = retry_recovered,
                            initial_missing_count = initial_missing.len(),
                            final_missing_count = final_missing.len(),
                            "team_coverage_gap"
                        );
                        self.run.coverage_retry_fired = true;
                        // Persist the retry response (not the original) so DB matches tasks
                        persist_response = retry_response;
                        retry_tasks
                    }
                }
            }
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
                    &persist_response,
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

    /// Delegation gate (mika#1676 Unit A).
    ///
    /// Wraps a decompose outcome. When the orchestrator returned `Tasks`, passes
    /// through. When it returned `Conversational`, decides whether that reply is
    /// an honest "no delegation needed" (trivial goal) or a *silent absorption*
    /// of an actionable goal:
    ///
    /// - Not actionable → clean `Conversational` (preserves the trivial-goal path).
    /// - Actionable → issue ONE reinforced retry with fresh context. If the retry
    ///   produces tasks, use them. If it is still actionable-but-conversational,
    ///   transition the run to the terminal `RunStatus::FailedNoDelegation` (with
    ///   `failure_context` carrying the phase) rather than letting it complete as
    ///   a zero-delegation success.
    async fn apply_delegation_gate(
        &mut self,
        first_result: DecomposeResult,
        feedback: Option<&str>,
        history: &[crate::db::TeamRunRow],
        previous_run: Option<&crate::db::TeamRunSummary>,
        phase: DecomposePhase,
    ) -> Result<GateOutcome> {
        let reply = match first_result {
            DecomposeResult::Tasks(tasks) => return Ok(GateOutcome::Tasks(tasks)),
            DecomposeResult::Conversational(reply) => reply,
        };

        // Not actionable → honest conversational reply, complete as orchestrator-solo.
        let Some(signal) = detect_actionability(&reply) else {
            return Ok(GateOutcome::Conversational(reply));
        };

        // Operator visibility into why the gate is about to fire (bounded volume).
        debug!(
            team_run_id = %self.run.run_id,
            team_id = %self.team.team.name,
            phase = %phase.as_str(),
            signal = %signal,
            "team_engine_actionability_signal"
        );

        // One reinforced retry with fresh context (architect U3).
        self.run.conversational_retry_fired = true;
        let retry = self
            .decompose(
                feedback,
                history,
                previous_run,
                Some(CONVERSATIONAL_REINFORCEMENT),
            )
            .await?;

        match retry {
            DecomposeResult::Tasks(tasks) => Ok(GateOutcome::Tasks(tasks)),
            DecomposeResult::Conversational(retry_reply) => {
                // If the retry is a non-actionable reply (e.g. an honest `[]` or a
                // plain message), respect it as orchestrator-solo. Only a second
                // actionable-but-no-tasks outcome is a confirmed failure.
                if detect_actionability(&retry_reply).is_none() {
                    return Ok(GateOutcome::Conversational(retry_reply));
                }

                self.run.status = RunStatus::FailedNoDelegation;
                self.run.failure_context = Some(format!("{{\"phase\": \"{}\"}}", phase.as_str()));
                self.run.deliverable = Some(retry_reply);
                warn!(
                    team_run_id = %self.run.run_id,
                    team_id = %self.team.team.name,
                    phase = %phase.as_str(),
                    "team_engine_no_delegation_failure"
                );
                Ok(GateOutcome::NoDelegation)
            }
        }
    }

    /// Execute all tasks by delegating to specialist agents concurrently.
    ///
    /// Uses `tokio::task::JoinSet` to run all specialist agents in parallel.
    /// Each agent has its own AsyncDatabase and the workspace is shared via
    /// the filesystem, so there are no shared mutable state concerns.
    async fn execute_tasks(&mut self) -> Result<ExecuteOutcome> {
        // Prepare shared resources that will be moved into spawned tasks.
        let agents = Arc::clone(&self.agents);
        let tool_registry = Arc::clone(&self.tool_registry);
        let run_id = self.run.run_id.clone();
        let callback = self.callback.clone();
        let team_name = self.run.team_name.clone();
        let brave_api_key = self.brave_api_key.clone();
        let github_token = self.github_token.clone();
        let gateway_url = self.gateway_url.clone();
        let internal_token = self.internal_token.clone();
        let github_app = self.github_app.clone();
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

        // mika#1676 Unit B: count member sessions being delegated this iteration.
        // Accumulated across re-decompose iterations; persisted by
        // `finalize_and_shutdown`. A terminal `Completed` run with
        // `delegation_count == 0` is flagged `solo_absorption`.
        self.run.delegation_count = self
            .run
            .delegation_count
            .saturating_add(inputs.len() as u32);

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
            timeout_at: Some(crate::timestamp::now_plus(chrono::Duration::seconds(
                TEAM_RUN_TIMEOUT_SECS as i64,
            ))),
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
            metadata: None,
            r#type: None,
            dispatch_class: None,
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
                timeout_at: Some(crate::timestamp::now_plus(chrono::Duration::seconds(
                    TEAM_RUN_TIMEOUT_SECS as i64,
                ))),
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
                metadata: None,
                r#type: None,
                dispatch_class: None,
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
            let tool_registry = Arc::clone(&tool_registry);
            let run_id = run_id.clone();
            let callback = callback.clone();
            let team_name = team_name.clone();
            let brave_api_key = brave_api_key.clone();
            let github_token = github_token.clone();
            let gateway_url = gateway_url.clone();
            let internal_token = internal_token.clone();
            let github_app = github_app.clone();
            let team_db = team_db.clone();
            let trace_id = self.trace_id.clone();
            let pr_reviews_posted = self.pr_reviews_posted.clone();

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

                    let result: Result<(String, DelegationOutcome)> = match resources {
                        Ok(resources) => {
                            let session_id = format!("team-{}-{}", run_id, agent_name);
                            // Create per-agent session before running (idempotent for resumed runs)
                            let agent_id = resources.db.agent_id.clone();
                            let metadata = serde_json::json!({
                                "trigger": "team",
                                "team_run_id": run_id,
                            })
                            .to_string();
                            if let Err(e) = resources
                                .db
                                .create_session_if_not_exists(
                                    &session_id,
                                    &agent_id,
                                    "team",
                                    Some(&metadata),
                                )
                                .await
                            {
                                warn!(agent = %agent_name, error = %e, "failed to create per-agent team session");
                            }
                            let skills_dirty = AtomicBool::new(false);
                            let child_task_id_ref = input.child_task_id.as_deref();
                            let params = TeamAgentParams {
                                db: &resources.db,
                                llm: resources.llm.as_ref(),
                                tools: &tool_registry,
                                skills: &resources.skills,
                                home_dir: &resources.home_dir,
                                task_message: &input.task_desc,
                                team_context: &input.context,
                                session_id: &session_id,
                                embedding_client: resources.embedding_client.as_ref(),
                                brave_api_key: brave_api_key.as_deref(),
                                github_token: github_token.as_deref(),
                                gateway_url: gateway_url.as_deref(),
                                internal_token: internal_token.as_deref(),
                                github_app: github_app.as_deref(),
                                skills_dirty: &skills_dirty,
                                settings: Some(&resources.settings),
                                mcp_manager: None,
                                agent_name,
                                child_task_id: child_task_id_ref,
                                // Team agents communicate through the orchestrator pipeline
                                // (workspace entries, deliverables, critic feedback), not
                                // directly to users via Telegram.
                                message_sender: None,
                                // Propagate the team engine's trace_id so delegate agent
                                // events correlate with the parent team run.
                                trace_id: Some(trace_id.clone()),
                            };
                            crate::agent::run_team_agent(&params).await.map(|o| {
                                // mika#1671: preserve the delegation classification
                                // alongside the text so the collection loop can
                                // evaluate the all-transport-failed short-circuit.
                                let outcome = o.delegation_outcome();
                                (o.into_text(), outcome)
                            })
                        }
                        Err(e) => Err(e),
                    };

                    // Persist and report completion/failure for this agent.
                    match &result {
                        Ok((response, _outcome)) => {
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

                    // End the per-agent session now that the agent task is done
                    let end_session_id = format!("team-{}-{}", run_id, &input.agent_name);
                    if let Err(e) = team_db.end_session(&end_session_id).await {
                        warn!(agent = %input.agent_name, error = %e, "failed to end per-agent team session");
                    }
                    if let Some(ref map) = pr_reviews_posted {
                        map.remove(&end_session_id);
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
        // mika#1671: per-task delegation classification, indexed by task index.
        // Left `None` for tasks that never join cleanly (JoinError) — an unclassified
        // slot cannot satisfy the all-transport-failed short-circuit.
        let mut delegation_outcomes: Vec<Option<DelegationOutcome>> = vec![None; total_count];
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
                                Ok((_, outcome)) => {
                                    self.run.tasks[index].status = TaskStatus::Completed;
                                    delegation_outcomes[index] = Some(outcome);
                                }
                                Err(e) => {
                                    // A hard agent-loop error — classify its string so a
                                    // transport-shaped failure still counts toward the
                                    // short-circuit while other failures do not (D2/AC4).
                                    delegation_outcomes[index] =
                                        Some(classify_delegation_from_error(&e.to_string()));
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

        // mika#1671 (AC3/AC5): if every completed delegation in this iteration failed
        // at the transport layer, short-circuit to a terminal FailedTransport state
        // rather than blocking to the full team-run timeout. Evaluated BEFORE the
        // grandchild suspend check (D3 ordering caveat): a fully transport-failed
        // iteration has no useful grandchildren to wait on, so fail fast wins over
        // suspend. A mixed iteration (some transport, some business-completed) does
        // NOT qualify (AC4) — `all_delegations_transport_failed` requires every slot
        // to be a classified TransportError.
        if all_delegations_transport_failed(&delegation_outcomes) {
            let reason = "all-delegations-transport-failed".to_string();
            warn!(
                run_id = %self.run.run_id,
                task_count = delegation_outcomes.len(),
                iteration = self.run.iteration,
                "all delegations transport-failed — short-circuiting team run (mika#1671)"
            );
            self.run.status = RunStatus::FailedTransport(reason.clone());
            self.emit_event(TeamEvent::RunFailed(format!(
                "All {} delegation(s) failed at the transport layer — short-circuited without review/deliver.",
                delegation_outcomes.len()
            )));

            // Persist an error entry to the workspace for observability, mirroring the
            // generic-failure path in `execute()`.
            if let Some(goal_id) = self.goal_msg_id
                && let Err(e) = self
                    .team_db
                    .insert_team_workspace_entry(
                        &self.run.run_id,
                        Some(goal_id),
                        None,
                        "error",
                        &reason,
                        self.run.iteration,
                        Some(&self.trace_id),
                    )
                    .await
            {
                warn!(error = %e, "failed to persist transport-failure workspace entry");
            }

            // Cancel the parent invoke_orchestrator task so the async resume path does
            // not later fire via sibling-completion detection (mirrors the not-suspended
            // path below).
            if let Err(e) = self
                .team_db
                .update_task_status(&parent_task_id, "cancelled")
                .await
            {
                warn!(
                    parent_task_id = %parent_task_id,
                    error = %e,
                    "failed to cancel parent task on transport-fail short-circuit"
                );
            }

            return Ok(ExecuteOutcome::AllTransportFailed);
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
            return Ok(ExecuteOutcome::Suspended);
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

        Ok(ExecuteOutcome::Proceed)
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
            .await?
            .into_text();

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
    ///
    /// On timeout, falls back to workspace content (#1128) rather than
    /// surfacing a misleading "Agent timed out" message.
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
        let outcome = self
            .run_agent(&agent_name, "Produce the final deliverable.", &context)
            .await?;

        let response = match outcome {
            TeamAgentOutcome::Done { text, .. } => text.unwrap_or_default(),
            TeamAgentOutcome::TimedOut(reason) => {
                warn!(
                    target: "mika::otel",
                    agent = %agent_name,
                    workspace = %self.workspace_dir.display(),
                    reason = %reason,
                    "deliver agent timed out — falling back to workspace content"
                );
                // Workspace-content fallback (#1128): specialist outputs are already
                // on disk — use them rather than surfacing a misleading timeout message.
                self.read_workspace_fallback().unwrap_or(reason)
            }
        };

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

    /// Read workspace files and format them as a fallback deliverable (#1128).
    ///
    /// Returns `None` if the workspace contains no non-metadata output files.
    fn read_workspace_fallback(&self) -> Option<String> {
        build_workspace_fallback(&self.workspace_dir)
    }

    /// Run an agent with team context and workspace tools.
    /// Returns a typed [`TeamAgentOutcome`] distinguishing success from timeout (#1128).
    async fn run_agent(
        &self,
        agent_name: &str,
        task_message: &str,
        team_context: &str,
    ) -> Result<TeamAgentOutcome> {
        let resources = self
            .agents
            .get(agent_name)
            .with_context(|| format!("agent '{}' not found in team resources", agent_name))?;

        // Use the pre-built tool registry (#255)
        let session_id = format!("team-{}-{}", self.run.run_id, agent_name);
        let skills_dirty = AtomicBool::new(false);
        let params = TeamAgentParams {
            db: &resources.db,
            llm: resources.llm.as_ref(),
            tools: &self.tool_registry,
            skills: &resources.skills,
            home_dir: &resources.home_dir,
            task_message,
            team_context,
            session_id: &session_id,
            embedding_client: resources.embedding_client.as_ref(),
            brave_api_key: self.brave_api_key.as_deref(),
            github_token: self.github_token.as_deref(),
            gateway_url: self.gateway_url.as_deref(),
            internal_token: self.internal_token.as_deref(),
            github_app: self.github_app.as_deref(),
            skills_dirty: &skills_dirty,
            settings: Some(&resources.settings),
            mcp_manager: None,
            agent_name,
            child_task_id: None,
            // Team agents communicate through the orchestrator pipeline
            // (workspace entries, deliverables, critic feedback), not
            // directly to users via Telegram.
            message_sender: None,
            // Propagate the team engine's trace_id for correlation.
            trace_id: Some(self.trace_id.clone()),
        };

        crate::agent::run_team_agent(&params).await
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

/// Reap orphaned team runs left in `status='running'` when no terminal-state
/// writer ran (mika#1652).
///
/// The failure-path sibling of the team-run lifecycle finalizer
/// (`AsyncDatabase::update_team_run`), analogous to `reap_orphaned_parent_tasks`
/// (mika#871) for the task table. A `team_runs` row stays `running` indefinitely
/// when the normal finalizer never executes — the `mika-spirit` process died
/// mid-run, the `run_team` future was dropped at the tool timeout, or the run
/// errored before `finalize_and_shutdown`. The held row keeps the team slot
/// occupied, blocking subsequent `run_team` calls (founding incident
/// `d68fdcaf`, 2026-06-29).
///
/// A run is orphaned when it has been `running` longer than
/// `TEAM_RUN_STUCK_THRESHOLD_SECS` AND its child sessions show no `llm_calls`
/// or `tool_calls` activity within `TEAM_RUN_LIVENESS_THRESHOLD_SECS` (the
/// mika#959 `NOT EXISTS` liveness pattern). Matched runs transition to
/// `'failed'` (not `'cancelled'` — the reaper is a system-level failure
/// detector, not an operator proxy) with an audit-event trail. The
/// `transition_team_run_terminal` guard (`WHERE status='running'`) makes the
/// transition idempotent and race-safe against the normal finalizer.
///
/// Runs every `DB_SCAN_INTERVAL_TICKS` from the task engine tick loop.
///
/// SOLE WRITER: this function is the only site that writes the
/// `reaper.team_runs` audit `tool_name`.
pub async fn reap_orphaned_team_runs(db: &AsyncDatabase) {
    let stuck = match db
        .find_stuck_team_runs(
            TEAM_RUN_STUCK_THRESHOLD_SECS,
            TEAM_RUN_LIVENESS_THRESHOLD_SECS,
        )
        .await
    {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "reaper.team_runs: failed to query stuck team runs");
            return;
        }
    };

    for run in stuck {
        let trace_id = mika_common::trace::generate_trace_id();
        let system_session = format!("system-{}", db.agent_id);
        let reason = format!(
            "Reaper: no liveness from child sessions for {TEAM_RUN_LIVENESS_THRESHOLD_SECS}s"
        );

        match db
            .transition_team_run_terminal(&run.id, "failed", &reason)
            .await
        {
            Ok(true) => {
                if let Err(e) = db
                    .log_audit_event(
                        &system_session,
                        "reaper.team_runs",
                        &run.id,
                        Some("running"),
                        Some("failed"),
                        Some(&reason),
                        Some(&trace_id),
                    )
                    .await
                {
                    warn!(
                        team_run_id = %run.id,
                        error = %e,
                        "reaper.team_runs: failed to write audit event"
                    );
                }
                info!(
                    team_run_id = %run.id,
                    team = %run.team_name,
                    started_at = %run.started_at,
                    "reaper.team_runs: transitioned orphaned team run to failed"
                );
            }
            Ok(false) => {
                // Row already left `running` (normal finalizer or operator
                // cancel won the race, or a duplicate query row) — skip.
                debug!(
                    team_run_id = %run.id,
                    "reaper.team_runs: team run already terminal, skipping"
                );
            }
            Err(e) => {
                warn!(
                    team_run_id = %run.id,
                    error = %e,
                    "reaper.team_runs: db error during transition"
                );
            }
        }
    }
}

/// Compute non-orchestrator team members who were not assigned any task.
///
/// Returns the names of members in `team.agents` (excluding the orchestrator)
/// that do not appear as an `agent` in any of the provided `tasks`.
fn missing_members(tasks: &[TaskAssignment], team: &TeamDefinition) -> Vec<String> {
    let assigned: std::collections::HashSet<&str> =
        tasks.iter().map(|t| t.agent.as_str()).collect();
    team.agents
        .iter()
        .filter(|a| a.name != team.team.orchestrator)
        .filter(|a| !assigned.contains(a.name.as_str()))
        .map(|a| a.name.clone())
        .collect()
}

/// Result of parsing the orchestrator's response.
enum DecomposeResult {
    /// Orchestrator produced valid task assignments.
    Tasks(Vec<TaskAssignment>),
    /// Orchestrator replied conversationally instead of producing tasks.
    Conversational(String),
}

/// Outcome of the delegation gate (mika#1676 Unit A).
enum GateOutcome {
    /// Orchestrator produced (or recovered, on retry) valid task assignments.
    Tasks(Vec<TaskAssignment>),
    /// Honest conversational reply — complete the run as orchestrator-solo.
    Conversational(String),
    /// Actionable goal absorbed with zero delegation, confirmed after one
    /// reinforced retry. The terminal status + `failure_context` are already set
    /// on `self.run`; the caller returns without completing.
    NoDelegation,
}

/// Which decompose phase the delegation gate is evaluating (mika#1676 Unit A,
/// architect F2 — phase carried in `failure_context`, single terminal state).
#[derive(Clone, Copy)]
enum DecomposePhase {
    First,
    RevisionAfterCritic,
}

impl DecomposePhase {
    fn as_str(self) -> &'static str {
        match self {
            DecomposePhase::First => "first_decompose",
            DecomposePhase::RevisionAfterCritic => "revision_after_critic",
        }
    }
}

/// Reinforcement appended to the gated decompose retry (mika#1676 Unit A).
const CONVERSATIONAL_REINFORCEMENT: &str = "Your previous response did not contain a parseable JSON array of task \
     assignments. Emit a JSON array of task assignments as your sole output, on \
     its own line, with no prose before or after. If this goal genuinely does \
     not require delegating to any team member, respond with exactly `[]`.";

/// Detect whether a conversational orchestrator reply nonetheless *intended* to
/// delegate an actionable goal (mika#1676 Unit A, architect F1 pinned).
///
/// Returns `Some(signal_description)` when EITHER:
/// - (a) the text matches the anchored intent-to-delegate keyword regex, OR
/// - (b) the text contains a JSON-array-of-objects shape (`[ { … } ]`) — which,
///   since we only reach here on the `Conversational` branch, means the array
///   failed `Vec<TaskAssignment>` validation (intent-to-delegate-via-JSON).
///
/// Plain prose with neither signal returns `None`, preserving the trivial-goal
/// conversational path. False negatives are covered by Unit B's observability
/// column; false positives only cost one extra retry (the gate is fail-open).
fn detect_actionability(text: &str) -> Option<String> {
    use std::sync::LazyLock;
    static KEYWORD_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(?i)\b(dispatching?|decompos\w*|delegat\w*|assign(?:ed|ing)?\s+(?:to|for)|hand(?:ing|ed)?\s+(?:off|over)\s+to|split\w*\s+(?:work|task)|parallel\w*\s+(?:delegat|specialis|member))\b",
        )
        .expect("actionability keyword regex is valid")
    });
    // (?s) so `.` spans newlines — a JSON array often straddles lines.
    static JSON_ARRAY_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?s)\[\s*\{.*\}\s*\]").expect("json-array-shape regex is valid")
    });

    if let Some(m) = KEYWORD_RE.find(text) {
        let span = m.as_str();
        let truncated: String = span.chars().take(200).collect();
        return Some(format!("keyword:{truncated}"));
    }
    if let Some(m) = JSON_ARRAY_RE.find(text) {
        return Some(format!("json_array_shape@{}", m.start()));
    }
    None
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

    // Extract JSON array from the response. mika#1676 Unit C: a prose/fence-
    // tolerant preprocessor feeds the existing schema-validation path. Strip
    // markdown fences, then locate the first balanced `[ { … } ]` even when
    // surrounded by narration (sibling of #876's `extract_first_json_object`).
    // Falls back to the legacy `extract_json` extractor (#257). On either hit the
    // extracted text flows into the SAME `Vec<serde_json::Value>` validation
    // below — tolerance is added, schema strictness is unchanged.
    let stripped = strip_markdown_fences(response);
    let json_str = match extract_first_json_array(&stripped)
        .or_else(|| extract_json(&stripped, '[', ']').map(|s| s.to_string()))
    {
        Some(s) => s,
        None => {
            warn!("orchestrator response contains no JSON array, treating as conversational");
            return Ok(DecomposeResult::Conversational(response.to_string()));
        }
    };

    let parsed: Vec<serde_json::Value> = match serde_json::from_str(&json_str) {
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
///
/// Strip markdown code-fence delimiter lines (```lang / ```), keeping inner
/// content (mika#1676 Unit C). A common non-Anthropic model behavior is to wrap
/// the decompose JSON in a fenced block; removing the delimiters lets the
/// extractor and `serde_json` see the bare array.
fn strip_markdown_fences(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("```"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Locate the first balanced JSON array-of-objects (`[ { … } ]`) within prose
/// (mika#1676 Unit C). Sibling of #876's `extract_first_json_object` for braces.
///
/// Unlike [`extract_json`] (which grabs the last `]` in the whole string via
/// `rfind` and so over-captures trailing prose), this does string-literal- and
/// escape-aware bracket-depth matching, so narration after the array does not
/// break the parse. Returns `None` when no `[` is followed by a `{`.
fn extract_first_json_array(text: &str) -> Option<String> {
    let mut search_from = 0;
    while let Some(rel) = text[search_from..].find('[') {
        let open = search_from + rel;
        // Only treat as an array-of-objects start when the next non-ws char is `{`.
        if text[open + 1..].trim_start().starts_with('{')
            && let Some(close) = match_balanced_array(text, open)
        {
            return Some(text[open..=close].to_string());
        }
        search_from = open + 1;
    }
    None
}

/// Find the byte index of the `]` that closes the `[` at `open_idx`, tracking
/// square-bracket depth while skipping over string literals (escape-aware).
fn match_balanced_array(text: &str, open_idx: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for (i, c) in text.char_indices().filter(|(i, _)| *i >= open_idx) {
        if in_string {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

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

/// Read workspace files and format them as a fallback deliverable (#1128).
///
/// Returns `None` if the workspace directory is unreadable or contains no
/// non-empty regular files. Directories (e.g. `.meta/`) are skipped.
/// Files are sorted by modification time so specialist outputs appear in execution order.
fn build_workspace_fallback(workspace_dir: &Path) -> Option<String> {
    let mut parts: Vec<(std::time::SystemTime, String)> = Vec::new();
    let entries = std::fs::read_dir(workspace_dir).ok()?;

    for entry in entries.flatten() {
        let path = entry.path();
        // Skip directories (e.g. .meta/) and non-files
        if path.is_dir() {
            continue;
        }
        // Use continue (not ?) so a single unreadable entry doesn't abort the whole scan.
        let Some(filename) = path.file_name() else {
            continue;
        };
        let filename = filename.to_string_lossy().to_string();
        let Some(mtime) = entry.metadata().ok().and_then(|m| m.modified().ok()) else {
            continue;
        };
        if let Ok(content) = std::fs::read_to_string(&path)
            && !content.trim().is_empty()
        {
            parts.push((mtime, format!("## {filename}\n\n{content}")));
        }
    }

    if parts.is_empty() {
        return None;
    }

    // Sort by modification time so specialist outputs appear in execution order.
    parts.sort_by_key(|(mtime, _)| *mtime);

    let header = "# Team Deliverable (workspace fallback)\n\n\
        > The synthesis agent timed out, but specialist outputs were completed successfully.\n\
        > Below are the workspace outputs in execution order.\n\n";
    let body: Vec<&str> = parts.iter().map(|(_, text)| text.as_str()).collect();
    Some(format!("{}{}", header, body.join("\n\n---\n\n")))
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

    // =======================================================================
    // mika#1671 (AC3/AC4) — all-transport-failed short-circuit predicate
    //
    // `all_delegations_transport_failed` is the gate that decides whether the
    // iteration short-circuits to `RunStatus::FailedTransport`. AC3: fires when
    // every completed delegation is a classified TransportError. AC4 invariant:
    // a mixed iteration (any non-transport or unclassified slot) does NOT fire.
    // =======================================================================

    fn transport() -> Option<DelegationOutcome> {
        Some(DelegationOutcome::TransportError {
            reason: "503".to_string(),
            retryable: true,
        })
    }

    #[test]
    fn all_transport_failed_fires_when_every_slot_is_transport() {
        // AC3/AC5 replay shape: every member emitted 503 → all-transport → true.
        let outcomes = vec![transport(), transport(), transport()];
        assert!(all_delegations_transport_failed(&outcomes));
    }

    #[test]
    fn all_transport_failed_false_on_mixed_iteration() {
        // AC4 invariant: one transport-failed + one business-completed → false.
        let outcomes = vec![transport(), Some(DelegationOutcome::Completed)];
        assert!(!all_delegations_transport_failed(&outcomes));
    }

    #[test]
    fn all_transport_failed_false_with_business_logic_failure() {
        // A hard non-transport failure alongside a transport one is still mixed.
        let outcomes = vec![transport(), Some(DelegationOutcome::BusinessLogicFailure)];
        assert!(!all_delegations_transport_failed(&outcomes));
    }

    #[test]
    fn all_transport_failed_false_on_unclassified_slot() {
        // A `None` slot (task panicked into a JoinError, never classified) cannot
        // satisfy the short-circuit — fail-safe toward the normal review/deliver flow.
        let outcomes = vec![transport(), None];
        assert!(!all_delegations_transport_failed(&outcomes));
    }

    #[test]
    fn all_transport_failed_false_on_empty_iteration() {
        // An empty iteration is not "all transport failed" — nothing to short-circuit.
        assert!(!all_delegations_transport_failed(&[]));
    }

    #[test]
    fn test_team_registry_suppresses_a2a_call() {
        // mika#1653 F1: hard structural assertion that the team orchestrator's
        // tool array never contains `a2a_call`. Anchored to the real registry
        // builder used by `init_resources()`, so a future `default_tools()`
        // change cannot silently re-introduce the cross-container reach tool.
        let tmp = tempfile::tempdir().unwrap();
        let registry = build_team_tool_registry(tmp.path(), None);
        let names: Vec<&str> = registry
            .definitions()
            .iter()
            .map(|d| d.name.as_str())
            .collect();

        assert!(
            !names.contains(&"a2a_call"),
            "a2a_call must be suppressed from the team-mode tool array (mika#1653); found: {names:?}"
        );

        // Workspace tools must still be present — the decompose→spawn→resume
        // cycle relies on members exchanging results via the workspace.
        for ws_tool in ["read_workspace", "write_workspace", "list_workspace"] {
            assert!(
                names.contains(&ws_tool),
                "team registry must retain workspace tool '{ws_tool}'; found: {names:?}"
            );
        }

        // A representative core tool must survive suppression — only `a2a_call`
        // is dropped, not the rest of `default_tools()`.
        assert!(
            names.contains(&"send_message"),
            "team registry must retain core tools like send_message; found: {names:?}"
        );
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

    fn multi_agent_team() -> TeamDefinition {
        TeamDefinition {
            team: TeamMeta {
                name: "test-team".to_string(),
                orchestrator: "orchestrator".to_string(),
            },
            agents: vec![
                TeamAgent {
                    name: "orchestrator".to_string(),
                    role: "orchestrator".to_string(),
                    mandate: "Plan".to_string(),
                },
                TeamAgent {
                    name: "alice".to_string(),
                    role: "specialist".to_string(),
                    mandate: "Research".to_string(),
                },
                TeamAgent {
                    name: "bob".to_string(),
                    role: "specialist".to_string(),
                    mandate: "Design".to_string(),
                },
                TeamAgent {
                    name: "carol".to_string(),
                    role: "specialist".to_string(),
                    mandate: "Review".to_string(),
                },
            ],
            flow: TeamFlow { max_iterations: 3 },
        }
    }

    #[test]
    fn test_missing_members_full_coverage() {
        let team = multi_agent_team();
        let tasks = vec![
            TaskAssignment {
                agent: "alice".to_string(),
                role: "specialist".to_string(),
                task: "do stuff".to_string(),
                output_file: "a.md".to_string(),
                status: TaskStatus::Pending,
            },
            TaskAssignment {
                agent: "bob".to_string(),
                role: "specialist".to_string(),
                task: "do stuff".to_string(),
                output_file: "b.md".to_string(),
                status: TaskStatus::Pending,
            },
            TaskAssignment {
                agent: "carol".to_string(),
                role: "specialist".to_string(),
                task: "do stuff".to_string(),
                output_file: "c.md".to_string(),
                status: TaskStatus::Pending,
            },
        ];
        assert!(missing_members(&tasks, &team).is_empty());
    }

    #[test]
    fn test_missing_members_partial_coverage() {
        let team = multi_agent_team();
        let tasks = vec![TaskAssignment {
            agent: "alice".to_string(),
            role: "specialist".to_string(),
            task: "do stuff".to_string(),
            output_file: "a.md".to_string(),
            status: TaskStatus::Pending,
        }];
        let missing = missing_members(&tasks, &team);
        assert_eq!(missing.len(), 2);
        assert!(missing.contains(&"bob".to_string()));
        assert!(missing.contains(&"carol".to_string()));
    }

    #[test]
    fn test_missing_members_excludes_orchestrator() {
        let team = multi_agent_team();
        // No tasks assigned — only non-orchestrator members should be missing
        let missing = missing_members(&[], &team);
        assert_eq!(missing.len(), 3);
        assert!(!missing.contains(&"orchestrator".to_string()));
    }

    #[test]
    fn test_missing_members_single_member_team() {
        let team = TeamDefinition {
            team: TeamMeta {
                name: "solo".to_string(),
                orchestrator: "orchestrator".to_string(),
            },
            agents: vec![
                TeamAgent {
                    name: "orchestrator".to_string(),
                    role: "orchestrator".to_string(),
                    mandate: "Plan".to_string(),
                },
                TeamAgent {
                    name: "only-worker".to_string(),
                    role: "specialist".to_string(),
                    mandate: "Work".to_string(),
                },
            ],
            flow: TeamFlow { max_iterations: 3 },
        };
        let tasks = vec![TaskAssignment {
            agent: "only-worker".to_string(),
            role: "specialist".to_string(),
            task: "do stuff".to_string(),
            output_file: "out.md".to_string(),
            status: TaskStatus::Pending,
        }];
        assert!(missing_members(&tasks, &team).is_empty());
    }

    // -- read_workspace_fallback tests (#1128) --

    #[test]
    fn test_workspace_fallback_populated() {
        let dir = tempfile::tempdir().unwrap();
        // Write two files with different mtimes
        std::fs::write(dir.path().join("cto-review.md"), "CTO analysis here").unwrap();
        // Brief sleep to ensure different mtimes
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(dir.path().join("quant-review.md"), "Quantitative analysis").unwrap();

        let result = build_workspace_fallback(dir.path());
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("# Team Deliverable (workspace fallback)"));
        assert!(text.contains("## cto-review.md"));
        assert!(text.contains("CTO analysis here"));
        assert!(text.contains("## quant-review.md"));
        assert!(text.contains("Quantitative analysis"));
        assert!(text.contains("---")); // separator between files
    }

    #[test]
    fn test_workspace_fallback_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(build_workspace_fallback(dir.path()).is_none());
    }

    #[test]
    fn test_workspace_fallback_only_directories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".meta")).unwrap();
        std::fs::write(
            dir.path().join(".meta/state.json"),
            r#"{"phase":"deliver"}"#,
        )
        .unwrap();
        assert!(build_workspace_fallback(dir.path()).is_none());
    }

    #[test]
    fn test_workspace_fallback_mixed_content() {
        let dir = tempfile::tempdir().unwrap();
        // Create a directory that should be skipped
        std::fs::create_dir(dir.path().join(".meta")).unwrap();
        std::fs::write(
            dir.path().join(".meta/state.json"),
            r#"{"phase":"deliver"}"#,
        )
        .unwrap();
        // Create a real output file
        std::fs::write(dir.path().join("analysis.md"), "Real output").unwrap();
        // Create an empty file that should be skipped
        std::fs::write(dir.path().join("empty.md"), "   ").unwrap();

        let result = build_workspace_fallback(dir.path());
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("## analysis.md"));
        assert!(text.contains("Real output"));
        // Empty file and .meta directory should not appear
        assert!(!text.contains("empty.md"));
        assert!(!text.contains(".meta"));
        assert!(!text.contains("state.json"));
        // No separator since there's only one file
        assert!(!text.contains("---"));
    }

    #[test]
    fn test_workspace_fallback_nonexistent_dir() {
        let path = std::path::PathBuf::from("/nonexistent/workspace/dir");
        assert!(build_workspace_fallback(&path).is_none());
    }

    // ---- team-run orphan reaper (mika#1652) ----

    use crate::async_db::AsyncDatabase;
    use crate::db::Database;

    fn reaper_test_db() -> AsyncDatabase {
        let db = Database::open_in_memory().unwrap();
        AsyncDatabase::new_with_agent(db, "mika")
    }

    /// Insert a team run and force its `started_at` to `age_secs` ago.
    async fn seed_team_run(db: &AsyncDatabase, run_id: &str, started_age_secs: i64) {
        db.insert_team_run(
            run_id,
            "test-team",
            "goal",
            3,
            &crate::timestamp::now(),
            None,
        )
        .await
        .unwrap();
        let id = run_id.to_string();
        let modifier = format!("-{started_age_secs} seconds");
        db.with_db(move |inner| {
            inner.conn.execute(
                "UPDATE team_runs SET started_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2) WHERE id = ?1",
                rusqlite::params![id, modifier],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    }

    /// Insert an `llm_calls` row on a child session aged `age_secs` ago.
    async fn seed_llm_call(db: &AsyncDatabase, session_id: &str, age_secs: i64) {
        let sid = session_id.to_string();
        let modifier = format!("-{age_secs} seconds");
        db.with_db(move |inner| {
            inner.conn.execute(
                "INSERT INTO llm_calls (id, agent_id, session_id, provider, model, created_at)
                 VALUES (?1, 'mika', ?2, 'mock', 'mock', strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?3))",
                rusqlite::params![uuid::Uuid::new_v4().to_string(), sid, modifier],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    }

    /// Insert a `tool_calls` row on a child session aged `age_secs` ago.
    async fn seed_tool_call(db: &AsyncDatabase, session_id: &str, age_secs: i64) {
        let sid = session_id.to_string();
        let modifier = format!("-{age_secs} seconds");
        db.with_db(move |inner| {
            inner.conn.execute(
                "INSERT INTO tool_calls (id, agent_id, session_id, tool_name, created_at)
                 VALUES (?1, 'mika', ?2, 'a2a_call', strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?3))",
                rusqlite::params![uuid::Uuid::new_v4().to_string(), sid, modifier],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    }

    /// AC1 + AC2: a stuck run (old, no liveness) is transitioned to `failed`
    /// with `ended_at`/`failure_reason` set and a `reaper.team_runs` audit event.
    #[tokio::test]
    async fn test_reaper_transitions_stuck_team_run_to_failed() {
        let db = reaper_test_db();
        let run_id = "stuck-run-1";
        // Older than the 20-min stuck threshold, no child-session activity.
        seed_team_run(&db, run_id, TEAM_RUN_STUCK_THRESHOLD_SECS + 60).await;

        reap_orphaned_team_runs(&db).await;

        let row = db.load_team_run_by_id(run_id).await.unwrap().unwrap();
        assert_eq!(row.status, "failed", "stuck run must be reaped to failed");
        assert!(row.ended_at.is_some(), "ended_at must be set");
        let reason = row.failure_reason.expect("failure_reason must be set");
        assert!(
            reason.contains(&TEAM_RUN_LIVENESS_THRESHOLD_SECS.to_string()),
            "failure_reason must name the liveness window, got: {reason}"
        );

        // AC2: audit event under the reaper system session.
        let events = db
            .get_audit_events(&format!("system-{}", db.agent_id))
            .await
            .unwrap();
        let reaper_event = events
            .iter()
            .find(|e| e.tool_name == "reaper.team_runs")
            .expect("a reaper.team_runs audit event must exist");
        assert_eq!(reaper_event.target_key, run_id, "audit names the run id");
        assert_eq!(reaper_event.after_value.as_deref(), Some("failed"));
    }

    /// AC1 (negative): a run younger than the stuck threshold is left alone.
    #[tokio::test]
    async fn test_reaper_ignores_young_running_team_run() {
        let db = reaper_test_db();
        let run_id = "young-run";
        seed_team_run(&db, run_id, TEAM_RUN_STUCK_THRESHOLD_SECS - 120).await;

        reap_orphaned_team_runs(&db).await;

        let row = db.load_team_run_by_id(run_id).await.unwrap().unwrap();
        assert_eq!(row.status, "running", "young run must not be reaped");
    }

    /// AC1 (liveness guard): an old run WITH recent child-session activity is
    /// alive and must not be reaped — covers both `llm_calls` and `tool_calls`.
    #[tokio::test]
    async fn test_reaper_skips_old_run_with_recent_liveness() {
        // Recent llm_call keeps it alive.
        let db = reaper_test_db();
        let run_id = "live-run-llm";
        seed_team_run(&db, run_id, TEAM_RUN_STUCK_THRESHOLD_SECS + 60).await;
        seed_llm_call(&db, &format!("team-{run_id}-worker"), 30).await;
        reap_orphaned_team_runs(&db).await;
        let row = db.load_team_run_by_id(run_id).await.unwrap().unwrap();
        assert_eq!(row.status, "running", "recent llm_call proves liveness");

        // Recent tool_call keeps it alive (the a2a_call shape).
        let db2 = reaper_test_db();
        let run_id2 = "live-run-tool";
        seed_team_run(&db2, run_id2, TEAM_RUN_STUCK_THRESHOLD_SECS + 60).await;
        seed_tool_call(&db2, &format!("team-{run_id2}"), 30).await;
        reap_orphaned_team_runs(&db2).await;
        let row2 = db2.load_team_run_by_id(run_id2).await.unwrap().unwrap();
        assert_eq!(row2.status, "running", "recent tool_call proves liveness");
    }

    /// AC1 (stale liveness): an old run whose only child activity predates the
    /// liveness window IS reaped — proves the window bound is enforced.
    #[tokio::test]
    async fn test_reaper_reaps_old_run_with_only_stale_liveness() {
        let db = reaper_test_db();
        let run_id = "stale-live-run";
        seed_team_run(&db, run_id, TEAM_RUN_STUCK_THRESHOLD_SECS + 600).await;
        // Activity older than the liveness window — does not count as alive.
        seed_llm_call(
            &db,
            &format!("team-{run_id}-worker"),
            TEAM_RUN_LIVENESS_THRESHOLD_SECS + 120,
        )
        .await;

        reap_orphaned_team_runs(&db).await;

        let row = db.load_team_run_by_id(run_id).await.unwrap().unwrap();
        assert_eq!(
            row.status, "failed",
            "stale liveness must not protect the run"
        );
    }

    /// AC5: a non-running run (completed/failed/suspended) is never touched.
    #[tokio::test]
    async fn test_reaper_ignores_non_running_team_runs() {
        let db = reaper_test_db();
        for status in ["completed", "failed", "suspended"] {
            let run_id = format!("terminal-{status}");
            seed_team_run(&db, &run_id, TEAM_RUN_STUCK_THRESHOLD_SECS + 600).await;
            let id = run_id.clone();
            let st = status.to_string();
            db.with_db(move |inner| {
                inner.conn.execute(
                    "UPDATE team_runs SET status = ?2 WHERE id = ?1",
                    rusqlite::params![id, st],
                )?;
                Ok(())
            })
            .await
            .unwrap();

            reap_orphaned_team_runs(&db).await;

            let row = db.load_team_run_by_id(&run_id).await.unwrap().unwrap();
            assert_eq!(row.status, status, "{status} run must be left untouched");
        }
    }

    /// AC3: idempotency — a second sweep does not double-transition, and the
    /// guarded UPDATE returns `false` on an already-terminal row.
    #[tokio::test]
    async fn test_reaper_is_idempotent() {
        let db = reaper_test_db();
        let run_id = "idempotent-run";
        seed_team_run(&db, run_id, TEAM_RUN_STUCK_THRESHOLD_SECS + 60).await;

        reap_orphaned_team_runs(&db).await;
        let ended_after_first = db
            .load_team_run_by_id(run_id)
            .await
            .unwrap()
            .unwrap()
            .ended_at;

        // Direct guard check: already-terminal row reports no change.
        let changed = db
            .transition_team_run_terminal(run_id, "failed", "second attempt")
            .await
            .unwrap();
        assert!(
            !changed,
            "guarded UPDATE must not re-transition a terminal row"
        );

        // Second full sweep is a no-op (run no longer matches the query).
        reap_orphaned_team_runs(&db).await;
        let row = db.load_team_run_by_id(run_id).await.unwrap().unwrap();
        assert_eq!(row.status, "failed");
        assert_eq!(
            row.ended_at, ended_after_first,
            "ended_at must not change on a repeat sweep"
        );

        // Exactly one reaper audit event was written.
        let events = db
            .get_audit_events(&format!("system-{}", db.agent_id))
            .await
            .unwrap();
        let count = events
            .iter()
            .filter(|e| e.tool_name == "reaper.team_runs")
            .count();
        assert_eq!(
            count, 1,
            "idempotent sweep must write exactly one audit event"
        );
    }

    // ------------------------------------------------------------------
    // mika#1676 Unit A — actionability detection tests
    //
    // `detect_actionability` is the load-bearing predicate that decides
    // whether a `Conversational` decompose reply merits a gated retry vs.
    // routing through as an honest orchestrator-solo completion. The
    // regex + JSON-array-shape pair is architect-F1-pinned; regressions
    // here would either (a) mute the gate on genuine intent-to-delegate
    // replies (false negative — the mika#1676 defect returns) or
    // (b) fire the gate on trivial conversational goals (false positive —
    // "what time is it" now costs one extra decompose call). Both matter.
    // ------------------------------------------------------------------

    #[test]
    fn test_detect_actionability_matches_delegate_verbs() {
        // Direct evidence from run #1 (founding incident): the orchestrator
        // said "Now dispatching to all three specialists in parallel" then
        // never spawned anyone. The keyword regex MUST catch this shape.
        for txt in [
            "Now dispatching to all three specialists in parallel",
            "I am delegating this to the CTO agent",
            "Assigning to the quant specialist",
            "Handing off to the CTO",
            "I'll decompose this into three parallel tasks",
            "Splitting work between the CFO and the quant",
            "Parallel delegation to the three specialists",
        ] {
            assert!(
                detect_actionability(txt).is_some(),
                "actionability regex must fire on intent-to-delegate phrase: {txt:?}",
            );
        }
    }

    #[test]
    fn test_detect_actionability_no_fire_on_trivial_prose() {
        // False-positive guard: trivial conversational replies with no
        // intent-to-delegate signal must route through cleanly. Otherwise
        // "what time is it" costs one extra decompose call.
        for txt in [
            "It is 3:15 PM local time.",
            "The team is ready.",
            "Everything looks good.",
            "Hello!",
            "I do not have enough information to answer that.",
        ] {
            assert!(
                detect_actionability(txt).is_none(),
                "actionability regex must NOT fire on trivial reply: {txt:?}",
            );
        }
    }

    #[test]
    fn test_detect_actionability_matches_json_array_shape() {
        // Even without any keyword, a JSON-array-of-objects shape that
        // failed `Vec<TaskAssignment>` validation upstream is itself
        // intent-to-delegate-via-JSON evidence (architect F1 branch b).
        let txt = "Sure, here's the plan:\n[{\"a\": 1}, {\"b\": 2}]\nMake sense?";
        let signal = detect_actionability(txt).expect("json-array-shape branch must fire");
        assert!(
            signal.starts_with("json_array_shape@"),
            "signal must identify the branch that fired; got {signal:?}",
        );
    }

    #[test]
    fn test_detect_actionability_json_array_spans_newlines() {
        // Real orchestrator responses pretty-print JSON across newlines.
        // The `(?s)` flag on JSON_ARRAY_RE MUST let `.` span line breaks;
        // without it a multi-line array would silently skip the gate.
        let txt = "Plan:\n[\n  {\n    \"agent\": \"cto\"\n  }\n]";
        assert!(
            detect_actionability(txt).is_some(),
            "json-array-shape regex must span newlines (needs (?s) flag)",
        );
    }

    // ------------------------------------------------------------------
    // mika#1676 Unit C — parse tolerance tests
    //
    // Unit C is a preprocessor that feeds the existing schema validation
    // path (architect F3): its job is to LOWER the rate at which the
    // Unit A gate has to fire on genuine JSON-in-prose responses. These
    // tests pin the two helpers (`strip_markdown_fences` +
    // `extract_first_json_array`) so a future refactor cannot silently
    // dilute their tolerance.
    // ------------------------------------------------------------------

    #[test]
    fn test_strip_markdown_fences_removes_fenced_json() {
        let response = "```json\n[{\"agent\":\"cto\"}]\n```";
        let stripped = strip_markdown_fences(response);
        assert!(!stripped.contains("```"), "fence backticks must be removed");
        assert!(stripped.contains("[{\"agent\":\"cto\"}]"));
    }

    #[test]
    fn test_strip_markdown_fences_is_noop_on_plain_text() {
        let response = "[{\"agent\":\"cto\",\"task\":\"go\",\"output_file\":\"o.md\"}]";
        assert_eq!(
            strip_markdown_fences(response),
            response,
            "plain text without fences must round-trip unchanged"
        );
    }

    #[test]
    fn test_extract_first_json_array_locates_array_in_prose() {
        // The founding-incident shape: JSON-in-prose narration that the
        // legacy `extract_json` regex would miss. The balanced-bracket
        // extractor MUST recover it so `parse_task_assignments` reaches
        // the schema validation path with clean input.
        let text = "Here's the plan:\n[{\"agent\":\"cto\",\"task\":\"do stuff\"}]\nEnd of plan.";
        let extracted = extract_first_json_array(text)
            .expect("extractor must locate the array embedded in prose");
        assert!(extracted.starts_with('['));
        assert!(extracted.ends_with(']'));
        assert!(extracted.contains("cto"));
    }

    #[test]
    fn test_extract_first_json_array_returns_none_on_prose_only() {
        // A response with no array at all must return None so the caller
        // falls through to the legacy extractor (which will also miss)
        // and the parser routes to `Conversational` — the correct outcome
        // for an honest prose reply.
        let text = "I do not have enough information to plan this.";
        assert!(
            extract_first_json_array(text).is_none(),
            "prose-only text must return None so the caller can route to Conversational",
        );
    }

    #[test]
    fn test_parse_task_assignments_recovers_fenced_json() {
        // End-to-end pin for Unit C: a markdown-fenced task-assignment
        // array MUST be recovered as `DecomposeResult::Tasks` (not
        // `Conversational`). Regression here would silently push the
        // fenced-JSON class into the Unit A gate — costing one extra
        // decompose call per fenced response, which glm-5.2 emits often.
        let team = test_team();
        let response = "```json\n[{\"agent\":\"worker\",\"task\":\"Do research\",\"output_file\":\"r.md\"}]\n```";
        let result = parse_task_assignments(response, &team).unwrap();
        match result {
            DecomposeResult::Tasks(tasks) => {
                assert_eq!(tasks.len(), 1);
                assert_eq!(tasks[0].agent, "worker");
            }
            DecomposeResult::Conversational(_) => {
                panic!("Unit C fence stripping must recover fenced JSON as Tasks");
            }
        }
    }
}

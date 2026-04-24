pub mod engine;
pub(crate) mod notification;
pub mod prompt;
pub mod types;

use anyhow::Result;
use std::path::Path;
use std::sync::Arc;

use mika_common::config::Settings;
use mika_common::github_app::GitHubApp;
use mika_common::team;

use crate::async_db::AsyncDatabase;

use self::engine::TeamEngine;
use self::types::{TeamEventCallback, TeamRun};

/// Run a team workflow end-to-end.
///
/// Loads the team definition, validates all agents exist, creates the
/// orchestration engine, and executes the full decompose -> execute -> review -> deliver flow.
///
/// If `reference_run_id` is provided, the referenced run's workspace is made available
/// as read-only context to workspace tools, and the referenced run's summary overrides
/// the auto-detected "last completed run" in the orchestrator prompt.
#[allow(clippy::too_many_arguments)]
pub async fn run_team(
    team_name: &str,
    goal: &str,
    global_home: &Path,
    settings: &Settings,
    callback: Option<TeamEventCallback>,
    team_db: AsyncDatabase,
    reference_run_id: Option<&str>,
    github_app: Option<Arc<GitHubApp>>,
) -> Result<TeamRun> {
    let def = team::load_team(global_home, team_name)?;
    team::validate_team(global_home, &def)?;

    let engine = TeamEngine::new(
        def,
        goal,
        global_home,
        settings,
        callback,
        team_db,
        reference_run_id,
        github_app,
    )?;
    engine.execute().await
}

/// Resume a suspended team run from a checkpoint.
///
/// Called by the `invoke_orchestrator` dispatcher when all child tasks
/// (agent delegations) have completed. Deserializes the team state from
/// the checkpoint, injects child results as agent responses, and continues
/// from the specified phase (typically Review → Deliver).
#[allow(clippy::too_many_arguments)]
pub async fn resume_team_run(
    _team_run_id: &str,
    team_name: &str,
    next_phase: &str,
    team_state: &str,
    child_results: &str,
    global_home: &Path,
    db: &AsyncDatabase,
    github_app: Option<Arc<GitHubApp>>,
) -> Result<()> {
    tracing::info!(
        team_name = team_name,
        next_phase = next_phase,
        "resuming suspended team run"
    );

    // Deserialize the team run state from the checkpoint.
    // Handles both versioned envelopes and legacy unversioned formats.
    let run = types::deserialize_checkpoint(team_state)?;

    // Load team definition and settings
    let def = team::load_team(global_home, team_name)?;
    team::validate_team(global_home, &def)?;

    let settings = Settings::load(global_home)?;

    // Create a new team_db connection for the resume
    let db_path = mika_common::home::container_db_path(global_home);
    let resume_db = crate::db::Database::open(&db_path)?;
    let team_db = AsyncDatabase::new_with_agent(resume_db, &db.agent_id);

    let engine =
        TeamEngine::new_for_resume(def, run, global_home, &settings, team_db, github_app).await?;
    let _run = engine.execute_from_phase(next_phase, child_results).await?;

    Ok(())
}

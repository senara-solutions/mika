pub mod engine;
pub mod history;
pub mod prompt;
pub mod types;

use anyhow::Result;
use std::path::Path;

use mika_common::config::Settings;
use mika_common::team;

use crate::async_db::AsyncDatabase;

use self::engine::TeamEngine;
use self::types::{TeamEventCallback, TeamRun};

/// Run a team workflow end-to-end.
///
/// Loads the team definition, validates all agents exist, creates the
/// orchestration engine, and executes the full decompose -> execute -> review -> deliver flow.
pub async fn run_team(
    team_name: &str,
    goal: &str,
    global_home: &Path,
    settings: &Settings,
    callback: Option<TeamEventCallback>,
    team_db: AsyncDatabase,
) -> Result<TeamRun> {
    let def = team::load_team(global_home, team_name)?;
    team::validate_team(global_home, &def)?;

    let engine = TeamEngine::new(def, goal, global_home, settings, callback, team_db)?;
    engine.execute().await
}

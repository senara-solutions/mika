pub mod engine;
pub mod prompt;
pub mod types;

use anyhow::Result;
use std::path::Path;

use mika_common::config::Settings;
use mika_common::team;

use crate::async_db::AsyncDatabase;
use crate::db::Database;

use self::engine::TeamEngine;
use self::types::{TeamEventCallback, TeamRun};

/// Error type for [`open_team_db`].
pub enum TeamDbError {
    /// The team data directory does not exist (no runs recorded yet).
    /// Contains a user-facing "No runs found" message — not a hard error.
    NoRuns(String),
    /// The database could not be opened (IO/corruption).
    OpenFailed(String),
}

impl std::fmt::Display for TeamDbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoRuns(msg) | Self::OpenFailed(msg) => f.write_str(msg),
        }
    }
}

/// Open a team's SQLite database for read-only access.
///
/// Returns [`TeamDbError::NoRuns`] if the team data directory does not
/// exist and [`TeamDbError::OpenFailed`] if the database cannot be opened.
pub fn open_team_db(home_dir: &Path, team_name: &str) -> Result<AsyncDatabase, TeamDbError> {
    let team_data_dir = team::team_dir(home_dir, team_name).join("data");
    if !team_data_dir.exists() {
        return Err(TeamDbError::NoRuns(format!(
            "No runs found for team '{team_name}'."
        )));
    }
    let team_db_path = team_data_dir.join("mika.db");
    match Database::open(&team_db_path) {
        Ok(db) => Ok(AsyncDatabase::new(db)),
        Err(e) => Err(TeamDbError::OpenFailed(format!(
            "Failed to open team database: {e}"
        ))),
    }
}

/// Open (or create) a team's SQLite database for read-write access.
///
/// Creates the team data directory if it does not already exist.
/// Returns `Err(String)` if the directory cannot be created or the
/// database cannot be opened.
pub fn open_or_create_team_db(
    home_dir: &Path,
    team_name: &str,
) -> Result<AsyncDatabase, String> {
    let team_data_dir = team::team_dir(home_dir, team_name).join("data");
    std::fs::create_dir_all(&team_data_dir)
        .map_err(|e| format!("Failed to create team data directory: {e}"))?;
    let team_db_path = team_data_dir.join("mika.db");
    match Database::open(&team_db_path) {
        Ok(db) => Ok(AsyncDatabase::new(db)),
        Err(e) => Err(format!("Failed to open team database: {e}")),
    }
}

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

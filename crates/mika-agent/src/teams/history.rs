use anyhow::{Context, Result};
use std::path::Path;

use super::types::TeamRun;

/// Save a team run to the history directory as a TOML file.
///
/// Filename format: `run-{date}-{short_id}.toml`
pub fn save_run(history_dir: &Path, run: &TeamRun) -> Result<()> {
    std::fs::create_dir_all(history_dir)
        .with_context(|| format!("failed to create history dir {}", history_dir.display()))?;

    let date = run.started_at.get(..10).unwrap_or("unknown");
    let short_id = &run.run_id[..run.run_id.len().min(8)];
    let filename = format!("run-{date}-{short_id}.toml");

    let content = toml::to_string_pretty(run).context("failed to serialize team run")?;
    let path = history_dir.join(&filename);
    std::fs::write(&path, content)
        .with_context(|| format!("failed to write {}", path.display()))?;

    Ok(())
}

/// Load the most recent run from the history directory.
pub fn load_latest_run(history_dir: &Path) -> Result<Option<TeamRun>> {
    let mut files = list_run_files(history_dir)?;
    if files.is_empty() {
        return Ok(None);
    }

    // Sort descending by filename (date-based names sort chronologically)
    files.sort();
    files.reverse();

    let path = history_dir.join(&files[0]);
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let run: TeamRun =
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(Some(run))
}

/// List all runs in the history directory.
pub fn list_runs(history_dir: &Path) -> Result<Vec<TeamRun>> {
    list_runs_limited(history_dir, usize::MAX)
}

/// List runs with an upper bound on how many files to read.
pub fn list_runs_limited(history_dir: &Path, limit: usize) -> Result<Vec<TeamRun>> {
    let mut files = list_run_files(history_dir)?;
    files.sort();
    files.reverse(); // most recent first

    let mut runs = Vec::with_capacity(limit.min(files.len()));
    for filename in files {
        if runs.len() >= limit {
            break;
        }
        let path = history_dir.join(&filename);
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let run: TeamRun = match toml::from_str(&content) {
            Ok(r) => r,
            Err(_) => continue,
        };
        runs.push(run);
    }

    Ok(runs)
}

fn list_run_files(history_dir: &Path) -> Result<Vec<String>> {
    if !history_dir.exists() {
        return Ok(Vec::new());
    }
    let entries = std::fs::read_dir(history_dir)
        .with_context(|| format!("failed to read {}", history_dir.display()))?;

    let files: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("run-") && name.ends_with(".toml") {
                Some(name)
            } else {
                None
            }
        })
        .collect();

    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::teams::types::RunStatus;
    use crate::test_utils::test_helpers::test_team_run;

    #[test]
    fn test_save_and_load_run() {
        let tmp = tempfile::tempdir().unwrap();
        let history = tmp.path().join("history");

        let run = test_team_run();
        save_run(&history, &run).unwrap();

        let loaded = load_latest_run(&history).unwrap().unwrap();
        assert_eq!(loaded.run_id, "abcd1234");
        assert_eq!(loaded.team_name, "dev-team");
        assert_eq!(loaded.status, RunStatus::Completed);
    }

    #[test]
    fn test_load_latest_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let history = tmp.path().join("history");

        let loaded = load_latest_run(&history).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_list_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let history = tmp.path().join("history");

        let mut run1 = test_team_run();
        run1.run_id = "aaaa1111".to_string();
        run1.started_at = "2026-02-24T10:00:00Z".to_string();
        save_run(&history, &run1).unwrap();

        let mut run2 = test_team_run();
        run2.run_id = "bbbb2222".to_string();
        run2.started_at = "2026-02-25T10:00:00Z".to_string();
        save_run(&history, &run2).unwrap();

        let runs = list_runs(&history).unwrap();
        assert_eq!(runs.len(), 2);
        // Most recent first
        assert_eq!(runs[0].run_id, "bbbb2222");
        assert_eq!(runs[1].run_id, "aaaa1111");
    }

    #[test]
    fn test_list_runs_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let runs = list_runs(tmp.path()).unwrap();
        assert!(runs.is_empty());
    }
}

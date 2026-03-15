use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::agent;

fn default_max_iterations() -> u32 {
    3
}

/// Top-level team definition, parsed from team.toml.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct TeamDefinition {
    pub team: TeamMeta,
    pub agents: Vec<TeamAgent>,
    #[serde(default)]
    pub flow: TeamFlow,
}

/// Metadata about the team.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct TeamMeta {
    pub name: String,
    pub orchestrator: String,
}

/// An agent participating in the team.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct TeamAgent {
    pub name: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub mandate: String,
}

/// Flow configuration for the team orchestration.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct TeamFlow {
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
}

impl Default for TeamFlow {
    fn default() -> Self {
        Self {
            max_iterations: default_max_iterations(),
        }
    }
}

/// Validate a team name.
///
/// Rules: lowercase alphanumeric + hyphens, 1-32 chars,
/// no leading/trailing/consecutive hyphens. (Same rules as agent names.)
pub fn validate_team_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("Team name cannot be empty");
    }
    if name.len() > 32 {
        bail!("Team name cannot exceed 32 characters");
    }
    if name.starts_with('-') || name.ends_with('-') {
        bail!("Team name cannot start or end with a hyphen");
    }
    if name.contains("--") {
        bail!("Team name cannot contain consecutive hyphens");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        bail!("Team name must contain only lowercase letters, digits, and hyphens");
    }
    Ok(())
}

/// Normalize a team name: trim whitespace and lowercase.
pub fn normalize_team_name(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Returns the directory path for a named team: `{home_dir}/teams/{name}/`
pub fn team_dir(home_dir: &Path, name: &str) -> PathBuf {
    home_dir.join("teams").join(name)
}

/// Check if a named team exists (has a team.toml file).
pub fn team_exists(home_dir: &Path, name: &str) -> bool {
    team_dir(home_dir, name).join("team.toml").exists()
}

/// List all teams in `{home_dir}/teams/`, returning sorted names
/// of directories that contain a team.toml file.
pub fn list_teams(home_dir: &Path) -> Vec<String> {
    let teams_dir = home_dir.join("teams");
    let Ok(entries) = std::fs::read_dir(&teams_dir) else {
        return Vec::new();
    };

    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if e.path().join("team.toml").exists() {
                Some(name)
            } else {
                None
            }
        })
        .collect();

    names.sort();
    names
}

/// Load and parse a team definition from `{home_dir}/teams/{name}/team.toml`.
pub fn load_team(home_dir: &Path, name: &str) -> Result<TeamDefinition> {
    let path = team_dir(home_dir, name).join("team.toml");
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let def: TeamDefinition =
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(def)
}

/// Validate that all agents referenced in a team definition exist.
pub fn validate_team(home_dir: &Path, def: &TeamDefinition) -> Result<()> {
    // Orchestrator must exist as an agent
    if !agent::agent_exists(home_dir, &def.team.orchestrator) {
        bail!(
            "Orchestrator agent '{}' does not exist",
            def.team.orchestrator
        );
    }

    // All team agents must exist
    for ta in &def.agents {
        if !agent::agent_exists(home_dir, &ta.name) {
            bail!("Team agent '{}' does not exist", ta.name);
        }
    }

    // Orchestrator must be listed in agents
    if !def.agents.iter().any(|a| a.name == def.team.orchestrator) {
        bail!(
            "Orchestrator '{}' must be listed in the agents section",
            def.team.orchestrator
        );
    }

    Ok(())
}

/// Returns the base workspace directory for a team (contains per-run subdirectories):
/// `{team_dir}/workspace/`
pub fn workspace_base_dir(home_dir: &Path, team_name: &str) -> PathBuf {
    team_dir(home_dir, team_name).join("workspace")
}

/// Returns the run-scoped workspace directory: `{team_dir}/workspace/{run_id}/`
pub fn workspace_run_dir(home_dir: &Path, team_name: &str, run_id: &str) -> PathBuf {
    workspace_base_dir(home_dir, team_name).join(run_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_validate_team_name_valid() {
        assert!(validate_team_name("dev-team").is_ok());
        assert!(validate_team_name("a").is_ok());
        assert!(validate_team_name("team-1").is_ok());
        assert!(validate_team_name("my-research-team").is_ok());
    }

    #[test]
    fn test_validate_team_name_empty() {
        let err = validate_team_name("").unwrap_err();
        assert!(err.to_string().contains("cannot be empty"));
    }

    #[test]
    fn test_validate_team_name_too_long() {
        let long = "a".repeat(33);
        let err = validate_team_name(&long).unwrap_err();
        assert!(err.to_string().contains("cannot exceed 32"));
    }

    #[test]
    fn test_validate_team_name_leading_hyphen() {
        let err = validate_team_name("-bad").unwrap_err();
        assert!(err.to_string().contains("start or end with a hyphen"));
    }

    #[test]
    fn test_validate_team_name_trailing_hyphen() {
        let err = validate_team_name("bad-").unwrap_err();
        assert!(err.to_string().contains("start or end with a hyphen"));
    }

    #[test]
    fn test_validate_team_name_consecutive_hyphens() {
        let err = validate_team_name("bad--name").unwrap_err();
        assert!(err.to_string().contains("consecutive hyphens"));
    }

    #[test]
    fn test_validate_team_name_uppercase() {
        let err = validate_team_name("BadName").unwrap_err();
        assert!(err.to_string().contains("lowercase"));
    }

    #[test]
    fn test_normalize_team_name() {
        assert_eq!(normalize_team_name("  Dev-Team  "), "dev-team");
        assert_eq!(normalize_team_name("MY-TEAM"), "my-team");
    }

    #[test]
    fn test_team_dir() {
        let home = Path::new("/home/user/.mika");
        assert_eq!(
            team_dir(home, "dev-team"),
            PathBuf::from("/home/user/.mika/teams/dev-team")
        );
    }

    #[test]
    fn test_workspace_base_dir() {
        let home = Path::new("/home/user/.mika");
        assert_eq!(
            workspace_base_dir(home, "dev-team"),
            PathBuf::from("/home/user/.mika/teams/dev-team/workspace")
        );
    }

    #[test]
    fn test_workspace_run_dir() {
        let home = Path::new("/home/user/.mika");
        assert_eq!(
            workspace_run_dir(home, "dev-team", "abc-123"),
            PathBuf::from("/home/user/.mika/teams/dev-team/workspace/abc-123")
        );
    }

    #[test]
    fn test_team_exists_false() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!team_exists(tmp.path(), "nonexistent"));
    }

    #[test]
    fn test_team_exists_true() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = team_dir(tmp.path(), "dev-team");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("team.toml"), "").unwrap();
        assert!(team_exists(tmp.path(), "dev-team"));
    }

    #[test]
    fn test_list_teams_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(list_teams(tmp.path()).is_empty());
    }

    #[test]
    fn test_list_teams_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        for name in &["zeta-team", "alpha-team", "beta-team"] {
            let dir = team_dir(tmp.path(), name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("team.toml"), "").unwrap();
        }
        // Dir without team.toml should be excluded
        fs::create_dir_all(team_dir(tmp.path(), "no-toml")).unwrap();

        let teams = list_teams(tmp.path());
        assert_eq!(teams, vec!["alpha-team", "beta-team", "zeta-team"]);
    }

    #[test]
    fn test_load_team_valid() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = team_dir(tmp.path(), "dev-team");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("team.toml"),
            r#"
[team]
name = "dev-team"
orchestrator = "planner"

[[agents]]
name = "planner"
role = "orchestrator"
mandate = "Decompose goals into tasks"

[[agents]]
name = "researcher"
role = "specialist"
mandate = "Research and gather information"

[flow]
steps = ["decompose", "execute", "review"]
max_iterations = 2
"#,
        )
        .unwrap();

        let def = load_team(tmp.path(), "dev-team").unwrap();
        assert_eq!(def.team.name, "dev-team");
        assert_eq!(def.team.orchestrator, "planner");
        assert_eq!(def.agents.len(), 2);
        assert_eq!(def.agents[0].name, "planner");
        assert_eq!(def.agents[1].role, "specialist");
        assert_eq!(def.flow.max_iterations, 2);
    }

    #[test]
    fn test_load_team_default_max_iterations() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = team_dir(tmp.path(), "team1");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("team.toml"),
            r#"
[team]
name = "team1"
orchestrator = "main"

[[agents]]
name = "main"
role = "orchestrator"
mandate = "Plan"

[flow]
steps = ["decompose", "execute"]
"#,
        )
        .unwrap();

        let def = load_team(tmp.path(), "team1").unwrap();
        assert_eq!(def.flow.max_iterations, 3); // default
    }

    #[test]
    fn test_load_team_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(load_team(tmp.path(), "nonexistent").is_err());
    }

    #[test]
    fn test_validate_team_agents_exist() {
        let tmp = tempfile::tempdir().unwrap();
        // Create agents with DB files
        for name in &["planner", "researcher"] {
            let agent = tmp.path().join("agents").join(name);
            fs::create_dir_all(&agent).unwrap();
            fs::write(agent.join("config.toml"), "# config").unwrap();
        }

        let def = TeamDefinition {
            team: TeamMeta {
                name: "test".to_string(),
                orchestrator: "planner".to_string(),
            },
            agents: vec![
                TeamAgent {
                    name: "planner".to_string(),
                    role: "orchestrator".to_string(),
                    mandate: "Plan".to_string(),
                },
                TeamAgent {
                    name: "researcher".to_string(),
                    role: "specialist".to_string(),
                    mandate: "Research".to_string(),
                },
            ],
            flow: TeamFlow { max_iterations: 3 },
        };

        assert!(validate_team(tmp.path(), &def).is_ok());
    }

    #[test]
    fn test_validate_team_missing_agent() {
        let tmp = tempfile::tempdir().unwrap();
        // Only create planner
        let agent = tmp.path().join("agents").join("planner");
        fs::create_dir_all(&agent).unwrap();
        fs::write(agent.join("config.toml"), "# config").unwrap();

        let def = TeamDefinition {
            team: TeamMeta {
                name: "test".to_string(),
                orchestrator: "planner".to_string(),
            },
            agents: vec![
                TeamAgent {
                    name: "planner".to_string(),
                    role: "orchestrator".to_string(),
                    mandate: "Plan".to_string(),
                },
                TeamAgent {
                    name: "missing".to_string(),
                    role: "specialist".to_string(),
                    mandate: "Research".to_string(),
                },
            ],
            flow: TeamFlow { max_iterations: 3 },
        };

        let err = validate_team(tmp.path(), &def).unwrap_err();
        assert!(err.to_string().contains("'missing' does not exist"));
    }

    #[test]
    fn test_validate_team_orchestrator_not_in_agents() {
        let tmp = tempfile::tempdir().unwrap();
        // Create both agents
        for name in &["planner", "worker"] {
            let agent = tmp.path().join("agents").join(name);
            fs::create_dir_all(&agent).unwrap();
            fs::write(agent.join("config.toml"), "# config").unwrap();
        }

        let def = TeamDefinition {
            team: TeamMeta {
                name: "test".to_string(),
                orchestrator: "planner".to_string(),
            },
            agents: vec![TeamAgent {
                name: "worker".to_string(),
                role: "specialist".to_string(),
                mandate: "Work".to_string(),
            }],
            flow: TeamFlow { max_iterations: 3 },
        };

        let err = validate_team(tmp.path(), &def).unwrap_err();
        assert!(err.to_string().contains("must be listed"));
    }

    #[test]
    fn test_validate_team_missing_orchestrator() {
        let tmp = tempfile::tempdir().unwrap();

        let def = TeamDefinition {
            team: TeamMeta {
                name: "test".to_string(),
                orchestrator: "ghost".to_string(),
            },
            agents: vec![TeamAgent {
                name: "ghost".to_string(),
                role: "orchestrator".to_string(),
                mandate: "Plan".to_string(),
            }],
            flow: TeamFlow { max_iterations: 3 },
        };

        let err = validate_team(tmp.path(), &def).unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn test_minimal_team_toml_parses() {
        let toml = r#"
[team]
name = "minimal"
orchestrator = "lead"

[[agents]]
name = "lead"

[[agents]]
name = "worker"
"#;
        let def: TeamDefinition = toml::from_str(toml).unwrap();
        assert_eq!(def.agents.len(), 2);
        assert_eq!(def.agents[0].role, "");
        assert_eq!(def.agents[0].mandate, "");
        assert_eq!(def.flow.max_iterations, 3);
    }
}

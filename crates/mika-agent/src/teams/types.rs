use serde::{Deserialize, Serialize};

/// Tracks the overall state of a team execution run.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TeamRun {
    pub run_id: String,
    pub team_name: String,
    pub goal: String,
    pub status: RunStatus,
    pub iteration: u32,
    pub max_iterations: u32,
    pub tasks: Vec<TaskAssignment>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub deliverable: Option<String>,
}

/// Status of the overall team run.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum RunStatus {
    Running,
    Completed,
    Failed(String),
}

/// A task delegated to a specialist agent.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TaskAssignment {
    pub agent: String,
    pub role: String,
    pub task: String,
    pub output_file: String,
    pub status: TaskStatus,
}

/// Status of an individual task within a run.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed(String),
}

impl std::fmt::Display for RunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunStatus::Running => write!(f, "running"),
            RunStatus::Completed => write!(f, "completed"),
            RunStatus::Failed(msg) => write!(f, "failed: {msg}"),
        }
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskStatus::Pending => write!(f, "pending"),
            TaskStatus::Running => write!(f, "running"),
            TaskStatus::Completed => write!(f, "completed"),
            TaskStatus::Failed(msg) => write!(f, "failed: {msg}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_team_run_serialization() {
        let run = TeamRun {
            run_id: "abc123".to_string(),
            team_name: "dev-team".to_string(),
            goal: "Research Rust patterns".to_string(),
            status: RunStatus::Completed,
            iteration: 1,
            max_iterations: 3,
            tasks: vec![TaskAssignment {
                agent: "researcher".to_string(),
                role: "specialist".to_string(),
                task: "Research async patterns".to_string(),
                output_file: "research.md".to_string(),
                status: TaskStatus::Completed,
            }],
            started_at: "2026-02-25T10:00:00Z".to_string(),
            ended_at: Some("2026-02-25T10:05:00Z".to_string()),
            deliverable: Some("Final summary".to_string()),
        };

        let toml_str = toml::to_string_pretty(&run).unwrap();
        let deserialized: TeamRun = toml::from_str(&toml_str).unwrap();
        assert_eq!(deserialized.run_id, "abc123");
        assert_eq!(deserialized.status, RunStatus::Completed);
        assert_eq!(deserialized.tasks.len(), 1);
        assert_eq!(deserialized.tasks[0].status, TaskStatus::Completed);
    }

    #[test]
    fn test_run_status_display() {
        assert_eq!(RunStatus::Running.to_string(), "running");
        assert_eq!(RunStatus::Completed.to_string(), "completed");
        assert_eq!(
            RunStatus::Failed("timeout".to_string()).to_string(),
            "failed: timeout"
        );
    }

    #[test]
    fn test_task_status_display() {
        assert_eq!(TaskStatus::Pending.to_string(), "pending");
        assert_eq!(TaskStatus::Running.to_string(), "running");
        assert_eq!(TaskStatus::Completed.to_string(), "completed");
        assert_eq!(
            TaskStatus::Failed("error".to_string()).to_string(),
            "failed: error"
        );
    }
}

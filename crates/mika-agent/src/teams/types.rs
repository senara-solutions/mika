/// Tracks the overall state of a team execution run.
#[derive(Clone, Debug)]
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
#[derive(Clone, Debug, PartialEq)]
pub enum RunStatus {
    Running,
    Completed,
    Failed(String),
}

/// A task delegated to a specialist agent.
#[derive(Clone, Debug)]
pub struct TaskAssignment {
    pub agent: String,
    pub role: String,
    pub task: String,
    pub output_file: String,
    pub status: TaskStatus,
}

/// Status of an individual task within a run.
#[derive(Clone, Debug, PartialEq)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed(String),
}

/// Events emitted by the team engine during execution.
///
/// Used to communicate structured progress to callers (TUI, management tools).
/// The caller decides what to display vs persist.
#[derive(Clone, Debug)]
pub enum TeamEvent {
    /// Transient progress message (e.g., "Decomposing goal...")
    Progress(String),
    /// Orchestrator decomposed goal into tasks.
    TasksAssigned {
        tasks: Vec<TaskAssignment>,
        iteration: u32,
    },
    /// Individual agent completed its task.
    AgentCompleted { agent: String, response: String },
    /// Individual agent failed.
    AgentFailed { agent: String, error: String },
    /// Critic reviewed outputs.
    CriticReview {
        approved: bool,
        feedback: String,
        iteration: u32,
    },
    /// Final deliverable produced.
    Deliverable(String),
    /// Run failed with an error.
    RunFailed(String),
}

/// Callback for receiving team events during execution.
pub type TeamEventCallback = Box<dyn Fn(TeamEvent) + Send + Sync>;

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

    #[test]
    fn test_team_event_debug() {
        let event = TeamEvent::Progress("Decomposing goal...".to_string());
        let debug = format!("{event:?}");
        assert!(debug.contains("Decomposing goal"));

        let event = TeamEvent::AgentCompleted {
            agent: "researcher".to_string(),
            response: "Found 3 papers".to_string(),
        };
        let debug = format!("{event:?}");
        assert!(debug.contains("researcher"));
    }

    #[test]
    fn test_team_event_clone() {
        let event = TeamEvent::CriticReview {
            approved: true,
            feedback: "Looks good".to_string(),
            iteration: 2,
        };
        let cloned = event.clone();
        if let TeamEvent::CriticReview {
            approved,
            feedback,
            iteration,
        } = cloned
        {
            assert!(approved);
            assert_eq!(feedback, "Looks good");
            assert_eq!(iteration, 2);
        } else {
            panic!("unexpected variant");
        }
    }
}

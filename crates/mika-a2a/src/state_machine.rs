use crate::error::A2aError;
use crate::types::TaskState;

/// Validates task state transitions according to the A2A protocol.
pub struct TaskStateMachine;

impl TaskStateMachine {
    /// Check if a state is terminal (no further transitions allowed).
    pub fn is_terminal(state: TaskState) -> bool {
        matches!(
            state,
            TaskState::Completed | TaskState::Failed | TaskState::Canceled | TaskState::Rejected
        )
    }

    /// Check if a transition from one state to another is valid.
    pub fn can_transition(from: TaskState, to: TaskState) -> bool {
        matches!(
            (from, to),
            // From Submitted
            (TaskState::Submitted, TaskState::Working)
                | (TaskState::Submitted, TaskState::Canceled)
                | (TaskState::Submitted, TaskState::Rejected)
                // From Working
                | (TaskState::Working, TaskState::Completed)
                | (TaskState::Working, TaskState::Failed)
                | (TaskState::Working, TaskState::Canceled)
                | (TaskState::Working, TaskState::InputRequired)
                | (TaskState::Working, TaskState::AuthRequired)
                // From InputRequired
                | (TaskState::InputRequired, TaskState::Working)
                | (TaskState::InputRequired, TaskState::Canceled)
                | (TaskState::InputRequired, TaskState::Failed)
                // From AuthRequired
                | (TaskState::AuthRequired, TaskState::Working)
                | (TaskState::AuthRequired, TaskState::Canceled)
                | (TaskState::AuthRequired, TaskState::Failed)
        )
    }

    /// Attempt a state transition. Returns error if invalid.
    pub fn transition(from: TaskState, to: TaskState) -> Result<TaskState, A2aError> {
        if Self::can_transition(from, to) {
            Ok(to)
        } else {
            Err(A2aError::InvalidStateTransition { from, to })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_states() {
        assert!(TaskStateMachine::is_terminal(TaskState::Completed));
        assert!(TaskStateMachine::is_terminal(TaskState::Failed));
        assert!(TaskStateMachine::is_terminal(TaskState::Canceled));
        assert!(TaskStateMachine::is_terminal(TaskState::Rejected));
        assert!(!TaskStateMachine::is_terminal(TaskState::Submitted));
        assert!(!TaskStateMachine::is_terminal(TaskState::Working));
        assert!(!TaskStateMachine::is_terminal(TaskState::InputRequired));
        assert!(!TaskStateMachine::is_terminal(TaskState::AuthRequired));
        assert!(!TaskStateMachine::is_terminal(TaskState::Unknown));
    }

    #[test]
    fn test_valid_transitions() {
        // Submitted transitions
        assert!(TaskStateMachine::can_transition(
            TaskState::Submitted,
            TaskState::Working
        ));
        assert!(TaskStateMachine::can_transition(
            TaskState::Submitted,
            TaskState::Canceled
        ));
        assert!(TaskStateMachine::can_transition(
            TaskState::Submitted,
            TaskState::Rejected
        ));

        // Working transitions
        assert!(TaskStateMachine::can_transition(
            TaskState::Working,
            TaskState::Completed
        ));
        assert!(TaskStateMachine::can_transition(
            TaskState::Working,
            TaskState::Failed
        ));
        assert!(TaskStateMachine::can_transition(
            TaskState::Working,
            TaskState::Canceled
        ));
        assert!(TaskStateMachine::can_transition(
            TaskState::Working,
            TaskState::InputRequired
        ));
        assert!(TaskStateMachine::can_transition(
            TaskState::Working,
            TaskState::AuthRequired
        ));

        // InputRequired transitions
        assert!(TaskStateMachine::can_transition(
            TaskState::InputRequired,
            TaskState::Working
        ));
        assert!(TaskStateMachine::can_transition(
            TaskState::InputRequired,
            TaskState::Canceled
        ));
        assert!(TaskStateMachine::can_transition(
            TaskState::InputRequired,
            TaskState::Failed
        ));

        // AuthRequired transitions
        assert!(TaskStateMachine::can_transition(
            TaskState::AuthRequired,
            TaskState::Working
        ));
        assert!(TaskStateMachine::can_transition(
            TaskState::AuthRequired,
            TaskState::Canceled
        ));
        assert!(TaskStateMachine::can_transition(
            TaskState::AuthRequired,
            TaskState::Failed
        ));
    }

    #[test]
    fn test_invalid_transitions() {
        // Cannot transition from terminal states
        assert!(!TaskStateMachine::can_transition(
            TaskState::Completed,
            TaskState::Working
        ));
        assert!(!TaskStateMachine::can_transition(
            TaskState::Failed,
            TaskState::Working
        ));
        assert!(!TaskStateMachine::can_transition(
            TaskState::Canceled,
            TaskState::Working
        ));
        assert!(!TaskStateMachine::can_transition(
            TaskState::Rejected,
            TaskState::Working
        ));

        // Cannot skip states
        assert!(!TaskStateMachine::can_transition(
            TaskState::Submitted,
            TaskState::Completed
        ));
        assert!(!TaskStateMachine::can_transition(
            TaskState::Submitted,
            TaskState::InputRequired
        ));

        // Cannot go backward
        assert!(!TaskStateMachine::can_transition(
            TaskState::Working,
            TaskState::Submitted
        ));
    }

    #[test]
    fn test_transition_success() {
        let result = TaskStateMachine::transition(TaskState::Submitted, TaskState::Working);
        assert_eq!(result.unwrap(), TaskState::Working);
    }

    #[test]
    fn test_transition_failure() {
        let result = TaskStateMachine::transition(TaskState::Completed, TaskState::Working);
        assert!(result.is_err());
    }
}

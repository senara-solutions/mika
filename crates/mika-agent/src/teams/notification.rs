//! Team-run completion notification formatting.
//!
//! Shared helper for building the single user-facing message at team-run
//! terminal state. Used by both the sync path (`run_team` tool) and the
//! async path (`dispatch_invoke_orchestrator`).

use crate::db::TeamRunRow;
use crate::teams::types::{RunStatus, TeamRun};

/// Maximum character count for the deliverable text in a notification.
/// 4000 is below the Telegram 4096-char text limit with headroom for the prefix.
const MAX_DELIVERABLE_CHARS: usize = 4000;

/// Result of building a completion message: the text and metadata for logging.
pub(crate) struct CompletionMessage {
    pub text: String,
    pub notification_kind: &'static str,
    pub deliverable_chars: usize,
    pub truncated: bool,
}

/// Build a user-facing completion message from an in-memory `TeamRun`.
///
/// Returns `None` for non-terminal statuses (`Running`, `Suspended`).
/// Used by the sync path (run_team tool).
pub(crate) fn build_run_completion_message(run: &TeamRun) -> Option<CompletionMessage> {
    match &run.status {
        RunStatus::Completed => {
            if let Some(ref deliverable) = run.deliverable {
                let (text, truncated) = format_deliverable(&run.team_name, deliverable);
                Some(CompletionMessage {
                    deliverable_chars: deliverable.chars().count(),
                    text,
                    notification_kind: "deliverable",
                    truncated,
                })
            } else {
                Some(CompletionMessage {
                    text: format!(
                        "Team '{}' completed (no deliverable produced).",
                        run.team_name
                    ),
                    notification_kind: "fallback",
                    deliverable_chars: 0,
                    truncated: false,
                })
            }
        }
        RunStatus::Failed(reason) => Some(CompletionMessage {
            text: format!("Team '{}' failed: {}", run.team_name, reason),
            notification_kind: "failure",
            deliverable_chars: 0,
            truncated: false,
        }),
        // mika#1676: orchestrator returned a conversational reply for an
        // actionable goal and delegated to zero members (even after one retry).
        RunStatus::FailedNoDelegation => Some(CompletionMessage {
            text: format!(
                "Team '{}' did not run: the orchestrator did not delegate the goal \
                 to any member. The goal looked actionable but produced no task \
                 assignments. Try rephrasing the goal or check the team's member roster.",
                run.team_name
            ),
            notification_kind: "failure",
            deliverable_chars: 0,
            truncated: false,
        }),
        // Running and Suspended are non-terminal — no notification.
        RunStatus::Running | RunStatus::Suspended => None,
    }
}

/// Build a user-facing completion message from a `TeamRunRow` (DB row).
///
/// Returns `None` for non-terminal statuses.
/// Used by the async path (dispatch_invoke_orchestrator).
pub(crate) fn build_run_completion_message_from_row(run: &TeamRunRow) -> Option<CompletionMessage> {
    match run.status.as_str() {
        "completed" => {
            if let Some(ref deliverable) = run.deliverable {
                let (text, truncated) = format_deliverable(&run.team_name, deliverable);
                Some(CompletionMessage {
                    deliverable_chars: deliverable.chars().count(),
                    text,
                    notification_kind: "deliverable",
                    truncated,
                })
            } else {
                Some(CompletionMessage {
                    text: format!(
                        "Team '{}' completed (no deliverable produced).",
                        run.team_name
                    ),
                    notification_kind: "fallback",
                    deliverable_chars: 0,
                    truncated: false,
                })
            }
        }
        "failed" => {
            let reason = run.failure_reason.as_deref().unwrap_or("unknown error");
            Some(CompletionMessage {
                text: format!("Team '{}' failed: {}", run.team_name, reason),
                notification_kind: "failure",
                deliverable_chars: 0,
                truncated: false,
            })
        }
        "cancelled" => Some(CompletionMessage {
            text: format!("Team '{}' was cancelled.", run.team_name),
            notification_kind: "cancelled",
            deliverable_chars: 0,
            truncated: false,
        }),
        // mika#1676: orchestrator delegated to zero members for an actionable goal.
        "failed_no_delegation" => Some(CompletionMessage {
            text: format!(
                "Team '{}' did not run: the orchestrator did not delegate the goal \
                 to any member. The goal looked actionable but produced no task \
                 assignments. Try rephrasing the goal or check the team's member roster.",
                run.team_name
            ),
            notification_kind: "failure",
            deliverable_chars: 0,
            truncated: false,
        }),
        // "running", "suspended", or unexpected — no notification.
        _ => None,
    }
}

/// Format a deliverable with UTF-8-safe truncation.
fn format_deliverable(team_name: &str, deliverable: &str) -> (String, bool) {
    let char_count = deliverable.chars().count();
    if char_count <= MAX_DELIVERABLE_CHARS {
        (
            format!(
                "Team '{}' completed. Deliverable:\n\n{}",
                team_name, deliverable
            ),
            false,
        )
    } else {
        // UTF-8-safe truncation: find the char boundary at MAX_DELIVERABLE_CHARS chars
        let truncation_byte_index = deliverable
            .char_indices()
            .nth(MAX_DELIVERABLE_CHARS)
            .map(|(i, _)| i)
            .unwrap_or(deliverable.len());
        let truncated_text = &deliverable[..truncation_byte_index];
        (
            format!(
                "Team '{}' completed. Deliverable:\n\n{}\n\n[…truncated after {} chars — full deliverable persisted on team_runs.deliverable]",
                team_name, truncated_text, MAX_DELIVERABLE_CHARS
            ),
            true,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::teams::types::RunStatus;

    fn make_run(status: RunStatus, deliverable: Option<String>) -> TeamRun {
        TeamRun {
            run_id: "test-run-1".to_string(),
            team_name: "alpha".to_string(),
            goal: "test goal".to_string(),
            status,
            iteration: 1,
            max_iterations: 3,
            tasks: vec![],
            started_at: "2026-04-24T12:00:00Z".to_string(),
            ended_at: Some("2026-04-24T12:05:00Z".to_string()),
            deliverable,
            coverage_retry_fired: false,
            conversational_retry_fired: false,
            delegation_count: 0,
            solo_absorption: false,
            failure_context: None,
        }
    }

    fn make_row(
        status: &str,
        failure_reason: Option<String>,
        deliverable: Option<String>,
    ) -> TeamRunRow {
        TeamRunRow {
            id: "test-run-1".to_string(),
            team_name: "alpha".to_string(),
            goal: "test goal".to_string(),
            status: status.to_string(),
            failure_reason,
            iteration: 1,
            max_iterations: 3,
            deliverable,
            started_at: "2026-04-24T12:00:00Z".to_string(),
            ended_at: Some("2026-04-24T12:05:00Z".to_string()),
            trace_id: None,
            delegation_count: 0,
            solo_absorption: false,
            failure_context: None,
        }
    }

    #[test]
    fn test_completed_with_short_deliverable() {
        let run = make_run(RunStatus::Completed, Some("short text".to_string()));
        let msg = build_run_completion_message(&run).unwrap();
        assert_eq!(
            msg.text,
            "Team 'alpha' completed. Deliverable:\n\nshort text"
        );
        assert_eq!(msg.notification_kind, "deliverable");
        assert_eq!(msg.deliverable_chars, 10);
        assert!(!msg.truncated);
    }

    #[test]
    fn test_completed_with_truncation() {
        // Create a 5000-char deliverable with some multi-byte chars
        let deliverable: String = "à".repeat(5000);
        let run = make_run(RunStatus::Completed, Some(deliverable));
        let msg = build_run_completion_message(&run).unwrap();

        assert!(msg.truncated);
        assert_eq!(msg.deliverable_chars, 5000);
        assert!(msg.text.contains("[…truncated after 4000 chars"));
        // Verify truncation is UTF-8 safe — the text should be valid
        assert!(msg.text.is_char_boundary(msg.text.len()));
    }

    #[test]
    fn test_completed_without_deliverable() {
        let run = make_run(RunStatus::Completed, None);
        let msg = build_run_completion_message(&run).unwrap();
        assert_eq!(
            msg.text,
            "Team 'alpha' completed (no deliverable produced)."
        );
        assert_eq!(msg.notification_kind, "fallback");
        assert_eq!(msg.deliverable_chars, 0);
        assert!(!msg.truncated);
    }

    #[test]
    fn test_failed() {
        let run = make_run(
            RunStatus::Failed("orchestrator timed out".to_string()),
            None,
        );
        let msg = build_run_completion_message(&run).unwrap();
        assert_eq!(msg.text, "Team 'alpha' failed: orchestrator timed out");
        assert_eq!(msg.notification_kind, "failure");
    }

    #[test]
    fn test_non_terminal_running() {
        let run = make_run(RunStatus::Running, None);
        assert!(build_run_completion_message(&run).is_none());
    }

    #[test]
    fn test_non_terminal_suspended() {
        let run = make_run(RunStatus::Suspended, None);
        assert!(build_run_completion_message(&run).is_none());
    }

    // TeamRunRow-based tests (async path)

    #[test]
    fn test_row_completed_with_deliverable() {
        let row = make_row("completed", None, Some("result text".to_string()));
        let msg = build_run_completion_message_from_row(&row).unwrap();
        assert_eq!(
            msg.text,
            "Team 'alpha' completed. Deliverable:\n\nresult text"
        );
        assert_eq!(msg.notification_kind, "deliverable");
    }

    #[test]
    fn test_row_failed() {
        let row = make_row("failed", Some("orchestrator timed out".to_string()), None);
        let msg = build_run_completion_message_from_row(&row).unwrap();
        assert_eq!(msg.text, "Team 'alpha' failed: orchestrator timed out");
        assert_eq!(msg.notification_kind, "failure");
    }

    #[test]
    fn test_row_failed_no_reason() {
        let row = make_row("failed", None, None);
        let msg = build_run_completion_message_from_row(&row).unwrap();
        assert_eq!(msg.text, "Team 'alpha' failed: unknown error");
    }

    #[test]
    fn test_row_cancelled() {
        let row = make_row("cancelled", None, None);
        let msg = build_run_completion_message_from_row(&row).unwrap();
        assert_eq!(msg.text, "Team 'alpha' was cancelled.");
        assert_eq!(msg.notification_kind, "cancelled");
    }

    #[test]
    fn test_row_non_terminal() {
        let row = make_row("running", None, None);
        assert!(build_run_completion_message_from_row(&row).is_none());

        let row = make_row("suspended", None, None);
        assert!(build_run_completion_message_from_row(&row).is_none());
    }

    #[test]
    fn test_truncation_exact_boundary() {
        // Exactly MAX_DELIVERABLE_CHARS should not truncate
        let deliverable: String = "a".repeat(MAX_DELIVERABLE_CHARS);
        let run = make_run(RunStatus::Completed, Some(deliverable));
        let msg = build_run_completion_message(&run).unwrap();
        assert!(!msg.truncated);
        assert_eq!(msg.deliverable_chars, MAX_DELIVERABLE_CHARS);
    }

    #[test]
    fn test_truncation_one_over() {
        let deliverable: String = "a".repeat(MAX_DELIVERABLE_CHARS + 1);
        let run = make_run(RunStatus::Completed, Some(deliverable));
        let msg = build_run_completion_message(&run).unwrap();
        assert!(msg.truncated);
        assert_eq!(msg.deliverable_chars, MAX_DELIVERABLE_CHARS + 1);
    }
}

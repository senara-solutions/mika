//! Webhook deferral queue for sequencing GitHub webhooks against in-flight callbacks.
//!
//! When a task has an in-flight `run_claude_pilot` callback, inbound webhooks
//! targeting the same task are deferred until the callback completes. This prevents
//! race conditions where the webhook arrives before callback metadata (especially `pr_url`)
//! is persisted.
//!
//! See issue #528 and the incident doc at
//! `docs/solutions/agent-quality/2026-04-11-mika-dev-verdict-misclassification-pr-522.md`.

use std::time::{Duration, Instant};

use regex::Regex;
use std::sync::LazyLock;
use tracing::{debug, warn};

use crate::async_db::AsyncDatabase;
use crate::task_engine::types::trigger_type;

use super::types::MessageRequest;
use super::verdict::parse_pr_review_event;

/// Maximum time a webhook can be deferred before forced replay.
pub const DEFERRAL_TIMEOUT: Duration = Duration::from_secs(60);

/// A webhook that has been deferred pending callback completion.
#[derive(Debug)]
pub struct DeferredWebhook {
    /// The original message request from the gateway.
    pub request: MessageRequest,
    /// When the webhook was received.
    pub received_at: Instant,
    /// The task ID this webhook correlates to.
    pub task_id: String,
    /// Human-readable description for audit events (e.g. "pull_request_review.submitted on repo#42").
    pub event_desc: String,
    /// Deadline after which the webhook is replayed regardless of callback state.
    pub deadline: Instant,
}

/// Result of attempting to correlate a webhook to a task.
#[derive(Debug, Clone)]
pub struct WebhookCorrelation {
    pub pr_url: Option<String>,
    pub branch: Option<String>,
    pub event_desc: String,
}

/// Regex for parsing check_suite events from gateway-formatted text.
/// Format: `[GitHub] Check suite {conclusion} on {repo} (branch: {branch})`
static CHECK_SUITE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\[GitHub\] Check suite (\S+) on (\S+) \(branch: ([^)]+)\)")
        .expect("check_suite regex")
});

/// Parse PR URL and/or branch from gateway-formatted webhook text.
///
/// Returns `None` for event types that cannot be correlated to a task
/// (e.g. `issues.assigned`, `issue_comment.created`).
pub fn correlate_webhook(text: &str) -> Option<WebhookCorrelation> {
    // Try pull_request_review format first (most common deferral case)
    if let Some(event) = parse_pr_review_event(text) {
        return Some(WebhookCorrelation {
            pr_url: Some(event.pr_url()),
            branch: None,
            event_desc: format!(
                "pull_request_review.submitted on {}#{}",
                event.repo, event.pr_number
            ),
        });
    }

    // Try check_suite format
    if let Some(caps) = CHECK_SUITE_RE.captures(text.lines().next().unwrap_or("")) {
        let conclusion = &caps[1];
        let repo = &caps[2];
        let branch = &caps[3];
        return Some(WebhookCorrelation {
            pr_url: None,
            branch: Some(branch.to_string()),
            event_desc: format!("check_suite.completed({conclusion}) on {repo}"),
        });
    }

    // No correlation for other event types (issues, issue_comment, etc.)
    None
}

/// Check whether a webhook should be deferred based on task state.
///
/// Returns `Some((task_id, event_desc))` if the webhook should be deferred,
/// `None` if it should be processed immediately.
pub async fn should_defer_webhook(
    db: &AsyncDatabase,
    correlation: &WebhookCorrelation,
) -> Option<(String, String)> {
    // Strategy 1: Try to find task by PR URL
    if let Some(ref pr_url) = correlation.pr_url
        && let Ok(Some(task)) = db.find_active_task_by_pr_url(pr_url).await
    {
        if has_active_callback_child(db, &task.id).await {
            return Some((task.id.clone(), correlation.event_desc.clone()));
        }
        // Task found but no active callback — process immediately
        return None;
    }

    // Strategy 2: Try to find task by branch
    if let Some(ref branch) = correlation.branch
        && let Ok(Some(task)) = db.find_active_task_by_branch(branch).await
    {
        if has_active_callback_child(db, &task.id).await {
            return Some((task.id.clone(), correlation.event_desc.clone()));
        }
        return None;
    }

    // Strategy 3: Fallback — check if exactly one active task has an active callback.
    // This handles the pre-pr_url state where metadata hasn't been written yet.
    find_sole_inflight_callback_task(db)
        .await
        .map(|task_id| (task_id, correlation.event_desc.clone()))
}

/// Check if a task has any active (pending or in_progress) callback child tasks.
pub(crate) async fn has_active_callback_child(db: &AsyncDatabase, task_id: &str) -> bool {
    match db.get_child_tasks(task_id).await {
        Ok(children) => children.iter().any(|c| {
            c.trigger_type == trigger_type::CALLBACK
                && matches!(c.status.as_str(), "pending" | "in_progress")
        }),
        Err(e) => {
            warn!(
                task_id = %task_id,
                error = %e,
                "failed to check callback children, allowing webhook through"
            );
            false // Fail-open: process the webhook rather than blocking it
        }
    }
}

/// Find exactly one active task with an active callback child for this agent.
///
/// Returns `Some(task_id)` if exactly one such task exists (unambiguous deferral).
/// Returns `None` if zero or multiple exist (ambiguous — don't defer).
async fn find_sole_inflight_callback_task(db: &AsyncDatabase) -> Option<String> {
    // Get all active manual tasks for this agent
    let tasks = match db.list_active_tasks().await {
        Ok(items) => items,
        Err(e) => {
            warn!(error = %e, "failed to list active tasks for fallback deferral check");
            return None;
        }
    };

    let mut inflight_ids: Vec<String> = Vec::new();
    for item in &tasks {
        if has_active_callback_child(db, &item.id).await {
            inflight_ids.push(item.id.clone());
        }
    }

    if inflight_ids.len() == 1 {
        debug!(
            task_id = %inflight_ids[0],
            "fallback deferral: exactly one task with active callback found"
        );
        Some(inflight_ids.remove(0))
    } else {
        if inflight_ids.len() > 1 {
            debug!(
                count = inflight_ids.len(),
                "fallback deferral: multiple tasks with active callbacks — not deferring"
            );
        }
        None
    }
}

/// Drain deferred webhooks for a specific task from the queue.
///
/// Returns the webhooks in arrival order (oldest first).
pub fn drain_for_task(queue: &mut Vec<DeferredWebhook>, task_id: &str) -> Vec<DeferredWebhook> {
    let mut drained = Vec::new();
    let mut i = 0;
    while i < queue.len() {
        if queue[i].task_id == task_id {
            drained.push(queue.remove(i));
        } else {
            i += 1;
        }
    }
    drained
}

/// Drain all expired webhooks from the queue (past their deadline).
pub fn drain_expired(queue: &mut Vec<DeferredWebhook>) -> Vec<DeferredWebhook> {
    let now = Instant::now();
    let mut expired = Vec::new();
    let mut i = 0;
    while i < queue.len() {
        if queue[i].deadline <= now {
            expired.push(queue.remove(i));
        } else {
            i += 1;
        }
    }
    expired
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_request(text: &str) -> MessageRequest {
        MessageRequest {
            text: text.to_string(),
            chat_id: None,
            channel: "github".to_string(),
            request_id: "test-req-1".to_string(),
            agent: "mika-dev".to_string(),
            images: None,
        }
    }

    fn make_deferred(task_id: &str, secs_until_deadline: u64) -> DeferredWebhook {
        DeferredWebhook {
            request: make_request("test"),
            received_at: Instant::now(),
            task_id: task_id.to_string(),
            event_desc: "test event".to_string(),
            deadline: Instant::now() + Duration::from_secs(secs_until_deadline),
        }
    }

    #[test]
    fn test_correlate_pr_review() {
        let text = "[GitHub] PR review (approved) on senara-solutions/mika#42 (feat: add thing) by @reviewer\nhttps://github.com/senara-solutions/mika/pull/42#pullrequestreview-123\n\nVERDICT: pass";
        let result = correlate_webhook(text);
        assert!(result.is_some());
        let c = result.unwrap();
        assert_eq!(
            c.pr_url,
            Some("https://github.com/senara-solutions/mika/pull/42".to_string())
        );
        assert!(c.branch.is_none());
        assert!(c.event_desc.contains("pull_request_review"));
        assert!(c.event_desc.contains("#42"));
    }

    #[test]
    fn test_correlate_check_suite_failure() {
        // Matches actual gateway format: no parentheses around conclusion
        let text = "[GitHub] Check suite failure on senara-solutions/mika (branch: feat/my-feature)\nhttps://github.com/...";
        let result = correlate_webhook(text);
        assert!(result.is_some());
        let c = result.unwrap();
        assert!(c.pr_url.is_none());
        assert_eq!(c.branch, Some("feat/my-feature".to_string()));
        assert!(c.event_desc.contains("check_suite"));
        assert!(c.event_desc.contains("failure"));
    }

    #[test]
    fn test_correlate_check_suite_success() {
        let text = "[GitHub] Check suite success on org/repo (branch: main)";
        let result = correlate_webhook(text);
        assert!(result.is_some());
        let c = result.unwrap();
        assert_eq!(c.branch, Some("main".to_string()));
        assert!(c.event_desc.contains("success"));
    }

    #[test]
    fn test_correlate_check_suite_timed_out() {
        let text = "[GitHub] Check suite timed_out on org/repo (branch: feat/x)";
        let result = correlate_webhook(text);
        assert!(result.is_some());
        let c = result.unwrap();
        assert_eq!(c.branch, Some("feat/x".to_string()));
        assert!(c.event_desc.contains("timed_out"));
    }

    #[test]
    fn test_correlate_issue_returns_none() {
        let text =
            "[GitHub] Issue assigned: senara-solutions/mika#100 — Fix bug\nhttps://github.com/...";
        assert!(correlate_webhook(text).is_none());
    }

    #[test]
    fn test_correlate_issue_comment_returns_none() {
        let text = "[GitHub] New comment on senara-solutions/mika#50 (Some title) by @user\nhttps://github.com/...\n\nComment body";
        assert!(correlate_webhook(text).is_none());
    }

    #[test]
    fn test_drain_for_task() {
        let mut queue = vec![
            make_deferred("wi-1", 60),
            make_deferred("wi-2", 60),
            make_deferred("wi-1", 60),
            make_deferred("wi-3", 60),
        ];

        let drained = drain_for_task(&mut queue, "wi-1");
        assert_eq!(drained.len(), 2);
        assert_eq!(queue.len(), 2);
        assert!(drained.iter().all(|d| d.task_id == "wi-1"));
    }

    #[test]
    fn test_drain_for_task_no_match() {
        let mut queue = vec![make_deferred("wi-1", 60), make_deferred("wi-2", 60)];

        let drained = drain_for_task(&mut queue, "wi-99");
        assert!(drained.is_empty());
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn test_drain_expired() {
        let mut queue = vec![
            // Already expired (deadline in the past)
            DeferredWebhook {
                request: make_request("expired"),
                received_at: Instant::now() - Duration::from_secs(120),
                task_id: "wi-1".to_string(),
                event_desc: "expired event".to_string(),
                deadline: Instant::now() - Duration::from_secs(1),
            },
            // Not yet expired
            make_deferred("wi-2", 60),
        ];

        let expired = drain_expired(&mut queue);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].task_id, "wi-1");
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].task_id, "wi-2");
    }
}

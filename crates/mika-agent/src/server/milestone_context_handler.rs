//! Milestone-context marker injector for PR-closed webhook turns (mika#1218).
//!
//! When a `[GitHub] PR closed:` webhook arrives and correlates to a task whose
//! parent has `type IN ('milestone', 'project')`, this handler prepends a
//! `[milestone-parent: <parent_id>]` marker to the user message so the inline
//! `webhook_milestone_advance` guard in `agent.rs` can fire.
//!
//! Never returns `VerdictAction::Handled` — the LLM still owns the
//! advance/halt decision.

use super::verdict_handler::VerdictAction;
use crate::async_db::AsyncDatabase;
use tracing::{debug, info, warn};

/// Prefix on PR-closed webhook messages from the gateway.
const PR_CLOSED_PREFIX: &str = "[GitHub] PR closed:";

/// Sentinel line indicating a merged PR (gateway appends this on close events).
const MERGED_TRUE_LINE: &str = "\nMerged: true";

/// Inspect a PR-closed webhook message and inject a `[milestone-parent: <id>]`
/// marker when the correlated task has a milestone/project parent.
///
/// Returns `Passthrough { enrichment: Some(...) }` with the marker when
/// conditions are met; otherwise `Passthrough { enrichment: None }`.
pub(crate) async fn try_handle_pr_closed_milestone_context(
    text: &str,
    db: &AsyncDatabase,
) -> VerdictAction {
    // Step 1: Match prefix.
    if !text.starts_with(PR_CLOSED_PREFIX) {
        return VerdictAction::Passthrough { enrichment: None };
    }

    // Step 2: Gate on merge truth (Pin F).
    if !text.contains(MERGED_TRUE_LINE) {
        debug!("milestone_context: PR closed but not merged, skipping marker injection");
        return VerdictAction::Passthrough { enrichment: None };
    }

    // Step 3: Extract PR URL — scan for the first GitHub PR URL line.
    let pr_url = match extract_pr_url(text) {
        Some(url) => url,
        None => {
            debug!("milestone_context: no PR URL found in webhook message");
            return VerdictAction::Passthrough { enrichment: None };
        }
    };

    // Step 4: Correlate task via PR URL.
    let task = match db.find_active_task_by_pr_url(&pr_url).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            info!(
                pr_url = %pr_url,
                "milestone_context: no active task found for PR"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
        Err(e) => {
            warn!(
                error = %e,
                pr_url = %pr_url,
                "milestone_context: failed to look up task by PR URL"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
    };

    // Check if the task has a parent.
    let parent_id = match &task.parent_task_id {
        Some(id) => id.clone(),
        None => {
            return VerdictAction::Passthrough { enrichment: None };
        }
    };

    // Step 5: Fetch parent task and check type.
    let parent = match db.get_task(&parent_id).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            warn!(
                task_id = %task.id,
                parent_task_id = %parent_id,
                "milestone_context: parent task not found"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
        Err(e) => {
            warn!(
                error = %e,
                parent_task_id = %parent_id,
                "milestone_context: failed to fetch parent task"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
    };

    let parent_type = &parent.r#type;
    if parent_type != "milestone" && parent_type != "project" {
        return VerdictAction::Passthrough { enrichment: None };
    }

    // Step 6: Emit the marker.
    info!(
        pr_url = %pr_url,
        task_id = %task.id,
        parent_task_id = %parent_id,
        parent_type = %parent_type,
        "milestone_context: injecting milestone-parent marker"
    );

    VerdictAction::Passthrough {
        enrichment: Some(format!("[milestone-parent: {parent_id}]\n")),
    }
}

/// Extract the first GitHub PR URL from the webhook message text.
/// Scans lines for `https://github.com/<owner>/<repo>/pull/<number>`.
fn extract_pr_url(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("https://github.com/") && trimmed.contains("/pull/") {
            // Validate the URL shape: https://github.com/<owner>/<repo>/pull/<number>
            let Some(suffix) = trimmed.strip_prefix("https://github.com/") else {
                continue;
            };
            let parts: Vec<&str> = suffix.splitn(4, '/').collect();
            // Require exactly 4 segments: owner, repo, "pull", number
            if parts.len() >= 4 && parts[2] == "pull" && !parts[3].is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_pr_url_valid() {
        let text = "[GitHub] PR closed: senara-solutions/mika#1000 — title (branch: foo)\nhttps://github.com/senara-solutions/mika/pull/1000\nMerged: true";
        assert_eq!(
            extract_pr_url(text),
            Some("https://github.com/senara-solutions/mika/pull/1000".to_string())
        );
    }

    #[test]
    fn test_extract_pr_url_missing() {
        let text =
            "[GitHub] PR closed: senara-solutions/mika#1000 — title (branch: foo)\nMerged: true";
        assert_eq!(extract_pr_url(text), None);
    }

    #[test]
    fn test_extract_pr_url_not_github() {
        let text = "https://gitlab.com/foo/bar/pull/123";
        assert_eq!(extract_pr_url(text), None);
    }
}

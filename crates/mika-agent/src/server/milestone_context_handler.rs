//! Milestone-context marker injector for PR-closed webhook turns (mika#1218).
//!
//! When a `[GitHub] PR closed:` webhook arrives and correlates to a task whose
//! parent has `type IN ('milestone', 'project')`, this handler prepends a
//! `[milestone-parent: <parent_id>]` marker to the user message so the inline
//! `webhook_milestone_advance` guard in `agent.rs` can fire.
//!
//! Phase tracking (mika#1153 E3): computes and persists phase progress on the
//! parent milestone task's metadata after sub-issue PR merges.
//!
//! Ready-label cascade (mika#1153 E5): when all current-phase sub-issues are
//! closed, automatically labels next-phase sub-issues `ready` to trigger the
//! autonomous dispatch loop.
//!
//! Never returns `VerdictAction::Handled` — the LLM still owns the
//! advance/halt decision.

use super::verdict_handler::VerdictAction;
use crate::async_db::AsyncDatabase;
use crate::github_graphql::{
    add_label_to_issue, fetch_milestone_issues_by_state, parse_phase_label,
};
use crate::task_metadata::merge_metadata;
use tracing::{debug, info, warn};

/// Prefix on PR-closed webhook messages from the gateway.
const PR_CLOSED_PREFIX: &str = "[GitHub] PR closed:";

/// Sentinel line indicating a merged PR (gateway appends this on close events).
const MERGED_TRUE_LINE: &str = "\nMerged: true";

/// Inspect a PR-closed webhook message and inject a `[milestone-parent: <id>]`
/// marker when the correlated task has a milestone/project parent.
///
/// Also computes phase progress and triggers ready-label cascade (mika#1153).
///
/// Returns `Passthrough { enrichment: Some(...) }` with the marker when
/// conditions are met; otherwise `Passthrough { enrichment: None }`.
pub(crate) async fn try_handle_pr_closed_milestone_context(
    text: &str,
    db: &AsyncDatabase,
    github_token: Option<&str>,
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

    // Step 7: Phase tracking + cascade (mika#1153, fire-and-forget).
    // Errors here never block marker emission.
    if let Some(token) = github_token
        && let Some(milestone_number) = extract_milestone_number(parent.reference_url.as_deref())
    {
        let (owner, repo) = match extract_owner_repo(parent.reference_url.as_deref()) {
            Some(pair) => pair,
            None => {
                debug!("milestone_context: could not extract owner/repo from reference_url");
                return VerdictAction::Passthrough {
                    enrichment: Some(format!("[milestone-parent: {parent_id}]\n")),
                };
            }
        };

        // Fire-and-forget: phase tracking + cascade
        try_phase_tracking_and_cascade(
            token,
            &owner,
            &repo,
            milestone_number,
            &parent_id,
            &parent.metadata,
            db,
        )
        .await;
    }

    VerdictAction::Passthrough {
        enrichment: Some(format!("[milestone-parent: {parent_id}]\n")),
    }
}

/// Phase tracking and cascade logic (mika#1153 E3 + E5).
///
/// Fire-and-forget: errors are logged but never block the caller.
async fn try_phase_tracking_and_cascade(
    token: &str,
    owner: &str,
    repo: &str,
    milestone_number: u64,
    parent_task_id: &str,
    parent_metadata: &Option<String>,
    db: &AsyncDatabase,
) {
    // Fetch all sub-issues to determine phase progress.
    let all_issues =
        match fetch_milestone_issues_by_state(token, owner, repo, milestone_number, "all").await {
            Ok(issues) => issues,
            Err(e) => {
                warn!(
                    error = %e,
                    milestone_number = milestone_number,
                    "milestone_context: failed to fetch milestone issues for phase tracking"
                );
                return;
            }
        };

    // Determine if this is a phased milestone — check if any sub-issue has a phase label.
    let mut phase_issues: Vec<(u64, u32, &str)> = Vec::new(); // (issue_number, phase, state)
    for issue in &all_issues {
        if let Some(phase) = parse_phase_label(&issue.labels) {
            phase_issues.push((issue.number, phase, &issue.state));
        }
    }

    if phase_issues.is_empty() {
        // Non-phased milestone — skip phase tracking entirely.
        debug!(
            milestone_number = milestone_number,
            "milestone_context: no phase labels on sub-issues, skipping phase tracking"
        );
        return;
    }

    // Compute phase progress.
    let phase_count = phase_issues.iter().map(|(_, p, _)| *p).max().unwrap_or(1);
    // Current phase: lowest phase with any OPEN issue, or phase_count if all closed.
    let current_phase = (1..=phase_count)
        .find(|&p| {
            phase_issues
                .iter()
                .any(|(_, pp, state)| *pp == p && *state == "open")
        })
        .unwrap_or(phase_count);
    let phase_total = phase_issues
        .iter()
        .filter(|(_, p, _)| *p == current_phase)
        .count() as u32;
    let phase_completed = phase_issues
        .iter()
        .filter(|(_, p, state)| *p == current_phase && *state == "closed")
        .count() as u32;

    // Write phase progress to parent task metadata.
    let phase_progress = serde_json::json!({
        "phase_count": phase_count,
        "current_phase": current_phase,
        "phase_total": phase_total,
        "phase_completed": phase_completed,
    });

    let mut base_metadata: serde_json::Value = parent_metadata
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    let incoming = serde_json::json!({ "phase_progress": phase_progress });
    merge_metadata(&mut base_metadata, &incoming);

    let merged_str = base_metadata.to_string();
    if let Err(e) = db.update_task_metadata(parent_task_id, &merged_str).await {
        warn!(
            error = %e,
            parent_task_id = parent_task_id,
            "milestone_context: failed to write phase metadata (fire-and-forget)"
        );
    }

    info!(
        parent_task_id = parent_task_id,
        milestone_number = milestone_number,
        phase_count = phase_count,
        current_phase = current_phase,
        phase_total = phase_total,
        phase_completed = phase_completed,
        "milestone_context: phase progress updated"
    );

    // Ready-label cascade (E5): if all current-phase issues are closed and
    // there is a next phase, label phase-(K+1) issues `ready`.
    if phase_completed == phase_total && current_phase < phase_count {
        try_phase_cascade(
            token,
            owner,
            repo,
            current_phase,
            &phase_issues,
            parent_task_id,
            milestone_number,
            &mut base_metadata,
            db,
        )
        .await;
    }
}

/// Cascade ready labels to next-phase issues (mika#1153 E5).
#[allow(clippy::too_many_arguments)]
async fn try_phase_cascade(
    token: &str,
    owner: &str,
    repo: &str,
    completed_phase: u32,
    phase_issues: &[(u64, u32, &str)],
    parent_task_id: &str,
    milestone_number: u64,
    base_metadata: &mut serde_json::Value,
    db: &AsyncDatabase,
) {
    // Check env var opt-out.
    let auto_cascade = std::env::var("MIKA_PHASE_CASCADE_AUTO")
        .map(|v| !v.eq_ignore_ascii_case("false") && v != "0")
        .unwrap_or(true); // default: auto

    let next_phase = completed_phase + 1;

    if !auto_cascade {
        // Write pending flag to metadata instead of labeling.
        let incoming = serde_json::json!({ "phase_cascade_pending": true });
        merge_metadata(base_metadata, &incoming);
        let merged_str = base_metadata.to_string();
        if let Err(e) = db.update_task_metadata(parent_task_id, &merged_str).await {
            warn!(
                error = %e,
                parent_task_id = parent_task_id,
                "milestone_context: failed to write phase_cascade_pending"
            );
        }
        info!(
            parent_task_id = parent_task_id,
            from_phase = completed_phase,
            to_phase = next_phase,
            "milestone_context: phase cascade pending (auto=false), operator must label manually"
        );
        return;
    }

    // Collect next-phase issue numbers.
    let next_phase_issues: Vec<u64> = phase_issues
        .iter()
        .filter(|(_, p, _)| *p == next_phase)
        .map(|(n, _, _)| *n)
        .collect();

    if next_phase_issues.is_empty() {
        return;
    }

    let mut issues_labeled = 0u32;
    for issue_number in &next_phase_issues {
        match add_label_to_issue(token, owner, repo, *issue_number, "ready").await {
            Ok(()) => {
                issues_labeled += 1;
            }
            Err(e) => {
                warn!(
                    error = %e,
                    issue_number = issue_number,
                    milestone_number = milestone_number,
                    "milestone_context: failed to label issue ready (continuing)"
                );
            }
        }
    }

    info!(
        parent_task_id = parent_task_id,
        milestone_number = milestone_number,
        from_phase = completed_phase,
        to_phase = next_phase,
        issues_labeled = issues_labeled,
        total_next_phase = next_phase_issues.len(),
        "phase_cascade_triggered"
    );
}

/// Extract the milestone number from a reference URL.
/// Expects format: `https://github.com/{owner}/{repo}/milestone/{number}`
fn extract_milestone_number(reference_url: Option<&str>) -> Option<u64> {
    let url = reference_url?;
    // Look for /milestone/ or /milestones/ in the URL
    let idx = url
        .find("/milestone/")
        .or_else(|| url.find("/milestones/"))?;
    let prefix_len = if url[idx..].starts_with("/milestones/") {
        "/milestones/".len()
    } else {
        "/milestone/".len()
    };
    let rest = &url[idx + prefix_len..];
    // Take digits until non-digit
    let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    num_str.parse::<u64>().ok()
}

/// Extract (owner, repo) from a GitHub reference URL.
/// Expects format: `https://github.com/{owner}/{repo}/...`
fn extract_owner_repo(reference_url: Option<&str>) -> Option<(String, String)> {
    let url = reference_url?;
    let path = url.strip_prefix("https://github.com/")?;
    let parts: Vec<&str> = path.splitn(3, '/').collect();
    if parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
        return None;
    }
    Some((parts[0].to_string(), parts[1].to_string()))
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

    // -- Milestone number extraction (mika#1153) --

    #[test]
    fn test_extract_milestone_number_valid() {
        let url = Some("https://github.com/senara-solutions/mika/milestone/14");
        assert_eq!(extract_milestone_number(url), Some(14));
    }

    #[test]
    fn test_extract_milestone_number_milestones_plural() {
        let url = Some("https://github.com/senara-solutions/mika/milestones/7");
        assert_eq!(extract_milestone_number(url), Some(7));
    }

    #[test]
    fn test_extract_milestone_number_none() {
        assert_eq!(extract_milestone_number(None), None);
    }

    #[test]
    fn test_extract_milestone_number_no_milestone_path() {
        let url = Some("https://github.com/senara-solutions/mika/issues/100");
        assert_eq!(extract_milestone_number(url), None);
    }

    // -- Owner/repo extraction --

    #[test]
    fn test_extract_owner_repo_valid() {
        let url = Some("https://github.com/senara-solutions/mika/milestone/14");
        assert_eq!(
            extract_owner_repo(url),
            Some(("senara-solutions".to_string(), "mika".to_string()))
        );
    }

    #[test]
    fn test_extract_owner_repo_none() {
        assert_eq!(extract_owner_repo(None), None);
    }

    #[test]
    fn test_extract_owner_repo_not_github() {
        let url = Some("https://gitlab.com/foo/bar");
        assert_eq!(extract_owner_repo(url), None);
    }
}

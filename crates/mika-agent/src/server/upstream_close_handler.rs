//! Complete-on-upstream-close cleanup for phantom tracking rows (mika#1934 AC4).
//!
//! The escalation surface writes a tracking row's parent-status to `blocked` and
//! then never converts it to a terminal state, because the operator resolves the
//! underlying ticket out-of-band on GitHub (merges the fix in a different PR,
//! closes the issue, supersedes it). None of those out-of-band events reach back
//! to the tracking row today — the mika#1712 sweep is the only thing that drains
//! them, at a 3600s grace. This handler closes that gap synchronously: when an
//! `issues.closed` or `pull_request.closed` webhook arrives, it terminal-marks
//! any `blocked`/`in_progress` tracking rows whose `reference_url` matches the
//! closed issue.
//!
//! Side-effect-only injector: it never returns `Handled`/`Dispatched` — the LLM
//! still owns whatever the webhook turn does. Mirrors the shape of
//! `milestone_context_handler` and `draft_pr_opened_handler`.
//!
//! Idempotent + no-op safe (AC4.c): the guarded UPDATE short-circuits on
//! already-terminal rows (re-delivery safe), and a webhook with zero matching
//! rows logs at DEBUG only — no WARN, no audit event.

use super::verdict_handler::VerdictAction;
use crate::async_db::AsyncDatabase;
use crate::task_state::tasks::{
    ISSUE_CLOSED_UPSTREAM, TRACKING_ROW_UPSTREAM_CLOSED_TOOL, UPSTREAM_PR_CLOSED_UNMERGED,
    UPSTREAM_PR_MERGED, strip_groom_phase_suffix,
};
use std::sync::LazyLock;
use tracing::{debug, info, warn};

/// Prefix on issue-closed webhook messages from the gateway.
const ISSUE_CLOSED_PREFIX: &str = "[GitHub] Issue closed:";

/// Prefix on PR-closed webhook messages from the gateway.
const PR_CLOSED_PREFIX: &str = "[GitHub] PR closed:";

/// Sentinel line indicating a merged PR (gateway appends this on close events).
const MERGED_TRUE_LINE: &str = "\nMerged: true";

/// Matches closing keywords (`Closes`/`Fixes`/`Resolves`, any inflection) plus a
/// same-repo issue reference (`#<n>`) in a PR body. Case-insensitive.
static CLOSING_REF_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\b(?:clos(?:e|es|ed)|fix(?:e|es|ed)|resolv(?:e|es|ed))\s+#(\d+)")
        .expect("closing-ref regex is valid")
});

/// Inspect an `issues.closed` / `pull_request.closed` webhook message and
/// terminal-mark any phantom tracking rows whose `reference_url` matches the
/// closed issue (mika#1934 AC4).
///
/// Always returns `Passthrough` — side-effect only. `Some(enrichment)` is never
/// produced; the handler mutates DB rows, it does not rewrite the message.
pub async fn try_handle_upstream_close(
    text: &str,
    db: &AsyncDatabase,
    session_id: &str,
    trace_id: &str,
) -> VerdictAction {
    if text.starts_with(ISSUE_CLOSED_PREFIX) {
        handle_issue_closed(text, db, session_id, trace_id).await;
    } else if text.starts_with(PR_CLOSED_PREFIX) {
        handle_pr_closed(text, db, session_id, trace_id).await;
    }
    // Not a close event we own, or handled above — either way, pass through.
    VerdictAction::Passthrough { enrichment: None }
}

/// `issues.closed`: the closed issue's own URL is the tracking `reference_url`.
async fn handle_issue_closed(text: &str, db: &AsyncDatabase, session_id: &str, trace_id: &str) {
    let Some(issue_url) = extract_issue_url(text) else {
        debug!("upstream_close: no issue URL in issues.closed message");
        return;
    };
    cleanup_rows_for_issue_url(
        db,
        session_id,
        trace_id,
        &issue_url,
        "issues.closed",
        "cancelled",
        ISSUE_CLOSED_UPSTREAM,
    )
    .await;
}

/// `pull_request.closed`: the tracking `reference_url` is the ISSUE the PR
/// closes, parsed from the PR body's `Closes #<n>` / `Fixes #<n>` refs. A merged
/// PR completes the row; an unmerged close cancels it.
async fn handle_pr_closed(text: &str, db: &AsyncDatabase, session_id: &str, trace_id: &str) {
    let Some(pr_url) = extract_pr_url(text) else {
        debug!("upstream_close: no PR URL in pull_request.closed message");
        return;
    };
    let Some((owner, repo)) = extract_owner_repo_from_url(&pr_url) else {
        debug!("upstream_close: could not parse owner/repo from PR URL");
        return;
    };

    let merged = text.contains(MERGED_TRUE_LINE);
    let (new_status, result) = if merged {
        ("completed", UPSTREAM_PR_MERGED)
    } else {
        ("cancelled", UPSTREAM_PR_CLOSED_UNMERGED)
    };

    let issue_numbers = parse_closing_issue_refs(text);
    if issue_numbers.is_empty() {
        debug!(
            pr_url = %pr_url,
            "upstream_close: PR body carries no Closes/Fixes issue refs — nothing to clean up"
        );
        return;
    }

    for number in issue_numbers {
        let issue_url = format!("https://github.com/{owner}/{repo}/issues/{number}");
        cleanup_rows_for_issue_url(
            db,
            session_id,
            trace_id,
            &issue_url,
            "pull_request.closed",
            new_status,
            result,
        )
        .await;
    }
}

/// Find phantom tracking rows for `issue_url` (exact + `?phase=groom` variant),
/// transition each to `new_status` with `result`, and emit one
/// `tracking_row_upstream_closed` audit event per transitioned row. No-op safe:
/// zero rows → DEBUG log, no audit event, no WARN.
async fn cleanup_rows_for_issue_url(
    db: &AsyncDatabase,
    session_id: &str,
    trace_id: &str,
    issue_url: &str,
    event_type: &str,
    new_status: &str,
    result: &str,
) {
    let base_url = strip_groom_phase_suffix(issue_url);
    let rows = match db
        .find_active_tracking_rows_by_reference_url_and_variants(base_url)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            warn!(
                event = "upstream_close_lookup_failed",
                issue_url = %issue_url,
                error = %e,
                "upstream_close: tracking-row lookup failed"
            );
            return;
        }
    };

    if rows.is_empty() {
        debug!(
            issue_url = %issue_url,
            event_type = %event_type,
            "upstream_close: no matching tracking rows (no-op)"
        );
        return;
    }

    for task in rows {
        match db
            .terminal_mark_tracking_row_upstream_closed(&task.id, new_status, result)
            .await
        {
            Ok(true) => {
                let reasoning = format!("event_type={event_type} reference_url={issue_url}");
                if let Err(e) = db
                    .log_audit_event(
                        session_id,
                        TRACKING_ROW_UPSTREAM_CLOSED_TOOL,
                        &format!("task:{}", task.id),
                        Some(&task.status),
                        Some(result),
                        Some(&reasoning),
                        Some(trace_id),
                    )
                    .await
                {
                    warn!(
                        event = "upstream_close_audit_failed",
                        task_id = %task.id,
                        error = %e,
                        "upstream_close: failed to write audit event (non-fatal)"
                    );
                }
                info!(
                    event = "tracking_row_upstream_closed",
                    task_id = %task.id,
                    issue_url = %issue_url,
                    event_type = %event_type,
                    new_status = %new_status,
                    result = %result,
                    "upstream_close: terminal-marked tracking row on upstream close"
                );
            }
            // Already terminal (idempotent re-delivery) or not phantom-shaped.
            Ok(false) => {}
            Err(e) => {
                warn!(
                    event = "upstream_close_transition_failed",
                    task_id = %task.id,
                    error = %e,
                    "upstream_close: guarded transition failed"
                );
            }
        }
    }
}

/// Extract the first GitHub issue URL from the webhook message text.
/// Scans lines for `https://github.com/<owner>/<repo>/issues/<number>`.
fn extract_issue_url(text: &str) -> Option<String> {
    extract_github_url_with_segment(text, "issues")
}

/// Extract the first GitHub PR URL from the webhook message text.
/// Scans lines for `https://github.com/<owner>/<repo>/pull/<number>`.
fn extract_pr_url(text: &str) -> Option<String> {
    extract_github_url_with_segment(text, "pull")
}

/// Shared scanner: first line shaped `https://github.com/<owner>/<repo>/<seg>/<n>`.
fn extract_github_url_with_segment(text: &str, seg: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(suffix) = trimmed.strip_prefix("https://github.com/") else {
            continue;
        };
        let parts: Vec<&str> = suffix.splitn(4, '/').collect();
        if parts.len() >= 4 && parts[2] == seg && !parts[3].is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

/// Extract (owner, repo) from a GitHub URL of the form
/// `https://github.com/{owner}/{repo}/...`.
fn extract_owner_repo_from_url(url: &str) -> Option<(String, String)> {
    let path = url.strip_prefix("https://github.com/")?;
    let parts: Vec<&str> = path.splitn(3, '/').collect();
    if parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
        return None;
    }
    Some((parts[0].to_string(), parts[1].to_string()))
}

/// Parse the same-repo issue numbers a PR body says it closes
/// (`Closes #N` / `Fixes #N` / `Resolves #N`, any inflection, case-insensitive).
/// Deduplicated, order-preserving.
fn parse_closing_issue_refs(text: &str) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    for cap in CLOSING_REF_RE.captures_iter(text) {
        if let Some(m) = cap.get(1)
            && let Ok(n) = m.as_str().parse::<u64>()
            && !out.contains(&n)
        {
            out.push(n);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_issue_url_valid() {
        let text = "[GitHub] Issue closed: senara-solutions/mika#1574 — title\nhttps://github.com/senara-solutions/mika/issues/1574";
        assert_eq!(
            extract_issue_url(text),
            Some("https://github.com/senara-solutions/mika/issues/1574".to_string())
        );
    }

    #[test]
    fn test_extract_issue_url_ignores_pr() {
        let text = "https://github.com/senara-solutions/mika/pull/1574";
        assert_eq!(extract_issue_url(text), None);
    }

    #[test]
    fn test_extract_pr_url_valid() {
        let text = "[GitHub] PR closed: senara-solutions/mika#1000 — t (branch: b)\nhttps://github.com/senara-solutions/mika/pull/1000\nMerged: true";
        assert_eq!(
            extract_pr_url(text),
            Some("https://github.com/senara-solutions/mika/pull/1000".to_string())
        );
    }

    #[test]
    fn test_extract_owner_repo_from_url() {
        assert_eq!(
            extract_owner_repo_from_url("https://github.com/senara-solutions/mika/pull/1000"),
            Some(("senara-solutions".to_string(), "mika".to_string()))
        );
        assert_eq!(extract_owner_repo_from_url("https://gitlab.com/a/b"), None);
    }

    #[test]
    fn test_parse_closing_issue_refs_variants() {
        let body = "This PR Closes #10, fixes #20 and Resolved #30. Also closes #10 again.";
        assert_eq!(parse_closing_issue_refs(body), vec![10, 20, 30]);
    }

    #[test]
    fn test_parse_closing_issue_refs_none() {
        let body = "No refs here, just #hashtag-ish text without a keyword.";
        assert_eq!(parse_closing_issue_refs(body), Vec::<u64>::new());
    }

    #[test]
    fn test_merged_sentinel() {
        let merged = "[GitHub] PR closed: a/b#1 — t (branch: x)\nurl\nMerged: true\n\nCloses #1";
        let unmerged = "[GitHub] PR closed: a/b#1 — t (branch: x)\nurl\nMerged: false\n\nCloses #1";
        assert!(merged.contains(MERGED_TRUE_LINE));
        assert!(!unmerged.contains(MERGED_TRUE_LINE));
    }
}

//! Auto-pull groomed tickets for mika-dev (mika#1363).
//!
//! When mika-dev's dispatch queue is idle, this module selects the
//! highest-priority groomed-not-ready ticket and applies the `ready`
//! label to trigger the webhook-driven dispatch flow.

use anyhow::{Result, anyhow};
use regex::Regex;
use std::sync::OnceLock;
use tracing::{debug, info, warn};

use crate::async_db::AsyncDatabase;

/// Default repo for auto-pull (mika-only for v1).
const DEFAULT_REPO: &str = "senara-solutions/mika";

/// Maximum failure count before a ticket is skipped by the circuit-breaker.
const CIRCUIT_BREAKER_THRESHOLD: i64 = 3;

// ───────────────────── Issue types ─────────────────────

#[derive(Debug, Clone)]
pub struct IssueLabel {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Issue {
    pub number: u64,
    pub body: String,
    pub labels: Vec<IssueLabel>,
    pub updated_at: String,
}

// ───────────────────── Grooming detection (F1) ─────────────────────

/// Structural detection of a groomed issue body.
///
/// Matches the canonical callout block emitted by the grooming pipeline:
/// ```text
/// > - **Branch:** `<branch>`
/// > - **Plan:** `docs/plans/<file>.md` (committed on branch @ <sha>)
/// > - **Grooming history:** <...> → second-pass (GROOMED) — session-id: <uuid>
/// ```
pub fn is_groomed(body: &str) -> bool {
    static GROOMING_HISTORY_RE: OnceLock<Regex> = OnceLock::new();
    let re = GROOMING_HISTORY_RE.get_or_init(|| {
        Regex::new(r"(?m)^> - \*\*Grooming history:\*\*.+second-pass \(GROOMED\)")
            .expect("grooming history regex must compile")
    });
    re.is_match(body)
        && body.contains("> - **Branch:** `")
        && body.contains("> - **Plan:** `docs/plans/")
}

// ───────────────────── Priority ranking ─────────────────────

/// Returns a numeric rank for priority labels: p0=4, p1=3, p2=2, p3=1, unlabelled=0.
/// Higher rank = higher priority.
pub fn priority_rank(labels: &[IssueLabel]) -> u8 {
    for label in labels {
        match label.name.as_str() {
            "p0" => return 4,
            "p1" => return 3,
            "p2" => return 2,
            "p3" => return 1,
            _ => {}
        }
    }
    0 // unlabelled
}

// ───────────────────── Selection logic ─────────────────────

/// Select the best groomed-not-ready ticket from a list of open issues.
///
/// Filters to groomed-only, ranks by priority (p0 > p1 > p2 > p3 > unlabelled),
/// then by oldest `updated_at` within same priority.
pub fn select_best_candidate(issues: Vec<Issue>) -> Option<Issue> {
    // Filter out issues that already have the `ready` label
    let without_ready: Vec<_> = issues
        .into_iter()
        .filter(|i| !i.labels.iter().any(|l| l.name == "ready"))
        .collect();

    // Filter to groomed-only
    let groomed: Vec<_> = without_ready
        .into_iter()
        .filter(|i| is_groomed(&i.body))
        .collect();

    if groomed.is_empty() {
        return None;
    }

    // Sort by priority (descending), then by updated_at (ascending = oldest first)
    groomed.into_iter().max_by(|a, b| {
        let pa = priority_rank(&a.labels);
        let pb = priority_rank(&b.labels);
        pa.cmp(&pb).then_with(|| b.updated_at.cmp(&a.updated_at))
    })
}

// ───────────────────── GitHub CLI helpers ─────────────────────

/// List open issues from the default repo via `gh issue list`.
///
/// Returns all open issues with number, body, labels, and updatedAt.
/// `gh` CLI has no negative label filter, so we fetch all and filter client-side.
async fn gh_list_open_issues(github_token: &str) -> Result<Vec<Issue>> {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.args([
        "issue",
        "list",
        "--repo",
        DEFAULT_REPO,
        "--state",
        "open",
        "--json",
        "number,body,labels,updatedAt",
        "--limit",
        "100",
    ]);
    cmd.env("GH_TOKEN", github_token);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);

    let output = cmd.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("gh issue list failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let raw: Vec<serde_json::Value> = serde_json::from_str(&stdout)?;

    let issues = raw
        .into_iter()
        .filter_map(|v| {
            let number = v["number"].as_u64()?;
            let body = v["body"].as_str().unwrap_or_default().to_string();
            let updated_at = v["updatedAt"].as_str().unwrap_or_default().to_string();
            let labels = v["labels"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|l| {
                            l["name"].as_str().map(|n| IssueLabel {
                                name: n.to_string(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(Issue {
                number,
                body,
                labels,
                updated_at,
            })
        })
        .collect();

    Ok(issues)
}

/// Apply a label to a GitHub issue.
async fn gh_apply_label(github_token: &str, issue_number: u64, label: &str) -> Result<()> {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.args([
        "issue",
        "edit",
        &issue_number.to_string(),
        "--repo",
        DEFAULT_REPO,
        "--add-label",
        label,
    ]);
    cmd.env("GH_TOKEN", github_token);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);

    let output = cmd.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "gh issue edit --add-label failed for #{}: {}",
            issue_number,
            stderr
        ));
    }
    Ok(())
}

// ───────────────────── Orchestration ─────────────────────

/// Run the auto-pull groomed ticket logic.
///
/// Returns `Some(issue_number)` if a ticket was selected and labelled `ready`,
/// `None` if no action was taken (queue not empty, no candidates, or circuit-breaker skip).
pub async fn auto_pull_groomed_ticket(
    db: &AsyncDatabase,
    github_token: &str,
    trace_id: &str,
    session_id: &str,
) -> Option<u64> {
    // 1. Queue-empty gate (F2): check if mika-dev has active self_dev tasks.
    let queue_count = match db.count_active_self_dev_tasks().await {
        Ok(count) => count,
        Err(e) => {
            warn!(error = %e, "auto_pull: failed to count active self_dev tasks");
            return None;
        }
    };
    if queue_count > 0 {
        debug!(
            queue_count,
            "auto_pull: queue not empty, skipping (webhook-driven dispatch covering)"
        );
        return None;
    }

    // 2. Fetch open issues from GitHub (F4: client-side filter for no `ready` label).
    let issues = match gh_list_open_issues(github_token).await {
        Ok(issues) => issues,
        Err(e) => {
            warn!(error = %e, "auto_pull: failed to list open issues");
            return None;
        }
    };

    // 3. Select the best groomed-not-ready candidate.
    let candidate = match select_best_candidate(issues) {
        Some(c) => c,
        None => {
            debug!("auto_pull: no groomed-not-ready candidates found");
            return None;
        }
    };

    // 4. Circuit-breaker check (AC3): skip if failure_count >= threshold.
    match db
        .get_auto_pull_failure_count(DEFAULT_REPO, candidate.number)
        .await
    {
        Ok(count) if count >= CIRCUIT_BREAKER_THRESHOLD => {
            info!(
                issue = candidate.number,
                failure_count = count,
                "auto_pull: circuit-breaker skip for #{} ({}× failures)",
                candidate.number,
                count
            );
            // Audit the skip decision
            if let Err(e) = db
                .log_audit_event(
                    session_id,
                    "auto_pull",
                    "auto_pull_skip",
                    None,
                    Some(&format!(
                        "#{} skipped: failure_count={} >= threshold={}",
                        candidate.number, count, CIRCUIT_BREAKER_THRESHOLD
                    )),
                    None,
                    Some(trace_id),
                )
                .await
            {
                warn!(error = %e, "auto_pull: failed to write skip audit event");
            }
            return None;
        }
        Err(e) => {
            warn!(error = %e, "auto_pull: failed to check circuit-breaker, proceeding");
            // Fail-open: proceed with the selection
        }
        _ => {} // count < threshold, proceed
    }

    let rank = priority_rank(&candidate.labels);

    // 5. Apply the `ready` label to trigger webhook-driven dispatch.
    if let Err(e) = gh_apply_label(github_token, candidate.number, "ready").await {
        warn!(
            error = %e,
            issue = candidate.number,
            "auto_pull: failed to apply ready label"
        );
        return None;
    }

    info!(
        issue = candidate.number,
        priority_rank = rank,
        "auto_pull: selected #{} and applied ready label",
        candidate.number
    );

    // 6. Record the auto-pull event for circuit-breaker tracking.
    if let Err(e) = db.record_auto_pull(DEFAULT_REPO, candidate.number).await {
        warn!(error = %e, "auto_pull: failed to record auto-pull event");
    }

    // 7. Audit the selection decision (AC5).
    if let Err(e) = db
        .log_audit_event(
            session_id,
            "auto_pull",
            "auto_pull",
            None,
            Some(&format!(
                "selected #{} (priority_rank={}, updated_at={})",
                candidate.number, rank, candidate.updated_at
            )),
            None,
            Some(trace_id),
        )
        .await
    {
        warn!(error = %e, "auto_pull: failed to write selection audit event");
    }

    Some(candidate.number)
}

// ───────────────────── Tests ─────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_groomed tests ──

    #[test]
    fn test_is_groomed_canonical_callout() {
        let body = r#"## Description

Some description of the issue.

> - **Branch:** `feat/123/some-feature`
> - **Plan:** `docs/plans/2026-06-01-001-some-plan.md` (committed on branch @ abc1234)
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED) — session-id: 550e8400-e29b-41d4-a716-446655440000
"#;
        assert!(is_groomed(body));
    }

    #[test]
    fn test_is_groomed_missing_branch() {
        let body = r#"## Description

> - **Plan:** `docs/plans/2026-06-01-001-some-plan.md` (committed on branch @ abc1234)
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED) — session-id: 550e8400
"#;
        assert!(!is_groomed(body));
    }

    #[test]
    fn test_is_groomed_missing_plan() {
        let body = r#"## Description

> - **Branch:** `feat/123/some-feature`
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED) — session-id: 550e8400
"#;
        assert!(!is_groomed(body));
    }

    #[test]
    fn test_is_groomed_missing_grooming_history() {
        let body = r#"## Description

> - **Branch:** `feat/123/some-feature`
> - **Plan:** `docs/plans/2026-06-01-001-some-plan.md` (committed on branch @ abc1234)
"#;
        assert!(!is_groomed(body));
    }

    #[test]
    fn test_is_groomed_only_first_pass() {
        let body = r#"## Description

> - **Branch:** `feat/123/some-feature`
> - **Plan:** `docs/plans/2026-06-01-001-some-plan.md` (committed on branch @ abc1234)
> - **Grooming history:** first-pass (READY) — session-id: 550e8400
"#;
        assert!(!is_groomed(body), "first-pass only should not be groomed");
    }

    #[test]
    fn test_is_groomed_prose_groomed_not_callout() {
        let body = r#"## Description

This ticket has been GROOMED and is ready.

> - **Branch:** `feat/123/some-feature`
> - **Plan:** `docs/plans/2026-06-01-001-some-plan.md` (committed on branch @ abc1234)
"#;
        assert!(
            !is_groomed(body),
            "prose GROOMED without callout should not match"
        );
    }

    #[test]
    fn test_is_groomed_empty_body() {
        assert!(!is_groomed(""));
    }

    // ── priority_rank tests ──

    #[test]
    fn test_priority_rank_p0() {
        let labels = vec![IssueLabel {
            name: "p0".to_string(),
        }];
        assert_eq!(priority_rank(&labels), 4);
    }

    #[test]
    fn test_priority_rank_p1() {
        let labels = vec![IssueLabel {
            name: "p1".to_string(),
        }];
        assert_eq!(priority_rank(&labels), 3);
    }

    #[test]
    fn test_priority_rank_p2() {
        let labels = vec![IssueLabel {
            name: "p2".to_string(),
        }];
        assert_eq!(priority_rank(&labels), 2);
    }

    #[test]
    fn test_priority_rank_p3() {
        let labels = vec![IssueLabel {
            name: "p3".to_string(),
        }];
        assert_eq!(priority_rank(&labels), 1);
    }

    #[test]
    fn test_priority_rank_unlabelled() {
        let labels = vec![IssueLabel {
            name: "bug".to_string(),
        }];
        assert_eq!(priority_rank(&labels), 0);
    }

    #[test]
    fn test_priority_rank_empty() {
        assert_eq!(priority_rank(&[]), 0);
    }

    #[test]
    fn test_priority_rank_multiple_labels() {
        let labels = vec![
            IssueLabel {
                name: "bug".to_string(),
            },
            IssueLabel {
                name: "p1".to_string(),
            },
            IssueLabel {
                name: "enhancement".to_string(),
            },
        ];
        assert_eq!(priority_rank(&labels), 3);
    }

    // ── select_best_candidate tests ──

    fn make_issue(number: u64, body: &str, label_names: &[&str], updated_at: &str) -> Issue {
        Issue {
            number,
            body: body.to_string(),
            labels: label_names
                .iter()
                .map(|n| IssueLabel {
                    name: n.to_string(),
                })
                .collect(),
            updated_at: updated_at.to_string(),
        }
    }

    const GROOMED_BODY: &str = r#"> - **Branch:** `feat/123/x`
> - **Plan:** `docs/plans/2026-06-01-001-x.md` (committed on branch @ abc1234)
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED) — session-id: 550e8400"#;

    const UNGROOMED_BODY: &str = "Just a regular issue description";

    #[test]
    fn test_select_filters_ungroomed() {
        let issues = vec![
            make_issue(1, UNGROOMED_BODY, &["p0"], "2026-06-01T00:00:00Z"),
            make_issue(2, GROOMED_BODY, &["p1"], "2026-06-01T00:00:00Z"),
        ];
        let selected = select_best_candidate(issues).unwrap();
        assert_eq!(selected.number, 2);
    }

    #[test]
    fn test_select_filters_ready_label() {
        let issues = vec![
            make_issue(1, GROOMED_BODY, &["p0", "ready"], "2026-06-01T00:00:00Z"),
            make_issue(2, GROOMED_BODY, &["p1"], "2026-06-01T00:00:00Z"),
        ];
        let selected = select_best_candidate(issues).unwrap();
        assert_eq!(selected.number, 2);
    }

    #[test]
    fn test_select_highest_priority() {
        let issues = vec![
            make_issue(1, GROOMED_BODY, &["p2"], "2026-06-01T00:00:00Z"),
            make_issue(2, GROOMED_BODY, &["p0"], "2026-06-01T00:00:00Z"),
            make_issue(3, GROOMED_BODY, &["p1"], "2026-06-01T00:00:00Z"),
        ];
        let selected = select_best_candidate(issues).unwrap();
        assert_eq!(selected.number, 2, "p0 should be selected");
    }

    #[test]
    fn test_select_same_priority_oldest_first() {
        let issues = vec![
            make_issue(1, GROOMED_BODY, &["p1"], "2026-06-05T00:00:00Z"),
            make_issue(2, GROOMED_BODY, &["p1"], "2026-06-01T00:00:00Z"),
            make_issue(3, GROOMED_BODY, &["p1"], "2026-06-03T00:00:00Z"),
        ];
        let selected = select_best_candidate(issues).unwrap();
        assert_eq!(
            selected.number, 2,
            "oldest updated_at within same priority should win"
        );
    }

    #[test]
    fn test_select_no_candidates() {
        let issues = vec![make_issue(
            1,
            UNGROOMED_BODY,
            &["p0"],
            "2026-06-01T00:00:00Z",
        )];
        assert!(select_best_candidate(issues).is_none());
    }

    #[test]
    fn test_select_empty_list() {
        assert!(select_best_candidate(vec![]).is_none());
    }

    #[test]
    fn test_select_unlabelled_vs_p3() {
        let issues = vec![
            make_issue(1, GROOMED_BODY, &[], "2026-06-01T00:00:00Z"),
            make_issue(2, GROOMED_BODY, &["p3"], "2026-06-01T00:00:00Z"),
        ];
        let selected = select_best_candidate(issues).unwrap();
        assert_eq!(selected.number, 2, "p3 beats unlabelled");
    }
}

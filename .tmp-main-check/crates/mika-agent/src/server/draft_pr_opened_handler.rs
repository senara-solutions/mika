//! Structural handler for `pull_request.opened` (draft) webhook events (mika#1849).
//!
//! When mika-dev/dev-pilot opens a **draft** PR closing an issue (systematically
//! via dispatch-lib's wip-rescue path, mika#1282), the issue's `ready` label is
//! never removed. It lingers on the issue permanently, so loop inventory reads
//! "N ready" when only a fraction are actually dispatchable. This handler
//! intercepts the `opened` webhook **before** the LLM turn and removes `ready`
//! from each closing-issue of a newly-opened draft PR.
//!
//! Companion to the auto-pull Phase 2 reconciler (mika#1824): once a draft PR
//! is open and its `closingIssuesReferences` is populated, Phase 2's Filter 2
//! (`open_pr_closing`) already excludes that issue from re-drive, so the
//! reconciler will not re-add `ready` after this handler removes it. The two
//! mechanisms compose; no coordination flag is needed.
//!
//! ## Invariants
//!
//! - Side-effect-only injector: returns `VerdictAction::Passthrough` on every
//!   path (mirrors `milestone_context_handler`). Never alters the mika-qa review
//!   turn — it mutates GitHub label state as a pure side effect.
//! - Self-selects on the `[GitHub] PR opened:` text prefix; passthrough for
//!   every other event type. Call order is irrelevant — handlers are disjoint
//!   on event_type.
//! - Repo-general: the repo is parsed from the webhook text, so the handler
//!   works for `mika`, `mika-cloud`, `mika-skills`, etc.
//! - Fail-open on every `gh` error (missing token, `pr view` failure, per-issue
//!   `issue view` / `issue edit` failure): log WARN and continue; never crash
//!   the handler or the turn.
//!
//! See mika#1849.

use tracing::{debug, info, warn};

use crate::async_db::AsyncDatabase;
use crate::tools::pr_merge_with_gate::run_gh_subprocess;

use super::verdict_handler::VerdictAction;

/// Prefix on PR-opened webhook messages from the gateway.
const PR_OPENED_PREFIX: &str = "[GitHub] PR opened: ";

/// The `ready` label name — the canonical positive-consent signal for mika-dev
/// autonomous dispatch (mika#841). Case-sensitive, matches `.github/labels.yml`.
const READY_LABEL: &str = "ready";

/// Parsed fields from a gateway-formatted `PR opened` event.
pub(crate) struct PrOpenedEvent {
    repo: String,
    number: u64,
}

/// Parse `[GitHub] PR opened: <repo>#<n> — <title> (branch: <b>)` → (repo, number).
///
/// Returns `None` for any non-`PR opened` event text.
pub(crate) fn parse_pr_opened(text: &str) -> Option<PrOpenedEvent> {
    let first_line = text.lines().next()?;
    let after = first_line.strip_prefix(PR_OPENED_PREFIX)?;

    // The token up to the em-dash separator is `<repo>#<number>`.
    let token = after.split(" — ").next()?.trim();
    let (repo, num_str) = token.rsplit_once('#')?;
    if repo.is_empty() {
        return None;
    }
    let number = num_str.trim().parse::<u64>().ok()?;

    Some(PrOpenedEvent {
        repo: repo.to_string(),
        number,
    })
}

/// The closing-issue numbers eligible for cleanup: `[]` unless `is_draft` AND
/// `closing_issue_numbers` is non-empty (AC1, AC5b, AC5c).
pub(crate) fn plan_ready_label_cleanup(is_draft: bool, closing_issue_numbers: &[u64]) -> Vec<u64> {
    if is_draft {
        closing_issue_numbers.to_vec()
    } else {
        Vec::new()
    }
}

/// True when the issue's current label set contains `ready`
/// (case-sensitive, matches the taxonomy).
pub(crate) fn issue_has_ready_label(labels: &[String]) -> bool {
    labels.iter().any(|l| l == READY_LABEL)
}

/// Parse the JSON from `gh pr view <n> --json isDraft,closingIssuesReferences`
/// into `(is_draft, closing_issue_numbers)`.
fn parse_pr_view_json(json: &str) -> Result<(bool, Vec<u64>), String> {
    let v: serde_json::Value =
        serde_json::from_str(json.trim()).map_err(|e| format!("parse pr view json: {e}"))?;
    let is_draft = v["isDraft"].as_bool().unwrap_or(false);
    let refs = v["closingIssuesReferences"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|r| r["number"].as_u64()).collect())
        .unwrap_or_default();
    Ok((is_draft, refs))
}

/// Parse the JSON from `gh issue view <n> --json labels` into a list of label names.
fn parse_issue_labels_json(json: &str) -> Result<Vec<String>, String> {
    let v: serde_json::Value =
        serde_json::from_str(json.trim()).map_err(|e| format!("parse issue labels json: {e}"))?;
    let labels = v["labels"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|l| l["name"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    Ok(labels)
}

/// Fetch `isDraft` + `closingIssuesReferences` for a PR via one `gh pr view` call.
async fn fetch_draft_and_closing_refs(
    pr_number: u64,
    repo: &str,
    token: &str,
) -> Result<(bool, Vec<u64>), String> {
    let n = pr_number.to_string();
    let args = vec![
        "pr",
        "view",
        &n,
        "--repo",
        repo,
        "--json",
        "isDraft,closingIssuesReferences",
    ];
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        run_gh_subprocess(&args, token),
    )
    .await
    .map_err(|_| "gh pr view timed out after 60s".to_string())??;
    parse_pr_view_json(&output)
}

/// Fetch the current label names on an issue via `gh issue view --json labels`.
async fn fetch_issue_labels(
    issue_number: u64,
    repo: &str,
    token: &str,
) -> Result<Vec<String>, String> {
    let n = issue_number.to_string();
    let args = vec!["issue", "view", &n, "--repo", repo, "--json", "labels"];
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        run_gh_subprocess(&args, token),
    )
    .await
    .map_err(|_| "gh issue view timed out after 60s".to_string())??;
    parse_issue_labels_json(&output)
}

/// Remove the `ready` label from an issue via `gh issue edit --remove-label`.
/// Idempotent on GitHub's side — removing an absent label is a no-op.
async fn remove_ready_label(issue_number: u64, repo: &str, token: &str) -> Result<(), String> {
    let n = issue_number.to_string();
    let args = vec![
        "issue",
        "edit",
        &n,
        "--repo",
        repo,
        "--remove-label",
        READY_LABEL,
    ];
    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        run_gh_subprocess(&args, token),
    )
    .await
    .map_err(|_| "gh issue edit timed out after 60s".to_string())??;
    Ok(())
}

/// Attempt to clean up leftover `ready` labels when a draft PR opens.
///
/// Returns `VerdictAction::Passthrough { enrichment: None }` on every path —
/// this is a side-effect-only injector (mika#1849, D1).
pub(crate) async fn try_handle_draft_pr_opened(
    text: &str,
    db: &AsyncDatabase,
    github_token: Option<&str>,
    session_id: &str,
    trace_id: &str,
) -> VerdictAction {
    // 1. Self-select on the PR-opened event prefix.
    let event = match parse_pr_opened(text) {
        Some(e) => e,
        None => return VerdictAction::Passthrough { enrichment: None },
    };

    // 2. Require a GitHub token — fail-open on absence.
    let token = match github_token {
        Some(t) => t,
        None => {
            warn!(
                event = "draft_pr_ready_cleanup_no_token",
                repo = %event.repo,
                pr = event.number,
                "draft_pr_opened_handler: no GitHub token — cannot clean up ready label"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
    };

    // 3. Fetch isDraft + closingIssuesReferences — fail-open on error.
    let (is_draft, closing_refs) =
        match fetch_draft_and_closing_refs(event.number, &event.repo, token).await {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    event = "draft_pr_ready_cleanup_pr_view_failed",
                    repo = %event.repo,
                    pr = event.number,
                    error = %e,
                    "draft_pr_opened_handler: gh pr view failed — passthrough"
                );
                return VerdictAction::Passthrough { enrichment: None };
            }
        };

    // 4. Decide which closing issues are eligible for cleanup.
    let to_clean = plan_ready_label_cleanup(is_draft, &closing_refs);
    if to_clean.is_empty() {
        debug!(
            repo = %event.repo,
            pr = event.number,
            is_draft = is_draft,
            closing_refs = closing_refs.len(),
            "draft_pr_opened_handler: no-op (non-draft or no closing refs)"
        );
        return VerdictAction::Passthrough { enrichment: None };
    }

    // 5. For each closing ref: read labels, remove `ready` only if present,
    //    emit one structured event per removal. Every gh call is fail-open.
    for issue_number in to_clean {
        let labels = match fetch_issue_labels(issue_number, &event.repo, token).await {
            Ok(l) => l,
            Err(e) => {
                warn!(
                    event = "draft_pr_ready_cleanup_issue_view_failed",
                    repo = %event.repo,
                    pr = event.number,
                    issue = issue_number,
                    error = %e,
                    "draft_pr_opened_handler: gh issue view failed — continuing"
                );
                continue;
            }
        };

        if !issue_has_ready_label(&labels) {
            continue;
        }

        match remove_ready_label(issue_number, &event.repo, token).await {
            Ok(()) => {
                info!(
                    event = "ready_label_cleaned_up_on_draft_open",
                    repo = %event.repo,
                    pr = event.number,
                    issue = issue_number,
                    "draft_pr_opened_handler: removed ready label from closing issue"
                );

                // Operator-visible audit record (fire-and-forget).
                if let Err(e) = db
                    .log_audit_event(
                        session_id,
                        "draft_pr_ready_label_cleaned",
                        &format!("issue:{}#{}", event.repo, issue_number),
                        None,
                        Some("ready_removed"),
                        Some(&format!(
                            "repo={} pr={} issue={}",
                            event.repo, event.number, issue_number
                        )),
                        Some(trace_id),
                    )
                    .await
                {
                    warn!(
                        event = "draft_pr_ready_cleanup_audit_failed",
                        repo = %event.repo,
                        issue = issue_number,
                        error = %e,
                        "draft_pr_opened_handler: failed to write audit event (non-fatal)"
                    );
                }
            }
            Err(e) => {
                warn!(
                    event = "draft_pr_ready_cleanup_remove_failed",
                    repo = %event.repo,
                    pr = event.number,
                    issue = issue_number,
                    error = %e,
                    "draft_pr_opened_handler: gh issue edit --remove-label failed — continuing"
                );
                continue;
            }
        }
    }

    VerdictAction::Passthrough { enrichment: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- parse_pr_opened tests --

    #[test]
    fn parse_pr_opened_owner_repo_form() {
        let text = "[GitHub] PR opened: senara-solutions/mika#1849 — fix: cleanup (branch: fix/1849)\nhttps://github.com/senara-solutions/mika/pull/1900";
        let event = parse_pr_opened(text).unwrap();
        assert_eq!(event.repo, "senara-solutions/mika");
        assert_eq!(event.number, 1849);
    }

    #[test]
    fn parse_pr_opened_mika_cloud_form() {
        let text =
            "[GitHub] PR opened: senara-solutions/mika-cloud#42 — chart bump (branch: feat/chart)";
        let event = parse_pr_opened(text).unwrap();
        assert_eq!(event.repo, "senara-solutions/mika-cloud");
        assert_eq!(event.number, 42);
    }

    #[test]
    fn parse_pr_opened_rejects_pr_closed() {
        let text = "[GitHub] PR closed: senara-solutions/mika#1849 — fix (branch: fix/1849)";
        assert!(parse_pr_opened(text).is_none());
    }

    #[test]
    fn parse_pr_opened_rejects_non_pr_text() {
        let text = "[GitHub] Issue labeled ready on senara-solutions/mika#1849 — fix";
        assert!(parse_pr_opened(text).is_none());
    }

    #[test]
    fn parse_pr_opened_rejects_empty() {
        assert!(parse_pr_opened("").is_none());
    }

    #[test]
    fn parse_pr_opened_rejects_unparsable_number() {
        let text = "[GitHub] PR opened: senara-solutions/mika#notanumber — title (branch: b)";
        assert!(parse_pr_opened(text).is_none());
    }

    // -- plan_ready_label_cleanup tests (AC5 a/b/c) --

    #[test]
    fn plan_cleanup_draft_with_refs_returns_refs() {
        // AC5a: draft PR opens with closing-refs → label removal eligible.
        assert_eq!(
            plan_ready_label_cleanup(true, &[1849, 1850]),
            vec![1849, 1850]
        );
    }

    #[test]
    fn plan_cleanup_non_draft_is_noop() {
        // AC5b: non-draft PR opens → no-op.
        assert!(plan_ready_label_cleanup(false, &[1849]).is_empty());
    }

    #[test]
    fn plan_cleanup_draft_no_refs_is_noop() {
        // AC5c: draft with no closing-refs → no-op.
        assert!(plan_ready_label_cleanup(true, &[]).is_empty());
    }

    // -- issue_has_ready_label tests --

    #[test]
    fn issue_has_ready_label_present() {
        let labels = vec!["bug".to_string(), "ready".to_string(), "p1".to_string()];
        assert!(issue_has_ready_label(&labels));
    }

    #[test]
    fn issue_has_ready_label_absent() {
        let labels = vec!["bug".to_string(), "p1".to_string()];
        assert!(!issue_has_ready_label(&labels));
    }

    #[test]
    fn issue_has_ready_label_case_sensitive() {
        // Taxonomy uses lowercase `ready`; `Ready` is not a match.
        let labels = vec!["Ready".to_string()];
        assert!(!issue_has_ready_label(&labels));
    }

    // -- parse_pr_view_json tests --

    #[test]
    fn parse_pr_view_json_draft_with_refs() {
        let json = r#"{
            "isDraft": true,
            "closingIssuesReferences": [
                {"number": 1849, "title": "fix"},
                {"number": 1850, "title": "other"}
            ]
        }"#;
        let (is_draft, refs) = parse_pr_view_json(json).unwrap();
        assert!(is_draft);
        assert_eq!(refs, vec![1849, 1850]);
    }

    #[test]
    fn parse_pr_view_json_non_draft_empty_refs() {
        let json = r#"{"isDraft": false, "closingIssuesReferences": []}"#;
        let (is_draft, refs) = parse_pr_view_json(json).unwrap();
        assert!(!is_draft);
        assert!(refs.is_empty());
    }

    #[test]
    fn parse_pr_view_json_missing_fields_defaults() {
        let json = r#"{}"#;
        let (is_draft, refs) = parse_pr_view_json(json).unwrap();
        assert!(!is_draft);
        assert!(refs.is_empty());
    }

    #[test]
    fn parse_pr_view_json_invalid_errors() {
        // AC5d seam: malformed JSON is a recoverable error, not a panic.
        assert!(parse_pr_view_json("not json").is_err());
    }

    // -- parse_issue_labels_json tests --

    #[test]
    fn parse_issue_labels_json_extracts_names() {
        let json = r#"{"labels": [{"name": "ready"}, {"name": "bug"}]}"#;
        let labels = parse_issue_labels_json(json).unwrap();
        assert_eq!(labels, vec!["ready".to_string(), "bug".to_string()]);
    }

    #[test]
    fn parse_issue_labels_json_empty() {
        let json = r#"{"labels": []}"#;
        assert!(parse_issue_labels_json(json).unwrap().is_empty());
    }

    #[test]
    fn parse_issue_labels_json_invalid_errors() {
        assert!(parse_issue_labels_json("{bad").is_err());
    }
}

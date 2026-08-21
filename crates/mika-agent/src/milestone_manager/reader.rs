//! Reader — composes milestone state from `gh` CLI output.
//!
//! **LECTURE SEULE.** Every subprocess spawned here is a read-only `gh`
//! subcommand (`view`, `list`). No `gh api PATCH/POST/DELETE`, no `gh issue
//! edit`, no `gh pr merge`. The `no_dispatch_test` in the module root
//! enforces this structurally.
//!
//! **Wrapper-only INV-2.** The subprocess shape mirrors
//! `crate::auto_pull::gh_list_open_issues` verbatim (env `GH_TOKEN`,
//! `stdin::null`, `kill_on_drop(true)`). No new GitHub client dep.

use super::types::{
    CiState, IssueState, MilestoneRef, MilestoneState, ProgressCounts, RecentActivity, SubIssue,
};
use anyhow::{Result, anyhow};
use serde_json::Value;
use std::collections::HashMap;

/// Reader — owns the `gh` invocations and composes `MilestoneState`.
///
/// `token` is an optional GitHub token. When `Some`, injected as
/// `GH_TOKEN` env var on every subprocess. When `None`, `gh` falls back
/// to the host user's `~/.config/gh/hosts.yml` (dev-mode friendly).
pub struct Reader {
    token: Option<String>,
}

/// Trait for executing `gh` commands — injectable for tests so the pure
/// composition logic can be exercised without live network calls.
#[async_trait::async_trait]
pub trait GhRunner: Send + Sync {
    async fn run(&self, args: &[&str]) -> Result<String>;
}

/// Production runner — spawns `gh` via `tokio::process::Command`.
pub struct ProcessGhRunner {
    token: Option<String>,
}

impl ProcessGhRunner {
    pub fn new(token: Option<String>) -> Self {
        Self { token }
    }
}

#[async_trait::async_trait]
impl GhRunner for ProcessGhRunner {
    async fn run(&self, args: &[&str]) -> Result<String> {
        let mut cmd = tokio::process::Command::new("gh");
        cmd.args(args);
        if let Some(t) = &self.token {
            cmd.env("GH_TOKEN", t);
        }
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd.kill_on_drop(true);

        let output = cmd.output().await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("gh {} failed: {}", args.join(" "), stderr));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

impl Reader {
    pub fn new(token: Option<String>) -> Self {
        Self { token }
    }

    /// Compose milestone state from live `gh` calls. Delegates the pure
    /// composition logic to `compose_from_gh_outputs` — see that function
    /// for the boundary between subprocess-owning code and pure parsing.
    pub async fn read(&self, milestone_ref: &MilestoneRef) -> Result<MilestoneState> {
        self.read_with_runner(milestone_ref, &ProcessGhRunner::new(self.token.clone()))
            .await
    }

    /// Test-visible variant that accepts an injectable `GhRunner`.
    pub async fn read_with_runner<R: GhRunner>(
        &self,
        milestone_ref: &MilestoneRef,
        runner: &R,
    ) -> Result<MilestoneState> {
        // 1. Milestone metadata.
        let milestone_json = runner
            .run(&[
                "api",
                &format!(
                    "/repos/{}/milestones/{}",
                    milestone_ref.repo, milestone_ref.number
                ),
            ])
            .await?;

        // 2. Sub-issue enumeration (issues attached to this milestone).
        let issues_json = runner
            .run(&[
                "issue",
                "list",
                "--repo",
                &milestone_ref.repo,
                "--milestone",
                &milestone_ref.number.to_string(),
                "--state",
                "all",
                "--json",
                "number,title,state,body,labels,updatedAt",
                "--limit",
                "100",
            ])
            .await?;

        // 3. For each sub-issue, look up the closing PR + CI (bounded).
        //    We do this via a single `gh pr list` scoped by the milestone —
        //    cheaper than N `gh pr view` calls. Fall back to per-issue
        //    lookup for anything missing.
        let pr_json = runner
            .run(&[
                "pr",
                "list",
                "--repo",
                &milestone_ref.repo,
                "--state",
                "all",
                "--search",
                &format!("milestone:{}", milestone_ref.number),
                "--json",
                "number,state,closingIssuesReferences,statusCheckRollup",
                "--limit",
                "100",
            ])
            .await?;

        compose_from_gh_outputs(milestone_ref, &milestone_json, &issues_json, &pr_json)
    }
}

/// Priority ranks from label — pure function, easy to test.
/// Lower is higher priority. `None` = unranked.
///
/// When multiple priority labels are present (rare — usually a labelling
/// mistake), the highest-priority one wins (lowest numeric rank).
pub fn priority_from_labels(labels: &[String]) -> Option<u8> {
    let mut best: Option<u8> = None;
    for l in labels {
        let rank = match l.as_str() {
            "p0-critical" => Some(0),
            "p1-important" => Some(1),
            "agent-core" => Some(2),
            "p2-normal" => Some(3),
            "p3-nice-to-have" => Some(4),
            _ => None,
        };
        if let Some(r) = rank {
            best = Some(match best {
                Some(b) => b.min(r),
                None => r,
            });
        }
    }
    best
}

/// Parse whether the issue body carries a `Plan: docs/plans/` callout.
/// Matches the loose form documented in the milestone-cascade contract.
pub fn plan_callout_present(body: &str) -> bool {
    // Canonical callout shape: `> - **Plan:** docs/plans/...`.
    // Loose form: any occurrence of `docs/plans/` in a callout blockquote.
    for line in body.lines() {
        let t = line.trim_start();
        if t.starts_with("> ") && t.contains("**Plan:**") {
            return true;
        }
        if t.starts_with("> ") && t.contains("docs/plans/") {
            return true;
        }
    }
    false
}

/// Parse whether the issue body carries a `> - **Branch:**` callout.
pub fn branch_callout_present(body: &str) -> bool {
    for line in body.lines() {
        let t = line.trim_start();
        if t.starts_with("> ") && t.contains("**Branch:**") {
            return true;
        }
    }
    false
}

/// Extract `Blocked by #N` blocker references from an issue body.
/// Pure text scan — the GitHub GraphQL `blockedBy` edge is the
/// authoritative source but requires a second call per issue; body-scan
/// is the wrapper-only shortcut for Phase 1.
pub fn extract_blockers(body: &str) -> Vec<u64> {
    let mut out = Vec::new();
    for line in body.lines() {
        let t = line.to_lowercase();
        // Match `blocked by #N` and `blocked-by: #N` forms.
        if !(t.contains("blocked by") || t.contains("blocked-by")) {
            continue;
        }
        // Extract numeric refs after `#`.
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '#' {
                let mut digits = String::new();
                while let Some(&next) = chars.peek() {
                    if next.is_ascii_digit() {
                        digits.push(next);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if let Ok(n) = digits.parse::<u64>()
                    && !out.contains(&n)
                {
                    out.push(n);
                }
            }
        }
    }
    out
}

/// Classify the CI state from a `gh pr view --json statusCheckRollup` array.
/// GitHub returns `SUCCESS`/`FAILURE`/`PENDING`/... — we collapse to the
/// four-variant `CiState` enum for the Assessor.
pub fn classify_ci_state(rollup: &Value) -> CiState {
    let Some(arr) = rollup.as_array() else {
        return CiState::None;
    };
    if arr.is_empty() {
        return CiState::None;
    }
    let mut any_pending = false;
    let mut any_failed = false;
    for check in arr {
        let conclusion = check
            .get("conclusion")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let status = check.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if status.eq_ignore_ascii_case("IN_PROGRESS")
            || status.eq_ignore_ascii_case("QUEUED")
            || status.eq_ignore_ascii_case("PENDING")
        {
            any_pending = true;
        }
        match conclusion {
            "FAILURE" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STALE" => {
                any_failed = true;
            }
            _ => {}
        }
    }
    if any_failed {
        CiState::Failed
    } else if any_pending {
        CiState::Pending
    } else {
        CiState::Success
    }
}

/// Pure composition: given the three JSON blobs, produce a `MilestoneState`.
///
/// Split out from `read()` so tests can exercise the parsing chain with
/// fixture data (no subprocess boundary in the fast path).
pub fn compose_from_gh_outputs(
    milestone_ref: &MilestoneRef,
    milestone_json: &str,
    issues_json: &str,
    pr_json: &str,
) -> Result<MilestoneState> {
    let m: Value = serde_json::from_str(milestone_json)
        .map_err(|e| anyhow!("failed to parse milestone json: {e}"))?;
    let issues: Vec<Value> = serde_json::from_str(issues_json)
        .map_err(|e| anyhow!("failed to parse issues json: {e}"))?;
    let prs: Vec<Value> =
        serde_json::from_str(pr_json).map_err(|e| anyhow!("failed to parse pr json: {e}"))?;

    // Build a lookup: sub-issue number → (pr_number, pr_state, ci_state).
    // GitHub's `closingIssuesReferences` is populated when the PR body
    // says `Closes #N` or the PR is manually linked to the issue.
    let mut pr_by_issue: HashMap<u64, (u64, String, CiState)> = HashMap::new();
    for pr in &prs {
        let pr_number = match pr.get("number").and_then(|v| v.as_u64()) {
            Some(n) => n,
            None => continue,
        };
        let pr_state = pr
            .get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let ci = pr
            .get("statusCheckRollup")
            .map(classify_ci_state)
            .unwrap_or(CiState::None);
        if let Some(refs) = pr.get("closingIssuesReferences").and_then(|v| v.as_array()) {
            for r in refs {
                if let Some(issue_num) = r.get("number").and_then(|v| v.as_u64()) {
                    // First-writer wins to keep deterministic behaviour.
                    pr_by_issue
                        .entry(issue_num)
                        .or_insert_with(|| (pr_number, pr_state.clone(), ci.clone()));
                }
            }
        }
    }

    let mut sub_issues: Vec<SubIssue> = Vec::with_capacity(issues.len());
    let mut recent_activity: Vec<RecentActivity> = Vec::new();
    let mut last_activity_at: Option<String> = None;

    for issue in &issues {
        let number = match issue.get("number").and_then(|v| v.as_u64()) {
            Some(n) => n,
            None => continue,
        };
        let title = issue
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let state_str = issue
            .get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("OPEN")
            .to_uppercase();
        let state = if state_str == "CLOSED" {
            IssueState::Closed
        } else {
            IssueState::Open
        };
        let body = issue
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let labels: Vec<String> = issue
            .get("labels")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|l| l.get("name").and_then(|n| n.as_str()).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let updated_at = issue
            .get("updatedAt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !updated_at.is_empty() {
            match &last_activity_at {
                Some(existing) if existing.as_str() >= updated_at.as_str() => {}
                _ => last_activity_at = Some(updated_at.clone()),
            }
        }

        let plan_present = plan_callout_present(&body);
        let branch_present = branch_callout_present(&body);
        let blockers = extract_blockers(&body);
        let priority_rank = priority_from_labels(&labels);
        let (pr_number, pr_state, ci_state) = pr_by_issue
            .get(&number)
            .cloned()
            .map(|(n, s, c)| (Some(n), Some(s), c))
            .unwrap_or((None, None, CiState::None));

        sub_issues.push(SubIssue {
            number,
            title: title.clone(),
            state: state.clone(),
            priority_rank,
            plan_present,
            branch_present,
            pr_number,
            pr_state,
            ci_state,
            blockers,
            updated_at: updated_at.clone(),
            labels,
        });

        // Attribute recent activity by the sub-issue's updated_at (crude
        // but sufficient for Phase 1 — a proper timeline would require an
        // extra `gh api /issues/N/timeline` call per issue).
        if state == IssueState::Closed {
            recent_activity.push(RecentActivity {
                at: updated_at.clone(),
                kind: "sub_issue_closed".to_string(),
                subject: format!("#{number} {title}"),
            });
        }
    }

    // Add PR merge activity from the PR list.
    for pr in &prs {
        if pr.get("state").and_then(|v| v.as_str()).unwrap_or("") == "MERGED" {
            let pr_number = pr.get("number").and_then(|v| v.as_u64()).unwrap_or(0);
            recent_activity.push(RecentActivity {
                at: "".to_string(),
                kind: "pr_merged".to_string(),
                subject: format!("#{pr_number}"),
            });
        }
    }
    // Sort desc, keep 10 most recent.
    recent_activity.sort_by(|a, b| b.at.cmp(&a.at));
    recent_activity.truncate(10);

    let progress = compute_progress(&sub_issues);

    let title = m
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let description = m
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let created_at = m
        .get("created_at")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let due_on = m.get("due_on").and_then(|v| v.as_str()).map(str::to_string);
    let milestone_state_str = m.get("state").and_then(|v| v.as_str()).unwrap_or("open");
    let m_state = if milestone_state_str.eq_ignore_ascii_case("closed") {
        IssueState::Closed
    } else {
        IssueState::Open
    };

    Ok(MilestoneState {
        milestone_ref: milestone_ref.clone(),
        title,
        description,
        state: m_state,
        created_at,
        due_on,
        last_activity_at,
        sub_issues,
        progress,
        recent_activity,
        executor_healthy: None,
    })
}

/// Compute `ProgressCounts` from sub-issues. Pure function.
pub fn compute_progress(subs: &[SubIssue]) -> ProgressCounts {
    let mut p = ProgressCounts {
        total: subs.len() as u32,
        ..Default::default()
    };
    for s in subs {
        match s.state {
            IssueState::Closed => p.completed += 1,
            IssueState::Open => {
                if s.is_hard_blocked() {
                    p.blocked += 1;
                } else if s.is_in_flight() {
                    p.in_flight += 1;
                } else {
                    p.unstarted += 1;
                }
            }
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_from_labels_p0_wins() {
        let labels = vec![
            "p2-normal".to_string(),
            "p0-critical".to_string(),
            "enhancement".to_string(),
        ];
        assert_eq!(priority_from_labels(&labels), Some(0));
    }

    #[test]
    fn priority_from_labels_no_priority() {
        let labels = vec!["enhancement".to_string(), "documentation".to_string()];
        assert_eq!(priority_from_labels(&labels), None);
    }

    #[test]
    fn plan_callout_canonical() {
        let body = "Header\n> - **Plan:** docs/plans/foo.md\n> - **Branch:** feat/x\n";
        assert!(plan_callout_present(body));
    }

    #[test]
    fn plan_callout_loose() {
        let body = "> Plan lives at docs/plans/x.md\n";
        assert!(plan_callout_present(body));
    }

    #[test]
    fn plan_callout_absent() {
        let body = "No plan here.\nSome plain text mentioning docs/plans/x.md outside a callout.\n";
        assert!(!plan_callout_present(body));
    }

    #[test]
    fn branch_callout_present_form() {
        let body = "> - **Branch:** `feat/x`";
        assert!(branch_callout_present(body));
    }

    #[test]
    fn branch_callout_absent_form() {
        assert!(!branch_callout_present("no callout here"));
    }

    #[test]
    fn extract_blockers_simple() {
        let body = "Blocked by #42 and #99";
        assert_eq!(extract_blockers(body), vec![42, 99]);
    }

    #[test]
    fn extract_blockers_deduped() {
        let body = "Blocked by #42\nblocked-by: #42";
        assert_eq!(extract_blockers(body), vec![42]);
    }

    #[test]
    fn extract_blockers_none_when_no_marker() {
        // Bare `#42` without `blocked by` context should NOT match.
        assert!(extract_blockers("See issue #42 for context").is_empty());
    }

    #[test]
    fn classify_ci_all_success() {
        let rollup = serde_json::json!([
            {"conclusion": "SUCCESS", "status": "COMPLETED"},
            {"conclusion": "SUCCESS", "status": "COMPLETED"}
        ]);
        assert_eq!(classify_ci_state(&rollup), CiState::Success);
    }

    #[test]
    fn classify_ci_any_failure() {
        let rollup = serde_json::json!([
            {"conclusion": "SUCCESS", "status": "COMPLETED"},
            {"conclusion": "FAILURE", "status": "COMPLETED"}
        ]);
        assert_eq!(classify_ci_state(&rollup), CiState::Failed);
    }

    #[test]
    fn classify_ci_pending_no_failure() {
        let rollup = serde_json::json!([
            {"conclusion": "SUCCESS", "status": "COMPLETED"},
            {"conclusion": "", "status": "IN_PROGRESS"}
        ]);
        assert_eq!(classify_ci_state(&rollup), CiState::Pending);
    }

    #[test]
    fn classify_ci_empty_none() {
        let rollup = serde_json::json!([]);
        assert_eq!(classify_ci_state(&rollup), CiState::None);
    }

    #[test]
    fn classify_ci_non_array_none() {
        let rollup = serde_json::json!(null);
        assert_eq!(classify_ci_state(&rollup), CiState::None);
    }

    #[test]
    fn compose_end_to_end() {
        let milestone_json = r#"{"title": "LC", "description": "Reconciliation", "state": "open", "created_at": "2026-07-17T00:00:00Z", "due_on": null}"#;
        let issues_json = r#"[
            {"number": 1801, "title": "LC.1 theme.css", "state": "CLOSED", "body": "> - **Plan:** docs/plans/lc1.md", "labels": [{"name":"p1-important"}], "updatedAt": "2026-08-15T00:00:00Z"},
            {"number": 1802, "title": "LC.2 tokens", "state": "OPEN", "body": "> - **Plan:** docs/plans/lc2.md\n> - **Branch:** `feat/lc2`", "labels": [{"name":"p2-normal"}], "updatedAt": "2026-08-20T00:00:00Z"},
            {"number": 1803, "title": "LC.3 blocked", "state": "OPEN", "body": "Blocked by #1802", "labels": [], "updatedAt": "2026-08-10T00:00:00Z"}
        ]"#;
        let pr_json = r#"[
            {"number": 2001, "state": "MERGED", "closingIssuesReferences": [{"number": 1801}], "statusCheckRollup": []},
            {"number": 2002, "state": "OPEN", "closingIssuesReferences": [{"number": 1802}], "statusCheckRollup": [{"conclusion": "", "status": "IN_PROGRESS"}]}
        ]"#;
        let r = MilestoneRef {
            repo: "senara-solutions/mika".into(),
            number: 1799,
        };
        let state = compose_from_gh_outputs(&r, milestone_json, issues_json, pr_json).unwrap();
        assert_eq!(state.title, "LC");
        assert_eq!(state.sub_issues.len(), 3);
        // 1801 closed → completed.
        // 1802 open + PR + plan → in flight.
        // 1803 open + blocker → blocked.
        assert_eq!(state.progress.completed, 1);
        assert_eq!(state.progress.in_flight, 1);
        assert_eq!(state.progress.blocked, 1);
        assert_eq!(state.progress.unstarted, 0);
        assert_eq!(state.progress.total, 3);

        // Verify PR linkage on 1802.
        let lc2 = state.sub_issues.iter().find(|s| s.number == 1802).unwrap();
        assert_eq!(lc2.pr_number, Some(2002));
        assert_eq!(lc2.ci_state, CiState::Pending);
        assert!(lc2.plan_present);
        assert!(lc2.branch_present);

        // Verify blocker on 1803.
        let lc3 = state.sub_issues.iter().find(|s| s.number == 1803).unwrap();
        assert_eq!(lc3.blockers, vec![1802]);
        assert!(lc3.is_hard_blocked());

        // Verify priority_rank derives from labels — protects the emit
        // wiring at compose time (injection-verification anchor).
        let lc1 = state.sub_issues.iter().find(|s| s.number == 1801).unwrap();
        assert_eq!(lc1.priority_rank, Some(1)); // p1-important → 1
        assert_eq!(lc2.priority_rank, Some(3)); // p2-normal → 3
        assert_eq!(lc3.priority_rank, None); // no priority label
    }

    #[test]
    fn compose_end_to_end_mix_closed_open_blocked_locks_avancement() {
        // mika#1933 AC2 lock-in: verify Reader denominator matches
        // GitHub's `open_issues + closed_issues` (avancement, not
        // reste-à-faire). Mix: 1 closed + 2 open + 1 blocked → 4 total.
        let milestone_json = r#"{"title": "RT-005", "description": "planning tokens", "state": "open", "created_at": "2026-08-17T00:00:00Z", "due_on": null}"#;
        let issues_json = r#"[
            {"number": 1889, "title": "RT-005 brick 4", "state": "CLOSED", "body": "> - **Plan:** docs/plans/rt5-b4.md", "labels": [{"name":"p3-nice-to-have"}], "updatedAt": "2026-08-20T00:00:00Z"},
            {"number": 1888, "title": "RT-005 brick 5", "state": "OPEN", "body": "> - **Plan:** docs/plans/rt5-b5.md", "labels": [{"name":"p3-nice-to-have"}], "updatedAt": "2026-08-19T00:00:00Z"},
            {"number": 1887, "title": "RT-005 brick 1", "state": "OPEN", "body": "no plan yet", "labels": [{"name":"p3-nice-to-have"}], "updatedAt": "2026-08-18T00:00:00Z"},
            {"number": 1890, "title": "RT-005 brick 6 (blocked)", "state": "OPEN", "body": "Blocked by #1888", "labels": [], "updatedAt": "2026-08-17T00:00:00Z"}
        ]"#;
        let pr_json = r#"[
            {"number": 1929, "state": "MERGED", "closingIssuesReferences": [{"number": 1889}], "statusCheckRollup": []}
        ]"#;
        let r = MilestoneRef {
            repo: "senara-solutions/mika".into(),
            number: 31,
        };
        let state = compose_from_gh_outputs(&r, milestone_json, issues_json, pr_json).unwrap();
        // Denominator is the full sub-issue count (closed + open), not open-only.
        assert_eq!(state.sub_issues.len(), 4);
        assert_eq!(state.progress.total, 4);
        assert_eq!(state.progress.completed, 1); // #1889 closed
        assert_eq!(state.progress.blocked, 1); // #1890 blocker
        assert_eq!(state.progress.in_flight, 1); // #1888 has plan
        assert_eq!(state.progress.unstarted, 1); // #1887 no plan
        // The closed sub-issue is enumerated with the correct state and PR linkage
        // (the composer's `closingIssuesReferences` reverse index must populate
        // `pr_number` for closed sub-issues too, not just open ones).
        let closed = state
            .sub_issues
            .iter()
            .find(|s| s.number == 1889)
            .expect("closed sub-issue present in enumeration");
        assert_eq!(closed.state, IssueState::Closed);
        assert_eq!(closed.pr_number, Some(1929));
    }

    #[test]
    fn compute_progress_all_completed() {
        let subs = vec![SubIssue {
            number: 1,
            title: "t".into(),
            state: IssueState::Closed,
            priority_rank: None,
            plan_present: false,
            branch_present: false,
            pr_number: None,
            pr_state: None,
            ci_state: CiState::None,
            blockers: vec![],
            updated_at: "".into(),
            labels: vec![],
        }];
        let p = compute_progress(&subs);
        assert_eq!(p.completed, 1);
        assert_eq!(p.total, 1);
    }
}

/// Injection-verified arg-capture guards (mika#1933 AC6, plan § S4).
///
/// **What this guards:** the `gh` invocation contract in `Reader::read_with_runner`.
/// The AC1/AC2 lock-in (closed sub-issues included in the denominator) depends
/// on TWO adjacent-pair arg tokens on the `issue list` call — `"--state", "all"`
/// and `"--limit", "100"` — plus the matching `"--state", "all"` on the `pr list`
/// call. A regression that reverts either token (e.g. someone re-defaulting to
/// `--state open`, which is the ticket body's original hypothesis and the pre-PR#1932
/// state) would silently take the Reader back to reste-à-faire semantics.
///
/// **Why arg-capture, not subprocess spawn:** per plan § S4 F4 architect note —
/// arg-capture guards the failure mode named in AC6 ("sed-inject bug that re-adds
/// `--state open` → tests fail") without external dependency flakiness (network,
/// `gh` availability, rate limits). Full subprocess spawn is a distinct integration
/// concern out of scope for AC6.
#[cfg(test)]
mod injection_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A `GhRunner` that captures every argument vector into a shared vec
    /// and returns pre-canned JSON responses based on the first two args.
    ///
    /// The composer only needs syntactically-valid JSON to run to completion —
    /// the tests do not assert on composed state, only on the captured args.
    struct RecordingGhRunner {
        calls: Arc<Mutex<Vec<Vec<String>>>>,
        milestone_json: String,
        issues_json: String,
        pr_json: String,
    }

    #[async_trait::async_trait]
    impl GhRunner for RecordingGhRunner {
        async fn run(&self, args: &[&str]) -> Result<String> {
            let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            self.calls.lock().unwrap().push(owned.clone());
            // Route by the first arg — matches the three call sites in
            // `Reader::read_with_runner`.
            match owned.first().map(String::as_str) {
                Some("api") => Ok(self.milestone_json.clone()),
                Some("issue") => Ok(self.issues_json.clone()),
                Some("pr") => Ok(self.pr_json.clone()),
                _ => Err(anyhow!(
                    "RecordingGhRunner: unexpected first arg in {:?}",
                    owned
                )),
            }
        }
    }

    fn build_recorder() -> (Arc<Mutex<Vec<Vec<String>>>>, RecordingGhRunner) {
        let calls: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let runner = RecordingGhRunner {
            calls: Arc::clone(&calls),
            // Minimal well-shaped JSON — composer runs to completion without
            // meaningful state, which is fine because these tests assert only
            // on the captured argument vectors.
            milestone_json: r#"{"title": "T", "description": "", "state": "open", "created_at": "2026-08-01T00:00:00Z", "due_on": null}"#.into(),
            issues_json: "[]".into(),
            pr_json: "[]".into(),
        };
        (calls, runner)
    }

    /// Assert `--state <expected>` appears as adjacent pair in the arg vector.
    fn assert_adjacent_pair(call: &[String], flag: &str, value: &str) {
        for pair in call.windows(2) {
            if pair[0] == flag && pair[1] == value {
                return;
            }
        }
        panic!(
            "expected adjacent pair (`{flag}`, `{value}`) in call args, got: {:?}",
            call
        );
    }

    #[tokio::test]
    async fn reader_issue_list_uses_state_all_and_limit_100() {
        let (calls, runner) = build_recorder();
        let mref = MilestoneRef {
            repo: "senara-solutions/mika".into(),
            number: 31,
        };
        Reader::new(None)
            .read_with_runner(&mref, &runner)
            .await
            .expect("read_with_runner should succeed on canned JSON");

        let calls = calls.lock().unwrap();
        let issue_call = calls
            .iter()
            .find(|c| c.first().map(String::as_str) == Some("issue"))
            .expect("issue list call captured");
        // Adjacent-pair assertions — a sed-inject that re-adds `--state open`
        // or drops `--limit 100` would fail here.
        assert_adjacent_pair(issue_call, "--state", "all");
        assert_adjacent_pair(issue_call, "--limit", "100");
        // Sanity: the milestone number is threaded through.
        assert_adjacent_pair(issue_call, "--milestone", "31");
    }

    #[tokio::test]
    async fn reader_pr_list_uses_state_all_and_limit_100() {
        let (calls, runner) = build_recorder();
        let mref = MilestoneRef {
            repo: "senara-solutions/mika".into(),
            number: 31,
        };
        Reader::new(None)
            .read_with_runner(&mref, &runner)
            .await
            .expect("read_with_runner should succeed on canned JSON");

        let calls = calls.lock().unwrap();
        let pr_call = calls
            .iter()
            .find(|c| c.first().map(String::as_str) == Some("pr"))
            .expect("pr list call captured");
        // PR-list guards: closed/merged PRs contribute the `closingIssuesReferences`
        // reverse index that powers the Reporter `Completed` section's PR links.
        // Dropping `--state all` here would hide merged PRs → closed sub-issues
        // would render `closed (no linked PR)` in every case.
        assert_adjacent_pair(pr_call, "--state", "all");
        assert_adjacent_pair(pr_call, "--limit", "100");
    }
}

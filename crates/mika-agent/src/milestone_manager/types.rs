//! Types for the mika-manager Phase 1 milestone-scope coordinator.
//!
//! LECTURE SEULE — no write authority anywhere. All types are read-only
//! composers over `gh` CLI output. See the module docstring in `mod.rs`
//! for the full Phase 1 discipline.

use serde::{Deserialize, Serialize};

/// A GitHub milestone reference, e.g. `senara-solutions/mika#1799`.
///
/// Parsed from the CLI `<repo>#<num>` argument and passed through the
/// Reader/Assessor/Reporter chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MilestoneRef {
    /// `owner/repo` — e.g. `senara-solutions/mika`.
    pub repo: String,
    /// The GitHub milestone number.
    pub number: u64,
}

impl MilestoneRef {
    /// Parse a `<repo>#<num>` string, e.g. `senara-solutions/mika#1799`.
    ///
    /// Rejects strings without exactly one `#`, empty repo, or non-numeric
    /// number. Returns `Err` with a human-readable message.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let (repo, num_str) = raw
            .split_once('#')
            .ok_or_else(|| format!("expected `<owner/repo>#<number>`, got `{raw}`"))?;
        if repo.is_empty() {
            return Err(format!("empty repo in `{raw}`"));
        }
        if !repo.contains('/') {
            return Err(format!("expected `<owner/repo>#<number>`, got `{raw}`"));
        }
        let number = num_str
            .parse::<u64>()
            .map_err(|_| format!("non-numeric milestone number in `{raw}`"))?;
        Ok(Self {
            repo: repo.to_string(),
            number,
        })
    }

    /// Canonical stringification: `<repo>#<num>`.
    pub fn as_display(&self) -> String {
        format!("{}#{}", self.repo, self.number)
    }

    /// A filesystem-safe slug for checkpoint/log files.
    pub fn slug(&self) -> String {
        format!("{}-{}", self.repo.replace('/', "-"), self.number)
    }
}

/// Sub-issue state on GitHub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueState {
    Open,
    Closed,
}

/// Aggregate CI state for a PR (from `statusCheckRollup`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CiState {
    /// No PR yet, or PR has no checks.
    None,
    /// Checks are still running.
    Pending,
    /// All required checks passed.
    Success,
    /// One or more required checks failed / cancelled / timed out.
    Failed,
}

/// A single sub-issue enumerated under the milestone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubIssue {
    pub number: u64,
    pub title: String,
    pub state: IssueState,
    /// A caller-derived priority rank, currently label-driven
    /// (p0-critical > p1-important > p2-normal > p3-nice-to-have; None = unranked).
    pub priority_rank: Option<u8>,
    /// True if the issue body carries a `Plan: docs/plans/` callout.
    pub plan_present: bool,
    /// True if the issue body carries a `> - **Branch:**` callout.
    pub branch_present: bool,
    /// PR number linked via GitHub `closingIssuesReferences` (first hit wins).
    pub pr_number: Option<u64>,
    /// PR state: `open`, `closed`, `merged` — plain string from `gh pr view`.
    pub pr_state: Option<String>,
    pub ci_state: CiState,
    /// Numbers of open blocker issues from the `blockedBy` GraphQL edge.
    pub blockers: Vec<u64>,
    /// ISO 8601 UTC — last update timestamp on this issue.
    pub updated_at: String,
    /// GitHub labels on the sub-issue.
    pub labels: Vec<String>,
}

impl SubIssue {
    /// True when the sub-issue is in flight — open with either a linked PR or a plan.
    pub fn is_in_flight(&self) -> bool {
        self.state == IssueState::Open && (self.pr_number.is_some() || self.plan_present)
    }

    /// True when the sub-issue is blocked (open with unresolved blockers) OR its PR CI failed.
    pub fn is_hard_blocked(&self) -> bool {
        (self.state == IssueState::Open && !self.blockers.is_empty())
            || self.ci_state == CiState::Failed
    }
}

/// Aggregate progress counts over a milestone's sub-issues.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressCounts {
    pub completed: u32,
    pub in_flight: u32,
    pub blocked: u32,
    pub unstarted: u32,
    pub total: u32,
}

/// A recent milestone-level activity event (sub-issue close, PR merge, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentActivity {
    pub at: String,
    pub kind: String,
    pub subject: String,
}

/// The composed milestone state — output of `Reader::read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MilestoneState {
    #[serde(rename = "ref")]
    pub milestone_ref: MilestoneRef,
    pub title: String,
    pub description: String,
    pub state: IssueState,
    pub created_at: String,
    pub due_on: Option<String>,
    /// ISO 8601 UTC — most recent activity timestamp across all sub-issues.
    pub last_activity_at: Option<String>,
    pub sub_issues: Vec<SubIssue>,
    pub progress: ProgressCounts,
    pub recent_activity: Vec<RecentActivity>,
    /// True when `MIKA_MANAGER_HEALTH_URL` was reachable and reported the target entity as fresh.
    pub executor_healthy: Option<bool>,
}

/// Severity classification for an assessment. Drives escalation routing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Normal state — has unblocked in-flight work or open work with no alerts.
    Healthy,
    /// Drift or silent-progress alerts present but forward motion possible.
    Attention,
    /// All in-flight sub-issues hard-blocked (blockers + failed CI) — escalation-worthy.
    Blocked,
}

/// A single Assessor alert. Surfaced in the report only when meaningful.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alert {
    pub kind: AlertKind,
    pub subject: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    /// A PR was merged but the sub-issue is still open (or the inverse).
    SilentProgress,
    /// A plan callout points to a plan file whose content changed post-approval.
    Drift,
    /// Silence threshold (JOURS) exceeded.
    Silence,
    /// A blocker points to a sub-issue that is no longer open — false blocker.
    StaleBlocker,
}

/// A next-action recommendation. Manager RECOMMENDS; operator DECIDES.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recommendation {
    /// The next sub-issue to work — by number, if any.
    pub next_sub_issue: Option<u64>,
    /// Human-readable justification.
    pub rationale: String,
}

/// The composed assessment — output of `Assessor::assess`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assessment {
    pub severity: Severity,
    pub recommendation: Recommendation,
    pub alerts: Vec<Alert>,
    /// Optional cross-cutting concerns from `mika-arch-groom-milestone`.
    /// Phase 1: always empty (hydration deferred to Phase 1.5).
    pub cross_cutting: Vec<String>,
}

/// Outcome of a single `run_manager_cycle` invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycleOutcome {
    pub milestone_ref: MilestoneRef,
    /// True when a report was delivered (either HTTP POST or offline sink write).
    pub delivered: bool,
    /// True when this cycle used the escalation route (Blocked severity).
    pub escalated: bool,
    /// True when a state change was detected vs the previous checkpoint.
    pub state_changed: bool,
    /// True when the heartbeat interval elapsed since last delivery.
    pub heartbeat_fired: bool,
    pub severity: Severity,
    pub generated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_milestone_ref() {
        let r = MilestoneRef::parse("senara-solutions/mika#1799").unwrap();
        assert_eq!(r.repo, "senara-solutions/mika");
        assert_eq!(r.number, 1799);
        assert_eq!(r.as_display(), "senara-solutions/mika#1799");
        assert_eq!(r.slug(), "senara-solutions-mika-1799");
    }

    #[test]
    fn rejects_missing_hash() {
        assert!(MilestoneRef::parse("senara-solutions/mika").is_err());
    }

    #[test]
    fn rejects_missing_slash() {
        assert!(MilestoneRef::parse("mika#42").is_err());
    }

    #[test]
    fn rejects_empty_repo() {
        assert!(MilestoneRef::parse("#42").is_err());
    }

    #[test]
    fn rejects_non_numeric_number() {
        assert!(MilestoneRef::parse("senara-solutions/mika#abc").is_err());
    }

    #[test]
    fn is_in_flight_semantics() {
        let base = SubIssue {
            number: 1,
            title: "t".into(),
            state: IssueState::Open,
            priority_rank: None,
            plan_present: false,
            branch_present: false,
            pr_number: None,
            pr_state: None,
            ci_state: CiState::None,
            blockers: vec![],
            updated_at: "2026-08-21T00:00:00Z".into(),
            labels: vec![],
        };
        // Open + no PR + no plan → not in flight.
        assert!(!base.is_in_flight());
        // Open + PR → in flight.
        let with_pr = SubIssue {
            pr_number: Some(99),
            ..base.clone()
        };
        assert!(with_pr.is_in_flight());
        // Open + plan → in flight.
        let with_plan = SubIssue {
            plan_present: true,
            ..base.clone()
        };
        assert!(with_plan.is_in_flight());
        // Closed → never in flight.
        let closed = SubIssue {
            state: IssueState::Closed,
            plan_present: true,
            pr_number: Some(99),
            ..base
        };
        assert!(!closed.is_in_flight());
    }

    #[test]
    fn is_hard_blocked_semantics() {
        let base = SubIssue {
            number: 1,
            title: "t".into(),
            state: IssueState::Open,
            priority_rank: None,
            plan_present: false,
            branch_present: false,
            pr_number: None,
            pr_state: None,
            ci_state: CiState::None,
            blockers: vec![],
            updated_at: "2026-08-21T00:00:00Z".into(),
            labels: vec![],
        };
        assert!(!base.is_hard_blocked());
        // Blocker on open issue → hard-blocked.
        let with_blocker = SubIssue {
            blockers: vec![42],
            ..base.clone()
        };
        assert!(with_blocker.is_hard_blocked());
        // Blocker on closed issue → NOT hard-blocked (issue is done).
        let closed_with_stale_blocker = SubIssue {
            state: IssueState::Closed,
            blockers: vec![42],
            ..base.clone()
        };
        assert!(!closed_with_stale_blocker.is_hard_blocked());
        // CI failed → hard-blocked regardless of blockers.
        let ci_failed = SubIssue {
            ci_state: CiState::Failed,
            ..base
        };
        assert!(ci_failed.is_hard_blocked());
    }
}

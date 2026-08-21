//! Assessor — pure rules engine that reads `MilestoneState` and produces `Assessment`.
//!
//! LECTURE SEULE. No side effects. Output = recommendation. Vincent / MPC DECIDES.
//!
//! Four rules fire (per brief § 2c):
//! 1. Blockers surface (`AlertKind::StaleBlocker` when blocker points to a closed issue)
//! 2. Drift detection (deferred to Phase 1.5 — plan-content-diff needs git integration)
//! 3. Silent-progress reconciliation (PR merged but sub-issue open, or inverse)
//! 4. Priority-ranking recommendation (next actionable sub-issue)
//!
//! Silence threshold in JOURS drives the `AlertKind::Silence` alert (rule fires when
//! `last_activity_at` is older than the threshold).

use super::types::{
    Alert, AlertKind, Assessment, IssueState, MilestoneState, Recommendation, Severity, SubIssue,
};
use chrono::{DateTime, Utc};

/// Assessor configuration.
#[derive(Debug, Clone)]
pub struct AssessorConfig {
    /// Silence threshold in JOURS (days). Default = 3.
    pub silence_threshold_days: u32,
}

impl Default for AssessorConfig {
    fn default() -> Self {
        Self {
            silence_threshold_days: 3,
        }
    }
}

/// Assessor — applies rules to a `MilestoneState` snapshot.
pub struct Assessor {
    cfg: AssessorConfig,
    /// Injectable clock for tests. Production uses `Utc::now`.
    now: Box<dyn Fn() -> DateTime<Utc> + Send + Sync>,
}

impl Assessor {
    pub fn new(cfg: AssessorConfig) -> Self {
        Self {
            cfg,
            now: Box::new(Utc::now),
        }
    }

    /// Test-only constructor with a fixed clock.
    pub fn with_clock(
        cfg: AssessorConfig,
        now: Box<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    ) -> Self {
        Self { cfg, now }
    }

    pub fn assess(&self, state: &MilestoneState) -> Assessment {
        let mut alerts = Vec::new();
        alerts.extend(detect_stale_blockers(state));
        alerts.extend(detect_silent_progress(state));
        alerts.extend(detect_silence(
            state,
            self.cfg.silence_threshold_days,
            (self.now)(),
        ));

        let recommendation = pick_next_action(state, &alerts);
        let severity = classify_severity(state, &alerts);

        Assessment {
            severity,
            recommendation,
            alerts,
            cross_cutting: Vec::new(),
        }
    }
}

/// Detect blockers that point to a closed sub-issue in this same milestone —
/// i.e., the blocker is stale and can be cleared.
pub fn detect_stale_blockers(state: &MilestoneState) -> Vec<Alert> {
    let mut out = Vec::new();
    let closed_numbers: std::collections::HashSet<u64> = state
        .sub_issues
        .iter()
        .filter(|s| s.state == IssueState::Closed)
        .map(|s| s.number)
        .collect();
    for s in &state.sub_issues {
        if s.state != IssueState::Open {
            continue;
        }
        for b in &s.blockers {
            if closed_numbers.contains(b) {
                out.push(Alert {
                    kind: AlertKind::StaleBlocker,
                    subject: format!("#{}", s.number),
                    detail: format!("blocker #{b} is already closed", b = b),
                });
            }
        }
    }
    out
}

/// Detect silent-progress divergence: PR merged but sub-issue still open.
/// (Inverse — sub-issue closed but PR not merged — is normal for
/// hand-closed issues and not surfaced.)
pub fn detect_silent_progress(state: &MilestoneState) -> Vec<Alert> {
    let mut out = Vec::new();
    for s in &state.sub_issues {
        if s.state == IssueState::Open
            && s.pr_state.as_deref() == Some("merged")
            && let Some(pr) = s.pr_number
        {
            out.push(Alert {
                kind: AlertKind::SilentProgress,
                subject: format!("#{}", s.number),
                detail: format!("PR #{pr} is merged but sub-issue still open — reconcile"),
            });
        }
    }
    out
}

/// Detect silence: no activity across the milestone for longer than the JOURS threshold.
///
/// Compares `MilestoneState::last_activity_at` (ISO 8601 UTC) against `now - threshold_days`.
/// When `last_activity_at` is missing, silence is not asserted (fail-open).
pub fn detect_silence(
    state: &MilestoneState,
    threshold_days: u32,
    now: DateTime<Utc>,
) -> Vec<Alert> {
    let Some(last_str) = &state.last_activity_at else {
        return Vec::new();
    };
    let Ok(last) = DateTime::parse_from_rfc3339(last_str) else {
        return Vec::new();
    };
    let last_utc = last.with_timezone(&Utc);
    let age = now.signed_duration_since(last_utc);
    if age.num_days() > threshold_days as i64 {
        vec![Alert {
            kind: AlertKind::Silence,
            subject: state.milestone_ref.as_display(),
            detail: format!(
                "no activity for {} days (threshold: {threshold_days} days)",
                age.num_days()
            ),
        }]
    } else {
        Vec::new()
    }
}

/// Pick the next actionable sub-issue.
///
/// Ranking (higher priority first):
/// 1. In-flight sub-issues that already have a PR — recommend continuing/reviewing.
/// 2. Open sub-issues with a plan but no PR — recommend implementation.
/// 3. Open unblocked sub-issues without a plan — recommend grooming.
/// 4. If everything is blocked, recommend clearing blockers (report `None`).
///
/// Within each tier: `priority_rank` ascending (p0 < p1 < p2 < p3 < unranked),
/// then issue number ascending.
pub fn pick_next_action(state: &MilestoneState, _alerts: &[Alert]) -> Recommendation {
    // Tier 1: in-flight with a PR.
    let mut tier1: Vec<&SubIssue> = state
        .sub_issues
        .iter()
        .filter(|s| s.state == IssueState::Open && s.pr_number.is_some() && !s.is_hard_blocked())
        .collect();
    tier1.sort_by_key(|s| (s.priority_rank.unwrap_or(u8::MAX), s.number));
    if let Some(s) = tier1.first() {
        return Recommendation {
            next_sub_issue: Some(s.number),
            rationale: format!(
                "#{} has an open PR (#{}) — advance review/merge next",
                s.number,
                s.pr_number.unwrap()
            ),
        };
    }

    // Tier 2: open + plan + no PR.
    let mut tier2: Vec<&SubIssue> = state
        .sub_issues
        .iter()
        .filter(|s| {
            s.state == IssueState::Open
                && s.plan_present
                && s.pr_number.is_none()
                && !s.is_hard_blocked()
        })
        .collect();
    tier2.sort_by_key(|s| (s.priority_rank.unwrap_or(u8::MAX), s.number));
    if let Some(s) = tier2.first() {
        return Recommendation {
            next_sub_issue: Some(s.number),
            rationale: format!(
                "#{} has a plan but no PR — implementation is the next step",
                s.number
            ),
        };
    }

    // Tier 3: open + no plan + unblocked.
    let mut tier3: Vec<&SubIssue> = state
        .sub_issues
        .iter()
        .filter(|s| s.state == IssueState::Open && !s.plan_present && !s.is_hard_blocked())
        .collect();
    tier3.sort_by_key(|s| (s.priority_rank.unwrap_or(u8::MAX), s.number));
    if let Some(s) = tier3.first() {
        return Recommendation {
            next_sub_issue: Some(s.number),
            rationale: format!(
                "#{} is unblocked without a plan — grooming is the next step",
                s.number
            ),
        };
    }

    // Tier 4: everything blocked or milestone complete.
    let open_count = state
        .sub_issues
        .iter()
        .filter(|s| s.state == IssueState::Open)
        .count();
    if open_count == 0 {
        Recommendation {
            next_sub_issue: None,
            rationale: "milestone complete — no open sub-issues remain".to_string(),
        }
    } else {
        Recommendation {
            next_sub_issue: None,
            rationale: "all open sub-issues are hard-blocked — clear blockers before dispatching"
                .to_string(),
        }
    }
}

/// Classify severity from state + alerts.
///
/// - `Blocked` iff there is at least one open sub-issue AND every open sub-issue
///   is hard-blocked (blockers unresolved OR CI failed). "No forward path."
/// - `Attention` iff any alert fires (drift, silent-progress, silence, stale blocker).
/// - `Healthy` otherwise.
pub fn classify_severity(state: &MilestoneState, alerts: &[Alert]) -> Severity {
    let open: Vec<&SubIssue> = state
        .sub_issues
        .iter()
        .filter(|s| s.state == IssueState::Open)
        .collect();
    if !open.is_empty() && open.iter().all(|s| s.is_hard_blocked()) {
        return Severity::Blocked;
    }
    if !alerts.is_empty() {
        return Severity::Attention;
    }
    Severity::Healthy
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::milestone_manager::types::{CiState, MilestoneRef, ProgressCounts, RecentActivity};
    use chrono::TimeZone;

    fn base_issue(number: u64) -> SubIssue {
        SubIssue {
            number,
            title: format!("issue {number}"),
            state: IssueState::Open,
            priority_rank: None,
            plan_present: false,
            branch_present: false,
            pr_number: None,
            pr_state: None,
            ci_state: CiState::None,
            blockers: vec![],
            updated_at: "2026-08-20T00:00:00Z".into(),
            labels: vec![],
        }
    }

    fn base_state(subs: Vec<SubIssue>) -> MilestoneState {
        MilestoneState {
            milestone_ref: MilestoneRef {
                repo: "senara-solutions/mika".into(),
                number: 1799,
            },
            title: "LC".into(),
            description: "".into(),
            state: IssueState::Open,
            created_at: "2026-07-17T00:00:00Z".into(),
            due_on: None,
            last_activity_at: Some("2026-08-20T00:00:00Z".into()),
            sub_issues: subs,
            progress: ProgressCounts::default(),
            recent_activity: vec![RecentActivity {
                at: "2026-08-20T00:00:00Z".into(),
                kind: "sub_issue_closed".into(),
                subject: "#1801".into(),
            }],
            executor_healthy: None,
        }
    }

    #[test]
    fn stale_blocker_fires_when_blocker_closed() {
        let mut open = base_issue(1802);
        open.blockers = vec![1801];
        let mut closed = base_issue(1801);
        closed.state = IssueState::Closed;
        let alerts = detect_stale_blockers(&base_state(vec![closed, open]));
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].kind, AlertKind::StaleBlocker);
    }

    #[test]
    fn stale_blocker_silent_when_blocker_still_open() {
        let mut open = base_issue(1802);
        open.blockers = vec![1801];
        let still_open = base_issue(1801);
        let alerts = detect_stale_blockers(&base_state(vec![still_open, open]));
        assert!(alerts.is_empty());
    }

    #[test]
    fn silent_progress_fires_when_pr_merged_but_issue_open() {
        let mut s = base_issue(1802);
        s.pr_number = Some(2002);
        s.pr_state = Some("merged".into());
        let alerts = detect_silent_progress(&base_state(vec![s]));
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].kind, AlertKind::SilentProgress);
    }

    #[test]
    fn silent_progress_silent_when_pr_open() {
        let mut s = base_issue(1802);
        s.pr_number = Some(2002);
        s.pr_state = Some("open".into());
        let alerts = detect_silent_progress(&base_state(vec![s]));
        assert!(alerts.is_empty());
    }

    #[test]
    fn silence_fires_beyond_threshold() {
        let s = base_issue(1802);
        let mut state = base_state(vec![s]);
        state.last_activity_at = Some("2026-08-10T00:00:00Z".into());
        let now = Utc.with_ymd_and_hms(2026, 8, 20, 0, 0, 0).unwrap();
        let alerts = detect_silence(&state, 3, now);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].kind, AlertKind::Silence);
    }

    #[test]
    fn silence_silent_within_threshold() {
        let s = base_issue(1802);
        let mut state = base_state(vec![s]);
        state.last_activity_at = Some("2026-08-19T00:00:00Z".into());
        let now = Utc.with_ymd_and_hms(2026, 8, 20, 0, 0, 0).unwrap();
        let alerts = detect_silence(&state, 3, now);
        assert!(alerts.is_empty());
    }

    #[test]
    fn silence_absent_when_no_activity_timestamp() {
        let s = base_issue(1802);
        let mut state = base_state(vec![s]);
        state.last_activity_at = None;
        let now = Utc.with_ymd_and_hms(2026, 8, 20, 0, 0, 0).unwrap();
        assert!(detect_silence(&state, 3, now).is_empty());
    }

    #[test]
    fn pick_next_tier1_pr_first() {
        let mut with_pr = base_issue(1802);
        with_pr.pr_number = Some(2002);
        let mut with_plan = base_issue(1803);
        with_plan.plan_present = true;
        let rec = pick_next_action(&base_state(vec![with_plan.clone(), with_pr]), &[]);
        assert_eq!(rec.next_sub_issue, Some(1802));
    }

    #[test]
    fn pick_next_tier2_plan_when_no_pr() {
        let mut with_plan = base_issue(1803);
        with_plan.plan_present = true;
        let plain = base_issue(1804);
        let rec = pick_next_action(&base_state(vec![plain.clone(), with_plan]), &[]);
        assert_eq!(rec.next_sub_issue, Some(1803));
    }

    #[test]
    fn pick_next_tier3_unblocked_when_no_plan() {
        let plain = base_issue(1804);
        let rec = pick_next_action(&base_state(vec![plain]), &[]);
        assert_eq!(rec.next_sub_issue, Some(1804));
    }

    #[test]
    fn pick_next_priority_wins_within_tier() {
        let mut lower = base_issue(1802);
        lower.plan_present = true;
        lower.priority_rank = Some(3);
        let mut higher = base_issue(1803);
        higher.plan_present = true;
        higher.priority_rank = Some(0);
        let rec = pick_next_action(&base_state(vec![lower, higher]), &[]);
        assert_eq!(rec.next_sub_issue, Some(1803));
    }

    #[test]
    fn pick_next_all_blocked_returns_none() {
        let mut blocked = base_issue(1802);
        blocked.blockers = vec![9999];
        let rec = pick_next_action(&base_state(vec![blocked]), &[]);
        assert_eq!(rec.next_sub_issue, None);
        assert!(rec.rationale.contains("hard-blocked"));
    }

    #[test]
    fn pick_next_all_closed_returns_complete() {
        let mut done = base_issue(1802);
        done.state = IssueState::Closed;
        let rec = pick_next_action(&base_state(vec![done]), &[]);
        assert_eq!(rec.next_sub_issue, None);
        assert!(rec.rationale.contains("milestone complete"));
    }

    #[test]
    fn severity_healthy_when_open_with_no_alerts() {
        let plain = base_issue(1802);
        assert_eq!(
            classify_severity(&base_state(vec![plain]), &[]),
            Severity::Healthy
        );
    }

    #[test]
    fn severity_attention_when_any_alert() {
        let plain = base_issue(1802);
        let alerts = vec![Alert {
            kind: AlertKind::Silence,
            subject: "s".into(),
            detail: "d".into(),
        }];
        assert_eq!(
            classify_severity(&base_state(vec![plain]), &alerts),
            Severity::Attention
        );
    }

    #[test]
    fn severity_blocked_when_all_open_are_hard_blocked() {
        let mut b1 = base_issue(1802);
        b1.blockers = vec![9999];
        let mut b2 = base_issue(1803);
        b2.ci_state = CiState::Failed;
        assert_eq!(
            classify_severity(&base_state(vec![b1, b2]), &[]),
            Severity::Blocked
        );
    }

    #[test]
    fn severity_not_blocked_when_no_open_issues() {
        let mut done = base_issue(1802);
        done.state = IssueState::Closed;
        // No open sub-issues, so we're not `Blocked` even though there's no forward work.
        assert_eq!(
            classify_severity(&base_state(vec![done]), &[]),
            Severity::Healthy
        );
    }

    #[test]
    fn full_assess_end_to_end() {
        let mut open_with_blocker = base_issue(1803);
        open_with_blocker.blockers = vec![1801];
        let mut closed = base_issue(1801);
        closed.state = IssueState::Closed;
        let mut in_flight = base_issue(1802);
        in_flight.pr_number = Some(2002);
        let state = base_state(vec![closed, in_flight, open_with_blocker]);
        let a = Assessor::new(AssessorConfig {
            silence_threshold_days: 90,
        })
        .assess(&state);
        // Stale-blocker alert should fire (#1801 is closed).
        assert!(a.alerts.iter().any(|al| al.kind == AlertKind::StaleBlocker));
        // Tier-1 recommendation: #1802.
        assert_eq!(a.recommendation.next_sub_issue, Some(1802));
        // Not all open are hard-blocked (1802 in-flight) → not Blocked.
        assert_eq!(a.severity, Severity::Attention);
        // Cross-cutting always empty in Phase 1.
        assert!(a.cross_cutting.is_empty());
    }
}

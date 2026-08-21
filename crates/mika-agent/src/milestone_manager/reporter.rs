//! Reporter — Markdown formatter for milestone reports.
//!
//! LECTURE SEULE. Pure function. Format matches the brief § 2d template:
//!
//! ```text
//! ## Milestone: <title> [<repo>#<num>]
//!
//! ### État
//! - Progress: N/M sub-issues complete (X% done)
//! - In-flight: <list with PR links + CI state>
//! - Blocked: <list with blockers + who could unblock>
//! - Unstarted: <list with priority ranking>
//!
//! ### Next action (recommendation)
//! <one clear ask, priority-justified, blocker-aware>
//!
//! ### Silent-progress / drift alerts
//! <if any — surface only when > threshold, else silent>
//!
//! ### Cross-cutting concerns (from mika-arch-groom-milestone)
//! <any principle-grounded pushback>
//! ```

use super::types::{Alert, AlertKind, Assessment, CiState, IssueState, MilestoneState, SubIssue};
use std::fmt::Write;

/// Reporter — composes a Markdown report from a `MilestoneState` + `Assessment`.
pub struct Reporter;

impl Reporter {
    pub fn new() -> Self {
        Self
    }

    /// Compose the full report as a single Markdown string.
    pub fn report(&self, state: &MilestoneState, assessment: &Assessment) -> String {
        let mut out = String::new();
        // Header.
        let _ = writeln!(
            out,
            "## Milestone: {} [{}]",
            state.title,
            state.milestone_ref.as_display()
        );
        writeln!(out).unwrap();

        // État section.
        writeln!(out, "### État").unwrap();
        let pct = if state.progress.total > 0 {
            (state.progress.completed * 100) / state.progress.total
        } else {
            0
        };
        let _ = writeln!(
            out,
            "- Progress: {}/{} sub-issues complete ({}% done)",
            state.progress.completed, state.progress.total, pct
        );
        write_in_flight(&mut out, &state.sub_issues, &state.milestone_ref.repo);
        write_blocked(&mut out, &state.sub_issues);
        write_unstarted(&mut out, &state.sub_issues);
        writeln!(out).unwrap();

        // Next action.
        writeln!(out, "### Next action (recommendation)").unwrap();
        match assessment.recommendation.next_sub_issue {
            Some(n) => {
                let _ = writeln!(
                    out,
                    "- Next: #{n} — {rat}",
                    n = n,
                    rat = assessment.recommendation.rationale
                );
            }
            None => {
                let _ = writeln!(out, "- Next: — {}", assessment.recommendation.rationale);
            }
        }
        writeln!(out).unwrap();

        // Silent-progress / drift alerts.
        writeln!(out, "### Silent-progress / drift alerts").unwrap();
        let drift_alerts: Vec<&Alert> = assessment
            .alerts
            .iter()
            .filter(|a| {
                matches!(
                    a.kind,
                    AlertKind::SilentProgress
                        | AlertKind::Drift
                        | AlertKind::Silence
                        | AlertKind::StaleBlocker
                )
            })
            .collect();
        if drift_alerts.is_empty() {
            writeln!(out, "- (none)").unwrap();
        } else {
            for a in drift_alerts {
                let _ = writeln!(
                    out,
                    "- [{}] {} — {}",
                    alert_kind_label(&a.kind),
                    a.subject,
                    a.detail
                );
            }
        }
        writeln!(out).unwrap();

        // Cross-cutting concerns.
        writeln!(
            out,
            "### Cross-cutting concerns (from mika-arch-groom-milestone)"
        )
        .unwrap();
        if assessment.cross_cutting.is_empty() {
            writeln!(out, "- (none — Phase 1: hydration deferred to Phase 1.5)").unwrap();
        } else {
            for c in &assessment.cross_cutting {
                let _ = writeln!(out, "- {c}");
            }
        }

        out
    }
}

impl Default for Reporter {
    fn default() -> Self {
        Self::new()
    }
}

fn write_in_flight(out: &mut String, subs: &[SubIssue], repo: &str) {
    let in_flight: Vec<&SubIssue> = subs
        .iter()
        .filter(|s| s.state == IssueState::Open && s.is_in_flight() && !s.is_hard_blocked())
        .collect();
    if in_flight.is_empty() {
        writeln!(out, "- In-flight: (none)").unwrap();
        return;
    }
    writeln!(out, "- In-flight:").unwrap();
    for s in in_flight {
        match s.pr_number {
            Some(pr) => {
                let _ = writeln!(
                    out,
                    "  - #{n} {title} — PR https://github.com/{repo}/pull/{pr} ({ci})",
                    n = s.number,
                    title = s.title,
                    ci = ci_label(&s.ci_state)
                );
            }
            None => {
                let _ = writeln!(
                    out,
                    "  - #{n} {title} — plan present, no PR yet",
                    n = s.number,
                    title = s.title
                );
            }
        }
    }
}

fn write_blocked(out: &mut String, subs: &[SubIssue]) {
    let blocked: Vec<&SubIssue> = subs
        .iter()
        .filter(|s| s.state == IssueState::Open && s.is_hard_blocked())
        .collect();
    if blocked.is_empty() {
        writeln!(out, "- Blocked: (none)").unwrap();
        return;
    }
    writeln!(out, "- Blocked:").unwrap();
    for s in blocked {
        let blocker_list = if s.blockers.is_empty() {
            "CI failed".to_string()
        } else {
            s.blockers
                .iter()
                .map(|b| format!("#{b}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let _ = writeln!(
            out,
            "  - #{n} {title} — blocked by {blockers}",
            n = s.number,
            title = s.title,
            blockers = blocker_list
        );
    }
}

fn write_unstarted(out: &mut String, subs: &[SubIssue]) {
    let mut unstarted: Vec<&SubIssue> = subs
        .iter()
        .filter(|s| s.state == IssueState::Open && !s.is_in_flight() && !s.is_hard_blocked())
        .collect();
    unstarted.sort_by_key(|s| (s.priority_rank.unwrap_or(u8::MAX), s.number));
    if unstarted.is_empty() {
        writeln!(out, "- Unstarted: (none)").unwrap();
        return;
    }
    writeln!(out, "- Unstarted:").unwrap();
    for s in unstarted {
        let pri = s
            .priority_rank
            .map(|r| format!("p{r}"))
            .unwrap_or_else(|| "unranked".to_string());
        let _ = writeln!(
            out,
            "  - #{n} {title} — {pri}",
            n = s.number,
            title = s.title
        );
    }
}

fn ci_label(state: &CiState) -> &'static str {
    match state {
        CiState::None => "no CI",
        CiState::Pending => "CI pending",
        CiState::Success => "CI green",
        CiState::Failed => "CI failed",
    }
}

fn alert_kind_label(k: &AlertKind) -> &'static str {
    match k {
        AlertKind::SilentProgress => "silent-progress",
        AlertKind::Drift => "drift",
        AlertKind::Silence => "silence",
        AlertKind::StaleBlocker => "stale-blocker",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::milestone_manager::types::{
        MilestoneRef, ProgressCounts, Recommendation, Severity, SubIssue,
    };

    fn base_issue(n: u64) -> SubIssue {
        SubIssue {
            number: n,
            title: format!("issue {n}"),
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

    fn state_with(subs: Vec<SubIssue>, progress: ProgressCounts) -> MilestoneState {
        MilestoneState {
            milestone_ref: MilestoneRef {
                repo: "senara-solutions/mika".into(),
                number: 1799,
            },
            title: "Luminescent Core reconciliation".into(),
            description: "".into(),
            state: IssueState::Open,
            created_at: "".into(),
            due_on: None,
            last_activity_at: None,
            sub_issues: subs,
            progress,
            recent_activity: vec![],
            executor_healthy: None,
        }
    }

    fn empty_assessment() -> Assessment {
        Assessment {
            severity: Severity::Healthy,
            recommendation: Recommendation {
                next_sub_issue: None,
                rationale: "milestone complete — no open sub-issues remain".into(),
            },
            alerts: vec![],
            cross_cutting: vec![],
        }
    }

    #[test]
    fn report_contains_all_required_headings() {
        let s = state_with(vec![], ProgressCounts::default());
        let a = empty_assessment();
        let out = Reporter::new().report(&s, &a);
        assert!(out.contains(
            "## Milestone: Luminescent Core reconciliation [senara-solutions/mika#1799]"
        ));
        assert!(out.contains("### État"));
        assert!(out.contains("### Next action (recommendation)"));
        assert!(out.contains("### Silent-progress / drift alerts"));
        assert!(out.contains("### Cross-cutting concerns (from mika-arch-groom-milestone)"));
    }

    #[test]
    fn report_progress_percentage() {
        let subs = vec![
            {
                let mut s = base_issue(1);
                s.state = IssueState::Closed;
                s
            },
            base_issue(2),
        ];
        let p = ProgressCounts {
            completed: 1,
            total: 2,
            unstarted: 1,
            ..Default::default()
        };
        let out = Reporter::new().report(&state_with(subs, p), &empty_assessment());
        assert!(out.contains("1/2 sub-issues complete (50% done)"));
    }

    #[test]
    fn report_in_flight_pr_link() {
        let mut s = base_issue(1802);
        s.pr_number = Some(2002);
        s.ci_state = CiState::Pending;
        let p = ProgressCounts {
            in_flight: 1,
            total: 1,
            ..Default::default()
        };
        let out = Reporter::new().report(&state_with(vec![s], p), &empty_assessment());
        assert!(out.contains("https://github.com/senara-solutions/mika/pull/2002"));
        assert!(out.contains("CI pending"));
    }

    #[test]
    fn report_blocked_lists_blockers() {
        let mut s = base_issue(1803);
        s.blockers = vec![1801, 1802];
        let p = ProgressCounts {
            blocked: 1,
            total: 1,
            ..Default::default()
        };
        let out = Reporter::new().report(&state_with(vec![s], p), &empty_assessment());
        assert!(out.contains("blocked by #1801, #1802"));
    }

    #[test]
    fn report_unstarted_shows_priority() {
        let mut s = base_issue(1804);
        s.priority_rank = Some(0);
        let p = ProgressCounts {
            unstarted: 1,
            total: 1,
            ..Default::default()
        };
        let out = Reporter::new().report(&state_with(vec![s], p), &empty_assessment());
        assert!(out.contains("issue 1804 — p0"));
    }

    #[test]
    fn report_no_alerts_shows_none() {
        let out = Reporter::new().report(
            &state_with(vec![], ProgressCounts::default()),
            &empty_assessment(),
        );
        // Both the alerts and cross-cutting sections should render "(none)"-shaped.
        assert!(out.contains("### Silent-progress / drift alerts\n- (none)"));
        assert!(out.contains("### Cross-cutting concerns"));
        assert!(out.contains("Phase 1: hydration deferred"));
    }

    #[test]
    fn report_alerts_render_with_labels() {
        let mut a = empty_assessment();
        a.alerts = vec![
            Alert {
                kind: AlertKind::SilentProgress,
                subject: "#1802".into(),
                detail: "PR merged".into(),
            },
            Alert {
                kind: AlertKind::Silence,
                subject: "milestone#1799".into(),
                detail: "5 days".into(),
            },
        ];
        let out = Reporter::new().report(&state_with(vec![], ProgressCounts::default()), &a);
        assert!(out.contains("[silent-progress] #1802 — PR merged"));
        assert!(out.contains("[silence] milestone#1799 — 5 days"));
    }

    #[test]
    fn report_recommendation_with_and_without_next() {
        let mut a = empty_assessment();
        a.recommendation = Recommendation {
            next_sub_issue: Some(1802),
            rationale: "has PR".into(),
        };
        let out = Reporter::new().report(&state_with(vec![], ProgressCounts::default()), &a);
        assert!(out.contains("Next: #1802 — has PR"));

        let mut a2 = empty_assessment();
        a2.recommendation = Recommendation {
            next_sub_issue: None,
            rationale: "all clear".into(),
        };
        let out2 = Reporter::new().report(&state_with(vec![], ProgressCounts::default()), &a2);
        assert!(out2.contains("Next: — all clear"));
    }
}

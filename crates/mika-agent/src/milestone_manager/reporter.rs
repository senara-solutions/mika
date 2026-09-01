//! Reporter — Markdown formatter for milestone reports.
//!
//! LECTURE SEULE. Pure function. Format matches the brief § 2d template:
//!
//! ```text
//! ## Milestone: <title> [<repo>#<num>]
//!
//! ### État
//! - Progress: N/M sub-issues complete (X% done)
//! - Completed: <list of closed sub-issues with PR links (or `(no linked PR)`)>
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
//!
//! **Section ordering is LOAD-BEARING** (mika#1933 BLOCKING F2 — Prime bearing):
//! `Completed` → `In-flight` → `Blocked` → `Unstarted`. Rationale
//! (Prime verbatim from mika#1933): *« l'établi qui contraint un jugement de
//! registre A/B-jamais-C »* — established brick anchors the judgment of the rest.
//!
//! **Operative rule (English gloss expanded per mika#1933 maintainability F2):**
//! the section that appears first constrains how the reader frames what follows.
//! Placing `Completed` first anchors the reader in what has advanced BEFORE
//! showing what remains, which is the *avancement* reading. Reordering to put
//! `In-flight`/`Blocked` first re-anchors the reader on open work, producing
//! the *reste-à-faire* reading — same visual data, opposite frame. That frame
//! shift is the exact bug mika#1933 fixes; a future reorder would silently
//! reintroduce it. Do NOT reorder these four write_* calls in `Reporter::report`
//! without an explicit Prime override captured in a follow-up plan.

use super::types::{
    Alert, AlertKind, Assessment, CiState, IssueState, MilestoneState, Severity, SubIssue,
};
use mika_common::auth_boundary::AuthBoundaryError;
use std::fmt::Write;

// ---- mika#1949 U4 — auth-boundary failures in the report -------------------

/// One auth-boundary failure, as it will appear in a report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthBoundaryNote {
    pub error: AuthBoundaryError,
    /// How loudly this note reads — and **only** that.
    ///
    /// # This does not re-route the report
    ///
    /// A note at `Blocked` does not set `CycleOutcome::severity`, does not
    /// reach `select_route`, and does not send the report to
    /// `MIKA_MANAGER_ESCALATION_URL`. Said plainly so no reader mistakes the
    /// label for a routing effect.
    ///
    /// The reason is not an oversight: `select_route` authenticates the
    /// escalation route with **the same** `cfg.delivery_token` as the normal
    /// route (`cadence.rs`). When the note says that credential is dead,
    /// re-routing on it would send the escalation through the identical closed
    /// door — a louder report delivered to nobody, plus the false impression
    /// that Vincent's direct channel had fired. The escalation that *is*
    /// wired for a persistent auth failure is `emit_auth_alarm`
    /// (mika#2013 volet B), which carries its own ERROR log as the floor.
    ///
    /// Giving a dead credential its own route means giving that route its own
    /// credential; that is a Phase 3 question, not a rendering one.
    pub severity: Severity,
}

/// Counts consecutive failures on the *same* `(token_name, boundary)` pair, so
/// a repeated one escalates.
///
/// # Why repetition is the signal
///
/// One refusal is worth an operator's attention; it is also what a restart
/// mid-rotation looks like. Three in a row on the same credential is not a
/// transient — it is a credential that will not come back on its own, and it
/// is exactly the shape mika#2013 produced sixteen times in one night while
/// reporting nothing.
///
/// # Why the counter does not persist
///
/// It is in-memory, per-process, and reset by any successful cycle. A
/// persisted counter would carry a stale escalation across a restart that had
/// already fixed the token — the failure mode of a watchdog that cannot be
/// told the problem is over.
#[derive(Debug, Default)]
pub struct AuthBoundaryTracker {
    /// The pair last observed and its consecutive count.
    last: Option<(String, u32)>,
}

/// Consecutive occurrences of one pair before the note escalates to `Blocked`.
pub const AUTH_BOUNDARY_ESCALATION_THRESHOLD: u32 = 3;

impl AuthBoundaryTracker {
    /// Observe one failure and produce the note the next report will carry.
    pub fn observe(&mut self, error: AuthBoundaryError) -> AuthBoundaryNote {
        let key = format!("{}@{}", error.token_name, error.boundary_key());
        let count = match self.last.take() {
            Some((seen, n)) if seen == key => n.saturating_add(1),
            // A different pair breaks the run: two boundaries each failing once
            // is two problems, not one escalating problem.
            _ => 1,
        };
        self.last = Some((key, count));
        let severity = if count >= AUTH_BOUNDARY_ESCALATION_THRESHOLD {
            Severity::Blocked
        } else {
            Severity::Attention
        };
        AuthBoundaryNote { error, severity }
    }

    /// A cycle that authenticated closes the window.
    pub fn reset(&mut self) {
        self.last = None;
    }
}

/// Render one note as the line an operator reads.
pub fn render_auth_boundary_note(note: &AuthBoundaryNote) -> String {
    format!(
        "- [{}] Auth failure: {}",
        severity_label(&note.severity),
        note.error
    )
}

fn severity_label(s: &Severity) -> &'static str {
    match s {
        Severity::Healthy => "healthy",
        Severity::Attention => "attention",
        Severity::Blocked => "blocked",
    }
}

/// Reporter — composes a Markdown report from a `MilestoneState` + `Assessment`.
pub struct Reporter;

impl Reporter {
    pub fn new() -> Self {
        Self
    }

    /// Compose the full report as a single Markdown string.
    pub fn report(&self, state: &MilestoneState, assessment: &Assessment) -> String {
        self.report_with_auth_notes(state, assessment, &[])
    }

    /// As [`Reporter::report`], plus any auth-boundary failures observed since
    /// the last report (mika#1949 U4). An empty slice renders identically to
    /// `report`.
    pub fn report_with_auth_notes(
        &self,
        state: &MilestoneState,
        assessment: &Assessment,
        auth_notes: &[AuthBoundaryNote],
    ) -> String {
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
        // Section ordering: Completed → In-flight → Blocked → Unstarted.
        // See module docstring for the load-bearing rationale (mika#1933 F2, Prime bearing).
        write_completed(&mut out, &state.sub_issues, &state.milestone_ref.repo);
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

        // Dispatch contention (mika#1948). Rendered ONLY when there is
        // contention to report — an always-present "(none)" line would train the
        // reader to skip the section, and this one matters precisely because it
        // is rare. Silence here means the manager was never blocked.
        if !assessment.contention_events.is_empty() {
            writeln!(out).unwrap();
            writeln!(out, "### Dispatch contention").unwrap();
            for e in &assessment.contention_events {
                let source = e
                    .blocking_dispatcher_source
                    .as_deref()
                    .unwrap_or("unknown (pre-v51 row)");
                let _ = writeln!(
                    out,
                    "- `{}` slot held by `{}` (dispatched by {source}) — {}",
                    e.dispatch_class, e.blocking_task_id, e.reason
                );
            }
        }

        // Auth boundary (mika#1949 U4). Same silence rule as Dispatch
        // contention above, and for the same reason: a permanent "(none)" line
        // teaches the reader to skip past it, and this section only ever
        // appears when a credential is actually failing.
        if !auth_notes.is_empty() {
            writeln!(out).unwrap();
            writeln!(out, "### Auth boundary").unwrap();
            for note in auth_notes {
                let _ = writeln!(out, "{}", render_auth_boundary_note(note));
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

/// Emit the `Completed` section (mika#1933 AC3).
///
/// Renders CLOSED sub-issues in a distinct list — the "established brick"
/// signal Prime asked for. Filter matches the semantic complement of
/// `write_in_flight`/`write_blocked`/`write_unstarted` (all three of which
/// gate on `state == IssueState::Open`), so any given sub-issue lands in
/// exactly one section by construction.
///
/// **Empty-case handling (F5 explicit invariant):** when no closed
/// sub-issues exist, emit `- Completed: (none)` — do NOT omit the section.
/// The section is always present so downstream parsers (Assessor consumers,
/// external `mika milestone report` clients) see a uniform shape.
///
/// PR-linkage: reuses `SubIssue::pr_number` populated by
/// `Reader::compose_from_gh_outputs` via the `closingIssuesReferences`
/// reverse index. When a closed sub-issue has no linked PR (hand-closed,
/// or PR not tagged with the milestone), the entry reads `closed (no linked PR)`.
fn write_completed(out: &mut String, subs: &[SubIssue], repo: &str) {
    let completed: Vec<&SubIssue> = subs
        .iter()
        .filter(|s| s.state == IssueState::Closed)
        .collect();
    if completed.is_empty() {
        writeln!(out, "- Completed: (none)").unwrap();
        return;
    }
    writeln!(out, "- Completed:").unwrap();
    for s in completed {
        match s.pr_number {
            Some(pr) => {
                let _ = writeln!(
                    out,
                    "  - #{n} {title} — PR https://github.com/{repo}/pull/{pr}",
                    n = s.number,
                    title = s.title
                );
            }
            None => {
                let _ = writeln!(
                    out,
                    "  - #{n} {title} — closed (no linked PR)",
                    n = s.number,
                    title = s.title
                );
            }
        }
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
        ContentionEvent, MilestoneRef, ProgressCounts, Recommendation, SubIssue,
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
            contention_events: vec![],
        }
    }

    // -- mika#1948 AC6: the Dispatch contention section --

    fn contention(source: Option<&str>) -> ContentionEvent {
        ContentionEvent {
            dispatch_class: "implement".into(),
            blocking_task_id: "task-holder".into(),
            blocking_dispatcher_source: source.map(str::to_string),
            reason: "slot held by another dispatcher".into(),
        }
    }

    #[test]
    fn report_renders_dispatch_contention_when_present() {
        let mut a = empty_assessment();
        a.contention_events = vec![contention(Some("mika_dev"))];
        let out = Reporter::new().report(&state_with(vec![], ProgressCounts::default()), &a);
        assert!(
            out.contains("### Dispatch contention"),
            "the section must appear when there is contention to report"
        );
        assert!(
            out.contains("task-holder") && out.contains("mika_dev"),
            "the entry must name the holder AND which dispatcher it was — \
             that is the whole point of the section. Got:\n{out}"
        );
    }

    /// Anti-vacuity twin: with no contention the section must be ABSENT, not
    /// rendered with a "(none)" line. A section that is always there is a
    /// section the reader learns to skip.
    #[test]
    fn report_omits_dispatch_contention_when_empty() {
        let a = empty_assessment();
        let out = Reporter::new().report(&state_with(vec![], ProgressCounts::default()), &a);
        assert!(
            !out.contains("Dispatch contention"),
            "silence when there is nothing to report. Got:\n{out}"
        );
    }

    #[test]
    fn report_contention_names_unknown_source_honestly() {
        let mut a = empty_assessment();
        a.contention_events = vec![contention(None)];
        let out = Reporter::new().report(&state_with(vec![], ProgressCounts::default()), &a);
        assert!(
            out.contains("unknown (pre-v51 row)"),
            "a NULL source must read as unknown, never be defaulted to a \
             dispatcher we did not observe. Got:\n{out}"
        );
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

    // -----------------------------------------------------------------
    // mika#1933 regression tests — AC3 (Completed section) + AC2 lock-in.
    // -----------------------------------------------------------------

    #[test]
    fn report_completed_section_present_when_closed_exists() {
        // Fixture: 1 closed + 2 open (one of which is in-flight).
        let mut closed = base_issue(1801);
        closed.state = IssueState::Closed;
        let mut in_flight = base_issue(1802);
        in_flight.pr_number = Some(2002);
        in_flight.ci_state = CiState::Success;
        let unstarted = base_issue(1803);
        let p = ProgressCounts {
            completed: 1,
            in_flight: 1,
            unstarted: 1,
            total: 3,
            ..Default::default()
        };
        let out = Reporter::new().report(
            &state_with(vec![closed, in_flight, unstarted], p),
            &empty_assessment(),
        );
        // Header is present.
        assert!(out.contains("- Completed:\n  - #1801"));
        // Section ordering (F2 — load-bearing): Completed appears before In-flight.
        // Anchor on `\n- Header:` (mika#1933 adversarial F2) so a future fixture
        // whose sub-issue title happens to contain `"- In-flight:"` (or another
        // section-header substring) can't false-pass or false-fail the assertion.
        let completed_pos = find_section_header(&out, "Completed");
        let in_flight_pos = find_section_header(&out, "In-flight");
        assert!(
            completed_pos < in_flight_pos,
            "Completed section must precede In-flight (mika#1933 F2 Prime bearing)"
        );
    }

    /// Locate a Reporter section header (`- <Name>:`) at line-start, returning
    /// its byte offset. Anchors on `\n- <Name>:` to distinguish from sub-issue
    /// entry lines that could legitimately contain the same substring inside
    /// a title. Prepends a synthetic `\n` so position 0 is coverable.
    ///
    /// mika#1933 adversarial F2: bare `str::find("- Completed:")` would match
    /// any occurrence of the substring anywhere in the output including inside
    /// a sub-issue title. The `\n` prefix pins the match to a line boundary.
    fn find_section_header(out: &str, name: &str) -> usize {
        let anchored = format!("\n- {name}:");
        let augmented = format!("\n{out}");
        augmented
            .find(&anchored)
            .unwrap_or_else(|| panic!("section header `- {name}:` not found in output:\n{out}"))
    }

    #[test]
    fn report_completed_section_shows_pr_link_when_available() {
        let mut closed = base_issue(1801);
        closed.state = IssueState::Closed;
        closed.pr_number = Some(2001);
        let p = ProgressCounts {
            completed: 1,
            total: 1,
            ..Default::default()
        };
        let out = Reporter::new().report(&state_with(vec![closed], p), &empty_assessment());
        assert!(out.contains(
            "- #1801 issue 1801 — PR https://github.com/senara-solutions/mika/pull/2001"
        ));
    }

    #[test]
    fn report_completed_section_shows_no_pr_fallback_when_absent() {
        let mut closed = base_issue(1801);
        closed.state = IssueState::Closed;
        // pr_number remains None — hand-closed sub-issue.
        let p = ProgressCounts {
            completed: 1,
            total: 1,
            ..Default::default()
        };
        let out = Reporter::new().report(&state_with(vec![closed], p), &empty_assessment());
        assert!(out.contains("- #1801 issue 1801 — closed (no linked PR)"));
    }

    #[test]
    fn report_completed_section_absent_message_when_no_closed() {
        // Fixture: all-open, no closed sub-issues at all.
        let s = base_issue(1802);
        let p = ProgressCounts {
            unstarted: 1,
            total: 1,
            ..Default::default()
        };
        let out = Reporter::new().report(&state_with(vec![s], p), &empty_assessment());
        // Section MUST be present — uniform shape for downstream parsers (F5).
        assert!(out.contains("- Completed: (none)"));
    }

    #[test]
    fn report_progress_matches_avancement_semantics() {
        // Fixture: 1 closed + 3 open + 1 blocked → Progress = 1/5 (20%).
        // Prime bearing (AC2 lock-in): Progress reports *avancement* (closed / total),
        // not *reste-à-faire* (open / total).
        let mut closed = base_issue(1801);
        closed.state = IssueState::Closed;
        let in_flight = {
            let mut s = base_issue(1802);
            s.pr_number = Some(2002);
            s
        };
        let with_plan = {
            let mut s = base_issue(1803);
            s.plan_present = true;
            s
        };
        let unstarted = base_issue(1804);
        let mut blocked = base_issue(1805);
        blocked.blockers = vec![1801];
        let p = ProgressCounts {
            completed: 1,
            in_flight: 2, // 1802 (PR) + 1803 (plan)
            blocked: 1,
            unstarted: 1,
            total: 5,
        };
        let out = Reporter::new().report(
            &state_with(vec![closed, in_flight, with_plan, unstarted, blocked], p),
            &empty_assessment(),
        );
        assert!(out.contains("1/5 sub-issues complete (20% done)"));
    }

    #[test]
    fn report_section_ordering_completed_first() {
        // Explicit F2 guard: exercise a fixture where every section renders
        // a non-empty entry, and assert the four write_* calls emit in the
        // load-bearing order Completed → In-flight → Blocked → Unstarted.
        let mut closed = base_issue(1801);
        closed.state = IssueState::Closed;
        let in_flight = {
            let mut s = base_issue(1802);
            s.pr_number = Some(2002);
            s
        };
        let mut blocked = base_issue(1803);
        blocked.blockers = vec![9999];
        let unstarted = base_issue(1804);
        let p = ProgressCounts {
            completed: 1,
            in_flight: 1,
            blocked: 1,
            unstarted: 1,
            total: 4,
        };
        let out = Reporter::new().report(
            &state_with(vec![closed, in_flight, blocked, unstarted], p),
            &empty_assessment(),
        );
        // Use the anchored helper (mika#1933 adversarial F2) so section-header
        // positions can't be shadowed by a substring inside a sub-issue title.
        let c = find_section_header(&out, "Completed");
        let i = find_section_header(&out, "In-flight");
        let b = find_section_header(&out, "Blocked");
        let u = find_section_header(&out, "Unstarted");
        assert!(c < i, "Completed must precede In-flight");
        assert!(i < b, "In-flight must precede Blocked");
        assert!(b < u, "Blocked must precede Unstarted");
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

    // ---- mika#1949 U4 — auth-boundary rendering + escalation ---------------

    use mika_common::auth_boundary::AuthBoundaryKind;

    fn delivery_err(kind: AuthBoundaryKind) -> AuthBoundaryError {
        AuthBoundaryError::new("MIKA_MANAGER_DELIVERY_TOKEN", "manager", "delivery", kind)
    }

    #[test]
    fn one_auth_failure_renders_at_attention() {
        let mut t = AuthBoundaryTracker::default();
        let note = t.observe(delivery_err(AuthBoundaryKind::Rejected));
        assert_eq!(note.severity, Severity::Attention);
        assert_eq!(
            render_auth_boundary_note(&note),
            "- [attention] Auth failure: rejected at manager->delivery boundary (token: MIKA_MANAGER_DELIVERY_TOKEN)"
        );
    }

    #[test]
    fn three_consecutive_on_the_same_pair_escalate_to_blocked() {
        let mut t = AuthBoundaryTracker::default();
        assert_eq!(
            t.observe(delivery_err(AuthBoundaryKind::Rejected)).severity,
            Severity::Attention
        );
        assert_eq!(
            t.observe(delivery_err(AuthBoundaryKind::Rejected)).severity,
            Severity::Attention
        );
        let third = t.observe(delivery_err(AuthBoundaryKind::Rejected));
        assert_eq!(third.severity, Severity::Blocked);
        assert!(render_auth_boundary_note(&third).contains("MIKA_MANAGER_DELIVERY_TOKEN"));
    }

    #[test]
    fn two_failures_on_different_boundaries_do_not_escalate() {
        let mut t = AuthBoundaryTracker::default();
        let a = t.observe(delivery_err(AuthBoundaryKind::Rejected));
        let b = t.observe(AuthBoundaryError::new(
            "MIKA_INTERNAL_TOKEN",
            "gateway",
            "spirit",
            AuthBoundaryKind::Rejected,
        ));
        let c = t.observe(delivery_err(AuthBoundaryKind::Rejected));
        for note in [&a, &b, &c] {
            assert_eq!(
                note.severity,
                Severity::Attention,
                "a run broken by a different pair is not a run"
            );
        }
    }

    #[test]
    fn a_successful_cycle_closes_the_escalation_window() {
        let mut t = AuthBoundaryTracker::default();
        t.observe(delivery_err(AuthBoundaryKind::Rejected));
        t.observe(delivery_err(AuthBoundaryKind::Rejected));
        t.reset();
        assert_eq!(
            t.observe(delivery_err(AuthBoundaryKind::Rejected)).severity,
            Severity::Attention,
            "a cycle that authenticated must not leave a half-built escalation behind"
        );
    }

    #[test]
    fn the_auth_boundary_section_appears_only_when_there_is_one() {
        let state = state_with(vec![base_issue(1)], ProgressCounts::default());
        let assessment = empty_assessment();
        let without = Reporter::new().report(&state, &assessment);
        assert!(!without.contains("### Auth boundary"));

        let mut t = AuthBoundaryTracker::default();
        let notes = vec![t.observe(delivery_err(AuthBoundaryKind::Missing))];
        let with = Reporter::new().report_with_auth_notes(&state, &assessment, &notes);
        assert!(with.contains("### Auth boundary"));
        assert!(with.contains("missing at manager->delivery boundary"));
        // The name, never a value — there is no value to leak, and this is the
        // assertion that says so at the surface an operator actually reads.
        assert!(with.contains("token: MIKA_MANAGER_DELIVERY_TOKEN"));
    }
}

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

use super::pool;
use super::relief;
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

        // Les candidats du milestone sont projetés **une seule fois** : le
        // bassin et la relève doivent juger la même lecture. Deux appels à
        // `milestone_pool_candidates` seraient deux projections du même état —
        // identiques aujourd'hui, et exactement le genre d'écart qu'aucune
        // assertion ne verrait le jour où la projection cesse d'être pure.
        let milestone_candidates = milestone_pool_candidates(state);
        let pool_verdict = pool::assess_pool(
            &state.ready_pool,
            &milestone_candidates,
            &state.outside_candidates,
            &state.plan_sizes,
        );
        // La relève dérive du verdict qu'on vient de poser, pas d'un second
        // jugement du bassin (mika#2700 vol 1, décision 1). `relief::name_relief`
        // est pure et ne refiltre rien.
        let relief = relief::name_relief(
            &pool_verdict,
            &milestone_candidates,
            &state.outside_candidates,
        );

        Assessment {
            severity,
            recommendation,
            alerts,
            cross_cutting: Vec::new(),
            // Phase 1 is read-only, so there is nothing to contend for yet.
            // Populated by the Phase 2 dispatch path (mika#1948).
            contention_events: Vec::new(),
            // Le verdict du bassin `ready` (mika#2699 Phase 2 (a), vols 1 à 3).
            // `assess_pool` reste pure : tout ce que l'assesseur fait ici est
            // de lui passer les trois lectures et de poser le résultat. Toujours
            // `Some` — un cycle qui arrive ici a lu le bassin, et `None` est
            // réservé aux blobs antérieurs au champ.
            pool: Some(pool_verdict),
            // L'assesseur ne voit pas les vols : il ne lit que l'état du
            // milestone. Les verdicts sont posés par la cadence, qui tient la
            // `FlightSource` et la borne du cycle précédent (mika#2698).
            flight_verdicts: Vec::new(),
            // La relève du **même** bassin que celui posé juste au-dessus
            // (mika#2700 Phase 2 (c) vol 1). Toujours `Some` pour la même
            // raison que `pool` : un cycle qui arrive ici a lu le bassin, et
            // `None` est réservé aux blobs antérieurs au champ.
            relief: Some(relief),
        }
    }
}

/// Les `sub_issues` du milestone cible, vues comme candidats au bassin.
///
/// Une projection et rien d'autre : la décision d'admission vit dans
/// [`pool::assess_pool`], qui est pure. Elle est ici plutôt qu'au reader parce
/// que `sub_issues` sert à tout le reste de l'assesseur sous sa forme complète.
///
/// `pub(super)` depuis mika#2699 vol 3 : le reader doit désigner les candidats
/// dont il ira lire le plan, et il doit les désigner **sur la même projection**
/// que celle que l'assesseur jugera au cycle suivant. Une seconde projection
/// écrite au reader serait identique aujourd'hui et muette le jour où celle-ci
/// change — exactement l'écart que le commentaire ci-dessus refuse déjà.
pub(super) fn milestone_pool_candidates(state: &MilestoneState) -> Vec<pool::PoolCandidate> {
    state.sub_issues.iter().map(Into::into).collect()
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
            ready_pool: vec![],
            outside_candidates: vec![],
            outside_read_truncated: None,
            plan_sizes: Default::default(),
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
    fn silence_threshold_unaffected_by_closed_count_mika_1933() {
        // mika#1933 AC4 — silence detection MUST key on `last_activity_at`
        // (a milestone-scope activity signal), NOT on the mix of closed vs
        // open sub-issues. Adding a Completed rendering surface (AC3) must
        // not shift silence semantics.
        //
        // Fixture A: 3 closed (stale updated_at) + 1 open (also stale). The
        // milestone's `last_activity_at` is what drives silence.
        {
            let mut a = base_issue(1);
            a.state = IssueState::Closed;
            let mut b = base_issue(2);
            b.state = IssueState::Closed;
            let mut c = base_issue(3);
            c.state = IssueState::Closed;
            let d = base_issue(4);
            let mut state = base_state(vec![a, b, c, d]);
            state.last_activity_at = Some("2026-08-19T00:00:00Z".into());
            let now = Utc.with_ymd_and_hms(2026, 8, 20, 0, 0, 0).unwrap();
            // Well within the 3-day threshold — no silence regardless of the
            // 3-to-1 closed-to-open ratio.
            assert!(detect_silence(&state, 3, now).is_empty());
        }
        // Fixture B: 3 closed + 1 open, but last_activity_at is old — silence
        // fires irrespective of the closed count. Capture the full alert set.
        let now = Utc.with_ymd_and_hms(2026, 8, 20, 0, 0, 0).unwrap();
        let fixture_b_alerts = {
            let mut a = base_issue(1);
            a.state = IssueState::Closed;
            let mut b = base_issue(2);
            b.state = IssueState::Closed;
            let mut c = base_issue(3);
            c.state = IssueState::Closed;
            let d = base_issue(4);
            let mut state = base_state(vec![a, b, c, d]);
            state.last_activity_at = Some("2026-08-10T00:00:00Z".into());
            let alerts = detect_silence(&state, 3, now);
            assert_eq!(alerts.len(), 1);
            assert_eq!(alerts[0].kind, AlertKind::Silence);
            alerts
        };
        // Fixture C: 0 closed + 4 open with the SAME stale timestamp — the
        // alert set must be BYTE-IDENTICAL to Fixture B's, proving closed
        // count does not swap AC4 semantics. Full struct equality (not just
        // len + kind) closes the false-pass gap named in mika#1933 adversarial
        // F3 — a future regression that made `Alert.subject`/`detail` differ
        // based on the closed-vs-open ratio would slip past a length+kind
        // check but fail the full-equality assertion below.
        {
            let subs: Vec<SubIssue> = (1..=4).map(base_issue).collect();
            let mut state = base_state(subs);
            state.last_activity_at = Some("2026-08-10T00:00:00Z".into());
            let fixture_c_alerts = detect_silence(&state, 3, now);
            assert_eq!(
                fixture_c_alerts, fixture_b_alerts,
                "AC4 invariance: Fixture C (0 closed + 4 open, stale) MUST produce \
                 the same alert set as Fixture B (3 closed + 1 open, same stale ts) — \
                 closed count MUST NOT influence silence-detection output"
            );
        }
    }

    #[test]
    fn assess_pipeline_silence_alerts_unaffected_by_closed_count_mika_1933() {
        // mika#1933 AC4 layer-2 coverage (paired with `silence_threshold_unaffected_by_closed_count_mika_1933`):
        // pin the invariance property at the FULL PIPELINE layer (`Assessor::assess`),
        // not just the internal `detect_silence` helper.
        //
        // The layer-1 test proves `detect_silence` reads only `last_activity_at`;
        // this test proves the invariance survives composition with the other three
        // rules (`detect_stale_blockers`, `detect_silent_progress`, severity/next-action)
        // as they compose into `Assessor::assess`. A future regression that made ANY
        // pipeline layer key its output on the closed-vs-open sub-issue mix (e.g., a
        // change to `classify_severity` that penalized high closed count, or a
        // `detect_silent_progress` that reversed on state) would slip past the
        // layer-1 assertion but be caught here.
        //
        // Guard is scoped to the `Silence` alert kind (AC4's target) —
        // `detect_stale_blockers` and `detect_silent_progress` legitimately DO
        // depend on state, so a full `assessment` equality check would be wrong.
        let now = Utc.with_ymd_and_hms(2026, 8, 20, 0, 0, 0).unwrap();
        let clock: Box<dyn Fn() -> DateTime<Utc> + Send + Sync> = Box::new(move || now);
        let assessor = Assessor::with_clock(AssessorConfig::default(), clock);

        // Fixture B': 3 closed + 1 open, stale milestone activity.
        let fixture_b_prime_silences: Vec<Alert> = {
            let mut a = base_issue(1);
            a.state = IssueState::Closed;
            let mut b = base_issue(2);
            b.state = IssueState::Closed;
            let mut c = base_issue(3);
            c.state = IssueState::Closed;
            let d = base_issue(4);
            let mut state = base_state(vec![a, b, c, d]);
            state.last_activity_at = Some("2026-08-10T00:00:00Z".into());
            assessor
                .assess(&state)
                .alerts
                .into_iter()
                .filter(|a| a.kind == AlertKind::Silence)
                .collect()
        };
        // Fixture C': 0 closed + 4 open, SAME stale timestamp.
        let fixture_c_prime_silences: Vec<Alert> = {
            let subs: Vec<SubIssue> = (1..=4).map(base_issue).collect();
            let mut state = base_state(subs);
            state.last_activity_at = Some("2026-08-10T00:00:00Z".into());
            assessor
                .assess(&state)
                .alerts
                .into_iter()
                .filter(|a| a.kind == AlertKind::Silence)
                .collect()
        };
        assert_eq!(fixture_b_prime_silences.len(), 1);
        assert_eq!(fixture_b_prime_silences[0].kind, AlertKind::Silence);
        assert_eq!(
            fixture_c_prime_silences, fixture_b_prime_silences,
            "AC4 pipeline invariance: `Assessor::assess` MUST produce the same \
             Silence alert set regardless of closed-vs-open sub-issue mix — the \
             invariance survives composition with the other three rules"
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

    /// Cas 14 (mika#2699) — l'assesseur passe les **trois** lectures à
    /// `assess_pool` et pose le verdict, sans rien décider lui-même.
    ///
    /// Le fixture rend chaque oubli distinguable : le bassin contient un ticket
    /// en vol (famine), le milestone un candidat groomé en p1, et le hors
    /// milestone un candidat groomé en p1 lui aussi. Un assesseur qui aurait
    /// oublié `state.ready_pool` donnerait une profondeur nulle ; un qui aurait
    /// oublié `state.sub_issues` admettrait le seul candidat du dehors ; un qui
    /// aurait oublié `state.outside_candidates` (le vol 1, littéralement)
    /// admettrait le seul candidat du milestone. C'est pour ça que les trois
    /// sont peuplées, et que les deux candidats partagent la même priorité :
    /// seul le rang de provenance les sépare, donc l'ordre obtenu dit laquelle
    /// des trois lectures a voyagé.
    #[test]
    fn mika2699_lassesseur_pose_le_verdict_du_bassin() {
        let mut candidat = base_issue(1802);
        candidat.plan_present = true;
        candidat.priority_rank = Some(1);
        let mut state = base_state(vec![candidat]);
        state.ready_pool = vec![pool::ReadyTicket {
            number: 2699,
            labels: vec!["ready".into(), "dispatch:loop".into()],
        }];
        state.outside_candidates = vec![pool::PoolCandidate {
            number: 1700,
            state: IssueState::Open,
            priority_rank: Some(1),
            plan_present: true,
            labels: vec!["p1-important".into()],
        }];

        let a = Assessor::new(AssessorConfig {
            silence_threshold_days: 90,
        })
        .assess(&state);

        let verdict = a
            .pool
            .expect("un cycle qui a lu le bassin porte un verdict");
        assert_eq!(
            verdict.depth.ready_total, 1,
            "le bassin lu est pris en compte"
        );
        assert_eq!(verdict.depth.in_flight, 1);
        assert_eq!(verdict.band, pool::PoolBand::Starved);
        assert_eq!(
            verdict.admit,
            vec![
                pool::Admission {
                    number: 1802,
                    origin: pool::CandidateOrigin::Milestone,
                    size: pool::PlanSizeReading::Unread(pool::UnreadCause::NotMeasured),
                },
                pool::Admission {
                    number: 1700,
                    origin: pool::CandidateOrigin::Outside,
                    size: pool::PlanSizeReading::Unread(pool::UnreadCause::NotMeasured),
                },
            ],
            "les deux ensembles sont candidats, le milestone d'abord à priorité égale"
        );
        // Et le verdict reste exactement ce que la fonction pure produit —
        // l'assesseur n'ajoute ni ne retire rien.
        assert_eq!(
            verdict,
            pool::assess_pool(
                &state.ready_pool,
                &state
                    .sub_issues
                    .iter()
                    .map(pool::PoolCandidate::from)
                    .collect::<Vec<_>>(),
                &state.outside_candidates,
                &state.plan_sizes,
            )
        );
    }

    /// Cas 11 (mika#2700) — l'assesseur pose la relève **du bassin qu'il vient
    /// de poser**, pas celle d'un second jugement.
    ///
    /// C'est la seule propriété que ni `relief.rs` ni `pool.rs` ne peuvent
    /// voir : un assesseur qui rappellerait `assess_pool` pour la relève
    /// produirait aujourd'hui le même verdict, et l'écart n'apparaîtrait que le
    /// jour où la projection des candidats cesse d'être pure — sans qu'aucune
    /// assertion de comportement rougisse. L'égalité est donc écrite contre le
    /// verdict **lu dans l'`Assessment`**, pas contre un verdict reconstruit.
    ///
    /// Le fixture rend l'oubli d'une lecture distinguable, comme celui du
    /// cas 14 : un candidat du milestone et un du dehors, tous deux non
    /// groomés, à priorité égale — seul le rang de provenance les sépare, donc
    /// l'ordre obtenu dit laquelle des deux lectures a voyagé jusqu'à la relève.
    #[test]
    fn mika2700_lassesseur_pose_la_releve_du_meme_bassin_quil_a_pose() {
        let mut a_grommer = base_issue(1802);
        a_grommer.plan_present = false;
        a_grommer.priority_rank = Some(1);
        let mut state = base_state(vec![a_grommer]);
        state.outside_candidates = vec![pool::PoolCandidate {
            number: 1700,
            state: IssueState::Open,
            priority_rank: Some(1),
            plan_present: false,
            labels: vec!["p1-important".into()],
        }];

        let a = Assessor::new(AssessorConfig {
            silence_threshold_days: 90,
        })
        .assess(&state);

        let verdict = a
            .pool
            .as_ref()
            .expect("un cycle qui a lu le bassin porte un verdict");
        let releve = a
            .relief
            .as_ref()
            .expect("un cycle qui a nommé la relève la porte");

        assert_eq!(
            releve.next.iter().map(|t| t.number).collect::<Vec<_>>(),
            vec![1802, 1700],
            "les deux lectures nourrissent la relève, le milestone d'abord à \
             priorité égale"
        );
        assert_eq!(
            releve.next[0].priority_rank,
            Some(1),
            "le rang relu depuis les candidats voyage avec le ticket"
        );
        assert_eq!(
            *releve,
            relief::name_relief(
                verdict,
                &state
                    .sub_issues
                    .iter()
                    .map(pool::PoolCandidate::from)
                    .collect::<Vec<_>>(),
                &state.outside_candidates,
            ),
            "la relève est exactement celle que la fonction pure tire du verdict \
             posé — l'assesseur ne juge pas le bassin deux fois"
        );
    }
}

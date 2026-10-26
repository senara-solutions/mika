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

use super::evaluator::{DeathCause, FlightIssue, FlightRecommendation, FlightVerdict};
use super::pool::{CandidateOrigin, POOL_MAX, POOL_MIN, PoolBand};
use super::reader::OUTSIDE_READ_LIMIT;
use super::reader::priority_from_labels;
use super::relief::RELIEF_SIZE;
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

        // Vols terminés (mika#2698, vol 1). Placée juste après « Next action »
        // parce que c'est là que l'opérateur compare : la recommandation du
        // manager pour la suite, et son verdict sur ce qui vient de finir, se
        // lisent ensemble. Même règle de silence que les deux sections
        // conditionnelles en fin de rapport — pas de ligne « (none) »
        // permanente, qui apprendrait au lecteur à sauter la section.
        //
        // MODE OMBRE : cette section est la seule destination du verdict. Rien
        // ici n'écrit sur la forge.
        if !assessment.flight_verdicts.is_empty() {
            writeln!(
                out,
                "### Vols terminés depuis le cycle précédent (mode ombre — aucune écriture)"
            )
            .unwrap();
            for v in &assessment.flight_verdicts {
                let _ = writeln!(out, "{}", render_flight_verdict(v));
            }
            writeln!(out).unwrap();
        }

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

        // Bassin `ready` (mika#2699 Phase 2 (a) vol 1). Conditionnelle comme
        // ses deux voisines, mais pour une autre raison : ici `None` veut dire
        // « le bassin n'a pas été lu » (blob antérieur au champ), et une section
        // qui s'afficherait quand même mentirait sur une mesure qui n'a pas eu
        // lieu. Lue, elle s'affiche toujours — y compris en bande nominale, où
        // la profondeur est précisément ce que l'opérateur vient vérifier.
        //
        // « mode ombre » est dans le TITRE, pas en note de bas de page : la
        // ligne « À admettre » se lit comme un geste fait si rien ne dit le
        // contraire au moment où l'œil tombe dessus.
        if let Some(pool) = &assessment.pool {
            writeln!(out).unwrap();
            writeln!(out, "### Bassin ready (mode ombre — aucune écriture)").unwrap();
            let _ = writeln!(
                out,
                "- Profondeur : {} ready, {} en vol, {} disponible — {} (cible {}–{})",
                pool.depth.ready_total,
                pool.depth.in_flight,
                pool.depth.available,
                band_label(&pool.band),
                POOL_MIN,
                POOL_MAX,
            );
            if !pool.admit.is_empty() {
                let admit = pool
                    .admit
                    .iter()
                    .map(|a| {
                        format!(
                            "#{}{}",
                            a.number,
                            candidate_suffix(state, a.number, a.origin)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(out, "- À admettre : {admit}");
            }
            // Une admission dont la taille n'a pas été **mesurée** se dit
            // (mika#2699 vol 3). La borne par vol ne l'a pas jugée : elle n'a
            // pas de total à comparer. Taire le fait ferait lire « À admettre »
            // comme « borne vérifiée », qui est précisément ce que le point 2
            // du ticket promet — et l'opérateur n'aurait aucun moyen de
            // distinguer un plan dimensionné d'un plan dont personne n'a lu la
            // taille. Chaque cause porte son propre libellé : le geste
            // correctif n'est pas le même pour un plan sans section de taille
            // et pour une branche absente d'`origin`.
            for a in &pool.admit {
                if let Some(motif) = a.size.unmeasured_label() {
                    let _ = writeln!(
                        out,
                        "- Taille non vérifiée : #{}{} — {motif}",
                        a.number,
                        candidate_suffix(state, a.number, a.origin),
                    );
                }
            }
            // Chaque refus est **nommé**, avec son motif (mika#2699 vol 2,
            // point 2 du ticket). Le vol 1 rendait « Écartés : 1 candidat sans
            // plan groomé » : l'opérateur apprenait qu'un filtre avait eu lieu
            // sans apprendre sur quoi agir, alors que le geste correctif porte
            // sur *ce* ticket-là. Un refus par ligne, parce qu'une énumération
            // sur une seule ligne cesse d'être lisible dès le troisième.
            for r in &pool.refusals {
                let _ = writeln!(
                    out,
                    "- Refusé : #{}{} — {}",
                    r.number,
                    candidate_suffix(state, r.number, r.origin),
                    r.reason.label(),
                );
            }
            // La page pleine se dit, elle ne se tait pas. Un classement établi
            // sur une lecture tronquée présenterait le meilleur des cent
            // premiers comme le meilleur du dépôt — le motif de
            // `scripts/pr-origin-report.sh` : refuser plutôt que mal répondre.
            // La condition est sur `Some(true)` seul : `None` veut dire « pas
            // mesuré » et ne justifie aucune ligne.
            if state.outside_read_truncated == Some(true) {
                let _ = writeln!(
                    out,
                    "- Lecture hors milestone tronquée à {OUTSIDE_READ_LIMIT} issues — aucun candidat hors milestone retenu ce cycle",
                );
            }
        }

        // Relève à grommer (mika#2700 Phase 2 (c) vol 1). Derrière la section
        // du bassin, parce que la relève se lit *derrière* le bassin : c'est
        // l'ordre du ticket et l'ordre de la décision — ce qui attend n'a de
        // sens qu'après ce qui entre.
        //
        // Deux conditions, pour deux faits distincts. `relief` à `None` veut
        // dire « la relève n'a pas été nommée » (blob antérieur au champ) : une
        // section affirmerait une mesure qui n'a pas eu lieu. `next` vide veut
        // dire « rien à grommer d'avance » : une section vide inviterait à
        // chercher une cause là où il n'y a rien.
        //
        // « mode ombre — aucun groom déclenché » est dans le TITRE, pas en note
        // de bas de page : une liste de tickets « à grommer » se lit comme un
        // geste fait si rien ne dit le contraire à l'instant où l'œil tombe
        // dessus. Au vol 1, aucun interrupteur n'existe — la mention n'est pas
        // une précaution, c'est le fait.
        if let Some(relief) = &assessment.relief
            && !relief.next.is_empty()
        {
            writeln!(out).unwrap();
            writeln!(
                out,
                "### Relève à grommer (mode ombre — aucun groom déclenché)"
            )
            .unwrap();
            // Le suffixe passe par `candidate_suffix` : priorité par son **nom
            // de label** et mention `hors milestone`, exactement comme le
            // bassin. Deux rendus de la même paire (numéro, provenance) dans un
            // même rapport se contrediraient le jour où l'un change.
            //
            // Le motif est celui du bassin, par `RefusalReason::label()` : un
            // libellé réécrit ici serait une seconde orthographe du même fait.
            for t in &relief.next {
                let _ = writeln!(
                    out,
                    "- #{}{} — à grommer : {}",
                    t.number,
                    candidate_suffix(state, t.number, t.origin),
                    t.motive.label(),
                );
            }
            // Le reste se **compte**, il ne s'énumère pas : au-delà de quatre,
            // l'opérateur ne lit plus une consigne mais un inventaire. Taire ce
            // compte ferait lire les quatre nommés comme la totalité de ce qui
            // attend.
            if relief.beyond > 0 {
                let pluriel = if relief.beyond == 1 { "" } else { "s" };
                let _ = writeln!(
                    out,
                    "- {} autre{pluriel} candidat{pluriel} au-delà des {RELIEF_SIZE} nommés",
                    relief.beyond,
                );
            }
            // La troncature se dit **ici aussi**. Sur `Some(true)`,
            // `outside_candidates` est vide par décision : la relève est alors
            // celle du milestone cible, pas celle du dépôt. Un opérateur qui
            // lit quatre numéros sans cette ligne lirait « les 4 prochains du
            // dépôt ». `Some(false)` et `None` ne justifient aucune ligne —
            // « pas mesuré » n'est pas « mesuré non tronqué ».
            if state.outside_read_truncated == Some(true) {
                let _ = writeln!(
                    out,
                    "- Lecture hors milestone tronquée à {OUTSIDE_READ_LIMIT} issues — relève limitée au milestone cible",
                );
            }
        }

        out
    }
}

/// La bande, dite en français — le `Debug` de [`PoolBand`] n'est pas une
/// surface opérateur.
fn band_label(band: &PoolBand) -> &'static str {
    match band {
        PoolBand::Starved => "famine",
        PoolBand::Nominal => "nominal",
        PoolBand::Saturated => "saturé",
    }
}

/// ` (p1-important, hors milestone)` — la priorité et la provenance d'un
/// candidat, celles qui sont connues, vides si aucune.
///
/// [`Admission`] et [`AdmissionRefusal`] ne portent qu'un numéro et une
/// provenance ; l'ordre d'`admit` encode déjà la priorité. Le suffixe la rend
/// lisible sans que l'opérateur ait à faire confiance à l'ordre, et son absence
/// dit « pas de label de priorité », ce qui est exactement le fait qui met ce
/// ticket en dernier.
///
/// La provenance n'est dite que pour `Outside` : « hors milestone » est
/// l'information, « dans le milestone » est le cas par défaut d'un rapport de
/// milestone et le répéter sur chaque ligne noierait l'exception.
fn candidate_suffix(state: &MilestoneState, number: u64, origin: CandidateOrigin) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(label) = priority_label(state, number) {
        parts.push(label);
    }
    if origin == CandidateOrigin::Outside {
        parts.push("hors milestone".to_string());
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

/// Le **nom du label** de priorité d'un ticket, cherché dans les deux ensembles
/// de candidats.
///
/// Les deux, parce qu'un candidat hors milestone cible n'est pas dans
/// `sub_issues` : le chercher là seulement le rendrait sans priorité dans le
/// rapport, donc impossible à arbitrer, exactement sur les tickets que le vol 2
/// vient d'ajouter.
///
/// C'est le nom du label qui est rendu, pas le rang : `agent-core` vaut le rang
/// 2 (voir [`priority_from_labels`]), donc l'écrire « p2 » serait faux.
fn priority_label(state: &MilestoneState, number: u64) -> Option<String> {
    let (rank, labels) = if let Some(sub) = state.sub_issues.iter().find(|s| s.number == number) {
        (sub.priority_rank?, &sub.labels)
    } else {
        let cand = state
            .outside_candidates
            .iter()
            .find(|c| c.number == number)?;
        (cand.priority_rank?, &cand.labels)
    };
    labels
        .iter()
        .find(|l| priority_from_labels(std::slice::from_ref(l)) == Some(rank))
        .cloned()
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

// ---- mika#2698 vol 1 — une ligne par vol terminé --------------------------

/// Un verdict de vol en une ligne de liste Markdown.
///
/// Forme : `- #<ticket> — <issue> — <tours> — <recommandation>`. Chacun des
/// quatre segments dit explicitement quand il ne sait pas, parce qu'un segment
/// manquant décale la lecture et qu'un `0` par défaut mentirait.
pub fn render_flight_verdict(v: &FlightVerdict) -> String {
    let ticket = match v.issue_number {
        Some(n) => format!("#{n}"),
        None => "(ticket inconnu)".to_string(),
    };
    format!(
        "- {ticket} — {issue} — {turns} — {reco}",
        issue = flight_issue_label(&v.issue),
        turns = flight_turns_label(v),
        reco = flight_recommendation_label(v.recommendation),
    )
}

fn flight_issue_label(issue: &FlightIssue) -> String {
    let pr = |number: &Option<u64>| match number {
        Some(n) => format!("#{n}"),
        None => "numéro inconnu".to_string(),
    };
    match issue {
        FlightIssue::PrOpened { number, .. } => format!("PR ouverte ({})", pr(number)),
        FlightIssue::RescuePr { number, .. } => format!("PR de sauvetage ({})", pr(number)),
        FlightIssue::Dead { cause } => match cause {
            DeathCause::CeilingReached => "mort au plafond (budget_exhausted)".to_string(),
            DeathCause::Refusal => "mort — refus (STRUCTURAL VIOLATION)".to_string(),
            DeathCause::PipelineFailure => "mort — panne de pipeline".to_string(),
            DeathCause::LauncherDead => "mort — lanceur mort".to_string(),
            DeathCause::Escalated => "mort — escalade".to_string(),
            // La famille non classée se lit telle que le producteur l'a écrite.
            DeathCause::Unknown(family) => format!("mort — cause non classée ({family})"),
        },
    }
}

/// Les tours, et ce qu'on sait du plafond.
///
/// `(plafond non instrumenté)` est dit à voix haute quand le vol n'a pas heurté
/// la borne : sans cette mention, « 83 tours » se lirait comme « 83 tours sur un
/// plafond que le manager connaît », ce qui est faux au vol 1. Quand la halte
/// `budget_exhausted` a eu lieu, la mention tombe — l'issue dit déjà que la
/// borne a été atteinte.
fn flight_turns_label(v: &FlightVerdict) -> String {
    match (v.turns_used, v.hit_ceiling) {
        (Some(n), true) => format!("{n} tours"),
        (Some(n), false) => format!("{n} tours (plafond non instrumenté)"),
        (None, true) => "tours non lus".to_string(),
        (None, false) => "tours non lus (plafond non instrumenté)".to_string(),
    }
}

fn flight_recommendation_label(r: FlightRecommendation) -> &'static str {
    match r {
        FlightRecommendation::MergeCandidate => "merge-candidat",
        FlightRecommendation::ManualResumption => "reprise à la main",
        FlightRecommendation::Regroom => "re-groom",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::milestone_manager::pool::{
        PlanSizeReading, PoolCandidate, ReadyTicket, UnreadCause, assess_pool,
        assess_pool_sans_tailles,
    };
    use crate::milestone_manager::relief::name_relief;
    use crate::milestone_manager::types::{
        ContentionEvent, MilestoneRef, ProgressCounts, Recommendation, SubIssue,
    };

    /// Les candidats du milestone tels que l'assesseur les projette. Les tests
    /// du rapporteur passent par ce helper plutôt que par un littéral, pour que
    /// la projection sous test soit la même que celle en production.
    fn milestone_cands(state: &MilestoneState) -> Vec<PoolCandidate> {
        state.sub_issues.iter().map(PoolCandidate::from).collect()
    }

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
            ready_pool: vec![],
            outside_candidates: vec![],
            outside_read_truncated: None,
            plan_sizes: Default::default(),
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
            pool: None,
            flight_verdicts: vec![],
            relief: None,
        }
    }

    // -- mika#2698 vol 1 : la section « Vols terminés », en mode ombre --

    fn verdict(
        issue_number: Option<u64>,
        issue: FlightIssue,
        turns_used: Option<u32>,
        hit_ceiling: bool,
        recommendation: FlightRecommendation,
    ) -> FlightVerdict {
        FlightVerdict {
            task_id: "task-x".into(),
            issue_number,
            issue,
            turns_used,
            hit_ceiling,
            turn_ceiling: None,
            last_commit_turn: None,
            recommendation,
            completed_at: Some("2026-10-13T08:00:00Z".into()),
        }
    }

    fn report_with_verdicts(verdicts: Vec<FlightVerdict>) -> String {
        let mut a = empty_assessment();
        a.flight_verdicts = verdicts;
        Reporter::new().report(&state_with(vec![], ProgressCounts::default()), &a)
    }

    /// Les trois issues de l'AC3, telles qu'elles se lisent dans le rapport.
    #[test]
    fn mika2698_la_section_rend_les_trois_issues() {
        let out = report_with_verdicts(vec![
            verdict(
                Some(2693),
                FlightIssue::PrOpened {
                    number: Some(2696),
                    url: None,
                },
                Some(83),
                false,
                FlightRecommendation::MergeCandidate,
            ),
            verdict(
                Some(2661),
                FlightIssue::Dead {
                    cause: DeathCause::CeilingReached,
                },
                Some(200),
                true,
                FlightRecommendation::Regroom,
            ),
            verdict(
                Some(2655),
                FlightIssue::RescuePr {
                    number: Some(2690),
                    url: None,
                },
                Some(141),
                false,
                FlightRecommendation::ManualResumption,
            ),
        ]);
        assert!(
            out.contains(
                "### Vols terminés depuis le cycle précédent (mode ombre — aucune écriture)"
            ),
            "le titre doit dire le mode ombre. Obtenu :\n{out}"
        );
        assert!(
            out.contains(
                "- #2693 — PR ouverte (#2696) — 83 tours (plafond non instrumenté) — merge-candidat"
            ),
            "Obtenu :\n{out}"
        );
        assert!(
            out.contains("- #2661 — mort au plafond (budget_exhausted) — 200 tours — re-groom"),
            "Obtenu :\n{out}"
        );
        assert!(
            out.contains(
                "- #2655 — PR de sauvetage (#2690) — 141 tours (plafond non instrumenté) — reprise à la main"
            ),
            "Obtenu :\n{out}"
        );
    }

    /// Jumeau anti-vacuité, même raison que `Dispatch contention` : sans vol
    /// terminé la section est **absente**, pas rendue avec une ligne « (none) ».
    /// Et le rapport doit être identique à celui d'avant mika#2698 — c'est ce
    /// qui rend la source non câblée sans effet observable.
    #[test]
    fn mika2698_la_section_est_absente_sans_vol_termine() {
        let out = report_with_verdicts(vec![]);
        assert!(
            !out.contains("Vols terminés"),
            "silence quand il n'y a rien à dire. Obtenu :\n{out}"
        );
    }

    /// La section se lit juste après « Next action » : c'est là que l'opérateur
    /// compare la recommandation de suite et le verdict de ce qui vient de finir.
    #[test]
    fn mika2698_la_section_se_lit_juste_apres_next_action() {
        let out = report_with_verdicts(vec![verdict(
            Some(2693),
            FlightIssue::PrOpened {
                number: Some(2696),
                url: None,
            },
            Some(83),
            false,
            FlightRecommendation::MergeCandidate,
        )]);
        let next = out.find("### Next action").expect("Next action présent");
        let vols = out.find("### Vols terminés").expect("section présente");
        let drift = out.find("### Silent-progress").expect("dérive présente");
        assert!(
            next < vols && vols < drift,
            "ordre attendu : Next action → Vols terminés → dérive. Obtenu :\n{out}"
        );
    }

    /// Aucun segment ne se tait : un verdict dont tout manque dit que tout
    /// manque, plutôt que de rendre une ligne décalée ou un `0` inventé.
    #[test]
    fn mika2698_une_donnee_absente_se_dit_au_lieu_de_se_deviner() {
        let out = report_with_verdicts(vec![verdict(
            None,
            FlightIssue::Dead {
                cause: DeathCause::Unknown("PLAN_COMMITTED".into()),
            },
            None,
            false,
            FlightRecommendation::ManualResumption,
        )]);
        assert!(
            out.contains(
                "- (ticket inconnu) — mort — cause non classée (PLAN_COMMITTED) — \
                 tours non lus (plafond non instrumenté) — reprise à la main"
            ),
            "Obtenu :\n{out}"
        );
        assert!(
            !out.contains("— 0 tours"),
            "une mesure absente ne devient jamais zéro (motif mika#2331). Obtenu :\n{out}"
        );
    }

    /// Les six causes de mort et les trois recommandations ont chacune leur
    /// libellé : un `match` muet rendrait deux causes distinctes identiques à
    /// l'écran.
    #[test]
    fn mika2698_chaque_cause_et_chaque_recommandation_a_son_libelle() {
        let causes = [
            (
                DeathCause::CeilingReached,
                "mort au plafond (budget_exhausted)",
            ),
            (DeathCause::Refusal, "mort — refus (STRUCTURAL VIOLATION)"),
            (DeathCause::PipelineFailure, "mort — panne de pipeline"),
            (DeathCause::LauncherDead, "mort — lanceur mort"),
            (DeathCause::Escalated, "mort — escalade"),
            (
                DeathCause::Unknown("zorglub".into()),
                "mort — cause non classée (zorglub)",
            ),
        ];
        for (cause, expected) in causes {
            assert_eq!(flight_issue_label(&FlightIssue::Dead { cause }), expected);
        }
        let recos = [
            (FlightRecommendation::MergeCandidate, "merge-candidat"),
            (FlightRecommendation::ManualResumption, "reprise à la main"),
            (FlightRecommendation::Regroom, "re-groom"),
        ];
        for (r, expected) in recos {
            assert_eq!(flight_recommendation_label(r), expected);
        }
        assert_eq!(
            flight_issue_label(&FlightIssue::PrOpened {
                number: None,
                url: None
            }),
            "PR ouverte (numéro inconnu)"
        );
        assert_eq!(
            flight_issue_label(&FlightIssue::RescuePr {
                number: None,
                url: None
            }),
            "PR de sauvetage (numéro inconnu)"
        );
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

    // -- mika#2699 Phase 2 (a) vol 1 : la section « Bassin ready » -----------

    /// Cas 15 — la section n'existe que si le bassin a été **lu**.
    ///
    /// `pool: None` ne veut pas dire « bassin vide » : il veut dire « pas de
    /// mesure » (blob antérieur au champ). Une section qui s'afficherait quand
    /// même mentirait sur une mesure qui n'a pas eu lieu.
    #[test]
    fn mika2699_la_section_nexiste_que_si_le_bassin_a_ete_lu() {
        let state = state_with(vec![base_issue(1)], ProgressCounts::default());
        let sans = Reporter::new().report(&state, &empty_assessment());
        assert!(
            !sans.contains("### Bassin ready"),
            "pas de mesure ⇒ pas de section — obtenu :\n{sans}"
        );

        let mut a = empty_assessment();
        a.pool = Some(assess_pool_sans_tailles(&[], &[], &[]));
        let avec = Reporter::new().report(&state, &a);
        assert!(
            avec.contains("### Bassin ready (mode ombre — aucune écriture)"),
            "une mesure, même de bassin vide, se dit — obtenu :\n{avec}"
        );
    }

    /// Cas 16 — « mode ombre » est dans le **titre**, et le rendu nominal ne
    /// recommande rien.
    ///
    /// Le titre porte la mention parce que la ligne « À admettre » se lit comme
    /// un geste fait si rien ne dit le contraire à l'endroit où l'œil tombe. Une
    /// note en fin de section arriverait après la conclusion du lecteur.
    #[test]
    fn mika2699_le_mode_ombre_est_dans_le_titre_et_le_nominal_se_tait() {
        let candidat = {
            let mut c = base_issue(1802);
            c.plan_present = true;
            c.priority_rank = Some(1);
            c
        };
        let state = state_with(vec![candidat], ProgressCounts::default());
        let pool = vec![
            ReadyTicket {
                number: 10,
                labels: vec!["ready".into()],
            },
            ReadyTicket {
                number: 11,
                labels: vec!["ready".into()],
            },
            ReadyTicket {
                number: 12,
                labels: vec!["ready".into()],
            },
        ];
        let mut a = empty_assessment();
        a.pool = Some(assess_pool_sans_tailles(
            &pool,
            &milestone_cands(&state),
            &[],
        ));
        let out = Reporter::new().report(&state, &a);

        let titre = "### Bassin ready (mode ombre — aucune écriture)";
        assert!(out.contains(titre), "titre attendu dans :\n{out}");
        assert!(
            out.contains("3 ready, 0 en vol, 3 disponible — nominal (cible 2–4)"),
            "la profondeur et la cible se disent en bande nominale :\n{out}"
        );
        assert!(
            !out.contains("À admettre"),
            "l'hystérésis est muette entre 2 et 4 :\n{out}"
        );
    }

    /// Cas 17 — le rendu de famine nomme la profondeur, la cible 2–4, les
    /// numéros admis dans l'ordre, et **chaque** refus par son numéro.
    ///
    /// Le fixture est l'état mesuré le 2026-10-15 : 2 `ready`, les deux en vol,
    /// 0 disponible. La priorité rendue est le **nom du label**, pas le rang :
    /// `agent-core` vaut le rang 2, donc l'écrire « p2 » serait faux.
    ///
    /// Le vol 2 a remplacé le compte (« Écartés : 1 candidat sans plan
    /// groomé ») par une ligne par ticket. Un compte dit à l'opérateur qu'il y
    /// a quelque chose à regarder sans dire quoi : il doit alors relire les
    /// candidats à la main, c'est-à-dire refaire le filtre que le manager
    /// vient de faire.
    #[test]
    fn mika2699_le_rendu_de_famine_nomme_profondeur_cible_et_admis() {
        let subs = vec![
            {
                let mut c = base_issue(2711);
                c.plan_present = true;
                c.priority_rank = Some(1);
                c.labels = vec!["p1-important".into()];
                c
            },
            {
                let mut c = base_issue(2708);
                c.plan_present = true;
                c.priority_rank = Some(3);
                c.labels = vec!["p2-normal".into()];
                c
            },
            {
                let mut c = base_issue(2720);
                c.plan_present = false;
                c.priority_rank = Some(0);
                c
            },
        ];
        let state = state_with(subs, ProgressCounts::default());
        let pool = vec![
            ReadyTicket {
                number: 2699,
                labels: vec!["ready".into(), "dispatch:loop".into()],
            },
            ReadyTicket {
                number: 2687,
                labels: vec!["ready".into(), "dispatch:loop".into()],
            },
        ];
        let mut a = empty_assessment();
        a.pool = Some(assess_pool_sans_tailles(
            &pool,
            &milestone_cands(&state),
            &[],
        ));
        let out = Reporter::new().report(&state, &a);

        assert!(
            out.contains("- Profondeur : 2 ready, 2 en vol, 0 disponible — famine (cible 2–4)"),
            "la ligne de profondeur doit dire les trois chiffres et la cible :\n{out}"
        );
        assert!(
            out.contains("- À admettre : #2711 (p1-important), #2708 (p2-normal)"),
            "les admis, dans l'ordre, nommés par leur label de priorité :\n{out}"
        );
        assert!(
            out.contains("- Refusé : #2720 — plan groomé absent"),
            "le refus est nommé par ticket et motivé (mika#2699 vol 2) :\n{out}"
        );
        // Restreint à la section du bassin : #2720 y est nommé — mais comme
        // refusé, jamais sur la ligne des admis.
        let section = out
            .split("### Bassin ready")
            .nth(1)
            .expect("la section du bassin doit être rendue");
        let ligne_admis = section
            .lines()
            .find(|l| l.starts_with("- À admettre"))
            .expect("la famine recommande");
        assert!(
            !ligne_admis.contains("#2720"),
            "un candidat sans plan groomé n'est jamais nommé comme admis :\n{ligne_admis}"
        );
    }

    // -- mika#2699 vol 2 : provenance, refus nommé, troncature ---------------

    /// Cas 18 — la provenance n'est dite que pour le dehors, et la priorité
    /// d'un candidat hors milestone se lit quand même.
    ///
    /// Deux choses se jouent ici. **Le suffixe `hors milestone`** : c'est
    /// l'exception, donc c'est elle qui se dit ; marquer aussi « dans le
    /// milestone » sur chaque ligne d'un rapport *de milestone* noierait
    /// l'exception dans la redondance. **Le label de priorité** : il doit être
    /// retrouvé dans `outside_candidates`, pas seulement dans `sub_issues` — le
    /// chercher uniquement dans les sub-issues rendrait tout candidat du dehors
    /// non-priorisé à l'affichage, alors que le tri, lui, l'a bien pris en
    /// compte. Un rapport qui n'explique pas son propre ordre.
    #[test]
    fn mika2699_la_provenance_nest_dite_que_pour_le_dehors() {
        let dedans = {
            let mut c = base_issue(2711);
            c.plan_present = true;
            c.priority_rank = Some(1);
            c.labels = vec!["p1-important".into()];
            c
        };
        let mut state = state_with(vec![dedans], ProgressCounts::default());
        state.outside_candidates = vec![PoolCandidate {
            number: 1700,
            state: IssueState::Open,
            priority_rank: Some(0),
            plan_present: true,
            labels: vec!["p0-critical".into()],
        }];
        let mut a = empty_assessment();
        a.pool = Some(assess_pool_sans_tailles(
            &[],
            &milestone_cands(&state),
            &state.outside_candidates,
        ));
        let out = Reporter::new().report(&state, &a);

        assert!(
            out.contains(
                "- À admettre : #1700 (p0-critical, hors milestone), #2711 (p1-important)"
            ),
            "le p0 du dehors passe devant, et seul le dehors porte sa provenance :\n{out}"
        );
    }

    /// Cas 19 — un refus hors milestone porte sa provenance lui aussi.
    ///
    /// Sans elle, l'opérateur lit « Refusé : #1700 — plan groomé absent » et
    /// cherche #1700 dans la liste des sub-issues du rapport, où il n'est pas.
    #[test]
    fn mika2699_un_refus_hors_milestone_porte_sa_provenance() {
        let mut state = state_with(vec![], ProgressCounts::default());
        state.outside_candidates = vec![PoolCandidate {
            number: 1700,
            state: IssueState::Open,
            priority_rank: None,
            plan_present: false,
            labels: vec![],
        }];
        let mut a = empty_assessment();
        a.pool = Some(assess_pool_sans_tailles(
            &[],
            &[],
            &state.outside_candidates,
        ));
        let out = Reporter::new().report(&state, &a);

        assert!(
            out.contains("- Refusé : #1700 (hors milestone) — plan groomé absent"),
            "provenance sans priorité : un seul segment entre parenthèses :\n{out}"
        );
    }

    /// Cas 19 bis (mika#2699 vol 3) — le second motif de refus arrive dans le
    /// rapport avec ses deux nombres, et sans avoir rien refondu.
    ///
    /// `RefusalReason` était déjà un `enum` pour cette raison : le rapporteur
    /// n'épelle aucun libellé, il appelle `label()`. Ce test vaut donc autant
    /// pour le motif que pour cette propriété — s'il faut toucher au rapporteur
    /// pour ajouter un troisième motif, c'est que quelqu'un a recopié un texte
    /// ici.
    #[test]
    fn mika2699_le_refus_par_la_borne_entre_dans_le_rapport_avec_ses_nombres() {
        let mut state = state_with(vec![], ProgressCounts::default());
        state.outside_candidates = vec![PoolCandidate {
            number: 1700,
            state: IssueState::Open,
            priority_rank: Some(0),
            plan_present: true,
            labels: vec!["p0-critical".into()],
        }];
        state.plan_sizes = [(1700, PlanSizeReading::Measured(900))]
            .into_iter()
            .collect();
        let mut a = empty_assessment();
        a.pool = Some(assess_pool(
            &[],
            &[],
            &state.outside_candidates,
            &state.plan_sizes,
        ));
        let out = Reporter::new().report(&state, &a);

        assert!(
            out.contains(
                "- Refusé : #1700 (p0-critical, hors milestone) — estimation 900 lignes \
                 au-delà de la borne par vol (279) — à découper"
            ),
            "le motif de la borne porte les deux nombres et la provenance :\n{out}"
        );
    }

    /// Cas 19 ter (mika#2699 vol 3) — une admission dont la taille n'a pas été
    /// mesurée est **nommée**, avec sa cause.
    ///
    /// Sans cette ligne, « À admettre : #1700 » se lit comme « borne vérifiée »,
    /// puisque c'est ce que le point 2 du ticket promet. L'opérateur n'aurait
    /// aucun moyen de distinguer un plan dimensionné d'un plan dont personne n'a
    /// pu lire la taille — et c'est précisément le seuil d'ouverture du vol 4
    /// (« aucun `ready` mal formé admis en ombre ») qui deviendrait invérifiable.
    #[test]
    fn mika2699_une_admission_de_taille_non_mesuree_est_nommee() {
        let mut state = state_with(vec![], ProgressCounts::default());
        state.outside_candidates = vec![PoolCandidate {
            number: 1700,
            state: IssueState::Open,
            priority_rank: Some(0),
            plan_present: true,
            labels: vec!["p0-critical".into()],
        }];
        state.plan_sizes = [(1700, PlanSizeReading::Unread(UnreadCause::FetchFailed))]
            .into_iter()
            .collect();
        let mut a = empty_assessment();
        a.pool = Some(assess_pool(
            &[],
            &[],
            &state.outside_candidates,
            &state.plan_sizes,
        ));
        let out = Reporter::new().report(&state, &a);

        assert!(
            out.contains("- À admettre : #1700 (p0-critical, hors milestone)"),
            "la lecture ratée ne refuse pas :\n{out}"
        );
        assert!(
            out.contains(
                "- Taille non vérifiée : #1700 (p0-critical, hors milestone) — \
                 plan introuvable au ref de sa branche"
            ),
            "…mais elle ne se tait pas, et la cause dit quel geste la corrige :\n{out}"
        );
    }

    /// Cas 19 quater (mika#2699 vol 3) — une taille **mesurée** sous la borne ne
    /// produit aucune ligne.
    ///
    /// Contrôle négatif du cas précédent : une ligne « Taille non vérifiée »
    /// rendue sur chaque admission aurait rendu le cas ci-dessus vert sans rien
    /// garantir, et aurait appris à l'opérateur à ignorer la ligne.
    #[test]
    fn mika2699_une_taille_mesuree_sous_la_borne_ne_dit_rien() {
        let mut state = state_with(vec![], ProgressCounts::default());
        state.outside_candidates = vec![PoolCandidate {
            number: 1700,
            state: IssueState::Open,
            priority_rank: Some(0),
            plan_present: true,
            labels: vec!["p0-critical".into()],
        }];
        state.plan_sizes = [(1700, PlanSizeReading::Measured(120))]
            .into_iter()
            .collect();
        let mut a = empty_assessment();
        a.pool = Some(assess_pool(
            &[],
            &[],
            &state.outside_candidates,
            &state.plan_sizes,
        ));
        let out = Reporter::new().report(&state, &a);

        assert!(out.contains("- À admettre : #1700 (p0-critical, hors milestone)"));
        assert!(
            !out.contains("Taille non vérifiée"),
            "un plan mesuré et sous la borne n'a rien à signaler :\n{out}"
        );
    }

    /// Cas 20 — une lecture tronquée se **déclare**, et l'ensemble hors
    /// milestone est vide ce cycle-là.
    ///
    /// C'est le motif de `scripts/pr-origin-report.sh` : refuser plutôt que mal
    /// répondre. Une page pleine ne dit pas « voici les candidats du dehors »,
    /// elle dit « il y en avait peut-être d'autres, et peut-être des p0 ». Un
    /// rapport qui admettrait le meilleur de la page lue affirmerait un ordre
    /// qu'il n'a pas mesuré.
    #[test]
    fn mika2699_une_lecture_tronquee_se_declare() {
        let mut state = state_with(vec![], ProgressCounts::default());
        state.outside_read_truncated = Some(true);
        let mut a = empty_assessment();
        a.pool = Some(assess_pool_sans_tailles(&[], &[], &[]));
        let out = Reporter::new().report(&state, &a);

        assert!(
            out.contains(
                "- Lecture hors milestone tronquée à 100 issues — aucun candidat hors \
                 milestone retenu ce cycle"
            ),
            "la troncature se dit, avec la borne demandée :\n{out}"
        );

        // Et `Some(false)` — la question posée, la page non pleine — ne dit
        // rien : une ligne « lecture non tronquée » à chaque cycle serait du
        // bruit qui finit par cacher la ligne qui compte.
        state.outside_read_truncated = Some(false);
        let nominal = Reporter::new().report(&state, &a);
        assert!(
            !nominal.contains("tronquée"),
            "une lecture complète ne se commente pas :\n{nominal}"
        );
    }

    // -- mika#2700 Phase 2 (c) vol 1 : la relève à grommer, en mode ombre ----

    /// Le rapport d'un état, bassin **et** relève posés par la production.
    ///
    /// Jamais par un littéral : le rapporteur doit rendre ce que l'assesseur
    /// produit, pas ce qu'un test imagine qu'il produit. Un fixture écrit à la
    /// main resterait vert le jour où `name_relief` change d'ordre.
    fn report_with_pool_and_relief(state: &MilestoneState) -> String {
        let cands = milestone_cands(state);
        let verdict =
            assess_pool_sans_tailles(&state.ready_pool, &cands, &state.outside_candidates);
        let relief = name_relief(&verdict, &cands, &state.outside_candidates);
        let mut a = empty_assessment();
        a.pool = Some(verdict);
        a.relief = Some(relief);
        Reporter::new().report(state, &a)
    }

    /// Cas 12 — chaque ticket de la relève est nommé, suffixé et motivé, dans
    /// l'ordre du bassin, et le reste est **compté**.
    ///
    /// Cinq candidats sans plan groomé, quatre nommés. Le fixture couvre les
    /// quatre formes de suffixe d'un seul coup : priorité + provenance
    /// (`#2740`), priorité seule (`#2731`, `#2726`), aucune des deux (`#2744`)
    /// — et `agent-core` est là pour que le rendu reste le **nom du label** et
    /// jamais le rang (l'écrire « p2 » serait faux).
    ///
    /// Le cinquième, `#2750`, n'est pas nommé : il est compté. Taire ce compte
    /// ferait lire les quatre comme la totalité de ce qui attend.
    #[test]
    fn mika2700_la_section_releve_nomme_chaque_ticket_avec_son_motif() {
        let subs = vec![
            {
                let mut c = base_issue(2731);
                c.priority_rank = Some(1);
                c.labels = vec!["p1-important".into()];
                c
            },
            {
                let mut c = base_issue(2726);
                c.priority_rank = Some(2);
                c.labels = vec!["agent-core".into()];
                c
            },
            base_issue(2744),
        ];
        let mut state = state_with(subs, ProgressCounts::default());
        state.outside_candidates = vec![
            PoolCandidate {
                number: 2740,
                state: IssueState::Open,
                priority_rank: Some(0),
                plan_present: false,
                labels: vec!["p0-critical".into()],
            },
            PoolCandidate {
                number: 2750,
                state: IssueState::Open,
                priority_rank: None,
                plan_present: false,
                labels: vec![],
            },
        ];
        let out = report_with_pool_and_relief(&state);
        let section = out
            .split("### Relève à grommer")
            .nth(1)
            .expect("la section de la relève doit être rendue");

        for attendu in [
            "- #2740 (p0-critical, hors milestone) — à grommer : plan groomé absent",
            "- #2731 (p1-important) — à grommer : plan groomé absent",
            "- #2726 (agent-core) — à grommer : plan groomé absent",
            "- #2744 — à grommer : plan groomé absent",
        ] {
            assert!(
                section.contains(attendu),
                "ligne attendue « {attendu} » dans :\n{section}"
            );
        }

        // L'ordre du bassin : le p0 du dehors passe devant tout le milestone,
        // puis la priorité, puis la provenance, puis le numéro.
        let ordre: Vec<usize> = [2740, 2731, 2726, 2744]
            .iter()
            .map(|n| {
                section
                    .find(&format!("- #{n}"))
                    .expect("chaque nommé doit être dans la section")
            })
            .collect();
        assert!(
            ordre.windows(2).all(|w| w[0] < w[1]),
            "la relève se lit dans l'ordre de tri du bassin :\n{section}"
        );

        assert!(
            !section.contains("#2750"),
            "au-delà de quatre, un candidat est compté et non nommé :\n{section}"
        );
        assert_eq!(RELIEF_SIZE, 4, "la borne rendue est celle du module");
        assert!(
            section.contains("- 1 autre candidat au-delà des 4 nommés"),
            "le reste se compte, au singulier quand il vaut un :\n{section}"
        );
    }

    /// Cas 13 — pas de relève, pas de section. Deux absences, deux faits
    /// distincts, et un contrôle négatif.
    ///
    /// `relief: None` veut dire « la relève n'a pas été nommée » (blob
    /// antérieur au champ) : une section affirmerait une mesure qui n'a pas eu
    /// lieu. `next` vide veut dire « rien à grommer d'avance » : une section
    /// vide inviterait à chercher une cause là où il n'y a rien.
    ///
    /// Le troisième arme est ce qui sépare les deux premiers d'un rapporteur
    /// qui ne rendrait jamais la section : sans lui, l'absence se lirait comme
    /// une condition qui marche.
    #[test]
    fn mika2700_sans_releve_la_section_est_absente() {
        let state = state_with(vec![base_issue(1)], ProgressCounts::default());

        let sans_mesure = Reporter::new().report(&state, &empty_assessment());
        assert!(
            !sans_mesure.contains("### Relève à grommer"),
            "pas de mesure ⇒ pas de section — obtenu :\n{sans_mesure}"
        );

        let mut a = empty_assessment();
        let verdict = assess_pool_sans_tailles(&[], &[], &[]);
        a.relief = Some(name_relief(&verdict, &[], &[]));
        a.pool = Some(verdict);
        let mesure_vide = Reporter::new().report(&state, &a);
        assert!(
            !mesure_vide.contains("### Relève à grommer"),
            "rien à grommer ⇒ pas de section — obtenu :\n{mesure_vide}"
        );
        assert!(
            !mesure_vide.contains("au-delà des"),
            "aucun reste à compter quand il n'y a aucun nommé :\n{mesure_vide}"
        );

        let avec = report_with_pool_and_relief(&state_with(
            vec![base_issue(2731)],
            ProgressCounts::default(),
        ));
        assert!(
            avec.contains("### Relève à grommer"),
            "contrôle négatif : un candidat à grommer et la section est là :\n{avec}"
        );
    }

    /// Cas 14 — la relève se lit **derrière** le bassin, et « mode ombre »
    /// est dans son titre.
    ///
    /// L'ordre est celui de la décision : ce qui attend n'a de sens qu'après ce
    /// qui entre. Et le titre porte la mention parce qu'une liste de tickets
    /// « à grommer » se lit comme un geste fait si rien ne dit le contraire à
    /// l'endroit où l'œil tombe — une note en fin de section arriverait après
    /// la conclusion du lecteur.
    #[test]
    fn mika2700_la_releve_se_lit_derriere_le_bassin_et_porte_le_mode_ombre_dans_son_titre() {
        let state = state_with(vec![base_issue(2731)], ProgressCounts::default());
        let out = report_with_pool_and_relief(&state);

        assert!(
            out.contains("### Relève à grommer (mode ombre — aucun groom déclenché)"),
            "le mode ombre est dans le titre :\n{out}"
        );
        let bassin = out
            .find("### Bassin ready")
            .expect("la section du bassin doit être rendue");
        let releve = out
            .find("### Relève à grommer")
            .expect("la section de la relève doit être rendue");
        assert!(
            bassin < releve,
            "la relève se lit derrière le bassin :\n{out}"
        );
    }

    /// Cas 15 — une lecture hors milestone tronquée borne la relève au
    /// milestone cible, et le dit.
    ///
    /// Sur `Some(true)`, `outside_candidates` est vide par décision du lecteur :
    /// la relève nommée est alors celle du milestone, pas celle du dépôt. Un
    /// opérateur qui lit quatre numéros sans cette ligne lirait « les 4
    /// prochains du dépôt » — un ordre que le cycle n'a pas mesuré.
    ///
    /// `Some(false)` est la question posée et la page non pleine ; `None` est
    /// « pas mesuré ». Ni l'un ni l'autre ne justifie une ligne, et pour deux
    /// raisons différentes qu'une même absence sert bien.
    #[test]
    fn mika2700_une_lecture_hors_milestone_tronquee_borne_la_releve_au_milestone() {
        let mut state = state_with(vec![base_issue(2731)], ProgressCounts::default());
        state.outside_read_truncated = Some(true);
        let out = report_with_pool_and_relief(&state);
        let section = out
            .split("### Relève à grommer")
            .nth(1)
            .expect("la section de la relève doit être rendue");
        assert!(
            section.contains(
                "- Lecture hors milestone tronquée à 100 issues — relève limitée au \
                 milestone cible"
            ),
            "la troncature se dit dans la section de la relève aussi :\n{section}"
        );

        for muet in [Some(false), None] {
            state.outside_read_truncated = muet;
            let nominal = report_with_pool_and_relief(&state);
            let section = nominal
                .split("### Relève à grommer")
                .nth(1)
                .expect("la section de la relève doit être rendue");
            assert!(
                !section.contains("tronquée"),
                "{muet:?} ne justifie aucune ligne de troncature :\n{section}"
            );
        }
    }

    // -- mika#2923 : le rapport nomme le refus de porte -----------------------

    /// V5 — le rapport du sink porte une ligne de refus par ticket gaté, avec
    /// la raison, et aucun gaté n'apparaît sur la ligne « À admettre » (AC3).
    ///
    /// Le test lit le **rapport rendu**, pas le verdict : c'est le rapport que
    /// l'opérateur a sous les yeux, et le ticket est né d'une ligne de ce
    /// rapport. Le rapporteur, lui, ne reçoit **aucune ligne de code** de ce
    /// vol — il appelle déjà `reason.label()`. Si ce test exige une édition de
    /// `reporter.rs`, c'est que quelqu'un y a recopié un libellé.
    #[test]
    fn mika2923_le_rapport_nomme_chaque_refus_de_porte_et_nen_admet_aucun() {
        let gate = |n: u64| {
            let mut c = base_issue(n);
            c.plan_present = true;
            c.priority_rank = Some(1);
            c.labels = vec!["p1-important".into(), "operator-gated".into()];
            c
        };
        let non_gate = {
            let mut c = base_issue(2923);
            c.plan_present = true;
            c.priority_rank = Some(1);
            c.labels = vec!["p1-important".into()];
            c
        };
        let mut state = state_with(
            vec![gate(2698), gate(2699), non_gate],
            ProgressCounts::default(),
        );
        state.outside_candidates = vec![PoolCandidate {
            number: 1806,
            state: IssueState::Open,
            priority_rank: Some(1),
            plan_present: true,
            labels: vec!["p1-important".into(), "operator-gated".into()],
        }];
        let mut a = empty_assessment();
        a.pool = Some(assess_pool_sans_tailles(
            &[],
            &milestone_cands(&state),
            &state.outside_candidates,
        ));
        let out = Reporter::new().report(&state, &a);

        for ligne in [
            "- Refusé : #1806 (p1-important, hors milestone) — tenu par l'opérateur — \
             label `operator-gated` à retirer",
            "- Refusé : #2698 (p1-important) — tenu par l'opérateur — \
             label `operator-gated` à retirer",
            "- Refusé : #2699 (p1-important) — tenu par l'opérateur — \
             label `operator-gated` à retirer",
        ] {
            assert!(
                out.contains(ligne),
                "une ligne de refus par ticket gaté, avec la raison :\n{ligne}\n--- \
                 rapport ---\n{out}"
            );
        }

        let section = out
            .split("### Bassin ready")
            .nth(1)
            .expect("la section du bassin doit être rendue");
        let ligne_admis = section
            .lines()
            .find(|l| l.starts_with("- À admettre"))
            .expect("le non-gaté est recommandé, donc la ligne existe");
        for gate in ["2698", "2699", "1806"] {
            assert!(
                !ligne_admis.contains(gate),
                "aucun ticket gaté n'apparaît dans « À admettre » :\n{ligne_admis}"
            );
        }
        assert!(
            ligne_admis.contains("#2923"),
            "contrôle négatif : le candidat non gaté y est, lui :\n{ligne_admis}"
        );
    }
}

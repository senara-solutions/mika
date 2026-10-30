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

use super::evaluator::{
    CiKnowledge, CiUnreadCause, DeathCause, FlightIssue, FlightRecommendation, FlightVerdict,
    MilestoneMembership, NotFlightReason, PrStateKnowledge,
};
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
        // Ce que la lecture du milestone a rendu, face à sa borne et au compte
        // de l'API (mika#2969). Juste sous la progression, parce que c'est la
        // progression que la troncature fausse : un ticket sorti de la page ne
        // compte plus dans le dénominateur, et rien ne le disait.
        //
        // La borne épelée est celle que **l'audit transporte**, jamais la
        // constante : un blob du puits hors-ligne doit se relire avec la borne
        // qui l'a produit. `None` ne dit rien du tout — ni « complète » ni
        // « tronquée » —, parce que « la question n'a pas été posée » n'est pas
        // une mesure (mika#2331), et parce que c'est « complète » qui serait ici
        // l'affirmation dangereuse.
        //
        // Le compte de l'API est dit avec sa **magnitude nommée** : il compte
        // les items du milestone, PR comprises, là où la lecture ne rend que des
        // issues. Les deux nombres ne sont pas égalisables et n'ont pas à
        // l'être — 192 contre 117 sur le #33 au 2026-10-29, l'écart valant ses
        // 75 PR. Le signal de complétude est le mot « complète », exact par
        // construction ; l'écart, lui, est de la classe mika#2205 et n'a aucun
        // pouvoir de détection.
        if let Some(audit) = &state.milestone_read_audit {
            if audit.truncated() {
                let _ = writeln!(
                    out,
                    "- Lecture du milestone TRONQUÉE à {} tickets — des tickets du #{} sont hors de ce rapport, de sa progression et de son bassin (API du milestone : {} items, PR comprises)",
                    audit.limit, state.milestone_ref.number, audit.api_item_total,
                );
            } else {
                let _ = writeln!(
                    out,
                    "- Lecture du milestone : {} tickets lus, borne {} — complète (API du milestone : {} items, PR comprises — autre population, mika#2969)",
                    audit.returned, audit.limit, audit.api_item_total,
                );
            }
        }
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
        //
        // Le titre nomme le périmètre depuis mika#2921 : la section couvre
        // **toute la boucle** — tout vol `implement` terminé sur un dépôt sous
        // contrôle — là où le reste du rapport reste par milestone. Sans cette
        // mention, un opérateur lirait les vols hors milestone cible comme un
        // débordement du rapport au lieu de son périmètre déclaré.
        if !assessment.flight_verdicts.is_empty() {
            writeln!(
                out,
                "### Vols terminés depuis le cycle précédent (toute la boucle — mode ombre, aucune écriture)"
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
/// PR-linkage: reuses `SubIssue::pr_number` and `SubIssue::pr_url`, both
/// populated by `Reader::compose_from_gh_outputs` — from the
/// `closingIssuesReferences` reverse index when the PR carries the milestone,
/// else from the ticket's own `closedByPullRequestsReferences` (mika#2924).
///
/// Three arms, because the two resolution paths do not carry the same thing
/// and collapsing them would make the report lie:
///
/// 1. **An URL is carried** (`pr_url`) — render it verbatim. It comes from the
///    issue-side reference, whose entry carries a nested `repository` object:
///    the PR may live in **another** repository, so recomposing
///    `{repo}/pull/{number}` would point at a PR that does not exist.
/// 2. **A number only** (`pr_number`) — recompose from `repo`. This is the
///    milestone-index path, built on the PR list of the milestone's own
///    repository, so the recomposition is sound there.
/// 3. **Neither** — the entry reads `closed (no linked PR)`, **word for
///    word**: that string is what the AC3 probe counts on the sink report
///    (`grep -c 'closed (no linked PR)'`, 87 → 14 expected). Changing it
///    silently retires the instrument.
///
/// The third arm now means what it says: hand-closed, and nothing else. Until
/// mika#2924 it also absorbed "the PR exists but does not carry the
/// milestone" — the autonomous loop's entire output, 87 lines on milestone 33
/// — and the report read as though nothing had been delivered.
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
        match (s.pr_url.as_deref(), s.pr_number) {
            (Some(url), _) => {
                let _ = writeln!(
                    out,
                    "  - #{n} {title} — PR {url}",
                    n = s.number,
                    title = s.title
                );
            }
            (None, Some(pr)) => {
                let _ = writeln!(
                    out,
                    "  - #{n} {title} — PR https://github.com/{repo}/pull/{pr}",
                    n = s.number,
                    title = s.title
                );
            }
            (None, None) => {
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
/// Forme (mika#2921) :
/// `- <dépôt>#<ticket> — <milestone> — <issue> — <tours> — [<état de PR> —] <recommandation>`.
///
/// Chaque segment dit explicitement quand il ne sait pas, parce qu'un segment
/// manquant décale la lecture et qu'un `0` par défaut mentirait. Deux segments
/// sont nés de l'élargissement du périmètre à toute la boucle :
///
/// - **le dépôt**, parce que deux vols de deux dépôts peuvent porter le même
///   numéro de ticket (`mika#193` et `mika-cloud#193` existent tous les deux) et
///   que depuis mika#2921 ils se lisent dans la même section ;
/// - **l'appartenance au milestone cible**, qui est ce que le rapport *sait* :
///   pour un vol hors cible le milestone réel n'a pas été lu, donc il n'est pas
///   rendu — l'absence nommée est l'absence du milestone **cible**, jamais une
///   prétention sur l'appartenance du ticket à un autre milestone.
///
/// Le segment d'état de PR, lui, est **omis** quand l'état a été lu et trouvé
/// vide ([`PrStateKnowledge::Read(None)`]) : un vol du milestone cible sans PR
/// connue ne gagne pas un segment qui ne dit rien. C'est aussi ce qui laisse
/// intact le rendu de la population d'avant mika#2921.
///
/// Le segment de CI (mika#2968, AC3) est le seul qui ne s'omette **jamais** :
/// il est inséré entre l'état de PR et la recommandation, parce que c'est l'une
/// des deux prémisses sur lesquelles cette recommandation repose et qu'une
/// prémisse se lit avant sa conclusion. Il est toujours rendu parce que
/// l'omettre ferait de « rien n'a été lu » et de « ce rapport est antérieur au
/// champ » les mêmes octets (classe mika#2205) : le silence serait le seul
/// endroit où une CI non lue pourrait passer pour verte.
pub fn render_flight_verdict(v: &FlightVerdict) -> String {
    let mut segments = vec![
        flight_ticket_label(v),
        flight_milestone_label(v.in_target_milestone).to_string(),
        flight_issue_label(&v.issue),
        flight_turns_label(v),
    ];
    if let Some(etat) = flight_pr_state_label(&v.pr_state) {
        segments.push(etat);
    }
    segments.push(flight_ci_label(&v.ci));
    segments.push(flight_recommendation_label(v.recommendation).to_string());
    format!("- {}", segments.join(" — "))
}

/// `<dépôt court>#<numéro>` — l'adresse du ticket telle qu'un humain l'écrit.
///
/// Le dépôt est rendu **court** (`mika`, `mika-cloud`), parce que c'est la forme
/// que le ticket, les PR et les messages de commit emploient, et que le
/// périmètre jugé est d'un seul propriétaire par construction (il vient de
/// `DISPATCHABLE_REPOS`).
///
/// `(ticket inconnu)` est conservé **tel quel** quand le numéro manque : sans
/// numéro, le dépôt ne désigne rien de lisible. Et un numéro sans dépôt dit
/// `(dépôt inconnu)#<n>` plutôt qu'un `#<n>` nu, parce que le `#<n>` nu est
/// exactement l'ambiguïté que le champ `repo` existe pour lever.
fn flight_ticket_label(v: &FlightVerdict) -> String {
    let Some(n) = v.issue_number else {
        return "(ticket inconnu)".to_string();
    };
    match v.repo.as_deref() {
        Some(repo) => {
            let court = repo.rsplit('/').next().unwrap_or(repo);
            format!("{court}#{n}")
        }
        None => format!("(dépôt inconnu)#{n}"),
    }
}

/// Ce que le rapport sait de l'appartenance du vol au milestone cible.
///
/// **Lu sur son propre champ depuis le vol 2a de mika#2968**, et non plus
/// déduit de [`PrStateKnowledge`]. L'ancienne déduction était correcte sous une
/// prémisse qui vient de tomber : l'état de PR n'avait qu'une source, les
/// sous-issues du milestone cible, donc un appariement réussi disait les deux
/// faits à la fois. Maintenant que `pr_state` peut venir d'une lecture **par
/// numéro de PR**, la déduction dirait « hors milestone cible » d'un vol lu par
/// l'autre porte.
///
/// Trois arbres, parce qu'il y a trois états de connaissance et qu'un
/// appariement qui n'a pas eu lieu n'est pas un appariement qui a échoué (motif
/// mika#2331).
fn flight_milestone_label(membership: MilestoneMembership) -> &'static str {
    match membership {
        MilestoneMembership::InTarget => "milestone cible",
        MilestoneMembership::OutOfTarget => "hors milestone cible",
        MilestoneMembership::NotRead => "appartenance au milestone non lue",
    }
}

/// L'état de la PR, ou le fait qu'il n'a pas été lu — `None` pour « lu et vide ».
fn flight_pr_state_label(pr_state: &PrStateKnowledge) -> Option<String> {
    match pr_state {
        PrStateKnowledge::Read(Some(etat)) => Some(format!("PR {etat}")),
        PrStateKnowledge::Read(None) => None,
        PrStateKnowledge::NotRead => Some("état de PR non lu".to_string()),
    }
}

/// Ce que le manager sait de la CI de la tête de la PR, et de quand il le sait
/// (mika#2968, AC3).
///
/// Le libellé d'une lecture porte **l'instant du cycle** et le dit ainsi :
/// « lue au cycle de <instant> ». La formule est délibérément moins précise que
/// « lue à <instant> », parce que l'instant est `ctx.now`, capturé après le
/// `reader.read()` du cycle — il majore la lecture de quelques secondes. Dire
/// « au cycle de » est exact ; dire « à » serait une précision inventée.
///
/// `CiState::None` se dit « CI sans check » et non « CI non lue » : c'est une
/// lecture qui a abouti et n'a trouvé aucun check. La confusion des deux est
/// exactement ce que [`CiKnowledge`] existe pour rendre impossible.
///
/// Le `match` est exhaustif sur les deux axes, sans bras `_` (mika#2923) : un
/// cinquième état CI ou une cinquième cause de non-lecture doit se décider ici,
/// pas se glisser dans un fourre-tout qui dirait le mot d'un autre.
fn flight_ci_label(ci: &CiKnowledge) -> String {
    match ci {
        CiKnowledge::Read { state, at } => {
            let etat = match state {
                CiState::Success => "CI verte",
                CiState::Failed => "CI rouge",
                CiState::Pending => "CI en cours",
                CiState::None => "CI sans check",
            };
            format!("{etat} (lue au cycle de {at})")
        }
        CiKnowledge::Unread(cause) => format!("CI non lue ({})", ci_unread_label(*cause)),
    }
}

/// Pourquoi la CI n'a pas été lue — nommé, parce que les dix causes n'appellent
/// pas la même action de l'opérateur : « lecture gh en échec » demande de
/// regarder le jeton du dépôt, « PR hors de la page lue » de regarder la borne,
/// « ce vol n'a pas de PR » rien du tout.
///
/// Les quatre premières ne sont plus écrites par aucune composition depuis
/// mika#2968 (vol 2a) ; elles restent rendues parce que les verdicts du vol 1
/// les portent encore. Voir [`CiUnreadCause`], qui date chacune.
fn ci_unread_label(cause: CiUnreadCause) -> &'static str {
    match cause {
        CiUnreadCause::NotFromPrList => "PR hors liste du milestone",
        CiUnreadCause::NoSubIssue => "aucune sous-issue appariée",
        CiUnreadCause::NoIssueNumber => "vol sans ticket",
        CiUnreadCause::NotMeasured => "non mesurée",
        CiUnreadCause::NoPrNumber => "ce vol n'a pas de PR",
        CiUnreadCause::NoRepo => "vol sans dépôt",
        CiUnreadCause::NoRunner => "aucun lecteur gh câblé",
        CiUnreadCause::ReadFailed => "lecture gh en échec",
        CiUnreadCause::NotFound => "PR hors de la page lue",
        CiUnreadCause::PrNotOpen => "PR non ouverte",
    }
}

/// `pub(crate)` depuis mika#2922 (vol 3) : le compte de concordance rend la
/// **même** issue que le rapport du manager, donc il appelle ce label plutôt
/// que d'en écrire un second. Un second vocabulaire français pour une seule
/// grammaire serait la divergence programmée de mika#2158 — visible le jour où
/// un terme change d'un côté seulement, et illisible pour l'opérateur qui lit
/// les deux surfaces.
pub(crate) fn flight_issue_label(issue: &FlightIssue) -> String {
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
        // mika#2946 : le libellé commence par « pas un vol » parce que c'est la
        // phrase que l'opérateur doit lire d'abord — avant mika#2946 ces lignes
        // disaient « mort » et réclamaient une reprise à la main.
        FlightIssue::NotAFlight { reason } => match reason {
            NotFlightReason::GroomCompleted => "pas un vol — groom terminé".to_string(),
            NotFlightReason::DispatchRefused => "pas un vol — dispatch refusé".to_string(),
            NotFlightReason::Superseded => "pas un vol — remplacé".to_string(),
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

/// `pub(crate)` depuis mika#2922 (vol 3), même raison que
/// [`flight_issue_label`].
pub(crate) fn flight_recommendation_label(r: FlightRecommendation) -> &'static str {
    match r {
        FlightRecommendation::MergeCandidate => "merge-candidat",
        FlightRecommendation::ManualResumption => "reprise à la main",
        FlightRecommendation::Regroom => "re-groom",
        FlightRecommendation::Nothing => "aucune recommandation",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::milestone_manager::pool::{
        PlanSizeReading, PoolCandidate, ReadyTicket, UnreadCause, assess_pool,
        assess_pool_sans_tailles,
    };
    use crate::milestone_manager::reader::compose_from_gh_outputs;
    use crate::milestone_manager::relief::name_relief;
    use crate::milestone_manager::types::{
        ContentionEvent, MilestoneReadAudit, MilestoneRef, ProgressCounts, Recommendation, SubIssue,
    };

    /// Les candidats du milestone tels que l'assesseur les projette. Les tests
    /// du rapporteur passent par ce helper plutôt que par un littéral, pour que
    /// la projection sous test soit la même que celle en production.
    fn milestone_cands(state: &MilestoneState) -> Vec<PoolCandidate> {
        state.sub_issues.iter().map(PoolCandidate::from).collect()
    }

    fn base_issue(n: u64) -> SubIssue {
        SubIssue {
            missing_grooming_signals: None,
            number: n,
            title: format!("issue {n}"),
            state: IssueState::Open,
            priority_rank: None,
            plan_present: false,
            plan_reading: None,
            dormancy: None,
            branch_present: false,
            pr_number: None,
            pr_state: None,
            pr_url: None,
            pr_link_source: None,
            merged_prs: None,
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
            milestone_read_audit: None,
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
            repo: Some("senara-solutions/mika".into()),
            issue_number,
            issue,
            turns_used,
            hit_ceiling,
            turn_ceiling: None,
            last_commit_turn: None,
            // Un vol du milestone cible dont le `Reader` n'a trouvé aucune PR :
            // la population d'avant mika#2921, donc le rendu attendu ne bouge
            // pas.
            pr_state: PrStateKnowledge::Read(None),
            in_target_milestone: MilestoneMembership::InTarget,
            ci: CiKnowledge::Unread(CiUnreadCause::NotMeasured),
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
                "### Vols terminés depuis le cycle précédent (toute la boucle — mode ombre, aucune écriture)"
            ),
            "le titre doit dire le périmètre ET le mode ombre. Obtenu :\n{out}"
        );
        assert!(
            out.contains(
                "- mika#2693 — milestone cible — PR ouverte (#2696) — 83 tours (plafond non instrumenté) — CI non lue (non mesurée) — merge-candidat"
            ),
            "Obtenu :\n{out}"
        );
        assert!(
            out.contains(
                "- mika#2661 — milestone cible — mort au plafond (budget_exhausted) — 200 tours — CI non lue (non mesurée) — re-groom"
            ),
            "Obtenu :\n{out}"
        );
        assert!(
            out.contains(
                "- mika#2655 — milestone cible — PR de sauvetage (#2690) — 141 tours (plafond non instrumenté) — CI non lue (non mesurée) — reprise à la main"
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
                "- (ticket inconnu) — milestone cible — mort — cause non classée (PLAN_COMMITTED) — \
                 tours non lus (plafond non instrumenté) — CI non lue (non mesurée) — \
                 reprise à la main"
            ),
            "Obtenu :\n{out}"
        );
        assert!(
            !out.contains("— 0 tours"),
            "une mesure absente ne devient jamais zéro (motif mika#2331). Obtenu :\n{out}"
        );
    }

    /// Les six causes de mort, les trois motifs de non-vol et les quatre
    /// recommandations ont chacun leur libellé : un `match` muet rendrait deux
    /// causes distinctes identiques à l'écran.
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
        // mika#2946 : les trois motifs de non-vol. Chacun a son libellé parce
        // que c'est le motif que l'opérateur doit lire pour savoir s'il a
        // quelque chose à faire ailleurs — un groom terminé est un succès, un
        // dispatch refusé attend un groom, un remplacement n'attend rien.
        let non_vols = [
            (
                NotFlightReason::GroomCompleted,
                "pas un vol — groom terminé",
            ),
            (
                NotFlightReason::DispatchRefused,
                "pas un vol — dispatch refusé",
            ),
            (NotFlightReason::Superseded, "pas un vol — remplacé"),
        ];
        for (reason, expected) in non_vols {
            assert_eq!(
                flight_issue_label(&FlightIssue::NotAFlight { reason }),
                expected
            );
            assert!(
                !flight_issue_label(&FlightIssue::NotAFlight { reason }).contains("mort"),
                "une ligne de non-vol ne dit jamais « mort » : c'est le défaut \
                 que mika#2946 ferme, et il se lit d'abord à l'écran"
            );
        }
        let recos = [
            (FlightRecommendation::MergeCandidate, "merge-candidat"),
            (FlightRecommendation::ManualResumption, "reprise à la main"),
            (FlightRecommendation::Regroom, "re-groom"),
            (FlightRecommendation::Nothing, "aucune recommandation"),
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

    // -- mika#2921 : la ligne porte le dépôt, le milestone et la lecture --

    /// Un verdict hors milestone cible — donc non apparié, donc `NotRead`.
    fn verdict_hors_cible(issue_number: Option<u64>, repo: Option<&str>) -> FlightVerdict {
        FlightVerdict {
            pr_state: PrStateKnowledge::NotRead,
            // Les **deux** axes, nommés séparément depuis le vol 2a de
            // mika#2968 : hors cible *et* état de PR non lu. C'est la
            // conjonction que produisait l'ancien champ unique, et l'écrire en
            // deux lignes est ce qui rend les deux moitiés du test suivant
            // réellement indépendantes.
            in_target_milestone: MilestoneMembership::OutOfTarget,
            repo: repo.map(str::to_string),
            ..verdict(
                issue_number,
                FlightIssue::PrOpened {
                    number: Some(9001),
                    url: None,
                },
                Some(42),
                false,
                FlightRecommendation::MergeCandidate,
            )
        }
    }

    /// AC5 côté rendu : un vol hors cible **nomme** sa non-lecture, et ne rend
    /// aucun numéro de milestone.
    ///
    /// Le contrôle positif est dans le même test, et c'est lui qui rend
    /// l'assertion concluante : un rendu qui dirait « non lu » partout passerait
    /// la première moitié. Les deux arms côte à côte séparent « le type est
    /// correct » de « le rendu est branché ».
    #[test]
    fn mika2921_un_vol_hors_cible_nomme_sa_non_lecture_et_pas_son_milestone() {
        let hors = render_flight_verdict(&verdict_hors_cible(
            Some(2900),
            Some("senara-solutions/mika"),
        ));
        assert_eq!(
            hors,
            "- mika#2900 — hors milestone cible — PR ouverte (#9001) — 42 tours (plafond non instrumenté) — état de PR non lu — CI non lue (non mesurée) — merge-candidat",
        );

        let dans = render_flight_verdict(&FlightVerdict {
            pr_state: PrStateKnowledge::Read(Some("open".into())),
            in_target_milestone: MilestoneMembership::InTarget,
            ..verdict_hors_cible(Some(2900), Some("senara-solutions/mika"))
        });
        assert_eq!(
            dans,
            "- mika#2900 — milestone cible — PR ouverte (#9001) — 42 tours (plafond non instrumenté) — PR open — CI non lue (non mesurée) — merge-candidat",
        );
        // L'aiguille **nomme son segment** depuis mika#2968 : « CI non lue »
        // contient « non lu », donc un `contains("non lu")` nu rougirait sur la
        // non-lecture de l'autre axe — un faux rouge qui ne dit rien de l'état
        // de PR, le seul que ce test atteste.
        assert!(
            !dans.contains("état de PR non lu"),
            "un état lu ne se rend jamais comme non lu. Obtenu : {dans}"
        );
    }

    /// « Lu, et vide » ne gagne **aucun** segment d'état de PR : cinq segments
    /// (six depuis mika#2968, qui a ajouté celui de la CI), pas celui-là.
    ///
    /// C'est ce qui laisse le rendu de la population d'avant mika#2921 intact —
    /// un segment vide serait du bruit sur chaque vol du milestone cible sans PR.
    #[test]
    fn mika2921_un_etat_lu_et_vide_ne_gagne_aucun_segment() {
        let ligne = render_flight_verdict(&FlightVerdict {
            pr_state: PrStateKnowledge::Read(None),
            in_target_milestone: MilestoneMembership::InTarget,
            ..verdict_hors_cible(Some(2900), Some("senara-solutions/mika"))
        });
        assert_eq!(
            ligne,
            "- mika#2900 — milestone cible — PR ouverte (#9001) — 42 tours (plafond non instrumenté) — CI non lue (non mesurée) — merge-candidat",
        );
        // Même raison que ci-dessus : l'aiguille nomme le segment d'état de PR,
        // pas le mot « non lu » que la CI emploie désormais sur son propre axe.
        assert!(
            !ligne.contains("PR open") && !ligne.contains("état de PR non lu"),
            "aucun segment d'état quand l'état lu est vide. Obtenu : {ligne}"
        );
    }

    /// Le dépôt est rendu court, et il **distingue** : `mika#193` et
    /// `mika-cloud#193` sont deux vols, pas un seul lu deux fois.
    #[test]
    fn mika2921_le_depot_court_distingue_deux_vols_de_meme_numero() {
        let mika = render_flight_verdict(&verdict_hors_cible(
            Some(193),
            Some("senara-solutions/mika"),
        ));
        let cloud = render_flight_verdict(&verdict_hors_cible(
            Some(193),
            Some("senara-solutions/mika-cloud"),
        ));
        assert!(mika.starts_with("- mika#193 — "), "obtenu : {mika}");
        assert!(cloud.starts_with("- mika-cloud#193 — "), "obtenu : {cloud}");
        assert_ne!(mika, cloud);
    }

    /// Un numéro sans dépôt le dit. Un `#193` nu est exactement l'ambiguïté que
    /// le champ `repo` existe pour lever — le rendre nu la réintroduirait.
    #[test]
    fn mika2921_un_numero_sans_depot_ne_se_lit_pas_comme_un_depot_connu() {
        let ligne = render_flight_verdict(&verdict_hors_cible(Some(193), None));
        assert!(
            ligne.starts_with("- (dépôt inconnu)#193 — "),
            "obtenu : {ligne}"
        );
    }

    /// **AC4, octet pour octet.** Sur la même entrée, tout ce qui n'est pas la
    /// section « Vols terminés » rend les mêmes octets qu'avant mika#2921.
    ///
    /// Les deux moitiés sont figées ici plutôt que comparées entre deux appels :
    /// un golden écrit à la main est ce qui rougit sur un changement, là où deux
    /// appels du même code rendraient toujours la même chose.
    #[test]
    fn mika2921_le_rapport_par_milestone_rend_les_memes_octets() {
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

        const AVANT: &str = concat!(
            "## Milestone: Luminescent Core reconciliation [senara-solutions/mika#1799]\n",
            "\n",
            "### État\n",
            "- Progress: 0/0 sub-issues complete (0% done)\n",
            "- Completed: (none)\n",
            "- In-flight: (none)\n",
            "- Blocked: (none)\n",
            "- Unstarted: (none)\n",
            "\n",
            "### Next action (recommendation)\n",
            "- Next: — milestone complete — no open sub-issues remain\n",
            "\n",
        );
        const APRES: &str = concat!(
            "### Silent-progress / drift alerts\n",
            "- (none)\n",
            "\n",
            "### Cross-cutting concerns (from mika-arch-groom-milestone)\n",
            "- (none — Phase 1: hydration deferred to Phase 1.5)\n",
        );

        let debut = out.find("### Vols terminés").expect("section présente");
        let fin = debut
            + out[debut..]
                .find("### Silent-progress")
                .expect("la dérive suit les vols");
        assert_eq!(&out[..debut], AVANT, "les sections d'avant ont bougé");
        assert_eq!(&out[fin..], APRES, "les sections d'après ont bougé");
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

    // -----------------------------------------------------------------
    // mika#2924 AC1 — le rapport, depuis une sortie `gh` réellement capturée.
    // -----------------------------------------------------------------

    /// La sortie `gh issue list --milestone 33` capturée le 2026-10-26,
    /// filtrée sur les deux contrôles. Même document que celui que les tests du
    /// `Reader` consomment : un seul fichier témoigne, pour les deux étages.
    const FIXTURE_MILESTONE_33: &str =
        include_str!("../../tests/fixtures/mika2924/issues-milestone-33.json");

    /// mika#2924 AC1 — **le contrôle positif et le contrôle négatif dans le
    /// même test**, sur le même document, bout en bout : la sortie `gh`
    /// capturée traverse le `Reader` puis le `Reporter`, et c'est le texte du
    /// rapport qui est assert.
    ///
    /// La liste des PR du milestone est `[]`, ce qui *est* la situation de
    /// production : le pilote rattache le milestone au ticket et jamais à la
    /// PR, et c'est exactement ce qui a fait dire `closed (no linked PR)` à 87
    /// lignes du rapport du sink.
    ///
    /// Les deux contrôles sont indissociables. Sans le négatif, un rapporteur
    /// qui rendrait un lien de PR pour tout ticket fermé passerait le positif ;
    /// sans le positif, un rapporteur qui ne rendrait jamais rien passerait le
    /// négatif. C'est le couple qui atteste.
    #[test]
    fn mika2924_le_rapport_rend_la_pr_de_cloture_et_garde_le_mot_pour_mot_sans_pr() {
        let milestone_json = r#"{"title": "M33", "description": "sink", "state": "open", "created_at": "2026-10-01T00:00:00Z", "due_on": null}"#;
        let r = MilestoneRef {
            repo: "senara-solutions/mika".into(),
            number: 33,
        };
        let state =
            compose_from_gh_outputs(&r, milestone_json, FIXTURE_MILESTONE_33, "[]").unwrap();
        let out = Reporter::new().report(&state, &empty_assessment());

        // Contrôle positif : #2886, fermé par la PR #2892 mergée le
        // 2026-10-23. L'URL est celle que la référence côté issue porte.
        let ligne_2886 = out
            .lines()
            .find(|l| l.contains("#2886"))
            .expect("la ligne de #2886 est dans la section Completed");
        assert!(
            ligne_2886.ends_with(" — PR https://github.com/senara-solutions/mika/pull/2892"),
            "la ligne de #2886 doit finir par le lien de sa PR de clôture, \
             or elle dit : {ligne_2886}"
        );

        // Contrôle négatif : #1680, fermé à la main, aucune référence de
        // clôture. La chaîne est celle que la sonde AC3 compte sur le rapport
        // du sink — `grep -c 'closed (no linked PR)'`, 87 → 14 attendu : elle
        // est assert mot pour mot, parce que la changer retirerait
        // l'instrument en silence.
        let ligne_1680 = out
            .lines()
            .find(|l| l.contains("#1680"))
            .expect("la ligne de #1680 est dans la section Completed");
        assert!(
            ligne_1680.ends_with(" — closed (no linked PR)"),
            "la ligne de #1680 doit finir par `closed (no linked PR)`, \
             or elle dit : {ligne_1680}"
        );

        // Et le rapport ne dit `no linked PR` qu'une seule fois : si la
        // résolution régressait, les deux lignes se remettraient à le dire, et
        // les deux assertions ci-dessus laisseraient passer celle de #1680.
        assert_eq!(
            out.matches("closed (no linked PR)").count(),
            1,
            "un seul des deux tickets fermés est sans PR :\n{out}"
        );
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
            missing_grooming_signals: None,
            number: 1700,
            state: IssueState::Open,
            priority_rank: Some(0),
            plan_present: true,
            plan_reading: None,
            dormancy: None,
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
            missing_grooming_signals: None,
            number: 1700,
            state: IssueState::Open,
            priority_rank: None,
            plan_present: false,
            plan_reading: None,
            dormancy: None,
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
            missing_grooming_signals: None,
            number: 1700,
            state: IssueState::Open,
            priority_rank: Some(0),
            plan_present: true,
            plan_reading: None,
            dormancy: None,
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
            missing_grooming_signals: None,
            number: 1700,
            state: IssueState::Open,
            priority_rank: Some(0),
            plan_present: true,
            plan_reading: None,
            dormancy: None,
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
            missing_grooming_signals: None,
            number: 1700,
            state: IssueState::Open,
            priority_rank: Some(0),
            plan_present: true,
            plan_reading: None,
            dormancy: None,
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

    // -- mika#2969 : la lecture du milestone se dit, complète ou tronquée -----

    /// **T4** — une lecture du milestone tronquée se déclare, et les tickets
    /// déjà lus restent **au rapport**.
    ///
    /// La deuxième moitié porte la décision du vol. Sur une page pleine,
    /// `outside_candidates` est vidée (mika#2699) ; `sub_issues` ne l'est pas,
    /// et ce n'est pas une inconséquence. La lecture du milestone est une
    /// **énumération** : partielle, elle informe encore — les 1000 tickets lus
    /// sont vrais. La lecture hors milestone est un **classement** : partiel, il
    /// répond faux, parce qu'il présenterait le meilleur de la page comme le
    /// meilleur du dépôt. On rapporte, on ne refuse pas.
    ///
    /// La ligne nomme aussi les trois surfaces que la troncature atteint —
    /// rapport, progression, bassin : c'est par là que mika#2969 s'est vu, un
    /// #1727 ouvert du #33 rapporté « hors milestone ».
    #[test]
    fn mika2969_une_lecture_du_milestone_tronquee_se_declare_sans_jeter_le_lu() {
        let subs = vec![base_issue(1721), base_issue(1727)];
        let mut state = state_with(
            subs,
            ProgressCounts {
                total: 2,
                unstarted: 2,
                ..Default::default()
            },
        );
        state.milestone_read_audit = Some(MilestoneReadAudit {
            returned: 1000,
            limit: 1000,
            api_item_total: 1200,
        });
        let out = Reporter::new().report(&state, &empty_assessment());

        assert!(
            out.contains(
                "- Lecture du milestone TRONQUÉE à 1000 tickets — des tickets du #1799 \
                 sont hors de ce rapport, de sa progression et de son bassin \
                 (API du milestone : 1200 items, PR comprises)"
            ),
            "la troncature se dit, avec la borne demandée et ce qu'elle emporte :\n{out}"
        );
        assert!(
            out.contains("#1721") && out.contains("#1727"),
            "une énumération tronquée reste une énumération : ce qui a été lu se \
             rapporte :\n{out}"
        );

        // Page non pleine : la lecture se dit **complète**, et c'est ce mot-là
        // que l'opérateur lit — jamais l'égalité des deux comptes, qui ne peut
        // pas tomber juste (classe mika#2205 : l'API compte les PR aussi).
        state.milestone_read_audit = Some(MilestoneReadAudit {
            returned: 117,
            limit: 1000,
            api_item_total: 192,
        });
        let complete = Reporter::new().report(&state, &empty_assessment());
        assert!(
            complete.contains(
                "- Lecture du milestone : 117 tickets lus, borne 1000 — complète \
                 (API du milestone : 192 items, PR comprises — autre population, mika#2969)"
            ),
            "une lecture complète dit son compte, sa borne et le mot « complète » :\n{complete}"
        );
        assert!(
            !complete.contains("TRONQUÉE"),
            "117 pour une borne de 1000 n'est pas une troncature :\n{complete}"
        );
    }

    /// **T5** — un audit absent ne fait dire **rien**.
    ///
    /// `None` veut dire « la question n'a pas été posée » : blob du puits
    /// hors-ligne antérieur au champ, ou binaire antérieur à ce vol (mika#2331,
    /// mika#2340). Ni « complète » ni « tronquée » — les deux affirmeraient une
    /// mesure qui n'a pas eu lieu, et « complète » serait l'affirmation
    /// dangereuse : c'est précisément le défaut que ce vol répare.
    #[test]
    fn mika2969_un_audit_absent_ne_fait_dire_ni_complete_ni_tronquee() {
        let state = state_with(vec![], ProgressCounts::default());
        assert!(
            state.milestone_read_audit.is_none(),
            "le helper doit partir d'un audit non posé"
        );
        let out = Reporter::new().report(&state, &empty_assessment());
        assert!(
            !out.contains("Lecture du milestone"),
            "sans audit, le rapport se tait plutôt que de dire faux :\n{out}"
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
                missing_grooming_signals: None,
                number: 2740,
                state: IssueState::Open,
                priority_rank: Some(0),
                plan_present: false,
                plan_reading: None,
                dormancy: None,
                labels: vec!["p0-critical".into()],
            },
            PoolCandidate {
                missing_grooming_signals: None,
                number: 2750,
                state: IssueState::Open,
                priority_rank: None,
                plan_present: false,
                plan_reading: None,
                dormancy: None,
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
            missing_grooming_signals: None,
            number: 1806,
            state: IssueState::Open,
            priority_rank: Some(1),
            plan_present: true,
            plan_reading: None,
            dormancy: None,
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

    // -- mika#2968 : le rapport dit la CI sur laquelle il s'appuie (AC3) --

    /// AC3 — les huit libellés, et l'instant de lecture avec eux.
    ///
    /// Les quatre états **lus** portent « lue au cycle de <instant> », et non
    /// « lue à <instant> » : l'instant est `ctx.now`, capturé après le
    /// `reader.read()` du cycle, donc il majore la lecture. Les quatre causes de
    /// non-lecture sont nommées séparément parce qu'elles n'appellent pas la
    /// même action — « PR hors liste du milestone » est le trou amont du
    /// ticket, « non mesurée » est un rapport antérieur au champ.
    #[test]
    fn mika2968_ac3_le_rapport_nomme_letat_ci_et_linstant_de_la_lecture() {
        let au_cycle_de = "2026-10-29T17:46:12Z";
        let rendu = |ci: CiKnowledge| {
            let mut v = verdict(
                Some(2963),
                FlightIssue::PrOpened {
                    number: Some(2966),
                    url: None,
                },
                Some(83),
                false,
                FlightRecommendation::MergeCandidate,
            );
            v.ci = ci;
            render_flight_verdict(&v)
        };
        let lue = |etat: CiState| CiKnowledge::Read {
            state: etat,
            at: au_cycle_de.to_string(),
        };

        for (ci, attendu) in [
            (
                lue(CiState::Success),
                "CI verte (lue au cycle de 2026-10-29T17:46:12Z)",
            ),
            (
                lue(CiState::Failed),
                "CI rouge (lue au cycle de 2026-10-29T17:46:12Z)",
            ),
            (
                lue(CiState::Pending),
                "CI en cours (lue au cycle de 2026-10-29T17:46:12Z)",
            ),
            (
                lue(CiState::None),
                "CI sans check (lue au cycle de 2026-10-29T17:46:12Z)",
            ),
            (
                CiKnowledge::Unread(CiUnreadCause::NotFromPrList),
                "CI non lue (PR hors liste du milestone)",
            ),
            (
                CiKnowledge::Unread(CiUnreadCause::NoSubIssue),
                "CI non lue (aucune sous-issue appariée)",
            ),
            (
                CiKnowledge::Unread(CiUnreadCause::NoIssueNumber),
                "CI non lue (vol sans ticket)",
            ),
            (
                CiKnowledge::Unread(CiUnreadCause::NotMeasured),
                "CI non lue (non mesurée)",
            ),
        ] {
            let ligne = rendu(ci);
            assert!(
                ligne.contains(attendu),
                "la ligne du vol doit porter « {attendu} » :\n{ligne}"
            );
        }
    }

    /// « Sans check » n'est pas « non lue », et le rendu ne les confond pas.
    ///
    /// C'est la frontière que l'AC3 nomme : une lecture qui a abouti et n'a
    /// trouvé aucun check est un fait sur la PR ; une non-lecture est un fait
    /// sur l'instrument. Les rendre par les mêmes octets ferait des deux
    /// populations une seule (classe mika#2205) — et puisque « CI sans check »
    /// ne s'oppose pas au merge, la confusion irait dans le sens où une CI non
    /// lue passerait pour mergeable sans réserve.
    #[test]
    fn mika2968_une_ci_sans_check_ne_se_rend_jamais_comme_non_lue() {
        let mut v = verdict(
            Some(2963),
            FlightIssue::PrOpened {
                number: Some(2966),
                url: None,
            },
            Some(83),
            false,
            FlightRecommendation::MergeCandidate,
        );
        v.ci = CiKnowledge::Read {
            state: CiState::None,
            at: "2026-10-29T17:46:12Z".into(),
        };
        let ligne = render_flight_verdict(&v);
        assert!(
            !ligne.contains("CI non lue"),
            "une lecture aboutie ne se rend jamais comme non lue :\n{ligne}"
        );
        assert!(ligne.contains("CI sans check"), "{ligne}");
    }

    /// Le segment CI ne s'omet **jamais**, sur aucune des quatre issues.
    ///
    /// L'omettre pour une non-lecture ferait de « rien n'a été lu » et de « ce
    /// rapport est antérieur au champ » les mêmes octets, et le silence serait
    /// alors le seul endroit où une CI non lue pourrait passer pour verte.
    #[test]
    fn mika2968_le_segment_ci_ne_somet_sur_aucune_issue() {
        for issue in [
            FlightIssue::PrOpened {
                number: Some(2966),
                url: None,
            },
            FlightIssue::RescuePr {
                number: Some(2690),
                url: None,
            },
            FlightIssue::Dead {
                cause: DeathCause::CeilingReached,
            },
            FlightIssue::NotAFlight {
                reason: NotFlightReason::GroomCompleted,
            },
        ] {
            let ligne = render_flight_verdict(&verdict(
                Some(2963),
                issue.clone(),
                Some(83),
                false,
                FlightRecommendation::Nothing,
            ));
            assert!(
                ligne.contains("CI non lue (non mesurée)"),
                "le segment CI est le seul qui ne s'omette jamais :\n{ligne}"
            );
        }
    }

    // -- mika#2968, vol 2a : les causes de la lecture par numéro (AC3 étendu) --

    /// AC3 — les six causes de non-lecture du vol 2a, chacune sous son libellé.
    ///
    /// Six libellés **distincts** parce que les six causes n'appellent pas la
    /// même action : « lecture gh en échec » envoie regarder le jeton du dépôt
    /// (mika#2963), « PR hors de la page lue » la borne de 100, « ce vol n'a pas
    /// de PR » rien du tout. Deux causes sous les mêmes octets referaient une
    /// seule population (classe mika#2205), donc la distinction est assertée
    /// aussi, pas seulement la présence.
    #[test]
    fn mika2968_ac3_les_six_causes_de_la_lecture_par_numero_se_disent() {
        let attendus = [
            (
                CiUnreadCause::NoPrNumber,
                "CI non lue (ce vol n'a pas de PR)",
            ),
            (CiUnreadCause::NoRepo, "CI non lue (vol sans dépôt)"),
            (
                CiUnreadCause::NoRunner,
                "CI non lue (aucun lecteur gh câblé)",
            ),
            (
                CiUnreadCause::ReadFailed,
                "CI non lue (lecture gh en échec)",
            ),
            (
                CiUnreadCause::NotFound,
                "CI non lue (PR hors de la page lue)",
            ),
            (CiUnreadCause::PrNotOpen, "CI non lue (PR non ouverte)"),
        ];
        for (cause, attendu) in attendus {
            let mut v = verdict(
                Some(2968),
                FlightIssue::PrOpened {
                    number: Some(2977),
                    url: None,
                },
                Some(83),
                false,
                FlightRecommendation::MergeCandidate,
            );
            v.ci = CiKnowledge::Unread(cause);
            let ligne = render_flight_verdict(&v);
            assert!(
                ligne.contains(attendu),
                "la ligne du vol doit porter « {attendu} » :\n{ligne}"
            );
        }
        let distincts: std::collections::HashSet<&str> = attendus
            .iter()
            .map(|(cause, _)| ci_unread_label(*cause))
            .collect();
        assert_eq!(
            distincts.len(),
            attendus.len(),
            "deux causes sous le même libellé se confondraient au rapport"
        );
    }

    /// AC3 — les trois appartenances au milestone, lues sur leur propre champ.
    ///
    /// Le `pr_state` est tenu **constant** et lu dans les trois cas : c'est ce
    /// qui prouve que le libellé d'appartenance ne le lit plus. Sous l'ancienne
    /// déduction, un `Read(_)` aurait rendu « milestone cible » trois fois.
    #[test]
    fn mika2968_ac3_les_trois_appartenances_se_disent_sans_lire_letat_de_pr() {
        for (appartenance, attendu) in [
            (MilestoneMembership::InTarget, " — milestone cible — "),
            (
                MilestoneMembership::OutOfTarget,
                " — hors milestone cible — ",
            ),
            (
                MilestoneMembership::NotRead,
                " — appartenance au milestone non lue — ",
            ),
        ] {
            let mut v = verdict(
                Some(2968),
                FlightIssue::PrOpened {
                    number: Some(2977),
                    url: None,
                },
                Some(83),
                false,
                FlightRecommendation::MergeCandidate,
            );
            v.pr_state = PrStateKnowledge::Read(Some("OPEN".into()));
            v.in_target_milestone = appartenance;
            let ligne = render_flight_verdict(&v);
            assert!(
                ligne.contains(attendu),
                "la ligne du vol doit porter « {attendu} » :\n{ligne}"
            );
            assert!(
                ligne.contains(" — PR OPEN — "),
                "l'état de PR reste rendu, indépendamment :\n{ligne}"
            );
        }
    }
}

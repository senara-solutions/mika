//! Types for the mika-manager Phase 1 milestone-scope coordinator.
//!
//! LECTURE SEULE — no write authority anywhere. All types are read-only
//! composers over `gh` CLI output. See the module docstring in `mod.rs`
//! for the full Phase 1 discipline.

use super::pool::{DormancyReading, PlanSizeReading, PoolCandidate, PoolVerdict, ReadyTicket};
use crate::auto_pull::GroomingSignal;
use crate::plan_callout::PlanReading;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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

/// D'où vient le lien entre un ticket et la PR qui le ferme (mika#2924).
///
/// Il y a deux chemins, et ils n'ont pas les mêmes garanties :
///
/// - [`Self::MilestonePrList`] — l'index inverse bâti sur
///   `gh pr list --search milestone:N` et `closingIssuesReferences`. Ce chemin
///   est le **seul** à porter `pr_state` et `ci_state` : il a vu la PR.
/// - [`Self::IssueClosingRef`] — la référence que GitHub tient *côté issue*
///   (`closedByPullRequestsReferences`). Elle répond pour les PR de la boucle
///   autonome, qui rattache le milestone au **ticket** et jamais à la PR, donc
///   que la liste ci-dessus ne voit pas. Elle ne dit rien de l'état de la PR.
///
/// **Pourquoi un champ, et pas une déduction.** Sans lui, « aucune PR » et « une
/// PR que la liste du milestone ne voit pas » rendent les mêmes octets une fois
/// le correctif posé (classe mika#2205 : deux populations rendant les mêmes
/// octets), et la sonde AC3 ne peut pas distinguer les 14 tickets fermés à la
/// main des tickets que le correctif vient de réparer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrLinkSource {
    /// L'index inverse des PR du milestone — porte `pr_state` et `ci_state`.
    MilestonePrList,
    /// La référence de clôture côté issue — ne porte que numéro et URL.
    IssueClosingRef,
}

/// Ce que la timeline REST d'un ticket **ouvert** a dit de ses PR (mika#2924,
/// vol 2).
///
/// Quatre variantes et non deux : « aucune PR mergée ne cite ce ticket » et
/// « la lecture n'a pas conclu » ont des remèdes opposés, et les confondre est
/// la classe mika#2205. Les deux causes d'indétermination sont distinctes pour
/// la même raison — une page pleine se corrige en paginant, un échec de lecture
/// se corrige à la forge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergedPrReading {
    /// Des PR mergées citent ce ticket, triées par `merged_at` **décroissant**.
    Citing(Vec<u64>),
    /// La timeline a été lue **en entier** et aucune PR mergée ne cite ce ticket.
    NoneCiting,
    /// La page est pleine : la timeline est possiblement tronquée, donc
    /// l'absence de PR n'est **pas** attestée (mika#2930 AC2).
    IndeterminatePageFull,
    /// L'appel `gh` a échoué. Un échec ne refuse pas le cycle et ne se tait pas.
    IndeterminateFetchFailed,
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
    /// Ce que le corps dit de son plan, en trois états (mika#2952).
    ///
    /// À côté de [`Self::plan_present`] et non à sa place : ce booléen est la
    /// tolérance **lâche** que mika#2608 a figée, et quatre consommateurs en
    /// dépendent. Cette lecture-ci sépare les deux populations que le booléen
    /// confondait — un callout vivant, et la ligne `Plan antérieur :` qu'écrit
    /// la livraison d'un vol.
    ///
    /// `None` veut dire **« le corps n'a pas été lu par ce binaire »**, jamais
    /// « aucun plan » : c'est le motif de [`Self::merged_prs`] et de
    /// [`Self::pr_link_source`] (mika#2340), et la raison pour laquelle le puits
    /// hors ligne relit ses blobs anciens sans changer de verdict.
    #[serde(default)]
    pub plan_reading: Option<PlanReading>,
    /// Ce que le corps dit de sa **dormance** (mika#2952, vol 2/2).
    ///
    /// Un axe orthogonal à [`Self::plan_reading`] et non un état de plus sur
    /// celui-ci : un ticket dormant peut porter n'importe laquelle des trois
    /// lectures de plan — #2854 porte les deux à la fois — et deux populations
    /// sous un seul nom ne sont pas soustrayables (mika#2205).
    ///
    /// `None` veut dire **« le corps n'a pas été lu par ce binaire »**, jamais
    /// « éveillé » (motif mika#2340). Un blob du puits hors ligne écrit avant ce
    /// vol relit donc sans dormance et garde son verdict d'origine, au lieu
    /// d'hériter d'un éveil que personne n'a mesuré.
    #[serde(default)]
    pub dormancy: Option<DormancyReading>,
    /// Lesquels des trois signaux de grooming manquent au corps (mika#2956).
    ///
    /// La face **mesurée** du prédicat strict `auto_pull::is_groomed`, celui que
    /// le gate du dispatch accepte par construction. À côté de
    /// [`Self::plan_present`] et non à sa place : ce booléen est le lecteur
    /// **lâche** de mika#2608, qui rend vrai pour toute ligne de citation
    /// contenant `docs/plans/` — y compris une ligne de prose qui *cite* un plan
    /// sans en porter le callout.
    ///
    /// `None` veut dire **« le corps n'a pas été lu par ce binaire »**, jamais
    /// « rien ne manque » (motif mika#2340, comme [`Self::plan_reading`] et
    /// [`Self::dormancy`]). `Some(vec![])` est la mesure « les trois signaux sont
    /// là » ; un `Some` non vide nomme ce qui manque, avec les mots du dispatch.
    #[serde(default)]
    pub missing_grooming_signals: Option<Vec<GroomingSignal>>,
    /// True if the issue body carries a `> - **Branch:**` callout.
    pub branch_present: bool,
    /// PR number linked via GitHub `closingIssuesReferences` (first hit wins).
    pub pr_number: Option<u64>,
    /// PR state: `open`, `closed`, `merged` — plain string from `gh pr view`.
    pub pr_state: Option<String>,
    /// L'URL de la PR de clôture **telle que GitHub la donne** (mika#2924),
    /// quand le lien vient de la référence côté issue.
    ///
    /// `None` sur le chemin liste-du-milestone, où le dépôt est connu par
    /// construction et l'URL recomposée depuis `{repo}/pull/{pr_number}`. Sur le
    /// chemin côté issue la recomposition est **refusée** : la référence peut
    /// porter une PR d'un autre dépôt, et un lien recomposé pointerait alors sur
    /// une PR qui n'existe pas.
    ///
    /// `#[serde(default)]` pour la raison de `outside_read_truncated` — le puits
    /// hors-ligne tient des blobs écrits avant ce champ.
    #[serde(default)]
    pub pr_url: Option<String>,
    /// D'où vient [`Self::pr_number`] (mika#2924).
    ///
    /// Posé par les **deux** chemins de résolution, pour que `None` dise
    /// « sérialisé par un binaire antérieur au correctif » et jamais
    /// « liste du milestone » (mika#2340 : un signal absent veut dire un binaire
    /// plus vieux, pas une valeur par défaut). Voir [`PrLinkSource`] pour ce que
    /// chaque chemin garantit.
    #[serde(default)]
    pub pr_link_source: Option<PrLinkSource>,
    /// Ce que la sonde de timeline a dit des PR de ce ticket (mika#2924,
    /// vol 2).
    ///
    /// `None` signifie **la sonde n'a pas été émise** — palier 1 vainqueur,
    /// ticket non désigné, ou blob d'un binaire antérieur au vol
    /// (motif mika#2340) — et jamais « aucune PR ». C'est pourquoi `None` rend
    /// le libellé **historique** côté `Assessor` : un binaire antérieur garde
    /// son texte, il n'hérite pas d'une incertitude qu'il n'a pas mesurée.
    #[serde(default)]
    pub merged_prs: Option<MergedPrReading>,
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
    /// Le bassin `ready` à l'échelle du **dépôt**, pas du milestone (mika#2699).
    ///
    /// Composé par [`super::reader::compose_ready_pool`], pas par
    /// `compose_from_gh_outputs` : le webhook `ready` dispatche sur tout le
    /// dépôt, donc le bassin n'est pas une propriété du milestone cible et ne
    /// sort pas des mêmes appels `gh`. `#[serde(default)]` pour la même raison
    /// que `Assessment::contention_events` — le puits hors-ligne tient des blobs
    /// écrits avant ce champ.
    #[serde(default)]
    pub ready_pool: Vec<ReadyTicket>,
    /// Les candidats au bassin qui **n'appartiennent pas** au milestone cible
    /// (mika#2699 vol 2) — le « avant le reste » du point 1 suppose qu'il y ait
    /// un reste.
    ///
    /// Lus par une cinquième lecture `gh` dépôt-large, pour la même raison que
    /// [`Self::ready_pool`] : le bassin se tient à l'échelle du dépôt. Les
    /// `sub_issues` du milestone cible en sont **retirées** à la composition —
    /// elles arrivent déjà par [`Self::sub_issues`], et les transporter deux
    /// fois alourdirait chaque blob du puits hors-ligne sans rien ajouter.
    /// `assess_pool` déduplique quand même : c'est là que la correction se
    /// décide, et une fonction pure ne doit pas dépendre de la prudence de son
    /// appelant.
    #[serde(default)]
    pub outside_candidates: Vec<PoolCandidate>,
    /// La lecture ci-dessus a-t-elle rempli sa page, donc potentiellement
    /// laissé des candidats dehors ?
    ///
    /// `Option`, et non `bool` : `None` dit « la question n'a pas été posée »
    /// (blob antérieur au champ, ou cycle qui n'a pas lu hors milestone), là où
    /// `Some(false)` dit « posée, et la page n'était pas pleine ». Le motif est
    /// celui de mika#2331 — un `false` par défaut affirmerait une mesure qui
    /// n'a pas eu lieu.
    ///
    /// Sur `Some(true)`, [`Self::outside_candidates`] est **vide par
    /// décision** : le motif de `scripts/pr-origin-report.sh` est de refuser
    /// plutôt que de mal répondre, et un classement fait sur une page tronquée
    /// recommanderait le meilleur des cent premiers en le présentant comme le
    /// meilleur du dépôt. Le rapport le dit au lieu de le taire.
    #[serde(default)]
    pub outside_read_truncated: Option<bool>,
    /// La taille annoncée par le plan de chaque candidat pressenti, lue à la
    /// forge au ref de **sa** branche (mika#2699 vol 3).
    ///
    /// Indexée par numéro d'issue, et **creuse par construction** : seuls les
    /// candidats que [`super::pool::numbers_needing_size`] a désignés sont lus,
    /// au plus [`super::pool::PLAN_SIZE_READ_MAX`] par cycle. Un candidat absent
    /// de la table n'est pas « de taille nulle » — il n'a pas été mesuré, et
    /// [`super::pool::UnreadCause::NotMeasured`] est ce que l'évaluation lui
    /// attribue. Le motif est celui de mika#2331 : un défaut à `0` affirmerait
    /// une mesure qui n'a pas eu lieu, et ici il admettrait le ticket que la
    /// borne devait refuser.
    ///
    /// `BTreeMap` et non `HashMap` : le blob du puits hors-ligne se relit en
    /// diff, donc son ordre de sérialisation doit être le même d'un cycle à
    /// l'autre.
    #[serde(default)]
    pub plan_sizes: BTreeMap<u64, PlanSizeReading>,
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

/// One dispatch the manager wanted to make and could not, because another
/// dispatcher held the exec slot (mika#1948, Porte 2).
///
/// Exists so contention is *reportable*. A manager whose dispatch silently
/// defers looks identical to a manager with nothing to do; the operator then
/// sees a throughput bottleneck with no name on it. Phase 1 never produces
/// these — the manager is read-only — but the channel ships now so Phase 2 has
/// a landing spot that is already rendered and already tested.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentionEvent {
    /// The exec-slot class that was busy: `"implement"` or `"groom"`.
    pub dispatch_class: String,
    /// The task holding the slot.
    pub blocking_task_id: String,
    /// Which dispatcher initiated the blocking task. `None` means the row
    /// predates the column (pre-v51) and is genuinely unknown — deliberately
    /// not defaulted, so a reader cannot mistake it for a positive attribution.
    pub blocking_dispatcher_source: Option<String>,
    /// Rejection reason as the dispatch path reported it.
    pub reason: String,
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
    /// Dispatch contention observed during this cycle (mika#1948).
    /// Phase 1: always empty — the manager does not dispatch yet.
    ///
    /// `#[serde(default)]` is load-bearing, not decoration. It buys two things:
    /// a report blob written before this field existed still deserializes (the
    /// offline sink holds such files), and an empty list serializes as `[]`
    /// rather than `null`, so a Phase 2 consumer can type it `Vec<_>` instead of
    /// `Option<Vec<_>>`.
    #[serde(default)]
    pub contention_events: Vec<ContentionEvent>,
    /// Le verdict du bassin `ready` pour ce cycle (mika#2699 Phase 2 (a) vol 1).
    ///
    /// `Option`, et pas un `PoolVerdict` nu : `None` dit « le bassin n'a pas été
    /// lu » — blob écrit par un binaire antérieur au champ. Un `PoolVerdict` par
    /// défaut dirait « bassin vide, famine », ce qui est une *mesure*, pas une
    /// absence de mesure (motif mika#2331 : un null n'est jamais un 0). Un cycle
    /// dont la lecture du bassin échoue n'arrive jamais ici — l'erreur remonte
    /// et il n'y a pas d'`Assessment`.
    ///
    /// **Vol 1 : rien de ce verdict n'est écrit sur la forge.** Il entre dans le
    /// rapport, et nulle part ailleurs.
    #[serde(default)]
    pub pool: Option<PoolVerdict>,
    /// Verdicts des vols terminés observés pendant ce cycle (mika#2698).
    ///
    /// **Vol 1 : mode ombre — ce champ est la seule destination du verdict.**
    /// Rien d'autre ne le consomme, et aucun chemin d'écriture ne part de lui.
    /// Vide quand aucune [`super::evaluator::FlightSource`] n'est câblée, ce qui
    /// rend le rapport identique à celui d'avant.
    ///
    /// `#[serde(default)]` pour les deux mêmes raisons que
    /// `contention_events` : un blob de rapport écrit avant ce champ se
    /// désérialise encore (le sink disque en garde), et la liste vide sort en
    /// `[]` plutôt qu'en `null`.
    #[serde(default)]
    pub flight_verdicts: Vec<super::evaluator::FlightVerdict>,
    /// La relève nommée de ce cycle — les prochains tickets à grommer derrière
    /// le bassin (mika#2700 Phase 2 (c) vol 1).
    ///
    /// `Option` pour la même raison que `pool`, dont elle dérive : `None` dit
    /// « la relève n'a pas été nommée » — blob écrit par un binaire antérieur
    /// au champ. Une [`super::relief::ReliefVerdict`] par défaut dirait « rien
    /// à grommer d'avance », ce qui est une *mesure* et non son absence (motif
    /// mika#2331 : un null n'est jamais un 0).
    ///
    /// **Vol 1 : mode ombre.** Ce champ est la seule destination de la relève ;
    /// elle entre dans le rapport et nulle part ailleurs. Aucun interrupteur,
    /// aucun geste, aucun chemin d'écriture ne part de lui.
    #[serde(default)]
    pub relief: Option<super::relief::ReliefVerdict>,
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
    /// The auth-boundary failure this cycle observed on its delivery attempt,
    /// if any (mika#1949 U3).
    ///
    /// `#[serde(default)]` because the offline sink holds outcome blobs
    /// written before this field existed, and an upgrade that could not read
    /// its own backlog would lose it — same reasoning as
    /// `Assessment::contention_events`.
    #[serde(default)]
    pub auth_boundary: Option<mika_common::auth_boundary::AuthBoundaryError>,
    /// Whether this cycle actually presented a credential at a boundary
    /// (mika#1949 U4).
    ///
    /// Needed because `auth_boundary: None` conflates three different things:
    /// a delivery that authenticated, a cycle that wrote to the offline sink,
    /// and a no-op cycle that delivered nothing at all. Only the first is
    /// evidence the credential works, and only evidence should close an
    /// escalation window. With `poll_interval` at 5 min against a 6 h
    /// heartbeat, no-op cycles are the *common* case — reading them as
    /// success would reset the repeat counter between almost every pair of
    /// real failures and make the `Blocked` escalation unreachable.
    #[serde(default)]
    pub auth_attempted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- mika#1948: contention_events wire format --

    fn assessment_json_without_contention() -> String {
        serde_json::json!({
            "severity": "healthy",
            "recommendation": { "next_sub_issue": null, "rationale": "nothing to do" },
            "alerts": [],
            "cross_cutting": []
        })
        .to_string()
    }

    /// A report blob written before `contention_events` existed must still
    /// deserialize. The offline sink holds such files, and an upgrade that
    /// could not read its own backlog would lose it.
    #[test]
    fn test_assessment_serde_default_empty_contention() {
        let a: Assessment = serde_json::from_str(&assessment_json_without_contention())
            .expect("pre-Porte-2 JSON must still parse after the field is added");
        assert!(
            a.contention_events.is_empty(),
            "a missing field must read as an empty list, not fail"
        );
    }

    /// And the field must serialize as `[]`, never `null`, so a Phase 2 consumer
    /// can type it `Vec<_>` rather than `Option<Vec<_>>`.
    #[test]
    fn test_assessment_serde_roundtrip_empty_contention() {
        let a = Assessment {
            severity: Severity::Healthy,
            recommendation: Recommendation {
                next_sub_issue: None,
                rationale: "nothing to do".into(),
            },
            alerts: vec![],
            cross_cutting: vec![],
            contention_events: vec![],
            pool: None,
            flight_verdicts: vec![],
            relief: None,
        };
        let out = serde_json::to_string(&a).unwrap();
        assert!(
            out.contains(r#""contention_events":[]"#),
            "empty contention must serialize as [] — got: {out}"
        );
        assert!(
            !out.contains(r#""contention_events":null"#),
            "must never serialize as null"
        );
    }

    // -- mika#2699 Phase 2 (a) vol 1 : format fil du verdict de bassin --

    /// Cas 11 — un blob de rapport écrit avant que `pool` existe doit encore se
    /// désérialiser, et le champ doit valoir `None` : le puits hors-ligne tient
    /// de tels fichiers, et une montée de version incapable de relire son propre
    /// arriéré le perdrait.
    #[test]
    fn mika2699_un_blob_anterieur_se_relit_avec_pool_absent() {
        let a: Assessment = serde_json::from_str(&assessment_json_without_contention())
            .expect("un JSON antérieur au champ doit encore se parser");
        assert_eq!(
            a.pool, None,
            "champ absent ⇒ None, et surtout pas un verdict par défaut"
        );
    }

    /// Cas 12 — `None` sérialise en `null`, **jamais** en objet vide : « le
    /// bassin n'a pas été lu » et « le bassin est vide » sont deux faits
    /// différents, et un `{}` les confondrait (motif mika#2331). Aller-retour
    /// pour que le puits reste relisible par lui-même.
    #[test]
    fn mika2699_pool_absent_serialise_en_null_jamais_en_objet_vide() {
        let a = Assessment {
            severity: Severity::Healthy,
            recommendation: Recommendation {
                next_sub_issue: None,
                rationale: "rien à faire".into(),
            },
            alerts: vec![],
            cross_cutting: vec![],
            contention_events: vec![],
            pool: None,
            flight_verdicts: vec![],
            relief: None,
        };
        let out = serde_json::to_string(&a).unwrap();
        assert!(
            out.contains(r#""pool":null"#),
            "un bassin non lu se dit null — obtenu : {out}"
        );
        assert!(
            !out.contains(r#""pool":{}"#),
            "jamais un objet vide — obtenu : {out}"
        );
        let back: Assessment = serde_json::from_str(&out).unwrap();
        assert_eq!(back, a, "aller-retour fidèle");
    }

    // -- mika#2698: flight_verdicts, même contrat de fil --

    /// Un blob de rapport écrit avant `flight_verdicts` doit encore se
    /// désérialiser — le sink disque en garde, et une montée de version qui ne
    /// sait plus lire son propre arriéré le perd.
    #[test]
    fn mika2698_assessment_serde_default_empty_flight_verdicts() {
        let a: Assessment = serde_json::from_str(&assessment_json_without_contention())
            .expect("un JSON d'avant le champ doit encore se lire");
        assert!(
            a.flight_verdicts.is_empty(),
            "un champ absent se lit comme une liste vide, il n'échoue pas"
        );
    }

    /// Et la liste vide sort en `[]`, jamais en `null` : un consommateur du
    /// vol 2 doit pouvoir la typer `Vec<_>` et non `Option<Vec<_>>`.
    #[test]
    fn mika2698_assessment_serde_roundtrip_empty_flight_verdicts() {
        let a = Assessment {
            severity: Severity::Healthy,
            recommendation: Recommendation {
                next_sub_issue: None,
                rationale: "nothing to do".into(),
            },
            alerts: vec![],
            cross_cutting: vec![],
            contention_events: vec![],
            flight_verdicts: vec![],
            pool: None,
            relief: None,
        };
        let out = serde_json::to_string(&a).unwrap();
        assert!(
            out.contains(r#""flight_verdicts":[]"#),
            "une liste vide doit sortir en [] — obtenu : {out}"
        );
        assert!(!out.contains(r#""flight_verdicts":null"#), "jamais null");
    }

    /// Et un verdict non vide fait l'aller-retour sans perdre les deux data que
    /// le vol 1 laisse non instrumentées.
    #[test]
    fn mika2698_un_verdict_fait_laller_retour() {
        use super::super::evaluator::{
            DeathCause, FlightIssue, FlightRecommendation, FlightVerdict, PrStateKnowledge,
        };
        let v = FlightVerdict {
            task_id: "t".into(),
            repo: Some("senara-solutions/mika".into()),
            issue_number: Some(2661),
            issue: FlightIssue::Dead {
                cause: DeathCause::CeilingReached,
            },
            turns_used: Some(200),
            hit_ceiling: true,
            turn_ceiling: None,
            last_commit_turn: None,
            pr_state: PrStateKnowledge::NotRead,
            recommendation: FlightRecommendation::Regroom,
            completed_at: Some("2026-10-13T10:00:00Z".into()),
        };
        let out = serde_json::to_string(&v).unwrap();
        assert_eq!(serde_json::from_str::<FlightVerdict>(&out).unwrap(), v);
        assert!(
            out.contains(r#""turn_ceiling":null"#),
            "le plafond non instrumenté doit rester visible comme null — obtenu : {out}"
        );
    }

    // -- mika#2921 : les deux champs neufs du verdict, et leur format fil --

    /// Un verdict écrit **avant** mika#2921 se relit, et ce qu'il n'a pas écrit
    /// se lit comme non lu — jamais comme lu-et-vide.
    ///
    /// Le puits hors-ligne garde de tels blobs. `pr_state` absent ⇒ `NotRead` :
    /// un blob qui ne porte aucune lecture ne doit pas se mettre à en attester
    /// une (motif mika#2331, classe mika#2205).
    #[test]
    fn mika2921_un_verdict_anterieur_se_relit_sans_depot_et_sans_lecture() {
        use super::super::evaluator::{FlightRecommendation, FlightVerdict, PrStateKnowledge};
        let avant = serde_json::json!({
            "task_id": "t",
            "issue_number": 2661,
            "issue": { "dead": { "cause": "ceiling_reached" } },
            "turns_used": 200,
            "hit_ceiling": true,
            "turn_ceiling": null,
            "last_commit_turn": null,
            "recommendation": "regroom",
            "completed_at": "2026-10-13T10:00:00Z"
        })
        .to_string();

        let v: FlightVerdict = serde_json::from_str(&avant)
            .expect("un JSON d'avant les deux champs doit encore se lire");
        assert_eq!(
            v.repo, None,
            "aucun dépôt nommé, et surtout pas une chaîne vide"
        );
        assert_eq!(
            v.pr_state,
            PrStateKnowledge::NotRead,
            "un blob muet n'a rien lu — `Read(None)` prétendrait le contraire"
        );
        assert_eq!(v.recommendation, FlightRecommendation::Regroom);
    }

    /// Et les trois états de connaissance font l'aller-retour en restant
    /// distinguables sur le fil.
    #[test]
    fn mika2921_les_trois_etats_de_connaissance_survivent_au_fil() {
        use super::super::evaluator::PrStateKnowledge;
        for etat in [
            PrStateKnowledge::Read(Some("open".into())),
            PrStateKnowledge::Read(None),
            PrStateKnowledge::NotRead,
        ] {
            let out = serde_json::to_string(&etat).unwrap();
            assert_eq!(
                serde_json::from_str::<PrStateKnowledge>(&out).unwrap(),
                etat,
                "aller-retour fidèle — obtenu : {out}"
            );
        }
        assert_ne!(
            serde_json::to_string(&PrStateKnowledge::Read(None)).unwrap(),
            serde_json::to_string(&PrStateKnowledge::NotRead).unwrap(),
            "« lu, et vide » et « pas lu » doivent différer sur le fil aussi"
        );
    }

    // -- mika#2700 Phase 2 (c) vol 1 : format fil de la relève --

    /// Cas 10 — un blob de rapport écrit avant que `relief` existe se relit, et
    /// le champ vaut `None`.
    ///
    /// Le puits hors-ligne tient de tels fichiers. Et l'aller-retour vérifie
    /// l'autre moitié du contrat : `None` sort en `null`, jamais en objet vide
    /// — « la relève n'a pas été nommée » et « il n'y a personne à grommer »
    /// sont deux faits différents, et un `{}` les confondrait (motif
    /// mika#2331 : un null n'est jamais un 0).
    #[test]
    fn mika2700_un_blob_sans_releve_se_relit_en_none() {
        let a: Assessment = serde_json::from_str(&assessment_json_without_contention())
            .expect("un JSON antérieur au champ doit encore se parser");
        assert_eq!(
            a.relief, None,
            "champ absent ⇒ None, et surtout pas une relève vide par défaut"
        );

        let out = serde_json::to_string(&a).unwrap();
        assert!(
            out.contains(r#""relief":null"#),
            "une relève non nommée se dit null — obtenu : {out}"
        );
        assert!(
            !out.contains(r#""relief":{}"#),
            "jamais un objet vide — obtenu : {out}"
        );

        // Et une relève **nommée** fait l'aller-retour sans perdre son rang
        // absent ni son compte des restants.
        let mut nommee = a.clone();
        nommee.relief = Some(super::super::relief::ReliefVerdict {
            next: vec![super::super::relief::ReliefTicket {
                number: 2731,
                origin: super::super::pool::CandidateOrigin::Outside,
                priority_rank: None,
                motive: super::super::pool::RefusalReason::NoGroomedPlan,
            }],
            beyond: 7,
        });
        let out = serde_json::to_string(&nommee).unwrap();
        assert_eq!(
            serde_json::from_str::<Assessment>(&out).unwrap(),
            nommee,
            "aller-retour fidèle : le puits doit rester relisible par lui-même"
        );
        assert!(
            out.contains(r#""priority_rank":null"#),
            "un rang inconnu reste visible comme null — obtenu : {out}"
        );
    }

    #[test]
    fn test_contention_event_roundtrips_with_unknown_source() {
        let e = ContentionEvent {
            dispatch_class: "implement".into(),
            blocking_task_id: "task-a".into(),
            blocking_dispatcher_source: None,
            reason: "slot held".into(),
        };
        let back: ContentionEvent =
            serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(
            back, e,
            "an unknown source must survive a roundtrip as None"
        );
    }

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
            missing_grooming_signals: None,
            number: 1,
            title: "t".into(),
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
            missing_grooming_signals: None,
            number: 1,
            title: "t".into(),
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

    // -- mika#2924 : la provenance du lien de PR, et son contrat de fil --

    /// **V6** — un `MilestoneState` écrit par un binaire antérieur à mika#2924
    /// se relit, et les deux champs neufs valent `None`.
    ///
    /// Le puits hors-ligne tient de tels blobs : une montée de version incapable
    /// de relire son propre arriéré le perdrait. Et `pr_link_source == None` doit
    /// rester lisible comme « sérialisé avant le correctif » — c'est le motif
    /// mika#2340, un signal absent veut dire un binaire plus vieux, jamais une
    /// valeur par défaut. Le ticket **porte un `pr_number`** dans ce blob, pour
    /// que l'assertion ne puisse pas passer par simple absence de lien.
    #[test]
    fn mika2924_un_blob_anterieur_se_relit_avec_provenance_absente() {
        let blob = r#"{
            "ref": {"repo": "senara-solutions/mika", "number": 33},
            "title": "m33",
            "description": "",
            "state": "open",
            "created_at": "2026-10-01T00:00:00Z",
            "due_on": null,
            "last_activity_at": null,
            "sub_issues": [{
                "number": 2886,
                "title": "t",
                "state": "closed",
                "priority_rank": null,
                "plan_present": true,
                "branch_present": true,
                "pr_number": 2892,
                "pr_state": "merged",
                "ci_state": "success",
                "blockers": [],
                "updated_at": "2026-10-23T00:00:00Z",
                "labels": []
            }],
            "progress": {"completed": 1, "in_flight": 0, "blocked": 0, "unstarted": 0, "total": 1},
            "recent_activity": [],
            "executor_healthy": null
        }"#;
        let st: MilestoneState =
            serde_json::from_str(blob).expect("un blob d'avant les champs doit encore se lire");
        let sub = &st.sub_issues[0];
        assert_eq!(
            sub.pr_number,
            Some(2892),
            "le lien préexistant est conservé"
        );
        assert_eq!(
            sub.pr_url, None,
            "champ absent ⇒ None, et surtout pas une URL recomposée"
        );
        assert_eq!(
            sub.pr_link_source, None,
            "champ absent ⇒ None, qui dit « binaire antérieur » et non \
             « liste du milestone » (mika#2340)"
        );
        // Vol 2 : le même blob est antérieur à la sonde de timeline. `None` doit
        // s'y lire « la sonde n'a pas été émise », et surtout pas
        // « aucune PR mergée ne cite ce ticket » — le libellé « Next » dépend de
        // cette distinction, et ce ticket-ci *a* des PR mergées.
        assert_eq!(
            sub.merged_prs, None,
            "champ absent ⇒ None, qui dit « sonde non émise » et non \
             NoneCiting (mika#2340)"
        );
    }

    /// `None` sort en `null`, jamais en chaîne vide ni en objet : « on ne sait
    /// pas d'où vient ce lien » et « il vient de la liste du milestone » sont
    /// deux faits différents, et les confondre est exactement ce que le champ
    /// existe pour empêcher (motif mika#2331).
    #[test]
    fn mika2924_une_provenance_absente_serialise_en_null() {
        let sub = SubIssue {
            missing_grooming_signals: None,
            number: 1,
            title: "t".into(),
            state: IssueState::Closed,
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
            updated_at: "2026-10-26T00:00:00Z".into(),
            labels: vec![],
        };
        let out = serde_json::to_string(&sub).unwrap();
        assert!(
            out.contains(r#""pr_url":null"#)
                && out.contains(r#""pr_link_source":null"#)
                && out.contains(r#""merged_prs":null"#),
            "une provenance non posée se dit null — obtenu : {out}"
        );
        let back: SubIssue = serde_json::from_str(&out).unwrap();
        assert_eq!(back, sub, "aller-retour fidèle");
    }

    /// Un blob antérieur à `plan_reading` se relit, et son `None` ne se confond
    /// pas avec « ce corps ne nomme aucun plan » (mika#2952, motif mika#2340).
    ///
    /// Le blob porte `plan_present: true` — le booléen lâche, qui était vrai
    /// pour la mauvaise raison sur la population mesurée. Un `None` relu comme
    /// `Absent` ferait du vieux blob un refus `NoGroomedPlan` qu'aucune lecture
    /// n'a établi ; un `None` relu comme `Live` affirmerait un callout que
    /// personne n'a vu. Il ne doit donc rien dire du tout.
    #[test]
    fn mika2952_un_blob_anterieur_se_relit_sans_lecture_de_plan() {
        let blob = r#"{
            "number": 2943,
            "title": "t",
            "state": "open",
            "priority_rank": 1,
            "plan_present": true,
            "branch_present": false,
            "pr_number": null,
            "pr_state": null,
            "ci_state": "none",
            "blockers": [],
            "updated_at": "2026-10-28T00:00:00Z",
            "labels": ["p1-important"]
        }"#;
        let sub: SubIssue =
            serde_json::from_str(blob).expect("un blob d'avant le champ doit encore se lire");
        assert!(sub.plan_present, "le booléen lâche est conservé tel quel");
        assert_eq!(
            sub.plan_reading, None,
            "champ absent ⇒ None, qui dit « corps non lu par ce binaire » et \
             non Absent (mika#2340)"
        );

        let out = serde_json::to_string(&sub).unwrap();
        assert!(
            out.contains(r#""plan_reading":null"#),
            "une lecture non faite se dit null — obtenu : {out}"
        );
        let back: SubIssue = serde_json::from_str(&out).unwrap();
        assert_eq!(back, sub, "aller-retour fidèle");

        // Et les trois états ont un nom de fil stable : le puits hors-ligne les
        // relit, et c'est là que la classe mika#2205 se rejouerait.
        for (reading, wire) in [
            (PlanReading::Live, r#""live""#),
            (PlanReading::PriorOnly, r#""prior_only""#),
            (PlanReading::Absent, r#""absent""#),
        ] {
            let s = serde_json::to_string(&reading).unwrap();
            assert_eq!(s, wire, "nom de fil figé pour {reading:?}");
            assert_eq!(
                serde_json::from_str::<PlanReading>(&s).unwrap(),
                reading,
                "aller-retour fidèle pour {reading:?}"
            );
        }
    }

    /// Un blob antérieur à `dormancy` se relit, et son `None` ne se confond pas
    /// avec « ce corps ne porte pas de condition de réveil » (mika#2952 AC2,
    /// motif mika#2340).
    ///
    /// La distinction est exactement celle que l'AC2 exige de nommer : un
    /// marqueur **absent** rend `Awake`, qui est une mesure, et le bassin admet
    /// — il n'exclut pas en silence. `None` est l'autre chose, l'absence de
    /// mesure, et n'autorise aucun refus. Les relire comme un seul état
    /// remettrait deux populations sous un nom (mika#2205) : le ticket éveillé
    /// et le ticket jamais lu.
    #[test]
    fn mika2952_un_blob_anterieur_se_relit_sans_lecture_de_dormance() {
        let blob = r#"{
            "number": 2854,
            "title": "t",
            "state": "open",
            "priority_rank": 2,
            "plan_present": true,
            "branch_present": false,
            "pr_number": null,
            "pr_state": null,
            "ci_state": "none",
            "blockers": [],
            "updated_at": "2026-10-28T00:00:00Z",
            "labels": ["p2-normal"]
        }"#;
        let sub: SubIssue =
            serde_json::from_str(blob).expect("un blob d'avant le champ doit encore se lire");
        assert_eq!(
            sub.dormancy, None,
            "champ absent ⇒ None, qui dit « corps non lu par ce binaire » et \
             non Awake (mika#2340)"
        );

        let out = serde_json::to_string(&sub).unwrap();
        assert!(
            out.contains(r#""dormancy":null"#),
            "une lecture non faite se dit null — obtenu : {out}"
        );
        let back: SubIssue = serde_json::from_str(&out).unwrap();
        assert_eq!(back, sub, "aller-retour fidèle");

        // Les deux états mesurés ont un nom de fil stable, et celui qui porte la
        // condition la transporte : le puits hors-ligne relit le motif de refus
        // que l'opérateur lira dans le rapport.
        let awake = serde_json::to_string(&DormancyReading::Awake).unwrap();
        assert_eq!(awake, r#""awake""#, "nom de fil figé pour Awake");
        assert_eq!(
            serde_json::from_str::<DormancyReading>(&awake).unwrap(),
            DormancyReading::Awake
        );

        let dormant = DormancyReading::Dormant {
            condition: "Condition de réveil : quand `MIKA_WEBHOOK_COMMENT_TRIAGE` passe à `on`"
                .into(),
        };
        let s = serde_json::to_string(&dormant).unwrap();
        assert!(
            s.starts_with(r#"{"dormant":{"condition":"#),
            "nom de fil figé pour Dormant — obtenu : {s}"
        );
        assert_ne!(s, awake, "les deux états ne se confondent pas sur le fil");
        assert_eq!(
            serde_json::from_str::<DormancyReading>(&s).unwrap(),
            dormant,
            "la condition citée fait l'aller-retour mot pour mot"
        );
    }

    /// Les deux variantes de provenance ont un nom de fil **distinct et
    /// stable** : le puits hors-ligne les relit, et la sonde AC3 s'en sert pour
    /// séparer les tickets fermés à la main de ceux que le correctif répare.
    #[test]
    fn mika2924_les_deux_provenances_ont_un_nom_de_fil_distinct() {
        let a = serde_json::to_string(&PrLinkSource::MilestonePrList).unwrap();
        let b = serde_json::to_string(&PrLinkSource::IssueClosingRef).unwrap();
        assert_eq!(a, r#""milestone_pr_list""#);
        assert_eq!(b, r#""issue_closing_ref""#);
        assert_ne!(a, b);
        assert_eq!(
            serde_json::from_str::<PrLinkSource>(&a).unwrap(),
            PrLinkSource::MilestonePrList
        );
        assert_eq!(
            serde_json::from_str::<PrLinkSource>(&b).unwrap(),
            PrLinkSource::IssueClosingRef
        );
    }

    // -- mika#2924, vol 2 : l'état des PR d'un ticket ouvert ----------------

    /// Les **quatre** variantes ont un nom de fil distinct, et `Citing`
    /// transporte un tableau JSON.
    ///
    /// Quatre et non deux : le puits hors-ligne relit ces blobs, et c'est là que
    /// la classe mika#2205 se jouerait. « Aucune PR mergée ne cite ce ticket »
    /// se corrige en implémentant, « la page était pleine » se corrige en
    /// paginant, « l'appel a échoué » se corrige à la forge — trois remèdes
    /// opposés qu'un seul nom de fil rendrait indiscernables.
    #[test]
    fn mika2924_les_quatre_etats_de_pr_ont_un_nom_de_fil_distinct() {
        let cases = [
            (
                MergedPrReading::Citing(vec![2931, 2929, 2892]),
                r#"{"citing":[2931,2929,2892]}"#,
            ),
            (MergedPrReading::NoneCiting, r#""none_citing""#),
            (
                MergedPrReading::IndeterminatePageFull,
                r#""indeterminate_page_full""#,
            ),
            (
                MergedPrReading::IndeterminateFetchFailed,
                r#""indeterminate_fetch_failed""#,
            ),
        ];
        let mut wires = Vec::new();
        for (value, expected) in &cases {
            let out = serde_json::to_string(value).unwrap();
            assert_eq!(&out, expected, "nom de fil figé pour {value:?}");
            assert_eq!(
                &serde_json::from_str::<MergedPrReading>(&out).unwrap(),
                value,
                "aller-retour fidèle pour {value:?}"
            );
            wires.push(out);
        }
        // Les deux indéterminations sont le couple qu'une simplification
        // replierait en un seul mot ; l'ensemble le refuse.
        let distinct: std::collections::HashSet<&String> = wires.iter().collect();
        assert_eq!(
            distinct.len(),
            cases.len(),
            "quatre variantes, quatre noms de fil — obtenu : {wires:?}"
        );
    }

    /// `Citing` garde l'**ordre** qu'on lui donne : c'est un tri par `merged_at`
    /// décroissant, posé par le composeur, et le fil ne doit pas le trier
    /// autrement ni le dédupliquer dans son dos.
    #[test]
    fn mika2924_letat_des_pr_voyage_sur_un_sub_issue() {
        let sub = SubIssue {
            missing_grooming_signals: None,
            number: 2698,
            title: "t".into(),
            state: IssueState::Open,
            priority_rank: None,
            plan_present: true,
            plan_reading: None,
            dormancy: None,
            branch_present: true,
            pr_number: None,
            pr_state: None,
            pr_url: None,
            pr_link_source: None,
            merged_prs: Some(MergedPrReading::Citing(vec![
                2931, 2929, 2892, 2706, 2704, 2712,
            ])),
            ci_state: CiState::None,
            blockers: vec![],
            updated_at: "2026-10-27T00:00:00Z".into(),
            labels: vec![],
        };
        let out = serde_json::to_string(&sub).unwrap();
        assert!(
            out.contains(r#""merged_prs":{"citing":[2931,2929,2892,2706,2704,2712]}"#),
            "l'ordre du composeur traverse le fil tel quel — obtenu : {out}"
        );
        let back: SubIssue = serde_json::from_str(&out).unwrap();
        assert_eq!(back, sub, "aller-retour fidèle");
    }
}

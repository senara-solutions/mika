//! Evaluator — le verdict du manager sur chaque vol d'implémentation terminé
//! (mika#2698, Phase 2 (b), **vol 1**).
//!
//! LECTURE SEULE, et à un degré plus fort que le reste du module : ce fichier
//! n'a pas d'interrupteur d'écriture OFF, il n'a **aucun chemin d'écriture du
//! tout**. Le verdict qu'il produit a exactement une destination —
//! [`super::types::Assessment::flight_verdicts`], que le `Reporter` rend dans le
//! rapport. C'est ce que le ticket appelle le *mode ombre*, et au vol 1 c'est
//! une propriété structurelle, pas une configuration : `MIKA_MANAGER_EVALUATE_WRITE`
//! n'est pas déclarée ici, parce qu'un interrupteur dont la branche ON ne fait
//! rien est un mensonge de surface. L'écriture (`gh issue comment`, idempotence
//! par checkpoint, exception nommée dans l'allowlist) est le vol 2.
//!
//! # Ce que l'évaluateur lit, et ce qu'il ne lit pas
//!
//! Tout vient de sources déjà disponibles côté moteur — **aucune nouvelle
//! dépendance réseau** :
//!
//! | donnée | source |
//! |---|---|
//! | issue du vol (PR / sauvetage / mort) | les lignes `Outcome:` / `PIPELINE FAILURE:` / `Halt class:` de `tasks.result`, écrites par `dispatch-lib.sh` |
//! | numéro et URL de la PR | `tasks.metadata.claude_pilot.pr_number` / `.pr_url`, sinon la ligne `PR: <url>` |
//! | tours consommés | `tasks.metadata.claude_pilot.turns`, sinon la ligne `Turns: N` |
//! | état de la PR | [`SubIssue::pr_state`] — déjà hydraté par le `Reader` via `gh`, aucun appel de plus |
//!
//! **Le plafond de tours, lui, n'est pas lisible ici.** `tasks.result` porte
//! `Turns: N` mais jamais la borne, et `_pilot_max_turns` /
//! `PILOT_LABEL_TURN_CEILINGS` (`skills/bundled/_shared/dispatch-lib.sh`)
//! déclarent un lecteur unique : le dupliquer en Rust serait le second lecteur
//! que cette déclaration refuse. Donc [`FlightVerdict::turn_ceiling`] est
//! **toujours `None`** au vol 1, et [`FlightVerdict::hit_ceiling`] dit seulement
//! si la halte `budget_exhausted` a eu lieu. `None` veut dire *non instrumenté*,
//! jamais `0` (motif mika#2331) — même règle pour
//! [`FlightVerdict::last_commit_turn`], que le vol 2 branchera sur le
//! transcript.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::types::{MilestoneState, SubIssue};

// ---- Le verdict ------------------------------------------------------------

/// Comment un vol d'implémentation s'est terminé.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlightIssue {
    /// Le pilote a ouvert une PR sur son chemin nominal.
    PrOpened {
        number: Option<u64>,
        url: Option<String>,
    },
    /// Le pipeline est resté incomplet **mais** porte une PR — le sauvetage de
    /// worktree sale de mika#1282. Du contenu a été récupéré ; l'issue n'est
    /// pas pour autant un succès.
    RescuePr {
        number: Option<u64>,
        url: Option<String>,
    },
    /// Aucune PR. La cause est nommée, jamais avalée.
    Dead { cause: DeathCause },
}

/// Pourquoi un vol est mort, dans les termes que le producteur écrit.
///
/// Chaque variante correspond à une forme textuelle précise de `tasks.result` —
/// voir [`classify_issue`]. [`DeathCause::Unknown`] **porte** le jeton
/// non classé au lieu de l'effacer : une famille de halte que ce module ne
/// connaît pas doit se lire dans le rapport, pas disparaître dedans.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeathCause {
    /// `Halt class: budget_exhausted` — le plafond de tours ou de budget.
    CeilingReached,
    /// `STRUCTURAL VIOLATION:` — refus explicite du pilote.
    Refusal,
    /// `PIPELINE FAILURE:` ou `PIPELINE_INCOMPLETE`, sans PR.
    PipelineFailure,
    /// `Outcome: LAUNCHER_DEAD` — le lanceur lui-même n'a pas survécu.
    LauncherDead,
    /// `Outcome: ESCALATE` — une étape a escaladé vers l'opérateur.
    Escalated,
    /// Famille de halte non classée — **nommée**, pas avalée.
    Unknown(String),
}

/// Ce que le manager **sait** de l'état de la PR d'un vol (mika#2921).
///
/// Pas un `Option<String>`, et c'est tout l'objet du type : depuis que
/// l'évaluateur juge toute la boucle, un vol peut porter sur un ticket que le
/// `Reader` n'a jamais hydraté — il n'hydrate que les sous-issues du milestone
/// cible, et le lire coûterait un appel réseau par vol et par cycle, que ce vol
/// refuse. « Lu, et vide » et « pas lu » sont alors deux populations
/// distinctes (classe mika#2205), qu'un `Option` aurait fusionnées en une seule
/// absence (motif mika#2331).
///
/// La recommandation ne change pas entre les deux — voir [`recommend`] — c'est
/// le **rendu** qui porte la non-lecture.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrStateKnowledge {
    /// Le ticket est apparié à une sous-issue du milestone cible : l'état a été
    /// lu, et il peut être vide (`None` = ce ticket n'a pas de PR connue).
    Read(Option<String>),
    /// Le ticket n'est apparié à aucune sous-issue du milestone cible : **ni**
    /// son milestone **ni** son état de PR n'ont été lus.
    ///
    /// C'est aussi le défaut à la désérialisation, et pour la même raison : un
    /// blob de rapport antérieur à mika#2921 ne porte pas le champ, donc il ne
    /// porte aucune lecture — dire `Read(None)` à sa place serait inventer une
    /// mesure.
    #[default]
    NotRead,
}

/// Ce que le manager recommande à l'opérateur pour ce vol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlightRecommendation {
    /// La PR est ouverte : elle attend une revue et une fusion.
    MergeCandidate,
    /// Du contenu existe mais le chemin nominal a échoué : un humain reprend.
    ManualResumption,
    /// Rien d'exploitable n'est sorti : le ticket doit repasser au grooming.
    Regroom,
}

/// Le verdict structuré d'un vol terminé.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlightVerdict {
    /// `tasks.id` du vol évalué — la clé qui rend le verdict rejouable.
    pub task_id: String,
    /// Le **dépôt** du ticket, `owner/repo` (mika#2921).
    ///
    /// `None` = le vol ne porte pas d'URL de ticket, jamais une chaîne vide
    /// (motif mika#2331). Deux vols de deux dépôts différents peuvent porter le
    /// même `issue_number` : depuis que le périmètre est la boucle entière,
    /// c'est ce champ qui les distingue, et le rendu le lit pour cela.
    ///
    /// `#[serde(default)]` pour la même raison que les champs de
    /// [`super::types::Assessment`] : le puits hors-ligne garde des blobs
    /// écrits avant ce champ, et une montée de version incapable de relire son
    /// propre arriéré le perd.
    #[serde(default)]
    pub repo: Option<String>,
    /// Le ticket sur lequel le vol portait, lu dans `reference_url`.
    pub issue_number: Option<u64>,
    pub issue: FlightIssue,
    /// Tours consommés. `None` = non instrumenté, jamais `0`.
    pub turns_used: Option<u32>,
    /// Vrai quand le vol a heurté `budget_exhausted`.
    pub hit_ceiling: bool,
    /// Le plafond lui-même. **Toujours `None` au vol 1** — voir le docstring du
    /// module : son lecteur unique est en shell.
    pub turn_ceiling: Option<u32>,
    /// Tour du dernier commit. **Toujours `None` au vol 1** (vol 2 : transcript).
    pub last_commit_turn: Option<u32>,
    /// Ce que le manager sait de l'état de la PR — lu, ou non lu (mika#2921).
    ///
    /// `#[serde(default)]` ⇒ [`PrStateKnowledge::NotRead`] : un blob antérieur
    /// à ce champ ne porte aucune lecture, et c'est exactement ce que la
    /// variante par défaut dit.
    #[serde(default)]
    pub pr_state: PrStateKnowledge,
    pub recommendation: FlightRecommendation,
    /// ISO 8601 UTC — `tasks.completed_at`.
    pub completed_at: Option<String>,
}

// ---- La source des vols ----------------------------------------------------

/// Une ligne de `tasks` réduite à ce que l'évaluateur lit.
///
/// Volontairement pas `crate::db::Task` : le type complet porte trente-deux
/// colonnes, dont `action_config` et `input_context`. Un évaluateur qui les voit
/// peut les lire ; celui-ci ne peut pas.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FinishedFlight {
    pub task_id: String,
    /// L'agent **propriétaire** de la ligne lue — une septième donnée, portée
    /// pour la seule observabilité (mika#2792), et l'exception assumée à la
    /// minceur décrite ci-dessus.
    ///
    /// Aucun chemin de verdict ne la lit : [`evaluate_flight`] ne la touche pas,
    /// et [`FlightVerdict`] ne la porte pas. Elle existe parce que la requête
    /// n'est **plus** scopée à un agent, donc « quels agents possèdent les vols
    /// trouvés » est devenu une mesure, et c'est la mesure qui distingue « aucun
    /// vol » de « mauvaise base ». La déclarer serait inutile ; la mesurer est le
    /// correctif.
    pub agent_id: String,
    pub status: String,
    pub result: Option<String>,
    /// JSON brut de `tasks.metadata` ; `claude_pilot` est lu là-dedans.
    pub metadata: Option<String>,
    pub reference_url: Option<String>,
    pub completed_at: Option<String>,
    /// Le **dépôt** du ticket du vol, `owner/repo` (mika#2921) — la huitième
    /// donnée, née du jour où l'évaluateur a cessé d'être cantonné à un seul
    /// dépôt.
    ///
    /// Dérivée de [`FinishedFlight::reference_url`] par la source
    /// ([`crate::flight_source::repo_from_issue_url`]), jamais de la liste
    /// interrogée : `None` est « ce vol ne nomme pas de dépôt », pas « dépôt
    /// inconnu de la liste » (motif mika#2331). Un vol sur deux dépôts
    /// différents peut porter le même numéro, donc le rendu la lit pour
    /// désambiguïser.
    pub repo: Option<String>,
}

/// D'où viennent les vols terminés.
///
/// Trait ici, implémentation base de données **hors** du module
/// (`crate::flight_source::DbFlightSource`) : c'est l'idiome d'injection de
/// `AuthBoundaryLedger` / `DbAuthBoundaryLedger`, et il a la même raison d'être
/// — aucun SQL ne passe ainsi sous le grep de `no_dispatch_test.rs`, et la tâche
/// de cadence ne tient aucune poignée de base.
#[async_trait]
pub trait FlightSource: Send + Sync {
    /// Vols d'implémentation terminés depuis `since` (exclu), sur **tout le
    /// périmètre des dépôts sous contrôle**. Lecture seule. Jamais d'erreur
    /// visible : un échec rend `vec![]` et un WARN — un rapport sans section
    /// vaut mieux qu'un cycle mort.
    ///
    /// **Le périmètre n'est pas un paramètre** (mika#2921). Il l'était —
    /// `issue_urls`, les sous-tickets du milestone surveillé — et c'est ce qui
    /// faisait du manager un rapporteur du milestone. L'implémentation le tient
    /// désormais elle-même, parce qu'aucun fichier de ce module ne doit nommer de
    /// dépôt (garde `no_repo_list_test.rs`) : le lecteur unique de la liste est
    /// [`crate::flight_source`]. Un appelant n'a donc plus de façon de restreindre
    /// la lecture — ni de l'élargir.
    async fn finished_implement_flights(&self, since: Option<&str>) -> Vec<FinishedFlight>;
}

// ---- Évaluation ------------------------------------------------------------

/// Les URLs d'issue du milestone, sous leur forme canonique.
///
/// La source appariera elle-même les variantes `?phase=groom`
/// (`task_state::tasks::issue_url_variants`) : ce qui sort d'ici est la forme
/// que les écrivains moteur composent.
pub fn milestone_issue_urls(state: &MilestoneState) -> Vec<String> {
    state
        .sub_issues
        .iter()
        .map(|s| {
            format!(
                "https://github.com/{}/issues/{}",
                state.milestone_ref.repo, s.number
            )
        })
        .collect()
}

/// Le verdict de chaque vol, dans l'ordre reçu.
///
/// `sub_issues` sert à **apparier**, jamais à borner : depuis mika#2921 la liste
/// des vols vient du périmètre des dépôts sous contrôle, et un vol qui ne
/// s'apparie à rien reçoit tout de même son verdict — avec
/// [`PrStateKnowledge::NotRead`] au lieu d'un état deviné.
pub fn evaluate_flights(flights: &[FinishedFlight], sub_issues: &[SubIssue]) -> Vec<FlightVerdict> {
    flights
        .iter()
        .map(|f| {
            let issue_number = issue_number_from_url(f.reference_url.as_deref());
            // L'échec d'appariement n'est pas « pas d'état » : c'est « pas lu ».
            let pr_state =
                match issue_number.and_then(|n| sub_issues.iter().find(|s| s.number == n)) {
                    Some(s) => PrStateKnowledge::Read(s.pr_state.clone()),
                    None => PrStateKnowledge::NotRead,
                };
            evaluate_flight(f, issue_number, &pr_state)
        })
        .collect()
}

/// Le verdict d'un vol, `pr_state` étant ce que le manager **sait** de l'état de
/// sa PR : [`PrStateKnowledge::Read`] quand le `Reader` l'a hydraté (`open` /
/// `closed` / `merged`, ou `None` s'il n'y en a pas),
/// [`PrStateKnowledge::NotRead`] quand le ticket n'appartient pas au milestone
/// cible et n'a donc jamais été lu.
pub fn evaluate_flight(
    flight: &FinishedFlight,
    issue_number: Option<u64>,
    pr_state: &PrStateKnowledge,
) -> FlightVerdict {
    let result = flight.result.as_deref().unwrap_or("");
    let pilot = PilotFields::parse(flight.metadata.as_deref());
    let issue = classify_issue(result, &pilot);
    FlightVerdict {
        task_id: flight.task_id.clone(),
        repo: flight.repo.clone(),
        issue_number,
        recommendation: recommend(&issue, pr_state),
        pr_state: pr_state.clone(),
        issue,
        turns_used: pilot.turns.or_else(|| {
            line_value(result, "Turns:").and_then(|v| v.split_whitespace().next()?.parse().ok())
        }),
        hit_ceiling: hit_ceiling(result),
        // Les deux data que le vol 1 n'a pas : plafond (lecteur unique en shell)
        // et tour du dernier commit (transcript). Présent-et-`None` dit
        // « non instrumenté » ; `0` dirait « mesuré à zéro » (motif mika#2331).
        turn_ceiling: None,
        last_commit_turn: None,
        completed_at: flight.completed_at.clone(),
    }
}

/// La matrice de recommandation, dérivée de l'issue et de ce qui est **su** de
/// l'état de la PR.
///
/// `Read(None)` sur une PR ouverte rend `MergeCandidate` : la seule raison de
/// s'en écarter serait une preuve que la PR est déjà fusionnée ou fermée, et la
/// recommandation est de toute façon *« va regarder »*, pas un geste.
///
/// `NotRead` rend **la même chose** (mika#2921), et ce n'est pas une
/// approximation : pour un vol qui a ouvert une PR, la recommandation utile est
/// identique avec ou sans l'état. Ce que le manager doit cesser de faire, c'est
/// la **présenter comme si l'état avait été lu** — et c'est au rendu, pas ici,
/// que la non-lecture se dit (R7). Deviner `ManualResumption` sur une
/// non-lecture enverrait un humain sur chaque vol de la boucle.
fn recommend(issue: &FlightIssue, pr_state: &PrStateKnowledge) -> FlightRecommendation {
    match issue {
        FlightIssue::PrOpened { .. } => match pr_state {
            // Fusionnée ou fermée pendant que le vol volait : le verdict du vol
            // est en retard sur la réalité, un humain tranche.
            PrStateKnowledge::Read(Some(s)) if !s.eq_ignore_ascii_case("open") => {
                FlightRecommendation::ManualResumption
            }
            _ => FlightRecommendation::MergeCandidate,
        },
        // Un sauvetage n'est jamais un merge-candidat, quel que soit l'état de
        // la PR : c'est un brouillon que le pipeline n'a pas fini.
        FlightIssue::RescuePr { .. } => FlightRecommendation::ManualResumption,
        FlightIssue::Dead { cause } => match cause {
            // Plafond, refus, escalade : le vol dit quelque chose sur le ticket
            // lui-même — trop gros, mal posé, ou hors périmètre. Re-groom.
            DeathCause::CeilingReached | DeathCause::Refusal | DeathCause::Escalated => {
                FlightRecommendation::Regroom
            }
            // Panne, lanceur mort, cause non classée : le ticket n'est pas en
            // cause, le substrat l'est. Un humain regarde avant de re-groomer.
            DeathCause::PipelineFailure | DeathCause::LauncherDead | DeathCause::Unknown(_) => {
                FlightRecommendation::ManualResumption
            }
        },
    }
}

/// Quelle issue `tasks.result` décrit.
fn classify_issue(result: &str, pilot: &PilotFields) -> FlightIssue {
    let outcome = line_value(result, "Outcome:").and_then(|v| v.split_whitespace().next());
    let (number, url) = pr_reference(result, pilot);
    match outcome {
        Some("PR_OPENED") => FlightIssue::PrOpened { number, url },
        // mika#1282 : un `PIPELINE_INCOMPLETE` qui **porte** une PR est un
        // sauvetage, pas une mort.
        Some("PIPELINE_INCOMPLETE") if number.is_some() || url.is_some() => {
            FlightIssue::RescuePr { number, url }
        }
        _ => FlightIssue::Dead {
            cause: death_cause(result, outcome),
        },
    }
}

/// Vrai quand la halte observée est `budget_exhausted` — la famille que
/// `dispatch-lib.sh` donne à `error_max_turns` et `error_max_budget_usd`.
///
/// Indépendant de l'issue : un vol peut heurter le plafond **et** laisser une PR
/// de sauvetage derrière lui.
fn hit_ceiling(result: &str) -> bool {
    line_value(result, "Halt class:").is_some_and(|v| v.starts_with("budget_exhausted"))
}

/// La cause de mort, dans l'ordre de spécificité décroissante.
///
/// L'ordre est la décision : une mort au plafond porte aussi la bannière
/// `PIPELINE FAILURE:`, et un refus structurel aussi — donc tester la bannière
/// d'abord classerait les trois en `PipelineFailure` et perdrait précisément la
/// distinction que le ticket demande.
fn death_cause(result: &str, outcome: Option<&str>) -> DeathCause {
    if hit_ceiling(result) {
        return DeathCause::CeilingReached;
    }
    if result.contains("STRUCTURAL VIOLATION:") {
        return DeathCause::Refusal;
    }
    match outcome {
        Some("LAUNCHER_DEAD") => DeathCause::LauncherDead,
        Some("ESCALATE") => DeathCause::Escalated,
        Some("PIPELINE_INCOMPLETE") => DeathCause::PipelineFailure,
        _ if result.contains("PIPELINE FAILURE:") => DeathCause::PipelineFailure,
        Some(other) => DeathCause::Unknown(other.to_string()),
        // Un vol terminé sans ligne `Outcome:` n'est pas un vol sans cause :
        // c'est un vol dont le producteur n'a rien écrit, et ça se lit.
        None => DeathCause::Unknown("no_outcome_line".to_string()),
    }
}

/// Le numéro et l'URL de la PR, de la source la plus structurée à la moins.
fn pr_reference(result: &str, pilot: &PilotFields) -> (Option<u64>, Option<String>) {
    let url = pilot
        .pr_url
        .clone()
        .or_else(|| line_value(result, "PR:").map(str::to_string))
        .or_else(|| {
            // `Outcome: PR_OPENED — <url>` porte l'URL quand le reste est muet.
            line_value(result, "Outcome:")?
                .split_whitespace()
                .find(|t| t.starts_with("https://"))
                .map(str::to_string)
        });
    let number = pilot
        .pr_number
        .or_else(|| pr_number_from_url(url.as_deref()?));
    (number, url)
}

/// `…/pull/2696` → `2696`.
fn pr_number_from_url(url: &str) -> Option<u64> {
    url.rsplit_once("/pull/")?
        .1
        .split(['/', '?', '#'])
        .next()?
        .parse()
        .ok()
}

/// `…/issues/2698` → `2698`, en tolérant le suffixe `?phase=groom`.
fn issue_number_from_url(url: Option<&str>) -> Option<u64> {
    url?.rsplit_once("/issues/")?
        .1
        .split(['/', '?', '#'])
        .next()?
        .parse()
        .ok()
}

/// La valeur d'une ligne `<prefix> <valeur>`, ancrée en début de ligne.
///
/// Ancrée, parce que `dispatch-lib.sh` écrit ces lignes en début de ligne et que
/// le corps d'un message d'erreur peut parfaitement contenir `Turns:` au milieu
/// d'une phrase.
fn line_value<'a>(source: &'a str, prefix: &str) -> Option<&'a str> {
    source
        .lines()
        .find(|l| l.starts_with(prefix))
        .map(|l| l[prefix.len()..].trim())
        .filter(|v| !v.is_empty())
}

/// Les champs de `tasks.metadata.claude_pilot` que l'évaluateur lit.
///
/// Même forme de lecture que `server::dashboard_dev_runs::DevRunResponse` : les
/// tours sont déjà persistés, aucun transcript à parser.
#[derive(Debug, Default)]
struct PilotFields {
    turns: Option<u32>,
    pr_number: Option<u64>,
    pr_url: Option<String>,
}

impl PilotFields {
    fn parse(metadata: Option<&str>) -> Self {
        let Some(cp) = metadata
            .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
            .and_then(|v| v.get("claude_pilot").cloned())
        else {
            return Self::default();
        };
        Self {
            turns: cp.get("turns").and_then(|v| v.as_u64()).map(|n| n as u32),
            pr_number: cp.get("pr_number").and_then(|v| v.as_u64()),
            pr_url: cp
                .get("pr_url")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Fixtures : les trois issues de l'AC3, au format du producteur ----

    /// Un vol nominal : `dispatch-lib.sh` § cascade mika#940 Unit 3, branche
    /// `$PR_URL` non vide.
    fn fixture_pr_opened() -> FinishedFlight {
        FinishedFlight {
            task_id: "task-pr".to_string(),
            agent_id: "mika-dev".to_string(),
            status: "completed".to_string(),
            result: Some(
                "Session: 5b6f13dc\n\
                 Turns: 83\n\
                 PR: https://github.com/senara-solutions/mika/pull/2696\n\
                 Outcome: PR_OPENED — https://github.com/senara-solutions/mika/pull/2696\n"
                    .to_string(),
            ),
            metadata: Some(
                r#"{"claude_pilot":{"turns":83,"pr_number":2696,
                   "pr_url":"https://github.com/senara-solutions/mika/pull/2696"}}"#
                    .to_string(),
            ),
            reference_url: Some("https://github.com/senara-solutions/mika/issues/2693".to_string()),
            completed_at: Some("2026-10-13T08:00:00Z".to_string()),
            repo: Some("senara-solutions/mika".to_string()),
        }
    }

    /// Un sauvetage mika#1282 : worktree sale, zéro commit, PR de brouillon
    /// ouverte, issue restée `PIPELINE_INCOMPLETE`.
    fn fixture_rescue_pr() -> FinishedFlight {
        FinishedFlight {
            task_id: "task-rescue".to_string(),
            agent_id: "mika-dev".to_string(),
            status: "failed".to_string(),
            result: Some(
                "PIPELINE FAILURE: dirty worktree with zero commits\n\
                 Session: 7c1a\n\
                 Turns: 141\n\
                 PR: https://github.com/senara-solutions/mika/pull/2690\n\
                 Outcome: PIPELINE_INCOMPLETE — manual recovery needed.\n"
                    .to_string(),
            ),
            metadata: Some(r#"{"claude_pilot":{"turns":141,"pr_number":2690}}"#.to_string()),
            reference_url: Some("https://github.com/senara-solutions/mika/issues/2655".to_string()),
            completed_at: Some("2026-10-13T09:00:00Z".to_string()),
            repo: Some("senara-solutions/mika".to_string()),
        }
    }

    /// Une mort au plafond : `_halt_family` donne `budget_exhausted` à
    /// `error_max_turns`, et aucune PR n'a été ouverte.
    fn fixture_dead_at_ceiling() -> FinishedFlight {
        FinishedFlight {
            task_id: "task-ceiling".to_string(),
            agent_id: "mika-dev".to_string(),
            status: "failed".to_string(),
            result: Some(
                "PIPELINE FAILURE: pilot halted\n\
                 Session: 9f2b\n\
                 Turns: 200\n\
                 Halt class: budget_exhausted — deterministic\n\
                 Retry hint: deterministic — a replay will halt identically\n\
                 Commits: 0\n\
                 NO_PR: no_branch_pushed\n\
                 Outcome: PIPELINE_INCOMPLETE — manual recovery needed.\n"
                    .to_string(),
            ),
            metadata: Some(r#"{"claude_pilot":{"turns":200}}"#.to_string()),
            reference_url: Some("https://github.com/senara-solutions/mika/issues/2661".to_string()),
            completed_at: Some("2026-10-13T10:00:00Z".to_string()),
            repo: Some("senara-solutions/mika".to_string()),
        }
    }

    fn sub_issue(number: u64, pr_state: Option<&str>) -> SubIssue {
        SubIssue {
            number,
            title: format!("sujet {number}"),
            state: crate::milestone_manager::types::IssueState::Open,
            priority_rank: None,
            plan_present: true,
            branch_present: true,
            pr_number: None,
            pr_state: pr_state.map(str::to_string),
            pr_url: None,
            pr_link_source: None,
            ci_state: crate::milestone_manager::types::CiState::None,
            blockers: vec![],
            updated_at: "2026-10-13T00:00:00Z".to_string(),
            labels: vec![],
        }
    }

    /// « Le `Reader` a lu cet état » — l'autre population étant
    /// [`PrStateKnowledge::NotRead`], écrite en clair partout où elle sert.
    fn lu(etat: Option<&str>) -> PrStateKnowledge {
        PrStateKnowledge::Read(etat.map(str::to_string))
    }

    // ---- AC3 : les trois issues, avec les tours lus correctement ----------

    #[test]
    fn mika2698_une_pr_ouverte_est_un_merge_candidat_avec_ses_tours() {
        let v = evaluate_flights(&[fixture_pr_opened()], &[sub_issue(2693, Some("open"))]);
        let v = &v[0];
        assert_eq!(v.issue_number, Some(2693));
        assert_eq!(
            v.issue,
            FlightIssue::PrOpened {
                number: Some(2696),
                url: Some("https://github.com/senara-solutions/mika/pull/2696".to_string()),
            }
        );
        assert_eq!(v.turns_used, Some(83), "les tours viennent de claude_pilot");
        assert!(!v.hit_ceiling);
        assert_eq!(v.recommendation, FlightRecommendation::MergeCandidate);
        assert_eq!(v.completed_at.as_deref(), Some("2026-10-13T08:00:00Z"));
    }

    #[test]
    fn mika2698_une_pr_de_sauvetage_demande_une_reprise_a_la_main() {
        let v = evaluate_flights(&[fixture_rescue_pr()], &[sub_issue(2655, Some("open"))]);
        let v = &v[0];
        assert_eq!(v.issue_number, Some(2655));
        assert_eq!(
            v.issue,
            FlightIssue::RescuePr {
                number: Some(2690),
                url: Some("https://github.com/senara-solutions/mika/pull/2690".to_string()),
            },
            "un PIPELINE_INCOMPLETE qui porte une PR est un sauvetage (mika#1282)"
        );
        assert_eq!(v.turns_used, Some(141));
        assert!(!v.hit_ceiling);
        assert_eq!(v.recommendation, FlightRecommendation::ManualResumption);
    }

    #[test]
    fn mika2698_une_mort_au_plafond_demande_un_re_groom() {
        let v = evaluate_flights(&[fixture_dead_at_ceiling()], &[sub_issue(2661, None)]);
        let v = &v[0];
        assert_eq!(v.issue_number, Some(2661));
        assert_eq!(
            v.issue,
            FlightIssue::Dead {
                cause: DeathCause::CeilingReached
            },
            "`Halt class: budget_exhausted` doit battre la bannière PIPELINE FAILURE"
        );
        assert_eq!(v.turns_used, Some(200));
        assert!(v.hit_ceiling);
        assert_eq!(v.recommendation, FlightRecommendation::Regroom);
    }

    #[test]
    fn mika2698_le_plafond_et_le_tour_du_dernier_commit_sont_non_instrumentes() {
        // Le lecteur unique du plafond est en shell (`_pilot_max_turns`), et le
        // tour du dernier commit vient du transcript : les deux sont vol 2.
        // `None` dit « non mesuré » — un `0` dirait « mesuré à zéro ».
        for f in [
            fixture_pr_opened(),
            fixture_rescue_pr(),
            fixture_dead_at_ceiling(),
        ] {
            let v = evaluate_flight(&f, None, &PrStateKnowledge::NotRead);
            assert_eq!(v.turn_ceiling, None, "{}", f.task_id);
            assert_eq!(v.last_commit_turn, None, "{}", f.task_id);
        }
    }

    // ---- Les tours : les deux sources, et leur ordre ----------------------

    #[test]
    fn mika2698_les_tours_retombent_sur_la_ligne_turns_sans_metadata() {
        let mut f = fixture_pr_opened();
        f.metadata = None;
        let v = evaluate_flight(&f, None, &lu(Some("open")));
        assert_eq!(
            v.turns_used,
            Some(83),
            "la ligne `Turns: 83` fait le relais"
        );
        assert_eq!(
            v.issue,
            FlightIssue::PrOpened {
                number: Some(2696),
                url: Some("https://github.com/senara-solutions/mika/pull/2696".to_string()),
            },
            "le numéro de PR se dérive de l'URL quand claude_pilot manque"
        );
    }

    #[test]
    fn mika2698_des_tours_illisibles_restent_absents_jamais_zero() {
        let mut f = fixture_pr_opened();
        f.metadata = Some(r#"{"claude_pilot":{}}"#.to_string());
        f.result = Some("Turns: unknown\nOutcome: PR_OPENED — x\n".to_string());
        assert_eq!(
            evaluate_flight(&f, None, &PrStateKnowledge::NotRead).turns_used,
            None
        );
    }

    #[test]
    fn mika2698_la_ligne_turns_est_ancree_en_debut_de_ligne() {
        let mut f = fixture_dead_at_ceiling();
        f.metadata = None;
        f.result = Some(
            "PIPELINE FAILURE: the model said Turns: 7 in prose\n\
             Turns: 200\n\
             Outcome: PIPELINE_INCOMPLETE — manual recovery needed.\n"
                .to_string(),
        );
        assert_eq!(
            evaluate_flight(&f, None, &PrStateKnowledge::NotRead).turns_used,
            Some(200)
        );
    }

    // ---- Les causes de mort, une par forme du producteur -----------------

    fn dead_cause_of(result: &str) -> DeathCause {
        let f = FinishedFlight {
            task_id: "t".to_string(),
            result: Some(result.to_string()),
            ..Default::default()
        };
        match evaluate_flight(&f, None, &PrStateKnowledge::NotRead).issue {
            FlightIssue::Dead { cause } => cause,
            other => panic!("attendu mort, obtenu {other:?}"),
        }
    }

    #[test]
    fn mika2698_chaque_forme_du_producteur_a_sa_cause_de_mort() {
        assert_eq!(
            dead_cause_of("Halt class: budget_exhausted — deterministic\nOutcome: UNKNOWN — x\n"),
            DeathCause::CeilingReached
        );
        assert_eq!(
            dead_cause_of(
                "STRUCTURAL VIOLATION: le plan demande un second lecteur\n\
                 Outcome: PIPELINE_INCOMPLETE — manual recovery needed.\n"
            ),
            DeathCause::Refusal,
            "un refus explicite n'est pas une panne de pipeline"
        );
        assert_eq!(
            dead_cause_of("PIPELINE FAILURE: no shipping tail\nOutcome: UNKNOWN — inspect.\n"),
            DeathCause::PipelineFailure
        );
        assert_eq!(
            dead_cause_of("Outcome: PIPELINE_INCOMPLETE — no_shipping_tail: nothing pushed\n"),
            DeathCause::PipelineFailure,
            "`no_shipping_tail` n'écrit pas la bannière mais reste une panne"
        );
        assert_eq!(
            dead_cause_of("Outcome: LAUNCHER_DEAD\n"),
            DeathCause::LauncherDead
        );
        assert_eq!(
            dead_cause_of("Outcome: ESCALATE — groom\n"),
            DeathCause::Escalated
        );
        assert_eq!(
            dead_cause_of("Outcome: PLAN_COMMITTED — docs/plans/x.md\n"),
            DeathCause::Unknown("PLAN_COMMITTED".to_string()),
            "une famille non classée se nomme dans le verdict"
        );
        assert_eq!(
            dead_cause_of("the pilot wrote nothing the engine recognises\n"),
            DeathCause::Unknown("no_outcome_line".to_string()),
            "un résultat muet est une absence nommée, pas une absence de cause"
        );
    }

    #[test]
    fn mika2698_un_resultat_absent_ne_panique_pas() {
        let f = FinishedFlight {
            task_id: "t".to_string(),
            ..Default::default()
        };
        let v = evaluate_flight(&f, None, &PrStateKnowledge::NotRead);
        assert_eq!(
            v.issue,
            FlightIssue::Dead {
                cause: DeathCause::Unknown("no_outcome_line".to_string())
            }
        );
        assert_eq!(v.turns_used, None);
        assert!(!v.hit_ceiling);
    }

    #[test]
    fn mika2698_un_vol_peut_heurter_le_plafond_et_laisser_une_pr() {
        // Les deux axes sont indépendants : `hit_ceiling` décrit la halte,
        // `issue` décrit ce qui en est sorti.
        let mut f = fixture_rescue_pr();
        f.result = Some(
            "PIPELINE FAILURE: pilot halted\n\
             Turns: 200\n\
             Halt class: budget_exhausted — deterministic\n\
             PR: https://github.com/senara-solutions/mika/pull/2690\n\
             Outcome: PIPELINE_INCOMPLETE — manual recovery needed.\n"
                .to_string(),
        );
        let v = evaluate_flight(&f, Some(2655), &lu(Some("open")));
        assert!(v.hit_ceiling);
        assert!(matches!(v.issue, FlightIssue::RescuePr { .. }));
        assert_eq!(v.recommendation, FlightRecommendation::ManualResumption);
    }

    // ---- La matrice de recommandation, exhaustivement ---------------------

    /// La matrice entière, les trois issues croisées avec les **trois** états de
    /// connaissance — `Read(Some(…))`, `Read(None)` et, depuis mika#2921,
    /// `NotRead`.
    ///
    /// La colonne `NotRead` est la moitié d'AC5 qui vit ici : la recommandation
    /// d'un vol non lu est celle d'un vol lu-sans-état, et c'est **voulu**.
    /// L'autre moitié — que le rapport le dise au lieu de le taire — est dans
    /// `reporter.rs`. Si cette colonne divergeait un jour de la précédente, un
    /// humain serait envoyé sur chaque vol de la boucle.
    #[test]
    fn mika2698_la_matrice_de_recommandation_est_exhaustive() {
        let pr = || FlightIssue::PrOpened {
            number: Some(1),
            url: None,
        };
        let rescue = || FlightIssue::RescuePr {
            number: Some(1),
            url: None,
        };
        let dead = |c: DeathCause| FlightIssue::Dead { cause: c };
        let non_lu = || PrStateKnowledge::NotRead;

        let cases: &[(FlightIssue, PrStateKnowledge, FlightRecommendation)] = &[
            (pr(), lu(Some("open")), FlightRecommendation::MergeCandidate),
            (pr(), lu(None), FlightRecommendation::MergeCandidate),
            (pr(), non_lu(), FlightRecommendation::MergeCandidate),
            (
                pr(),
                lu(Some("merged")),
                FlightRecommendation::ManualResumption,
            ),
            (
                pr(),
                lu(Some("closed")),
                FlightRecommendation::ManualResumption,
            ),
            (
                rescue(),
                lu(Some("open")),
                FlightRecommendation::ManualResumption,
            ),
            (
                rescue(),
                lu(Some("merged")),
                FlightRecommendation::ManualResumption,
            ),
            (rescue(), lu(None), FlightRecommendation::ManualResumption),
            (rescue(), non_lu(), FlightRecommendation::ManualResumption),
            (
                dead(DeathCause::CeilingReached),
                lu(None),
                FlightRecommendation::Regroom,
            ),
            (
                dead(DeathCause::CeilingReached),
                non_lu(),
                FlightRecommendation::Regroom,
            ),
            (
                dead(DeathCause::Refusal),
                lu(None),
                FlightRecommendation::Regroom,
            ),
            (
                dead(DeathCause::Escalated),
                lu(None),
                FlightRecommendation::Regroom,
            ),
            (
                dead(DeathCause::PipelineFailure),
                lu(None),
                FlightRecommendation::ManualResumption,
            ),
            (
                dead(DeathCause::PipelineFailure),
                non_lu(),
                FlightRecommendation::ManualResumption,
            ),
            (
                dead(DeathCause::LauncherDead),
                lu(None),
                FlightRecommendation::ManualResumption,
            ),
            (
                dead(DeathCause::Unknown("x".to_string())),
                lu(None),
                FlightRecommendation::ManualResumption,
            ),
        ];
        for (issue, pr_state, expected) in cases {
            assert_eq!(
                recommend(issue, pr_state),
                *expected,
                "issue={issue:?} pr_state={pr_state:?}"
            );
        }
    }

    /// Un sous-ticket non apparié ne prête pas son état — et depuis mika#2921 le
    /// verdict dit *pourquoi* : il n'a rien lu.
    ///
    /// L'assertion s'est renforcée, pas affaiblie. Avant, le troisième cas
    /// vérifiait seulement que la recommandation ne bougeait pas ; un
    /// `Read(None)` posé par erreur à la place du `NotRead` l'aurait laissée
    /// verte, et le rapport aurait annoncé un état lu qui n'existait pas.
    #[test]
    fn mika2698_letat_de_pr_vient_du_sous_ticket_apparie() {
        // Le même vol, deux milestones : seul l'état hydraté par le Reader
        // change le verdict. Aucun appel `gh` de plus.
        let flights = [fixture_pr_opened()];
        let apparie_fusionne = &evaluate_flights(&flights, &[sub_issue(2693, Some("merged"))])[0];
        assert_eq!(
            apparie_fusionne.recommendation,
            FlightRecommendation::ManualResumption
        );
        assert_eq!(
            apparie_fusionne.pr_state,
            lu(Some("merged")),
            "un vol apparié porte l'état que le Reader a lu"
        );
        assert_eq!(
            evaluate_flights(&flights, &[sub_issue(2693, Some("open"))])[0].recommendation,
            FlightRecommendation::MergeCandidate
        );

        let non_apparie = &evaluate_flights(&flights, &[sub_issue(9999, Some("merged"))])[0];
        assert_eq!(
            non_apparie.recommendation,
            FlightRecommendation::MergeCandidate,
            "un sous-ticket qui n'apparie pas ne doit pas prêter son état"
        );
        assert_eq!(
            non_apparie.pr_state,
            PrStateKnowledge::NotRead,
            "et le verdict doit dire qu'il n'a rien lu, pas qu'il a lu du vide"
        );
    }

    /// AC5, le contrôle positif à côté du négatif : le **même** vol apparié dit
    /// `Read`, non apparié dit `NotRead`, et les deux populations ne se
    /// confondent pas (classe mika#2205).
    #[test]
    fn mika2921_lu_vide_et_non_lu_sont_deux_populations() {
        let flights = [fixture_pr_opened()];
        let lu_vide = &evaluate_flights(&flights, &[sub_issue(2693, None)])[0];
        let jamais_lu = &evaluate_flights(&flights, &[])[0];

        assert_eq!(lu_vide.pr_state, PrStateKnowledge::Read(None));
        assert_eq!(jamais_lu.pr_state, PrStateKnowledge::NotRead);
        assert_ne!(
            lu_vide.pr_state, jamais_lu.pr_state,
            "« lu, et vide » n'est pas « pas lu » — un Option les aurait fusionnés"
        );
        assert_eq!(
            lu_vide.recommendation, jamais_lu.recommendation,
            "la recommandation est bien la même : c'est le rendu qui porte la différence"
        );
    }

    /// Le dépôt traverse jusqu'au verdict, et vient du vol — jamais de la liste
    /// interrogée (mika#2921).
    #[test]
    fn mika2921_le_verdict_porte_le_depot_du_vol() {
        let v = &evaluate_flights(&[fixture_pr_opened()], &[])[0];
        assert_eq!(v.repo.as_deref(), Some("senara-solutions/mika"));

        let mut sans_depot = fixture_pr_opened();
        sans_depot.repo = None;
        sans_depot.reference_url = None;
        let v = &evaluate_flights(&[sans_depot], &[])[0];
        assert_eq!(
            v.repo, None,
            "un vol qui ne nomme aucun dépôt ne s'en voit pas attribuer un"
        );
    }

    // ---- Les URLs --------------------------------------------------------

    #[test]
    fn mika2698_le_numero_de_ticket_tolere_le_suffixe_de_grooming() {
        assert_eq!(
            issue_number_from_url(Some(
                "https://github.com/senara-solutions/mika/issues/2698?phase=groom"
            )),
            Some(2698)
        );
        assert_eq!(
            issue_number_from_url(Some("https://github.com/senara-solutions/mika/issues/2698")),
            Some(2698)
        );
        assert_eq!(issue_number_from_url(None), None);
        assert_eq!(issue_number_from_url(Some("https://example.com/x")), None);
    }

    #[test]
    fn mika2698_les_urls_du_milestone_sont_la_forme_canonique() {
        let state = MilestoneState {
            milestone_ref: crate::milestone_manager::types::MilestoneRef {
                repo: "senara-solutions/mika".to_string(),
                number: 42,
            },
            title: "m".to_string(),
            description: String::new(),
            state: crate::milestone_manager::types::IssueState::Open,
            created_at: "2026-10-01T00:00:00Z".to_string(),
            due_on: None,
            last_activity_at: None,
            sub_issues: vec![sub_issue(2693, None), sub_issue(2661, None)],
            progress: Default::default(),
            recent_activity: vec![],
            executor_healthy: None,
            ready_pool: vec![],
            outside_candidates: vec![],
            outside_read_truncated: None,
            plan_sizes: Default::default(),
        };
        assert_eq!(
            milestone_issue_urls(&state),
            vec![
                "https://github.com/senara-solutions/mika/issues/2693".to_string(),
                "https://github.com/senara-solutions/mika/issues/2661".to_string(),
            ]
        );
    }
}

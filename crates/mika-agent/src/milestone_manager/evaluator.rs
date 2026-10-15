//! Evaluator — le verdict du manager sur chaque vol d'implémentation terminé
//! (mika#2698, Phase 2 (b)).
//!
//! LECTURE SEULE, et à un degré plus fort que le reste du module : ce fichier
//! n'a pas d'interrupteur d'écriture OFF, il n'a **aucun chemin d'écriture du
//! tout**. Le verdict qu'il produit a deux destinations possibles, et une seule
//! par défaut : [`super::types::Assessment::flight_verdicts`], que le `Reporter`
//! rend dans le rapport — c'est le *mode ombre* du ticket. La seconde,
//! `MIKA_MANAGER_EVALUATE_WRITE` ouverte, est le commentaire que
//! [`super::verdict_write`] poste sur le ticket ; l'interrupteur, le geste et
//! l'exception nommée dans l'allowlist vivent là-bas, **pas ici**. Un évaluateur
//! qui porterait son propre interrupteur d'écriture pourrait écrire : celui-ci
//! ne peut pas.
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
//! | plafond de tours | la ligne `Turn ceiling: N` de `tasks.result` (mika#2698 vol 2) |
//! | état de la PR | [`SubIssue::pr_state`] — déjà hydraté par le `Reader` via `gh`, aucun appel de plus |
//!
//! **Le plafond est lu, jamais recalculé.** `_pilot_max_turns` /
//! `PILOT_LABEL_TURN_CEILINGS` (`skills/bundled/_shared/dispatch-lib.sh`)
//! déclarent un lecteur unique : le dupliquer en Rust serait le second lecteur
//! que cette déclaration refuse. Le vol 2 a donc fait écrire la valeur par le
//! **producteur**, à côté de `Turns:`, et ce module la lit comme il lit tout le
//! reste. Un dispatch désarmé écrit `Turn ceiling: unbounded` — qui ne parse
//! pas, donc rend `None`, qui est la lecture juste : il n'y avait pas de borne.
//!
//! **Le tour du dernier commit, lui, reste non instrumenté.**
//! [`FlightVerdict::last_commit_turn`] est **toujours `None`** : aucune des
//! sources que le ticket autorise ne le porte. `tasks.result` donne les commits
//! par sha (`Commits: <pre>..<post>`) et jamais par tour, et le JSON de sortie
//! du pilote n'expose que `turns` / `cost_usd` / `duration_ms` /
//! `session_id` — le dériver demanderait de parser le transcript de session
//! dans un format qu'aucun test de ce dépôt ne pin, donc de deviner. `None` dit
//! *non instrumenté*, jamais `0` (motif mika#2331), et le rapport le dit à voix
//! haute plutôt que de laisser un chiffre inventé passer pour une mesure.

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
    /// Le ticket sur lequel le vol portait, lu dans `reference_url`.
    pub issue_number: Option<u64>,
    pub issue: FlightIssue,
    /// Tours consommés. `None` = non instrumenté, jamais `0`.
    pub turns_used: Option<u32>,
    /// Vrai quand le vol a heurté `budget_exhausted`.
    pub hit_ceiling: bool,
    /// Le plafond lui-même, **lu** dans la ligne `Turn ceiling:` que le
    /// producteur écrit (mika#2698 vol 2), jamais recalculé ici : le lecteur
    /// unique de `PILOT_LABEL_TURN_CEILINGS` est en shell. `None` = résultat
    /// antérieur à cette ligne, ou dispatch sans borne (`unbounded`).
    pub turn_ceiling: Option<u32>,
    /// Tour du dernier commit. **Toujours `None`** — aucune source autorisée ne
    /// le porte ; voir le docstring du module.
    pub last_commit_turn: Option<u32>,
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
    pub status: String,
    pub result: Option<String>,
    /// JSON brut de `tasks.metadata` ; `claude_pilot` est lu là-dedans.
    pub metadata: Option<String>,
    pub reference_url: Option<String>,
    pub completed_at: Option<String>,
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
    /// Vols d'implémentation terminés depuis `since` (exclu), pour les issues
    /// données. Lecture seule. Jamais d'erreur visible : un échec rend `vec![]`
    /// et un WARN — un rapport sans section vaut mieux qu'un cycle mort.
    async fn finished_implement_flights(
        &self,
        since: Option<&str>,
        issue_urls: &[String],
    ) -> Vec<FinishedFlight>;
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
pub fn evaluate_flights(flights: &[FinishedFlight], sub_issues: &[SubIssue]) -> Vec<FlightVerdict> {
    flights
        .iter()
        .map(|f| {
            let issue_number = issue_number_from_url(f.reference_url.as_deref());
            let pr_state = issue_number
                .and_then(|n| sub_issues.iter().find(|s| s.number == n))
                .and_then(|s| s.pr_state.as_deref());
            evaluate_flight(f, issue_number, pr_state)
        })
        .collect()
}

/// Le verdict d'un vol, `pr_state` étant l'état de la PR tel que le `Reader` l'a
/// déjà hydraté (`open` / `closed` / `merged`, ou `None` s'il n'y en a pas).
pub fn evaluate_flight(
    flight: &FinishedFlight,
    issue_number: Option<u64>,
    pr_state: Option<&str>,
) -> FlightVerdict {
    let result = flight.result.as_deref().unwrap_or("");
    let pilot = PilotFields::parse(flight.metadata.as_deref());
    let issue = classify_issue(result, &pilot);
    FlightVerdict {
        task_id: flight.task_id.clone(),
        issue_number,
        recommendation: recommend(&issue, pr_state),
        issue,
        turns_used: pilot.turns.or_else(|| {
            line_value(result, "Turns:").and_then(|v| v.split_whitespace().next()?.parse().ok())
        }),
        hit_ceiling: hit_ceiling(result),
        // mika#2698 vol 2 — le plafond vient du producteur, qui est le lecteur
        // unique de `PILOT_LABEL_TURN_CEILINGS`. `Turn ceiling: unbounded` (le
        // dispatch désarmé) ne parse pas, donc rend `None` : c'est la lecture
        // juste, il n'y avait pas de borne.
        turn_ceiling: line_value(result, "Turn ceiling:").and_then(|v| v.parse().ok()),
        // Le seul datum du verdict qu'aucune source n'offre encore — voir le
        // docstring du module. Présent-et-`None` dit « non instrumenté » ; `0`
        // dirait « mesuré à zéro » (motif mika#2331).
        last_commit_turn: None,
        completed_at: flight.completed_at.clone(),
    }
}

/// La matrice de recommandation, dérivée de l'issue et de l'état de la PR.
///
/// `pr_state: None` sur une PR ouverte rend `MergeCandidate` : la seule raison
/// de s'en écarter serait une preuve que la PR est déjà fusionnée ou fermée, et
/// la recommandation est de toute façon *« va regarder »*, pas un geste.
fn recommend(issue: &FlightIssue, pr_state: Option<&str>) -> FlightRecommendation {
    match issue {
        FlightIssue::PrOpened { .. } => match pr_state {
            // Fusionnée ou fermée pendant que le vol volait : le verdict du vol
            // est en retard sur la réalité, un humain tranche.
            Some(s) if !s.eq_ignore_ascii_case("open") => FlightRecommendation::ManualResumption,
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
            status: "completed".to_string(),
            result: Some(
                "Session: 5b6f13dc\n\
                 Turns: 83\n\
                 Turn ceiling: 150\n\
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
        }
    }

    /// Un sauvetage mika#1282 : worktree sale, zéro commit, PR de brouillon
    /// ouverte, issue restée `PIPELINE_INCOMPLETE`.
    fn fixture_rescue_pr() -> FinishedFlight {
        FinishedFlight {
            task_id: "task-rescue".to_string(),
            status: "failed".to_string(),
            result: Some(
                "PIPELINE FAILURE: dirty worktree with zero commits\n\
                 Session: 7c1a\n\
                 Turns: 141\n\
                 Turn ceiling: 200\n\
                 PR: https://github.com/senara-solutions/mika/pull/2690\n\
                 Outcome: PIPELINE_INCOMPLETE — manual recovery needed.\n"
                    .to_string(),
            ),
            metadata: Some(r#"{"claude_pilot":{"turns":141,"pr_number":2690}}"#.to_string()),
            reference_url: Some("https://github.com/senara-solutions/mika/issues/2655".to_string()),
            completed_at: Some("2026-10-13T09:00:00Z".to_string()),
        }
    }

    /// Une mort au plafond : `_halt_family` donne `budget_exhausted` à
    /// `error_max_turns`, et aucune PR n'a été ouverte.
    fn fixture_dead_at_ceiling() -> FinishedFlight {
        FinishedFlight {
            task_id: "task-ceiling".to_string(),
            status: "failed".to_string(),
            result: Some(
                "PIPELINE FAILURE: pilot halted\n\
                 Session: 9f2b\n\
                 Turns: 200\n\
                 Turn ceiling: 200\n\
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
            ci_state: crate::milestone_manager::types::CiState::None,
            blockers: vec![],
            updated_at: "2026-10-13T00:00:00Z".to_string(),
            labels: vec![],
        }
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
    fn mika2698_le_plafond_vient_de_la_ligne_du_producteur() {
        // Les trois issues de l'AC3, avec le plafond lu sur chacune : c'est
        // `tours consommés SUR LE PLAFOND` qui devient lisible, et la paire
        // 200/200 du troisième fixture est la seule forme où « 200 tours » se
        // laisse vérifier au lieu d'être crue.
        for (f, expected) in [
            (fixture_pr_opened(), 150u32),
            (fixture_rescue_pr(), 200),
            (fixture_dead_at_ceiling(), 200),
        ] {
            let v = evaluate_flight(&f, None, None);
            assert_eq!(v.turn_ceiling, Some(expected), "{}", f.task_id);
        }
    }

    #[test]
    fn mika2698_un_dispatch_desarme_na_pas_de_plafond_plutot_que_zero() {
        // `Turn ceiling: unbounded` est ce que le producteur écrit quand
        // `_pilot_max_turns` ne pose rien. Ne pas parser est la bonne réponse :
        // il n'y avait pas de borne, et `0` dirait « borne à zéro tour ».
        let mut f = fixture_pr_opened();
        f.result = Some("Turns: 83\nTurn ceiling: unbounded\n".to_string());
        assert_eq!(evaluate_flight(&f, None, None).turn_ceiling, None);
    }

    #[test]
    fn mika2698_un_resultat_anterieur_au_producteur_na_pas_de_plafond() {
        // Les vols terminés AVANT que `dispatch-lib.sh` n'écrive la ligne
        // existent en base et seront évalués. Leur plafond est inconnu, pas
        // nul — sans quoi le premier cycle après déploiement rendrait une
        // colonne de `0/0 tours` sur tout l'historique.
        let mut f = fixture_dead_at_ceiling();
        f.result = Some(f.result.unwrap().replace("Turn ceiling: 200\n", ""));
        let v = evaluate_flight(&f, None, None);
        assert_eq!(v.turn_ceiling, None);
        assert!(v.hit_ceiling, "la halte reste lisible sans le plafond");
    }

    #[test]
    fn mika2698_le_tour_du_dernier_commit_reste_non_instrumente() {
        // Aucune source autorisée par le ticket ne le porte : `tasks.result`
        // donne les commits par sha, et le JSON du pilote n'expose que
        // turns/cost/duration/session. `None` dit « non mesuré » — un `0`
        // dirait « mesuré à zéro » (motif mika#2331). Voir le docstring du
        // module : c'est une absence tenue, pas un oubli.
        for f in [
            fixture_pr_opened(),
            fixture_rescue_pr(),
            fixture_dead_at_ceiling(),
        ] {
            let v = evaluate_flight(&f, None, None);
            assert_eq!(v.last_commit_turn, None, "{}", f.task_id);
        }
    }

    // ---- Les tours : les deux sources, et leur ordre ----------------------

    #[test]
    fn mika2698_les_tours_retombent_sur_la_ligne_turns_sans_metadata() {
        let mut f = fixture_pr_opened();
        f.metadata = None;
        let v = evaluate_flight(&f, None, Some("open"));
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
        assert_eq!(evaluate_flight(&f, None, None).turns_used, None);
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
        assert_eq!(evaluate_flight(&f, None, None).turns_used, Some(200));
    }

    // ---- Les causes de mort, une par forme du producteur -----------------

    fn dead_cause_of(result: &str) -> DeathCause {
        let f = FinishedFlight {
            task_id: "t".to_string(),
            result: Some(result.to_string()),
            ..Default::default()
        };
        match evaluate_flight(&f, None, None).issue {
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
        let v = evaluate_flight(&f, None, None);
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
        let v = evaluate_flight(&f, Some(2655), Some("open"));
        assert!(v.hit_ceiling);
        assert!(matches!(v.issue, FlightIssue::RescuePr { .. }));
        assert_eq!(v.recommendation, FlightRecommendation::ManualResumption);
    }

    // ---- La matrice de recommandation, exhaustivement ---------------------

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

        let cases: &[(FlightIssue, Option<&str>, FlightRecommendation)] = &[
            (pr(), Some("open"), FlightRecommendation::MergeCandidate),
            (pr(), None, FlightRecommendation::MergeCandidate),
            (pr(), Some("merged"), FlightRecommendation::ManualResumption),
            (pr(), Some("closed"), FlightRecommendation::ManualResumption),
            (
                rescue(),
                Some("open"),
                FlightRecommendation::ManualResumption,
            ),
            (
                rescue(),
                Some("merged"),
                FlightRecommendation::ManualResumption,
            ),
            (rescue(), None, FlightRecommendation::ManualResumption),
            (
                dead(DeathCause::CeilingReached),
                None,
                FlightRecommendation::Regroom,
            ),
            (
                dead(DeathCause::Refusal),
                None,
                FlightRecommendation::Regroom,
            ),
            (
                dead(DeathCause::Escalated),
                None,
                FlightRecommendation::Regroom,
            ),
            (
                dead(DeathCause::PipelineFailure),
                None,
                FlightRecommendation::ManualResumption,
            ),
            (
                dead(DeathCause::LauncherDead),
                None,
                FlightRecommendation::ManualResumption,
            ),
            (
                dead(DeathCause::Unknown("x".to_string())),
                None,
                FlightRecommendation::ManualResumption,
            ),
        ];
        for (issue, pr_state, expected) in cases {
            assert_eq!(
                recommend(issue, *pr_state),
                *expected,
                "issue={issue:?} pr_state={pr_state:?}"
            );
        }
    }

    #[test]
    fn mika2698_letat_de_pr_vient_du_sous_ticket_apparie() {
        // Le même vol, deux milestones : seul l'état hydraté par le Reader
        // change le verdict. Aucun appel `gh` de plus.
        let flights = [fixture_pr_opened()];
        assert_eq!(
            evaluate_flights(&flights, &[sub_issue(2693, Some("merged"))])[0].recommendation,
            FlightRecommendation::ManualResumption
        );
        assert_eq!(
            evaluate_flights(&flights, &[sub_issue(2693, Some("open"))])[0].recommendation,
            FlightRecommendation::MergeCandidate
        );
        assert_eq!(
            evaluate_flights(&flights, &[sub_issue(9999, Some("merged"))])[0].recommendation,
            FlightRecommendation::MergeCandidate,
            "un sous-ticket qui n'apparie pas ne doit pas prêter son état"
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

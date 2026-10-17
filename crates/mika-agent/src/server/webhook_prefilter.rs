//! Pré-filtre déterministe des événements webhook sans suite possible
//! (mika#2675, phase 1).
//!
//! Mesuré le 2026-10-07 : une `check_suite` verte sur `main` coûte deux tours
//! LLM (mika-dev et mika-qa, ≈ 100 k tokens d'entrée à eux deux) qui concluent
//! « CI green on main. No action », et un label sans effet coûte un tour
//! mika-dev qui conclut « pas de `ready`, pas de dispatch ». Le moteur sait
//! les deux choses sans modèle.
//!
//! # Placement : après les handlers déterministes, avant `run_agent`
//!
//! Les handlers structurels gardent tous leurs effets (évaluation de merge,
//! notification, nettoyage de lignes). Ce module ne change pas ce que le moteur
//! FAIT, seulement si l'on demande ENSUITE au modèle. Les deux classes de la
//! phase 1 exigent en plus un texte intact : un handler qui a remplacé ou
//! enrichi `req.text` a quelque chose à dire au modèle, et le tour a lieu.
//!
//! # Fail-safe vers le LLM
//!
//! Interrupteur désarmé, texte touché, recherche d'état en erreur : le tour a
//! lieu comme avant. On paie, on ne perd rien.
//!
//! # Phase 2 : la classe (c), `pr_review` non actionnable sans tâche
//!
//! Le `verdict_handler` sait déjà, en code, qu'aucune tâche n'est active pour
//! la PR : il notifie l'opérateur d'un `hold[review]` (mika#2667 AC2), refuse le
//! `pass` d'une identité qui n'est pas le relecteur (mika#2667 AC3), traite une
//! ligne VERDICT illisible en `hold` par défaut — puis rend quand même la main
//! au modèle, qui conclut « stale — no action ». Ces chemins enrichissent ou
//! remplacent TOUJOURS le texte, donc la règle « texte intact » de la phase 1
//! les excluait par construction. Pour (c), le texte doit être intact APRÈS le
//! `verdict_handler` : lui a déjà dit au modèle ce qu'il avait à dire, et ce
//! n'est rien d'actionnable ; un handler ultérieur qui toucherait le texte
//! garde le tour.
//!
//! Une revue SANS ligne `VERDICT:` n'est pas un verdict : c'est la forme d'une
//! consigne écrite en revue, et l'identité GitHub de l'opérateur est partagée
//! (AC2, « un commentaire n'est jamais filtré »). Elle n'entre pas dans (c).
//!
//! # Phase 3 : la classe (b), `check_suite` verte sur une tête déjà décidée
//!
//! Mesuré le 2026-10-07 : les traitements d'une même tête sont espacés de
//! plusieurs minutes, donc au-delà de la dédup de 60 s de `ci_success_handler`.
//! Une `check_suite` tardive retraite la tête, redécide DECISION-CORE, renotifie
//! l'opérateur, et le tour LLM qui suit conclut « Already notified ». Les
//! retours de dédup (étapes 2b/2c) ne portent, eux, aucune trace de ce que la
//! tête a décidé : ils gardent leur tour.
//!
//! Trois termes, tous lus dans une trace durable, et la fonction pure
//! [`head_decided_and_notified`] les porte tous :
//! - **décision attestée** — CET événement a écrit l'attestation
//!   `ci_success_handler_decision_core_hold` (le marqueur `processed`, écrit
//!   avant l'évaluation, ne suffit pas) ;
//! - **même tête** — une attestation d'un AUTRE événement porte exactement la
//!   même cible `pr:{repo}#{n}@{sha}` ;
//! - **notification attestée** — cette attestation antérieure dit `notified`.
//!
//! Le texte ne doit avoir été touché que par `ci_success_handler`, comme (c)
//! l'exige du `verdict_handler`. Une variante et ses faits, pas un second
//! mécanisme.
//!
//! # Phase 4 : les sorties « rien à faire » du CI failure handler (mika#2683)
//!
//! Mesuré le 2026-10-12 : une `check_suite` ROUGE sur `main` coûte deux tours
//! (116 k tokens d'entrée) qui concluent « No action », alors que l'étape 2 du
//! handler a déjà tranché la même chose en code ; et la CI rouge de la PR
//! dependabot mika#2682, non adoptée, en coûte un de plus (55 754 tokens
//! d'entrée) APRÈS que `surface_for_adoption` a notifié l'opérateur et écrit sa
//! ligne d'audit. Deux formes, deux mécanismes :
//!
//! - **rouge sur la branche par défaut** — deux termes de texte (`conclusion ∈
//!   {failure, timed_out}`, `branch == DEFAULT_BRANCH`) et un terme d'état,
//!   `branch_has_active_task`, par symétrie avec la classe verte voisine.
//!   L'étape 2 du handler skippe, elle, inconditionnellement : la porte est donc
//!   STRICTEMENT plus conservatrice que lui, le bon sens de l'écart pour un
//!   dispositif fail-safe vers le LLM.
//! - **rouge sur une branche de PR** — la décision du handler passe par un
//!   appel GitHub (`find_open_pr`) et deux recherches de tâche. La re-dériver
//!   ici serait une divergence programmée, la classe que ce dépôt a déjà payée
//!   deux fois (mika#2158 `grooming_marker`, mika#2362 le seuil de retry
//!   recopié). On lit donc la trace durable que le handler a laissée pour CET
//!   événement — l'idiome de la phase 3 — et [`ci_failure_no_action_class`]
//!   porte le terme de refus : une ligne `ci_failure_handled` ou
//!   `ci_failure_escalated` lui fait rendre `None`, le handler a agi.
//!
//! Les sorties qui **informent** le modèle d'un état qu'il doit prendre en
//! compte (jeton GitHub absent, forge illisible, correctif déjà en vol,
//! escalade) n'écrivent aucune ligne terminale : c'est l'absence de trace qui
//! leur garde leur tour.
//!
//! # Le registre des tours webhook (mika#2680, vol 1)
//!
//! Tout ce qui précède décrit ce que la porte ÉCARTE. Cette section décrit ce
//! qu'elle LAISSE PASSER, et c'est un autre instrument. Le critère
//! d'acceptation de la mesure de mika#2680 — « refaire la mesure sur une
//! fenêtre comparable » — est resté ouvert deux fois (2026-10-07, 2026-10-08)
//! parce qu'il a fallu, chaque fois, rejoindre à la main `llm_calls` au
//! PREMIER message `user` de chaque session pour deviner l'événement
//! déclencheur. Une ligne nomme désormais ce que cette reconstruction
//! cherchait. Elle n'ajoute aucune classe de filtre : elle rend la mesure
//! prenable.
//!
//! [`WEBHOOK_TURN_CHARGED_EVENT`] est émise au seul endroit qui connaisse les
//! deux faits à la fois — le texte d'ORIGINE du webhook, et que le tour a
//! lieu. Quatre champs, un par question que la mesure a dû poser :
//!
//! - `trigger` — le terme de [`TriggerClass`], calculé sur le texte d'origine.
//!   Calculé sur `req.text`, il classerait `other` la classe (f) du ticket (un
//!   `pr_review` réécrit par le `verdict_handler`), c'est-à-dire précisément la
//!   population à chiffrer : c'est la mutation rouge d'AC2 ;
//! - `text_touched` — [`Touched::by_any_handler`], qui sépare « intact » de
//!   « réécrit ». Sans lui, (f) est indiscernable d'un `pr_review` ordinaire ;
//! - `candidate` — le variant de [`Candidate`] quand [`classify`] en rend un,
//!   **absent** sinon. Absent dit « l'événement n'est pas dans la population du
//!   pré-filtre », ce qui n'est pas la même affirmation que « il y est et a
//!   gardé son tour ». Jamais `0`, jamais `"none"` (motif mika#2331) ;
//! - `prefilter_enabled` — l'état de l'interrupteur, parce qu'une fenêtre où il
//!   est désarmé est exactement une fenêtre qu'on veut pouvoir mesurer.
//!
//! **Classer, pas décider.** [`trigger_class`] est pure : aucune lecture
//! d'état, aucun appel GitHub, aucune ligne de DB — testable sans bouchon. Elle
//! n'écrit pas non plus un seul littéral de grammaire : elle lit les constantes
//! de `mika_common::github_event_format`, source de vérité unique dont le test
//! cross-crate de `mika-gateway::github::tests` refuse la dérive.
//!
//! **Aucune ligne d'audit.** `audit_events` reste réservé aux DÉCISIONS du
//! moteur, et [`PREFILTER_SKIPPED_TOOL`] y garde son monopole. Un tour qui a
//! lieu n'est pas une décision, c'est un fait de coût : il va dans `tracing`,
//! sur `target: "mika::otel"`, et **dégaté** de `MIKA_STORE_LLM_CALLS` comme
//! `turn_usage` — une fenêtre où la télémétrie a été coupée pour réduire le
//! bruit est précisément celle qu'on ne veut pas perdre.
//!
//! **Contrôle positif (mika#2205, mika#2340).** Pour un `trace_id` de canal
//! `github`, un `turn_usage` présent SANS `webhook_turn_charged` se lit
//! « binaire en service antérieur au champ », jamais « aucun tour webhook ». La
//! chaîne est rebuild → deploy : tant que le binaire servi n'embarque pas ce
//! vol, le registre est vide et ce vide ne mesure rien.
//!
//! **Périmètre : les tours webhook.** L'émission n'a lieu que là où
//! `original_github_text` est `Some`, c'est-à-dire `req.channel == "github"`.
//! Telegram et les tours silencieux restent hors registre.
//!
//! # (e) `closed`/`merged` d'une PR sans tâche de boucle (mika#2680, vol 3)
//!
//! AC2 de mika#2675 excluait `closed`/`merged` du filtre **par principe**. Le
//! ticket a demandé de le rouvrir avec preuve, et la preuve est venue : n=2,
//! 193 k puis 279 k tokens d'entrée pour un tour déclenché par le merge d'une
//! PR qui ne portait aucune tâche de boucle. (e) est la jumelle de (a) —
//! déclencheur dans le texte, un seul terme d'état, fenêtre de texte la plus
//! stricte.
//!
//! **Ce que (e) lit, et ce qu'elle refuse de relire.** Du texte : l'URL de la
//! PR, par l'extracteur de `upstream_close_handler` — jamais une troisième
//! copie du scanner, parce que cette URL est la clé de `pr_has_active_task` et
//! que deux lectures qui dérivent, ce sont deux populations de tours qui ne se
//! soustraient plus. De l'état : `pr_has_active_task` et rien d'autre. Pas
//! `Merged:`, pas l'état forge, pas les refs de clôture : la vérité du merge et
//! la clôture du ticket appartiennent aux handlers structurels, qui tournent
//! **avant** cette porte — écarter le tour ne leur retire rien. Une URL
//! inextractible ou une lecture en erreur rendent le tour (AC3).
//!
//! **Le coût accepté, nommé plutôt que passé sous silence.** Le tour
//! `PR closed` ne faisait pas *rien* : comme tout tour, il pouvait écrire de la
//! mémoire et toucher le fil de priorités. Trois raisons de l'accepter ici,
//! quand la classe (h) du ticket le refuse : l'opérateur a rangé (e) du côté à
//! implémenter à n=2 ; un merge de PR est rare et déjà entièrement tracé en
//! base par `upstream_close_handler`, là où un `issue_comment` porte du texte
//! humain neuf ; et le geste est réversible — `MIKA_WEBHOOK_PREFILTER` désarme
//! le filtre entier, et chaque tour écarté écrit sa classe, donc
//! `skipped_by_class` chiffrera (e) dès la première fenêtre mesurée et l'écart
//! avec les 193 k / 279 k attendus sera lisible plutôt que perdu.
//!
//! **Pourquoi la mesure 3 ne la conditionne pas.** Le critère d'acceptation du
//! ticket existe pour *soustraire ce qu'une classe déjà livrée écarte*. Aucune
//! classe de (a) à (d) ne touche le déclencheur `pr_closed` : son
//! `skipped_by_class` ne peut être qu'un seau vide, et une soustraction par zéro
//! n'informe rien. (g) est au contraire bloquée par cette mesure, parce que son
//! déclencheur est exactement celui que (b) traite.

use tracing::{info, warn};

use mika_common::github_event_format::{
    BRANCH_MARKER, CHECK_SUITE_EVENT_PREFIX, ISSUE_CLOSED_EVENT_PREFIX, ISSUE_COMMENT_EVENT_PREFIX,
    ISSUE_EVENT_PREFIX, ISSUE_LABELED_EVENT_PREFIX, PR_CLOSED_EVENT_PREFIX, PR_EVENT_PREFIX,
    PR_OPENED_EVENT_PREFIX, PR_REVIEW_EVENT_PREFIX, PUSH_EVENT_PREFIX,
};

use crate::async_db::AsyncDatabase;

use super::ci_failure_handler::{
    CI_FAILURE_ESCALATED_TOOL, CI_FAILURE_HANDLED_TOOL, CI_FAILURE_NO_ACTION_NO_OPEN_PR,
    CI_FAILURE_NO_ACTION_TASK_NOT_IN_PROGRESS, CI_FAILURE_NO_ACTION_TOOL,
    SURFACE_FOR_ADOPTION_TOOL, branch_key, parse_check_suite_failure,
};
use super::ci_success_handler::{
    DECISION_CORE_HOLD_NOTIFIED, DECISION_CORE_HOLD_TOOL, parse_check_suite_success,
};
use super::verdict::{Verdict, parse_pr_review_event, parse_verdict, verdict_raw_value};
use super::webhook_queue_v2::{WebhookEventKind, classify_event};

/// Kill-switch, armé par défaut. Même table de vérité que les interrupteurs
/// de mika#2671 ([`crate::qa_head_supersession::parse_switch`]).
pub(crate) const PREFILTER_ENV: &str = "MIKA_WEBHOOK_PREFILTER";

/// `audit_events.tool_name` d'un événement écarté. **SOLE WRITER** :
/// [`skip_turn`].
pub(crate) const PREFILTER_SKIPPED_TOOL: &str = "webhook_prefilter_skipped";

/// Nom d'événement `tracing` d'un tour webhook qui A LIEU (mika#2680).
/// **SEUL ÉCRIVAIN** : [`emit_turn_charged`].
///
/// Ce n'est délibérément PAS un `tool_name` d'audit, contrairement à son
/// voisin ci-dessus : un tour qui a lieu n'est pas une décision du moteur.
pub(crate) const WEBHOOK_TURN_CHARGED_EVENT: &str = "webhook_turn_charged";

/// La branche par défaut des dépôts que la boucle sert.
pub(crate) const DEFAULT_BRANCH: &str = "main";

/// Délai de la lecture de l'état de PR côté forge. Au-delà, l'état est
/// illisible et le tour a lieu (AC3).
const PR_STATE_TIMEOUT_SECS: u64 = 15;

/// Une classe d'événement écartée sans LLM. Valeur de `after_value` dans
/// l'audit : un format de fil, épinglé par test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrefilterClass {
    /// (a) `check_suite` verte sur la branche par défaut, sans tâche active.
    GreenCheckSuiteDefaultBranch,
    /// (d) label qu'aucune partie de la boucle ne lit.
    InertLabel,
    /// (c) `hold[review]` sans tâche : `hold_review_without_task` l'a déjà
    /// remis à l'opérateur (notification + ligne d'audit).
    VerdictHoldTracked,
    /// (c) `pass` d'une identité qui n'est pas le relecteur QA.
    VerdictNonReviewer,
    /// (c) ligne `VERDICT:` présente mais illisible.
    VerdictUnreadable,
    /// (c) PR fermée ou mergée côté forge.
    VerdictPrClosed,
    /// (b) `check_suite` verte sur une tête de PR déjà décidée DECISION-CORE
    /// et notifiée, que cet événement redécide à l'identique.
    GreenCheckSuiteHeadDecided,
    /// (a rouge) `check_suite` en échec ou `timed_out` sur la branche par
    /// défaut, sans tâche active sur cette branche (mika#2683).
    RedCheckSuiteDefaultBranch,
    /// (c) `check_suite` rouge d'une branche sans PR ouverte.
    CiFailureNoOpenPr,
    /// (c) `check_suite` rouge d'une PR sans tâche : `surface_for_adoption` a
    /// déjà notifié l'opérateur et écrit sa ligne d'audit (mika#1745).
    CiFailureSurfacedForAdoption,
    /// (c) `check_suite` rouge d'une PR dont la tâche n'est pas `in_progress`.
    CiFailureTaskNotInProgress,
    /// (e) `closed` ou `merged` d'une PR sans tâche de boucle active
    /// (mika#2680). Mesurée à n=2 : 193 k puis 279 k tokens d'entrée pour un
    /// tour qui n'avait rien à décider.
    PrClosedNoLoopTask,
}

impl PrefilterClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::GreenCheckSuiteDefaultBranch => "green_check_suite_default_branch",
            Self::InertLabel => "inert_label",
            Self::VerdictHoldTracked => "verdict_hold_tracked",
            Self::VerdictNonReviewer => "verdict_non_reviewer",
            Self::VerdictUnreadable => "verdict_unreadable",
            Self::VerdictPrClosed => "verdict_pr_closed",
            Self::GreenCheckSuiteHeadDecided => "green_check_suite_head_decided",
            Self::RedCheckSuiteDefaultBranch => "red_check_suite_default_branch",
            Self::CiFailureNoOpenPr => "ci_failure_no_open_pr",
            Self::CiFailureSurfacedForAdoption => "ci_failure_surfaced_for_adoption",
            Self::CiFailureTaskNotInProgress => "ci_failure_task_not_in_progress",
            Self::PrClosedNoLoopTask => "pr_closed_no_loop_task",
        }
    }
}

/// Ce que le texte dit d'un verdict, avant consultation de l'état. Chaque
/// champ est un terme de « non actionnable » ; aucun ne suffit sans l'absence
/// de tâche active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReviewVerdictFacts {
    pub(crate) repo: String,
    pub(crate) pr_number: u64,
    pub(crate) pr_url: String,
    /// `hold[review]` — le seul `hold` que le handler remet à l'opérateur.
    /// Un `hold[x]` inconnu n'est suivi par personne et garde son tour.
    pub(crate) hold_tracked: bool,
    /// `pass` dont l'auteur n'est pas `REVIEWER_FORGE_LOGIN` : le seul verdict
    /// de non-relecteur que le handler referme (refus + audit, mika#2667 AC3).
    /// Un `block[*]` ou un `hold[x]` d'une autre identité n'est notifié à
    /// personne — l'identité de l'opérateur est partagée — et garde son tour.
    pub(crate) non_reviewer_pass: bool,
    /// Ligne `VERDICT:` présente, valeur non reconnue.
    pub(crate) unreadable: bool,
}

/// Un événement qui POURRAIT être écarté, avant consultation de l'état.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Candidate {
    GreenDefaultBranch {
        repo: String,
        branch: String,
    },
    /// (b) toute `check_suite` verte hors branche par défaut. Elle n'est
    /// écartée qu'au terme de [`decide`], sur la trace laissée par le handler.
    GreenPrBranch {
        repo: String,
        branch: String,
    },
    /// (a rouge) `check_suite` en échec ou `timed_out` sur la branche par
    /// défaut (mika#2683).
    RedDefaultBranch {
        repo: String,
        branch: String,
    },
    /// (c) toute `check_suite` rouge hors branche par défaut. Elle n'est écartée
    /// qu'au terme de [`decide`], sur la trace laissée par le handler.
    RedPrBranch {
        repo: String,
        branch: String,
    },
    InertLabel {
        repo: String,
        issue: u64,
        label: String,
    },
    /// (c) toute revue portant une ligne `VERDICT:`. Le verdict n'est
    /// écarté qu'au terme de [`decide`].
    ReviewVerdict(ReviewVerdictFacts),
    /// (e) `closed` ou `merged` d'une PR dont l'URL est lisible (mika#2680).
    /// L'URL est le seul champ : c'est la clé de `pr_has_active_task`, et
    /// relire `Merged:` reviendrait à redériver une vérité qui appartient aux
    /// handlers structurels. Écartée au terme de [`decide`].
    PrClosedNoTask {
        pr_url: String,
    },
}

/// Fonction pure : classe le texte du gateway. `None` = hors population, le
/// tour a lieu. Lit les grammaires existantes, n'en écrit aucune.
pub(crate) fn classify(text: &str) -> Option<Candidate> {
    if let Some(event) = parse_check_suite_success(text) {
        return Some(if event.branch == DEFAULT_BRANCH {
            Candidate::GreenDefaultBranch {
                repo: event.repo,
                branch: event.branch,
            }
        } else {
            Candidate::GreenPrBranch {
                repo: event.repo,
                branch: event.branch,
            }
        });
    }
    if let Some(event) = parse_check_suite_failure(text) {
        return Some(if event.branch == DEFAULT_BRANCH {
            Candidate::RedDefaultBranch {
                repo: event.repo,
                branch: event.branch,
            }
        } else {
            Candidate::RedPrBranch {
                repo: event.repo,
                branch: event.branch,
            }
        });
    }
    if let Some(event) = parse_pr_review_event(text) {
        // Pas de ligne VERDICT : une revue-consigne, jamais filtrée (AC2).
        verdict_raw_value(&event.body)?;
        let verdict = parse_verdict(&event.body);
        let pr_url = event.pr_url();
        return Some(Candidate::ReviewVerdict(ReviewVerdictFacts {
            hold_tracked: matches!(&verdict, Verdict::Hold(r) if r.eq_ignore_ascii_case("review")),
            non_reviewer_pass: matches!(verdict, Verdict::Pass)
                && !mika_common::forge_identity::is_reviewer_forge_login(&event.reviewer),
            unreadable: matches!(verdict, Verdict::Missing { .. }),
            repo: event.repo,
            pr_number: event.pr_number,
            pr_url,
        }));
    }
    if text.starts_with(PR_CLOSED_EVENT_PREFIX) {
        // L'URL est l'unique terme de texte, et elle est obligatoire : un
        // événement dont on ne sait pas de quelle PR il parle n'est pas un
        // candidat, il est un tour (fail-safe, AC3 de mika#2675). Le gateway
        // interpole `""` quand `html_url` manque, donc ce cas est une forme de
        // production, pas une forme de synthèse.
        return crate::server::upstream_close_handler::extract_pr_url(text)
            .map(|pr_url| Candidate::PrClosedNoTask { pr_url });
    }
    match classify_event(text) {
        WebhookEventKind::IssueLabeled { repo, issue, label }
            if !crate::webhook_dispatch::label_read_by_loop(&label) =>
        {
            Some(Candidate::InertLabel { repo, issue, label })
        }
        _ => None,
    }
}

/// L'état d'une PR côté forge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrForgeState {
    Open,
    Closed,
    Merged,
}

/// Lit `gh pr view --json state`. Toute autre forme est une erreur : un état
/// inconnu est illisible, jamais « ouvert » ni « fermé » (AC3).
pub(crate) fn parse_pr_state(json: &str) -> anyhow::Result<PrForgeState> {
    let value: serde_json::Value = serde_json::from_str(json.trim())?;
    match value.get("state").and_then(|s| s.as_str()) {
        Some("OPEN") => Ok(PrForgeState::Open),
        Some("CLOSED") => Ok(PrForgeState::Closed),
        Some("MERGED") => Ok(PrForgeState::Merged),
        other => anyhow::bail!("état de PR non reconnu : {other:?}"),
    }
}

/// Une ligne d'audit, réduite à ce que la classe (b) lit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuditFact {
    /// Rang d'écriture (`audit_events.id`) : seule une attestation strictement
    /// ANTÉRIEURE à celle de cet événement vaut « déjà décidée ».
    pub(crate) id: i64,
    pub(crate) tool: String,
    pub(crate) target: String,
    pub(crate) after: Option<String>,
    pub(crate) trace: Option<String>,
}

impl From<crate::db::AuditEvent> for AuditFact {
    fn from(e: crate::db::AuditEvent) -> Self {
        Self {
            id: e.id,
            tool: e.tool_name,
            target: e.target_key,
            after: e.after_value,
            trace: e.trace_id,
        }
    }
}

/// Au plus autant d'attestations antérieures lues pour une PR.
const PRIOR_HOLDS_LIMIT: u32 = 50;

/// Les recherches d'état du pré-filtre, injectées pour que les tests les
/// bouchonnent : aucun appel GitHub réel en test.
pub(crate) trait PrefilterState {
    async fn branch_has_active_task(&self, branch: &str) -> anyhow::Result<bool>;
    async fn pr_has_active_task(&self, pr_url: &str) -> anyhow::Result<bool>;
    async fn pr_state(&self, repo: &str, pr_number: u64) -> anyhow::Result<PrForgeState>;
    /// (b) les lignes d'audit écrites pour CET événement (son `trace_id`).
    async fn event_audit_rows(&self) -> anyhow::Result<Vec<AuditFact>>;
    /// (b) les attestations DECISION-CORE dont la cible commence par
    /// `pr_prefix` (`pr:{repo}#{n}@`), tous événements confondus.
    async fn decision_core_holds(&self, pr_prefix: &str) -> anyhow::Result<Vec<AuditFact>>;
}

/// L'état réel : base de l'agent, et `gh` pour la forge.
pub(crate) struct LiveState<'a> {
    pub(crate) db: &'a AsyncDatabase,
    pub(crate) github_token: Option<&'a str>,
    /// L'identité de l'événement, que les handlers posent en `trace_id`.
    pub(crate) request_id: &'a str,
}

impl PrefilterState for LiveState<'_> {
    async fn branch_has_active_task(&self, branch: &str) -> anyhow::Result<bool> {
        Ok(self.db.find_active_task_by_branch(branch).await?.is_some())
    }

    async fn pr_has_active_task(&self, pr_url: &str) -> anyhow::Result<bool> {
        Ok(self.db.find_active_task_by_pr_url(pr_url).await?.is_some())
    }

    async fn pr_state(&self, repo: &str, pr_number: u64) -> anyhow::Result<PrForgeState> {
        let Some(token) = self.github_token else {
            anyhow::bail!("aucun jeton GitHub résolu");
        };
        let number = pr_number.to_string();
        let args = ["pr", "view", &number, "--repo", repo, "--json", "state"];
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(PR_STATE_TIMEOUT_SECS),
            crate::tools::pr_merge_with_gate::run_gh_subprocess(&args, token),
        )
        .await
        .map_err(|_| anyhow::anyhow!("gh pr view : délai de {PR_STATE_TIMEOUT_SECS} s dépassé"))?
        .map_err(|e| anyhow::anyhow!(e))?;
        parse_pr_state(&output)
    }

    async fn event_audit_rows(&self) -> anyhow::Result<Vec<AuditFact>> {
        let rows = self
            .db
            .get_audit_events_by_trace_ids(vec![self.request_id.to_string()])
            .await?;
        Ok(rows.into_iter().map(AuditFact::from).collect())
    }

    async fn decision_core_holds(&self, pr_prefix: &str) -> anyhow::Result<Vec<AuditFact>> {
        let rows = self
            .db
            .get_audit_events_for_target_prefix(
                DECISION_CORE_HOLD_TOOL,
                pr_prefix,
                PRIOR_HOLDS_LIMIT,
            )
            .await?;
        Ok(rows.into_iter().map(AuditFact::from).collect())
    }
}

/// (b) Fonction pure, trois termes conjonctifs. Rend la tête quand CET
/// événement a redécidé DECISION-CORE sur `repo` (décision attestée) et qu'un
/// AUTRE événement l'avait déjà décidée AVANT lui sur la même cible (même
/// tête) et notifiée (notification attestée). Tout le reste garde son tour —
/// en particulier le premier traitement d'une tête, même si un traitement plus
/// tardif a écrit son attestation avant que le premier n'atteigne la porte.
pub(crate) fn head_decided_and_notified(
    repo: &str,
    this_event: &[AuditFact],
    prior: &[AuditFact],
) -> Option<String> {
    let mine = this_event
        .iter()
        .find(|r| r.tool == DECISION_CORE_HOLD_TOOL)?;
    if !mine.target.starts_with(&format!("pr:{repo}#")) {
        return None;
    }
    prior
        .iter()
        .any(|p| {
            p.tool == DECISION_CORE_HOLD_TOOL
                && p.trace != mine.trace
                && p.id < mine.id
                && p.target == mine.target
                && p.after.as_deref() == Some(DECISION_CORE_HOLD_NOTIFIED)
        })
        .then(|| mine.target.clone())
}

/// (c) Fonction pure sur les lignes d'audit de CET événement. Rend la classe
/// lorsqu'elles montrent une sortie terminale du CI failure handler — et `None`
/// dès qu'une ligne dit que le handler a AGI (`ci_failure_handled`,
/// `ci_failure_escalated`), auquel cas le tour a lieu. C'est le garde-fou qui
/// empêche la classe de mordre sur un événement où quelque chose a été fait.
///
/// Les trois noms lus le sont par constante, jamais par littéral recopié : la
/// même trace, lue par un second fichier. `surface_for_adoption` reste SOLE
/// WRITER de son nom (le garde porte sur le littéral, pas sur l'identifiant).
///
/// Un motif `ci_failure_no_action` que cette version ne connaît pas rend `None`
/// comme le reste : une valeur de fil ajoutée en amont ne fait pas écarter de
/// tour avant que la porte ne la connaisse.
pub(crate) fn ci_failure_no_action_class(this_event: &[AuditFact]) -> Option<PrefilterClass> {
    if this_event
        .iter()
        .any(|r| r.tool == CI_FAILURE_HANDLED_TOOL || r.tool == CI_FAILURE_ESCALATED_TOOL)
    {
        return None;
    }
    this_event.iter().find_map(|r| {
        if r.tool == SURFACE_FOR_ADOPTION_TOOL {
            return Some(PrefilterClass::CiFailureSurfacedForAdoption);
        }
        if r.tool != CI_FAILURE_NO_ACTION_TOOL {
            return None;
        }
        let motive = r.after.as_deref()?;
        if motive == CI_FAILURE_NO_ACTION_NO_OPEN_PR {
            Some(PrefilterClass::CiFailureNoOpenPr)
        } else if motive == CI_FAILURE_NO_ACTION_TASK_NOT_IN_PROGRESS {
            Some(PrefilterClass::CiFailureTaskNotInProgress)
        } else {
            None
        }
    })
}

/// `pr:{repo}#{n}@{sha}` → `pr:{repo}#{n}@`. `None` si la cible n'a pas de SHA.
fn pr_prefix(head: &str) -> Option<&str> {
    head.rfind('@').map(|i| &head[..=i])
}

/// L'issue de la décision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decision {
    Llm,
    Skip {
        class: PrefilterClass,
        target: String,
    },
}

fn state_unreadable(what: &str, target: &str, e: &anyhow::Error) -> Decision {
    warn!(
        event = "webhook_prefilter_state_unreadable",
        what,
        target = %target,
        error = %e,
        "pré-filtre : état illisible — le tour a lieu (fail-safe)"
    );
    Decision::Llm
}

/// Décide un candidat. Toute erreur de recherche laisse passer (AC3).
pub(crate) async fn decide(candidate: Candidate, state: &impl PrefilterState) -> Decision {
    match candidate {
        Candidate::GreenDefaultBranch { repo, branch } => {
            decide_default_branch(
                PrefilterClass::GreenCheckSuiteDefaultBranch,
                &repo,
                &branch,
                state,
            )
            .await
        }
        Candidate::RedDefaultBranch { repo, branch } => {
            decide_default_branch(
                PrefilterClass::RedCheckSuiteDefaultBranch,
                &repo,
                &branch,
                state,
            )
            .await
        }
        Candidate::GreenPrBranch { repo, branch } => {
            decide_head_decided(&repo, &branch, state).await
        }
        Candidate::RedPrBranch { repo, branch } => {
            decide_ci_failure_no_action(&repo, &branch, state).await
        }
        Candidate::InertLabel { repo, issue, .. } => Decision::Skip {
            class: PrefilterClass::InertLabel,
            target: format!("issue:{repo}#{issue}"),
        },
        Candidate::ReviewVerdict(facts) => decide_review_verdict(facts, state).await,
        Candidate::PrClosedNoTask { pr_url } => decide_pr_closed(pr_url, state).await,
    }
}

/// (a) verte et rouge sur la branche par défaut : deux classes, un seul terme
/// d'état. Une tâche active sur la branche est un état que le modèle doit
/// prendre en compte ; une lecture en erreur laisse passer (AC3).
async fn decide_default_branch(
    class: PrefilterClass,
    repo: &str,
    branch: &str,
    state: &impl PrefilterState,
) -> Decision {
    let target = branch_key(repo, branch);
    match state.branch_has_active_task(branch).await {
        Ok(false) => Decision::Skip { class, target },
        Ok(true) => Decision::Llm,
        Err(e) => state_unreadable("branch_task", &target, &e),
    }
}

/// (c) : la trace que le CI failure handler a laissée pour CET événement, lue
/// par [`ci_failure_no_action_class`]. Aucune autre recherche d'état — la
/// décision a déjà été prise en code, on ne la re-dérive pas. Un audit
/// illisible laisse passer (AC3).
async fn decide_ci_failure_no_action(
    repo: &str,
    branch: &str,
    state: &impl PrefilterState,
) -> Decision {
    let target = branch_key(repo, branch);
    let this_event = match state.event_audit_rows().await {
        Ok(rows) => rows,
        Err(e) => return state_unreadable("event_audit", &target, &e),
    };
    match ci_failure_no_action_class(&this_event) {
        Some(class) => Decision::Skip { class, target },
        None => Decision::Llm,
    }
}

/// (b) : la trace de CET événement d'abord (index sur `trace_id`, presque
/// toujours sans attestation), les attestations de la PR ensuite. Toute lecture
/// en erreur laisse passer (AC3).
async fn decide_head_decided(repo: &str, branch: &str, state: &impl PrefilterState) -> Decision {
    let pending = branch_key(repo, branch);
    let this_event = match state.event_audit_rows().await {
        Ok(rows) => rows,
        Err(e) => return state_unreadable("event_audit", &pending, &e),
    };
    let Some(prefix) = this_event
        .iter()
        .find(|r| r.tool == DECISION_CORE_HOLD_TOOL)
        .and_then(|r| pr_prefix(&r.target))
    else {
        return Decision::Llm;
    };
    let prior = match state.decision_core_holds(prefix).await {
        Ok(rows) => rows,
        Err(e) => return state_unreadable("decision_core_holds", prefix, &e),
    };
    match head_decided_and_notified(repo, &this_event, &prior) {
        Some(head) => Decision::Skip {
            class: PrefilterClass::GreenCheckSuiteHeadDecided,
            target: format!("check_suite:{}", head.strip_prefix("pr:").unwrap_or(&head)),
        },
        None => Decision::Llm,
    }
}

/// (c) : sans tâche active, ET l'un des quatre termes de « non actionnable ».
/// Les trois termes lus dans le texte passent avant la forge, pour ne payer
/// `gh` qu'au besoin.
async fn decide_review_verdict(facts: ReviewVerdictFacts, state: &impl PrefilterState) -> Decision {
    let target = format!("pr_review:{}#{}", facts.repo, facts.pr_number);
    match state.pr_has_active_task(&facts.pr_url).await {
        Ok(false) => {}
        Ok(true) => return Decision::Llm,
        Err(e) => return state_unreadable("pr_task", &target, &e),
    }
    let class = if facts.hold_tracked {
        PrefilterClass::VerdictHoldTracked
    } else if facts.non_reviewer_pass {
        PrefilterClass::VerdictNonReviewer
    } else if facts.unreadable {
        PrefilterClass::VerdictUnreadable
    } else {
        match state.pr_state(&facts.repo, facts.pr_number).await {
            Ok(PrForgeState::Closed | PrForgeState::Merged) => PrefilterClass::VerdictPrClosed,
            Ok(PrForgeState::Open) => return Decision::Llm,
            Err(e) => return state_unreadable("pr_state", &target, &e),
        }
    };
    Decision::Skip { class, target }
}

/// (e) : un seul terme d'état, `pr_has_active_task`, lu par la même porte que
/// (c) revue. Rien d'autre n'est consulté — ni `Merged:` dans le texte, ni
/// l'état forge, ni les refs de clôture : la clôture de ticket et la mémoire
/// passent par les handlers structurels, qui tournent AVANT le pré-filtre et à
/// qui écarter un tour ne retire rien. Une lecture en erreur laisse passer
/// (AC3).
async fn decide_pr_closed(pr_url: String, state: &impl PrefilterState) -> Decision {
    let target = format!("pr_closed:{pr_url}");
    match state.pr_has_active_task(&pr_url).await {
        Ok(false) => Decision::Skip {
            class: PrefilterClass::PrClosedNoLoopTask,
            target,
        },
        Ok(true) => Decision::Llm,
        Err(e) => state_unreadable("pr_has_active_task", &target, &e),
    }
}

/// L'interrupteur est-il armé ? Lu à chaque événement.
pub(crate) fn prefilter_enabled() -> bool {
    crate::qa_head_supersession::parse_switch(
        PREFILTER_ENV,
        std::env::var(PREFILTER_ENV).ok().as_deref(),
    )
}

/// Ce que les handlers ont fait du texte. (a) verte et (d) exigent un texte
/// intact ; (c) revue exige un texte intact APRÈS le `verdict_handler` ; (b)
/// exige qu'aucun handler autre que `ci_success_handler` n'y ait touché ; les
/// classes de mika#2683, qu'aucun autre que `ci_failure_handler`.
///
/// Un terme par fenêtre, et non un assouplissement du premier : le sens de ce
/// type est « quel handler a le droit d'avoir touché ce texte pour cette
/// classe ». Élargir `by_any_handler` ferait porter le relâchement à toutes les
/// classes, dont celles où un texte touché est précisément le signal de danger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Touched {
    pub(crate) by_any_handler: bool,
    pub(crate) after_verdict_handler: bool,
    pub(crate) outside_ci_success_handler: bool,
    /// mika#2683 : `surface_for_adoption` ENRICHIT `req.text`, donc
    /// `by_any_handler` et `outside_ci_success_handler` sont tous deux vrais dès
    /// que le handler a signalé — la fenêtre doit être son propre appel.
    pub(crate) outside_ci_failure_handler: bool,
}

impl Touched {
    fn applies_to(self, candidate: &Candidate) -> bool {
        match candidate {
            Candidate::ReviewVerdict(_) => self.after_verdict_handler,
            Candidate::GreenPrBranch { .. } => self.outside_ci_success_handler,
            Candidate::RedDefaultBranch { .. } | Candidate::RedPrBranch { .. } => {
                self.outside_ci_failure_handler
            }
            // (e) rejoint la fenêtre la plus stricte : la seule chose que (e)
            // lit dans le texte est l'URL de la PR, donc tout handler qui a
            // réécrit ce texte a pu réécrire la ligne qui porte la clé de
            // `pr_has_active_task` (mika#2680).
            Candidate::GreenDefaultBranch { .. }
            | Candidate::InertLabel { .. }
            | Candidate::PrClosedNoTask { .. } => self.by_any_handler,
        }
    }
}

/// La porte. Rend `true` quand le tour LLM est écarté ; une ligne `info!` et
/// une ligne d'audit nommées sont alors écrites.
pub(crate) async fn skip_turn(
    db: &AsyncDatabase,
    state: &impl PrefilterState,
    original_text: &str,
    touched: Touched,
    request_id: &str,
    enabled: bool,
) -> bool {
    if !enabled {
        return false;
    }
    let Some(candidate) = classify(original_text) else {
        return false;
    };
    if touched.applies_to(&candidate) {
        return false;
    }
    let Decision::Skip { class, target } = decide(candidate, state).await else {
        return false;
    };

    info!(
        event = PREFILTER_SKIPPED_TOOL,
        agent_id = %db.agent_id(),
        class = class.as_str(),
        target = %target,
        request_id,
        "événement sans suite possible — aucun tour LLM (mika#2675)"
    );
    if let Err(e) = db
        .log_audit_event(
            "system",
            PREFILTER_SKIPPED_TOOL,
            &target,
            None,
            Some(class.as_str()),
            Some(&format!("request_id={request_id}")),
            Some(request_id),
        )
        .await
    {
        warn!(
            event = "webhook_prefilter_audit_failed",
            target = %target,
            error = %e,
            "événement écarté mais ligne d'audit non écrite"
        );
    }
    true
}

// ---------------------------------------------------------------------------
// Le registre des tours webhook (mika#2680, vol 1)
// ---------------------------------------------------------------------------

/// Ce que le texte d'ORIGINE d'un webhook déclare être, avant tout handler.
///
/// Une taxonomie, pas une décision : aucun bras de ce type n'écarte quoi que
/// ce soit, et rien dans le moteur ne le lit. C'est la valeur du champ
/// `trigger` de [`WEBHOOK_TURN_CHARGED_EVENT`], donc un format de fil au même
/// titre que [`PrefilterClass::as_str`] : un opérateur en fait un `GROUP BY`,
/// et une orthographe qui change scinde une population sans le dire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TriggerClass {
    /// `check_suite` sur [`DEFAULT_BRANCH`], **toutes conclusions confondues**.
    /// La taxonomie nomme l'événement, pas son issue : le vert et le rouge se
    /// séparent par `candidate`, qui porte déjà cette distinction.
    CheckSuiteDefaultBranch,
    /// `check_suite` sur toute autre branche.
    CheckSuitePrBranch,
    /// `pull_request_review`, verdict lisible ou non.
    PrReview,
    /// `pull_request` action `opened`.
    PrOpened,
    /// `pull_request` action `closed`, **mergée ou non** : le gateway rend la
    /// même forme dans les deux cas et ne dit `Merged: {bool}` qu'en ligne
    /// suivante. La classe (e) du ticket vit ici.
    PrClosed,
    /// Toute autre action de `pull_request` (`synchronize`,
    /// `review_requested`, `ready_for_review`, …).
    PrOther,
    /// `issues` action `labeled`, label lu par la boucle ou non.
    IssueLabeled,
    /// `issues` action `closed`. Moitié de la classe (h) du ticket.
    IssueClosed,
    /// Toute autre action de `issues`, `opened` comprise — la taxonomie n'a
    /// pas de terme `issue_opened` et ne prétend pas en avoir un.
    IssueOther,
    /// `issue_comment`. L'autre moitié de (h), et 79 % de la fenêtre mesurée
    /// le 2026-10-08 à elle seule.
    IssueComment,
    /// `push`, tel qu'il arrive réellement — par l'attrape-tout du gateway,
    /// sous la forme `[GitHub] push.unknown on {repo}`.
    Push,
    /// Repli : l'attrape-tout de `format_event_text` pour un type d'événement
    /// qu'il ne met pas en forme. Aucun des six gabarits nommés du gateway ne
    /// doit y retomber (AC3).
    Other,
}

impl TriggerClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::CheckSuiteDefaultBranch => "check_suite_default_branch",
            Self::CheckSuitePrBranch => "check_suite_pr_branch",
            Self::PrReview => "pr_review",
            Self::PrOpened => "pr_opened",
            Self::PrClosed => "pr_closed",
            Self::PrOther => "pr_other",
            Self::IssueLabeled => "issue_labeled",
            Self::IssueClosed => "issue_closed",
            Self::IssueOther => "issue_other",
            Self::IssueComment => "issue_comment",
            Self::Push => "push",
            Self::Other => "other",
        }
    }
}

/// La branche d'un texte de `check_suite`, lue entre [`BRANCH_MARKER`] et le
/// `)` qui ferme la première ligne. Même idiome que `ci_success_handler`, à
/// ceci près que la conclusion n'est pas épinglée.
fn check_suite_branch(head: &str) -> Option<&str> {
    let at = head.find(BRANCH_MARKER)?;
    head[at + BRANCH_MARKER.len()..].strip_suffix(')')
}

/// Classe le texte d'ORIGINE d'un webhook. Fonction pure : aucune lecture
/// d'état, aucun appel GitHub, aucun littéral de grammaire.
///
/// **L'ordre des bras est porteur.** `PR review ` commence par `PR `, et
/// `Issue labeled ` commence par `Issue ` : chaque raffinement passe avant son
/// générique, faute de quoi `pr_review` et `issue_labeled` seraient
/// inatteignables — en silence, avec toutes les assertions de comportement
/// encore vertes. Le test cross-crate
/// `mika2680_chaque_prefixe_declare_est_celui_que_le_gateway_emet` épingle les
/// deux inclusions côté producteur.
///
/// Le terme de repli est [`TriggerClass::Other`] et il n'a pas de liste
/// d'exclusion : il appartient au bras attrape-tout du gateway.
pub(crate) fn trigger_class(text: &str) -> TriggerClass {
    // Chaque gabarit du gateway tient sa forme sur sa PREMIÈRE ligne ; l'URL,
    // le corps du ticket et la ligne `Labeled by:` suivent et ne décident rien.
    let head = text.lines().next().unwrap_or(text);

    if head.starts_with(CHECK_SUITE_EVENT_PREFIX) {
        return match check_suite_branch(head) {
            Some(branch) if branch == DEFAULT_BRANCH => TriggerClass::CheckSuiteDefaultBranch,
            Some(_) => TriggerClass::CheckSuitePrBranch,
            // Un `check_suite` dont la branche est illisible n'est ni l'un ni
            // l'autre. Le gateway en émet toujours une (`"unknown"` à défaut),
            // donc ce bras est inatteignable en production — et se taire y
            // vaut mieux que nommer une branche qu'on n'a pas lue.
            None => TriggerClass::Other,
        };
    }
    if head.starts_with(PR_REVIEW_EVENT_PREFIX) {
        return TriggerClass::PrReview;
    }
    if head.starts_with(PR_OPENED_EVENT_PREFIX) {
        return TriggerClass::PrOpened;
    }
    if head.starts_with(PR_CLOSED_EVENT_PREFIX) {
        return TriggerClass::PrClosed;
    }
    if head.starts_with(PR_EVENT_PREFIX) {
        return TriggerClass::PrOther;
    }
    if head.starts_with(ISSUE_COMMENT_EVENT_PREFIX) {
        return TriggerClass::IssueComment;
    }
    if head.starts_with(ISSUE_LABELED_EVENT_PREFIX) {
        return TriggerClass::IssueLabeled;
    }
    if head.starts_with(ISSUE_CLOSED_EVENT_PREFIX) {
        return TriggerClass::IssueClosed;
    }
    if head.starts_with(ISSUE_EVENT_PREFIX) {
        return TriggerClass::IssueOther;
    }
    if head.starts_with(PUSH_EVENT_PREFIX) {
        return TriggerClass::Push;
    }
    TriggerClass::Other
}

impl Candidate {
    /// Le nom de fil du variant, pour le champ `candidate` du registre. Même
    /// forme et même contrainte que [`PrefilterClass::as_str`].
    ///
    /// Vit avec le registre plutôt que sous la déclaration de [`Candidate`]
    /// parce que c'est le registre, et lui seul, qui le consomme : aucune
    /// décision du moteur ne lit ce nom.
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::GreenDefaultBranch { .. } => "green_default_branch",
            Self::GreenPrBranch { .. } => "green_pr_branch",
            Self::RedDefaultBranch { .. } => "red_default_branch",
            Self::RedPrBranch { .. } => "red_pr_branch",
            Self::InertLabel { .. } => "inert_label",
            Self::ReviewVerdict(_) => "review_verdict",
            Self::PrClosedNoTask { .. } => "pr_closed_no_task",
        }
    }
}

/// Émet [`WEBHOOK_TURN_CHARGED_EVENT`] : CE tour webhook a lieu. **SEUL
/// ÉCRIVAIN** de cette ligne.
///
/// `original_text` est le texte d'AVANT les handlers — la seule forme qui
/// classe correctement un événement qu'un handler a réécrit. Aucune ligne
/// d'audit n'est écrite ici, et la ligne n'est gatée par rien : voir la
/// section « Le registre des tours webhook » en tête de module.
pub(crate) fn emit_turn_charged(
    agent_id: &str,
    original_text: &str,
    touched: Touched,
    request_id: &str,
    enabled: bool,
) {
    let trigger = trigger_class(original_text).as_str();
    let text_touched = touched.by_any_handler;

    // Deux bras pour un seul champ de différence, parce que `candidate` doit
    // pouvoir MANQUER. Un `?candidate` sur un `Option` écrirait la chaîne
    // `"None"`, que le lecteur de vol 2 ne saurait pas distinguer d'une classe
    // nommée ainsi — et « absent » dit autre chose que « aucune » (motif
    // mika#2331). `tracing` ne sait pas omettre un champ conditionnellement
    // dans un seul appel ; les deux listes doivent donc rester identiques
    // champ pour champ.
    match classify(original_text) {
        Some(candidate) => info!(
            target: "mika::otel",
            event = WEBHOOK_TURN_CHARGED_EVENT,
            agent_id,
            request_id,
            trigger,
            text_touched,
            candidate = candidate.as_str(),
            prefilter_enabled = enabled,
            "tour webhook facturé — le tour a lieu (mika#2680)"
        ),
        None => info!(
            target: "mika::otel",
            event = WEBHOOK_TURN_CHARGED_EVENT,
            agent_id,
            request_id,
            trigger,
            text_touched,
            prefilter_enabled = enabled,
            "tour webhook facturé — le tour a lieu (mika#2680)"
        ),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const GREEN_MAIN: &str = "[GitHub] Check suite success on senara-solutions/mika (branch: main)";
    const INERT: &str = "[GitHub] Issue labeled p1-important on senara-solutions/mika#2675 — fix(webhooks): pré-filtre\nhttps://github.com/senara-solutions/mika/issues/2675\nLabeled by: @samidarko";

    /// (e) le gabarit réel du gateway pour `pull_request` (`github.rs`, bras
    /// `pull_request`) : l'URL de la PR est en **deuxième** ligne, et `closed`
    /// ajoute une troisième ligne `Merged: <bool>`. La classe (e) ne relit pas
    /// ce booléen — `closed` et `merged` sont le même événement pour elle, et
    /// la vérité du merge appartient aux handlers.
    const PR_CLOSED: &str = "[GitHub] PR closed: senara-solutions/mika#2679 — fix(webhooks): x (branch: fix/2679/x)\nhttps://github.com/senara-solutions/mika/pull/2679\nMerged: true";
    const PR_CLOSED_URL: &str = "https://github.com/senara-solutions/mika/pull/2679";
    /// Même gabarit, `html_url` absent : le gateway interpole `""`, donc la
    /// deuxième ligne est vide. Forme de production, pas forme de synthèse.
    const PR_CLOSED_SANS_URL: &str =
        "[GitHub] PR closed: senara-solutions/mika#2679 — t (branch: x)\n\nMerged: true";

    fn labeled(label: &str) -> String {
        format!("[GitHub] Issue labeled {label} on senara-solutions/mika#2675 — t\nhttps://x")
    }

    #[test]
    fn mika2675_classify_reconnait_les_deux_classes() {
        assert_eq!(
            classify(GREEN_MAIN),
            Some(Candidate::GreenDefaultBranch {
                repo: "senara-solutions/mika".into(),
                branch: "main".into()
            })
        );
        assert_eq!(
            classify(INERT),
            Some(Candidate::InertLabel {
                repo: "senara-solutions/mika".into(),
                issue: 2675,
                label: "p1-important".into()
            })
        );
    }

    /// (a) terme par terme : conclusion, branche, préfixe. Une branche autre
    /// que `main` sort de (a) et entre dans (b) ; un texte sans préfixe n'est
    /// candidat à rien (AC2). Depuis mika#2683, un échec et un `timed_out`
    /// entrent dans leur propre population — plus bas.
    #[test]
    fn mika2675_classify_a_chaque_terme_sort_de_la_population() {
        assert_eq!(
            classify("Check suite success on senara-solutions/mika (branch: main)"),
            None
        );
        for (text, branch) in [
            (
                "[GitHub] Check suite success on senara-solutions/mika (branch: fix/2675/x)",
                "fix/2675/x",
            ),
            (
                "[GitHub] Check suite success on senara-solutions/mika (branch: mainline)",
                "mainline",
            ),
        ] {
            assert_eq!(
                classify(text),
                Some(Candidate::GreenPrBranch {
                    repo: "senara-solutions/mika".into(),
                    branch: branch.into()
                }),
                "{text}"
            );
        }
    }

    /// (d) chaque famille de l'ensemble lu par la boucle sort de la population.
    #[test]
    fn mika2675_classify_d_chaque_label_lu_sort_de_la_population() {
        for label in [
            "ready",
            "Ready",
            "blocked",
            "operator-review",
            "operator-gated",
            "operator-iterate",
            "dispatch:loop",
            "dispatch:ssc",
            "loop-substrate",
            "needs-build",
            "wip-rescue",
            "phase:2",
            "origin:loop",
        ] {
            assert_eq!(classify(&labeled(label)), None, "{label}");
        }
        for label in ["bug", "p2-normal", "enhancement", "help wanted"] {
            assert!(classify(&labeled(label)).is_some(), "{label}");
        }
    }

    #[test]
    fn mika2675_classify_hors_population() {
        for text in [
            "[GitHub] PR opened: senara-solutions/mika#1 — t (branch: main)\nu",
            "[GitHub] New comment on senara-solutions/mika#1 (t) by @samidarko\nu\n\nfais-le",
            "[GitHub] Issue labeled: senara-solutions/mika#1 — t\nu",
            "bonjour",
        ] {
            assert_eq!(classify(text), None, "{text}");
        }
    }

    /// État bouchonné. `None` = la recherche ne doit pas être consultée :
    /// elle panique si elle l'est. Aucun appel GitHub réel.
    #[derive(Default)]
    pub(crate) struct Stub {
        pub(crate) branch_task: Option<Result<bool, &'static str>>,
        pub(crate) pr_task: Option<Result<bool, &'static str>>,
        pub(crate) pr_state: Option<Result<PrForgeState, &'static str>>,
        pub(crate) event_rows: Option<Result<Vec<AuditFact>, &'static str>>,
        pub(crate) holds: Option<Result<Vec<AuditFact>, &'static str>>,
    }

    fn answer_rows(
        slot: &Option<Result<Vec<AuditFact>, &'static str>>,
        what: &str,
    ) -> anyhow::Result<Vec<AuditFact>> {
        match slot {
            Some(Ok(v)) => Ok(v.clone()),
            Some(Err(e)) => Err(anyhow::anyhow!(*e)),
            None => panic!("recherche d'état non attendue : {what}"),
        }
    }

    fn answer<T: Copy>(slot: &Option<Result<T, &'static str>>, what: &str) -> anyhow::Result<T> {
        match slot {
            Some(Ok(v)) => Ok(*v),
            Some(Err(e)) => Err(anyhow::anyhow!(*e)),
            None => panic!("recherche d'état non attendue : {what}"),
        }
    }

    impl PrefilterState for Stub {
        async fn branch_has_active_task(&self, _: &str) -> anyhow::Result<bool> {
            answer(&self.branch_task, "branch_task")
        }
        async fn pr_has_active_task(&self, _: &str) -> anyhow::Result<bool> {
            answer(&self.pr_task, "pr_task")
        }
        async fn pr_state(&self, _: &str, _: u64) -> anyhow::Result<PrForgeState> {
            answer(&self.pr_state, "pr_state")
        }
        async fn event_audit_rows(&self) -> anyhow::Result<Vec<AuditFact>> {
            answer_rows(&self.event_rows, "event_rows")
        }
        async fn decision_core_holds(&self, _: &str) -> anyhow::Result<Vec<AuditFact>> {
            answer_rows(&self.holds, "holds")
        }
    }

    fn branch(r: Result<bool, &'static str>) -> Stub {
        Stub {
            branch_task: Some(r),
            ..Stub::default()
        }
    }

    #[tokio::test]
    async fn mika2675_decide_a_suit_la_recherche_de_tache() {
        let c = || classify(GREEN_MAIN).unwrap();
        assert_eq!(
            decide(c(), &branch(Ok(false))).await,
            Decision::Skip {
                class: PrefilterClass::GreenCheckSuiteDefaultBranch,
                target: "check_suite:senara-solutions/mika@main".into()
            }
        );
        assert_eq!(decide(c(), &branch(Ok(true))).await, Decision::Llm);
        // AC3 : base illisible ⇒ le tour a lieu.
        assert_eq!(decide(c(), &branch(Err("db down"))).await, Decision::Llm);
    }

    #[tokio::test]
    async fn mika2675_decide_d_ne_consulte_aucun_etat() {
        let d = decide(classify(INERT).unwrap(), &Stub::default()).await;
        assert_eq!(
            d,
            Decision::Skip {
                class: PrefilterClass::InertLabel,
                target: "issue:senara-solutions/mika#2675".into()
            }
        );
    }

    /// AC4 : trois paliers, une coquille ne désarme pas.
    #[test]
    fn mika2675_interrupteur_trois_paliers() {
        use crate::qa_head_supersession::parse_switch;
        for (raw, armed) in [
            (None, true),
            (Some(""), true),
            (Some("1"), true),
            (Some("0"), false),
            (Some("false"), false),
            (Some("OFF"), false),
            (Some("of"), true),
        ] {
            assert_eq!(parse_switch(PREFILTER_ENV, raw), armed, "{raw:?}");
        }
    }

    /// Les valeurs d'audit sont un format de fil.
    #[test]
    fn mika2675_valeurs_daudit_format_de_fil() {
        assert_eq!(PREFILTER_SKIPPED_TOOL, "webhook_prefilter_skipped");
        assert_eq!(
            PrefilterClass::GreenCheckSuiteDefaultBranch.as_str(),
            "green_check_suite_default_branch"
        );
        assert_eq!(PrefilterClass::InertLabel.as_str(), "inert_label");
        assert_eq!(
            PrefilterClass::VerdictHoldTracked.as_str(),
            "verdict_hold_tracked"
        );
        assert_eq!(
            PrefilterClass::VerdictNonReviewer.as_str(),
            "verdict_non_reviewer"
        );
        assert_eq!(
            PrefilterClass::VerdictUnreadable.as_str(),
            "verdict_unreadable"
        );
        assert_eq!(
            PrefilterClass::VerdictPrClosed.as_str(),
            "verdict_pr_closed"
        );
        assert_eq!(
            PrefilterClass::GreenCheckSuiteHeadDecided.as_str(),
            "green_check_suite_head_decided"
        );
        assert_eq!(
            PrefilterClass::RedCheckSuiteDefaultBranch.as_str(),
            "red_check_suite_default_branch"
        );
        assert_eq!(
            PrefilterClass::CiFailureNoOpenPr.as_str(),
            "ci_failure_no_open_pr"
        );
        assert_eq!(
            PrefilterClass::CiFailureSurfacedForAdoption.as_str(),
            "ci_failure_surfaced_for_adoption"
        );
        assert_eq!(
            PrefilterClass::CiFailureTaskNotInProgress.as_str(),
            "ci_failure_task_not_in_progress"
        );
        // Les deux motifs écrits par le handler sont lus par des `GROUP BY` :
        // même discipline de format de fil (mika#2683).
        assert_eq!(CI_FAILURE_NO_ACTION_TOOL, "ci_failure_no_action");
        assert_eq!(CI_FAILURE_NO_ACTION_NO_OPEN_PR, "no_open_pr");
        assert_eq!(
            CI_FAILURE_NO_ACTION_TASK_NOT_IN_PROGRESS,
            "task_not_in_progress"
        );
        assert_eq!(
            PrefilterClass::PrClosedNoLoopTask.as_str(),
            "pr_closed_no_loop_task"
        );
    }

    // ---- phase 2 — classe (c) -------------------------------------------

    /// Une revue telle que le gateway la délivre.
    pub(crate) fn review(state: &str, author: &str, body: &str) -> String {
        format!(
            "[GitHub] PR review ({state}) on senara-solutions/mika#2680 (fix(x): y) by @{author}\n\
             https://github.com/senara-solutions/mika/pull/2680#pullrequestreview-1\n\n{body}"
        )
    }

    const QA: &str = "mika-platform-qa";

    fn facts(text: &str) -> ReviewVerdictFacts {
        match classify(text) {
            Some(Candidate::ReviewVerdict(f)) => f,
            other => panic!("pas un verdict : {other:?}"),
        }
    }

    /// Terme par terme : chaque champ lu dans le texte est indépendant.
    #[test]
    fn mika2675_c_classify_lit_chaque_terme_separement() {
        let hold = facts(&review("commented", QA, "VERDICT: hold[review]"));
        assert!(hold.hold_tracked && !hold.non_reviewer_pass && !hold.unreadable);

        let other = facts(&review("approved", "samidarko", "VERDICT: pass"));
        assert!(!other.hold_tracked && other.non_reviewer_pass && !other.unreadable);

        let bad = facts(&review("commented", QA, "VERDICT: peut-être"));
        assert!(!bad.hold_tracked && !bad.non_reviewer_pass && bad.unreadable);

        let pass = facts(&review("approved", QA, "VERDICT: pass"));
        assert!(!pass.hold_tracked && !pass.non_reviewer_pass && !pass.unreadable);
        assert_eq!(
            pass.pr_url,
            "https://github.com/senara-solutions/mika/pull/2680"
        );

        assert!(facts(&review("commented", QA, "VERDICT: hold[Review]")).hold_tracked);
        // Seul le `pass` d'un non-relecteur est refermé par le handler : un
        // `block[*]` ou un `hold[x]` d'une autre identité garde son tour.
        for body in [
            "VERDICT: block[security]",
            "VERDICT: block[ac]",
            "VERDICT: hold[foo]",
        ] {
            assert!(
                !facts(&review("commented", "samidarko", body)).non_reviewer_pass,
                "{body}"
            );
        }
        // Un `hold[x]` inconnu n'est suivi par personne.
        assert!(!facts(&review("commented", QA, "VERDICT: hold[foo]")).hold_tracked);
        // Le suffixe `[bot]` reste le relecteur.
        assert!(
            !facts(&review(
                "approved",
                "mika-platform-qa[bot]",
                "VERDICT: pass"
            ))
            .non_reviewer_pass
        );
    }

    /// AC2 : une revue sans ligne VERDICT est une consigne, jamais filtrée.
    #[test]
    fn mika2675_c_revue_sans_ligne_verdict_hors_population() {
        for author in [QA, "samidarko"] {
            let text = review("commented", author, "Merci de renommer cette fonction.");
            assert_eq!(classify(&text), None, "{author}");
        }
    }

    fn pr(task: Result<bool, &'static str>) -> Stub {
        Stub {
            pr_task: Some(task),
            ..Stub::default()
        }
    }

    fn skip(class: PrefilterClass) -> Decision {
        Decision::Skip {
            class,
            target: "pr_review:senara-solutions/mika#2680".into(),
        }
    }

    /// Les trois termes lus dans le texte écartent sans consulter la forge.
    #[tokio::test]
    async fn mika2675_c_trois_termes_textuels_sans_forge() {
        for (body, author, class) in [
            (
                "VERDICT: hold[review]",
                QA,
                PrefilterClass::VerdictHoldTracked,
            ),
            (
                "VERDICT: pass",
                "samidarko",
                PrefilterClass::VerdictNonReviewer,
            ),
            ("VERDICT: peut-être", QA, PrefilterClass::VerdictUnreadable),
        ] {
            let c = classify(&review("commented", author, body)).unwrap();
            assert_eq!(
                decide(c, &pr(Ok(false))).await,
                skip(class),
                "{body} / {author}"
            );
        }
    }

    /// Le quatrième terme : PR fermée ou mergée côté forge.
    #[tokio::test]
    async fn mika2675_c_pr_fermee_ou_mergee() {
        for forge in [PrForgeState::Closed, PrForgeState::Merged] {
            let c = classify(&review("approved", QA, "VERDICT: pass")).unwrap();
            let state = Stub {
                pr_task: Some(Ok(false)),
                pr_state: Some(Ok(forge)),
                ..Stub::default()
            };
            assert_eq!(
                decide(c, &state).await,
                skip(PrefilterClass::VerdictPrClosed)
            );
        }
    }

    /// AC2 et AC3 : verdict actionnable, tâche active, forge ouverte ou
    /// illisible, base illisible — le tour a lieu.
    #[tokio::test]
    async fn mika2675_c_controles_negatifs() {
        let actionable = || classify(&review("approved", QA, "VERDICT: pass")).unwrap();
        let block = || classify(&review("changes_requested", QA, "VERDICT: block[ac]")).unwrap();
        for forge in [Ok(PrForgeState::Open), Err("gh 401")] {
            for c in [actionable(), block()] {
                let state = Stub {
                    pr_task: Some(Ok(false)),
                    pr_state: Some(forge),
                    ..Stub::default()
                };
                assert_eq!(decide(c, &state).await, Decision::Llm, "{forge:?}");
            }
        }
        // Tâche active : même un verdict non actionnable passe, sans forge.
        for (body, author) in [
            ("VERDICT: pass", QA),
            ("VERDICT: hold[review]", QA),
            ("VERDICT: pass", "samidarko"),
            ("VERDICT: peut-être", QA),
        ] {
            let c = classify(&review("approved", author, body)).unwrap();
            assert_eq!(decide(c, &pr(Ok(true))).await, Decision::Llm, "{body}");
        }
        // AC3 : base illisible.
        let c = classify(&review("commented", QA, "VERDICT: hold[review]")).unwrap();
        assert_eq!(decide(c, &pr(Err("db down"))).await, Decision::Llm);
    }

    #[test]
    fn mika2675_c_etat_de_forge_lu_strictement() {
        assert_eq!(
            parse_pr_state(r#"{"state":"OPEN"}"#).unwrap(),
            PrForgeState::Open
        );
        assert_eq!(
            parse_pr_state("{\"state\":\"CLOSED\"}\n").unwrap(),
            PrForgeState::Closed
        );
        assert_eq!(
            parse_pr_state(r#"{"state":"MERGED"}"#).unwrap(),
            PrForgeState::Merged
        );
        for bad in [
            r#"{"state":"open"}"#,
            r#"{"state":null}"#,
            "{}",
            "",
            "gh: not found",
        ] {
            assert!(parse_pr_state(bad).is_err(), "{bad}");
        }
    }

    /// La règle « texte intact » : après le `verdict_handler` pour (c), par
    /// n'importe quel handler pour (a)/(d).
    #[test]
    fn mika2675_c_texte_intact_apres_le_verdict_handler() {
        let verdict = classify(&review("commented", QA, "VERDICT: hold[review]")).unwrap();
        let green = classify(GREEN_MAIN).unwrap();
        let by_verdict_only = Touched {
            by_any_handler: true,
            after_verdict_handler: false,
            outside_ci_success_handler: true,
            outside_ci_failure_handler: true,
        };
        let after = Touched {
            by_any_handler: true,
            after_verdict_handler: true,
            outside_ci_success_handler: true,
            outside_ci_failure_handler: true,
        };
        assert!(!by_verdict_only.applies_to(&verdict));
        assert!(after.applies_to(&verdict));
        assert!(by_verdict_only.applies_to(&green));
    }

    // ---- phase 3 — classe (b) -------------------------------------------

    const REPO: &str = "senara-solutions/mika";
    pub(crate) const HEAD_A: &str =
        "pr:senara-solutions/mika#2678@1a2edc8091fee6eec4c736b5dd974221802287b6";
    const HEAD_B: &str = "pr:senara-solutions/mika#2678@ffffffffffffffffffffffffffffffffffffffff";
    pub(crate) const GREEN_PR: &str =
        "[GitHub] Check suite success on senara-solutions/mika (branch: fix/2675/x)";

    pub(crate) fn hold(target: &str, after: &str, trace: &str) -> AuditFact {
        AuditFact {
            // Rangs déterministes : « maintenant » / « now » écrit après tout le reste.
            id: if trace.contains("now") || trace.contains("maintenant") {
                10
            } else {
                1
            },
            tool: DECISION_CORE_HOLD_TOOL.into(),
            target: target.into(),
            after: Some(after.into()),
            trace: Some(trace.into()),
        }
    }

    fn processed(target: &str, trace: &str) -> AuditFact {
        AuditFact {
            id: 9,
            tool: "ci_success_handler_processed".into(),
            target: target.into(),
            after: None,
            trace: Some(trace.into()),
        }
    }

    /// Le cas mesuré (mika#2672, #2674) : cet événement redécide, un
    /// précédent avait décidé et notifié la même tête.
    fn decided_case() -> (Vec<AuditFact>, Vec<AuditFact>) {
        let mine = hold(HEAD_A, "notified", "rid-now");
        (
            vec![processed(HEAD_A, "rid-now"), mine.clone()],
            vec![mine, hold(HEAD_A, "notified", "rid-before")],
        )
    }

    #[test]
    fn mika2675_b_les_trois_termes_tenus_ecartent() {
        let (this_event, prior) = decided_case();
        assert_eq!(
            head_decided_and_notified(REPO, &this_event, &prior),
            Some(HEAD_A.to_string())
        );
    }

    /// Terme « décision attestée » : sans attestation de CET événement — dédup
    /// 2b/2c, pas de verdict, checks en attente, simple marqueur `processed` —
    /// le tour a lieu.
    #[test]
    fn mika2675_b_terme_decision_attestee() {
        let (_, prior) = decided_case();
        for this_event in [
            vec![],
            vec![processed(HEAD_A, "rid-now")],
            vec![AuditFact {
                tool: "ci_success_merge_ready".into(),
                ..processed(HEAD_A, "rid-now")
            }],
        ] {
            assert_eq!(
                head_decided_and_notified(REPO, &this_event, &prior),
                None,
                "{this_event:?}"
            );
        }
        // Une ligne antérieure d'un autre outil sur la même tête n'atteste pas
        // la décision.
        let (this_event, _) = decided_case();
        let prior = vec![AuditFact {
            tool: "ci_success_handler_human_gate_required".into(),
            ..hold(HEAD_A, "notified", "rid-before")
        }];
        assert_eq!(head_decided_and_notified(REPO, &this_event, &prior), None);
    }

    /// Terme « même tête » : une décision notifiée sur un AUTRE SHA de la même
    /// PR ne compte pas — une nouvelle tête n'est jamais filtrée (AC2).
    #[test]
    fn mika2675_b_terme_meme_tete() {
        let (this_event, _) = decided_case();
        let prior = vec![hold(HEAD_B, "notified", "rid-before")];
        assert_eq!(head_decided_and_notified(REPO, &this_event, &prior), None);
        // Et la tête de cet événement doit appartenir au dépôt de l'événement.
        let (this_event, prior) = decided_case();
        assert_eq!(
            head_decided_and_notified("senara-solutions/mika-cloud", &this_event, &prior),
            None
        );
    }

    /// Terme « notification attestée » : antérieure non notifiée, ou sans
    /// valeur, ne compte pas.
    #[test]
    fn mika2675_b_terme_notification_attestee() {
        let (this_event, _) = decided_case();
        for after in [Some("not_notified"), None] {
            let prior = vec![AuditFact {
                after: after.map(Into::into),
                ..hold(HEAD_A, "", "rid-before")
            }];
            assert_eq!(
                head_decided_and_notified(REPO, &this_event, &prior),
                None,
                "{after:?}"
            );
        }
    }

    /// Une attestation d'un autre événement écrite APRÈS celle de cet événement
    /// ne vaut pas « déjà » : le premier traitement d'une tête, lent, garde son
    /// tour même si un traitement plus tardif l'a devancé dans l'audit (AC2).
    #[test]
    fn mika2675_b_terme_anteriorite() {
        let (this_event, _) = decided_case();
        let later = AuditFact {
            id: 11,
            ..hold(HEAD_A, "notified", "rid-after")
        };
        assert_eq!(head_decided_and_notified(REPO, &this_event, &[later]), None);
    }

    /// Premier traitement d'une tête : la seule attestation est la sienne,
    /// même notifiée — elle ne vaut pas « déjà » (AC2).
    #[test]
    fn mika2675_b_premier_traitement_garde_son_tour() {
        let mine = hold(HEAD_A, "notified", "rid-now");
        assert_eq!(
            head_decided_and_notified(
                REPO,
                std::slice::from_ref(&mine),
                std::slice::from_ref(&mine)
            ),
            None
        );
    }

    fn rows(
        event: Result<Vec<AuditFact>, &'static str>,
        holds: Option<Result<Vec<AuditFact>, &'static str>>,
    ) -> Stub {
        Stub {
            event_rows: Some(event),
            holds,
            ..Stub::default()
        }
    }

    #[tokio::test]
    async fn mika2675_b_decide_ecarte_avec_cible_nommee() {
        let (this_event, prior) = decided_case();
        assert_eq!(
            decide(
                classify(GREEN_PR).unwrap(),
                &rows(Ok(this_event), Some(Ok(prior)))
            )
            .await,
            Decision::Skip {
                class: PrefilterClass::GreenCheckSuiteHeadDecided,
                target: format!("check_suite:{}", HEAD_A.strip_prefix("pr:").unwrap()),
            }
        );
    }

    /// Sans attestation de cet événement, les attestations de la PR ne sont
    /// même pas lues (le bouchon paniquerait).
    #[tokio::test]
    async fn mika2675_b_decide_sans_attestation_ne_lit_pas_la_pr() {
        let c = classify(GREEN_PR).unwrap();
        assert_eq!(
            decide(c, &rows(Ok(vec![processed(HEAD_A, "rid-now")]), None)).await,
            Decision::Llm
        );
    }

    /// AC3 : audit illisible, à l'une ou l'autre lecture ⇒ le tour a lieu.
    #[tokio::test]
    async fn mika2675_b_audit_illisible_le_tour_a_lieu() {
        let (this_event, _) = decided_case();
        let c = || classify(GREEN_PR).unwrap();
        assert_eq!(
            decide(c(), &rows(Err("db down"), None)).await,
            Decision::Llm
        );
        assert_eq!(
            decide(c(), &rows(Ok(this_event), Some(Err("db down")))).await,
            Decision::Llm
        );
    }

    #[test]
    fn mika2675_b_prefixe_de_pr() {
        assert_eq!(pr_prefix(HEAD_A), Some("pr:senara-solutions/mika#2678@"));
        assert_eq!(pr_prefix("pr:senara-solutions/mika#2678"), None);
    }

    /// (b) n'admet qu'un texte touché par `ci_success_handler` seul.
    #[test]
    fn mika2675_b_texte_touche_hors_ci_success_handler() {
        let b = classify(GREEN_PR).unwrap();
        let by = |outside| Touched {
            by_any_handler: true,
            after_verdict_handler: true,
            outside_ci_success_handler: outside,
            outside_ci_failure_handler: true,
        };
        assert!(!by(false).applies_to(&b));
        assert!(by(true).applies_to(&b));
    }

    // ---- phase 4 — les sorties « rien à faire » du CI failure handler ----
    //
    // mika#2683. Les textes sont ceux que le gateway délivre réellement.

    const RED_MAIN: &str = "[GitHub] Check suite failure on senara-solutions/mika (branch: main)";
    const RED_PR: &str =
        "[GitHub] Check suite failure on senara-solutions/mika (branch: dependabot/cargo/x)";

    /// (a rouge) terme par terme. La conclusion est portée par
    /// `parse_check_suite_failure` : retirer ce terme ferait entrer les
    /// conclusions neutres dans la population. La branche sépare (a) de (c) :
    /// retirer ce terme ferait rendre `RedDefaultBranch` sur une branche de PR.
    #[test]
    fn mika2683_classify_les_deux_termes_de_la_classe_rouge() {
        for conclusion in ["failure", "timed_out"] {
            assert_eq!(
                classify(&format!(
                    "[GitHub] Check suite {conclusion} on senara-solutions/mika (branch: main)"
                )),
                Some(Candidate::RedDefaultBranch {
                    repo: "senara-solutions/mika".into(),
                    branch: "main".into()
                }),
                "{conclusion}"
            );
            // Hors branche par défaut : jugée par (c), jamais par (a).
            assert_eq!(
                classify(&format!(
                    "[GitHub] Check suite {conclusion} on senara-solutions/mika (branch: fix/2683/x)"
                )),
                Some(Candidate::RedPrBranch {
                    repo: "senara-solutions/mika".into(),
                    branch: "fix/2683/x".into()
                }),
                "{conclusion}"
            );
        }
        // Toute autre conclusion sort de la population : ni verte, ni rouge.
        for conclusion in [
            "cancelled",
            "neutral",
            "action_required",
            "stale",
            "skipped",
        ] {
            for branch in ["main", "fix/2683/x"] {
                let text = format!(
                    "[GitHub] Check suite {conclusion} on senara-solutions/mika (branch: {branch})"
                );
                assert_eq!(classify(&text), None, "{text}");
            }
        }
    }

    /// (a rouge) le terme d'état, par symétrie avec la classe verte voisine.
    #[tokio::test]
    async fn mika2683_a_rouge_suit_la_recherche_de_tache() {
        let c = || classify(RED_MAIN).unwrap();
        assert_eq!(
            decide(c(), &branch(Ok(false))).await,
            Decision::Skip {
                class: PrefilterClass::RedCheckSuiteDefaultBranch,
                target: "check_suite:senara-solutions/mika@main".into()
            }
        );
        // Une tâche active sur `main` est un état que le modèle doit prendre en
        // compte — la porte est strictement plus conservatrice que l'étape 2 du
        // handler, qui skippe inconditionnellement.
        assert_eq!(decide(c(), &branch(Ok(true))).await, Decision::Llm);
        // AC3 : base illisible ⇒ le tour a lieu.
        assert_eq!(decide(c(), &branch(Err("db down"))).await, Decision::Llm);
    }

    fn no_action(motive: &str) -> AuditFact {
        AuditFact {
            id: 7,
            tool: CI_FAILURE_NO_ACTION_TOOL.into(),
            target: "check_suite:senara-solutions/mika@dependabot/cargo/x".into(),
            after: Some(motive.into()),
            trace: Some("rid-now".into()),
        }
    }

    fn acted(tool: &str) -> AuditFact {
        AuditFact {
            tool: tool.into(),
            ..no_action("ignoré")
        }
    }

    /// (c) chaque ligne terminale rend SA classe — trois populations
    /// dénombrables séparément (mika#2156).
    #[test]
    fn mika2683_c_chaque_sortie_terminale_rend_sa_classe() {
        for (row, class) in [
            (
                no_action(CI_FAILURE_NO_ACTION_NO_OPEN_PR),
                PrefilterClass::CiFailureNoOpenPr,
            ),
            (
                no_action(CI_FAILURE_NO_ACTION_TASK_NOT_IN_PROGRESS),
                PrefilterClass::CiFailureTaskNotInProgress,
            ),
            (
                acted(SURFACE_FOR_ADOPTION_TOOL),
                PrefilterClass::CiFailureSurfacedForAdoption,
            ),
        ] {
            assert_eq!(
                ci_failure_no_action_class(std::slice::from_ref(&row)),
                Some(class),
                "{row:?}"
            );
        }
    }

    /// (c) le garde-fou : dès qu'une ligne dit que le handler a AGI, le tour a
    /// lieu — même quand une ligne terminale est présente par ailleurs.
    #[test]
    fn mika2683_c_une_ligne_dagir_rend_le_tour() {
        for acted_tool in [CI_FAILURE_HANDLED_TOOL, CI_FAILURE_ESCALATED_TOOL] {
            for terminal in [
                no_action(CI_FAILURE_NO_ACTION_NO_OPEN_PR),
                acted(SURFACE_FOR_ADOPTION_TOOL),
            ] {
                let rows = vec![terminal.clone(), acted(acted_tool)];
                assert_eq!(
                    ci_failure_no_action_class(&rows),
                    None,
                    "{acted_tool} / {terminal:?}"
                );
            }
        }
    }

    /// (c) l'absence de ligne est ce qui garde son tour à une sortie qui
    /// INFORME le modèle : jeton GitHub absent, forge illisible, correctif en
    /// vol. Et un motif que cette version ne connaît pas ne mord pas.
    #[test]
    fn mika2683_c_sans_ligne_terminale_le_tour_a_lieu() {
        assert_eq!(ci_failure_no_action_class(&[]), None);
        for row in [
            // Les lignes du voisin `ci_success_handler`, sur le même événement.
            acted(DECISION_CORE_HOLD_TOOL),
            acted("ci_success_handler_processed"),
            // Motif inconnu, ou absent.
            no_action("motif_du_futur"),
            AuditFact {
                after: None,
                ..no_action(CI_FAILURE_NO_ACTION_NO_OPEN_PR)
            },
        ] {
            assert_eq!(
                ci_failure_no_action_class(std::slice::from_ref(&row)),
                None,
                "{row:?}"
            );
        }
    }

    /// (c) sur `decide` : la classe vient de la trace, et un audit illisible
    /// laisse passer (AC3).
    #[tokio::test]
    async fn mika2683_c_decide_lit_la_trace_de_levenement() {
        let c = || classify(RED_PR).unwrap();
        assert_eq!(
            decide(
                c(),
                &rows(Ok(vec![no_action(CI_FAILURE_NO_ACTION_NO_OPEN_PR)]), None)
            )
            .await,
            Decision::Skip {
                class: PrefilterClass::CiFailureNoOpenPr,
                target: "check_suite:senara-solutions/mika@dependabot/cargo/x".into(),
            }
        );
        assert_eq!(decide(c(), &rows(Ok(vec![]), None)).await, Decision::Llm);
        assert_eq!(
            decide(c(), &rows(Err("db down"), None)).await,
            Decision::Llm
        );
    }

    /// Les classes de la phase 4 n'admettent qu'un texte touché par le
    /// `ci_failure_handler` seul — et les classes voisines ne sont pas
    /// relâchées par le nouveau terme.
    #[test]
    fn mika2683_texte_touche_hors_ci_failure_handler() {
        let by = |outside| Touched {
            by_any_handler: true,
            after_verdict_handler: true,
            outside_ci_success_handler: true,
            outside_ci_failure_handler: outside,
        };
        for c in [classify(RED_MAIN).unwrap(), classify(RED_PR).unwrap()] {
            assert!(!by(false).applies_to(&c), "{c:?}");
            assert!(by(true).applies_to(&c), "{c:?}");
        }
        // Le nouveau terme ne décide d'aucune classe antérieure.
        for c in [
            classify(GREEN_MAIN).unwrap(),
            classify(INERT).unwrap(),
            classify(GREEN_PR).unwrap(),
            classify(&review("commented", QA, "VERDICT: hold[review]")).unwrap(),
        ] {
            assert!(by(false).applies_to(&c), "{c:?}");
        }
    }

    // ---- vol 1 — le registre des tours webhook (mika#2680) ---------------
    //
    // Les textes de cette section sont ceux que le gateway délivre
    // réellement ; le test cross-crate
    // `mika2680_chaque_prefixe_declare_est_celui_que_le_gateway_emet`
    // (`mika-gateway::github::tests`) tient l'autre bout du contrat.

    /// Un `check_suite` sur la branche de PR, dans la forme que le gateway émet.
    const PR_BRANCH_SUITE: &str =
        "[GitHub] Check suite failure on senara-solutions/mika (branch: feat/2680/x)";

    /// Le pré-digest que `verdict_handler::format_verdict_classification_failed_pre_digest`
    /// substitue au texte d'origine. Reproduit ici en tête et en forme — c'est
    /// la PREMIÈRE ligne qui décide, et elle est à elle seule la mutation
    /// rouge d'AC2.
    fn verdict_handler_rewrite(origin: &str) -> String {
        format!(
            "<verdict_handler>\n{origin}\n\
             [verdict_classification_failed] No parseable VERDICT: line in review body.\n\n\
             Operator has been notified.\n</verdict_handler>"
        )
    }

    /// Même forme, côté `ci_success_handler` : le handler n'enrichit pas le
    /// texte, il le remplace par un pré-digest balisé.
    fn ci_success_handler_rewrite(origin: &str) -> String {
        format!(
            "<ci_success_handler>\n{origin}\n\
             CI checks all green — every gate cleared.\n</ci_success_handler>"
        )
    }

    /// Terme par terme : chacun des douze termes est atteint par un texte que
    /// le gateway émet, et aucun n'est atteint par un autre.
    #[test]
    fn mika2680_la_taxonomie_atteint_chacun_de_ses_douze_termes() {
        use TriggerClass::*;

        let cas: [(&str, TriggerClass); 12] = [
            (GREEN_MAIN, CheckSuiteDefaultBranch),
            (PR_BRANCH_SUITE, CheckSuitePrBranch),
            (
                "[GitHub] PR review (approved) on senara-solutions/mika#2680 (t) by @mika-platform-qa",
                PrReview,
            ),
            (
                "[GitHub] PR opened: senara-solutions/mika#2680 — t (branch: feat/2680/x)",
                PrOpened,
            ),
            (
                "[GitHub] PR closed: senara-solutions/mika#2679 — t (branch: feat/2679/x)\nMerged: true",
                PrClosed,
            ),
            (
                "[GitHub] PR synchronize: senara-solutions/mika#2680 — t (branch: feat/2680/x)",
                PrOther,
            ),
            (INERT, IssueLabeled),
            (
                "[GitHub] Issue closed: senara-solutions/mika#2294 — t",
                IssueClosed,
            ),
            (
                "[GitHub] Issue opened: senara-solutions/mika#2680 — t",
                IssueOther,
            ),
            (
                "[GitHub] New comment on senara-solutions/mika#2654 (t) by @samidarko",
                IssueComment,
            ),
            ("[GitHub] push.unknown on senara-solutions/mika", Push),
            ("une consigne de l'opérateur par Telegram", Other),
        ];

        for (texte, attendu) in cas {
            assert_eq!(trigger_class(texte), attendu, "texte : {texte:?}");
        }

        // Douze cas pour douze termes, et deux fois le même terme passerait le
        // bloc ci-dessus sans rien dire : la couverture est donc comptée.
        let mut termes: Vec<&str> = cas.iter().map(|(_, c)| c.as_str()).collect();
        termes.sort_unstable();
        termes.dedup();
        assert_eq!(
            termes.len(),
            12,
            "un terme est couvert deux fois et un autre pas du tout : {termes:?}"
        );
    }

    /// Les deux pièges d'ordre, pris par le bas : si un raffinement passait
    /// après son générique, `pr_review` et `issue_labeled` deviendraient
    /// inatteignables en silence.
    #[test]
    fn mika2680_un_raffinement_passe_avant_son_generique() {
        let revue = review("commented", QA, "VERDICT: hold[review]");
        assert!(revue.starts_with(PR_EVENT_PREFIX), "prémisse du piège");
        assert_eq!(trigger_class(&revue), TriggerClass::PrReview);

        let label = labeled("loop-substrate");
        assert!(label.starts_with(ISSUE_EVENT_PREFIX), "prémisse du piège");
        assert_eq!(trigger_class(&label), TriggerClass::IssueLabeled);
    }

    /// AC2 — le déclencheur se lit sur le texte d'ORIGINE, jamais sur
    /// `req.text`. C'est la mutation rouge du ticket : les deux handlers qui
    /// réécrivent remplacent le texte par un pré-digest balisé, dont la
    /// première ligne est `<handler>` et ne porte donc aucun préfixe de
    /// grammaire — la classe (f) du ticket tomberait sur `other`, c'est-à-dire
    /// précisément sur la population à chiffrer.
    #[test]
    fn mika2680_le_declencheur_se_lit_sur_le_texte_dorigine() {
        let origine = review("commented", QA, "VERDICT: illisible");
        let reecrit = verdict_handler_rewrite(&origine);
        assert_eq!(trigger_class(&origine), TriggerClass::PrReview);
        assert_eq!(
            trigger_class(&reecrit),
            TriggerClass::Other,
            "si ceci rend `pr_review`, la mutation rouge d'AC2 ne mord plus : \
             le pré-digest du verdict_handler ne commence plus par sa balise"
        );

        let reecrit = ci_success_handler_rewrite(GREEN_PR);
        assert_eq!(trigger_class(GREEN_PR), TriggerClass::CheckSuitePrBranch);
        assert_eq!(trigger_class(&reecrit), TriggerClass::Other);

        // Et c'est exactement ce que `text_touched` nomme : le tour est le
        // même, seul le texte a changé de main.
        let touched = Touched {
            by_any_handler: true,
            after_verdict_handler: true,
            outside_ci_success_handler: true,
            outside_ci_failure_handler: true,
        };
        assert!(touched.by_any_handler);
    }

    /// AC3 — exhaustivité. Les six gabarits que `format_event_text` met en
    /// forme sont énumérés ici ; aucun ne retombe sur le terme de repli. Le
    /// septième, l'attrape-tout, est `other` et c'est sa place légitime.
    #[test]
    fn mika2680_aucun_gabarit_nomme_du_gateway_ne_retombe_sur_other() {
        use mika_common::github_event_format::ISSUE_OPENED_EVENT_PREFIX;

        // Un texte minimal par gabarit, construit sur le préfixe déclaré —
        // donc sans littéral de grammaire, comme la taxonomie elle-même.
        let gabarits: [(&str, String); 6] = [
            (
                "issues.labeled",
                format!("{ISSUE_LABELED_EVENT_PREFIX}ready on senara-solutions/mika#2680 — t"),
            ),
            (
                "issues.{action}",
                format!("{ISSUE_OPENED_EVENT_PREFIX} senara-solutions/mika#2680 — t"),
            ),
            (
                "issue_comment",
                format!("{ISSUE_COMMENT_EVENT_PREFIX}senara-solutions/mika#2680 (t) by @samidarko"),
            ),
            (
                "pull_request",
                format!("{PR_OPENED_EVENT_PREFIX} senara-solutions/mika#2680 — t (branch: x)"),
            ),
            (
                "pull_request_review",
                format!(
                    "{PR_REVIEW_EVENT_PREFIX}(approved) on senara-solutions/mika#2680 (t) by @qa"
                ),
            ),
            (
                "check_suite",
                format!(
                    "{CHECK_SUITE_EVENT_PREFIX}success on senara-solutions/mika (branch: main)"
                ),
            ),
        ];

        for (gabarit, texte) in &gabarits {
            assert_ne!(
                trigger_class(texte),
                TriggerClass::Other,
                "le gabarit {gabarit} retombe sur le repli — la taxonomie a cessé de le nommer : \
                 {texte:?}"
            );
        }

        // L'attrape-tout, septième forme : c'est lui, et lui seul, que `other`
        // décrit. Un `push` en est l'instance réelle et porte son propre terme.
        assert_eq!(
            trigger_class("[GitHub] deployment_status.success on senara-solutions/mika"),
            TriggerClass::Other
        );
        assert_eq!(
            trigger_class("[GitHub] push.unknown on senara-solutions/mika"),
            TriggerClass::Push
        );
    }

    /// Un `check_suite` dont la branche est illisible n'est ni l'une ni
    /// l'autre : se taire vaut mieux que nommer une branche qu'on n'a pas lue.
    #[test]
    fn mika2680_un_check_suite_sans_branche_lisible_ne_nomme_aucune_branche() {
        assert_eq!(
            trigger_class("[GitHub] Check suite success on senara-solutions/mika"),
            TriggerClass::Other
        );
        // La branche se lit entre le marqueur et le `)` de fin de ligne.
        assert_eq!(
            trigger_class("[GitHub] Check suite success on senara-solutions/mika (branch: main"),
            TriggerClass::Other
        );
    }

    /// AC6 — `candidate` est le nom d'un variant quand `classify` en rend un,
    /// et **manque** sinon. Jamais `0`, jamais `"none"` : « l'événement n'est
    /// pas dans la population du pré-filtre » n'est pas la même affirmation
    /// que « il y est et a gardé son tour » (motif mika#2331). Le champ est
    /// omis par le bras `None` de [`emit_turn_charged`] ; ce test en tient la
    /// prémisse — quels textes ont un candidat, lesquels n'en ont pas.
    #[test]
    fn mika2680_le_candidat_manque_quand_la_classification_ne_rend_rien() {
        // Dans la population du pré-filtre : le nom du variant est écrit.
        for (texte, nom) in [
            (GREEN_MAIN, "green_default_branch"),
            (GREEN_PR, "green_pr_branch"),
            (RED_MAIN, "red_default_branch"),
            (RED_PR, "red_pr_branch"),
            (INERT, "inert_label"),
            (PR_CLOSED, "pr_closed_no_task"),
        ] {
            assert_eq!(classify(texte).unwrap().as_str(), nom, "{texte:?}");
        }
        assert_eq!(
            classify(&review("commented", QA, "VERDICT: hold[review]"))
                .unwrap()
                .as_str(),
            "review_verdict"
        );

        // Hors population : aucun candidat, donc aucun champ à écrire. Chacun
        // de ces tours a pourtant un `trigger`, et c'est tout l'intérêt —
        // `issue_comment` est 79 % de la fenêtre mesurée le 2026-10-08. Le
        // `PR closed` sans URL en fait partie depuis mika#2680 : la classe (e)
        // ne sait pas de quelle PR il parle, donc elle n'en est pas candidate.
        for texte in [
            "[GitHub] New comment on senara-solutions/mika#2654 (t) by @samidarko",
            "[GitHub] Issue closed: senara-solutions/mika#2294 — t",
            PR_CLOSED_SANS_URL,
        ] {
            assert!(classify(texte).is_none(), "{texte:?}");
            assert_ne!(trigger_class(texte), TriggerClass::Other, "{texte:?}");
        }
    }

    /// Les orthographes de fil, figées. Le vol 2 (« le lecteur ») en fera des
    /// `GROUP BY` : une orthographe qui bouge scinde une population sans le
    /// dire — même discipline que [`PrefilterClass::as_str`].
    #[test]
    fn mika2680_les_orthographes_de_fil_sont_figees() {
        assert_eq!(WEBHOOK_TURN_CHARGED_EVENT, "webhook_turn_charged");

        assert_eq!(
            TriggerClass::CheckSuiteDefaultBranch.as_str(),
            "check_suite_default_branch"
        );
        assert_eq!(
            TriggerClass::CheckSuitePrBranch.as_str(),
            "check_suite_pr_branch"
        );
        assert_eq!(TriggerClass::PrReview.as_str(), "pr_review");
        assert_eq!(TriggerClass::PrOpened.as_str(), "pr_opened");
        assert_eq!(TriggerClass::PrClosed.as_str(), "pr_closed");
        assert_eq!(TriggerClass::PrOther.as_str(), "pr_other");
        assert_eq!(TriggerClass::IssueLabeled.as_str(), "issue_labeled");
        assert_eq!(TriggerClass::IssueClosed.as_str(), "issue_closed");
        assert_eq!(TriggerClass::IssueOther.as_str(), "issue_other");
        assert_eq!(TriggerClass::IssueComment.as_str(), "issue_comment");
        assert_eq!(TriggerClass::Push.as_str(), "push");
        assert_eq!(TriggerClass::Other.as_str(), "other");
    }

    // ---- vol 3 — classe (e) : `PR closed` sans tâche de boucle -----------
    //
    // Les textes de cette section sont ceux que le gateway délivre réellement,
    // URL en deuxième ligne et `Merged:` en troisième.

    /// (e) terme de texte : l'URL de la PR, et elle seule. Le `Merged: true`
    /// présent dans le gabarit n'est PAS lu — la vérité du merge appartient aux
    /// handlers structurels, qui tournent avant cette porte.
    #[test]
    fn mika2680_e_classify_rend_lurl_de_la_pr() {
        assert_eq!(
            classify(PR_CLOSED),
            Some(Candidate::PrClosedNoTask {
                pr_url: PR_CLOSED_URL.into()
            })
        );
    }

    /// (e) sans URL extractible, l'événement n'est pas un candidat : on ne sait
    /// pas de quelle PR il parle, donc le tour a lieu (fail-safe, AC3). Les
    /// trois formes viennent du même extracteur
    /// (`upstream_close_handler::extract_pr_url`) : ligne vide quand le gateway
    /// interpole un `html_url` absent, mauvais segment, numéro manquant.
    #[test]
    fn mika2680_e_sans_url_extractible_le_tour_a_lieu() {
        for texte in [
            PR_CLOSED_SANS_URL,
            "[GitHub] PR closed: senara-solutions/mika#2679 — t (branch: x)\nhttps://github.com/senara-solutions/mika/issues/2679\nMerged: true",
            "[GitHub] PR closed: senara-solutions/mika#2679 — t (branch: x)\nhttps://github.com/senara-solutions/mika/pull/\nMerged: true",
        ] {
            assert_eq!(classify(texte), None, "{texte:?}");
        }
    }

    /// (e) voisinage : `PR_OPENED_EVENT_PREFIX` et `PR_CLOSED_EVENT_PREFIX`
    /// partagent leur tête `[GitHub] PR `, et un `PR opened` porte la même URL
    /// en deuxième ligne. Il ne rend pas cette variante — ni aucune autre :
    /// `PR opened` est 536 k tokens de la fenêtre mesurée, et il garde son tour.
    #[test]
    fn mika2680_e_un_pr_opened_ne_rend_pas_cette_variante() {
        let ouvert = format!(
            "{PR_OPENED_EVENT_PREFIX} senara-solutions/mika#2679 — t (branch: x)\n{PR_CLOSED_URL}"
        );
        assert!(ouvert.starts_with(PR_EVENT_PREFIX), "{ouvert:?}");
        assert_eq!(classify(&ouvert), None, "{ouvert:?}");
    }

    /// (e) prend la fenêtre de texte la plus STRICTE, et le test le vérifie au
    /// lieu de le supposer : avec `by_any_handler` seul armé et les trois autres
    /// fenêtres à `false`, le candidat s'applique quand même — donc c'est bien
    /// `by_any_handler` qu'il lit. Un texte réécrit par un handler a pu
    /// réécrire la ligne qui porte la clé de `pr_has_active_task`.
    #[test]
    fn mika2680_e_tout_handler_qui_a_touche_le_texte_rend_le_tour() {
        let c = classify(PR_CLOSED).unwrap();
        let strict = Touched {
            by_any_handler: true,
            after_verdict_handler: false,
            outside_ci_success_handler: false,
            outside_ci_failure_handler: false,
        };
        assert!(strict.applies_to(&c), "{c:?}");
        let intact = Touched {
            by_any_handler: false,
            after_verdict_handler: true,
            outside_ci_success_handler: true,
            outside_ci_failure_handler: true,
        };
        assert!(!intact.applies_to(&c), "{c:?}");
    }

    /// (e) terme d'état : `pr_has_active_task`, et lui seul. Le bouchon laisse
    /// `pr_state`, `event_rows` et `holds` à `None` — ils paniquent s'ils sont
    /// consultés, donc ce test tient aussi « (e) ne redérive rien ».
    ///
    /// Le contrôle négatif de la même graine est
    /// [`mika2680_e_une_tache_active_garde_son_tour`] : la leçon mika#2277 est
    /// qu'une batterie uniquement positive est satisfaite par un filtre qui
    /// écarte tout.
    #[tokio::test]
    async fn mika2680_e_sans_tache_active_le_tour_est_ecarte() {
        assert_eq!(
            decide(classify(PR_CLOSED).unwrap(), &pr(Ok(false))).await,
            Decision::Skip {
                class: PrefilterClass::PrClosedNoLoopTask,
                target: format!("pr_closed:{PR_CLOSED_URL}"),
            }
        );
    }

    /// Contrôle négatif de [`mika2680_e_sans_tache_active_le_tour_est_ecarte`] :
    /// même texte, même bouchon, **seul l'état diffère**. Une tâche de boucle
    /// active sur cette PR est exactement ce que le modèle doit prendre en
    /// compte : le tour a lieu.
    #[tokio::test]
    async fn mika2680_e_une_tache_active_garde_son_tour() {
        assert_eq!(
            decide(classify(PR_CLOSED).unwrap(), &pr(Ok(true))).await,
            Decision::Llm
        );
    }

    /// (e) fail-safe (AC3 de mika#2675) : une lecture d'état en erreur rend le
    /// tour. La raison est nommée dans le fil sous
    /// `webhook_prefilter_state_unreadable` avec `what = "pr_has_active_task"` —
    /// le nom de la porte consultée, pas celui de la classe, pour qu'un grep
    /// d'opérateur distingue cette lecture de celle de (c) revue (`pr_task`).
    #[tokio::test]
    async fn mika2680_e_un_etat_illisible_rend_le_tour() {
        assert_eq!(
            decide(classify(PR_CLOSED).unwrap(), &pr(Err("db down"))).await,
            Decision::Llm
        );
    }
}

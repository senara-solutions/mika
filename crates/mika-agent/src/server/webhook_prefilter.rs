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
//!
//! # Le terme de relâche (mika#2680, vol 4)
//!
//! Le registre du vol 1 nomme ce qu'un tour ÉTAIT — `trigger`, `text_touched`,
//! `candidate`, `prefilter_enabled` — et jamais POURQUOI il a eu lieu. Ce vol
//! ajoute le terme direct : [`Release`], dix valeurs, une par porte qui relâche
//! un tour. Il ne décide rien — aucun tour de plus n'est écarté, aucun de moins
//! — et c'est ce qui le rend livrable quand (f) et (g) ne le sont pas : le
//! terme se lit dans le flot de contrôle de [`skip_turn`] et de [`decide`], qui
//! est devant les yeux, pas dans une mesure.
//!
//! **Un champ, pas une conjugaison.** `release` porte UNE valeur par tour
//! facturé. Laisser le lecteur conjuguer les champs existants était l'autre
//! option, et le § 5.2 du protocole de mesure la refuse : une population lue
//! par conjonction de deux champs est une population qu'on compte mal un jour.
//! L'agrégat groupe par un champ ou ne groupe pas. Le champ est transporté, pas
//! redérivé : [`Gate::Charged`] le remonte de la porte qui a relâché jusqu'à
//! [`emit_turn_charged`], d'où un `match` sans bras par défaut au site d'appel
//! — un onzième terme ne compile pas sans être transporté.
//!
//! **Trois choix de forme.** [`Release::StateUnreadable`] est UN terme et pas
//! cinq : le prédicat en échec est déjà nommé à l'octet près par le `warn!` de
//! [`state_unreadable`] (champ `what`), et le dupliquer dans la valeur de
//! relâche multiplierait la cardinalité du seau par cinq pour rien.
//! [`Release::SwitchDisarmed`] existe bien qu'il redouble `prefilter_enabled:
//! false` — le redoublement est le but, une fenêtre partiellement désarmée doit
//! se lire dans le même tableau que le reste. Et [`Candidate::InertLabel`] n'a
//! **pas** de terme, parce que `decide` ne le relâche jamais : une valeur pour
//! un chemin inatteignable est une valeur qu'un lecteur cherchera un jour.
//!
//! **L'absence est nommée.** Une ligne `webhook_turn_charged` écrite par un
//! binaire antérieur à ce vol ne porte pas `release` ; l'agrégat du lecteur la
//! range sous `absent_pre_field`, jamais sous `null`, `unknown` ni `"none"`
//! (motif mika#2331, déjà tenu par `candidate`). Le seau dit la raison de
//! l'absence, donc aucun second drapeau de halte n'est ajouté au § 5 du
//! protocole : une fenêtre entièrement antérieure au déploiement se lit à 100 %
//! dans ce seau.
//!
//! **Ce que ce terme tranche, et que `text_touched` ne tranchait pas.** Le
//! corps de mika#2680 dit de sa classe (f) — un `pr_review` sur une PR fermée
//! dont le `verdict_handler` a réécrit le texte — que *« la règle "texte intact
//! après handler" de la phase 2 exclut ce cas »*. **C'est faux sur le code
//! servi**, et la prémisse a été re-dérivée trois fois le 2026-10-19, au prix
//! d'un dispatch chacune. [`Touched::applies_to`] ne lit, pour une revue, que
//! `after_verdict_handler`, calculé au site d'appel comme « un handler a touché
//! le texte DEPUIS l'instantané pris après le `verdict_handler` » : la
//! réécriture du `verdict_handler` est donc DANS la fenêtre, et seul un handler
//! ultérieur garde le tour. Les deux formes que le corps cite sont déjà
//! écartées par (c) — `CIRCUIT BREAKER TRIPPED` par
//! [`PrefilterClass::VerdictPrClosed`], `verdict_classification_failed` à
//! valeur illisible par [`PrefilterClass::VerdictUnreadable`] —, ce que
//! `mika2680_f_le_pr_review_reecrit_est_deja_ecarte` fige en assertions plutôt
//! qu'en prose. Quatre chemins peuvent donc expliquer les 192 k mesurés, et
//! `release` est exactement ce qui les sépare : `not_classified` (une revue
//! sans ligne `VERDICT:`, donc une question AC2 posée à l'opérateur),
//! `pr_task_active` (une tâche vit), `state_unreadable` (la forge était
//! illisible), `text_touched` (un handler ultérieur parle). Quatre réponses,
//! quatre suites différentes, dont trois ne sont pas des filtres.

use tracing::{info, warn};

use mika_common::github_event_format::{
    BRANCH_MARKER, CHECK_SUITE_EVENT_PREFIX, CheckSuiteHeads, ISSUE_CLOSED_EVENT_PREFIX,
    ISSUE_COMMENT_EVENT_PREFIX, ISSUE_EVENT_PREFIX, ISSUE_LABELED_EVENT_PREFIX,
    PR_CLOSED_EVENT_PREFIX, PR_EVENT_PREFIX, PR_OPENED_EVENT_PREFIX, PR_REVIEW_EVENT_PREFIX,
    PUSH_EVENT_PREFIX, parse_check_suite_heads,
};
use mika_common::llm::LlmProvider;

use crate::async_db::AsyncDatabase;
use crate::qa_reviewed_head::QA_REVIEWED_HEAD_TOOL;

use super::ci_failure_handler::{
    CI_FAILURE_ESCALATED_TOOL, CI_FAILURE_HANDLED_TOOL, CI_FAILURE_NO_ACTION_NO_OPEN_PR,
    CI_FAILURE_NO_ACTION_TASK_NOT_IN_PROGRESS, CI_FAILURE_NO_ACTION_TOOL,
    SURFACE_FOR_ADOPTION_TOOL, branch_key, parse_check_suite_failure,
};
use super::ci_success_handler::{
    DECISION_CORE_HOLD_NOTIFIED, DECISION_CORE_HOLD_TOOL, head_key, parse_check_suite_success,
};
use super::comment_triage::{self, TriageVerdict};
use super::verdict::{Verdict, parse_pr_review_event, parse_verdict, verdict_raw_value};
use super::webhook_queue_v2::{WebhookEventKind, classify_event};

/// Kill-switch, armé par défaut. Même table de vérité que les interrupteurs
/// de mika#2671 ([`crate::qa_head_supersession::parse_switch`]).
pub(crate) const PREFILTER_ENV: &str = "MIKA_WEBHOOK_PREFILTER";

/// Interrupteur de la seule classe (g′), armé par défaut (mika#2917, vol 3).
/// Table de vérité et raison du défaut : [`reviewed_head_enabled`].
pub(crate) const REVIEWED_HEAD_ENV: &str = "MIKA_WEBHOOK_PREFILTER_REVIEWED_HEAD";

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
///
/// **« Sans LLM » admet une exception depuis mika#2854** : la classe (h)
/// ([`PrefilterClass::CommentTriagedNote`]) écarte le tour *complet* au prix
/// d'un brin de triage borné, sans outils. Elle n'écarte donc pas un tour sans
/// appeler de modèle — elle écarte 77 k tokens d'entrée au prix de 160 tokens
/// de sortie. Les treize autres classes restent purement déterministes.
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
    /// (h) `issue_comment` ou `issues` `closed` sur un ticket sans tâche de
    /// boucle, dont le triage n'a trouvé **aucune décision actionnable**
    /// (mika#2854). La seule classe dont la décision passe par un modèle : le
    /// texte est humain et neuf, et c'est pour cette raison que mika#2680 avait
    /// refusé de l'écarter sur un prédicat de forme. Mesurée à 13 tours et
    /// 1 007 748 tokens d'entrée sur la fenêtre du 2026-10-19.
    CommentTriagedNote,
    /// (g′) `check_suite` verte sur une tête de PR que le skill
    /// `qa-review-webhook-success` a **déjà traitée** (mika#2917, vol 3).
    ///
    /// Voisine de [`PrefilterClass::GreenCheckSuiteHeadDecided`] et non son
    /// remplaçante : (b) lit l'audit du MOTEUR — une ligne DECISION-CORE que la
    /// boucle a écrite — là où (g′) lit le registre que le SKILL écrit
    /// ([`crate::qa_reviewed_head`]). Deux sources, deux populations, et la
    /// seconde couvre les tours que la première ne voyait pas : les 51 tours
    /// `check_suite_pr_branch` mesurés sur 4,9 jours qui se terminaient en
    /// `stop_verdict_recorded` (35) ou `stop_already_reviewed_sha` (16), pour
    /// 7,06 M tokens d'entrée.
    ///
    /// **Le transfert de population est attendu, et il n'est pas une
    /// régression.** (g′) est consultée AVANT (b) : un événement que les deux
    /// sauraient écarter s'inscrit désormais ici. La quantité conservée est la
    /// **somme** des deux classes, jamais l'une d'elles (mika#2205).
    GreenCheckSuiteHeadAlreadyReviewed,
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
            Self::CommentTriagedNote => "comment_triaged_note",
            Self::GreenCheckSuiteHeadAlreadyReviewed => "green_check_suite_head_already_reviewed",
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

/// Laquelle des moitiés de la classe (h) a ouvert le tour (mika#2854).
///
/// Ce terme n'existe pas pour brancher, mais pour **séparer les populations
/// dans l'audit** : « 60 % d'escalade sur (h) » n'est pas une mesure si les
/// commentaires et les fermetures y sont additionnés — deux populations sous
/// un seul nom ne se soustraient pas (mika#2205). Il sépare aussi deux
/// chemins de lecture, et c'est la raison d'AC4 :
/// les deux moitiés ne lisent pas l'état du ticket au même endroit — la
/// fermeture le lit AVANT `upstream_close_handler` (voir
/// [`snapshot_closure_task_state`]), le commentaire le lit dans la porte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TriageHalf {
    /// `issue_comment` : du texte humain neuf, la moitié chiffrée par mika#2680.
    Comment,
    /// `issues` `closed` (mika#2854 AC4) : la moitié dont l'état du ticket se
    /// lit sur un instantané d'avant le nettoyage, et non en direct.
    IssueClosed,
}

impl TriageHalf {
    /// Le préfixe de la cible d'audit. Pas un `as_str` de classe : c'est la
    /// clé de regroupement qui sépare les moitiés.
    ///
    /// `"issue_closed"` est déjà l'orthographe de [`TriggerClass::IssueClosed`] :
    /// les deux axes nomment la même moitié du même nom, donc un `GROUP BY`
    /// croisé reste lisible.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Comment => "issue_comment",
            Self::IssueClosed => "issue_closed",
        }
    }
}

/// L'état du ticket lu AVANT `upstream_close_handler` (mika#2854 AC4).
///
/// Quatre variantes et non un `Option<bool>` : « pas une fermeture », « triage
/// désarmé » et « lecture en échec » sont trois causes de non-lecture qu'un
/// `None` fusionnerait, et dont deux sont des défauts de câblage tandis que la
/// troisième est le régime nominal. Classe mika#2205.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClosureSnapshot {
    /// Le texte n'est pas une fermeture de (h) : le terme n'a pas de sujet.
    NotAClosure,
    /// Le triage est désarmé — aucun état n'est lu. Tient la promesse § D8 du
    /// vol 1 : « désarmé, aucun appel n'est fait et aucun état n'est lu ».
    Disarmed,
    /// L'état du ticket avant le nettoyage.
    Read(bool),
    /// La lecture a échoué, et l'erreur est nommée.
    Unreadable(String),
}

impl ClosureSnapshot {
    /// Le nom de la variante, pour le champ `what` du WARN de
    /// [`snapshot_absent`]. Les deux absences ne doivent pas rendre les mêmes
    /// octets : « ce texte n'était pas une fermeture » et « la classe était
    /// désarmée » appellent deux enquêtes différentes (mika#2205).
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::NotAClosure => "not_a_closure",
            Self::Disarmed => "disarmed",
            Self::Read(_) => "read",
            Self::Unreadable(_) => "unreadable",
        }
    }
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
        /// Ce que l'événement nomme de SA tête (mika#2917), lu du texte par
        /// [`parse_check_suite_heads`] — jamais par un appel `gh`, que le
        /// bearing de mika#2680 refuse depuis ce module (mika#2260).
        ///
        /// **Aucun décideur ne le lit dans ce vol.** Le champ est transporté
        /// et rien de plus : la pièce 3 de la forme (a) — comparer cette tête
        /// à l'état de dedup du skill — attend que la pièce 2 existe, et une
        /// garde qui arrive avec le diff qu'elle juge n'a jamais rien refusé
        /// (mika#2578, mika#2752). Ce qui est acquis ici, c'est que la tête
        /// traverse : elle était jetée au gateway.
        ///
        /// [`CheckSuiteHeads::LineAbsent`] n'est pas [`CheckSuiteHeads::Empty`]
        /// — la première dit « binaire de gateway antérieur à mika#2917 »
        /// (motif mika#2340), la seconde « la forge ne nomme aucune PR », ce
        /// qui est la forme mesurée sur mika#2681. Les confondre serait
        /// additionner deux populations sous un nom (mika#2205).
        heads: CheckSuiteHeads,
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
    /// (h) `issue_comment` ou `issues` `closed` dont l'URL d'issue est lisible
    /// (mika#2854). Écartée au terme de [`decide`], et seulement si le triage
    /// ne trouve aucune décision actionnable.
    ///
    /// **Le texte est un champ** parce que c'est lui que le sixième décideur
    /// juge : la même raison qui met les faits d'un verdict dans
    /// [`ReviewVerdictFacts`] plutôt que de relire le gabarit en aval. Le
    /// candidat porte ce que le décideur lit du texte, et pour (h) le texte
    /// *est* le terme — y compris la ligne `by @auteur`, que le prompt
    /// interdit d'utiliser mais qu'on ne retire pas (ce serait une seconde
    /// lecture du gabarit, mika#2854 D2).
    HumanTextNoTask {
        issue_url: String,
        half: TriageHalf,
        text: String,
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
                // La fonction reste pure : la tête se lit dans le MÊME texte,
                // par le parse déclaré dans `mika-common` à côté de la
                // grammaire que le gateway émet (mika#2917).
                heads: parse_check_suite_heads(text),
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
    // (d) reste AVANT (h), et les deux populations sont disjointes **par
    // construction**, non par cet ordre : (h) n'accepte que le terme
    // `IssueComment`, et `trigger_class` teste `ISSUE_LABELED_EVENT_PREFIX`
    // avant `ISSUE_EVENT_PREFIX`, donc un `Issue labeled ` ne peut jamais le
    // rendre. La disjonction est
    // épinglée par `mika2854_h_ne_touche_pas_la_population_de_d` plutôt que
    // laissée à la lecture : ce qu'un `_ =>` protège se perd en silence le jour
    // où quelqu'un y ajoute un bras.
    if let WebhookEventKind::IssueLabeled { repo, issue, label } = classify_event(text)
        && !crate::webhook_dispatch::label_read_by_loop(&label)
    {
        return Some(Candidate::InertLabel { repo, issue, label });
    }
    // (h) mika#2854. La grammaire lue est `trigger_class`, et non un préfixe
    // neuf : `WebhookEventKind` ne porte aucune variante pour ces deux
    // événements (ils tombent sur `Other`), alors que `trigger_class` les
    // distingue déjà — c'est la table que la mesure de mika#2680 a chiffrée.
    //
    // L'URL d'issue est obligatoire, et son absence rend le chemin
    // d'aujourd'hui : elle est la clé de `issue_has_active_task`, donc sans
    // elle on ne peut pas savoir si un ticket est en vol, et AC7 refuse de
    // deviner.
    //
    // **La clé se lit sur la ligne 2 du gabarit, et nulle part ailleurs.** Le
    // gateway y pose l'URL du sujet (`github.rs` : `{comment_url}` pour un
    // commentaire, `{url}` pour une fermeture) ; le scanner
    // d'`extract_issue_url` parcourt tout le texte, donc sur un commentaire de
    // PR (`/pull/N#issuecomment-…`) il prendrait la première URL d'issue
    // citée dans le CORPS — un autre ticket, lu comme sans tâche, et un
    // commentaire de PR qui entre dans (h) alors que § D4 l'en exclut.
    //
    // **Les deux moitiés ne lisent pas l'état du ticket au même endroit**, et
    // c'est pour cela que [`TriageHalf`] existe. `upstream_close_handler`
    // tourne AVANT la porte et termine les lignes de suivi du ticket fermé,
    // donc un `issue_has_active_task` appelé ici lirait un état déjà nettoyé —
    // AC7 serait aveugle précisément sur un ticket en vol. La moitié fermeture
    // consulte donc l'instantané qu'[`snapshot_closure_task_state`] a pris bien
    // plus haut dans `handlers.rs`, et c'est le décideur qui aiguille.
    let half = match trigger_class(text) {
        TriggerClass::IssueComment => TriageHalf::Comment,
        TriggerClass::IssueClosed => TriageHalf::IssueClosed,
        _ => return None,
    };
    let subject_line = text.lines().nth(1)?;
    crate::server::upstream_close_handler::extract_issue_url(subject_line).map(|issue_url| {
        Candidate::HumanTextNoTask {
            issue_url,
            half,
            text: text.to_string(),
        }
    })
}

/// Prendre l'instantané de la moitié fermeture, AVANT `upstream_close_handler`
/// (mika#2854 AC4).
///
/// # Pourquoi cette fonction existe, et pourquoi elle est appelée si tôt
///
/// `upstream_close_handler::try_handle_upstream_close` tourne à
/// `handlers.rs:1994`, donc **avant** la porte du pré-filtre, et marque
/// terminales les lignes de suivi du ticket qui vient de se fermer. Lire l'état
/// du ticket *dans* la porte lirait donc l'état d'APRÈS ce nettoyage : la garde
/// AC7 rendrait `false` exactement sur les tickets en vol, c'est-à-dire qu'elle
/// serait aveugle là où elle sert. L'appel se fait pour cette raison juste après
/// que `original_github_text` a été pris (`handlers.rs:1749`), au plus tôt, et
/// non « juste avant la l. 1994 » — un handler inséré entre les deux rouvrirait
/// la classe en silence.
///
/// # La clé vient de `classify`, pas d'une seconde orthographe
///
/// [`classify`] est une fonction pure sur une `&str` : l'appeler une seconde
/// fois par webhook de fermeture ne coûte rien, et **c'est ce qui garantit par
/// construction que la clé lue ici est la clé que le décideur lira**. La
/// troncature du fragment vit à un seul endroit, l'extraction ; la recopier ici
/// serait la seconde chance de divergence que mika#2854 § D6 a fermée.
///
/// L'ordre des deux gardes est voulu : la classification d'abord — gratuite, et
/// c'est elle qui décide si la question a un sujet — le désarmement ensuite. Un
/// [`ClosureSnapshot::Disarmed`] n'est donc rendu que pour un texte qui
/// *aurait* été trié.
pub(crate) async fn snapshot_closure_task_state(
    original: Option<&str>,
    db: &AsyncDatabase,
) -> ClosureSnapshot {
    let Some(text) = original else {
        return ClosureSnapshot::NotAClosure;
    };
    let Some(Candidate::HumanTextNoTask {
        issue_url,
        half: TriageHalf::IssueClosed,
        ..
    }) = classify(text)
    else {
        return ClosureSnapshot::NotAClosure;
    };
    if !prefilter_enabled() || !comment_triage::triage_enabled() {
        return ClosureSnapshot::Disarmed;
    }
    match db.has_active_self_dev_task_for_issue(&issue_url).await {
        Ok(v) => ClosureSnapshot::Read(v),
        Err(e) => ClosureSnapshot::Unreadable(e.to_string()),
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
    /// (h) une tâche de self-dev vit-elle sur ce ticket ? La clé est l'URL
    /// d'issue **sans fragment** (mika#2854) — `extract_issue_url` la tronque à
    /// l'extraction, parce qu'un `#issuecomment-123` ferait rendre `false` à
    /// tout commentaire et écarterait des tickets en vol.
    async fn issue_has_active_task(&self, issue_url: &str) -> anyhow::Result<bool>;
    /// (h) la moitié fermeture : l'état du ticket tel qu'il était AVANT
    /// `upstream_close_handler` (mika#2854 AC4). Pas d'`issue_url` en argument
    /// et pas d'`async` : la lecture a DÉJÀ eu lieu, dans `handlers.rs`, avant
    /// le nettoyage. Ce terme ne fait que la rendre — ce qui rend « lire en
    /// direct sur une fermeture » non représentable.
    ///
    /// Un `async fn closure_has_active_task(&self, url: &str)` aurait eu
    /// exactement la même forme qu'[`Self::issue_has_active_task`] et se serait
    /// laissé implémenter par une lecture live sans qu'aucun test ne rougisse.
    /// La signature est ce qui interdit la faute.
    fn closure_task_snapshot(&self) -> &ClosureSnapshot;
    /// (h) le brin de triage. Un terme d'état comme les autres, délibérément :
    /// un appel de modèle borné et sans outils est, du point de vue de
    /// [`decide`], une lecture qui peut échouer — et la faire passer par le
    /// trait est ce qui rend les quatre pannes bouchonnables sans réseau.
    async fn triage(&self, text: &str) -> anyhow::Result<TriageVerdict>;
    /// (g′) l'issue attestée au registre pour CETTE tête, telle que
    /// `record_reviewed_head` l'a écrite — `None` quand le registre ne porte pas
    /// la tête (mika#2917, vol 3). La clé est celle de
    /// [`crate::qa_reviewed_head::head_key`], `pr:{repo}#{n}@{sha}`, retapée
    /// nulle part.
    ///
    /// La valeur est rendue **BRUTE** : la validation contre
    /// [`crate::qa_reviewed_head::REVIEWED_HEAD_OUTCOMES`] appartient au
    /// décideur, pour qu'une valeur hors contrat soit une relâche nommée —
    /// `Release::ReviewedHeadOutOfContract` — et non un silence. Normaliser
    /// ici rendrait « le registre porte autre chose » indiscernable de « le
    /// registre ne porte rien », et ce sont deux populations (mika#2205).
    async fn reviewed_head(&self, head_key: &str) -> anyhow::Result<Option<String>>;
}

/// L'état réel : base de l'agent, et `gh` pour la forge.
pub(crate) struct LiveState<'a> {
    pub(crate) db: &'a AsyncDatabase,
    pub(crate) github_token: Option<&'a str>,
    /// L'identité de l'événement, que les handlers posent en `trace_id`.
    pub(crate) request_id: &'a str,
    /// (h) le fournisseur du brin de triage (mika#2854). `Option`, parce que
    /// l'absence doit rendre le **tour complet** et non la note : un `None` ici
    /// est l'une des quatre pannes de [`Release::TriageUnreadable`], jamais un
    /// silence.
    pub(crate) llm: Option<&'a dyn LlmProvider>,
    /// (h) l'instantané pris AVANT `upstream_close_handler` (mika#2854 AC4).
    ///
    /// Une **référence** et non une valeur : le champ transporte une lecture
    /// déjà faite, et un `LiveState` construit sans elle n'a rien à inventer —
    /// il prête `&ClosureSnapshot::NotAClosure`, que le décideur nomme sous sa
    /// propre relâche plutôt que de le confondre avec « ce ticket n'a pas de
    /// tâche ».
    pub(crate) closure_snapshot: &'a ClosureSnapshot,
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

    async fn issue_has_active_task(&self, issue_url: &str) -> anyhow::Result<bool> {
        self.db.has_active_self_dev_task_for_issue(issue_url).await
    }

    fn closure_task_snapshot(&self) -> &ClosureSnapshot {
        self.closure_snapshot
    }

    async fn triage(&self, text: &str) -> anyhow::Result<TriageVerdict> {
        let Some(llm) = self.llm else {
            anyhow::bail!("aucun fournisseur LLM injecté pour le triage (h)");
        };
        comment_triage::run_triage(llm, text).await
    }

    /// (g′) mika#2917, vol 3. **Aucun `gh`** : la base de l'agent, par
    /// l'accesseur de mika#2242.
    ///
    /// L'`agent_id` n'est pas un argument — `latest_audit_event_for_target`
    /// clone celui du handle et le passe comme paramètre SQL 1. Le premier point
    /// de l'audit MPC du vol 3 est donc tenu structurellement : cette signature
    /// ne permet pas de lire le registre d'un autre agent, donc ne permet pas de
    /// le passer de travers.
    ///
    /// Seule la colonne `after_value` est retenue ; `reasoning` et `created_at`
    /// de la projection sont jetés. `after_value` reste un `Option` jusqu'au
    /// décideur et n'est jamais coercé en `""` (mika#2331).
    async fn reviewed_head(&self, head_key: &str) -> anyhow::Result<Option<String>> {
        let since = crate::qa_reviewed_head::reviewed_head_lookup_since();
        Ok(self
            .db
            .latest_audit_event_for_target(QA_REVIEWED_HEAD_TOOL, head_key, &since)
            .await?
            .and_then(|(after_value, _reasoning, _created_at)| after_value))
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

/// POURQUOI le tour a lieu : le terme de relâche (mika#2680, vol 4).
///
/// Un terme par porte qui relâche un tour, et le registre en porte exactement
/// un. Les trois premiers sont rendus par [`skip_turn`] lui-même, avant toute
/// consultation d'état ; les quinze autres par [`decide`] et ses sept décideurs
/// (le compte annonçait « quatre » alors que [`decide_pr_closed`] en était le
/// cinquième depuis mika#2680 : corrigé au vol 4, en même temps que le
/// sixième ; le septième est `decide_green_pr_branch`, mika#2917 vol 3).
/// Un dix-neuvième site de relâche ne compile pas sans son terme : c'est la
/// propriété que ce type achète, et la raison de ne pas s'être contenté d'une
/// chaîne au site d'émission.
///
/// Ce n'est pas la taxonomie inverse de [`PrefilterClass`] : une classe nomme
/// l'événement écarté, un terme nomme la porte qui n'a pas écarté. Les deux
/// vivent côte à côte sur deux moitiés du registre, et aucun des deux ne se
/// dérive de l'autre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Release {
    /// Interrupteur désarmé : `skip_turn` rend la main sans rien classer.
    ///
    /// Double délibérément `prefilter_enabled: false`, déjà sur la ligne. Le
    /// registre doit pouvoir se lire sur la seule colonne `release` — une
    /// jointure mentale entre deux champs est ce qu'un terme achète.
    SwitchDisarmed,
    /// `classify` n'a rien reconnu : l'événement n'est d'aucune population
    /// candidate. Le gros du trafic, et le dénominateur de toute mesure de
    /// couverture.
    NotClassified,
    /// Un handler a touché le texte dans la fenêtre que la classe exige
    /// (`Touched::applies_to`). Fail-safe de la phase 2 : le tour a lieu parce
    /// que le texte n'est plus celui que la classe sait lire.
    TextTouched,
    /// (a) `branch_has_active_task` → `Ok(true)` : une tâche vit sur la
    /// branche, son état est à prendre en compte (`decide_default_branch`).
    BranchTaskActive,
    /// (c) revue et (e) `PR closed` : `pr_has_active_task` → `Ok(true)`. Deux
    /// sites, un seul terme — c'est le même prédicat sur la même clé, et les
    /// distinguer ferait porter au registre la structure du code plutôt que
    /// celle de la décision.
    PrTaskActive,
    /// (c) revue : la PR est `Open` côté forge, aucun des trois termes lus dans
    /// le texte n'a mordu (`decide_review_verdict`).
    PrOpen,
    /// (c) CI rouge : l'audit de cet événement ne porte aucune trace terminale
    /// que `ci_failure_no_action_class` reconnaisse (`decide_ci_failure_no_action`).
    NoTerminalTrace,
    /// (b) : aucune ligne DECISION-CORE dans l'audit de cet événement, donc
    /// aucune tête à comparer (`decide_head_decided`, premier retour).
    NoDecisionAttested,
    /// (b) : la tête de cet événement n'a pas d'attestation antérieure — c'est
    /// son premier traitement (`head_decided_and_notified` → `None`).
    HeadNotPreviouslyDecided,
    /// Un état est illisible : le tour a lieu (AC3). **UN** terme pour les cinq
    /// appels de [`state_unreadable`], parce que le champ `what` du `warn!`
    /// nomme déjà le prédicat qui a échoué — en faire cinq termes dupliquerait
    /// cette information dans le registre, où la question est « combien de
    /// tours une panne de lecture a-t-elle coûtés », jamais « laquelle ».
    StateUnreadable,
    /// (h) le triage a lu une décision actionnable dans le texte : le tour
    /// complet a lieu (mika#2854). **Le numérateur du taux d'escalade** —
    /// `triage_escalated / (triage_escalated + comment_triaged_note)` est la
    /// mesure que le vol existe pour rendre chiffrable.
    TriageEscalated,
    /// (h) le triage n'a pas rendu de verdict exploitable : erreur de transport,
    /// expiration des 20 s, sortie hors grammaire, ou aucun fournisseur LLM
    /// injecté. **Fail-open** : le tour complet a lieu, jamais la note.
    ///
    /// Distinct de [`Release::StateUnreadable`] pour une raison de mesure, pas
    /// de symétrie : une panne de triage est le coût de la classe (h) et se
    /// soustrait de son économie, là où une panne de lecture d'état est un
    /// incident de base de données partagé par cinq classes. Les additionner
    /// rendrait l'économie de (h) illisible (mika#2205).
    TriageUnreadable,
    /// (h) `issue_has_active_task` → `Ok(true)` : un vol est en cours sur ce
    /// ticket, le tour passe par le chemin d'aujourd'hui sans qu'aucun triage
    /// soit appelé (AC7). Terme propre, et non [`Release::PrTaskActive`] : ce
    /// n'est ni le même prédicat ni la même clé — une URL d'issue, pas de PR.
    IssueTaskActive,
    /// (h) `MIKA_WEBHOOK_COMMENT_TRIAGE` absent ou `off` : la classe est livrée
    /// désarmée (mika#2854 D8), aucun appel n'est fait et le tour a lieu.
    ///
    /// Le terme existe pour que « la classe est désarmée » et « la classe est
    /// armée et escalade tout » ne rendent pas les mêmes octets : sans lui, un
    /// armement oublié ressemblerait à un triage parfaitement prudent.
    TriageDisarmed,
    /// (h) moitié fermeture, armée, et pourtant aucun instantané (mika#2854
    /// AC4) : le site de capture de `handlers.rs` n'a pas tourné, ou il a
    /// tourné sur un autre texte. **Fail-open** — le tour complet a lieu.
    ///
    /// Une sixième relâche et non un réemploi de [`Release::TriageUnreadable`] :
    /// « le triage n'a pas pu se prononcer » et « l'instantané n'a jamais été
    /// pris » sont deux populations, et les fusionner rendrait la prochaine
    /// mesure incapable de distinguer une panne de modèle d'un site de capture
    /// qui a sauté (mika#2205). En régime nominal ce terme vaut **zéro**, et
    /// c'est ce zéro qui atteste le câblage.
    ClosureSnapshotAbsent,
    /// (g′) le payload nomme UNE PR et sa tête, mais le registre du skill ne
    /// porte **aucune** ligne pour cette tête (mika#2917, vol 3). (b) est alors
    /// consultée entière, et c'est son verdict qui décide ; ce terme ne se lit
    /// que lorsque (b) ne mord pas non plus.
    ///
    /// **Le terme nominal du régime, et non une panne.** C'est ce que rend une
    /// tête neuve — le premier `check_suite` d'un push — et c'est le contrôle
    /// négatif d'AC1 : une nouvelle tête sur une PR déjà revue passe. Son
    /// effondrement à zéro serait le symptôme à craindre, pas sa présence.
    ReviewedHeadAbsent,
    /// (g′) le registre porte une ligne pour cette tête, mais sa valeur n'est
    /// pas l'une de celles que [`crate::qa_reviewed_head::REVIEWED_HEAD_OUTCOMES`]
    /// déclare (mika#2917, vol 3). **Fail-safe** : (b) est consultée, et le tour
    /// a lieu si elle ne mord pas.
    ///
    /// Distinct de [`Release::StateUnreadable`], qui dit « la base n'a pas
    /// répondu » : ici la base a répondu, et c'est l'écrivain qui a écrit hors
    /// contrat. Deux populations, deux causes, deux gestes de réparation — les
    /// fusionner enverrait le diagnostic vers SQLite quand le défaut est dans le
    /// prompt du skill (mika#2205).
    ///
    /// En régime attendu ce terme vaut **zéro** : l'outil `record_reviewed_head`
    /// est le seul écrivain et il valide sa valeur. Un non-zéro nomme une
    /// divergence entre l'outil et cette liste.
    ReviewedHeadOutOfContract,
    /// (g′) `MIKA_WEBHOOK_PREFILTER_REVIEWED_HEAD` à `off` : la classe est
    /// désarmée et (b) décide seule, exactement comme avant ce vol (mika#2917,
    /// vol 3).
    ///
    /// Le terme existe pour que « la classe est désarmée » et « la classe est
    /// armée et ne trouve jamais rien » ne rendent pas les mêmes octets :
    /// sans lui, un désarmement oublié ressemblerait à un registre muet, et le
    /// diagnostic partirait vers l'écrivain. En régime attendu — l'interrupteur
    /// est armé par défaut — ce terme vaut **zéro**, et c'est ce zéro qui dit
    /// que la classe tourne.
    ReviewedHeadSwitchDisarmed,
}

impl Release {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SwitchDisarmed => "switch_disarmed",
            Self::NotClassified => "not_classified",
            Self::TextTouched => "text_touched",
            Self::BranchTaskActive => "branch_task_active",
            Self::PrTaskActive => "pr_task_active",
            Self::PrOpen => "pr_open",
            Self::NoTerminalTrace => "no_terminal_trace",
            Self::NoDecisionAttested => "no_decision_attested",
            Self::HeadNotPreviouslyDecided => "head_not_previously_decided",
            Self::StateUnreadable => "state_unreadable",
            Self::TriageEscalated => "triage_escalated",
            Self::TriageUnreadable => "triage_unreadable",
            Self::IssueTaskActive => "issue_task_active",
            Self::TriageDisarmed => "triage_disarmed",
            Self::ClosureSnapshotAbsent => "closure_snapshot_absent",
            Self::ReviewedHeadAbsent => "reviewed_head_absent",
            Self::ReviewedHeadOutOfContract => "reviewed_head_out_of_contract",
            Self::ReviewedHeadSwitchDisarmed => "reviewed_head_switch_disarmed",
        }
    }
}

/// L'issue de la décision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decision {
    Llm(Release),
    Skip {
        class: PrefilterClass,
        target: String,
    },
    /// Écarté **et** porteur d'une trace en mémoire (mika#2854). Troisième
    /// variante plutôt qu'un `Option<String>` sur [`Decision::Skip`] : les
    /// douze autres classes écartent sans rien écrire, et un champ qu'elles
    /// rempliraient toutes de `None` invite à l'oublier au treizième site.
    /// Ici la note est la raison même de la variante.
    SkipWithNote {
        class: PrefilterClass,
        target: String,
        note: String,
    },
}

/// La SONDE d'un état illisible, sans le verdict (mika#2917, vol 3).
///
/// Extraite de [`state_unreadable`] parce que (g′) a besoin des deux moitiés
/// **séparément** : elle doit nommer la panne de lecture du registre — sinon une
/// base muette et un registre vide rendent les mêmes octets — puis consulter (b)
/// entière, qui peut très bien écarter le tour. Un seul appel rendant
/// `Decision::Llm` aurait forcé (g′) à relâcher le tour pour pouvoir le dire.
///
/// Les cinq appelants historiques passent par [`state_unreadable`], inchangé :
/// l'extraction n'ajoute pas un second site d'émission, elle en sépare les deux
/// moitiés.
fn warn_state_unreadable(what: &str, target: &str, e: &anyhow::Error) {
    warn!(
        event = "webhook_prefilter_state_unreadable",
        what,
        target = %target,
        error = %e,
        "pré-filtre : état illisible — le tour a lieu (fail-safe)"
    );
}

fn state_unreadable(what: &str, target: &str, e: &anyhow::Error) -> Decision {
    warn_state_unreadable(what, target, e);
    Decision::Llm(Release::StateUnreadable)
}

/// Le pendant de [`state_unreadable`] pour le brin de triage (mika#2854).
///
/// Quatre pannes convergent ici — transport, expiration, verdict hors
/// grammaire, fournisseur absent — et toutes rendent le **tour complet**.
/// C'est l'invariant de fail-open du ticket, et il est tenu par le type de
/// retour : cette fonction n'a aucun chemin qui rende un [`Decision::Skip`]
/// ou un [`Decision::SkipWithNote`].
fn triage_unreadable(target: &str, e: &anyhow::Error) -> Decision {
    warn!(
        event = "webhook_triage_unreadable",
        target = %target,
        error = %e,
        "triage (h) : pas de verdict exploitable — le tour complet a lieu (fail-open)"
    );
    Decision::Llm(Release::TriageUnreadable)
}

/// La moitié fermeture est armée, et pourtant aucun instantané n'a été pris
/// (mika#2854 AC4).
///
/// Le seul chemin qui mène ici est un défaut de câblage : la capture de
/// `handlers.rs` n'a pas tourné, ou elle a tourné sur un autre texte que celui
/// que la porte classe. Ce n'est **jamais** « ce ticket n'a pas de tâche de
/// boucle » — confondre les deux donnerait une note à un ticket en vol, ce que
/// AC7 refuse.
///
/// `what` porte le nom de la variante et non celui du prédicat : les deux
/// absences possibles — « ce texte n'était pas une fermeture » et « la classe
/// était désarmée » — appellent deux enquêtes différentes, et un nom unique les
/// rendrait non soustractibles (mika#2205).
fn snapshot_absent(snapshot: &ClosureSnapshot, target: &str) -> Decision {
    warn!(
        event = "webhook_closure_snapshot_absent",
        what = snapshot.as_str(),
        target = %target,
        "triage (h) fermeture : aucun instantané d'avant le nettoyage — le tour complet a lieu (fail-open)"
    );
    Decision::Llm(Release::ClosureSnapshotAbsent)
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
        // `heads` est enfin lu (mika#2917, vol 3) : le vol 1 l'a mis dans le
        // texte, le vol 2 a posé le registre, et c'est ici que les deux se
        // comparent. (b) reste branchée **entière** derrière (g′).
        Candidate::GreenPrBranch {
            repo,
            branch,
            heads,
        } => decide_green_pr_branch(&repo, &branch, &heads, state).await,
        Candidate::RedPrBranch { repo, branch } => {
            decide_ci_failure_no_action(&repo, &branch, state).await
        }
        Candidate::InertLabel { repo, issue, .. } => Decision::Skip {
            class: PrefilterClass::InertLabel,
            target: format!("issue:{repo}#{issue}"),
        },
        Candidate::ReviewVerdict(facts) => decide_review_verdict(facts, state).await,
        Candidate::PrClosedNoTask { pr_url } => decide_pr_closed(pr_url, state).await,
        Candidate::HumanTextNoTask {
            issue_url,
            half,
            text,
        } => decide_comment_triage(&issue_url, half, &text, state).await,
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
        Ok(true) => Decision::Llm(Release::BranchTaskActive),
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
        None => Decision::Llm(Release::NoTerminalTrace),
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
        return Decision::Llm(Release::NoDecisionAttested);
    };
    let prior = match state.decision_core_holds(prefix).await {
        Ok(rows) => rows,
        Err(e) => return state_unreadable("decision_core_holds", prefix, &e),
    };
    match head_decided_and_notified(repo, &this_event, &prior) {
        Some(head) => Decision::Skip {
            class: PrefilterClass::GreenCheckSuiteHeadDecided,
            target: check_suite_target(&head),
        },
        None => Decision::Llm(Release::HeadNotPreviouslyDecided),
    }
}

/// La cible qu'une classe de tête inscrit au registre : la clé
/// [`head_key`] privée de son préfixe `pr:`. **Un seul site**, partagé par (b)
/// et (g′) — deux `format!` jumeaux auraient fait porter au registre la
/// structure du code, et un `GROUP BY target` d'opérateur aurait vu deux
/// grammaires là où il y a une cible (mika#2158).
///
/// `strip_prefix` et jamais un découpage indexé : `scripts/check-byte-slices.sh`
/// (mika#764) refuse la seconde forme, et le `unwrap_or` rend la clé telle
/// quelle plutôt que de paniquer sur une forme inattendue.
fn check_suite_target(head: &str) -> String {
    format!("check_suite:{}", head.strip_prefix("pr:").unwrap_or(head))
}

/// (g′) puis (b) : le septième décideur (mika#2917, vol 3).
///
/// **Une seule forme, quatre bras.** (g′) tente l'écart sur la seule population
/// qu'elle sait juger — le texte nomme UNE PR et sa tête ; si elle ne l'obtient
/// pas, [`decide_head_decided`] est consultée **entière**, et c'est son `Skip`
/// qui l'emporte quand elle mord. La relâche rendue est alors celle que (g′) a
/// nommée, parce qu'elle dit pourquoi la porte la plus proche de la cause n'a
/// pas servi — là où `NoDecisionAttested` ne dirait que « (b) n'avait rien à
/// comparer », ce qu'on savait déjà.
///
/// **Rien n'est retiré à (b) sur aucune entrée.** C'est la propriété de
/// non-régression du vol, et elle tient structurellement : (b) est appelée sur
/// les quatre bras et sur les trois populations que (g′) ne touche pas. Le
/// désarmement de l'interrupteur rend donc exactement le régime de `main` avant
/// ce vol, pas un troisième régime.
///
/// **L'ordre de consultation a une conséquence mesurable, et elle n'est pas une
/// régression** : un événement que les deux porte sauraient écarter s'inscrit
/// désormais sous (g′). `green_check_suite_head_decided` baisse donc au profit
/// de `green_check_suite_head_already_reviewed`, et la grandeur conservée est la
/// **somme** des deux classes (mika#2205). Idem pour `no_decision_attested` et
/// `head_not_previously_decided`, qui passent à `reviewed_head_absent` sur cette
/// population et restent intégralement en service sur les trois autres.
///
/// Aucun appel `gh` : la tête vient du texte (vol 1), le registre vient de la
/// base de l'agent (vol 2, scopée par `agent_id`). C'est le refus non
/// négociable du bearing de mika#2680 (mika#2260).
async fn decide_green_pr_branch(
    repo: &str,
    branch: &str,
    heads: &CheckSuiteHeads,
    state: &impl PrefilterState,
) -> Decision {
    // `LineAbsent`, `Empty` et `Many` : (b) seule, relâches comprises, comme
    // avant ce vol. Trois populations distinctes qui partagent un bras parce
    // qu'aucune ne donne de tête unique à comparer — jamais parce qu'elles
    // seraient la même chose (mika#2205, mika#2331).
    let CheckSuiteHeads::One { pr, head_sha } = heads else {
        return decide_head_decided(repo, branch, state).await;
    };
    // L'interrupteur est lu ICI et non dans `skip_turn` : désarmer (g′) doit
    // laisser (b) décider, ce qui serait impossible depuis la porte.
    if !reviewed_head_enabled() {
        return head_decided_or(Release::ReviewedHeadSwitchDisarmed, repo, branch, state).await;
    }
    let key = head_key(repo, *pr, head_sha);
    match state.reviewed_head(&key).await {
        // L'unique chemin qui écarte dans tout ce vol.
        Ok(Some(raw)) if crate::qa_reviewed_head::reviewed_head_outcome(&raw).is_some() => {
            Decision::Skip {
                class: PrefilterClass::GreenCheckSuiteHeadAlreadyReviewed,
                target: check_suite_target(&key),
            }
        }
        // La base a répondu, et l'écrivain a écrit hors contrat. Nommé, parce
        // qu'un silence enverrait le diagnostic vers SQLite.
        Ok(Some(_)) => {
            head_decided_or(Release::ReviewedHeadOutOfContract, repo, branch, state).await
        }
        Ok(None) => head_decided_or(Release::ReviewedHeadAbsent, repo, branch, state).await,
        Err(e) => {
            // Le WARN seul, puis le repli : `state_unreadable` rendrait la
            // `Decision` immédiatement et forcerait (g′) à relâcher le tour
            // pour pouvoir nommer la panne.
            warn_state_unreadable("reviewed_head", &key, &e);
            head_decided_or(Release::StateUnreadable, repo, branch, state).await
        }
    }
}

/// Le repli de (g′) sur (b), avec la relâche que (g′) a nommée.
///
/// Un seul site, appelé sur les quatre bras de [`decide_green_pr_branch`] : la
/// forme « tente, replie, nomme » ne se répète donc pas quatre fois, et un
/// cinquième bras ne peut pas oublier le repli.
async fn head_decided_or(
    release: Release,
    repo: &str,
    branch: &str,
    state: &impl PrefilterState,
) -> Decision {
    match decide_head_decided(repo, branch, state).await {
        Decision::Llm(_) => Decision::Llm(release),
        ecarte => ecarte,
    }
}

/// (c) : sans tâche active, ET l'un des quatre termes de « non actionnable ».
/// Les trois termes lus dans le texte passent avant la forge, pour ne payer
/// `gh` qu'au besoin.
async fn decide_review_verdict(facts: ReviewVerdictFacts, state: &impl PrefilterState) -> Decision {
    let target = format!("pr_review:{}#{}", facts.repo, facts.pr_number);
    match state.pr_has_active_task(&facts.pr_url).await {
        Ok(false) => {}
        Ok(true) => return Decision::Llm(Release::PrTaskActive),
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
            Ok(PrForgeState::Open) => return Decision::Llm(Release::PrOpen),
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
        Ok(true) => Decision::Llm(Release::PrTaskActive),
        Err(e) => state_unreadable("pr_has_active_task", &target, &e),
    }
}

/// (h) le sixième décideur, et le seul qui consulte un modèle (mika#2854).
///
/// **Trois portes, dans cet ordre, et l'ordre est le contrat :**
///
/// 1. L'interrupteur dédié. Désarmé, aucun appel n'est fait et aucun état n'est
///    lu : un vol livré désarmé ne doit pas peser sur la base.
/// 2. L'état du ticket. Un ticket en vol passe par le chemin d'aujourd'hui
///    (AC7) **sans qu'aucun triage soit appelé** — c'est ce que l'ordre achète,
///    et un test le prouve en rendant la panique sur un triage non attendu.
/// 3. Le triage. `DECISION` escalade, `NOTE:` écarte avec sa note, tout le reste
///    escalade aussi ([`triage_unreadable`]).
///
/// **La deuxième porte a deux corps, un par moitié** (mika#2854 AC4), et c'est
/// la seule différence entre les deux : la moitié commentaire lit en direct,
/// la moitié fermeture consulte l'instantané pris AVANT
/// `upstream_close_handler` ([`snapshot_closure_task_state`]). Une lecture live
/// sur une fermeture verrait l'état d'après le nettoyage, donc AC7 serait
/// aveugle exactement là où elle compte. L'**ordre** des trois portes, lui, est
/// inchangé : la bifurcation remplace le corps de la deuxième, pas sa place.
///
/// La cible porte la moitié en préfixe, pour que les deux populations de (h)
/// restent soustractibles dans l'audit ([`TriageHalf`]).
async fn decide_comment_triage(
    issue_url: &str,
    half: TriageHalf,
    text: &str,
    state: &impl PrefilterState,
) -> Decision {
    let target = format!("{}:{issue_url}", half.as_str());
    if !comment_triage::triage_enabled() {
        return Decision::Llm(Release::TriageDisarmed);
    }
    let active = match half {
        TriageHalf::Comment => match state.issue_has_active_task(issue_url).await {
            Ok(v) => v,
            Err(e) => return state_unreadable("issue_has_active_task", &target, &e),
        },
        TriageHalf::IssueClosed => match state.closure_task_snapshot() {
            ClosureSnapshot::Read(v) => *v,
            ClosureSnapshot::Unreadable(e) => {
                return state_unreadable(
                    "closure_task_snapshot",
                    &target,
                    &anyhow::anyhow!(e.clone()),
                );
            }
            // Armé, (h), et pourtant pas d'instantané : un défaut de câblage, et
            // jamais « ce ticket n'a pas de tâche ». Le tour a lieu, et la
            // relâche le dit sous son propre nom.
            absent => return snapshot_absent(absent, &target),
        },
    };
    if active {
        return Decision::Llm(Release::IssueTaskActive);
    }
    match state.triage(text).await {
        Ok(TriageVerdict::Decision) => Decision::Llm(Release::TriageEscalated),
        Ok(TriageVerdict::Note(note)) => Decision::SkipWithNote {
            class: PrefilterClass::CommentTriagedNote,
            target,
            note,
        },
        Err(e) => triage_unreadable(&target, &e),
    }
}

/// L'interrupteur est-il armé ? Lu à chaque événement.
pub(crate) fn prefilter_enabled() -> bool {
    crate::qa_head_supersession::parse_switch(
        PREFILTER_ENV,
        std::env::var(PREFILTER_ENV).ok().as_deref(),
    )
}

/// L'interrupteur de (g′) est-il armé ? Lu à chaque `check_suite` verte de
/// branche de PR (mika#2917, vol 3).
///
/// **Exactement la table de vérité de [`prefilter_enabled`], l'absence
/// comprise : armé.** C'est la ligne qui diverge de
/// [`super::comment_triage::triage_enabled`], et la divergence est argumentée,
/// pas héritée :
///
/// - (h) est livrée **désarmée** parce qu'elle appelle un modèle sur du texte
///   humain et neuf, et qu'un armement par déploiement y engagerait une dépense
///   que personne n'a demandée.
/// - (g′) est livrée **armée** parce qu'elle ne décide sur rien de neuf : elle
///   compare une tête à un registre que le skill a écrit, et son seul effet est
///   de ne pas repayer un tour dont la conclusion est déjà inscrite. La livrer
///   désarmée reviendrait à déployer la mesure sans la mesurer — exactement la
///   forme que mika#2578/#2752 condamnent, une garde que rien n'exécute.
///
/// Le reste de la table est celui de tout interrupteur du dépôt, **y compris la
/// valeur non reconnue qui laisse l'interrupteur armé** avec son WARN
/// `qa_switch_invalid` : taper le nom de la variable *est* l'adhésion, en
/// garbler la valeur est une faute de frappe.
///
/// `MIKA_WEBHOOK_PREFILTER` désarme en amont : (g′) vit dans le chemin du
/// pré-filtre, donc le désarmement global l'emporte sans que cette fonction ait
/// à le savoir.
///
/// **Lu dans le décideur, pas dans [`skip_turn`]** : un désarmement doit laisser
/// (b) décider comme avant ce vol, et non relâcher le tour. Faire traverser
/// l'interrupteur par `skip_turn` puis `decide` ferait porter à sept décideurs
/// un argument qu'un seul lit.
pub(crate) fn reviewed_head_enabled() -> bool {
    crate::qa_head_supersession::parse_switch(
        REVIEWED_HEAD_ENV,
        std::env::var(REVIEWED_HEAD_ENV).ok().as_deref(),
    )
}

/// Le garde RAII de [`REVIEWED_HEAD_ENV`], jumeau de
/// [`super::comment_triage::TriageEnvGuard`] (mika#2917, vol 3).
///
/// Un second type plutôt qu'un garde générique sur un nom de variable : le
/// générique serait plus court et rendrait possible de surveiller la mauvaise
/// variable sans que rien ne le dise. Le prix est vingt lignes jumelles ; le
/// prix de l'autre forme est un test vert qui ne mesure rien.
///
/// Tout test qui le construit porte `#[serial_test::serial]` : l'environnement
/// est un état global du processus.
#[cfg(test)]
pub(crate) struct ReviewedHeadEnvGuard(Option<String>);

#[cfg(test)]
impl ReviewedHeadEnvGuard {
    /// `None` retire la variable : la ligne « absente » de la table de vérité,
    /// celle du régime de production.
    pub(crate) fn set(value: Option<&str>) -> Self {
        let previous = std::env::var(REVIEWED_HEAD_ENV).ok();
        // SAFETY: les tests qui construisent ce garde sont sérialisés par
        // `#[serial_test::serial]`, donc aucun autre fil ne lit
        // l'environnement pendant la mutation.
        match value {
            Some(v) => unsafe { std::env::set_var(REVIEWED_HEAD_ENV, v) },
            None => unsafe { std::env::remove_var(REVIEWED_HEAD_ENV) },
        }
        Self(previous)
    }

    /// Armé (`1`) ou désarmé (`0`).
    pub(crate) fn armed(armed: bool) -> Self {
        Self::set(Some(if armed { "1" } else { "0" }))
    }
}

#[cfg(test)]
impl Drop for ReviewedHeadEnvGuard {
    fn drop(&mut self) {
        // SAFETY: même sérialisation qu'à la pose.
        match self.0.take() {
            Some(v) => unsafe { std::env::set_var(REVIEWED_HEAD_ENV, v) },
            None => unsafe { std::env::remove_var(REVIEWED_HEAD_ENV) },
        }
    }
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
            // (h) rejoint la même fenêtre, et pour une raison plus forte encore
            // (mika#2854) : ce que (h) lit du texte, c'est le texte ENTIER — un
            // handler qui l'enrichit change ce que le triage juge, donc peut
            // changer le verdict. Un texte touché rend le tour complet, qui est
            // le côté sûr.
            Candidate::GreenDefaultBranch { .. }
            | Candidate::InertLabel { .. }
            | Candidate::PrClosedNoTask { .. }
            | Candidate::HumanTextNoTask { .. } => self.by_any_handler,
        }
    }
}

/// Ce que la porte a fait de ce tour : écarté, ou relâché pour un motif nommé.
///
/// Remplace le `bool` d'origine. Un booléen rendait indicible la moitié
/// intéressante — « le tour a lieu » était un `false` sans cause, et le motif
/// n'existait qu'au moment de la décision, à l'intérieur de cette fonction,
/// d'où il ne ressortait pas. Le type le fait ressortir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Gate {
    Skipped,
    Charged(Release),
}

/// La porte. Rend [`Gate::Skipped`] quand le tour LLM est écarté — une ligne
/// `info!` et une ligne d'audit nommées sont alors écrites ; sinon
/// [`Gate::Charged`] avec le terme de la porte qui a relâché (mika#2680).
pub(crate) async fn skip_turn(
    db: &AsyncDatabase,
    state: &impl PrefilterState,
    original_text: &str,
    touched: Touched,
    request_id: &str,
    enabled: bool,
) -> Gate {
    if !enabled {
        return Gate::Charged(Release::SwitchDisarmed);
    }
    let Some(candidate) = classify(original_text) else {
        return Gate::Charged(Release::NotClassified);
    };
    if touched.applies_to(&candidate) {
        return Gate::Charged(Release::TextTouched);
    }
    // La note de (h) est portée à part, et versée APRÈS la ligne d'audit : le
    // registre est le contrat (AC5) et ne dépend pas d'une écriture en mémoire,
    // là où une note sans sa ligne d'audit serait une économie non chiffrable.
    let (class, target, note) = match decide(candidate, state).await {
        Decision::Llm(release) => return Gate::Charged(release),
        Decision::Skip { class, target } => (class, target, None),
        Decision::SkipWithNote {
            class,
            target,
            note,
        } => (class, target, Some(note)),
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
    if let Some(note) = note {
        comment_triage::write_note(db, &target, &note).await;
    }
    Gate::Skipped
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
            // Un seul nom pour les deux moitiés : la moitié vit dans la cible de
            // la ligne d'audit ([`TriageHalf::as_str`]), et la dupliquer ici
            // ferait deux endroits où elle peut divergir.
            Self::HumanTextNoTask { .. } => "human_text_no_task",
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
///
/// `release` ne se dérive pas ici : il vient du [`Gate::Charged`] que
/// [`skip_turn`] vient de rendre, seul endroit où la porte qui a relâché est
/// connue. Le reconstruire depuis `original_text` et `touched` le ferait
/// divergir de la vraie porte au premier décideur qui change d'ordre.
pub(crate) fn emit_turn_charged(
    agent_id: &str,
    original_text: &str,
    touched: Touched,
    request_id: &str,
    enabled: bool,
    release: Release,
) {
    let trigger = trigger_class(original_text).as_str();
    let text_touched = touched.by_any_handler;
    let release = release.as_str();

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
            release,
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
            release,
            "tour webhook facturé — le tour a lieu (mika#2680)"
        ),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// AC1 **sous vrai modèle** (mika#2854, vol 3/3) : le corpus réel joué
    /// contre un vrai fournisseur, derrière `#[ignore]` +
    /// `MIKA_EVAL_TRIAGE_PROVIDERS`.
    ///
    /// Enfant de ce module, et non voisin sous `tests/` du crate : il pilote le
    /// `decide` privé à travers le `PrefilterState` `pub(crate)`, que seule une
    /// éval intra-caisse atteint. Un enfant voit les items privés de son
    /// ancêtre ; un voisin non (même idiome que `db/tests/`, mika#2321).
    mod ac1_vrai_modele_2854;

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
                    branch: branch.into(),
                    // Ces deux textes sont ceux d'avant mika#2917 : ils ne
                    // portent AUCUNE ligne de tête, et c'est ce que
                    // `LineAbsent` dit. Un gateway à jour écrit `none` sur un
                    // payload sans PR, donc `Empty` — les deux valeurs ne se
                    // confondent pas, et l'ancien texte continue d'entrer
                    // dans la population exactement comme avant.
                    heads: CheckSuiteHeads::LineAbsent,
                }),
                "{text}"
            );
        }
    }

    /// Le texte d'un `check_suite` d'avant mika#2917 et celui d'après, tels
    /// que le gateway les émet.
    ///
    /// Le second est obtenu en ajoutant les lignes 2 et 3 au premier, comme
    /// `format_event_text` le fait — et non recopié à la main : les deux
    /// moitiés du couple doivent rester le même premier mot pour que le test
    /// dise quelque chose.
    fn mika2917_ancien_puis_nouveau(premiere_ligne: &str) -> (String, String) {
        use mika_common::github_event_format::{
            CHECK_SUITE_HEAD_LINE_PREFIX, CHECK_SUITE_PRS_LINE_PREFIX,
        };
        let nouveau = format!(
            "{premiere_ligne}\n\
             {CHECK_SUITE_HEAD_LINE_PREFIX}1a2b3c4d\n\
             {CHECK_SUITE_PRS_LINE_PREFIX}2917@1a2b3c4d"
        );
        (premiere_ligne.to_string(), nouveau)
    }

    /// mika#2917 — les QUATRE lecteurs du texte `check_suite` lisent l'ancien
    /// et le nouveau texte **identiquement**.
    ///
    /// C'est la Definition of Done de ce vol, et la raison pour laquelle la
    /// tête s'écrit en lignes 2 et 3. Les quatre ne lisent que la première
    /// ligne — `parse_check_suite_success` et `parse_check_suite_failure` par
    /// `lines().next()` puis `strip_suffix(')')`, les deux `CHECK_SUITE_RE` par
    /// un motif ancré en tête dont le `[^)]+` s'arrête sur la parenthèse. Mais
    /// cette propriété est tenue par quatre sites indépendants, dont deux
    /// régulières recopiées l'une de l'autre : elle doit être épinglée, pas
    /// déduite. Trois des quatre casseraient en silence.
    #[test]
    fn mika2917_les_quatre_lecteurs_lisent_lancien_et_le_nouveau_texte_identiquement() {
        use super::super::webhook_queue::correlate_webhook;

        // (1) et (3) — le succès : `parse_check_suite_success` et les deux
        // régulières, sur la forme verte.
        let (ancien, nouveau) = mika2917_ancien_puis_nouveau(
            "[GitHub] Check suite success on senara-solutions/mika (branch: feat/2917/x)",
        );
        for (etiquette, texte) in [("ancien", &ancien), ("nouveau", &nouveau)] {
            let event = parse_check_suite_success(texte).unwrap_or_else(|| {
                panic!("`parse_check_suite_success` ne lit plus le {etiquette}")
            });
            assert_eq!(event.repo, "senara-solutions/mika", "{etiquette}");
            assert_eq!(event.branch, "feat/2917/x", "{etiquette}");

            // `CHECK_SUITE_RE` de `webhook_queue` (mika#528), par son seul
            // consommateur.
            let correlation = correlate_webhook(texte)
                .unwrap_or_else(|| panic!("`correlate_webhook` ne corrèle plus le {etiquette}"));
            assert_eq!(
                correlation.branch,
                Some("feat/2917/x".to_string()),
                "{etiquette}"
            );
            assert!(
                correlation.event_desc.contains("success"),
                "{etiquette} : {correlation:?}"
            );

            // `CHECK_SUITE_RE` de `webhook_queue_v2`, le jumeau recopié.
            assert_eq!(
                classify_event(texte),
                WebhookEventKind::CheckSuite {
                    repo: "senara-solutions/mika".to_string(),
                    branch: "feat/2917/x".to_string(),
                },
                "{etiquette}"
            );
        }

        // (2) — l'échec : `parse_check_suite_failure`, qui lit la conclusion
        // en plus du couple repo/branche.
        let (ancien, nouveau) = mika2917_ancien_puis_nouveau(
            "[GitHub] Check suite failure on senara-solutions/mika (branch: feat/2917/x)",
        );
        for (etiquette, texte) in [("ancien", &ancien), ("nouveau", &nouveau)] {
            let event = parse_check_suite_failure(texte).unwrap_or_else(|| {
                panic!("`parse_check_suite_failure` ne lit plus le {etiquette}")
            });
            assert_eq!(event.repo, "senara-solutions/mika", "{etiquette}");
            assert_eq!(event.branch, "feat/2917/x", "{etiquette}");
            assert_eq!(event.conclusion, "failure", "{etiquette}");
        }

        // La moitié qui rend le test non vide : une tête écrite en PREMIÈRE
        // ligne casse bien les lecteurs. Sans ce contrôle, le test ci-dessus
        // passerait aussi si les quatre lecteurs s'étaient mis à tout lire.
        let tete_en_premier = format!(
            "{}1a2b3c4d\n[GitHub] Check suite success on senara-solutions/mika (branch: feat/2917/x)",
            mika_common::github_event_format::CHECK_SUITE_HEAD_LINE_PREFIX
        );
        assert!(
            parse_check_suite_success(&tete_en_premier).is_none(),
            "si la tête en ligne 1 ne cassait rien, la contrainte de position \
             de ce vol serait sans objet — et ce test, vide"
        );
        assert!(correlate_webhook(&tete_en_premier).is_none());
        assert_eq!(classify_event(&tete_en_premier), WebhookEventKind::Other);
    }

    /// mika#2917 — `classify` transporte la tête, et distingue ses quatre
    /// formes.
    ///
    /// Aucun décideur ne la lit dans ce vol : ce test est donc tout ce qui
    /// atteste que la tête traverse. Il épingle en particulier que
    /// `LineAbsent` et `Empty` ne sont pas la même valeur — un gateway
    /// antérieur au vol et une forge qui ne nomme aucune PR sont deux
    /// populations, et une seule des deux se corrige par un déploiement.
    #[test]
    fn mika2917_classify_transporte_la_tete_sans_que_rien_ne_la_lise() {
        fn heads_de(texte: &str) -> CheckSuiteHeads {
            match classify(texte) {
                Some(Candidate::GreenPrBranch { heads, .. }) => heads,
                autre => panic!("hors population : {autre:?}"),
            }
        }

        let premiere =
            "[GitHub] Check suite success on senara-solutions/mika (branch: feat/2917/x)";
        let (ancien, nouveau) = mika2917_ancien_puis_nouveau(premiere);

        assert_eq!(
            heads_de(&ancien),
            CheckSuiteHeads::LineAbsent,
            "un texte sans ligne de tête vient d'un gateway antérieur au vol"
        );
        assert_eq!(
            heads_de(&nouveau),
            CheckSuiteHeads::One {
                pr: 2917,
                head_sha: "1a2b3c4d".to_string(),
            }
        );
        assert_ne!(heads_de(&ancien), CheckSuiteHeads::Empty);

        // `none` : la forge ne nomme aucune PR — la forme mesurée sur
        // mika#2681, et celle qu'un gateway à jour écrit sur un payload sans
        // `pull_requests[]`.
        let vide = format!(
            "{premiere}\n{}1a2b3c4d\n{}{}",
            mika_common::github_event_format::CHECK_SUITE_HEAD_LINE_PREFIX,
            mika_common::github_event_format::CHECK_SUITE_PRS_LINE_PREFIX,
            mika_common::github_event_format::CHECK_SUITE_PRS_NONE,
        );
        assert_eq!(heads_de(&vide), CheckSuiteHeads::Empty);

        // Plusieurs PR : la suite est partagée, elle ne nomme la tête
        // d'aucune.
        let plusieurs = format!(
            "{premiere}\n{}1a2b3c4d\n{}2917@1a2b3c4d 2918@9f8e7d6c",
            mika_common::github_event_format::CHECK_SUITE_HEAD_LINE_PREFIX,
            mika_common::github_event_format::CHECK_SUITE_PRS_LINE_PREFIX,
        );
        assert_eq!(heads_de(&plusieurs), CheckSuiteHeads::Many(2));
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
        /// (h) mika#2854. `None` ici est ce qui prouve AC7 : un ticket avec
        /// tâche de boucle ne doit pas atteindre le triage, et le `panic!`
        /// d'[`answer`] est l'assertion.
        pub(crate) issue_task: Option<Result<bool, &'static str>>,
        pub(crate) triage: Option<Result<TriageVerdict, &'static str>>,
        /// (h) moitié fermeture, mika#2854 AC4. Même convention que les autres
        /// fentes : `None` veut dire « l'instantané ne doit pas être consulté »
        /// et panique s'il l'est — c'est ce qui prouve que la moitié
        /// commentaire ne passe pas par ce chemin.
        pub(crate) closure_snapshot: Option<ClosureSnapshot>,
        /// (g′) mika#2917, vol 3. Le registre **indexé par clé de tête**, et non
        /// une réponse unique :
        ///
        /// - `None` — « le registre ne doit pas être consulté », et le `panic!`
        ///   l'atteste. C'est ce qui prouve qu'un texte sans tête nommable, et
        ///   la classe désarmée, n'ouvrent pas la lecture.
        /// - `Some(Ok(lignes))` — le registre porte ces lignes, et la lecture
        ///   répond sur la CLÉ demandée. Un vecteur **vide** est un registre lu
        ///   et muet, ce qui n'est pas `None` (mika#2331).
        /// - `Some(Err(_))` — la base n'a pas répondu.
        ///
        /// Indexé plutôt qu'unique parce que le contrôle négatif d'AC1 — « la PR
        /// porte une ligne sur son ANCIENNE tête, le texte en nomme une
        /// neuve » — n'est sinon pas représentable : un bouchon qui rend la même
        /// valeur à toute clé rendrait ce contrôle conventionnel au lieu de
        /// structurel. Les valeurs sont **brutes** : le bouchon ne valide rien,
        /// exactement comme la base.
        pub(crate) reviewed_head: Option<Result<Vec<(String, String)>, &'static str>>,
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
        async fn issue_has_active_task(&self, _: &str) -> anyhow::Result<bool> {
            answer(&self.issue_task, "issue_task")
        }
        async fn triage(&self, _: &str) -> anyhow::Result<TriageVerdict> {
            match &self.triage {
                Some(Ok(v)) => Ok(v.clone()),
                Some(Err(e)) => Err(anyhow::anyhow!(*e)),
                None => panic!("recherche d'état non attendue : triage"),
            }
        }
        fn closure_task_snapshot(&self) -> &ClosureSnapshot {
            self.closure_snapshot
                .as_ref()
                .unwrap_or_else(|| panic!("recherche d'état non attendue : closure_snapshot"))
        }
        /// La seule recherche du bouchon qui LIT son argument : (g′) demande une
        /// clé, et le registre répond sur cette clé ou sur rien.
        async fn reviewed_head(&self, head_key: &str) -> anyhow::Result<Option<String>> {
            match &self.reviewed_head {
                Some(Ok(lignes)) => Ok(lignes
                    .iter()
                    .find(|(cle, _)| cle == head_key)
                    .map(|(_, issue)| issue.clone())),
                Some(Err(e)) => Err(anyhow::anyhow!(*e)),
                None => panic!("recherche d'état non attendue : reviewed_head"),
            }
        }
    }

    /// Un registre bouchonné, lu sur la clé : `registre(&[(HEAD_A, "review")])`.
    fn registre(lignes: &[(&str, &str)]) -> Option<Result<Vec<(String, String)>, &'static str>> {
        Some(Ok(lignes
            .iter()
            .map(|(cle, issue)| ((*cle).to_string(), (*issue).to_string()))
            .collect()))
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
        assert_eq!(
            decide(c(), &branch(Ok(true))).await,
            Decision::Llm(Release::BranchTaskActive)
        );
        // AC3 : base illisible ⇒ le tour a lieu.
        assert_eq!(
            decide(c(), &branch(Err("db down"))).await,
            Decision::Llm(Release::StateUnreadable)
        );
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
        assert_eq!(
            PrefilterClass::CommentTriagedNote.as_str(),
            "comment_triaged_note"
        );
        // (g′) mika#2917, vol 3. La classe ET ses trois termes de relâche :
        // `measure-webhook-turn-cost` groupe dynamiquement sur `.class` et sur
        // `.release`, donc ces quatre chaînes sont les clés de la mesure d'AC4.
        // Les renommer sans renommer la requête casse la mesure en silence —
        // c'est le seul endroit du vol où un littéral est une interface.
        assert_eq!(
            PrefilterClass::GreenCheckSuiteHeadAlreadyReviewed.as_str(),
            "green_check_suite_head_already_reviewed"
        );
        assert_eq!(Release::ReviewedHeadAbsent.as_str(), "reviewed_head_absent");
        assert_eq!(
            Release::ReviewedHeadOutOfContract.as_str(),
            "reviewed_head_out_of_contract"
        );
        assert_eq!(
            Release::ReviewedHeadSwitchDisarmed.as_str(),
            "reviewed_head_switch_disarmed"
        );
        // Le nom de l'interrupteur est lui aussi une interface : il vit dans la
        // doc opérateur et dans le geste de retraite.
        assert_eq!(REVIEWED_HEAD_ENV, "MIKA_WEBHOOK_PREFILTER_REVIEWED_HEAD");
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
        // Les deux relâches de cette porte sont nommées SÉPARÉMENT : une PR
        // ouverte et une forge illisible rendent toutes deux le tour, mais la
        // première est un verdict à traiter et la seconde une panne de lecture.
        // Les confondre rendrait un `gh` cassé indistinguable d'un flux de
        // revues normal, sur la colonne même qui sert à décider s'il reste du
        // coût à filtrer (mika#2680).
        for (forge, release) in [
            (Ok(PrForgeState::Open), Release::PrOpen),
            (Err("gh 401"), Release::StateUnreadable),
        ] {
            for c in [actionable(), block()] {
                let state = Stub {
                    pr_task: Some(Ok(false)),
                    pr_state: Some(forge),
                    ..Stub::default()
                };
                assert_eq!(decide(c, &state).await, Decision::Llm(release), "{forge:?}");
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
            assert_eq!(
                decide(c, &pr(Ok(true))).await,
                Decision::Llm(Release::PrTaskActive),
                "{body}"
            );
        }
        // AC3 : base illisible.
        let c = classify(&review("commented", QA, "VERDICT: hold[review]")).unwrap();
        assert_eq!(
            decide(c, &pr(Err("db down"))).await,
            Decision::Llm(Release::StateUnreadable)
        );
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

    /// Le même `check_suite` vert, porteur de la ligne que le gateway émet
    /// depuis le vol 1 : UNE PR et sa tête (mika#2917).
    ///
    /// `head_key(REPO, 2678, <le SHA ci-dessous>)` vaut exactement [`HEAD_A`] —
    /// c'est ce qui permet au même fixture de servir (b) et (g′), et donc de
    /// mesurer leur ordre de consultation sur un seul texte.
    ///
    /// [`GREEN_PR`], lui, n'a **pas** cette ligne : il vaut
    /// `CheckSuiteHeads::LineAbsent`, et c'est la raison pour laquelle toute la
    /// section (b) reste verte sans une retouche. La non-régression est une
    /// propriété des fixtures, pas une promesse de prose.
    pub(crate) const GREEN_PR_ONE_HEAD: &str = "[GitHub] Check suite success on senara-solutions/mika (branch: fix/2675/x)\n\
         Check-Suite-PRs: 2678@1a2edc8091fee6eec4c736b5dd974221802287b6";

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
            Decision::Llm(Release::NoDecisionAttested)
        );
    }

    /// AC3 : audit illisible, à l'une ou l'autre lecture ⇒ le tour a lieu.
    #[tokio::test]
    async fn mika2675_b_audit_illisible_le_tour_a_lieu() {
        let (this_event, _) = decided_case();
        let c = || classify(GREEN_PR).unwrap();
        assert_eq!(
            decide(c(), &rows(Err("db down"), None)).await,
            Decision::Llm(Release::StateUnreadable)
        );
        assert_eq!(
            decide(c(), &rows(Ok(this_event), Some(Err("db down")))).await,
            Decision::Llm(Release::StateUnreadable)
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

    // ---- vol 3 — la classe (g′) (mika#2917) ------------------------------

    /// Le SHA que [`GREEN_PR_ONE_HEAD`] nomme ; [`HEAD_A`] en est la clé.
    const SHA_A: &str = "1a2edc8091fee6eec4c736b5dd974221802287b6";
    /// La tête d'après un nouveau push sur la même PR ; [`HEAD_B`] en est la clé.
    const SHA_B: &str = "ffffffffffffffffffffffffffffffffffffffff";

    /// Le même vert sur la même PR, après un nouveau push : la ligne du vol 1
    /// nomme [`SHA_B`]. C'est le texte du contrôle négatif d'AC1.
    const GREEN_PR_NEW_HEAD: &str = "[GitHub] Check suite success on senara-solutions/mika (branch: fix/2675/x)\n\
         Check-Suite-PRs: 2678@ffffffffffffffffffffffffffffffffffffffff";

    /// La ligne est là et ne nomme **aucune** tête : `CheckSuiteHeads::Empty`,
    /// l'état mesuré sur mika#2681.
    const GREEN_PR_NO_HEAD: &str = "[GitHub] Check suite success on senara-solutions/mika (branch: fix/2675/x)\n\
         Check-Suite-PRs: none";

    /// Deux PR : le payload ne nomme pas LA tête de l'événement.
    const GREEN_PR_MANY_HEADS: &str = "[GitHub] Check suite success on senara-solutions/mika (branch: fix/2675/x)\n\
         Check-Suite-PRs: 2678@1a2edc8091fee6eec4c736b5dd974221802287b6 2679@ffffffffffffffffffffffffffffffffffffffff";

    /// La cible attendue au registre pour une clé de tête — le pendant de test
    /// de [`check_suite_target`], écrit à part pour que le test ne soit pas
    /// l'écho de la fonction qu'il juge.
    fn cible(head: &str) -> String {
        format!("check_suite:{}", head.strip_prefix("pr:").unwrap())
    }

    /// La charnière du vol : la clé que le décideur construit à partir du TEXTE
    /// est exactement celle sur laquelle `record_reviewed_head` écrit.
    ///
    /// Les deux moitiés passent par `head_key` ; ce test dit qu'elles passent
    /// par la MÊME, et c'est lui qui rend [`HEAD_A`] lisible comme une clé
    /// plutôt que comme une chaîne de caractères heureuse. Les quatre fixtures
    /// de (g′) en dépendent — sans lui, l'un d'eux pourrait porter un SHA qui ne
    /// correspond à rien et tout le corpus resterait vert en ne mesurant rien.
    #[test]
    fn mika2917_vol3_la_cle_du_texte_est_celle_du_registre() {
        assert_eq!(head_key(REPO, 2678, SHA_A), HEAD_A);
        assert_eq!(head_key(REPO, 2678, SHA_B), HEAD_B);
        assert!(GREEN_PR_ONE_HEAD.contains(SHA_A));
        assert!(GREEN_PR_NEW_HEAD.contains(SHA_B));
        assert_eq!(
            classify(GREEN_PR_ONE_HEAD).map(|c| c.as_str()),
            Some("green_pr_branch")
        );
    }

    /// Ce que chaque fixture dit de ses têtes. Les quatre valeurs de
    /// [`CheckSuiteHeads`], une par texte : c'est la prémisse de tous les tests
    /// qui suivent, et elle est attestée au lieu d'être supposée.
    #[test]
    fn mika2917_vol3_les_quatre_fixtures_couvrent_les_quatre_valeurs() {
        let heads = |texte| match classify(texte) {
            Some(Candidate::GreenPrBranch { heads, .. }) => heads,
            autre => panic!("pas une (b) : {autre:?}"),
        };
        assert_eq!(heads(GREEN_PR), CheckSuiteHeads::LineAbsent);
        assert_eq!(heads(GREEN_PR_NO_HEAD), CheckSuiteHeads::Empty);
        assert_eq!(
            heads(GREEN_PR_ONE_HEAD),
            CheckSuiteHeads::One {
                pr: 2678,
                head_sha: SHA_A.into()
            }
        );
        assert_eq!(heads(GREEN_PR_MANY_HEADS), CheckSuiteHeads::Many(2));
    }

    /// AC1 — le corpus rejoué, en **un seul** test comme le ticket l'exige.
    ///
    /// Les 51 tours de la mesure du 2026-10-26 ne sont pas rejouables tels
    /// quels (leurs payloads n'ont pas été capturés), donc chaque ligne est
    /// construite depuis une issue **attestée** de cette mesure.
    ///
    /// La quatrième est le contrôle négatif, et il est **structurel** : le
    /// registre porte une ligne sur l'ANCIENNE tête de la même PR, le texte en
    /// nomme une neuve, et le bouchon répond sur la clé demandée. Rien n'y
    /// repose sur une convention de nommage — seulement sur la clé.
    ///
    /// « Aucun tour terminé en `review` n'est écarté » tient par la même
    /// structure : un tour qui se *terminerait* en `review` porte une tête que
    /// le registre ne porte pas encore, et c'est exactement cette quatrième
    /// ligne. Une ligne `review` AU registre, elle, dit « un tour a déjà posté
    /// la revue de cette tête », dont le successeur se terminerait en
    /// `stop_already_reviewed_sha` : elle doit donc écarter, et c'est la
    /// première ligne.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2917_vol3_le_corpus_rejoue() {
        let _garde = ReviewedHeadEnvGuard::armed(true);

        enum Attendu {
            Ecarte,
            Passe(Release),
        }

        struct Cas {
            nom: &'static str,
            texte: &'static str,
            /// Le registre TEL QU'IL EST, indexé par clé — et non « l'issue de
            /// la tête du texte », une forme qui rendrait le contrôle négatif
            /// vacant.
            registre: &'static [(&'static str, &'static str)],
            attendu: Attendu,
        }

        for cas in [
            Cas {
                nom: "un tour antérieur a posté la revue de cette tête",
                texte: GREEN_PR_ONE_HEAD,
                registre: &[(HEAD_A, "review")],
                attendu: Attendu::Ecarte,
            },
            Cas {
                nom: "un tour antérieur a conclu au verdict déjà enregistré",
                texte: GREEN_PR_ONE_HEAD,
                registre: &[(HEAD_A, "stop_verdict_recorded")],
                attendu: Attendu::Ecarte,
            },
            Cas {
                nom: "un tour antérieur a conclu à la tête déjà revue",
                texte: GREEN_PR_ONE_HEAD,
                registre: &[(HEAD_A, "stop_already_reviewed_sha")],
                attendu: Attendu::Ecarte,
            },
            Cas {
                nom: "contrôle négatif : tête NEUVE, la PR n'a de ligne que sur l'ancienne",
                texte: GREEN_PR_NEW_HEAD,
                registre: &[(HEAD_A, "review")],
                attendu: Attendu::Passe(Release::ReviewedHeadAbsent),
            },
        ] {
            let state = Stub {
                reviewed_head: registre(cas.registre),
                // (b) est consultée dès que (g′) n'écarte pas, et elle ne mord
                // sur aucune ligne du corpus : aucune attestation
                // DECISION-CORE sur cet événement.
                ..rows(Ok(vec![]), None)
            };
            let attendu = match cas.attendu {
                Attendu::Ecarte => Decision::Skip {
                    class: PrefilterClass::GreenCheckSuiteHeadAlreadyReviewed,
                    target: cible(HEAD_A),
                },
                Attendu::Passe(terme) => Decision::Llm(terme),
            };
            assert_eq!(
                decide(classify(cas.texte).unwrap(), &state).await,
                attendu,
                "{}",
                cas.nom
            );
        }
    }

    /// L'ordre de consultation, prouvé plutôt que supposé : quand (g′) écarte,
    /// (b) n'est **pas** consultée — `event_rows: None` panique à toute lecture.
    ///
    /// C'est ce qui rend lisible le transfert de population du § R3 : un
    /// événement que les deux portes sauraient écarter s'inscrit sous (g′), donc
    /// `green_check_suite_head_decided` baisse au déploiement, et la grandeur
    /// conservée est la somme des deux classes (mika#2205).
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2917_vol3_quand_g_prime_ecarte_b_nest_pas_consultee() {
        let _garde = ReviewedHeadEnvGuard::armed(true);
        let state = Stub {
            reviewed_head: registre(&[(HEAD_A, "review")]),
            ..Stub::default()
        };
        assert_eq!(
            decide(classify(GREEN_PR_ONE_HEAD).unwrap(), &state).await,
            Decision::Skip {
                class: PrefilterClass::GreenCheckSuiteHeadAlreadyReviewed,
                target: cible(HEAD_A),
            }
        );
    }

    /// AC2 (1/5) — la ligne du vol 1 est **absente** : binaire de gateway
    /// antérieur à mika#2917. (b) décide entière, relâches comprises, et le
    /// registre n'est pas consulté (`reviewed_head: None` paniquerait).
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2917_vol3_ac2_ligne_absente() {
        let _garde = ReviewedHeadEnvGuard::armed(true);
        let (this_event, prior) = decided_case();
        assert_eq!(
            decide(
                classify(GREEN_PR).unwrap(),
                &rows(Ok(this_event), Some(Ok(prior)))
            )
            .await,
            Decision::Skip {
                class: PrefilterClass::GreenCheckSuiteHeadDecided,
                target: cible(HEAD_A),
            }
        );
        assert_eq!(
            decide(classify(GREEN_PR).unwrap(), &rows(Ok(vec![]), None)).await,
            Decision::Llm(Release::NoDecisionAttested)
        );
    }

    /// AC2 (2/5) — la ligne est là et ne nomme aucune tête : le `pull_requests`
    /// vide que le ticket cite, mesuré sur mika#2681.
    ///
    /// Même bras du décideur que [`mika2917_vol3_ac2_ligne_absente`], et **deux
    /// tests** malgré ça : la grammaire du vol 1 sépare ces deux populations, et
    /// les sommer sous un seul cas rendrait le jour où elles divergent
    /// indétectable (mika#2205, mika#2331).
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2917_vol3_ac2_aucune_tete() {
        let _garde = ReviewedHeadEnvGuard::armed(true);
        let (this_event, prior) = decided_case();
        assert_eq!(
            decide(
                classify(GREEN_PR_NO_HEAD).unwrap(),
                &rows(Ok(this_event), Some(Ok(prior)))
            )
            .await,
            Decision::Skip {
                class: PrefilterClass::GreenCheckSuiteHeadDecided,
                target: cible(HEAD_A),
            }
        );
        assert_eq!(
            decide(classify(GREEN_PR_NO_HEAD).unwrap(), &rows(Ok(vec![]), None)).await,
            Decision::Llm(Release::NoDecisionAttested)
        );
    }

    /// AC2 (3/5) — plusieurs PR : le payload ne nomme pas LA tête de
    /// l'événement, donc (g′) n'a rien à comparer et (b) décide entière.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2917_vol3_ac2_plusieurs_pr() {
        let _garde = ReviewedHeadEnvGuard::armed(true);
        let (this_event, prior) = decided_case();
        assert_eq!(
            decide(
                classify(GREEN_PR_MANY_HEADS).unwrap(),
                &rows(Ok(this_event), Some(Ok(prior)))
            )
            .await,
            Decision::Skip {
                class: PrefilterClass::GreenCheckSuiteHeadDecided,
                target: cible(HEAD_A),
            }
        );
        assert_eq!(
            decide(
                classify(GREEN_PR_MANY_HEADS).unwrap(),
                &rows(Ok(vec![]), None)
            )
            .await,
            Decision::Llm(Release::NoDecisionAttested)
        );
    }

    /// AC2 (4/5) — le registre est lu et ne porte pas la tête : le tour passe,
    /// sous `reviewed_head_absent`, et l'écart de (b) survit quand elle mord.
    ///
    /// La valeur hors contrat suit le même chemin sous son propre terme : la
    /// base a répondu, c'est l'écrivain qui a écrit hors de
    /// `REVIEWED_HEAD_OUTCOMES`, et les deux causes appellent deux gestes de
    /// réparation (mika#2205).
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2917_vol3_ac2_registre_muet() {
        let _garde = ReviewedHeadEnvGuard::armed(true);
        for (nom, lignes, terme) in [
            ("registre muet", registre(&[]), Release::ReviewedHeadAbsent),
            (
                "ligne sur une AUTRE tête de la même PR",
                registre(&[(HEAD_B, "review")]),
                Release::ReviewedHeadAbsent,
            ),
            (
                "valeur hors contrat",
                registre(&[(HEAD_A, "review_failed")]),
                Release::ReviewedHeadOutOfContract,
            ),
        ] {
            let state = Stub {
                reviewed_head: lignes,
                ..rows(Ok(vec![]), None)
            };
            assert_eq!(
                decide(classify(GREEN_PR_ONE_HEAD).unwrap(), &state).await,
                Decision::Llm(terme),
                "{nom}"
            );
        }
    }

    /// AC2 (5/5) — le registre est illisible : le tour passe, sous
    /// `state_unreadable`, et le WARN `webhook_prefilter_state_unreadable` porte
    /// `what = "reviewed_head"`.
    ///
    /// `Release::StateUnreadable` est **réutilisé** et non dédoublé : c'est
    /// délibérément un terme pour tous les sites de lecture en erreur, et (g′)
    /// en est le sixième. L'écart de (b) survit à cette panne — c'est le
    /// fail-safe du vol, et la seconde assertion est ce qui l'atteste.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2917_vol3_ac2_registre_illisible() {
        let _garde = ReviewedHeadEnvGuard::armed(true);
        let state = |event, holds| Stub {
            reviewed_head: Some(Err("db down")),
            ..rows(event, holds)
        };
        assert_eq!(
            decide(
                classify(GREEN_PR_ONE_HEAD).unwrap(),
                &state(Ok(vec![]), None)
            )
            .await,
            Decision::Llm(Release::StateUnreadable)
        );
        let (this_event, prior) = decided_case();
        assert_eq!(
            decide(
                classify(GREEN_PR_ONE_HEAD).unwrap(),
                &state(Ok(this_event), Some(Ok(prior)))
            )
            .await,
            Decision::Skip {
                class: PrefilterClass::GreenCheckSuiteHeadDecided,
                target: cible(HEAD_A),
            }
        );
    }

    /// R8/V6 — **aucun écart de (b) n'est perdu**, sur aucune entrée.
    ///
    /// Le cas mesuré de (b) (mika#2672, #2674) servi sur le texte qui porte UNE
    /// tête, donc celui où (g′) est consultée : dans les quatre situations où
    /// elle n'écarte pas — registre muet, valeur hors contrat, lecture en panne,
    /// interrupteur désarmé — (b) écarte toujours, sous sa propre classe.
    ///
    /// Un test dédié et non un corollaire : c'est la propriété que le vol
    /// promet, et la seule qui rende l'interrupteur sans danger.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2917_vol3_aucun_ecart_de_b_nest_perdu() {
        let ecart_de_b = Decision::Skip {
            class: PrefilterClass::GreenCheckSuiteHeadDecided,
            target: cible(HEAD_A),
        };
        for (nom, arme, lignes) in [
            ("registre muet", true, registre(&[])),
            (
                "valeur hors contrat",
                true,
                registre(&[(HEAD_A, "review_failed")]),
            ),
            ("lecture en panne", true, Some(Err("db down"))),
            ("interrupteur désarmé", false, None),
        ] {
            let _garde = ReviewedHeadEnvGuard::armed(arme);
            let (this_event, prior) = decided_case();
            let state = Stub {
                reviewed_head: lignes,
                ..rows(Ok(this_event), Some(Ok(prior)))
            };
            assert_eq!(
                decide(classify(GREEN_PR_ONE_HEAD).unwrap(), &state).await,
                ecart_de_b,
                "{nom}"
            );
        }
    }

    /// V7 — désarmé, (g′) rend **exactement** le régime de `main` avant ce vol.
    ///
    /// Trois assertions, et chacune dit une chose que les deux autres ne disent
    /// pas : la classe que (b) rendrait, la relâche sous le nom de (g′) — parce
    /// que « l'interrupteur est baissé » et « la classe ne mord sur rien »
    /// doivent rendre des octets différents (mika#2205) — et le fait que **rien
    /// n'est lu**, que `reviewed_head: None` atteste par son `panic!`.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2917_vol3_linterrupteur_desarme_rend_le_regime_anterieur() {
        let _garde = ReviewedHeadEnvGuard::armed(false);
        let (this_event, prior) = decided_case();
        assert_eq!(
            decide(
                classify(GREEN_PR_ONE_HEAD).unwrap(),
                &rows(Ok(this_event), Some(Ok(prior)))
            )
            .await,
            Decision::Skip {
                class: PrefilterClass::GreenCheckSuiteHeadDecided,
                target: cible(HEAD_A),
            }
        );
        assert_eq!(
            decide(
                classify(GREEN_PR_ONE_HEAD).unwrap(),
                &rows(Ok(vec![]), None)
            )
            .await,
            Decision::Llm(Release::ReviewedHeadSwitchDisarmed)
        );
    }

    /// La table de vérité de l'interrupteur de (g′), et la ligne qui la rend
    /// différente de celle de (h) : **l'absence vaut armé**.
    ///
    /// C'est le régime de production, donc la ligne qu'un test qui pose toujours
    /// la variable ne verrait jamais. La valeur non reconnue laisse armé, avec
    /// son WARN `qa_switch_invalid` : taper le nom de la variable *est*
    /// l'adhésion, en garbler la valeur est une faute de frappe.
    #[test]
    #[serial_test::serial]
    fn mika2917_vol3_labsence_de_variable_laisse_g_prime_armee() {
        let _absente = ReviewedHeadEnvGuard::set(None);
        assert!(reviewed_head_enabled());
        let _off = ReviewedHeadEnvGuard::armed(false);
        assert!(!reviewed_head_enabled());
        let _garbled = ReviewedHeadEnvGuard::set(Some("peut-être"));
        assert!(reviewed_head_enabled());
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
        assert_eq!(
            decide(c(), &branch(Ok(true))).await,
            Decision::Llm(Release::BranchTaskActive)
        );
        // AC3 : base illisible ⇒ le tour a lieu.
        assert_eq!(
            decide(c(), &branch(Err("db down"))).await,
            Decision::Llm(Release::StateUnreadable)
        );
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
        assert_eq!(
            decide(c(), &rows(Ok(vec![]), None)).await,
            Decision::Llm(Release::NoTerminalTrace)
        );
        assert_eq!(
            decide(c(), &rows(Err("db down"), None)).await,
            Decision::Llm(Release::StateUnreadable)
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
            (COMMENT_2854, "human_text_no_task"),
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
        // de ces tours a pourtant un `trigger`, et c'est tout l'intérêt. Le
        // `PR closed` sans URL en fait partie depuis mika#2680 : la classe (e)
        // ne sait pas de quelle PR il parle, donc elle n'en est pas candidate.
        //
        // Les deux premiers sont la MÊME raison pour (h) depuis mika#2854 :
        // sans ligne d'URL, aucune clé à donner à
        // `has_active_self_dev_task_for_issue`, donc aucun moyen de savoir si
        // le ticket porte une tâche de boucle — et AC7 interdit de deviner. Ce
        // sont les mêmes gabarits que `COMMENT_2854` ci-dessus, à la ligne
        // d'URL près : la paire est le contrôle négatif de la classe. La
        // fermeture sans URL y reste depuis mika#2854 AC4 — la moitié
        // fermeture est livrée, mais le refus d'une clé absente ne dépend pas
        // de la moitié (§ D15 du plan).
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

        // (h) mika#2854 : les deux moitiés, et l'ÉGALITÉ avec l'axe du
        // déclencheur. Les deux axes nomment la même moitié du même nom, donc
        // un `GROUP BY` croisé reste lisible ; si l'un des deux dérive, la
        // mesure scinde une population sans le dire.
        assert_eq!(TriageHalf::Comment.as_str(), "issue_comment");
        assert_eq!(TriageHalf::IssueClosed.as_str(), "issue_closed");
        assert_eq!(
            TriageHalf::IssueClosed.as_str(),
            TriggerClass::IssueClosed.as_str()
        );
        assert_eq!(
            TriageHalf::Comment.as_str(),
            TriggerClass::IssueComment.as_str()
        );
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
            Decision::Llm(Release::PrTaskActive)
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
            Decision::Llm(Release::StateUnreadable)
        );
    }

    // ---- vol 4 — le terme de relâche (mika#2680) -------------------------

    /// Le cas qui atteint chaque terme de relâche, et le `match` qui tient
    /// l'exhaustivité.
    ///
    /// Chaque bouchon n'ouvre **que** la porte que son terme traverse : les
    /// trois premiers reçoivent `Stub::default()`, qui panique à toute
    /// recherche d'état. C'est ainsi que « rendu AVANT `decide` » est prouvé et
    /// non supposé — un garde déplacé après la décision ferait paniquer le cas
    /// au lieu de le rendre vert autrement.
    ///
    /// Le quatrième terme du tuple est l'interrupteur **de (h)** (mika#2854) :
    /// trois de ses quatre termes n'existent que la classe armée, et le
    /// quatrième n'existe qu'elle désarmée. C'est le seul endroit du module qui
    /// touche à une variable d'environnement, d'où le `#[serial]` sur le
    /// recensement.
    fn cas_de_relache(terme: Release) -> (String, Stub, Touched, bool, bool) {
        let intact = Touched {
            by_any_handler: false,
            after_verdict_handler: false,
            outside_ci_success_handler: false,
            outside_ci_failure_handler: false,
        };
        let touche = Touched {
            by_any_handler: true,
            ..intact
        };
        // Pas de bras `_` : ajouter un dix-neuvième terme ne compile pas tant
        // qu'il n'a pas son cas ici (DoD 8).
        match terme {
            Release::SwitchDisarmed => (GREEN_MAIN.into(), Stub::default(), intact, false, false),
            Release::NotClassified => ("bonjour".into(), Stub::default(), intact, true, false),
            Release::TextTouched => (GREEN_MAIN.into(), Stub::default(), touche, true, false),
            Release::BranchTaskActive => (GREEN_MAIN.into(), branch(Ok(true)), intact, true, false),
            Release::PrTaskActive => (
                review("commented", QA, "VERDICT: hold[review]"),
                pr(Ok(true)),
                intact,
                true,
                false,
            ),
            Release::PrOpen => (
                review("approved", QA, "VERDICT: pass"),
                Stub {
                    pr_task: Some(Ok(false)),
                    pr_state: Some(Ok(PrForgeState::Open)),
                    ..Stub::default()
                },
                intact,
                true,
                false,
            ),
            Release::NoTerminalTrace => {
                (RED_PR.into(), rows(Ok(vec![]), None), intact, true, false)
            }
            // Aucune ligne `decision_core_hold` sur CET événement : la seconde
            // lecture n'a pas lieu, et `holds: None` le prouve en paniquant.
            Release::NoDecisionAttested => (
                GREEN_PR.into(),
                rows(Ok(vec![processed(HEAD_A, "rid-now")]), None),
                intact,
                true,
                false,
            ),
            // Une attestation existe, mais sur une AUTRE tête de la même PR.
            Release::HeadNotPreviouslyDecided => (
                GREEN_PR.into(),
                rows(
                    Ok(vec![
                        processed(HEAD_A, "rid-now"),
                        hold(HEAD_A, "notified", "rid-now"),
                    ]),
                    Some(Ok(vec![hold(HEAD_B, "notified", "rid-before")])),
                ),
                intact,
                true,
                false,
            ),
            Release::StateUnreadable => (
                GREEN_MAIN.into(),
                branch(Err("db down")),
                intact,
                true,
                false,
            ),
            // (h) mika#2854 — les cinq termes du sixième décideur. Le premier
            // est le seul du module à vouloir l'interrupteur de (h) DÉSARMÉ :
            // c'est l'état livré par ce vol.
            Release::TriageDisarmed => (COMMENT_2854.into(), Stub::default(), intact, true, false),
            Release::IssueTaskActive => (
                COMMENT_2854.into(),
                triage_stub(Some(Ok(true)), None),
                intact,
                true,
                true,
            ),
            Release::TriageEscalated => (
                COMMENT_2854.into(),
                triage_stub(Some(Ok(false)), Some(Ok(TriageVerdict::Decision))),
                intact,
                true,
                true,
            ),
            Release::TriageUnreadable => (
                COMMENT_2854.into(),
                triage_stub(Some(Ok(false)), Some(Err("llm down"))),
                intact,
                true,
                true,
            ),
            // (h) AC4 — armé, une fermeture, et pourtant un instantané sans
            // sujet : le site de capture n'a pas tourné sur CE texte. Le
            // bouchon laisse `triage: None`, donc le test prouve aussi que le
            // modèle n'est pas consulté.
            Release::ClosureSnapshotAbsent => (
                ISSUE_CLOSED_2854.into(),
                closure_stub(ClosureSnapshot::NotAClosure, None),
                intact,
                true,
                true,
            ),
            // (g′) mika#2917, vol 3 — les trois termes du septième décideur. Le
            // texte porte UNE tête, donc (g′) est consultée ; dans les trois cas
            // elle n'écarte pas, (b) est consultée entière et ne mord pas non
            // plus (`event_rows: Ok(vec![])`, aucune ligne DECISION-CORE), et
            // c'est le terme nommé par (g′) qui s'inscrit — jamais le
            // `NoDecisionAttested` de (b).
            Release::ReviewedHeadAbsent => (
                GREEN_PR_ONE_HEAD.into(),
                Stub {
                    reviewed_head: registre(&[]),
                    ..rows(Ok(vec![]), None)
                },
                intact,
                true,
                false,
            ),
            // `review_failed` est précisément l'une des issues que l'outil
            // n'écrit PAS : la trouver au registre nomme une divergence entre
            // l'écrivain et `REVIEWED_HEAD_OUTCOMES`.
            Release::ReviewedHeadOutOfContract => (
                GREEN_PR_ONE_HEAD.into(),
                Stub {
                    reviewed_head: registre(&[(HEAD_A, "review_failed")]),
                    ..rows(Ok(vec![]), None)
                },
                intact,
                true,
                false,
            ),
            // Désarmée, (g′) ne lit RIEN : `reviewed_head: None` panique à
            // toute lecture, donc ce cas prouve l'abstention au lieu de la
            // supposer.
            Release::ReviewedHeadSwitchDisarmed => (
                GREEN_PR_ONE_HEAD.into(),
                rows(Ok(vec![]), None),
                intact,
                true,
                false,
            ),
        }
    }

    /// Terme par terme (DoD 8) : chacun des dix-huit termes de relâche est
    /// atteint par la PORTE elle-même — `skip_turn`, pas `decide` — parce que
    /// trois d'entre eux sont rendus **avant** `decide` et qu'un test sur
    /// `decide` seul ne les verrait jamais.
    ///
    /// L'exhaustivité est tenue par le `match` de [`cas_de_relache`], qui n'a
    /// pas de bras par défaut : un dix-neuvième terme ne COMPILE pas tant qu'il
    /// n'a pas son cas. Le voisin
    /// [`mika2680_la_taxonomie_atteint_chacun_de_ses_douze_termes`] compte ses
    /// termes après coup, et ce comptage a une faiblesse que celui-ci n'a pas :
    /// il ne sait rien de l'énumération, donc une variante ajoutée sans cas le
    /// laisse vert.
    ///
    /// La base est réelle et vide : les dix-huit termes rendent tous **avant**
    /// la ligne d'audit, donc rien n'y est écrit — `skip_turn` n'en demande une
    /// que pour l'`agent_id` de sa ligne `info!`.
    ///
    /// `#[serial]` depuis mika#2854 : les cinq termes de (h) portent chacun
    /// une position de `MIKA_WEBHOOK_COMMENT_TRIAGE`, et cette variable est un
    /// état global du processus. Depuis mika#2917 (vol 3) le garde de (g′) s'y
    /// ajoute, posé **en fonction du terme** plutôt qu'en sixième membre du
    /// tuple : un seul des dix-huit le veut désarmé, et un troisième booléen
    /// rendrait `(…, true, false, true)` illisible aux dix-sept autres.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2680_la_relache_atteint_chacun_de_ses_dix_huit_termes() {
        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());

        for terme in [
            Release::SwitchDisarmed,
            Release::NotClassified,
            Release::TextTouched,
            Release::BranchTaskActive,
            Release::PrTaskActive,
            Release::PrOpen,
            Release::NoTerminalTrace,
            Release::NoDecisionAttested,
            Release::HeadNotPreviouslyDecided,
            Release::StateUnreadable,
            Release::TriageDisarmed,
            Release::IssueTaskActive,
            Release::TriageEscalated,
            Release::TriageUnreadable,
            Release::ClosureSnapshotAbsent,
            Release::ReviewedHeadAbsent,
            Release::ReviewedHeadOutOfContract,
            Release::ReviewedHeadSwitchDisarmed,
        ] {
            let (texte, state, touched, enabled, triage) = cas_de_relache(terme);
            let _garde = comment_triage::TriageEnvGuard::armed(triage);
            let _garde_g =
                ReviewedHeadEnvGuard::armed(terme != Release::ReviewedHeadSwitchDisarmed);
            assert_eq!(
                skip_turn(&db, &state, &texte, touched, "rid-now", enabled).await,
                Gate::Charged(terme),
                "{terme:?}"
            );
        }
    }

    /// (f) est **déjà écartée** par (c) pour les deux formes que le corps de
    /// mika#2680 cite, et ce test le figent en assertions plutôt qu'en prose :
    /// la prémisse *« la règle "texte intact après handler" exclut ce cas »* a
    /// été re-dérivée trois fois le 2026-10-19, au prix d'un dispatch chacune.
    ///
    /// Ce qu'elle manque : `Touched::applies_to` ne lit, pour une revue, que
    /// `after_verdict_handler` — « un handler a touché le texte DEPUIS
    /// l'instantané pris après le `verdict_handler` ». La réécriture du
    /// `verdict_handler` est donc DANS la fenêtre ; seul un handler ultérieur
    /// garde le tour.
    #[tokio::test]
    async fn mika2680_f_le_pr_review_reecrit_est_deja_ecarte() {
        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());
        let cible = "pr_review:senara-solutions/mika#2680";

        // 1. Le disjoncteur (`CIRCUIT BREAKER TRIPPED`) : le corps de revue
        //    porte un `VERDICT: block[*]` — aucun des trois termes de texte ne
        //    mord — et la PR est mergée. (c) écarte.
        let disjoncteur = review("commented", QA, "VERDICT: block[security]");
        assert_eq!(
            decide(
                classify(&disjoncteur).unwrap(),
                &Stub {
                    pr_task: Some(Ok(false)),
                    pr_state: Some(Ok(PrForgeState::Merged)),
                    ..Stub::default()
                }
            )
            .await,
            Decision::Skip {
                class: PrefilterClass::VerdictPrClosed,
                target: cible.into(),
            }
        );
        // Et la réécriture du `verdict_handler` ne rend pas ce tour : le
        // candidat ne lit que `after_verdict_handler`, qu'elle laisse à `false`.
        assert!(
            !Touched {
                by_any_handler: true,
                after_verdict_handler: false,
                outside_ci_success_handler: true,
                outside_ci_failure_handler: true,
            }
            .applies_to(&classify(&disjoncteur).unwrap())
        );

        // 2. `verdict_classification_failed` à ligne `VERDICT:` présente mais
        //    valeur non reconnue : `facts.unreadable` écarte SANS consulter la
        //    forge. Le bouchon le prouve — `pr_state: None` panique s'il est lu.
        let illisible = review("commented", QA, "VERDICT: peut-être");
        assert_eq!(
            decide(classify(&illisible).unwrap(), &pr(Ok(false))).await,
            Decision::Skip {
                class: PrefilterClass::VerdictUnreadable,
                target: cible.into(),
            }
        );

        // 3. La revue **sans aucune** ligne `VERDICT:` : hors population, donc
        //    relâche `not_classified`. C'est le seul résidu de (f) qui soit une
        //    décision de filtre, et elle épingle **AC2 de mika#2675** : une
        //    revue sans verdict est la forme d'une consigne d'opérateur, et
        //    l'identité GitHub de l'opérateur est partagée. Renverser cette
        //    assertion accepte qu'un « ouvre un ticket de suivi pour X » écrit
        //    en revue ne soit jamais lu, et la perte est silencieuse — c'est
        //    une décision opérateur, pas un refactor.
        let consigne = review("commented", QA, "Merci d'ouvrir un ticket de suivi pour X.");
        assert!(classify(&consigne).is_none());
        let intact = Touched {
            by_any_handler: false,
            after_verdict_handler: false,
            outside_ci_success_handler: false,
            outside_ci_failure_handler: false,
        };
        assert_eq!(
            skip_turn(&db, &Stub::default(), &consigne, intact, "rid-now", true).await,
            Gate::Charged(Release::NotClassified)
        );

        // 4. Contrôle négatif de la même graine (leçon mika#2277) : les mêmes
        //    textes, **seul l'état diffère**. Une tâche de boucle vivante sur
        //    cette PR garde le tour — une batterie purement positive serait
        //    satisfaite par un filtre qui écarte tout.
        for texte in [&disjoncteur, &illisible] {
            assert_eq!(
                decide(classify(texte).unwrap(), &pr(Ok(true))).await,
                Decision::Llm(Release::PrTaskActive),
                "{texte:?}"
            );
        }
        // La consigne, elle, reste hors population quel que soit l'état : son
        // terme ne dépend d'aucune porte, et c'est ce qui la distingue des deux
        // autres — le chemin A du § *Décision 2* n'est pas B.
        assert_eq!(
            skip_turn(&db, &pr(Ok(true)), &consigne, intact, "rid-now", true).await,
            Gate::Charged(Release::NotClassified)
        );
    }

    // ---- vol 1 de mika#2854 — classe (h) : le tour léger qui trie ---------
    //
    // Les textes de cette section sont les gabarits réels du gateway pour
    // `issue_comment` et `issues` `closed` : première ligne de forme, deuxième
    // ligne d'URL, corps ensuite. Le fragment `#issuecomment-<id>` est présent
    // parce que le gateway le délivre — c'est lui que C1 tronque.

    /// La moitié `issue_comment`, fragment compris, portant un ORDRE.
    const COMMENT_2854: &str = "[GitHub] New comment on senara-solutions/mika#2854 (t) by @samidarko\nhttps://github.com/senara-solutions/mika/issues/2854#issuecomment-123\n\nlève operator-gated sur #2784, je valide le vol";
    /// La moitié `issues` `closed`, même ticket, URL comprise (mika#2854 AC4).
    /// La ligne 2 porte l'URL du sujet pour les deux moitiés.
    const ISSUE_CLOSED_2854: &str = "[GitHub] Issue closed: senara-solutions/mika#2854 — t\nhttps://github.com/senara-solutions/mika/issues/2854";
    /// La clé d'état que la moitié commentaire doit rendre, fragment retiré.
    const ISSUE_URL_2854: &str = "https://github.com/senara-solutions/mika/issues/2854";

    /// Un bouchon de (h) : `None` sur une fente est ce qui PROUVE qu'une porte
    /// n'a pas été consultée, puisqu'il panique au lieu de répondre.
    fn triage_stub(
        issue_task: Option<Result<bool, &'static str>>,
        triage: Option<Result<TriageVerdict, &'static str>>,
    ) -> Stub {
        Stub {
            issue_task,
            triage,
            ..Stub::default()
        }
    }

    /// Le bouchon de la moitié FERMETURE (mika#2854 AC4) : la fente
    /// `issue_task` reste `None`, et c'est l'assertion — une lecture live sur
    /// une fermeture panique au lieu de répondre, donc le test ne peut pas être
    /// vert si le décideur se trompe de porte.
    fn closure_stub(
        snapshot: ClosureSnapshot,
        triage: Option<Result<TriageVerdict, &'static str>>,
    ) -> Stub {
        Stub {
            closure_snapshot: Some(snapshot),
            triage,
            ..Stub::default()
        }
    }

    fn intacte() -> Touched {
        Touched {
            by_any_handler: false,
            after_verdict_handler: false,
            outside_ci_success_handler: false,
            outside_ci_failure_handler: false,
        }
    }

    /// (h) terme de texte, les deux moitiés — et la troncature du fragment,
    /// lue ici à son seul endroit décisionnel.
    ///
    /// Le fragment est le piège de la classe : sans la troncature de C1, la clé
    /// servie à `has_active_self_dev_task_for_issue` serait
    /// `…/issues/2854#issuecomment-123`, qui ne correspond à aucune tâche, donc
    /// **tout** commentaire passerait pour « sans tâche de boucle » et (h)
    /// écarterait des tickets en vol. C'est l'inverse exact d'AC7.
    #[test]
    fn mika2854_h_classify_rend_la_moitie_commentaire_sans_le_fragment() {
        assert_eq!(
            classify(COMMENT_2854),
            Some(Candidate::HumanTextNoTask {
                issue_url: ISSUE_URL_2854.into(),
                half: TriageHalf::Comment,
                text: COMMENT_2854.into(),
            })
        );
    }

    /// V1 — (h) la moitié FERMETURE entre dans la population (mika#2854 AC4),
    /// et une fermeture sans URL en ligne 2 n'y entre toujours pas.
    ///
    /// La paire est le contrôle négatif de la moitié : le vol 1 retirait la
    /// fermeture de `classify` tout entière, donc « elle est classée » et « elle
    /// est classée pour la bonne raison » n'étaient pas distinguables. Ici la
    /// seule différence entre les deux textes est la ligne d'URL, qui est aussi
    /// la clé de l'instantané — sans elle, AC7 devrait deviner, et elle le
    /// refuse (§ D15).
    #[test]
    fn mika2854_h_classify_rend_la_moitie_fermeture() {
        assert_eq!(trigger_class(ISSUE_CLOSED_2854), TriggerClass::IssueClosed);
        assert_eq!(
            classify(ISSUE_CLOSED_2854),
            Some(Candidate::HumanTextNoTask {
                issue_url: ISSUE_URL_2854.into(),
                half: TriageHalf::IssueClosed,
                text: ISSUE_CLOSED_2854.into(),
            })
        );

        let sans_url = "[GitHub] Issue closed: senara-solutions/mika#2294 — t";
        assert_eq!(trigger_class(sans_url), TriggerClass::IssueClosed);
        assert!(classify(sans_url).is_none());
    }

    /// (h) ne mange pas la population de (d) — la disjonction que le commentaire
    /// d'ordre de [`classify`] annonce, en assertions.
    ///
    /// `INERT` porte une ligne d'URL d'issue parfaitement extractible : si (h)
    /// acceptait un terme de plus que ses deux, un `Issue labeled` deviendrait
    /// un candidat de triage et la classe (d) fondrait dans (h) sans qu'aucune
    /// autre assertion du module ne bouge.
    #[test]
    fn mika2854_h_ne_touche_pas_la_population_de_d() {
        assert_eq!(trigger_class(INERT), TriggerClass::IssueLabeled);
        assert!(matches!(
            classify(INERT),
            Some(Candidate::InertLabel { .. })
        ));
        // Le label lu par la boucle reste hors population tout court : (h) ne
        // le ramasse pas davantage.
        assert!(classify(&labeled("ready")).is_none());
    }

    /// (h) hors population : sans URL d'issue, le tour d'aujourd'hui.
    ///
    /// Deux formes, et la seconde est celle qu'on oublie — un `issue_comment`
    /// sur une **PR** porte le même gabarit de première ligne, mais une URL
    /// `/pull/`. Le filtre ne saurait pas de quel ticket il parle, donc il ne
    /// décide rien.
    #[test]
    fn mika2854_h_sans_url_dissue_le_tour_a_lieu() {
        let sur_une_pr = "[GitHub] New comment on senara-solutions/mika#2854 (t) by @samidarko\nhttps://github.com/senara-solutions/mika/pull/2854#issuecomment-123\n\nlève operator-gated";
        assert_eq!(trigger_class(sur_une_pr), TriggerClass::IssueComment);
        assert!(classify(sur_une_pr).is_none());

        // Le piège du scanner : un commentaire de PR dont le CORPS cite une
        // issue. La clé ne vient que de la ligne 2 ; sinon ce commentaire
        // entrerait dans (h) sous la clé d'un autre ticket.
        let pr_citant_une_issue = format!(
            "{sur_une_pr}\nvoir https://github.com/senara-solutions/mika/issues/1\nhttps://github.com/senara-solutions/mika/issues/1"
        );
        assert!(classify(&pr_citant_une_issue).is_none());

        // Contrôle positif : le même corps sous un commentaire d'ISSUE garde la
        // clé de la ligne 2, pas celle du corps.
        assert_eq!(
            classify(&format!(
                "{COMMENT_2854}\nhttps://github.com/senara-solutions/mika/issues/1"
            )),
            Some(Candidate::HumanTextNoTask {
                issue_url: ISSUE_URL_2854.into(),
                half: TriageHalf::Comment,
                text: format!("{COMMENT_2854}\nhttps://github.com/senara-solutions/mika/issues/1"),
            })
        );
    }

    /// (h) lit le texte ENTIER, donc la fenêtre stricte : tout handler qui l'a
    /// touché rend le tour, et seulement lui. Même forme que
    /// [`mika2680_e_tout_handler_qui_a_touche_le_texte_rend_le_tour`] ; la
    /// seconde moitié traverse la porte armée sous un bouchon qui panique à
    /// toute lecture, donc le texte touché rend le tour AVANT l'état et le
    /// triage.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2854_h_tout_handler_qui_a_touche_le_texte_rend_le_tour() {
        let c = classify(COMMENT_2854).unwrap();
        let strict = Touched {
            by_any_handler: true,
            ..intacte()
        };
        assert!(strict.applies_to(&c), "{c:?}");
        let fenetres_etroites = Touched {
            by_any_handler: false,
            after_verdict_handler: true,
            outside_ci_success_handler: true,
            outside_ci_failure_handler: true,
        };
        assert!(!fenetres_etroites.applies_to(&c), "{c:?}");

        let _garde = comment_triage::TriageEnvGuard::armed(true);
        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());
        assert_eq!(
            skip_turn(&db, &Stub::default(), COMMENT_2854, strict, "rid-now", true).await,
            Gate::Charged(Release::TextTouched)
        );
    }

    /// AC7 — un ticket **avec** tâche de boucle garde le tour, et le triage
    /// n'est pas appelé.
    ///
    /// `triage: None` est l'assertion forte : le bouchon panique à toute
    /// recherche de verdict, donc « la porte d'état passe AVANT l'appel de
    /// modèle » est prouvé, pas supposé. Un garde déplacé après le triage
    /// ferait paniquer ce test au lieu de le rendre vert autrement.
    ///
    /// **Les deux moitiés, et deux bouchons différents** (mika#2854 AC4) : la
    /// fermeture laisse `issue_task: None` et la moitié commentaire laisse
    /// `closure_snapshot: None`, donc chaque cas panique si le décideur se
    /// trompe de porte. C'est ce qui rend « la fermeture lit l'instantané »
    /// asserté plutôt que documenté.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2854_h_une_tache_de_boucle_vivante_garde_le_tour_sans_triage() {
        let _garde = comment_triage::TriageEnvGuard::armed(true);
        for (texte, stub) in [
            (COMMENT_2854, triage_stub(Some(Ok(true)), None)),
            (
                ISSUE_CLOSED_2854,
                closure_stub(ClosureSnapshot::Read(true), None),
            ),
        ] {
            assert_eq!(
                decide(classify(texte).unwrap(), &stub).await,
                Decision::Llm(Release::IssueTaskActive),
                "{texte:?}"
            );
        }
    }

    /// AC2 et AC1 au niveau du décideur : `DECISION` escalade, `NOTE:` écarte.
    ///
    /// La paire est le cœur du ticket. Même texte, même état, **seul le verdict
    /// diffère** — une batterie qui ne porterait que la branche note serait
    /// satisfaite par un filtre qui écarte tout, et c'est exactement le danger
    /// que le corps de mika#2854 nomme.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2854_h_decision_escalade_et_note_ecarte() {
        let _garde = comment_triage::TriageEnvGuard::armed(true);

        assert_eq!(
            decide(
                classify(COMMENT_2854).unwrap(),
                &triage_stub(Some(Ok(false)), Some(Ok(TriageVerdict::Decision)))
            )
            .await,
            Decision::Llm(Release::TriageEscalated)
        );

        assert_eq!(
            decide(
                classify(COMMENT_2854).unwrap(),
                &triage_stub(
                    Some(Ok(false)),
                    Some(Ok(TriageVerdict::Note("accusé de réception".into())))
                )
            )
            .await,
            Decision::SkipWithNote {
                class: PrefilterClass::CommentTriagedNote,
                target: format!("issue_comment:{ISSUE_URL_2854}"),
                note: "accusé de réception".into(),
            }
        );
    }

    /// V3 et V2 — la moitié FERMETURE suit la même règle, lue sur l'instantané
    /// (mika#2854 AC4), et sa cible porte sa propre moitié.
    ///
    /// Le bouchon laisse `issue_task: None` sur les deux branches : une lecture
    /// live glissée sur ce chemin — exactement le défaut que le point dur du
    /// ticket décrit — fait PANIQUER ce test, elle ne le rend pas vert
    /// autrement. Et la cible est `issue_closed:<url>` et non
    /// `issue_comment:<url>` : sans cette assertion, les deux populations de (h)
    /// deviendraient non soustractibles dans l'audit (mika#2205), et la mesure
    /// ne saurait plus laquelle des deux moitiés paie quoi.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2854_h_la_fermeture_suit_la_meme_regle_sur_linstantane() {
        let _garde = comment_triage::TriageEnvGuard::armed(true);

        assert_eq!(
            decide(
                classify(ISSUE_CLOSED_2854).unwrap(),
                &closure_stub(
                    ClosureSnapshot::Read(false),
                    Some(Ok(TriageVerdict::Decision))
                )
            )
            .await,
            Decision::Llm(Release::TriageEscalated)
        );

        assert_eq!(
            decide(
                classify(ISSUE_CLOSED_2854).unwrap(),
                &closure_stub(
                    ClosureSnapshot::Read(false),
                    Some(Ok(TriageVerdict::Note("fermeture routinière".into())))
                )
            )
            .await,
            Decision::SkipWithNote {
                class: PrefilterClass::CommentTriagedNote,
                target: format!("issue_closed:{ISSUE_URL_2854}"),
                note: "fermeture routinière".into(),
            }
        );
    }

    /// V5 — les trois non-lectures de l'instantané, sous trois noms distincts
    /// (mika#2854 AC4).
    ///
    /// `Unreadable` est un incident de base et rejoint les quatre autres
    /// lecteurs sous [`Release::StateUnreadable`] ; `NotAClosure` et `Disarmed`
    /// arrivés jusqu'ici sont des défauts de **câblage** et portent leur propre
    /// terme. Les trois rendent le tour complet, aucun ne rend la note : c'est
    /// l'invariant de fail-open, et le bouchon le prouve en laissant `triage:
    /// None` — un chemin qui consulterait le modèle paniquerait.
    ///
    /// Si ces trois-là partageaient un nom, « le modèle a lâché » et « le site
    /// de capture a sauté » rendraient les mêmes octets, et la prochaine mesure
    /// chercherait la panne du mauvais côté (mika#2205).
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2854_h_les_trois_non_lectures_de_linstantane_rendent_le_tour() {
        let _garde = comment_triage::TriageEnvGuard::armed(true);
        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());

        for (snapshot, attendu) in [
            (
                ClosureSnapshot::Unreadable("db down".into()),
                Release::StateUnreadable,
            ),
            (ClosureSnapshot::NotAClosure, Release::ClosureSnapshotAbsent),
            (ClosureSnapshot::Disarmed, Release::ClosureSnapshotAbsent),
        ] {
            assert_eq!(
                skip_turn(
                    &db,
                    &closure_stub(snapshot.clone(), None),
                    ISSUE_CLOSED_2854,
                    intacte(),
                    "rid-now",
                    true
                )
                .await,
                Gate::Charged(attendu),
                "{snapshot:?}"
            );
        }

        // Aucune note : les trois non-lectures rendent le tour.
        assert!(
            db.search_events("Triage webhook").await.unwrap().is_empty(),
            "une non-lecture d'instantané ne consigne JAMAIS de note"
        );

        // Et les deux absences ne rendent pas les mêmes octets dans le WARN,
        // qui est le seul endroit où l'enquête se décide.
        assert_eq!(ClosureSnapshot::NotAClosure.as_str(), "not_a_closure");
        assert_eq!(ClosureSnapshot::Disarmed.as_str(), "disarmed");
        assert_eq!(ClosureSnapshot::Read(true).as_str(), "read");
        assert_eq!(
            ClosureSnapshot::Unreadable(String::new()).as_str(),
            "unreadable"
        );
    }

    /// AC3 — fail-open, les quatre pannes.
    ///
    /// Erreur de transport, verdict illisible, expiration : toutes trois
    /// arrivent au décideur sous la même forme (`Err` de `state.triage`), parce
    /// que `run_triage` les a déjà réduites à un `anyhow::Error`. La quatrième
    /// est d'un autre genre — la lecture d'état elle-même en erreur — et porte
    /// son propre terme, parce qu'un opérateur qui grep « pourquoi ce tour a eu
    /// lieu » n'a pas la même suite à donner selon que c'est la base ou le
    /// modèle qui a lâché.
    ///
    /// La porte entière est traversée (`skip_turn`, pas `decide`) : c'est elle
    /// qui a le pouvoir d'écrire la note, donc c'est elle qu'il faut voir ne pas
    /// l'écrire.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2854_h_les_pannes_rendent_le_tour_jamais_la_note() {
        let _garde = comment_triage::TriageEnvGuard::armed(true);
        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());

        for (state, attendu) in [
            (
                triage_stub(Some(Ok(false)), Some(Err("transport"))),
                Release::TriageUnreadable,
            ),
            (
                triage_stub(Some(Ok(false)), Some(Err("verdict illisible"))),
                Release::TriageUnreadable,
            ),
            (
                triage_stub(Some(Ok(false)), Some(Err("triage expiré après 20 s"))),
                Release::TriageUnreadable,
            ),
            (
                triage_stub(Some(Err("db down")), None),
                Release::StateUnreadable,
            ),
        ] {
            assert_eq!(
                skip_turn(&db, &state, COMMENT_2854, intacte(), "rid-now", true).await,
                Gate::Charged(attendu),
                "{attendu:?}"
            );
        }

        // Aucune note n'a été écrite : les quatre pannes rendent le tour.
        assert!(
            db.search_events("Triage webhook").await.unwrap().is_empty(),
            "une panne de triage ne consigne JAMAIS de note"
        );
    }

    /// V8 — l'interrupteur désarmé rend le tour **avant** toute lecture.
    ///
    /// `Stub::default()` panique sur chacune de ses fentes, donc ce test
    /// n'affirme pas seulement « le tour a lieu » : il affirme qu'aucune base
    /// n'est interrogée et qu'aucun modèle n'est appelé. C'est la propriété qui
    /// rend la livraison désarmée gratuite, et celle qu'un garde déplacé
    /// casserait en silence autrement.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2854_h_desarmee_ne_lit_rien_du_tout() {
        let _garde = comment_triage::TriageEnvGuard::armed(false);
        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());

        for texte in [COMMENT_2854, ISSUE_CLOSED_2854] {
            assert_eq!(
                skip_turn(&db, &Stub::default(), texte, intacte(), "rid-now", true).await,
                Gate::Charged(Release::TriageDisarmed),
                "{texte:?}"
            );
        }

        // Et la capture elle-même ne lit rien : désarmée, elle rend
        // `Disarmed` sans toucher la base (mika#2854 AC4, § D11). Sans cette
        // assertion, « le vol est livré désarmé » cesserait d'être gratuit au
        // premier `issues` `closed` reçu en production.
        assert_eq!(
            snapshot_closure_task_state(Some(ISSUE_CLOSED_2854), &db).await,
            ClosureSnapshot::Disarmed
        );
    }

    /// AC5 — la note écarte le tour, écrit sa ligne d'audit sous la classe (h),
    /// et laisse sa trace en mémoire.
    ///
    /// Les trois en un seul test parce qu'ils sont un seul passage de la porte :
    /// les séparer laisserait croire qu'on peut avoir l'un sans l'autre, alors
    /// que l'ordre est le contrat (la ligne d'audit d'abord, la note ensuite —
    /// une note sans sa ligne serait une économie non chiffrable).
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2854_h_la_note_ecarte_le_tour_et_laisse_ses_deux_traces() {
        let _garde = comment_triage::TriageEnvGuard::armed(true);
        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());
        let state = triage_stub(
            Some(Ok(false)),
            Some(Ok(TriageVerdict::Note("accusé de réception".into()))),
        );

        assert_eq!(
            skip_turn(&db, &state, COMMENT_2854, intacte(), "rid-now", true).await,
            Gate::Skipped
        );

        let lignes = db
            .get_audit_events_by_trace_ids(vec!["rid-now".to_string()])
            .await
            .unwrap();
        let ecartees: Vec<_> = lignes
            .iter()
            .filter(|l| l.tool_name == PREFILTER_SKIPPED_TOOL)
            .collect();
        assert_eq!(ecartees.len(), 1, "une ligne d'audit, et une seule");
        assert_eq!(
            ecartees[0].after_value.as_deref(),
            Some(PrefilterClass::CommentTriagedNote.as_str())
        );
        assert_eq!(
            ecartees[0].target_key,
            format!("issue_comment:{ISSUE_URL_2854}")
        );

        let notes = db.search_events("Triage webhook").await.unwrap();
        assert_eq!(notes.len(), 1, "une note, et une seule");
        assert!(
            notes[0].description.contains(ISSUE_URL_2854),
            "la note nomme son ticket : {:?}",
            notes[0].description
        );
        assert_eq!(notes[0].context.as_deref(), Some("accusé de réception"));
    }

    /// Le triage juge le CONTENU, jamais l'auteur (contrôle négatif du corps du
    /// ticket : « un ordre peut venir d'un relais »).
    ///
    /// Deux textes qui ne diffèrent que par la ligne `by @…` traversent la porte
    /// sous le **même** bouchon et obtiennent le **même** verdict. Aucune liste
    /// d'identités n'existe dans la classe ; ce test est ce qui rougirait le jour
    /// où quelqu'un en ajouterait une « pour être sûr ».
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2854_h_le_verdict_ne_depend_pas_de_lauteur() {
        let _garde = comment_triage::TriageEnvGuard::armed(true);
        let de_vincent = COMMENT_2854.replace("by @samidarko", "by @samidarko-vincent");
        let dun_inconnu = COMMENT_2854.replace("by @samidarko", "by @passant-anonyme");

        for texte in [&de_vincent, &dun_inconnu] {
            assert_eq!(
                decide(
                    classify(texte).unwrap(),
                    &triage_stub(Some(Ok(false)), Some(Ok(TriageVerdict::Decision)))
                )
                .await,
                Decision::Llm(Release::TriageEscalated),
                "{texte:?}"
            );
            // Et la symétrie : sous un verdict de note, les deux sont écartés
            // de la même façon. Une asymétrie d'auteur se lirait ici.
            assert!(matches!(
                decide(
                    classify(texte).unwrap(),
                    &triage_stub(Some(Ok(false)), Some(Ok(TriageVerdict::Note("x".into()))))
                )
                .await,
                Decision::SkipWithNote { .. }
            ));
        }
    }

    /// AC1 — le corpus d'ordres d'opérateur, et **le rouge mesuré**.
    ///
    /// Les six textes ci-dessous sont les décisions réelles du 2026-10-20 sur
    /// #2784, #2680, #2814 et #1381, remises dans le gabarit du gateway. Le
    /// ticket demande que chacun escalade, et que *« le rouge soit mesuré avec
    /// un triage qui note tout »*. C'est ce que ce test fait, et il assume sa
    /// forme : sous un bouchon dégénéré qui répond `NOTE:` à tout, **les six
    /// sont dégradés en note** — l'invariant central violé, six fois.
    ///
    /// Ce n'est donc pas un test de l'escalade : c'est la preuve que le corpus
    /// DISCRIMINE. Un corpus que même un triage dégénéré fait escalader
    /// n'aurait rien mesuré, et une batterie verte sur un bouchon complaisant
    /// n'aurait rien dit du modèle. La moitié verte — les six escaladent sous un
    /// vrai modèle — appartient au vol 2 (`Tracked in:
    /// senara-solutions/mika#2854`), qui seul peut la rendre : aucun bouchon ne
    /// peut témoigner de l'obéissance d'un gabarit (discipline mika#1190).
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2854_h_le_corpus_dordres_est_discriminant() {
        let _garde = comment_triage::TriageEnvGuard::armed(true);
        let corpus = corpus_dordres();
        assert_eq!(corpus.len(), 6, "AC1 : six ordres réels");

        for texte in &corpus {
            // Chacun est bien dans la population de (h) : un corpus hors
            // population ne mesurerait rien du triage.
            assert!(
                matches!(classify(texte), Some(Candidate::HumanTextNoTask { .. })),
                "{texte:?}"
            );
            // Le rouge, mesuré : un triage qui note tout les dégrade tous.
            assert!(
                matches!(
                    decide(
                        classify(texte).unwrap(),
                        &triage_stub(
                            Some(Ok(false)),
                            Some(Ok(TriageVerdict::Note("rien d'actionnable".into())))
                        )
                    )
                    .await,
                    Decision::SkipWithNote { .. }
                ),
                "le corpus doit être dégradable, sinon il ne mesure rien : {texte:?}"
            );
        }
    }

    /// Les six ordres d'opérateur du 2026-10-20, remis dans le gabarit du
    /// gateway. Partagé avec les tests e2e de `server::tests` et avec le vol 2.
    pub(crate) fn corpus_dordres() -> Vec<String> {
        [
            (2784, "lève operator-gated, c'est bon pour moi"),
            (
                2680,
                "option (c) : un tour léger qui trie. Ouvre le ticket.",
            ),
            (2814, "je valide le vol 2, lance-le"),
            (1381, "Q1 : oui. Q2 : la seconde option. Ferme le ticket."),
            (2854, "ferme celui-ci, il est livré"),
            (2843, "relance le groom avec cette contrainte en plus"),
        ]
        .iter()
        .map(|(issue, corps)| {
            format!(
                "[GitHub] New comment on senara-solutions/mika#{issue} (t) by @samidarko\n\
                 https://github.com/senara-solutions/mika/issues/{issue}#issuecomment-1\n\n{corps}"
            )
        })
        .collect()
    }

    /// `LiveState` sans fournisseur de modèle est une ERREUR, donc le tour
    /// complet — jamais la note.
    ///
    /// Le champ est un `Option` parce que les chemins qui construisent un
    /// `LiveState` hors du serveur n'ont pas de `llm` à donner ; ce test fige
    /// que l'absence se comporte comme une panne (fail-open) et non comme un
    /// « rien à dire » (qui serait une note).
    #[tokio::test]
    async fn mika2854_h_sans_fournisseur_le_triage_est_une_panne() {
        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());
        let state = LiveState {
            db: &db,
            github_token: None,
            request_id: "rid-now",
            llm: None,
            closure_snapshot: &ClosureSnapshot::NotAClosure,
        };
        assert!(state.triage("peu importe le texte").await.is_err());
    }
}

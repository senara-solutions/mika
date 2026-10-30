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

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::reader::{HeadReview, RepoPrRead};
use super::types::{CiState, MilestoneState, SubIssue};

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
    /// **Ce n'est pas un vol d'implémentation** (mika#2946). La tâche est
    /// terminée, elle est bien passée par la boucle, mais ce qu'elle a terminé
    /// n'est pas une implémentation : un groom, ou une enveloppe de dispatch que
    /// la porte a refusée ou qu'un dispatch frais a remplacée.
    ///
    /// Elle reste **rendue**, et c'est la décision du plan : un vol filtré est
    /// un silence indistinguable d'un vol absent (classe mika#2205, motif
    /// mika#2331), et le compte de concordance a besoin que la ligne existe
    /// pour que MPC puisse la contredire (AC3).
    NotAFlight { reason: NotFlightReason },
}

/// Pourquoi cette tâche terminée n'est pas un vol d'implémentation (mika#2946).
///
/// Énumération **fermée** : chaque motif correspond à un fait qu'un producteur
/// unique a écrit dans `tasks.result`, pas à une heuristique. Élargir la famille
/// — `ISSUE_CLOSED_UPSTREAM`, `UPSTREAM_PR_MERGED`,
/// `UPSTREAM_PR_CLOSED_UNMERGED` — demande d'ajouter la variante ici, donc de
/// décider par écrit, jamais de laisser un joker absorber le cas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotFlightReason {
    /// Un groom qui a convergé — [`crate::task_state::tasks::GROOM_SUCCESS_MARKER`]
    /// en position de ligne, lu par son lecteur unique.
    GroomCompleted,
    /// Une enveloppe `ready-label` refusée par la porte des marqueurs de
    /// grooming — [`crate::task_state::tasks::DISPATCH_NO_GROOMING_MARKER`].
    DispatchRefused,
    /// Une ligne de tracking annulée parce qu'un dispatch frais l'a remplacée —
    /// [`crate::task_state::tasks::SUPERSEDED_BY_NEW_DISPATCH`].
    Superseded,
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

/// Ce que le manager **sait** de l'appartenance du vol au milestone cible
/// (mika#2968, vol 2a).
///
/// Un troisième axe de connaissance, et il est né d'un **découplage** : jusqu'ici
/// le rapport lisait l'appartenance dans [`PrStateKnowledge`], parce que les deux
/// faits avaient une source unique — le `Reader` n'hydratait que les sous-issues
/// du milestone cible, donc un appariement réussi disait les deux à la fois. Le
/// vol 2a casse cette équivalence : `pr_state` vient désormais d'une lecture
/// **par numéro de PR**, qui ignore le milestone. Garder un seul champ ferait
/// dire « hors milestone cible » à un vol dont la PR a été lue par l'autre porte,
/// et « milestone cible » à un vol simplement lu — deux populations sous un nom
/// (classe mika#2205).
///
/// `NotRead` est le défaut à la désérialisation : un blob de rapport antérieur à
/// ce champ ne porte aucune lecture d'appartenance, et c'est exactement ce que
/// cette variante dit — jamais `OutOfTarget`, qui serait une mesure inventée
/// (motif mika#2340).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MilestoneMembership {
    /// Le ticket du vol est apparié à une sous-issue du milestone cible.
    InTarget,
    /// Le ticket du vol n'est apparié à **aucune** sous-issue du milestone
    /// cible — l'appariement a eu lieu et a échoué.
    OutOfTarget,
    /// L'appartenance n'a pas été lue (défaut serde).
    #[default]
    NotRead,
}

/// Ce que le manager **sait** de la CI de la tête de la PR d'un vol (mika#2968).
///
/// Même forme et même raison que [`PrStateKnowledge`], sur un autre axe : la CI
/// d'une PR n'est lue que lorsque la lecture par numéro de PR
/// ([`super::reader::read_flight_prs`], vol 2a) a abouti sur son dépôt **et**
/// trouvé ce numéro sur une PR ouverte. Partout ailleurs il n'y a **pas de
/// mesure** —
/// et « pas de mesure » n'est pas « pas de check » : [`CiState::None`] dit qu'une
/// lecture a eu lieu et n'a trouvé aucun check, [`CiKnowledge::Unread`] dit que
/// rien n'a été lu. Deux populations sous un `Option` auraient rendu les mêmes
/// octets (classe mika#2205), et la plus dangereuse des deux confusions serait
/// qu'une CI non lue passe pour verte (AC du ticket, point 3).
///
/// La variante `Read` porte **le moment de la lecture**, parce que la
/// recommandation qu'elle fonde est écrite une fois et jamais révisée : sans cet
/// instant, le rapport affirmerait un état CI sans dire de quand il date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CiKnowledge {
    /// La CI de la tête a été lue, à cet instant (ISO 8601 UTC — le `now` du
    /// cycle, pas une horloge de ce module).
    Read { state: CiState, at: String },
    /// Rien n'a été lu, et la cause est **nommée** plutôt qu'avalée.
    Unread(CiUnreadCause),
}

/// Pourquoi la CI d'un vol n'a pas été lue (mika#2968).
///
/// Énumération **fermée** : chaque variante correspond à un point précis où la
/// chaîne de provenance se casse, et aucune ne vaut « échec inconnu ». Élargir
/// la famille demande d'ajouter la variante ici, donc de décider par écrit.
///
/// # Deux générations, et pourquoi les quatre premières restent
///
/// Les quatre variantes du vol 1 décrivaient une chaîne qui partait de la
/// sous-issue du milestone. Le vol 2a part du **numéro de PR du vol**, donc
/// aucune des quatre n'est plus écrite par
/// [`evaluate_flights`] : les six suivantes couvrent la nouvelle chaîne en
/// entier. Elles ne sont pas supprimées pour autant — `VerdictRecord::ci`
/// (`crate::flight_verdict_ledger`) désérialise des blobs écrits par le vol 1,
/// et retirer une variante ferait échouer cette lecture sur un verdict déjà
/// écrit. Chacune porte la date après laquelle plus rien ne la produit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CiUnreadCause {
    /// La sous-issue est appariée, mais son lien de PR ne vient **pas** de la
    /// liste du milestone — donc son `ci_state` ne vient d'aucune lecture de la
    /// PR. C'était le cas modal de la boucle : mesuré le 2026-10-29, aucune des
    /// cinq PR du jour ne portait le milestone.
    ///
    /// **Retirée de la production le 2026-10-30** (vol 2a) : la lecture ne part
    /// plus de la liste du milestone. Conservée pour les verdicts du vol 1.
    NotFromPrList,
    /// Le ticket du vol n'est apparié à aucune sous-issue du milestone cible —
    /// le pendant CI de [`PrStateKnowledge::NotRead`].
    ///
    /// **Retirée de la production le 2026-10-30** (vol 2a) : la CI ne dépend
    /// plus de l'appariement. Un vol hors milestone cible a désormais sa CI lue
    /// comme les autres, et son appartenance se dit par
    /// [`MilestoneMembership::OutOfTarget`].
    NoSubIssue,
    /// Le vol ne porte pas d'URL de ticket, donc il n'y a rien à apparier.
    ///
    /// **Retirée de la production le 2026-10-30** (vol 2a) : la lecture ne
    /// passe plus par le ticket. Un vol sans ticket mais avec un numéro de PR
    /// est lu ; un vol sans numéro de PR rend [`CiUnreadCause::NoPrNumber`].
    NoIssueNumber,
    /// **Non mesurée** : c'est le défaut à la désérialisation. Un blob de rapport
    /// antérieur à mika#2968 ne porte pas le champ, donc il ne porte aucune
    /// lecture — dire `Read` à sa place serait inventer une mesure.
    ///
    /// Toujours écrite, mais par serde seul et jamais par une composition.
    NotMeasured,
    /// Le vol n'a pas de numéro de PR : son issue n'est ni
    /// [`FlightIssue::PrOpened`] ni [`FlightIssue::RescuePr`], ou la PR n'a pas
    /// laissé de numéro dans `tasks.metadata` ni de ligne `PR:` dans son
    /// `result`. Il n'y a **rien** à lire, et c'est une propriété du vol, pas
    /// un échec de lecture.
    NoPrNumber,
    /// Le vol ne dit pas sur quel dépôt il a volé (`FinishedFlight::repo` à
    /// `None`), donc le numéro de PR n'est adressable nulle part : `#2966` n'a
    /// de sens que joint à un dépôt.
    NoRepo,
    /// **Aucune lecture n'a été tentée pour ce dépôt** — soit le cycle n'a pas
    /// de lecteur `gh` câblé ([`FlightPrReads::Unwired`]), soit le dépôt de ce
    /// vol n'était pas dans l'ensemble lu. Une seule population : *non tentée*,
    /// que [`CiUnreadCause::ReadFailed`] sépare de *tentée et échouée*.
    NoRunner,
    /// La lecture a eu lieu et a échoué : le premier appel `gh` du dépôt n'a
    /// pas abouti ([`RepoPrRead::Failed`]), ou la PR est ouverte mais le second
    /// appel — celui qui porte `statusCheckRollup` — n'a pas abouti. C'est le
    /// cas des dépôts où le jeton ne lit pas le rollup (mika#2963).
    ReadFailed,
    /// Le dépôt a été lu, et le numéro de la PR n'était **pas** dans la page
    /// rendue. Une page pleine ne dit pas « voici toutes les PR » : la cause
    /// nomme la borne plutôt que de laisser croire que la PR n'existe pas.
    NotFound,
    /// La PR a été lue et n'est pas ouverte (fusionnée, fermée). Le second
    /// appel ne demande le rollup que des PR ouvertes, donc il n'y a pas de CI
    /// à lire — et la recommandation n'en a pas besoin : un état autre que
    /// `open` décide à lui seul.
    PrNotOpen,
}

impl CiKnowledge {
    /// Le défaut serde du champ [`FlightVerdict::ci`] — voir
    /// [`CiUnreadCause::NotMeasured`].
    ///
    /// `pub(crate)` parce que `VerdictRecord::ci`
    /// (`crate::flight_verdict_ledger`) prend **le même** défaut : les deux
    /// désérialisent la même population de blobs antérieurs au champ, et une
    /// seconde graphie du défaut diverge le jour où l'une des deux change
    /// (mika#2201). Un `impl Default` aurait été cette seconde graphie.
    pub(crate) fn default_unread() -> Self {
        CiKnowledge::Unread(CiUnreadCause::NotMeasured)
    }
}

/// Pourquoi la **revue** de la tête n'a pas été lue (mika#2968, vol 2b).
///
/// Un **alias**, et non un second énuméré : les causes de non-lecture de la tête
/// sont exactement les mêmes pour la CI et pour la revue, parce que les deux
/// sortent de l'appel 2 de [`super::reader::read_flight_prs`], joint au même
/// appel 1. Dix variantes de plus diraient la même chose deux fois, et
/// divergeraient au premier ajout fait d'un seul côté.
///
/// `serde(rename_all)` porte sur les **variantes**, jamais sur le nom du type :
/// l'alias est donc neutre à la sérialisation — `ReviewKnowledge::Unread` écrit
/// les mêmes octets qu'un énuméré jumeau l'aurait fait.
pub type HeadUnreadCause = CiUnreadCause;

/// Ce que le manager **sait** de la revue de la tête de la PR d'un vol
/// (mika#2968, vol 2b).
///
/// Même forme et même raison que [`CiKnowledge`], sur le troisième axe de la
/// même tête : voir son docstring pour pourquoi « lue et vide » et « non lue »
/// sont deux variantes et non un `Option`. Ce qui est propre à cet axe est
/// ailleurs — dans [`HeadReviewState`] pour les états, et dans le docstring de
/// `recommend` pour l'asymétrie avec la CI sur la non-lecture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewKnowledge {
    /// La revue de la tête a été lue, à cet instant (ISO 8601 UTC — le `now` du
    /// cycle, comme [`CiKnowledge::Read`]).
    Read {
        state: HeadReviewState,
        /// `reviewDecision` tel que la forge l'agrège, **pour le rapport
        /// seul** : il ne dit pas sur quel commit, donc il ne peut pas fonder
        /// `ApprovedOnHead`. Absent quand la forge ne le renseigne pas, jamais
        /// une chaîne vide (motif mika#2331).
        #[serde(default)]
        decision: Option<String>,
        at: String,
    },
    /// Rien n'a été lu, et la cause est **nommée** plutôt qu'avalée.
    Unread(HeadUnreadCause),
}

/// L'état de la dernière revue du relecteur attendu, **relativement à la tête**
/// (mika#2968, vol 2b).
///
/// Chaque variante est tranchée sur des champs **structurés** — `state`,
/// l'auteur, `commit` — jamais sur le texte de la revue : une revue `COMMENTED`
/// n'est jamais une approbation, quel que soit son corps. Le seul endroit où le
/// corps est lu est `reason`, qui ne sert qu'au rapport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeadReviewState {
    /// Approuvée par le relecteur attendu, **sur la tête exacte**. Le seul état
    /// qui laisse `MergeCandidate` (AC6).
    ApprovedOnHead,
    /// Approuvée, mais sur un commit qui n'est pas la tête — ou sur un commit
    /// que la lecture n'a pas pu comparer à la tête. Les deux vont ici parce
    /// que ni l'une ni l'autre n'est une approbation *de ce qui serait fusionné*,
    /// et c'est le sens sûr : une tête absente ne peut pas compter pour une
    /// égalité.
    ApprovedOnOlderCommit {
        /// Le commit que la revue visait, s'il est connu.
        reviewed: Option<String>,
    },
    /// La dernière revue porte un `VERDICT: hold[<motif>]` — le cas mesuré de
    /// mika#2972, où le tour de revue avait été coupé à 30 steps d'outil.
    Held {
        reason: String,
        /// Si ce `hold` portait bien sur la tête. Un `hold` sur un commit
        /// antérieur ne décrit pas ce qui serait fusionné, et le rapport le dit.
        on_head: bool,
    },
    /// La dernière revue n'approuve pas et ne porte pas de verdict moteur :
    /// un commentaire, une revue rejetée, un `changes_requested`.
    NoVerdict { on_head: bool },
    /// Le relecteur attendu n'a posé **aucune** revue — sur un tableau `reviews`
    /// qui a bien été lu.
    NoReview,
}

impl ReviewKnowledge {
    /// Le défaut serde du champ [`FlightVerdict::review`] — même rôle et même
    /// `pub(crate)` que [`CiKnowledge::default_unread`], parce que
    /// `VerdictRecord::review` prend le même.
    pub(crate) fn default_unread() -> Self {
        ReviewKnowledge::Unread(HeadUnreadCause::NotMeasured)
    }
}

/// Ce que le cycle a lu des PR des vols, par dépôt (mika#2968, vol 2a).
///
/// Enveloppe nommée autour du rendu de [`super::reader::read_flight_prs`], et
/// non un `Option<HashMap<…>>`, parce que « le cycle n'a pas de lecteur `gh` »
/// et « le lecteur a tourné et n'a rien rendu » sont deux populations qu'un
/// `None` rendrait indistinguables (classe mika#2205). La première est une
/// propriété du câblage du cycle — en production elle ne survient que sur un
/// cycle sans jeton — la seconde une propriété des dépôts lus.
///
/// Elle vit ici plutôt que dans le `reader` pour que la signature de
/// `read_flight_prs` reste celle du § 4 du plan : c'est `cadence.rs` qui
/// enveloppe, parce que c'est lui qui sait s'il a un lecteur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlightPrReads {
    /// Aucun lecteur `gh` n'est câblé dans ce cycle : rien n'a été tenté.
    Unwired,
    /// Les lectures, indexées par dépôt (`senara-solutions/mika`, …). Un dépôt
    /// absent de la table n'a pas été tenté non plus ; un dépôt présent porte
    /// son propre [`RepoPrRead`].
    Read(HashMap<String, RepoPrRead>),
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
    /// **Rien à recommander** (mika#2946) : la tâche n'est pas un vol
    /// d'implémentation, donc aucun geste n'est dû sur elle.
    ///
    /// Une variante, et pas un `Option<FlightRecommendation>` : le champ
    /// [`FlightVerdict::recommendation`] est non optionnel, sérialisé par
    /// `VerdictRecord`, traduit par `VerdictTerms`, reconstruit par
    /// `parse_judgment_terms` et exigé par `--recommendation`. Un `Option`
    /// rippellerait sur cinq sites **et** fondrait « rien à recommander » dans
    /// « recommandation absente » — deux populations sous un nom (mika#2205).
    Nothing,
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
    /// Ce que le manager sait de l'appartenance du vol au **milestone cible**
    /// (mika#2968, vol 2a) — le fait que `pr_state` portait jusqu'ici par
    /// accident de provenance.
    ///
    /// `#[serde(default)]` ⇒ [`MilestoneMembership::NotRead`], même raison que
    /// `pr_state`.
    #[serde(default)]
    pub in_target_milestone: MilestoneMembership,
    /// Ce que le manager sait de la CI de la tête de la PR (mika#2968) — lue à
    /// un instant nommé, ou non lue pour une cause nommée.
    ///
    /// `#[serde(default = "CiKnowledge::default_unread")]` ⇒
    /// `Unread(NotMeasured)`, pour la même raison que `pr_state` : un blob
    /// antérieur à ce champ ne porte aucune lecture, et c'est exactement ce que
    /// ce défaut dit.
    #[serde(default = "CiKnowledge::default_unread")]
    pub ci: CiKnowledge,
    /// Ce que le manager sait de la **revue** de la tête de la PR (mika#2968,
    /// vol 2b) — le second axe de la même tête, lu par le même appel.
    ///
    /// `#[serde(default = "ReviewKnowledge::default_unread")]`, même raison que
    /// `ci` juste au-dessus.
    #[serde(default = "ReviewKnowledge::default_unread")]
    pub review: ReviewKnowledge,
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
///
/// `read_at` est l'instant **du cycle** (ISO 8601 UTC), porté par la variante
/// [`CiKnowledge::Read`] (mika#2968). Il est passé plutôt que capturé ici :
/// aucune horloge ne vit dans ce module, et le `now` du cycle majore la lecture
/// `gh` au lieu de la dater après les sept appels — d'où le libellé « au cycle
/// de » plutôt que « lue à » (§ 2.5 du plan).
///
/// `pr_reads` est la lecture **par numéro de PR** du vol 2a, et depuis ce vol
/// c'est la seule source de `ci` **et** de `pr_state`. Les deux voyagent
/// ensemble parce qu'ils sortent du même appel `gh` : les séparer rendrait
/// possible un verdict dont l'état de PR vient d'une lecture et la CI d'une
/// autre. L'ancienne porte — le `pr_link_source` de la sous-issue appariée —
/// est retirée : elle ne s'ouvrait que pour les PR portant elles-mêmes le
/// milestone, c'est-à-dire aucune des cinq PR de la boucle du 2026-10-29.
/// `sub_issues` ne sert donc plus qu'à l'appartenance au milestone.
pub fn evaluate_flights(
    flights: &[FinishedFlight],
    sub_issues: &[SubIssue],
    pr_reads: &FlightPrReads,
    read_at: &str,
) -> Vec<FlightVerdict> {
    flights
        .iter()
        .map(|f| {
            let issue_number = issue_number_from_url(f.reference_url.as_deref());
            let matched = issue_number.and_then(|n| sub_issues.iter().find(|s| s.number == n));
            // L'appartenance au milestone cible est **son propre** champ
            // (mika#2968, vol 2a) : c'est ce découplage qui permet à la CI
            // d'être lue pour un vol hors milestone cible.
            let in_target_milestone = match (issue_number, matched) {
                (None, _) => MilestoneMembership::NotRead,
                (Some(_), None) => MilestoneMembership::OutOfTarget,
                (Some(_), Some(_)) => MilestoneMembership::InTarget,
            };
            let (pr_state, ci, review) =
                compose_pr_knowledge(f.repo.as_deref(), flight_pr_number(f), pr_reads, read_at);
            evaluate_flight(
                f,
                issue_number,
                &pr_state,
                &ci,
                &review,
                in_target_milestone,
            )
        })
        .collect()
}

/// Le numéro de la PR qu'un vol a produite, s'il en a produit une.
///
/// Passe par [`classify_issue`], qui **est** le lecteur unique de ce que
/// `tasks.result` et `tasks.metadata` disent du dénouement d'un vol
/// (mika#2201). Une fonction pure appelée deux fois dans le même tour n'est pas
/// un second lecteur : c'est le même, et les deux appels rendent la même issue
/// par construction. Rouvrir `result` ici à la main en serait un.
fn flight_pr_number(flight: &FinishedFlight) -> Option<u64> {
    let pilot = PilotFields::parse(flight.metadata.as_deref());
    let result = flight.result.as_deref().unwrap_or("");
    match classify_issue(result, &pilot) {
        FlightIssue::PrOpened { number, .. } | FlightIssue::RescuePr { number, .. } => number,
        FlightIssue::Dead { .. } | FlightIssue::NotAFlight { .. } => None,
    }
}

/// Ce que la lecture par numéro dit de la PR d'un vol : son état, sa CI, et la
/// revue de sa tête.
///
/// Les trois sortent **de la même lecture**, donc d'une seule fonction. La table
/// est celle des § 5 et § 6 du plan, étendue au troisième terme par le vol 2b :
///
/// | situation | `pr_state` | `ci` | `review` |
/// |---|---|---|---|
/// | le vol n'a pas de numéro de PR | `NotRead` | `Unread(NoPrNumber)` | `Unread(NoPrNumber)` |
/// | le vol ne dit pas son dépôt | `NotRead` | `Unread(NoRepo)` | `Unread(NoRepo)` |
/// | rien n'a été tenté sur ce dépôt | `NotRead` | `Unread(NoRunner)` | `Unread(NoRunner)` |
/// | la lecture du dépôt a échoué | `NotRead` | `Unread(ReadFailed)` | `Unread(ReadFailed)` |
/// | le numéro n'est pas dans la page lue | `NotRead` | `Unread(NotFound)` | `Unread(NotFound)` |
/// | PR lue, non ouverte | `Read(Some(état))` | `Unread(PrNotOpen)` | `Unread(PrNotOpen)` |
/// | PR lue, ouverte, second appel échoué | `Read(Some(état))` | `Unread(ReadFailed)` | `Unread(ReadFailed)` |
/// | PR lue, ouverte, second appel lu | `Read(Some(état))` | `Read { … }` | `Read { … }` |
///
/// **Les deux dernières colonnes sont identiques par construction, et c'est la
/// justification de l'alias [`HeadUnreadCause`]** : les causes de non-lecture
/// sont les mêmes parce que `ci`, `head_oid` et `review` sortent du même appel
/// `gh`. Une divergence future entre ces deux colonnes serait le signal qu'il
/// faut deux énumérés — pas avant.
///
/// **L'ordre de lecture est porteur** : `state` est lu **avant** `ci` et
/// `review`, parce que `FlightPrRead::ci` et `FlightPrRead::review` valent
/// `None` dans deux cas de sens opposé — sur une PR non ouverte le second appel
/// `gh` ne les a pas demandées, sur une PR ouverte il a échoué. Lire `ci`
/// d'abord fondrait les deux en une seule cause, et « la CI n'avait pas à être
/// lue » passerait pour « la CI n'a pas pu être lue » (classe mika#2205).
///
/// `pr_state` est `Read` **dès que le numéro est trouvé**, y compris pour une PR
/// non ouverte : l'état *a* été lu, et c'est lui qui décide la recommandation
/// dans ce cas.
fn compose_pr_knowledge(
    repo: Option<&str>,
    pr_number: Option<u64>,
    pr_reads: &FlightPrReads,
    read_at: &str,
) -> (PrStateKnowledge, CiKnowledge, ReviewKnowledge) {
    let non_lu = |cause| {
        (
            PrStateKnowledge::NotRead,
            CiKnowledge::Unread(cause),
            ReviewKnowledge::Unread(cause),
        )
    };
    let Some(number) = pr_number else {
        return non_lu(CiUnreadCause::NoPrNumber);
    };
    let Some(repo) = repo else {
        return non_lu(CiUnreadCause::NoRepo);
    };
    let FlightPrReads::Read(par_depot) = pr_reads else {
        return non_lu(CiUnreadCause::NoRunner);
    };
    let Some(depot) = par_depot.get(repo) else {
        return non_lu(CiUnreadCause::NoRunner);
    };
    let prs = match depot {
        RepoPrRead::Read(prs) => prs,
        RepoPrRead::Failed => return non_lu(CiUnreadCause::ReadFailed),
    };
    let Some(pr) = prs.get(&number) else {
        return non_lu(CiUnreadCause::NotFound);
    };
    let pr_state = PrStateKnowledge::Read(Some(pr.state.clone()));
    // `open` est comparé sans égard à la casse, comme dans `recommend` : `gh`
    // écrit `OPEN`, et c'est la graphie que le rapport affiche telle quelle.
    if !pr.state.eq_ignore_ascii_case("open") {
        return (
            pr_state,
            CiKnowledge::Unread(CiUnreadCause::PrNotOpen),
            ReviewKnowledge::Unread(HeadUnreadCause::PrNotOpen),
        );
    }
    let ci = match &pr.ci {
        Some(state) => CiKnowledge::Read {
            state: state.clone(),
            at: read_at.to_string(),
        },
        None => CiKnowledge::Unread(CiUnreadCause::ReadFailed),
    };
    // `head_oid` est la preuve que le second appel a abouti : il est toujours
    // renseigné sur une PR ouverte que `gh` a rendue. Son absence est donc une
    // lecture échouée, et surtout jamais une approbation — voir
    // [`compose_head_review_state`].
    let review = match &pr.head_oid {
        Some(head_oid) => ReviewKnowledge::Read {
            state: compose_head_review_state(pr.review.as_ref(), head_oid),
            decision: pr.review_decision.clone(),
            at: read_at.to_string(),
        },
        None => ReviewKnowledge::Unread(HeadUnreadCause::ReadFailed),
    };
    (pr_state, ci, review)
}

/// La dernière revue du relecteur attendu, située **par rapport à la tête**.
///
/// Trois propriétés que ce classement porte, et qu'un futur éditeur ne doit pas
/// avoir à redériver :
///
/// - **`APPROVED` est comparé sans égard à la casse**, comme `open` au-dessus :
///   `gh` écrit les états de revue en capitales.
/// - **Un `commit` absent ne vaut pas la tête.** `reviews[].commit.oid` peut
///   revenir vide (c'est ce que `latestReviews` rend *systématiquement*, mesuré
///   le 2026-10-30 sur cinq PR sur cinq, et c'est pourquoi le lecteur interroge
///   `reviews`). Une approbation dont on ne sait pas ce qu'elle visait n'est pas
///   une approbation de ce qui serait fusionné : elle tombe dans
///   `ApprovedOnOlderCommit { reviewed: None }`, qui ne laisse pas `MergeCandidate`.
/// - **Le `hold` n'est lu que sur une revue non approuvante.** Une approbation
///   qui citerait `hold[...]` en prose reste une approbation : la décision tient
///   aux champs structurés, jamais au texte (mika#2972 a montré l'inverse — une
///   revue `COMMENTED` portant le verdict moteur — et c'est ce cas-là que
///   `Held` nomme).
fn compose_head_review_state(review: Option<&HeadReview>, head_oid: &str) -> HeadReviewState {
    let Some(review) = review else {
        return HeadReviewState::NoReview;
    };
    let on_head = review
        .commit
        .as_deref()
        .is_some_and(|c| c.eq_ignore_ascii_case(head_oid));
    if review.state.eq_ignore_ascii_case("APPROVED") {
        return if on_head {
            HeadReviewState::ApprovedOnHead
        } else {
            HeadReviewState::ApprovedOnOlderCommit {
                reviewed: review.commit.clone(),
            }
        };
    }
    match &review.hold_reason {
        Some(reason) => HeadReviewState::Held {
            reason: reason.clone(),
            on_head,
        },
        None => HeadReviewState::NoVerdict { on_head },
    }
}

/// Le verdict d'un vol, `pr_state` étant ce que le manager **sait** de l'état de
/// sa PR : [`PrStateKnowledge::Read`] quand le `Reader` l'a hydraté (`open` /
/// `closed` / `merged`, ou `None` s'il n'y en a pas),
/// [`PrStateKnowledge::NotRead`] quand le ticket n'appartient pas au milestone
/// cible et n'a donc jamais été lu.
///
/// `ci` est ce qu'il sait de la CI de la tête de cette PR (mika#2968) — lue à un
/// instant nommé, ou non lue pour une cause nommée. L'appelant la **compose**,
/// ce qui garde la provenance hors de cette fonction. `review` est le second axe
/// de la même tête (vol 2b) et suit exactement la même règle.
///
/// `in_target_milestone` suit la même règle, et c'est pourquoi c'est un
/// paramètre plutôt qu'une dérivation de `pr_state` : depuis le vol 2a de
/// mika#2968 les deux faits n'ont plus la même source, et les recoller ici
/// rendrait le découplage inopérant au premier appelant qui l'oublie. Le
/// compilateur exige donc que chaque site **nomme** ce qu'il sait.
pub fn evaluate_flight(
    flight: &FinishedFlight,
    issue_number: Option<u64>,
    pr_state: &PrStateKnowledge,
    ci: &CiKnowledge,
    review: &ReviewKnowledge,
    in_target_milestone: MilestoneMembership,
) -> FlightVerdict {
    let result = flight.result.as_deref().unwrap_or("");
    let pilot = PilotFields::parse(flight.metadata.as_deref());
    let issue = classify_issue(result, &pilot);
    FlightVerdict {
        task_id: flight.task_id.clone(),
        repo: flight.repo.clone(),
        issue_number,
        recommendation: recommend(&issue, pr_state, ci),
        pr_state: pr_state.clone(),
        in_target_milestone,
        ci: ci.clone(),
        review: review.clone(),
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
///
/// **La CI entre dans la décision depuis mika#2968, et par un seul bras** : une
/// CI *lue* et rouge sur une PR encore ouverte rend `ManualResumption`. Trois
/// choses que ce bras décide, et qui ne se devinent pas en le lisant :
///
/// - **`ManualResumption`, pas une variante neuve.** C'est le geste dû — la PR
///   attend un correctif — et c'est ce que MPC a jugé sur le vol mesuré. Une
///   quatrième variante ajouterait un terme au vocabulaire que `VerdictTerms`
///   compare, donc à celui que MPC doit apprendre (mika#2158), pour un geste
///   identique. Le *pourquoi* va dans le segment CI du rapport.
/// - **`Pending` reste `MergeCandidate`.** Un verdict est écrit une fois, ~2 min
///   après l'ouverture de la PR, pendant que la CI tourne. Refuser sur `Pending`
///   rendrait `ManualResumption` sur la quasi-totalité des vols sains, et un
///   instrument qui crie à chaque vol ne dit plus rien. Une CI en cours est une
///   absence de preuve, pas une preuve de refus — et le gate de merge la refuse
///   de lui-même.
/// - **`Unread(_)` reste `MergeCandidate`**, comme `PrStateKnowledge::NotRead`
///   et pour la même raison : deviner le refus sur une non-lecture enverrait un
///   humain sur chaque vol. Ce que le manager doit cesser de faire, c'est
///   présenter la non-lecture comme une CI verte, et c'est **au rendu** que
///   cette distinction se dit.
///
/// Un rouge posé **après** le verdict ne le fait pas réviser : le verdict est
/// une mesure horodatée, et l'`at` de [`CiKnowledge::Read`] est ce qui la rend
/// honnête.
fn recommend(
    issue: &FlightIssue,
    pr_state: &PrStateKnowledge,
    ci: &CiKnowledge,
) -> FlightRecommendation {
    match issue {
        FlightIssue::PrOpened { .. } => match pr_state {
            // Fusionnée ou fermée pendant que le vol volait : le verdict du vol
            // est en retard sur la réalité, un humain tranche. **Avant** le bras
            // CI, parce que l'état de la PR décide quelle que soit la CI.
            PrStateKnowledge::Read(Some(s)) if !s.eq_ignore_ascii_case("open") => {
                FlightRecommendation::ManualResumption
            }
            // AC1 : une CI **lue** et rouge sur une PR encore ouverte.
            _ if matches!(
                ci,
                CiKnowledge::Read {
                    state: CiState::Failed,
                    ..
                }
            ) =>
            {
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
        // mika#2946 : la tâche n'est pas un vol, donc le manager ne recommande
        // rien. C'est le défaut que ce ticket ferme — ces huit lignes
        // recommandaient « reprise à la main » sur des vols qui n'ont jamais
        // existé.
        FlightIssue::NotAFlight { .. } => FlightRecommendation::Nothing,
    }
}

/// Quelle issue `tasks.result` décrit.
fn classify_issue(result: &str, pilot: &PilotFields) -> FlightIssue {
    // mika#2946 — en TÊTE, avant toute lecture d'`Outcome:`. L'ordre est la
    // décision : `death_cause` a un bras attrape-tout (`Unknown`), donc tout
    // test placé après lui est inatteignable pour les trois formes ci-dessous.
    if let Some(reason) = not_a_flight_reason(result) {
        return FlightIssue::NotAFlight { reason };
    }
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

/// Cette tâche terminée est-elle autre chose qu'un vol d'implémentation ?
/// (mika#2946)
///
/// Fonction **pure**, un seul argument : les trois faits se lisent dans
/// `tasks.result` et nulle part ailleurs. `PilotFields` n'y entre pas — le plan
/// l'avait esquissée en second paramètre, mais aucune des trois formes ne
/// consulte les métadonnées du pilote, et un paramètre inutilisé laisse croire
/// à une dépendance qui n'existe pas.
///
/// # Les trois faits, et par quel lecteur
///
/// 1. **Groom convergé.** `Outcome: PLAN_GROOMED` est un jeton canonique de
///    classe B dont [`crate::task_state::tasks::groom_result_convergence`] est le
///    lecteur **unique** (mika#2590) : ce module l'appelle et ne nomme jamais le
///    marqueur. Seul [`crate::task_state::tasks::GroomConvergence::Converged`]
///    compte — un `MarkerOutOfPosition` est le JSON d'auto-skip qui *cite* le
///    marqueur sans l'ancrer (les 651 caractères que mika#2590 a dû refuser), et
///    il reste classé exactement comme aujourd'hui.
/// 2. **Refus au dispatch.** Lecture d'un **champ** de l'enveloppe JSON —
///    `"error"` — jamais un `contains` sur le corps du `result` : le texte de
///    `recovery` et celui de `reason` parlent tous deux du grooming, et une
///    sous-chaîne y trouverait ce qu'elle cherche dans une phrase qui explique
///    le contraire (classe mika#2050 / mika#2590).
/// 3. **Remplacé.** `tasks.result` vaut exactement
///    [`crate::task_state::tasks::SUPERSEDED_BY_NEW_DISPATCH`], écrit par
///    `Database::cancel_task_superseded` et par personne d'autre.
fn not_a_flight_reason(result: &str) -> Option<NotFlightReason> {
    use crate::task_state::tasks::{
        DISPATCH_NO_GROOMING_MARKER, SUPERSEDED_BY_NEW_DISPATCH, groom_result_convergence,
    };

    if groom_result_convergence(result).is_converged() {
        return Some(NotFlightReason::GroomCompleted);
    }
    if envelope_error_is(result, DISPATCH_NO_GROOMING_MARKER) {
        return Some(NotFlightReason::DispatchRefused);
    }
    if result.trim() == SUPERSEDED_BY_NEW_DISPATCH {
        return Some(NotFlightReason::Superseded);
    }
    None
}

/// Le `result` est-il une enveloppe JSON dont le champ `"error"` vaut
/// exactement `token` ?
///
/// Un **champ**, et pas une sous-chaîne : c'est tout l'objet de cette fonction.
/// Faux dès que le `result` n'est pas un objet JSON, ou que `error` est absent,
/// n'est pas une chaîne, ou dit autre chose.
fn envelope_error_is(result: &str, token: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(result)
        .ok()
        .as_ref()
        .and_then(|v| v.get("error")?.as_str())
        == Some(token)
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
    use super::super::reader::FlightPrRead;
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
            missing_grooming_signals: None,
            number,
            title: format!("sujet {number}"),
            state: crate::milestone_manager::types::IssueState::Open,
            priority_rank: None,
            plan_present: true,
            plan_reading: None,
            dormancy: None,
            branch_present: true,
            pr_number: None,
            pr_state: pr_state.map(str::to_string),
            pr_url: None,
            pr_link_source: None,
            merged_prs: None,
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

    /// « Rien n'a été lu de la CI » (mika#2968) — la connaissance que portent
    /// les tests qui n'instruisent pas cet axe. La valeur est écrite ici
    /// plutôt qu'empruntée à `CiKnowledge::default_unread` : un test qui
    /// suivrait le défaut de production n'attesterait plus rien le jour où ce
    /// défaut changerait.
    const CI_NON_LUE: CiKnowledge = CiKnowledge::Unread(CiUnreadCause::NotMeasured);

    /// « Rien n'a été lu de la revue » (vol 2b) — même rôle et même raison de
    /// ne pas emprunter `ReviewKnowledge::default_unread` que [`CI_NON_LUE`].
    const REVUE_NON_LUE: ReviewKnowledge = ReviewKnowledge::Unread(HeadUnreadCause::NotMeasured);

    /// « La tête est approuvée par le relecteur attendu » — la seule
    /// connaissance de revue qui laisse `MergeCandidate` (AC6).
    ///
    /// Les tests antérieurs au vol 2b portent sur d'autres axes et attendent
    /// leur recommandation d'avant : ils passent donc *celle-ci*, qui est
    /// neutre pour la nouvelle garde. Les faire passer [`REVUE_NON_LUE`]
    /// retournerait leur attente sans rien attester de la revue.
    fn revue_approuvee() -> ReviewKnowledge {
        ReviewKnowledge::Read {
            state: HeadReviewState::ApprovedOnHead,
            decision: Some("APPROVED".to_string()),
            at: AU_CYCLE_DE.to_string(),
        }
    }

    /// « L'appartenance au milestone n'a pas été lue » (mika#2968, vol 2a) — la
    /// connaissance que portent les appels directs à [`evaluate_flight`] sans
    /// numéro de ticket, où aucun appariement n'a pu avoir lieu. Écrite ici pour
    /// la même raison que [`CI_NON_LUE`].
    const CIBLE_NON_LUE: MilestoneMembership = MilestoneMembership::NotRead;

    /// L'instant du cycle qui date la lecture CI.
    const AU_CYCLE_DE: &str = "2026-10-29T17:46:12Z";

    /// Le dépôt des fixtures de ce module — celui que chacune nomme dans son
    /// champ `repo`, donc celui sous lequel une lecture doit être rangée pour
    /// que la jointure ait lieu.
    const DEPOT: &str = "senara-solutions/mika";

    /// Aucun lecteur `gh` câblé (mika#2968, vol 2a) : la connaissance que
    /// portent les tests qui n'instruisent pas l'axe de la lecture.
    const SANS_LECTEUR: FlightPrReads = FlightPrReads::Unwired;

    /// Une lecture par numéro de PR sur [`DEPOT`], gréée : `(numéro, état, CI)`.
    ///
    /// `ci: None` est **une propriété de la lecture**, jamais une CI verte : sur
    /// une PR non ouverte le second appel `gh` ne l'a pas demandée, sur une PR
    /// ouverte il a échoué. L'helper laisse donc l'appelant le dire.
    fn lectures(prs: &[(u64, &str, Option<CiState>)]) -> FlightPrReads {
        let mut page = HashMap::new();
        for (numero, etat, ci) in prs {
            page.insert(
                *numero,
                FlightPrRead {
                    state: etat.to_string(),
                    ci: ci.clone(),
                    head_oid: None,
                    review: None,
                    review_decision: None,
                },
            );
        }
        let mut par_depot = HashMap::new();
        par_depot.insert(DEPOT.to_string(), RepoPrRead::Read(page));
        FlightPrReads::Read(par_depot)
    }

    /// La lecture d'un dépôt dont le premier appel `gh` n'a pas abouti.
    fn lecture_en_echec() -> FlightPrReads {
        let mut par_depot = HashMap::new();
        par_depot.insert(DEPOT.to_string(), RepoPrRead::Failed);
        FlightPrReads::Read(par_depot)
    }

    // ---- AC3 : les trois issues, avec les tours lus correctement ----------

    #[test]
    fn mika2698_une_pr_ouverte_est_un_merge_candidat_avec_ses_tours() {
        let v = evaluate_flights(
            &[fixture_pr_opened()],
            &[sub_issue(2693, Some("open"))],
            &lectures(&[(2696, "OPEN", Some(CiState::Success))]),
            AU_CYCLE_DE,
        );
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
        let v = evaluate_flights(
            &[fixture_rescue_pr()],
            &[sub_issue(2655, Some("open"))],
            &lectures(&[(2690, "OPEN", Some(CiState::Success))]),
            AU_CYCLE_DE,
        );
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
        let v = evaluate_flights(
            &[fixture_dead_at_ceiling()],
            &[sub_issue(2661, None)],
            // Le dépôt est lu, et la page est vide : la cause de non-lecture
            // doit être « ce vol n'a pas de PR », pas « hors de la page ».
            &lectures(&[]),
            AU_CYCLE_DE,
        );
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
        assert_eq!(
            v.ci,
            CiKnowledge::Unread(CiUnreadCause::NoPrNumber),
            "un vol sans PR n'a pas de CI à lire, et le dit par sa cause propre"
        );
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
            let v = evaluate_flight(
                &f,
                None,
                &PrStateKnowledge::NotRead,
                &CI_NON_LUE,
                &REVUE_NON_LUE,
                CIBLE_NON_LUE,
            );
            assert_eq!(v.turn_ceiling, None, "{}", f.task_id);
            assert_eq!(v.last_commit_turn, None, "{}", f.task_id);
        }
    }

    // ---- Les tours : les deux sources, et leur ordre ----------------------

    #[test]
    fn mika2698_les_tours_retombent_sur_la_ligne_turns_sans_metadata() {
        let mut f = fixture_pr_opened();
        f.metadata = None;
        let v = evaluate_flight(
            &f,
            None,
            &lu(Some("open")),
            &CI_NON_LUE,
            &revue_approuvee(),
            CIBLE_NON_LUE,
        );
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
            evaluate_flight(
                &f,
                None,
                &PrStateKnowledge::NotRead,
                &CI_NON_LUE,
                &REVUE_NON_LUE,
                CIBLE_NON_LUE
            )
            .turns_used,
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
            evaluate_flight(
                &f,
                None,
                &PrStateKnowledge::NotRead,
                &CI_NON_LUE,
                &REVUE_NON_LUE,
                CIBLE_NON_LUE
            )
            .turns_used,
            Some(200)
        );
    }

    // ---- Les causes de mort, une par forme du producteur -----------------

    /// Le verdict d'un `result` nu — ni métadonnées, ni sous-ticket apparié : la
    /// forme minimale où la seule variable est ce que le producteur a écrit.
    fn verdict_of(result: &str) -> FlightVerdict {
        let f = FinishedFlight {
            task_id: "t".to_string(),
            result: Some(result.to_string()),
            ..Default::default()
        };
        evaluate_flight(
            &f,
            None,
            &PrStateKnowledge::NotRead,
            &CI_NON_LUE,
            &REVUE_NON_LUE,
            CIBLE_NON_LUE,
        )
    }

    fn dead_cause_of(result: &str) -> DeathCause {
        match verdict_of(result).issue {
            FlightIssue::Dead { cause } => cause,
            other => panic!("attendu mort, obtenu {other:?}"),
        }
    }

    /// Chaque forme du producteur, et la cause qu'elle porte.
    ///
    /// **Depuis mika#2946 cette population est plus étroite qu'elle n'en a
    /// l'air** : trois formes — un groom convergé, une enveloppe de dispatch
    /// refusée, une ligne remplacée — n'entrent plus dans [`death_cause`] du
    /// tout, parce que [`classify_issue`] les écarte en tête. Elles tombaient
    /// ici, dans `Unknown("PLAN_GROOMED")` et `Unknown("no_outcome_line")`, et
    /// c'est le défaut que ce ticket ferme. Leur classement est asséré par
    /// `mika2946_un_groom_un_refus_et_un_remplacement_ne_sont_pas_des_vols` ; ce
    /// qui reste tenu **ici** est que les deux `Unknown` restent atteignables
    /// pour tout le reste, car un prédicat trop large en amont les viderait.
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
        let v = evaluate_flight(
            &f,
            None,
            &PrStateKnowledge::NotRead,
            &CI_NON_LUE,
            &REVUE_NON_LUE,
            CIBLE_NON_LUE,
        );
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
        let v = evaluate_flight(
            &f,
            Some(2655),
            &lu(Some("open")),
            &CI_NON_LUE,
            // Approuvée : la `ManualResumption` attendue doit venir du bras
            // `RescuePr`, pas de la garde de revue ajoutée par le vol 2b.
            &revue_approuvee(),
            MilestoneMembership::InTarget,
        );
        assert!(v.hit_ceiling);
        assert!(matches!(v.issue, FlightIssue::RescuePr { .. }));
        assert_eq!(v.recommendation, FlightRecommendation::ManualResumption);
    }

    // ---- mika#2946 : ce qui n'est pas un vol, et ce qui en est un ---------

    /// **AC1 et AC2, dans le même test.** Les trois formes qui ne sont pas des
    /// vols d'implémentation sont classées à part et ne recommandent rien ; les
    /// trois morts réelles restent des morts, avec leur cause et leur
    /// recommandation.
    ///
    /// Le contrôle négatif est **ici**, et pas dans un test voisin, parce que
    /// c'est l'écart qui est la propriété. [`not_a_flight_reason`] est consulté
    /// en tête de [`classify_issue`], donc un prédicat trop large y requalifie
    /// en non-vol des morts que plus personne ne reprendrait à la main — le
    /// défaut de mika#2946 retourné, et un test qui ne regarderait que la
    /// première moitié resterait vert pendant tout ce temps.
    ///
    /// Les trois jetons sont **repris de leurs producteurs**, jamais retapés :
    /// les deux bouts de chacun sont les deux moitiés d'un même fait, et un
    /// littéral recopié ici divergerait en silence le jour où le producteur
    /// bouge (mika#2158). C'est un emprunt de fixture et non un second lecteur :
    /// la décision reste chez [`not_a_flight_reason`] et, pour le groom, chez
    /// [`crate::task_state::tasks::groom_result_convergence`].
    #[test]
    fn mika2946_un_groom_un_refus_et_un_remplacement_ne_sont_pas_des_vols() {
        use crate::task_state::tasks::{
            DISPATCH_NO_GROOMING_MARKER, GROOM_SUCCESS_MARKER, SUPERSEDED_BY_NEW_DISPATCH,
        };

        // --- AC1 : les trois formes, au format de leurs producteurs ---------

        // Un groom convergé : depuis mika#2590 U5a la ligne est posée par
        // `_set_outcome_line`, donc elle est ancrée en début de ligne.
        let groom = format!(
            "Session: d4533d9b\n\
             Turns: 37\n\
             {GROOM_SUCCESS_MARKER} — docs/plans/2026-10-28-002-x-plan.md\n"
        );
        // L'enveloppe que `evaluate_grooming_gate` écrit sur refus. Sa prose
        // parle du grooming de bout en bout — `recovery` comme `reason` — donc
        // une lecture par sous-chaîne y trouverait ce qu'elle cherche dans une
        // phrase qui dit le contraire (classe mika#2050).
        let refus = serde_json::json!({
            "error": DISPATCH_NO_GROOMING_MARKER,
            "task_id": "45dd5b15",
            "issue": "senara-solutions/mika#2943",
            "missing_signals": ["branch_callout"],
            "recovery": "Dispatch dev-groom first so the autonomous loop \
                         produces the canonical callout block.",
            "reason": "Cannot dispatch dev-pilot on ticket #2943: issue body is \
                       missing one or more grooming-marker signals.",
        })
        .to_string();

        for (result, reason) in [
            (groom.as_str(), NotFlightReason::GroomCompleted),
            (refus.as_str(), NotFlightReason::DispatchRefused),
            (SUPERSEDED_BY_NEW_DISPATCH, NotFlightReason::Superseded),
        ] {
            let v = verdict_of(result);
            assert_eq!(
                v.issue,
                FlightIssue::NotAFlight { reason },
                "ce n'est pas un vol : {result:?}"
            );
            assert_eq!(
                v.recommendation,
                FlightRecommendation::Nothing,
                "et le manager ne recommande donc rien : {result:?}"
            );
        }

        // --- AC2 : le contrôle négatif, trois morts qui restent des morts ---

        for (result, cause, recommandation) in [
            (
                "PIPELINE FAILURE: pilot halted\n\
                 Halt class: budget_exhausted — deterministic\n\
                 Commits: 0\n\
                 NO_PR: no_branch_pushed\n\
                 Outcome: PIPELINE_INCOMPLETE — manual recovery needed.\n",
                DeathCause::CeilingReached,
                FlightRecommendation::Regroom,
            ),
            (
                "STRUCTURAL VIOLATION: le plan demande un second lecteur\n\
                 Outcome: PIPELINE_INCOMPLETE — manual recovery needed.\n",
                DeathCause::Refusal,
                FlightRecommendation::Regroom,
            ),
            (
                "PIPELINE FAILURE: no shipping tail\n\
                 Outcome: PIPELINE_INCOMPLETE — manual recovery needed.\n",
                DeathCause::PipelineFailure,
                FlightRecommendation::ManualResumption,
            ),
        ] {
            let v = verdict_of(result);
            assert_eq!(
                v.issue,
                FlightIssue::Dead { cause },
                "une mort reste une mort, avec sa cause : {result:?}"
            );
            assert_eq!(
                v.recommendation, recommandation,
                "et sa recommandation : {result:?}"
            );
        }

        // Et un vol nominal n'est pas davantage requalifié.
        let nominal = evaluate_flight(
            &fixture_pr_opened(),
            None,
            &PrStateKnowledge::NotRead,
            &CI_NON_LUE,
            &REVUE_NON_LUE,
            CIBLE_NON_LUE,
        );
        assert!(
            matches!(nominal.issue, FlightIssue::PrOpened { .. }),
            "une PR ouverte reste une PR ouverte : {:?}",
            nominal.issue
        );
    }

    /// **Non-régression mika#2590 et mika#2050.** Une *mention* du jeton n'est
    /// pas le fait : ni l'enveloppe d'auto-saut qui cite le marqueur de groom, ni
    /// celle dont la prose nomme le jeton de refus, ne sont requalifiées en
    /// non-vol. Les deux restent classées exactement comme avant mika#2946.
    ///
    /// La première est la forme des 651 caractères que mika#2590 a dû refuser ;
    /// la seconde est pourquoi [`envelope_error_is`] lit un **champ** et non une
    /// sous-chaîne. Dans les deux cas, remplacer le lecteur par un `contains`
    /// ferait disparaître une mort du rapport — et sans ce test, en silence.
    #[test]
    fn mika2946_citer_le_jeton_ne_suffit_pas_a_sortir_du_compte() {
        use crate::task_state::tasks::{DISPATCH_NO_GROOMING_MARKER, GROOM_SUCCESS_MARKER};

        let muet = FlightIssue::Dead {
            cause: DeathCause::Unknown("no_outcome_line".to_string()),
        };

        let auto_saut = serde_json::json!({
            "status": "skipped",
            "reason": format!("the issue already carries `{GROOM_SUCCESS_MARKER}` from a prior pass"),
        })
        .to_string();
        assert_eq!(
            verdict_of(&auto_saut).issue,
            muet,
            "le marqueur hors position ne prouve pas un groom (mika#2590)"
        );

        let autre_refus = serde_json::json!({
            "error": "already_groomed",
            "reason": format!("not a {DISPATCH_NO_GROOMING_MARKER} refusal: the gate was never reached"),
        })
        .to_string();
        assert_eq!(
            verdict_of(&autre_refus).issue,
            muet,
            "le jeton se lit dans le champ `error`, jamais dans le corps"
        );
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
        let non_vol = |r: NotFlightReason| FlightIssue::NotAFlight { reason: r };
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
            // mika#2946 — les trois motifs de non-vol, et les deux états de
            // connaissance qui leur sont atteignables. L'état de la PR n'entre
            // pas dans la décision : il n'y a pas de PR, puisqu'il n'y a pas eu
            // de vol. Les trois sont là plutôt qu'un seul parce que la
            // recommandation est portée par la **variante** et non par le
            // motif : un futur motif qui recommanderait autre chose doit se
            // décider ici, pas se glisser dans le bras existant.
            (
                non_vol(NotFlightReason::GroomCompleted),
                lu(None),
                FlightRecommendation::Nothing,
            ),
            (
                non_vol(NotFlightReason::GroomCompleted),
                non_lu(),
                FlightRecommendation::Nothing,
            ),
            (
                non_vol(NotFlightReason::DispatchRefused),
                non_lu(),
                FlightRecommendation::Nothing,
            ),
            (
                non_vol(NotFlightReason::Superseded),
                non_lu(),
                FlightRecommendation::Nothing,
            ),
        ];
        for (issue, pr_state, expected) in cases {
            assert_eq!(
                recommend(issue, pr_state, &CI_NON_LUE),
                *expected,
                "issue={issue:?} pr_state={pr_state:?}"
            );
        }
    }

    /// L'état de PR vient de la **lecture par numéro**, et le sous-ticket
    /// apparié ne le prête plus (mika#2968, vol 2a).
    ///
    /// Ce que mika#2698 tenait : « le même vol, deux milestones : seul l'état
    /// hydraté par le Reader change le verdict ». La prémisse était que
    /// `pr_state` sortait de `SubIssue::pr_state`, donc de l'appariement. Le
    /// vol 2a la remplace : la source est `read_flight_prs`, interrogée par le
    /// numéro que le vol porte lui-même, et l'appariement ne décide plus que
    /// l'appartenance au milestone. Ce test mesure la substitution, pas une
    /// version affaiblie de l'ancienne : l'axe `pr_state` est épinglé sur les
    /// trois états que la lecture peut rendre, et le sous-ticket qui n'apparie
    /// pas ne le change pas.
    #[test]
    fn mika2968_letat_de_pr_vient_de_la_lecture_par_numero() {
        let flights = [fixture_pr_opened()];
        let apparies = [sub_issue(2693, Some("merged"))];

        // La lecture dit « fusionnée » : la reprise est manuelle, quoi que dise
        // le sous-ticket apparié.
        let fusionnee = &evaluate_flights(
            &flights,
            &apparies,
            &lectures(&[(2696, "MERGED", None)]),
            AU_CYCLE_DE,
        )[0];
        assert_eq!(
            fusionnee.recommendation,
            FlightRecommendation::ManualResumption
        );
        assert_eq!(
            fusionnee.pr_state,
            lu(Some("MERGED")),
            "le verdict porte l'état que la lecture par numéro a rendu"
        );
        assert_eq!(
            fusionnee.ci,
            CiKnowledge::Unread(CiUnreadCause::PrNotOpen),
            "sur une PR fermée la CI n'est pas demandée, et le dit"
        );

        // La même lecture, état `OPEN` et CI verte : merge-candidat.
        let ouverte = &evaluate_flights(
            &flights,
            &apparies,
            &lectures(&[(2696, "OPEN", Some(CiState::Success))]),
            AU_CYCLE_DE,
        )[0];
        assert_eq!(ouverte.recommendation, FlightRecommendation::MergeCandidate);
        assert_eq!(ouverte.pr_state, lu(Some("OPEN")));

        // Un sous-ticket qui n'apparie pas ne retire rien à la lecture : c'est
        // exactement ce que mika#2698 faisait dépendre de l'appariement.
        let non_apparie = &evaluate_flights(
            &flights,
            &[sub_issue(9999, Some("merged"))],
            &lectures(&[(2696, "OPEN", Some(CiState::Success))]),
            AU_CYCLE_DE,
        )[0];
        assert_eq!(
            non_apparie.pr_state,
            lu(Some("OPEN")),
            "la lecture par numéro ne passe pas par le milestone"
        );
        assert_eq!(
            non_apparie.in_target_milestone,
            MilestoneMembership::OutOfTarget,
            "l'appariement garde son axe propre : l'appartenance au milestone"
        );
    }

    /// AC5, le contrôle positif à côté du négatif : le **même** vol dit `Read`
    /// quand la PR a été lue, `NotRead` quand aucune lecture n'a été tentée, et
    /// les deux populations ne se confondent pas (classe mika#2205).
    ///
    /// mika#2921 opposait `Read(None)` à `NotRead` ; depuis le vol 2a de
    /// mika#2968, `Read(None)` n'est plus produit par `evaluate_flights` —
    /// trouver le numéro, c'est lire l'état. L'opposition est donc portée par la
    /// porte neuve, et ce qu'elle épingle est le même : deux populations, deux
    /// variantes, jamais un `Option` qui les fusionne.
    #[test]
    fn mika2968_lu_et_non_lu_sont_deux_populations() {
        let flights = [fixture_pr_opened()];
        let lue = &evaluate_flights(
            &flights,
            &[],
            &lectures(&[(2696, "OPEN", Some(CiState::Success))]),
            AU_CYCLE_DE,
        )[0];
        let jamais_lue = &evaluate_flights(&flights, &[], &SANS_LECTEUR, AU_CYCLE_DE)[0];

        assert_eq!(lue.pr_state, lu(Some("OPEN")));
        assert_eq!(jamais_lue.pr_state, PrStateKnowledge::NotRead);
        assert_ne!(
            lue.pr_state, jamais_lue.pr_state,
            "« lu » n'est pas « pas lu » — un Option les aurait fusionnés"
        );
        assert_ne!(
            lue.ci, jamais_lue.ci,
            "et la CI suit la même porte : non tentée se dit, elle ne se devine pas"
        );
        assert_eq!(
            jamais_lue.ci,
            CiKnowledge::Unread(CiUnreadCause::NoRunner),
            "sans lecteur câblé, la cause est « rien n'a été tenté »"
        );
    }

    /// Le dépôt traverse jusqu'au verdict, et vient du vol — jamais de la liste
    /// interrogée (mika#2921).
    #[test]
    fn mika2921_le_verdict_porte_le_depot_du_vol() {
        let v = &evaluate_flights(
            &[fixture_pr_opened()],
            &[],
            &lectures(&[(2696, "OPEN", Some(CiState::Success))]),
            AU_CYCLE_DE,
        )[0];
        assert_eq!(v.repo.as_deref(), Some("senara-solutions/mika"));

        let mut sans_depot = fixture_pr_opened();
        sans_depot.repo = None;
        sans_depot.reference_url = None;
        let v = &evaluate_flights(
            &[sans_depot],
            &[],
            &lectures(&[(2696, "OPEN", Some(CiState::Success))]),
            AU_CYCLE_DE,
        )[0];
        assert_eq!(
            v.repo, None,
            "un vol qui ne nomme aucun dépôt ne s'en voit pas attribuer un"
        );
        assert_eq!(
            v.ci,
            CiKnowledge::Unread(CiUnreadCause::NoRepo),
            "et sans dépôt, aucune page de lecture ne lui correspond : la cause le nomme"
        );
    }

    /// Contrôle négatif de la porte : un vol dont le dépôt n'a pas été lu ne
    /// peut **jamais** sortir avec `CiKnowledge::Read`, quelle que soit la
    /// lecture posée sur un *autre* dépôt (mika#2968, AC2).
    ///
    /// C'est la garde qui sépare « la CI est verte » de « je n'ai pas regardé » :
    /// sans elle, une page lue sur un dépôt voisin pourrait verdir un vol dont
    /// la PR n'a jamais été interrogée.
    #[test]
    fn mika2968_un_depot_non_lu_ne_rend_jamais_une_ci_lue() {
        let mut autre_depot = fixture_pr_opened();
        autre_depot.repo = Some("senara-solutions/mika-cloud".to_string());

        let candidats: [(&str, FlightPrReads); 3] = [
            ("aucun lecteur", SANS_LECTEUR),
            (
                "lecture sur un autre dépôt",
                lectures(&[(2696, "OPEN", Some(CiState::Success))]),
            ),
            ("lecture en échec", lecture_en_echec()),
        ];

        for (cas, reads) in &candidats {
            let v = &evaluate_flights(&[autre_depot.clone()], &[], reads, AU_CYCLE_DE)[0];
            assert!(
                matches!(v.ci, CiKnowledge::Unread(_)),
                "{cas} : la CI d'un dépôt non lu ne peut pas être « lue » (ci={:?})",
                v.ci
            );
            assert_eq!(
                v.pr_state,
                PrStateKnowledge::NotRead,
                "{cas} : et l'état de PR non plus"
            );
        }
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
            milestone_read_audit: None,
        };
        assert_eq!(
            milestone_issue_urls(&state),
            vec![
                "https://github.com/senara-solutions/mika/issues/2693".to_string(),
                "https://github.com/senara-solutions/mika/issues/2661".to_string(),
            ]
        );
    }

    // ---- mika#2968 : la CI entre dans la recommandation ------------------

    /// Le rollup **mesuré** de mika#2966 à 17:46:12Z, réduit au check qui
    /// décide. Pas un littéral inventé : `Egress Manifest Lint` est le job
    /// 123624936407 du run 40924115345, terminé en `failure` à 17:44:16Z — 116 s
    /// avant le verdict de la ligne 805100 d'`audit_events`, qui a tout de même
    /// recommandé « merge-candidat ». Le second check dit que la PR n'était pas
    /// rouge de bout en bout : un seul check suffisait.
    fn rollup_2966_mesure() -> serde_json::Value {
        serde_json::json!([
            { "name": "Egress Manifest Lint", "status": "COMPLETED", "conclusion": "FAILURE" },
            { "name": "Check", "status": "COMPLETED", "conclusion": "SUCCESS" }
        ])
    }

    /// AC2 — les trente cellules dans **un** appel, parce qu'une matrice éclatée
    /// en trente tests ne dit pas qu'elle est exhaustive. Douze viennent du
    /// vol 1 (quatre CI lues × deux états, puis les quatre non-lectures d'alors
    /// sur PR ouverte) ; les dix-huit autres sont les six causes du vol 2a
    /// croisées avec les trois états que la lecture par numéro peut rendre.
    ///
    /// Le cas « rouge et ouverte » — celui du vol 4f1fe163 — ne part pas d'un
    /// `CiState::Failed` écrit à la main : il traverse
    /// [`crate::milestone_manager::reader::classify_ci_state`], le lecteur de
    /// production, depuis le rollup mesuré de mika#2966. Un littéral aurait
    /// attesté la décision sans attester que le rollup réel y mène, et c'est
    /// exactement l'articulation qui manquait.
    #[test]
    fn mika2968_ac2_la_matrice_ci_x_etat_de_pr_decide_la_recommandation() {
        let rouge_mesuree =
            crate::milestone_manager::reader::classify_ci_state(&rollup_2966_mesure());
        assert_eq!(
            rouge_mesuree,
            CiState::Failed,
            "le rollup mesuré de mika#2966 se réduit en CI rouge : si ce n'est \
             plus le cas, la cellule rouge de la matrice ne teste plus rien"
        );

        let pr_du_vol = FlightIssue::PrOpened {
            number: Some(2966),
            url: None,
        };
        let lue = |etat: CiState| CiKnowledge::Read {
            state: etat,
            at: AU_CYCLE_DE.to_string(),
        };

        let cellules: Vec<(&str, PrStateKnowledge, CiKnowledge, FlightRecommendation)> = vec![
            // --- PR encore ouverte : la CI décide ---
            (
                "verte + ouverte",
                lu(Some("open")),
                lue(CiState::Success),
                FlightRecommendation::MergeCandidate,
            ),
            (
                "rouge mesurée (#2966) + ouverte",
                lu(Some("open")),
                lue(rouge_mesuree.clone()),
                FlightRecommendation::ManualResumption,
            ),
            (
                "en cours + ouverte",
                lu(Some("open")),
                lue(CiState::Pending),
                // Le verdict est écrit ~2 min après l'ouverture, pendant que la
                // CI tourne (pour #2966 le run a fini à 17:57:56Z). « En cours »
                // n'est donc pas un refus : c'est l'état nominal au moment du
                // verdict, et le rapport le dit à côté de la recommandation.
                FlightRecommendation::MergeCandidate,
            ),
            (
                "sans check + ouverte",
                lu(Some("open")),
                lue(CiState::None),
                // Une lecture qui a abouti et n'a trouvé aucun check. Rien ne
                // s'oppose au merge — et surtout, ce n'est pas une non-lecture.
                FlightRecommendation::MergeCandidate,
            ),
            // --- PR fusionnée ou fermée : l'état de PR décide, quelle que soit
            //     la CI. Le bras est **avant** le bras CI dans `recommend`.
            (
                "verte + fusionnée",
                lu(Some("merged")),
                lue(CiState::Success),
                FlightRecommendation::ManualResumption,
            ),
            (
                "rouge + fusionnée",
                lu(Some("merged")),
                lue(CiState::Failed),
                FlightRecommendation::ManualResumption,
            ),
            (
                "en cours + fermée",
                lu(Some("closed")),
                lue(CiState::Pending),
                FlightRecommendation::ManualResumption,
            ),
            (
                "sans check + fermée",
                lu(Some("closed")),
                lue(CiState::None),
                FlightRecommendation::ManualResumption,
            ),
            // --- Les quatre non-lectures du vol 1, PR ouverte. Aucune n'est
            //     plus écrite par une composition depuis le vol 2a, mais
            //     `VerdictRecord::ci` désérialise encore les verdicts déjà
            //     écrits : la décision qu'elles portent reste épinglée. Aucune
            //     ne bascule la recommandation : une CI non lue n'est pas une
            //     CI rouge. Elle n'est pas non plus une CI verte — c'est le
            //     rapport qui porte la différence (AC3), jamais la
            //     recommandation seule.
            (
                "non lue (PR hors liste du milestone) + ouverte",
                lu(Some("open")),
                CiKnowledge::Unread(CiUnreadCause::NotFromPrList),
                FlightRecommendation::MergeCandidate,
            ),
            (
                "non lue (aucune sous-issue appariée) + ouverte",
                lu(Some("open")),
                CiKnowledge::Unread(CiUnreadCause::NoSubIssue),
                FlightRecommendation::MergeCandidate,
            ),
            (
                "non lue (vol sans ticket) + ouverte",
                lu(Some("open")),
                CiKnowledge::Unread(CiUnreadCause::NoIssueNumber),
                FlightRecommendation::MergeCandidate,
            ),
            (
                "non lue (non mesurée) + ouverte",
                lu(Some("open")),
                CiKnowledge::Unread(CiUnreadCause::NotMeasured),
                FlightRecommendation::MergeCandidate,
            ),
        ];

        let mut matrice: Vec<(String, PrStateKnowledge, CiKnowledge, FlightRecommendation)> =
            cellules
                .into_iter()
                .map(|(nom, pr_state, ci, attendu)| (nom.to_string(), pr_state, ci, attendu))
                .collect();

        // --- Vol 2a : les six causes que `compose_pr_knowledge` peut produire,
        //     croisées avec les trois états de PR que la lecture peut rendre.
        //     `recommend` est une fonction pure de son triplet, donc la matrice
        //     couvre son domaine entier — y compris les cellules que la
        //     production n'atteint pas (`NoPrNumber` ne cohabite jamais avec un
        //     état lu). S'arrêter aux combinaisons atteignables n'attesterait
        //     pas que le bras d'état reste **avant** le bras CI : c'est
        //     justement sur une non-lecture de CI que l'ordre se mesure.
        let causes_du_vol_2a = [
            ("vol sans numéro de PR", CiUnreadCause::NoPrNumber),
            ("vol sans dépôt", CiUnreadCause::NoRepo),
            ("lecture non tentée", CiUnreadCause::NoRunner),
            ("lecture en échec", CiUnreadCause::ReadFailed),
            ("numéro absent de la page", CiUnreadCause::NotFound),
            ("PR non ouverte", CiUnreadCause::PrNotOpen),
        ];
        let etats_de_pr = [
            (
                "ouverte",
                lu(Some("open")),
                FlightRecommendation::MergeCandidate,
            ),
            (
                "fusionnée",
                lu(Some("merged")),
                FlightRecommendation::ManualResumption,
            ),
            (
                "état non lu",
                PrStateKnowledge::NotRead,
                FlightRecommendation::MergeCandidate,
            ),
        ];
        for (nom_cause, cause) in &causes_du_vol_2a {
            for (nom_etat, pr_state, attendu) in &etats_de_pr {
                matrice.push((
                    format!("non lue ({nom_cause}) + {nom_etat}"),
                    pr_state.clone(),
                    CiKnowledge::Unread(*cause),
                    *attendu,
                ));
            }
        }

        assert_eq!(
            matrice.len(),
            30,
            "la matrice de l'AC2 compte trente cellules : les douze du vol 1, \
             plus les six causes du vol 2a croisées avec les trois états de PR"
        );

        for (nom, pr_state, ci, attendu) in &matrice {
            assert_eq!(
                &recommend(&pr_du_vol, pr_state, ci),
                attendu,
                "cellule « {nom} »"
            );
        }
    }

    /// Le contrôle négatif de la cellule rouge : **même** vol, **même** état de
    /// PR, seule la couleur change. Sans lui, la matrice resterait verte le jour
    /// où `recommend` rendrait `ManualResumption` pour tout vol `PrOpened` —
    /// elle attesterait alors la panne d'un axe, pas sa lecture.
    #[test]
    fn mika2968_ac1_seule_la_couleur_separe_les_deux_recommandations() {
        let vol = FlightIssue::PrOpened {
            number: Some(2966),
            url: None,
        };
        let ouverte = lu(Some("open"));
        let ci = |etat: CiState| CiKnowledge::Read {
            state: etat,
            at: AU_CYCLE_DE.to_string(),
        };

        assert_eq!(
            recommend(&vol, &ouverte, &ci(CiState::Failed)),
            FlightRecommendation::ManualResumption
        );
        assert_eq!(
            recommend(&vol, &ouverte, &ci(CiState::Success)),
            FlightRecommendation::MergeCandidate
        );
    }

    /// Un blob de verdict antérieur au champ `ci` se relit en « non mesurée »,
    /// jamais en CI verte (mika#2331 : l'absence n'est pas zéro). C'est le
    /// défaut de production qui est éprouvé ici — le seul endroit du module où
    /// `default_unread` est appelé par un test plutôt que contourné.
    #[test]
    fn mika2968_un_verdict_anterieur_au_champ_se_relit_en_non_mesuree() {
        let ancien = r#"{
            "task_id": "4f1fe163-a7a2-442d-ba70-56e343470dba",
            "agent_id": "mika-dev",
            "issue_number": 2963,
            "issue": { "pr_opened": { "number": 2966, "url": null } },
            "turns_used": 83,
            "hit_ceiling": false,
            "turn_ceiling": 200,
            "last_commit_turn": null,
            "pr_state": "not_read",
            "recommendation": "merge_candidate",
            "evidence": []
        }"#;
        let v: FlightVerdict = serde_json::from_str(ancien).expect("un blob antérieur se relit");
        assert_eq!(v.ci, CiKnowledge::Unread(CiUnreadCause::NotMeasured));
        assert_eq!(v.ci, CiKnowledge::default_unread());
    }
}

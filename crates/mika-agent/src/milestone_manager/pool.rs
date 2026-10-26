//! Pool — le bassin `ready` : sa profondeur, sa bande, et qui devrait y entrer.
//!
//! **LECTURE SEULE**, et ce module le reste au vol 4. mika#2699 Phase 2 (a) :
//! le manager *voit* le bassin et *dit* qui devrait y entrer, et pourquoi il
//! refuse les autres. Il ne pose aucun label, ne rattache aucun milestone, ne
//! déclare aucun interrupteur. `assess_pool` est une fonction pure sans E/S.
//! Depuis le vol 4 le verdict a une seconde destination, et une seule :
//! [`super::pool_writer`] — voir § *Ce que ce module ne fait pas* en bas.
//!
//! # Le bassin n'est pas une propriété du milestone cible
//!
//! Le webhook `ready` dispatche à l'échelle du **dépôt**, pas du milestone. Un
//! bassin compté sur les seules `sub_issues` du milestone serait sous-compté, et
//! la recommandation fausse exactement le jour où elle compte. Mesuré le
//! 2026-10-15 : 2 issues `ready` ouvertes (mika#2699, mika#2687), **les deux**
//! portant `dispatch:loop` — profondeur disponible 0, bande *famine*. Un bassin
//! lu sur le seul milestone aurait affiché « 2, nominal ».
//!
//! # Deux ensembles de candidats, et pourquoi le rang de provenance ne vient pas
//! en premier
//!
//! Le vol 2 ajoute les candidats **hors milestone cible**
//! ([`CandidateOrigin::Outside`]), parce que le ticket demande un ordre à deux
//! critères : « priorité (p0, puis p1, puis p2) ; dans le milestone cible, avant
//! le reste ». L'ordre des deux critères dans cette phrase est la décision :
//! **la priorité domine la provenance**. Un p0 hors milestone passe donc devant
//! un p1 du milestone, et la provenance ne tranche qu'à priorité égale. Lire
//! l'inverse — milestone d'abord, priorité ensuite — ferait tenir un p0 critique
//! derrière un p2 de confort, ce que l'orchestrateur ne fait pas à la main.
//!
//! # Le second motif de refus mesure, et il refuse — là où le bash ne refuse pas
//!
//! Le vol 3 ajoute [`RefusalReason::EstimateOverFlightBound`] : un candidat dont
//! le plan annonce plus de lignes que la borne par vol de **son** plafond de
//! tours n'est pas recommandé. La taille vient du plan lu au ref de la branche
//! du candidat, et le contrat de lecture est celui de [`super::plan_size`].
//!
//! Le gate 3 de `dispatch-lib.sh` fait le même calcul et **ne refuse rien**, ce
//! qui n'est pas une incohérence mais la conséquence de l'endroit où chacun se
//! tient. Le bash juge au **dispatch**, sur une file déjà groomée dont aucun
//! plan ne repasse par une porte de groom : y refuser gèlerait d'un coup tout
//! `docs/plans/`. Le manager juge à l'**admission**, en amont, sur un ticket qui
//! n'est pas encore `ready` — le refus y est réversible d'un découpage de plan,
//! et le ticket demande explicitement qu'il ait lieu (« n'est jamais
//! recommandé. Le rapport dit pourquoi »).
//!
//! **Seul un total mesuré strictement au-dessus de la borne refuse.** Un plan
//! sans section de taille, un total illisible, un plan non lu : aucun n'est une
//! estimation « au-delà », donc aucun n'est ce refus-là. Ils ne sont pas non
//! plus tus — [`PlanSizeReading`] les distingue et le rapport les nomme, parce
//! qu'admettre en silence sous une taille jamais mesurée serait présenter une
//! absence de mesure comme un `under_bound` (*un `null` n'est jamais un `0`*,
//! mika#2331).
//!
//! # Le troisième motif : un ticket que l'opérateur tient n'est pas admissible
//!
//! mika#2923 ajoute [`RefusalReason::OperatorHeld`] : un candidat portant l'un
//! des labels de [`crate::webhook_dispatch::OPERATOR_HELD_LABELS`]
//! (`blocked`, `operator-review`, `operator-gated`) est refusé, et le refus
//! **nomme lequel** — le geste correctif est de retirer *ce* label-là.
//!
//! Mesuré le 2026-10-26 : le rapport du sink recommandait d'admettre #2698,
//! #2699 et #1806, tous trois `operator-gated`. Le prédicat d'admission ne
//! lisait aucun de ces labels, donc le jour où `MIKA_MANAGER_POOL_WRITE`
//! s'ouvrirait, le poseur aurait posé `ready` sur un ticket que l'opérateur
//! retient — exactement l'autorité que l'interrupteur devait borner.
//!
//! Les deux littéraux ne sont pas retapés ici : la liste est celle de
//! mika#2263, déjà « la liste unique derrière deux surfaces » (le feeder
//! d'`auto_pull` et la porte du `ready_label_handler`). Le bassin est la
//! troisième surface qui pose la même question, et une quatrième orthographe de
//! la liste serait la divergence que mika#2263 a déjà payée une fois.
//!
//! # Ce que ce module ne fait pas
//!
//! L'écriture. Le vol 4 l'a livrée, et délibérément **pas ici** : `assess_pool`
//! doit rester pure pour que le verdict se teste sans exécutant, et le grep plat
//! de `no_dispatch_test.rs` doit pouvoir nommer **un seul** fichier en
//! exception. Le geste vit donc dans [`super::pool_writer`] —
//! `MIKA_MANAGER_POOL_WRITE`, OFF par défaut, et les deux mutations en un seul
//! `gh issue edit` : poser `ready` et rattacher au milestone cible, jamais un
//! retrait. L'exception nommée à `no_dispatch_test.rs` est arrivée dans le même
//! commit que ce geste, au fichier et au jeton près, jamais comme un
//! relâchement du gabarit.
//!
//! Ce module reste pourtant le lieu de la **décision**, et c'est ce qui tient le
//! poseur court : il n'écrit que ce que [`PoolVerdict::admit`] nomme. Les deux
//! motifs de refus des vols 2 et 3 protègent donc le dépôt une fois
//! l'interrupteur ouvert, sans qu'une ligne du poseur les reprenne.
//!
//! Reste au **vol 5** : la seconde moitié de l'AC3 — l'idempotence inter-cycle
//! épinglée par un test de verdict rejoué, là où elle n'est aujourd'hui qu'une
//! propriété du filtre de candidats ci-dessous — et le rapport nommant ce qui a
//! été réellement posé. Suivi sur `senara-solutions/mika#2699`.

use super::plan_size;
use super::types::{IssueState, SubIssue};
use crate::webhook_dispatch::{DISPATCH_SEAT_LABEL_PREFIX, READY_LABEL, operator_held_label};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

/// Ce qu'il faut savoir d'un ticket pour décider s'il entre au bassin — et rien
/// de plus.
///
/// Pourquoi pas [`SubIssue`] : cette structure porte douze champs, dont la PR,
/// l'état CI et la présence de branche. La lecture dépôt-large des candidats
/// hors milestone (`gh issue list`) **n'en mesure aucun**, et les remplir à leur
/// valeur neutre ferait dire « pas de CI » à une absence de mesure — le motif
/// mika#2331 que ce dépôt refuse ailleurs (« un null n'est jamais un 0 »). Les
/// cinq champs ci-dessous sont exactement ceux que `assess_pool` consulte, donc
/// les deux ensembles de candidats partagent un type sans qu'aucun ait à
/// inventer de valeur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolCandidate {
    pub number: u64,
    pub state: IssueState,
    pub priority_rank: Option<u8>,
    pub plan_present: bool,
    pub labels: Vec<String>,
}

impl From<&SubIssue> for PoolCandidate {
    /// Une `sub_issue` du milestone cible vue comme candidat au bassin. La
    /// projection est sans perte pour ce que `assess_pool` décide : elle laisse
    /// tomber les champs qu'aucune règle d'admission ne lit.
    fn from(s: &SubIssue) -> Self {
        PoolCandidate {
            number: s.number,
            state: s.state.clone(),
            priority_rank: s.priority_rank,
            plan_present: s.plan_present,
            labels: s.labels.clone(),
        }
    }
}

/// Plancher du bassin : sous cette profondeur **disponible**, le rail risque de
/// se vider et le manager recommande une admission.
pub const POOL_MIN: usize = 2;

/// Plafond du bassin : la cible à atteindre quand on admet. Au-delà, saturé.
pub const POOL_MAX: usize = 4;

/// Un ticket `ready` ouvert quelque part dans le dépôt, vu depuis la forge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadyTicket {
    pub number: u64,
    pub labels: Vec<String>,
}

impl ReadyTicket {
    /// Ce ticket porte-t-il un siège de dispatch — autrement dit, un vol est-il
    /// en cours dessus ?
    ///
    /// Le prédicat est sur la **famille** `dispatch:`, jamais sur
    /// `KNOWN_DISPATCH_SEATS` : un siège neuf ne doit pas rendre le bassin
    /// optimiste le jour de son introduction. Insensible à la casse pour la même
    /// raison — `Dispatch:SSC` réclame le même siège que `dispatch:ssc`
    /// (`webhook_dispatch::classify_dispatch_seat` le lit ainsi), et un vol lu
    /// comme du rail disponible est une admission de trop.
    pub fn holds_dispatch_seat(&self) -> bool {
        labels_hold_dispatch_seat(&self.labels)
    }
}

/// D'où vient un candidat — le second critère de tri du point 1 du ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateOrigin {
    /// Une `sub_issue` du milestone cible.
    Milestone,
    /// Une issue ouverte du dépôt qui n'appartient pas au milestone cible.
    Outside,
}

impl CandidateOrigin {
    /// Rang de provenance dans la clé de tri — `Milestone` avant `Outside`, et
    /// **après** la priorité (voir le docstring du module).
    fn rank(self) -> u8 {
        match self {
            CandidateOrigin::Milestone => 0,
            CandidateOrigin::Outside => 1,
        }
    }
}

/// Profondeur du bassin à l'instant du cycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolDepth {
    /// Tickets portant `ready`, ouverts, à l'échelle du dépôt.
    pub ready_total: usize,
    /// … dont un siège `dispatch:<seat>` est posé.
    pub in_flight: usize,
    /// `ready_total − in_flight` — le seul chiffre qui décide la bande.
    pub available: usize,
}

/// Où se situe la profondeur **disponible** vis-à-vis de [`POOL_MIN`]/[`POOL_MAX`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoolBand {
    /// Sous le plancher : le rail risque de se vider.
    Starved,
    /// Entre le plancher et le plafond, bornes incluses — rien à dire.
    Nominal,
    /// Au-dessus du plafond.
    Saturated,
}

/// Nombre maximal de plans lus à la forge par cycle (mika#2699 vol 3).
///
/// Chaque lecture est un appel `gh api` de plus, et la liste des candidats
/// recevables peut compter quelques dizaines de tickets sur ce dépôt : sans
/// borne, le coût du cycle suivrait la taille du dépôt plutôt que celle de la
/// décision. La borne porte sur les candidats **dans l'ordre du bassin**, donc
/// elle mesure d'abord ceux qui peuvent entrer.
///
/// `3 × POOL_MAX`, et pas `POOL_MAX` : une admission refusée pour dépassement
/// laisse la place à la suivante, donc mesurer quatre candidats pour quatre
/// places rendrait un bassin plus creux que nécessaire le jour où deux d'entre
/// eux dépassent. Au-delà, la taille est *non mesurée* et le rapport le dit —
/// jamais « sous la borne » par défaut.
pub const PLAN_SIZE_READ_MAX: usize = 3 * POOL_MAX;

/// Ce que le manager sait de la taille estimée d'un candidat.
///
/// Les trois premières variantes sont les trois jetons de
/// [`plan_size::PlanSizeTotal`] ; la quatrième est la cause que le bash n'a pas,
/// parce que là-bas le plan est un fichier local déjà validé et qu'ici c'est une
/// lecture distante. Les replier sur `absent` ferait lire « ce plan n'est pas
/// dimensionné » là où le fait est « nous n'avons pas réussi à le lire » — deux
/// remèdes opposés sous un seul mot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanSizeReading {
    /// Le total annoncé par la section `## Taille estimée` du plan.
    Measured(u64),
    /// Le plan a été lu ; il ne porte pas de section de taille.
    SectionAbsent,
    /// La section est là, sa ligne `Total estimé :` non.
    Unparsable,
    /// Le plan n'a pas été lu — et la cause est dite.
    Unread(UnreadCause),
}

/// Pourquoi le plan d'un candidat n'a pas été lu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnreadCause {
    /// Le corps ne porte pas, sous leur forme stricte, le callout `Plan:` et le
    /// callout `Branch:` qu'il faut pour savoir *quel fichier* lire *où*.
    ///
    /// Distinct de [`RefusalReason::NoGroomedPlan`], et pas par excès de
    /// prudence : `plan_present` vient d'un lecteur **lâche** (quatrième
    /// tolérance de [`crate::plan_callout`]), donc un corps peut passer le
    /// prédicat d'admission sans livrer de chemin au lecteur strict.
    NoCallout,
    /// La forge n'a pas rendu le plan à ce ref — branche disparue, plan absent
    /// de la branche, appel en échec.
    FetchFailed,
    /// La lecture n'a pas été tentée : candidat au-delà de
    /// [`PLAN_SIZE_READ_MAX`], ou verdict d'un cycle antérieur au champ.
    NotMeasured,
}

impl PlanSizeReading {
    /// Le défaut de désérialisation : *pas mesuré*, jamais un nombre.
    fn default_unread() -> Self {
        PlanSizeReading::Unread(UnreadCause::NotMeasured)
    }

    /// Le motif, pour l'opérateur, quand la taille n'est **pas** un nombre.
    ///
    /// `None` sur [`Self::Measured`] : une taille mesurée sous la borne n'a rien
    /// à signaler, et une ligne par admission conforme serait du bruit qui finit
    /// par cacher celle qui compte.
    pub fn unmeasured_label(self) -> Option<&'static str> {
        match self {
            PlanSizeReading::Measured(_) => None,
            PlanSizeReading::SectionAbsent => Some("section « Taille estimée » absente du plan"),
            PlanSizeReading::Unparsable => Some("total du plan illisible"),
            PlanSizeReading::Unread(UnreadCause::NoCallout) => {
                Some("callout de plan ou de branche absent")
            }
            PlanSizeReading::Unread(UnreadCause::FetchFailed) => {
                Some("plan introuvable au ref de sa branche")
            }
            PlanSizeReading::Unread(UnreadCause::NotMeasured) => Some("taille non mesurée"),
        }
    }
}

/// Un ticket que le manager recommande de faire entrer dans le bassin.
///
/// Porte sa provenance parce que le rapport doit la dire : « #2708 (p2-normal,
/// hors milestone) » est une recommandation qu'un opérateur peut arbitrer,
/// « #2708 » seul ne l'est pas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Admission {
    pub number: u64,
    pub origin: CandidateOrigin,
    /// Ce que vaut sa taille estimée — porté sur l'admission, pas seulement sur
    /// le refus (mika#2699 vol 3).
    ///
    /// Sans ce champ, « admis » couvrirait indifféremment « mesuré sous la
    /// borne » et « jamais mesuré », et le second se lirait comme le premier.
    /// `#[serde(default)]` pour la raison des autres champs du vol : le puits
    /// hors-ligne tient des blobs écrits avant lui.
    #[serde(default = "PlanSizeReading::default_unread")]
    pub size: PlanSizeReading,
}

/// Pourquoi un candidat n'est pas recommandé.
///
/// Un `enum` plutôt qu'un message, et le vol 3 est ce que cette forme achetait :
/// le second motif du point 2 du ticket arrive comme un **variant**, pas comme
/// une refonte du rapport. mika#2923 en est la confirmation : le troisième
/// motif arrive de la même façon.
///
/// **`Clone` sans `Copy`**, depuis mika#2923 : [`RefusalReason::OperatorHeld`]
/// porte une `String` parce que ce type est `Serialize + Deserialize` (le
/// verdict va dans le puits hors-ligne du sink) et qu'un `&'static str` ne se
/// désérialise pas. Précédent exact une porte plus loin :
/// `auto_pull::RefusalReason` est `Clone` sans `Copy` pour la même raison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalReason {
    /// Aucun callout `Plan: docs/plans/…` dans le corps du ticket.
    NoGroomedPlan,
    /// Le plan annonce plus de lignes que la borne par vol de ce ticket, et ne
    /// se découpe pas — second motif du point 2 (mika#2699 vol 3).
    ///
    /// Les deux nombres voyagent avec le motif : « au-delà de la borne » sans
    /// eux ne dit pas de combien découper, et le geste correctif est
    /// précisément un découpage chiffré.
    EstimateOverFlightBound { total_loc: u64, bound_loc: u32 },
    /// Le candidat porte un label par lequel l'opérateur — ou une autre
    /// instance — le tient (mika#2923).
    ///
    /// `label` dit **lequel**, pour la raison que porte déjà
    /// [`crate::webhook_dispatch::operator_held_label`] : le geste correctif est
    /// de retirer *ce* label-là, et un compteur doit pouvoir distinguer « tenu
    /// par `blocked` » de « tenu par `operator-gated` ».
    OperatorHeld { label: String },
}

impl RefusalReason {
    /// Le motif, dit à l'opérateur. Vit ici et non dans le rapport pour qu'un
    /// variant neuf ne puisse pas être rendu sans libellé — y compris quand le
    /// libellé porte des nombres, auquel cas il est **composé ici** plutôt
    /// qu'épelé une seconde fois par le rapporteur.
    pub fn label(&self) -> String {
        match self {
            RefusalReason::NoGroomedPlan => "plan groomé absent".to_string(),
            RefusalReason::EstimateOverFlightBound {
                total_loc,
                bound_loc,
            } => format!(
                "estimation {total_loc} lignes au-delà de la borne par vol ({bound_loc}) \
                 — à découper"
            ),
            RefusalReason::OperatorHeld { label } => {
                format!("tenu par l'opérateur — label `{label}` à retirer")
            }
        }
    }

    /// Ce refus se corrige-t-il par un **grooming** ?
    ///
    /// `match` exhaustif, sans bras `_` : un variant neuf ne compile pas sans
    /// que son auteur ait répondu. C'est ce qui empêche la relève de proposer
    /// un travail que rien ne débloque — « à grommer : tenu par l'opérateur »
    /// serait un faux conseil dans le rapport, et le lecteur agirait dessus.
    pub fn remediable_by_grooming(&self) -> bool {
        match self {
            RefusalReason::NoGroomedPlan => true,
            // Le geste correctif est un découpage chiffré du plan, donc un
            // re-groom : comportement d'aujourd'hui, inchangé.
            RefusalReason::EstimateOverFlightBound { .. } => true,
            RefusalReason::OperatorHeld { .. } => false,
        }
    }
}

/// Un candidat refusé à l'admission, **nommé** — point 2 du ticket.
///
/// Le vol 1 rendait un compte (« Écartés : 1 candidat sans plan groomé »), ce
/// qui disait à l'opérateur qu'un filtre avait eu lieu sans lui dire sur quoi
/// agir. Nommer le ticket est ce qui rend le refus actionnable : le geste
/// correctif est de groomer *ce* ticket-là.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionRefusal {
    pub number: u64,
    pub origin: CandidateOrigin,
    pub reason: RefusalReason,
}

/// Le verdict du bassin pour ce cycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolVerdict {
    pub depth: PoolDepth,
    pub band: PoolBand,
    /// Tickets à faire entrer, dans l'ordre d'admission. Vide hors famine.
    pub admit: Vec<Admission>,
    /// Candidats écartés, nommés, avec leur motif. Rempli **même hors famine** :
    /// un ticket mal formé reste mal formé quand le bassin est plein, et c'est
    /// le moment utile pour le dire — avant que la famine le réclame.
    #[serde(default)]
    pub refusals: Vec<AdmissionRefusal>,
}

/// La clé de tri du bassin : `(priorité, provenance, numéro)` — **site unique**
/// du dépôt (mika#2700 Phase 2 (c) vol 1).
///
/// Extraite parce qu'un second lecteur est arrivé : la relève nommée doit
/// présenter les prochains tickets à grommer *dans l'ordre du bassin*, et une
/// deuxième écriture du triplet est une divergence silencieuse — les deux
/// listes se contrediraient dans le même rapport sans qu'aucune assertion de
/// comportement rougisse (doctrine du lecteur unique, mika#2201).
///
/// `None` vaut `u8::MAX`, donc **en dernier** : l'absence de label de priorité
/// n'est pas p0. La priorité domine la provenance — voir le docstring du module
/// pour pourquoi l'ordre des deux critères est la décision et non un détail.
pub fn pool_sort_key(
    priority_rank: Option<u8>,
    origin: CandidateOrigin,
    number: u64,
) -> (u8, u8, u64) {
    (priority_rank.unwrap_or(u8::MAX), origin.rank(), number)
}

/// Juge le bassin et nomme les admissions comme les refus — fonction **pure**,
/// sans E/S.
///
/// Cinq décisions, chacune pour une raison nommable :
///
/// 1. **C'est `available` qui décide la bande**, pas `ready_total`. Un ticket
///    `ready` déjà en vol n'est pas du rail disponible.
/// 2. **Hystérésis.** On ne recommande une admission que **sous** [`POOL_MIN`],
///    et on recommande alors de remonter à [`POOL_MAX`]. Sans elle, à 3
///    disponibles le manager recommanderait un ticket à chaque cycle jusqu'à 4,
///    puis à nouveau dès qu'un vol démarre : un geste par cycle, soit exactement
///    le bruit que l'orchestrateur évite à la main.
/// 3. **Le tri passe par [`pool_sort_key`]**, son site unique depuis
///    mika#2700. La priorité domine la provenance — voir le docstring du module
///    pour pourquoi l'ordre des deux critères est la décision et non un détail.
///    Un ticket sans label de priorité passe **en dernier** : l'absence de
///    priorité n'est pas p0.
/// 4. **Un même numéro n'est jugé qu'une fois.** Les deux ensembles sont pris à
///    deux appels `gh` distincts et se recoupent par construction (une
///    `sub_issue` du milestone est aussi une issue ouverte du dépôt). Le premier
///    vu gagne, et les candidats du milestone sont parcourus d'abord — donc un
///    ticket du milestone n'est jamais étiqueté `Outside`.
/// 5. **Les refus sont rendus hors famine aussi**, et triés par la même clé,
///    priorité absente, pour que le rapport soit stable d'un cycle à l'autre
///    quand rien n'a changé.
/// 6. **Une taille non mesurée n'admet pas en silence et ne refuse pas non
///    plus** (vol 3). Seul un total strictement au-dessus de la borne refuse ;
///    les trois autres lectures passent, en portant leur cause sur l'admission
///    pour que le rapport la dise. Voir le docstring du module.
///
/// `sizes` est indexé par numéro d'issue ; un candidat absent de la table vaut
/// [`UnreadCause::NotMeasured`]. C'est l'appelant qui décide combien de plans il
/// va chercher ([`numbers_needing_size`]) — cette fonction reste pure.
pub fn assess_pool(
    pool: &[ReadyTicket],
    milestone_candidates: &[PoolCandidate],
    outside_candidates: &[PoolCandidate],
    sizes: &BTreeMap<u64, PlanSizeReading>,
) -> PoolVerdict {
    let ready_total = pool.len();
    let in_flight = pool.iter().filter(|t| t.holds_dispatch_seat()).count();
    let available = ready_total.saturating_sub(in_flight);

    let band = if available < POOL_MIN {
        PoolBand::Starved
    } else if available <= POOL_MAX {
        PoolBand::Nominal
    } else {
        PoolBand::Saturated
    };

    let (eligible, mut refusals) =
        well_formed_candidates(pool, milestone_candidates, outside_candidates);

    // Le second motif, appliqué dans l'ordre du bassin : un candidat dont le
    // plan annonce plus que la borne par vol de SON plafond quitte la liste des
    // recevables pour celle des refusés. La borne est par ticket et non de
    // flotte — un `loop-substrate` tient 429 lignes là où un ticket nu en tient
    // 279, et les comparer à la même borne refuserait le premier à tort.
    let mut admissible: Vec<(&PoolCandidate, CandidateOrigin, PlanSizeReading)> = Vec::new();
    for (s, origin) in eligible {
        let bound = plan_size::per_flight_bound(plan_size::turn_ceiling_for_labels(&s.labels));
        let size = sizes
            .get(&s.number)
            .copied()
            .unwrap_or(PlanSizeReading::Unread(UnreadCause::NotMeasured));
        match size {
            // `>` et non `>=` : un plan qui tient exactement la borne la tient.
            PlanSizeReading::Measured(total) if total > u64::from(bound) => {
                refusals.push(AdmissionRefusal {
                    number: s.number,
                    origin,
                    reason: RefusalReason::EstimateOverFlightBound {
                        total_loc: total,
                        bound_loc: bound,
                    },
                });
            }
            _ => admissible.push((s, origin, size)),
        }
    }

    // Un `AdmissionRefusal` ne porte pas de rang de priorité — la décision de
    // refus ne le consulte pas — donc il entre dans la clé en **absent**. Le
    // premier terme est alors constant sur toute la liste et l'ordre obtenu est
    // exactement l'ancien `(origin.rank(), number)` : les tests du bassin sont
    // le contrôle de non-régression de cette égalité. Le tri a lieu **après**
    // les refus du vol 3, sans quoi ils s'empileraient derrière ceux du vol 2
    // dans l'ordre d'arrivée.
    refusals.sort_by_key(|r| pool_sort_key(None, r.origin, r.number));

    // `POOL_MAX - available` ne peut pas déborder : la bande famine impose
    // `available < POOL_MIN`, et `POOL_MIN <= POOL_MAX`.
    let admit: Vec<Admission> = match band {
        PoolBand::Starved => admissible
            .iter()
            .take(POOL_MAX - available)
            .map(|(s, origin, size)| Admission {
                number: s.number,
                origin: *origin,
                size: *size,
            })
            .collect(),
        PoolBand::Nominal | PoolBand::Saturated => Vec::new(),
    };

    PoolVerdict {
        depth: PoolDepth {
            ready_total,
            in_flight,
            available,
        },
        band,
        admit,
        refusals,
    }
}

/// Les candidats dont le plan mérite une lecture à la forge, dans l'ordre du
/// bassin et bornés par [`PLAN_SIZE_READ_MAX`] (mika#2699 vol 3).
///
/// Rendue **publique et séparée** parce que l'appelant qui paie les appels `gh`
/// n'est pas celui qui juge : [`super::reader`] va chercher les plans,
/// [`assess_pool`] reste pure. Les deux passent par
/// [`well_formed_candidates`], donc ils ne peuvent pas diverger sur qui est un
/// candidat — une seconde écriture du filtre serait deux réponses à la même
/// question dans le même cycle.
///
/// La liste est bornée **après** le tri, jamais avant : borner d'abord
/// mesurerait les plus petits numéros plutôt que les plus prioritaires.
pub fn numbers_needing_size(
    pool: &[ReadyTicket],
    milestone_candidates: &[PoolCandidate],
    outside_candidates: &[PoolCandidate],
) -> Vec<u64> {
    let (eligible, _) = well_formed_candidates(pool, milestone_candidates, outside_candidates);
    eligible
        .iter()
        .take(PLAN_SIZE_READ_MAX)
        .map(|(s, _)| s.number)
        .collect()
}

/// Les candidats recevables **sur la forme du ticket seule**, triés, et les
/// refus que cette seule lecture suffit à nommer.
///
/// Site unique du filtre d'admission, partagé par [`assess_pool`] et
/// [`numbers_needing_size`]. Ce qui est décidé ici ne consulte aucune lecture
/// distante : c'est précisément ce qui permet d'appeler cette fonction *avant*
/// d'aller chercher les plans.
fn well_formed_candidates<'a>(
    pool: &[ReadyTicket],
    milestone_candidates: &'a [PoolCandidate],
    outside_candidates: &'a [PoolCandidate],
) -> (
    Vec<(&'a PoolCandidate, CandidateOrigin)>,
    Vec<AdmissionRefusal>,
) {
    // Deux lectures distinctes peuvent dire « ce ticket est déjà `ready` » : le
    // bassin dépôt-large et les labels de la sub-issue. Elles sont prises à deux
    // appels `gh` différents, donc elles peuvent diverger d'un cycle ; on écarte
    // sur l'union, parce qu'une admission en double est pire qu'une admission
    // tardive.
    let already_ready: HashSet<u64> = pool.iter().map(|t| t.number).collect();

    let mut eligible: Vec<(&PoolCandidate, CandidateOrigin)> = Vec::new();
    let mut refusals: Vec<AdmissionRefusal> = Vec::new();
    // Décision 4 : le milestone est parcouru d'abord, donc un ticket présent
    // dans les deux ensembles est jugé `Milestone` et le dehors le saute.
    let mut seen: HashSet<u64> = HashSet::new();
    for (set, origin) in [
        (milestone_candidates, CandidateOrigin::Milestone),
        (outside_candidates, CandidateOrigin::Outside),
    ] {
        for s in set {
            if !seen.insert(s.number) {
                continue;
            }
            if s.state != IssueState::Open {
                continue;
            }
            if already_ready.contains(&s.number) || labels_carry_ready(&s.labels) {
                continue;
            }
            if labels_hold_dispatch_seat(&s.labels) {
                continue;
            }
            // À partir d'ici le candidat est recevable *en principe* : ouvert,
            // pas déjà dans le bassin, pas en vol. Ce qui le refuse maintenant
            // est un défaut de forme, donc un refus qui se **nomme** — les
            // filtres ci-dessus, eux, ne sont pas des reproches : un ticket
            // déjà `ready` n'a rien à corriger.
            //
            // Premier des refus nommés, et l'ordre des deux est la priorité du
            // motif : `AdmissionRefusal` n'en porte qu'un. Un ticket que
            // l'opérateur tient n'a pas besoin d'un plan — le geste correctif
            // est de retirer le label — donc le nommer `NoGroomedPlan` le
            // ferait entrer dans la relève à grommer (`relief::name_relief`) et
            // proposerait un travail que rien ne débloque.
            //
            // Après les filtres silencieux, pas avant : un ticket déjà `ready`
            // *et* tenu est une anomalie réelle, mais elle n'appartient pas à
            // l'admission — poser `ready` n'y est plus en jeu, et cette
            // population est déjà la charge de la porte du
            // `ready_label_handler` (mika#2263).
            if let Some(held) = operator_held_label(s.labels.iter().map(String::as_str)) {
                refusals.push(AdmissionRefusal {
                    number: s.number,
                    origin,
                    reason: RefusalReason::OperatorHeld {
                        label: held.to_string(),
                    },
                });
                continue;
            }
            if !s.plan_present {
                refusals.push(AdmissionRefusal {
                    number: s.number,
                    origin,
                    reason: RefusalReason::NoGroomedPlan,
                });
                continue;
            }
            eligible.push((s, origin));
        }
    }
    eligible.sort_by_key(|(s, origin)| pool_sort_key(s.priority_rank, *origin, s.number));
    (eligible, refusals)
}

/// Un label de la famille `dispatch:` est-il posé ? Voir
/// [`ReadyTicket::holds_dispatch_seat`] pour le pourquoi de la famille et de la
/// casse.
fn labels_hold_dispatch_seat(labels: &[String]) -> bool {
    labels.iter().any(|l| {
        l.trim()
            .to_ascii_lowercase()
            .starts_with(DISPATCH_SEAT_LABEL_PREFIX)
    })
}

/// Le label `ready` est-il posé ? Casse ignorée, comme le fait la forge.
fn labels_carry_ready(labels: &[String]) -> bool {
    labels
        .iter()
        .any(|l| l.trim().eq_ignore_ascii_case(READY_LABEL))
}

/// [`assess_pool`] avec **aucune taille mesurée** — double de test partagé par
/// les trois modules qui jugent un verdict de bassin (mika#2699 vol 3).
///
/// Les tests des vols 1 et 2 portent sur la profondeur, le tri, la provenance
/// et le refus « plan groomé absent » ; aucun ne parle de taille. Leur passer
/// une table vide est donc la lecture fidèle de ce qu'ils mesurent, et le nom
/// le dit plutôt que de le cacher derrière un `&BTreeMap::new()` recopié
/// trente fois. **Ce que ce double ne doit pas devenir** : le chemin par
/// défaut. Les tests du vol 3 appellent [`assess_pool`] directement, avec une
/// table peuplée — sans quoi la borne ne serait exercée nulle part.
#[cfg(test)]
pub(super) fn assess_pool_sans_tailles(
    pool: &[ReadyTicket],
    milestone_candidates: &[PoolCandidate],
    outside_candidates: &[PoolCandidate],
) -> PoolVerdict {
    assess_pool(
        pool,
        milestone_candidates,
        outside_candidates,
        &BTreeMap::new(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(number: u64, labels: &[&str]) -> ReadyTicket {
        ReadyTicket {
            number,
            labels: labels.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Un bassin de `n` tickets dont `in_flight` portent un siège.
    fn pool_of(n: usize, in_flight: usize) -> Vec<ReadyTicket> {
        (0..n)
            .map(|i| {
                if i < in_flight {
                    ready(1000 + i as u64, &["ready", "dispatch:loop"])
                } else {
                    ready(1000 + i as u64, &["ready"])
                }
            })
            .collect()
    }

    fn cand(number: u64, rank: Option<u8>, plan_present: bool, labels: &[&str]) -> PoolCandidate {
        PoolCandidate {
            number,
            state: IssueState::Open,
            priority_rank: rank,
            plan_present,
            labels: labels.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Les numéros admis, dans l'ordre — la plupart des cas ne portent que sur
    /// l'ordre et pas sur la provenance.
    fn admitted(v: &PoolVerdict) -> Vec<u64> {
        v.admit.iter().map(|a| a.number).collect()
    }

    /// L'admission telle que la rendent les cas des vols 1 et 2 : taille **non
    /// mesurée**, puisqu'ils passent par [`assess_pool_sans_tailles`].
    fn admis(number: u64, origin: CandidateOrigin) -> Admission {
        Admission {
            number,
            origin,
            size: PlanSizeReading::Unread(UnreadCause::NotMeasured),
        }
    }

    /// Une table de tailles, pour les cas du vol 3.
    fn tailles(entries: &[(u64, PlanSizeReading)]) -> BTreeMap<u64, PlanSizeReading> {
        entries.iter().copied().collect()
    }

    /// Cas 1 — les bandes, au littéral de « entre 2 et 4 ».
    #[test]
    fn mika2699_les_bandes_disent_entre_2_et_4() {
        for (n, expected) in [
            (0, PoolBand::Starved),
            (1, PoolBand::Starved),
            (2, PoolBand::Nominal),
            (3, PoolBand::Nominal),
            (4, PoolBand::Nominal),
            (5, PoolBand::Saturated),
        ] {
            let v = assess_pool_sans_tailles(&pool_of(n, 0), &[], &[]);
            assert_eq!(
                v.band, expected,
                "{n} disponibles doivent donner {expected:?}, obtenu {:?}",
                v.band
            );
            assert_eq!(v.depth.available, n, "profondeur disponible à {n}");
        }
    }

    /// Cas 2 — `available` retranche les vols. Le fixture est l'état mesuré le
    /// 2026-10-15 élargi d'un ticket libre : 3 `ready` dont 2 en vol → 1
    /// disponible, famine. Un bassin qui ne retrancherait pas aurait dit
    /// « 3, nominal ».
    #[test]
    fn mika2699_un_ready_en_vol_nest_pas_du_rail_disponible() {
        let v = assess_pool_sans_tailles(&pool_of(3, 2), &[], &[]);
        assert_eq!(v.depth.ready_total, 3);
        assert_eq!(v.depth.in_flight, 2);
        assert_eq!(v.depth.available, 1);
        assert_eq!(v.band, PoolBand::Starved);
    }

    /// Cas 3 — un siège **inconnu** compte quand même comme vol en cours : le
    /// prédicat est sur la famille `dispatch:`, pas sur `KNOWN_DISPATCH_SEATS`.
    /// La casse ne le sauve pas non plus. Sans cela, le jour où un siège neuf
    /// est introduit, le bassin devient optimiste et admet en trop.
    #[test]
    fn mika2699_un_siege_inconnu_compte_comme_vol() {
        let pool = vec![
            ready(1, &["ready", "dispatch:zorglub"]),
            ready(2, &["ready", "Dispatch:SSC"]),
            ready(3, &["ready"]),
        ];
        let v = assess_pool_sans_tailles(&pool, &[], &[]);
        assert_eq!(v.depth.in_flight, 2, "les deux sièges comptent: {:?}", v);
        assert_eq!(v.depth.available, 1);
    }

    /// Cas 4 — hystérésis : muette entre 2 et 4, et en famine elle remonte à
    /// [`POOL_MAX`], pas d'un cran.
    #[test]
    fn mika2699_lhysteresis_ne_parle_quen_famine() {
        let candidates: Vec<PoolCandidate> = (1..=6).map(|n| cand(n, Some(1), true, &[])).collect();

        let nominal = assess_pool_sans_tailles(&pool_of(3, 0), &candidates, &[]);
        assert!(
            nominal.admit.is_empty(),
            "à 3 disponibles, rien à recommander: {:?}",
            nominal.admit
        );

        let starved = assess_pool_sans_tailles(&pool_of(1, 0), &candidates, &[]);
        assert_eq!(
            starved.admit.len(),
            POOL_MAX - 1,
            "à 1 disponible, de quoi atteindre {POOL_MAX}: {:?}",
            starved.admit
        );
    }

    /// Cas 5 — le tri : p0 avant p1 avant p2 ; à priorité égale, numéro
    /// croissant ; **un candidat sans label de priorité passe après tous les
    /// autres** — l'absence de priorité n'est pas p0.
    #[test]
    fn mika2699_le_tri_met_labsence_de_priorite_en_dernier() {
        let candidates = vec![
            cand(50, None, true, &[]),
            cand(40, Some(3), true, &[]),
            cand(30, Some(1), true, &[]),
            cand(20, Some(1), true, &[]),
            cand(10, Some(0), true, &[]),
        ];
        let v = assess_pool_sans_tailles(&pool_of(0, 0), &candidates, &[]);
        assert_eq!(
            admitted(&v),
            vec![10, 20, 30, 40],
            "ordre (priorité, numéro) attendu, obtenu {:?}",
            v.admit
        );
        assert!(
            !admitted(&v).contains(&50),
            "le non-priorisé ne passe pas devant un p2"
        );
    }

    /// Cas 6 — le filtre « plan groomé présent » : jamais dans `admit`, et
    /// **nommé** dans `refusals` avec son motif. Un filtre muet est
    /// indistinguable d'un bassin sans candidat, et un filtre qui ne rend qu'un
    /// compte ne dit pas sur quel ticket agir.
    #[test]
    fn mika2699_un_candidat_sans_plan_est_ecarte_et_nomme() {
        let candidates = vec![
            cand(10, Some(0), false, &[]),
            cand(11, Some(0), false, &[]),
            cand(12, Some(2), true, &[]),
        ];
        let v = assess_pool_sans_tailles(&pool_of(0, 0), &candidates, &[]);
        assert_eq!(admitted(&v), vec![12], "seul le candidat groomé entre");
        assert_eq!(
            v.refusals,
            vec![
                AdmissionRefusal {
                    number: 10,
                    origin: CandidateOrigin::Milestone,
                    reason: RefusalReason::NoGroomedPlan,
                },
                AdmissionRefusal {
                    number: 11,
                    origin: CandidateOrigin::Milestone,
                    reason: RefusalReason::NoGroomedPlan,
                },
            ],
            "les deux non-groomés sont nommés avec leur motif"
        );
    }

    /// Cas 7 — déjà `ready` (par le bassin **ou** par ses propres labels), déjà
    /// en vol, ou fermé : jamais recommandé, et jamais **refusé** non plus. Un
    /// ticket déjà dans le bassin n'est pas un candidat mal formé ; le nommer
    /// comme refusé serait un reproche sans geste correctif.
    #[test]
    fn mika2699_deja_ready_deja_en_vol_ou_ferme_nest_ni_admis_ni_refuse() {
        let pool = vec![ready(10, &["ready"])];
        let mut closed = cand(13, Some(0), true, &[]);
        closed.state = IssueState::Closed;
        let candidates = vec![
            cand(10, Some(0), true, &[]),
            cand(11, Some(0), true, &["ready"]),
            cand(12, Some(0), true, &["dispatch:loop"]),
            closed,
            cand(14, Some(2), true, &[]),
        ];
        let v = assess_pool_sans_tailles(&pool, &candidates, &[]);
        assert_eq!(
            admitted(&v),
            vec![14],
            "seul #14 est admissible: {:?}",
            v.admit
        );
        assert!(
            v.refusals.is_empty(),
            "aucun motif de refus: {:?}",
            v.refusals
        );
    }

    /// Cas 8 — bassin vide et aucun candidat : famine, `admit` vide, pas de
    /// panique (et surtout pas de soustraction qui déborde).
    #[test]
    fn mika2699_bassin_vide_sans_candidat_ne_panique_pas() {
        let v = assess_pool_sans_tailles(&[], &[], &[]);
        assert_eq!(v.depth.available, 0);
        assert_eq!(v.band, PoolBand::Starved);
        assert!(v.admit.is_empty());
        assert!(v.refusals.is_empty());
    }

    /// Cas 9 — à priorité égale, le milestone cible passe **avant le reste**, et
    /// chaque admission porte sa provenance pour que le rapport puisse la dire.
    #[test]
    fn mika2699_a_priorite_egale_le_milestone_passe_avant_le_reste() {
        let inside = vec![cand(90, Some(1), true, &[])];
        let outside = vec![cand(20, Some(1), true, &[])];
        let v = assess_pool_sans_tailles(&pool_of(0, 0), &inside, &outside);
        assert_eq!(
            v.admit,
            vec![
                admis(90, CandidateOrigin::Milestone),
                admis(20, CandidateOrigin::Outside),
            ],
            "le #90 du milestone précède le #20 hors milestone malgré son numéro plus grand"
        );
    }

    /// Cas 10 — la priorité domine la provenance : un p0 hors milestone passe
    /// devant un p1 du milestone. C'est l'ordre des deux critères dans le point
    /// 1 du ticket, et l'inverse tiendrait un p0 critique derrière un ticket de
    /// confort.
    #[test]
    fn mika2699_un_p0_hors_milestone_passe_devant_un_p1_du_milestone() {
        let inside = vec![cand(90, Some(1), true, &[])];
        let outside = vec![cand(91, Some(0), true, &[])];
        let v = assess_pool_sans_tailles(&pool_of(0, 0), &inside, &outside);
        assert_eq!(
            admitted(&v),
            vec![91, 90],
            "le p0 hors milestone passe premier: {:?}",
            v.admit
        );
    }

    /// Cas 11 — les candidats hors milestone subissent les mêmes filtres, et
    /// leur refus est nommé avec la provenance. Sans la provenance, l'opérateur
    /// ne sait pas si le ticket à groomer est dans son milestone ou ailleurs.
    #[test]
    fn mika2699_un_candidat_hors_milestone_subit_les_memes_filtres() {
        let mut closed = cand(23, Some(0), true, &[]);
        closed.state = IssueState::Closed;
        let outside = vec![
            cand(20, Some(0), false, &[]),
            cand(21, Some(0), true, &["ready"]),
            cand(22, Some(0), true, &["dispatch:loop"]),
            closed,
            cand(24, Some(2), true, &[]),
        ];
        let v = assess_pool_sans_tailles(&pool_of(0, 0), &[], &outside);
        assert_eq!(
            admitted(&v),
            vec![24],
            "seul #24 est admissible: {:?}",
            v.admit
        );
        assert_eq!(
            v.refusals,
            vec![AdmissionRefusal {
                number: 20,
                origin: CandidateOrigin::Outside,
                reason: RefusalReason::NoGroomedPlan,
            }],
            "le non-groomé hors milestone est nommé comme tel"
        );
    }

    /// Cas 12 — un ticket présent dans les deux ensembles n'est jugé qu'une
    /// fois, et comme ticket **du milestone**. Les deux lectures se recoupent
    /// par construction : la lecture dépôt-large rend aussi les `sub_issues` du
    /// milestone cible. Sans la déduplication, le manager recommanderait deux
    /// fois le même ticket et consommerait deux places du bassin pour une.
    #[test]
    fn mika2699_un_candidat_vu_deux_fois_nest_juge_quune_fois() {
        let shared = cand(90, Some(1), true, &[]);
        // Le **même** candidat dans les deux ensembles, par `from_ref` plutôt que
        // par `clone` : c'est précisément le recoupement que le test décrit.
        let v = assess_pool_sans_tailles(
            &pool_of(0, 0),
            std::slice::from_ref(&shared),
            std::slice::from_ref(&shared),
        );
        assert_eq!(
            v.admit,
            vec![admis(90, CandidateOrigin::Milestone)],
            "une seule admission, provenance milestone: {:?}",
            v.admit
        );
    }

    /// Cas 13 — un refus vu dans les deux ensembles n'est nommé qu'une fois, et
    /// l'ordre des refus est déterministe : milestone d'abord, puis numéro
    /// croissant. Un rapport dont la section bouge sans que l'état ait changé
    /// est un rapport qu'on cesse de lire.
    #[test]
    fn mika2699_lordre_des_refus_est_deterministe() {
        let shared = cand(50, Some(0), false, &[]);
        let inside = vec![cand(60, Some(0), false, &[]), shared.clone()];
        let outside = vec![cand(30, Some(0), false, &[]), shared];
        let v = assess_pool_sans_tailles(&pool_of(3, 0), &inside, &outside);
        assert_eq!(
            v.refusals,
            vec![
                AdmissionRefusal {
                    number: 50,
                    origin: CandidateOrigin::Milestone,
                    reason: RefusalReason::NoGroomedPlan,
                },
                AdmissionRefusal {
                    number: 60,
                    origin: CandidateOrigin::Milestone,
                    reason: RefusalReason::NoGroomedPlan,
                },
                AdmissionRefusal {
                    number: 30,
                    origin: CandidateOrigin::Outside,
                    reason: RefusalReason::NoGroomedPlan,
                },
            ],
            "refus triés (provenance, numéro), sans doublon: {:?}",
            v.refusals
        );
    }

    /// Cas 14 — les refus sont rendus **hors famine aussi**. Un ticket mal formé
    /// reste mal formé quand le bassin est plein, et c'est le moment utile pour
    /// le dire : attendre la famine, c'est découvrir le refus quand le rail est
    /// déjà vide.
    #[test]
    fn mika2699_un_refus_est_dit_meme_quand_le_bassin_est_plein() {
        let inside = vec![cand(60, Some(0), false, &[])];
        let v = assess_pool_sans_tailles(&pool_of(4, 0), &inside, &[]);
        assert_eq!(v.band, PoolBand::Nominal);
        assert!(v.admit.is_empty(), "rien à admettre hors famine");
        assert_eq!(
            v.refusals,
            vec![AdmissionRefusal {
                number: 60,
                origin: CandidateOrigin::Milestone,
                reason: RefusalReason::NoGroomedPlan,
            }],
            "le refus est dit quand même: {:?}",
            v.refusals
        );
    }

    /// Cas 15 — la projection `SubIssue → PoolCandidate` porte les cinq champs
    /// que l'admission consulte. Un champ oublié ici ne casse aucune règle de
    /// façon visible : il rend simplement tous les candidats du milestone
    /// non-priorisés, ou tous non-groomés, ce qui se lit comme un bassin sans
    /// candidat.
    #[test]
    fn mika2699_la_projection_dune_sub_issue_porte_les_champs_de_ladmission() {
        use crate::milestone_manager::types::CiState;
        let sub = SubIssue {
            number: 2699,
            title: "le manager tient le bassin ready".into(),
            state: IssueState::Open,
            priority_rank: Some(2),
            plan_present: true,
            branch_present: true,
            pr_number: Some(2710),
            pr_state: None,
            ci_state: CiState::Success,
            blockers: vec![2701],
            updated_at: "2026-10-15T00:00:00Z".into(),
            labels: vec!["ready".into(), "agent-core".into()],
        };
        assert_eq!(
            PoolCandidate::from(&sub),
            PoolCandidate {
                number: 2699,
                state: IssueState::Open,
                priority_rank: Some(2),
                plan_present: true,
                labels: vec!["ready".into(), "agent-core".into()],
            }
        );
    }

    // -- mika#2699 vol 2 : le format fil du verdict -------------------------

    /// Cas 17 — `refusals` absent se relit en liste vide, et une admission se
    /// dit sur le fil comme un **objet** portant sa provenance.
    ///
    /// Le verdict voyage : il est le champ `pool` de [`Assessment`], donc il part
    /// dans chaque `DeliveryBody` et chaque fichier du puits hors-ligne. Les deux
    /// moitiés du test sont les deux questions qu'un lecteur futur se posera sur
    /// ces fichiers — un champ que le vol 2 a ajouté peut-il manquer, et la
    /// provenance est-elle sur le fil ou seulement dans le Markdown rendu.
    ///
    /// [`Assessment`]: crate::milestone_manager::types::Assessment
    #[test]
    fn mika2699_un_verdict_sans_refus_se_relit_et_ladmission_porte_sa_provenance() {
        let sans_refus = r#"{
            "depth": {"ready_total": 1, "in_flight": 1, "available": 0},
            "band": "starved",
            "admit": [{"number": 1700, "origin": "outside"}]
        }"#;
        let v: PoolVerdict =
            serde_json::from_str(sans_refus).expect("`refusals` absent doit valoir la liste vide");
        assert!(v.refusals.is_empty(), "champ absent ⇒ vide, pas d'erreur");
        assert_eq!(
            v.admit,
            vec![admis(1700, CandidateOrigin::Outside)],
            "la provenance se lit depuis le fil, pas seulement depuis le rendu ; \
             et une admission du vol 2, sans champ `size`, se relit en « non \
             mesurée » — jamais en taille nulle (mika#2699 vol 3, motif mika#2331)"
        );
        let out = serde_json::to_string(&v).unwrap();
        assert_eq!(
            serde_json::from_str::<PoolVerdict>(&out).unwrap(),
            v,
            "aller-retour fidèle : le puits doit rester relisible par lui-même"
        );
    }

    /// Cas 18 — un verdict écrit par le **vol 1** ne se relit plus, et ce test
    /// est là pour que ce soit dit.
    ///
    /// Le vol 1 écrivait `admit: [1802]` (des entiers) et `withheld_no_plan: 1`
    /// (un compte). Le vol 2 écrit `admit: [{number, origin}]` et `refusals`
    /// nommés — un changement de forme, pas une extension, parce qu'un compte
    /// agrégé ne peut pas être rouvert en une liste de tickets nommés : les
    /// numéros n'y sont plus. Un `#[serde(default)]` n'aurait rien sauvé ici ;
    /// seul un `#[serde(untagged)]` sur `admit` l'aurait fait, au prix d'un
    /// chemin de lecture ambigu permanent pour un arriéré d'un seul vol.
    ///
    /// La rupture est **acceptée** : le dépôt est pré-1.0 (racine `CLAUDE.md`
    /// § *Versioning*), et surtout **aucun code ne relit ces fichiers** —
    /// `sink_dir::list_sink_reports` énumère des chemins et ne désérialise rien,
    /// et le `DeliveryBody` part vers le control-monitor sans retour. L'arriéré
    /// du puits est une archive lue par un opérateur, et un opérateur voit
    /// `withheld_no_plan` pour ce qu'il est.
    ///
    /// Ce qui serait une régression, et que ce test attrape : qu'un futur vol
    /// casse à nouveau la forme **en croyant qu'elle n'a jamais bougé**. Si ce
    /// test rougit parce que le blob du vol 1 se relit de nouveau, quelqu'un a
    /// ajouté un chemin de compatibilité — qu'il le documente ici plutôt que de
    /// supprimer le cas.
    #[test]
    fn mika2699_un_verdict_du_vol_1_ne_se_relit_pas_et_cest_assume() {
        let blob_vol_1 = r#"{
            "depth": {"ready_total": 2, "in_flight": 2, "available": 0},
            "band": "starved",
            "admit": [1802],
            "withheld_no_plan": 1
        }"#;
        assert!(
            serde_json::from_str::<PoolVerdict>(blob_vol_1).is_err(),
            "`admit` a changé de forme au vol 2 : la relecture échoue, et le test \
             existe pour que ce fait soit nommé plutôt que découvert"
        );
    }

    // -- mika#2699 vol 3 : la borne par vol, second motif de refus -----------

    /// Cas 19 — un plan au-delà de la borne de **son** plafond est refusé, et le
    /// refus porte les deux nombres.
    ///
    /// Les deux nombres, parce qu'un « estimation au-delà de la borne » sans
    /// chiffres n'indique pas de geste : découper en deux ou en cinq ne se
    /// décide pas sans savoir de combien on dépasse.
    #[test]
    fn mika2699_une_estimation_au_dela_de_la_borne_refuse_et_dit_les_deux_nombres() {
        let candidats = vec![cand(90, Some(1), true, &[])];
        // Plafond par défaut 150 ⇒ borne 279. 500 est au-delà.
        let v = assess_pool(
            &pool_of(0, 0),
            &candidats,
            &[],
            &tailles(&[(90, PlanSizeReading::Measured(500))]),
        );
        assert!(
            v.admit.is_empty(),
            "un plan de 500 lignes ne passe pas une borne de 279 : {:?}",
            v.admit
        );
        assert_eq!(
            v.refusals,
            vec![AdmissionRefusal {
                number: 90,
                origin: CandidateOrigin::Milestone,
                reason: RefusalReason::EstimateOverFlightBound {
                    total_loc: 500,
                    bound_loc: 279,
                },
            }],
        );
        assert_eq!(
            v.refusals[0].reason.label(),
            "estimation 500 lignes au-delà de la borne par vol (279) — à découper",
            "l'opérateur lit de combien il dépasse, pas seulement qu'il dépasse"
        );
    }

    /// Cas 20 — un plan qui tient **exactement** la borne la tient.
    ///
    /// Le ticket dit « au-delà de la borne », pas « à la borne ». Un `>=` ici
    /// refuserait le plan dimensionné *pour* ce plafond — c'est-à-dire celui que
    /// le grooming a correctement découpé — et la correction serait de le
    /// découper encore, sans raison.
    #[test]
    fn mika2699_un_plan_qui_tient_exactement_la_borne_la_tient() {
        let candidats = vec![cand(90, Some(1), true, &[])];
        let v = assess_pool(
            &pool_of(0, 0),
            &candidats,
            &[],
            &tailles(&[(90, PlanSizeReading::Measured(279))]),
        );
        assert_eq!(
            v.admit,
            vec![Admission {
                number: 90,
                origin: CandidateOrigin::Milestone,
                size: PlanSizeReading::Measured(279),
            }],
            "279 est la borne du plafond par défaut, donc elle est tenue"
        );
        assert!(v.refusals.is_empty());
    }

    /// Cas 21 — la borne est celle du plafond **du candidat**, pas une borne de
    /// flotte.
    ///
    /// Un `loop-substrate` porte un plafond de 200 tours (mika#2542) donc une
    /// borne de 429. Juger les deux candidats à la même borne refuserait le
    /// premier à tort — et c'est le seul test qui voit cette erreur-là : avec une
    /// borne unique, les deux tickets ci-dessous rendraient le même verdict pour
    /// la mauvaise raison si l'un des deux manquait.
    #[test]
    fn mika2699_la_borne_est_celle_du_plafond_du_candidat() {
        let candidats = vec![
            cand(90, Some(1), true, &["loop-substrate"]),
            cand(91, Some(1), true, &[]),
        ];
        let v = assess_pool(
            &pool_of(0, 0),
            &candidats,
            &[],
            &tailles(&[
                (90, PlanSizeReading::Measured(400)),
                (91, PlanSizeReading::Measured(400)),
            ]),
        );
        assert_eq!(
            admitted(&v),
            vec![90],
            "400 tient sous la borne 429 d'un `loop-substrate`, pas sous les 279 d'un ticket nu"
        );
        assert_eq!(
            v.refusals.iter().map(|r| r.number).collect::<Vec<_>>(),
            vec![91],
        );
        assert!(matches!(
            v.refusals[0].reason,
            RefusalReason::EstimateOverFlightBound { bound_loc: 279, .. }
        ));
    }

    /// Cas 22 — aucune des trois lectures non mesurées ne refuse, et aucune ne
    /// se tait.
    ///
    /// C'est le cœur du vol 3. « Plan sans section de taille » n'est pas « plan
    /// au-delà de la borne » : refuser dessus inventerait un motif que la
    /// mesure ne porte pas. Mais admettre en silence ferait lire « À admettre »
    /// comme « borne vérifiée », qui est précisément ce que le point 2 promet.
    /// La sortie est donc l'admission **plus** la cause, que le rapporteur
    /// rendra.
    #[test]
    fn mika2699_une_taille_non_mesuree_nadmet_pas_en_silence_et_ne_refuse_pas() {
        let candidats = vec![
            cand(90, Some(1), true, &[]),
            cand(91, Some(1), true, &[]),
            cand(92, Some(1), true, &[]),
            cand(93, Some(1), true, &[]),
        ];
        let v = assess_pool(
            &pool_of(0, 0),
            &candidats,
            &[],
            &tailles(&[
                (90, PlanSizeReading::SectionAbsent),
                (91, PlanSizeReading::Unparsable),
                (92, PlanSizeReading::Unread(UnreadCause::FetchFailed)),
                // #93 est absent de la table : non mesuré par omission.
            ]),
        );
        assert!(
            v.refusals.is_empty(),
            "aucune de ces lectures n'est une estimation « au-delà » : {:?}",
            v.refusals
        );
        assert_eq!(
            v.admit.iter().map(|a| a.size).collect::<Vec<_>>(),
            vec![
                PlanSizeReading::SectionAbsent,
                PlanSizeReading::Unparsable,
                PlanSizeReading::Unread(UnreadCause::FetchFailed),
                PlanSizeReading::Unread(UnreadCause::NotMeasured),
            ],
            "chaque cause voyage avec son admission ; un candidat absent de la \
             table est « non mesuré », jamais de taille nulle (mika#2331)"
        );
        for a in &v.admit {
            assert!(
                a.size.unmeasured_label().is_some(),
                "le rapport doit pouvoir nommer chacune : {:?}",
                a.size
            );
        }
    }

    /// Cas 23 — les plans à lire sont ceux de l'admission, dans l'ordre du
    /// bassin et bornés.
    ///
    /// Deux propriétés qu'aucune autre assertion ne voit. **L'ordre** : la liste
    /// est tronquée après le tri, donc une borne atteinte garde les plus
    /// prioritaires et non les plus petits numéros. **Le filtre** : un ticket
    /// sans plan groomé est déjà refusé, donc ne coûte aucun appel `gh` — c'est
    /// ce qui tient le coût de la lecture par candidat.
    #[test]
    fn mika2699_les_plans_a_lire_sont_ceux_de_ladmission_tries_et_bornes() {
        let mut candidats: Vec<PoolCandidate> = (0..PLAN_SIZE_READ_MAX as u64 + 3)
            // Numéros décroissants à priorité croissante : si la borne était
            // appliquée avant le tri, la liste commencerait par les numéros
            // hauts et ce test le verrait.
            .map(|i| cand(900 - i, Some((i % 3) as u8), true, &[]))
            .collect();
        candidats.push(cand(50, Some(0), false, &[]));

        let wanted = numbers_needing_size(&pool_of(0, 0), &candidats, &[]);
        assert_eq!(
            wanted.len(),
            PLAN_SIZE_READ_MAX,
            "la liste est bornée par `PLAN_SIZE_READ_MAX`"
        );
        assert!(
            !wanted.contains(&50),
            "#50 n'a pas de plan groomé : il est refusé avant d'avoir coûté une lecture"
        );
        let attendu: Vec<u64> = {
            let (eligible, _) = well_formed_candidates(&pool_of(0, 0), &candidats, &[]);
            eligible
                .iter()
                .take(PLAN_SIZE_READ_MAX)
                .map(|(c, _)| c.number)
                .collect()
        };
        assert_eq!(wanted, attendu, "l'ordre est celui du bassin");
        // #886 est le plus petit numéro de l'ensemble, et il est p2 : le
        // premier à lire est #888, le plus petit numéro **parmi les p0**.
        assert_eq!(
            wanted[0], 888,
            "le premier à lire est le p0 de plus petit numéro, pas le plus petit numéro"
        );
    }

    /// Cas 24 — le filtre d'admission a **un site**, partagé par le jugement et
    /// par la désignation des plans à lire.
    ///
    /// Les deux réponses sont identiques aujourd'hui ; ce qu'elles ne doivent
    /// pas devenir, c'est deux écritures du même filtre qui divergent en silence
    /// — le manager irait alors lire le plan d'un ticket qu'il ne jugera pas, et
    /// jugerait sans taille un ticket dont le plan était lisible (doctrine du
    /// lecteur unique, mika#2201).
    #[test]
    fn mika2699_le_filtre_dadmission_a_un_site_partage() {
        let candidats = vec![
            cand(90, Some(1), true, &[]),
            cand(91, Some(0), true, &[]),
            cand(92, None, false, &[]),
            cand(93, Some(2), true, &["ready"]),
        ];
        let outside = vec![cand(20, Some(0), true, &[])];
        let pool = pool_of(0, 0);

        let wanted = numbers_needing_size(&pool, &candidats, &outside);
        // Le même état jugé : l'ordre des admissions, bande famine, est le
        // préfixe de l'ordre des lectures.
        let v = assess_pool(&pool, &candidats, &outside, &BTreeMap::new());
        assert_eq!(
            admitted(&v),
            wanted[..v.admit.len()].to_vec(),
            "ce qui est admis est le préfixe de ce qui a été désigné à la lecture"
        );
        assert!(
            !wanted.contains(&93),
            "un ticket déjà `ready` n'est pas un candidat, donc pas une lecture"
        );
    }

    // -- mika#2700 Phase 2 (c) vol 1 : la clé de tri a un site unique --------

    /// La clé de tri du bassin n'a qu'un site, et c'est [`pool_sort_key`].
    ///
    /// Deux moitiés, parce qu'aucune des deux ne voit la panne de l'autre.
    /// **Le comportement** : la priorité domine la provenance, qui domine le
    /// numéro, et une priorité absente passe en dernier. **Le balayage** :
    /// `origin.rank()` ne s'appelle que là. Sans lui, la relève nommée pourrait
    /// réécrire le triplet à son compte et présenter, dans le même rapport, un
    /// ordre qui contredit celui du bassin — sans qu'aucune assertion de
    /// comportement rougisse (doctrine du lecteur unique, mika#2201 ; classe
    /// du marqueur silencieux, mika#2158).
    ///
    /// Le balayage lit le répertoire **à plat** : la porte 1 du `DecisionCore`
    /// interdit un `tests/` imbriqué sous `milestone_manager/`, donc tout test
    /// du module vit derrière un `#[cfg(test)]` de son propre fichier et la
    /// troncature ci-dessous suffit (prémisse mika#2321).
    #[test]
    fn mika2700_la_cle_de_tri_du_bassin_a_un_site_unique() {
        assert!(
            pool_sort_key(Some(0), CandidateOrigin::Outside, u64::MAX)
                < pool_sort_key(Some(1), CandidateOrigin::Milestone, 1),
            "la priorité domine la provenance comme le numéro"
        );
        assert!(
            pool_sort_key(Some(1), CandidateOrigin::Milestone, u64::MAX)
                < pool_sort_key(Some(1), CandidateOrigin::Outside, 1),
            "à priorité égale, le milestone passe avant le dehors"
        );
        assert!(
            pool_sort_key(Some(1), CandidateOrigin::Milestone, 1)
                < pool_sort_key(Some(1), CandidateOrigin::Milestone, 2),
            "à priorité et provenance égales, le plus petit numéro passe devant"
        );
        assert!(
            pool_sort_key(Some(u8::MAX - 1), CandidateOrigin::Outside, u64::MAX)
                < pool_sort_key(None, CandidateOrigin::Milestone, 1),
            "une priorité absente passe en dernier — l'absence n'est pas p0"
        );

        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/milestone_manager");
        let mut sites: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(dir).expect("le répertoire du module doit être lisible") {
            let path = entry.expect("entrée de répertoire lisible").path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            let src = std::fs::read_to_string(&path).expect("source du module lisible");
            // La production seule : un test a le droit d'appeler la clé, et
            // celui-ci l'appelle quatre fois juste au-dessus.
            let production = src.split("#[cfg(test)]").next().unwrap_or_default();
            for (i, line) in production.lines().enumerate() {
                // La prose *sur* la clé n'est pas un appel : trois paragraphes
                // en parlent au-dessus (forme du signal S, mika#2050).
                let code = line.split("//").next().unwrap_or_default();
                if code.contains(".rank()") {
                    sites.push(format!("{name}:{}", i + 1));
                }
            }
        }
        assert_eq!(
            sites.len(),
            1,
            "`origin.rank()` ne doit s'appeler que dans `pool_sort_key` ; \
             sites trouvés : {sites:?}. Un second site réécrit la clé de tri : \
             route-le par `pool_sort_key` plutôt que d'allonger ce compte"
        );
        assert!(
            sites[0].starts_with("pool.rs:"),
            "le site unique vit dans `pool.rs` ; trouvé : {}",
            sites[0]
        );
    }

    // -- mika#2923 : un ticket que l'opérateur tient n'entre pas -------------

    /// Membres de `OPERATOR_HELD_LABELS` qui, délibérément, **n'excluent pas**
    /// un candidat du bassin. **Livrée vide, et c'est l'état nominal** : les
    /// trois membres d'aujourd'hui tiennent tous le ticket au même titre
    /// (mika#2263).
    ///
    /// Une entrée future doit nommer le label, dire pourquoi l'admission
    /// l'ignore, et référencer son ticket de suivi — jamais un nom seul.
    const EXCEPTIONS_ADMISSION: &[(&str, &str)] = &[];

    /// V1 — le corpus du rapport du 2026-10-26, contrôle positif et contrôle
    /// négatif dans le **même** test (AC1).
    ///
    /// Les vrais numéros et les vrais labels : un corpus inventé testerait le
    /// prédicat sans attester la cause du ticket. Le rapport recommandait
    /// d'admettre #2698, #2699 et #1806, tous trois `operator-gated` ; le
    /// quatrième candidat, groomé et sans label gaté, est ce qui sépare « le
    /// filtre marche » de « plus rien n'est admis ».
    #[test]
    fn mika2923_le_corpus_du_2026_10_26_refuse_les_trois_gates_et_admet_le_non_gate() {
        let milestone = vec![
            cand(2698, Some(1), true, &["p1-important", "operator-gated"]),
            cand(2699, Some(1), true, &["p1-important", "operator-gated"]),
            // Le candidat non gaté, groomé : le contrôle négatif.
            cand(2923, Some(1), true, &["p1-important"]),
        ];
        let outside = vec![cand(
            1806,
            Some(1),
            true,
            &["p1-important", "operator-gated"],
        )];
        // Bande famine pour que `admit` soit non vide.
        let v = assess_pool_sans_tailles(&pool_of(0, 0), &milestone, &outside);

        assert_eq!(
            admitted(&v),
            vec![2923],
            "seul le candidat sans label gaté est recommandé : {:?}",
            v.admit
        );
        let refuses: Vec<(u64, String)> = v
            .refusals
            .iter()
            .map(|r| (r.number, r.reason.label()))
            .collect();
        assert_eq!(
            refuses,
            vec![
                (
                    2698,
                    "tenu par l'opérateur — label `operator-gated` à retirer".to_string()
                ),
                (
                    2699,
                    "tenu par l'opérateur — label `operator-gated` à retirer".to_string()
                ),
                // Le dehors vient après le milestone : les refus sortent dans
                // l'ordre de parcours des candidats, pas dans l'ordre de tri du
                // bassin — c'est `admit` qui est trié.
                (
                    1806,
                    "tenu par l'opérateur — label `operator-gated` à retirer".to_string()
                ),
            ],
            "les trois gatés sont refusés, et le refus **nomme** le label à retirer"
        );
    }

    /// V2a — `operator-gated` **seul** suffit à refuser (AC2).
    ///
    /// Son jumeau non labellisé est dans le même test : sans lui, un refus
    /// pourrait venir de n'importe quel autre filtre du semis.
    #[test]
    fn mika2923_operator_gated_seul_suffit_a_refuser() {
        let milestone = vec![
            cand(70, Some(1), true, &["operator-gated"]),
            cand(71, Some(1), true, &[]),
        ];
        let v = assess_pool_sans_tailles(&pool_of(0, 0), &milestone, &[]);

        assert_eq!(
            admitted(&v),
            vec![71],
            "le jumeau sans label est admis — c'est le label qui écarte #70, \
             pas le hasard du semis : {:?}",
            v.admit
        );
        assert_eq!(
            v.refusals,
            vec![AdmissionRefusal {
                number: 70,
                origin: CandidateOrigin::Milestone,
                reason: RefusalReason::OperatorHeld {
                    label: "operator-gated".to_string(),
                },
            }],
        );
    }

    /// V2b — `blocked` **seul** suffit à refuser (AC2).
    ///
    /// Un test distinct de V2a, exprès : neutraliser un terme du prédicat ne
    /// doit pas laisser l'autre vert par accident.
    #[test]
    fn mika2923_blocked_seul_suffit_a_refuser() {
        let milestone = vec![
            cand(80, Some(1), true, &["blocked"]),
            cand(81, Some(1), true, &[]),
        ];
        let v = assess_pool_sans_tailles(&pool_of(0, 0), &milestone, &[]);

        assert_eq!(
            admitted(&v),
            vec![81],
            "le jumeau sans label est admis : {:?}",
            v.admit
        );
        assert_eq!(
            v.refusals,
            vec![AdmissionRefusal {
                number: 80,
                origin: CandidateOrigin::Milestone,
                reason: RefusalReason::OperatorHeld {
                    label: "blocked".to_string(),
                },
            }],
        );
    }

    /// V3 — **chaque** membre de `OPERATOR_HELD_LABELS` refuse à l'admission.
    ///
    /// L'itération est sur la constante, jamais sur une liste recopiée : donc
    /// `operator-review` est couvert aujourd'hui sans l'avoir nommé, et un
    /// membre ajouté demain l'est le jour de son arrivée. C'est le détecteur de
    /// ce vol, et il atterrit vert sur les trois membres.
    #[test]
    fn mika2923_chaque_label_tenu_refuse_a_ladmission() {
        use crate::webhook_dispatch::OPERATOR_HELD_LABELS;

        // Sans cette assertion, la boucle garderait zéro chose et le test
        // passerait en ne mesurant rien.
        assert!(
            !OPERATOR_HELD_LABELS.is_empty(),
            "une liste vide ferait de ce test une boucle sur rien"
        );

        for label in OPERATOR_HELD_LABELS {
            if EXCEPTIONS_ADMISSION.iter().any(|(l, _)| l == label) {
                continue;
            }
            let milestone = vec![cand(60, Some(1), true, &[label])];
            let v = assess_pool_sans_tailles(&pool_of(0, 0), &milestone, &[]);
            assert!(
                v.admit.is_empty(),
                "`{label}` tient le ticket : il ne doit pas être admis — {:?}",
                v.admit
            );
            assert_eq!(
                v.refusals,
                vec![AdmissionRefusal {
                    number: 60,
                    origin: CandidateOrigin::Milestone,
                    reason: RefusalReason::OperatorHeld {
                        label: (*label).to_string(),
                    },
                }],
                "`{label}` doit refuser en se nommant"
            );
        }

        // Une exception qui survit au label qu'elle exemptait est la façon dont
        // une allowlist meurt sans bruit.
        for (label, _raison) in EXCEPTIONS_ADMISSION {
            assert!(
                OPERATOR_HELD_LABELS.contains(label),
                "exception stale : `{label}` n'est plus dans OPERATOR_HELD_LABELS — \
                 retire l'entrée, l'exception ne protège plus rien"
            );
        }
    }

    /// V6 — `MIKA_MANAGER_POOL_WRITE` reste lu exactement comme avant (AC4).
    ///
    /// Épinglage **par valeur**, dans le même commit que le filtre : « rien
    /// n'a changé » doit être une assertion et pas une déclaration. Le vol
    /// n'écrit aucune ligne dans `pool_writer.rs`, et c'est ce test qui le dit.
    #[test]
    fn mika2923_linterrupteur_decriture_reste_lu_comme_avant() {
        use crate::milestone_manager::pool_writer::ENV_POOL_WRITE;

        assert_eq!(
            ENV_POOL_WRITE, "MIKA_MANAGER_POOL_WRITE",
            "le nom de l'interrupteur est le contrat opérateur : il ne bouge pas"
        );
    }
}

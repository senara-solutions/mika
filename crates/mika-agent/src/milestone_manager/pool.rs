//! Pool — le bassin `ready` : sa profondeur, sa bande, et qui devrait y entrer.
//!
//! **LECTURE SEULE.** mika#2699 Phase 2 (a), vols 1 et 2 : le manager *voit* le
//! bassin et *dit* qui devrait y entrer, et pourquoi il refuse les autres. Il ne
//! pose aucun label, ne rattache aucun milestone, ne déclare aucun
//! interrupteur. `assess_pool` est une fonction pure sans E/S ; le verdict entre
//! dans le rapport et nulle part ailleurs. L'écriture
//! (`MIKA_MANAGER_POOL_WRITE` et le geste groupé) reste à venir, avec son
//! exception nommée à `no_dispatch_test.rs`.
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
//! # Ce que ce module ne dit pas encore
//!
//! [`RefusalReason`] ne porte qu'un motif, `NoGroomedPlan`. Le second motif que
//! le point 2 du ticket nomme — *estimation au-delà de la borne par vol sans
//! découpage* — demande une lecture du plan **par candidat** au ref de sa
//! branche (`gh api /repos/…/contents/<chemin>?ref=<branche>`) et un portage en
//! Rust du contrat de parse à trois jetons de `_plan_size_total_loc`
//! (`skills/bundled/_shared/dispatch-lib.sh`, porte 3 de mika#2701). Mesuré à
//! ≈ 150–270 lignes estimées de plus, ce qui porte ce vol au-delà de la borne
//! par vol de 431 lignes ; il est donc le vol suivant, et ce paragraphe est
//! l'endroit où un éditeur futur le retrouve. L'enum existe déjà sous forme
//! d'enum pour que l'ajout du motif soit un variant et non une refonte du
//! rapport.

use super::types::{IssueState, SubIssue};
use crate::webhook_dispatch::{DISPATCH_SEAT_LABEL_PREFIX, READY_LABEL};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

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

/// Un ticket que le manager recommande de faire entrer dans le bassin.
///
/// Porte sa provenance parce que le rapport doit la dire : « #2708 (p2-normal,
/// hors milestone) » est une recommandation qu'un opérateur peut arbitrer,
/// « #2708 » seul ne l'est pas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Admission {
    pub number: u64,
    pub origin: CandidateOrigin,
}

/// Pourquoi un candidat n'est pas recommandé.
///
/// Un `enum` plutôt qu'un message : le second motif du point 2 du ticket
/// (estimation au-delà de la borne par vol sans découpage) arrive au vol
/// suivant, et doit arriver comme un variant — voir le docstring du module pour
/// ce qu'il coûte et pourquoi il n'est pas ici.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalReason {
    /// Aucun callout `Plan: docs/plans/…` dans le corps du ticket.
    NoGroomedPlan,
}

impl RefusalReason {
    /// Le motif, dit à l'opérateur. Vit ici et non dans le rapport pour qu'un
    /// variant neuf ne puisse pas être rendu sans libellé.
    pub fn label(self) -> &'static str {
        match self {
            RefusalReason::NoGroomedPlan => "plan groomé absent",
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
/// 3. **Le tri est `(priority_rank.unwrap_or(u8::MAX), origin.rank(), number)`.**
///    La priorité domine la provenance — voir le docstring du module pour
///    pourquoi l'ordre des deux critères est la décision et non un détail. Un
///    ticket sans label de priorité passe **en dernier** : l'absence de priorité
///    n'est pas p0.
/// 4. **Un même numéro n'est jugé qu'une fois.** Les deux ensembles sont pris à
///    deux appels `gh` distincts et se recoupent par construction (une
///    `sub_issue` du milestone est aussi une issue ouverte du dépôt). Le premier
///    vu gagne, et les candidats du milestone sont parcourus d'abord — donc un
///    ticket du milestone n'est jamais étiqueté `Outside`.
/// 5. **Les refus sont rendus hors famine aussi**, et triés
///    `(origin.rank(), number)` pour que le rapport soit stable d'un cycle à
///    l'autre quand rien n'a changé.
pub fn assess_pool(
    pool: &[ReadyTicket],
    milestone_candidates: &[PoolCandidate],
    outside_candidates: &[PoolCandidate],
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
    eligible
        .sort_by_key(|(s, origin)| (s.priority_rank.unwrap_or(u8::MAX), origin.rank(), s.number));
    refusals.sort_by_key(|r| (r.origin.rank(), r.number));

    // `POOL_MAX - available` ne peut pas déborder : la bande famine impose
    // `available < POOL_MIN`, et `POOL_MIN <= POOL_MAX`.
    let admit: Vec<Admission> = match band {
        PoolBand::Starved => eligible
            .iter()
            .take(POOL_MAX - available)
            .map(|(s, origin)| Admission {
                number: s.number,
                origin: *origin,
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
            let v = assess_pool(&pool_of(n, 0), &[], &[]);
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
        let v = assess_pool(&pool_of(3, 2), &[], &[]);
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
        let v = assess_pool(&pool, &[], &[]);
        assert_eq!(v.depth.in_flight, 2, "les deux sièges comptent: {:?}", v);
        assert_eq!(v.depth.available, 1);
    }

    /// Cas 4 — hystérésis : muette entre 2 et 4, et en famine elle remonte à
    /// [`POOL_MAX`], pas d'un cran.
    #[test]
    fn mika2699_lhysteresis_ne_parle_quen_famine() {
        let candidates: Vec<PoolCandidate> = (1..=6).map(|n| cand(n, Some(1), true, &[])).collect();

        let nominal = assess_pool(&pool_of(3, 0), &candidates, &[]);
        assert!(
            nominal.admit.is_empty(),
            "à 3 disponibles, rien à recommander: {:?}",
            nominal.admit
        );

        let starved = assess_pool(&pool_of(1, 0), &candidates, &[]);
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
        let v = assess_pool(&pool_of(0, 0), &candidates, &[]);
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
        let v = assess_pool(&pool_of(0, 0), &candidates, &[]);
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
        let v = assess_pool(&pool, &candidates, &[]);
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
        let v = assess_pool(&[], &[], &[]);
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
        let v = assess_pool(&pool_of(0, 0), &inside, &outside);
        assert_eq!(
            v.admit,
            vec![
                Admission {
                    number: 90,
                    origin: CandidateOrigin::Milestone,
                },
                Admission {
                    number: 20,
                    origin: CandidateOrigin::Outside,
                },
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
        let v = assess_pool(&pool_of(0, 0), &inside, &outside);
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
        let v = assess_pool(&pool_of(0, 0), &[], &outside);
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
        let v = assess_pool(
            &pool_of(0, 0),
            std::slice::from_ref(&shared),
            std::slice::from_ref(&shared),
        );
        assert_eq!(
            v.admit,
            vec![Admission {
                number: 90,
                origin: CandidateOrigin::Milestone,
            }],
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
        let v = assess_pool(&pool_of(3, 0), &inside, &outside);
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
        let v = assess_pool(&pool_of(4, 0), &inside, &[]);
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
            vec![Admission {
                number: 1700,
                origin: CandidateOrigin::Outside
            }],
            "la provenance se lit depuis le fil, pas seulement depuis le rendu"
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
}

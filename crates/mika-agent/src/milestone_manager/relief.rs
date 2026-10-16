//! La relève à grommer — les prochains tickets derrière le bassin `ready`
//! (mika#2700 Phase 2 (c), vol 1).
//!
//! LECTURE SEULE, et ici **mode ombre** : ce vol nomme la relève et s'arrête
//! là. Aucun interrupteur, aucun geste, aucun déclenchement de groom. Le vol 2
//! portera `MIKA_MANAGER_GROOM_WRITE` et le déclenchement par la voie moteur
//! existante (le groom passe par mika-dev, jamais par un spawn) ; tant qu'il
//! n'est pas là, la propriété « avec l'interrupteur OFF, aucun déclenchement de
//! groom » (AC1) tient sous sa forme la plus forte — **il n'existe aucun
//! chemin d'écriture à fermer**, et le balayage de `no_dispatch_test.rs` le
//! prouve sans exception.
//!
//! Fonction **pure** : pas d'`async`, pas d'E/S, pas de lecture
//! d'environnement. Les quatre décisions de ce module :
//!
//! 1. **La relève dérive des refus du bassin et ne refiltre rien.** Tout ce
//!    qu'il faut pour la nommer est déjà dans [`PoolVerdict::refusals`] :
//!    `assess_pool` a écarté le fermé, le déjà-`ready`, le déjà en vol, puis a
//!    nommé ce qui reste comme refus de forme. Refiltrer ici serait un second
//!    lecteur de la même décision (doctrine mika#2201) — et c'est exactement
//!    cette dérivation qui donne l'AC2 structurellement : un ticket déjà groomé
//!    n'est pas un refus, donc n'est pas une relève ; un ticket en cours de
//!    groom porte un siège `dispatch:`, donc n'est pas un refus non plus.
//! 2. **L'ordre est celui du bassin**, par [`pool_sort_key`] et non par une
//!    seconde écriture du triplet. Deux ordres divergents dans le même rapport
//!    ne rougissent aucune assertion de comportement.
//! 3. **Un refus dont le candidat ne se retrouve pas passe en queue, sans
//!    disparaître.** Le rang de priorité est relu dans les deux ensembles de
//!    candidats, qui sont ceux-là mêmes que `assess_pool` a jugés ; s'il
//!    manque quand même, le ticket entre avec `priority_rank: None` et finit
//!    dernier. Le laisser tomber ferait mentir le compte des restants sur une
//!    liste dont l'opérateur ne verrait jamais le trou.
//! 4. **`beyond` est un compte, pas un booléen.** « 7 autres candidats » dit à
//!    l'opérateur si le bassin a du fond ; « il en reste » ne le dit pas.
//!
//! Le motif, lui, réutilise [`RefusalReason`] : un `enum` jumeau divergerait du
//! bassin au premier variant neuf, et le motif de la relève *est* le motif du
//! refus.

use serde::{Deserialize, Serialize};

use super::pool::{CandidateOrigin, PoolCandidate, PoolVerdict, RefusalReason, pool_sort_key};

/// Combien de tickets la relève nomme par cycle — les « 4 prochains » du
/// point 1 du ticket.
pub const RELIEF_SIZE: usize = 4;

/// Un ticket nommé dans la relève, avec son motif.
///
/// Porte son `priority_rank` parce que le rapport en a besoin pour être
/// arbitrable et que la clé de tri en a besoin pour être rejouable : sans lui,
/// l'ordre ne serait vérifiable qu'en refaisant la lecture des candidats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReliefTicket {
    pub number: u64,
    pub origin: CandidateOrigin,
    pub priority_rank: Option<u8>,
    /// Pourquoi ce ticket doit être groomé — **le motif du bassin**, pas une
    /// reformulation.
    pub motive: RefusalReason,
}

/// La relève de ce cycle : les prochains nommés, et le compte du reste.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReliefVerdict {
    /// Au plus [`RELIEF_SIZE`] tickets, dans l'ordre de tri du bassin.
    pub next: Vec<ReliefTicket>,
    /// Combien de candidats restent **au-delà** des nommés. `0` quand la relève
    /// tient tout entière.
    #[serde(default)]
    pub beyond: usize,
}

/// Nomme la relève à grommer depuis le verdict du bassin — fonction **pure**.
///
/// Les deux ensembles de candidats sont ceux que [`super::pool::assess_pool`] a
/// jugés : ils ne servent ici qu'à **relire** le rang de priorité d'un refus,
/// que [`super::pool::AdmissionRefusal`] ne porte pas. Aucun filtre ne
/// s'applique une seconde fois — voir la décision 1 du docstring du module.
pub fn name_relief(
    verdict: &PoolVerdict,
    milestone_candidates: &[PoolCandidate],
    outside_candidates: &[PoolCandidate],
) -> ReliefVerdict {
    let mut candidates: Vec<ReliefTicket> = verdict
        .refusals
        .iter()
        .map(|r| ReliefTicket {
            number: r.number,
            origin: r.origin,
            priority_rank: rank_of(r.number, milestone_candidates, outside_candidates),
            motive: r.reason,
        })
        .collect();

    candidates.sort_by_key(|t| pool_sort_key(t.priority_rank, t.origin, t.number));

    let beyond = candidates.len().saturating_sub(RELIEF_SIZE);
    candidates.truncate(RELIEF_SIZE);

    ReliefVerdict {
        next: candidates,
        beyond,
    }
}

/// Le rang de priorité d'un numéro, cherché dans le milestone **d'abord**.
///
/// Même ordre que celui dont `assess_pool` tire la provenance (sa décision 4),
/// pour qu'un ticket présent dans les deux ensembles soit lu avec le rang que
/// le bassin lui a reconnu. `None` quand le numéro n'est dans aucun des deux :
/// voir la décision 3 du docstring du module.
fn rank_of(
    number: u64,
    milestone_candidates: &[PoolCandidate],
    outside_candidates: &[PoolCandidate],
) -> Option<u8> {
    milestone_candidates
        .iter()
        .chain(outside_candidates.iter())
        .find(|c| c.number == number)
        .and_then(|c| c.priority_rank)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::milestone_manager::pool::{
        AdmissionRefusal, PoolBand, PoolDepth, ReadyTicket, assess_pool,
    };
    use crate::milestone_manager::types::IssueState;

    fn cand(number: u64, rank: Option<u8>, plan_present: bool, labels: &[&str]) -> PoolCandidate {
        PoolCandidate {
            number,
            state: IssueState::Open,
            priority_rank: rank,
            plan_present,
            labels: labels.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Un verdict minimal portant les refus voulus — la relève ne lit que ça.
    fn verdict_with(refusals: Vec<AdmissionRefusal>) -> PoolVerdict {
        PoolVerdict {
            depth: PoolDepth {
                ready_total: 3,
                in_flight: 0,
                available: 3,
            },
            band: PoolBand::Nominal,
            admit: vec![],
            refusals,
        }
    }

    fn refusal(number: u64, origin: CandidateOrigin) -> AdmissionRefusal {
        AdmissionRefusal {
            number,
            origin,
            reason: RefusalReason::NoGroomedPlan,
        }
    }

    fn numbers(v: &ReliefVerdict) -> Vec<u64> {
        v.next.iter().map(|t| t.number).collect()
    }

    /// Cas 1 — la relève est **exactement** les refus du bassin : rien ajouté,
    /// rien retiré.
    ///
    /// La propriété qui compte n'est pas « la relève est non vide » mais « son
    /// ensemble est celui des refus ». Un filtre de plus ici, ou une source
    /// autre que `refusals`, se verrait comme un écart d'ensemble.
    #[test]
    fn mika2700_la_releve_est_exactement_les_refus_du_bassin() {
        let milestone = vec![
            cand(2731, Some(1), false, &[]),
            cand(2732, Some(2), false, &[]),
        ];
        let outside = vec![cand(2740, Some(2), false, &[])];
        let v = assess_pool(&[], &milestone, &outside);
        let relief = name_relief(&v, &milestone, &outside);

        let mut attendus: Vec<u64> = v.refusals.iter().map(|r| r.number).collect();
        attendus.sort_unstable();
        let mut obtenus = numbers(&relief);
        obtenus.sort_unstable();
        assert_eq!(
            obtenus, attendus,
            "la relève est l'ensemble des refus du bassin, ni plus ni moins"
        );
        assert_eq!(
            relief.beyond, 0,
            "trois refus tiennent dans les quatre noms"
        );
    }

    /// Cas 2 — la relève suit l'ordre de tri du bassin : priorité, puis
    /// provenance, puis numéro.
    #[test]
    fn mika2700_la_releve_suit_lordre_de_tri_du_bassin() {
        // Semé dans le désordre exprès : le p0 du dehors doit passer devant le
        // p1 du milestone, et les deux p1 se départager par la provenance.
        let milestone = vec![
            cand(2800, Some(1), false, &[]),
            cand(2801, Some(1), false, &[]),
        ];
        let outside = vec![
            cand(2700, Some(1), false, &[]),
            cand(2600, Some(0), false, &[]),
        ];
        let v = assess_pool(&[], &milestone, &outside);
        let relief = name_relief(&v, &milestone, &outside);

        assert_eq!(
            numbers(&relief),
            vec![2600, 2800, 2801, 2700],
            "p0 hors milestone d'abord ; puis les p1 du milestone par numéro ; \
             puis le p1 du dehors"
        );
        assert_eq!(
            relief.next[0].origin,
            CandidateOrigin::Outside,
            "la provenance voyage avec le ticket — le rapport la dit"
        );
    }

    /// Cas 3 — un ticket **déjà groomé** n'est pas une relève (AC2).
    ///
    /// Il n'a aucun défaut de forme, donc `assess_pool` ne le refuse pas, donc
    /// il n'entre pas dans la relève. La propriété est structurelle : elle
    /// tient par la dérivation, pas par un filtre de ce module.
    #[test]
    fn mika2700_un_ticket_deja_groome_nest_pas_releve() {
        let milestone = vec![
            cand(2810, Some(1), true, &[]),  // plan groomé présent
            cand(2811, Some(1), false, &[]), // à grommer
        ];
        let v = assess_pool(&[], &milestone, &[]);
        let relief = name_relief(&v, &milestone, &[]);

        assert_eq!(
            numbers(&relief),
            vec![2811],
            "seul le ticket sans plan groomé est nommé ; #2810 est déjà fait"
        );
    }

    /// Cas 4 — un ticket **en cours de groom** n'est pas une relève, et le
    /// contrôle négatif le prouve (AC2).
    ///
    /// Les deux moitiés d'un même semis, le siège `dispatch:` en seule
    /// différence : sans la paire, une relève vide se lirait comme un filtre
    /// qui marche alors qu'elle pourrait être vide pour toute autre raison.
    #[test]
    fn mika2700_un_ticket_en_cours_de_groom_nest_pas_releve() {
        let en_vol = vec![cand(2820, Some(1), false, &["dispatch:groom"])];
        let v = assess_pool(&[], &en_vol, &[]);
        let relief = name_relief(&v, &en_vol, &[]);
        assert!(
            relief.next.is_empty() && relief.beyond == 0,
            "un groom déjà en cours n'est pas relancé — ni nommé, ni compté"
        );

        let libre = vec![cand(2820, Some(1), false, &[])];
        let v = assess_pool(&[], &libre, &[]);
        let relief = name_relief(&v, &libre, &[]);
        assert_eq!(
            numbers(&relief),
            vec![2820],
            "contrôle négatif : le même ticket sans siège de dispatch est bien \
             nommé — c'est le siège qui l'écarte, pas le hasard du semis"
        );
    }

    /// Cas 5 — la relève est bornée à quatre et **dit** le reste.
    #[test]
    fn mika2700_la_releve_est_bornee_a_quatre_et_dit_le_reste() {
        let milestone: Vec<PoolCandidate> = (0..11)
            .map(|i| cand(2900 + i, Some(1), false, &[]))
            .collect();
        let v = assess_pool(&[], &milestone, &[]);
        let relief = name_relief(&v, &milestone, &[]);

        assert_eq!(
            numbers(&relief),
            vec![2900, 2901, 2902, 2903],
            "les quatre premiers de l'ordre du bassin"
        );
        assert_eq!(
            relief.next.len(),
            RELIEF_SIZE,
            "la borne est `RELIEF_SIZE`, pas un 4 littéral"
        );
        assert_eq!(
            relief.beyond, 7,
            "un compte, pas un booléen : « 7 autres » dit si le bassin a du fond"
        );
    }

    /// Cas 6 — un refus dont le candidat ne se retrouve pas passe en queue
    /// **sans disparaître**.
    ///
    /// Le cas arrive si les deux lectures `gh` divergent d'un cycle. Le laisser
    /// tomber ferait mentir `beyond` sur un trou que personne ne verrait.
    #[test]
    fn mika2700_un_refus_sans_candidat_retrouve_passe_en_queue_sans_disparaitre() {
        let milestone = vec![cand(3000, Some(2), false, &[])];
        let v = verdict_with(vec![
            refusal(3000, CandidateOrigin::Milestone),
            // Aucun candidat ne porte ce numéro : rang inconnu.
            refusal(10, CandidateOrigin::Milestone),
        ]);
        let relief = name_relief(&v, &milestone, &[]);

        assert_eq!(
            numbers(&relief),
            vec![3000, 10],
            "le refus sans rang finit dernier malgré son petit numéro — et il \
             est bien là"
        );
        assert_eq!(
            relief.next[1].priority_rank, None,
            "son rang est dit absent, pas inventé à 0 (motif mika#2331)"
        );
    }

    /// Cas 7 — un bassin sans refus ne nomme personne.
    #[test]
    fn mika2700_un_bassin_sans_refus_ne_nomme_personne() {
        let pool = vec![ReadyTicket {
            number: 1,
            labels: vec!["ready".into()],
        }];
        let milestone = vec![cand(3100, Some(1), true, &[])];
        let v = assess_pool(&pool, &milestone, &[]);
        assert!(v.refusals.is_empty(), "prémisse du cas : aucun refus");

        let relief = name_relief(&v, &milestone, &[]);
        assert!(
            relief.next.is_empty(),
            "rien à grommer d'avance, donc personne de nommé"
        );
        assert_eq!(relief.beyond, 0, "et rien au-delà non plus");
    }

    /// Cas 8 — le motif est celui du bassin, pas une reformulation.
    ///
    /// L'égalité est sur le variant lui-même : un `enum` jumeau divergerait au
    /// premier motif neuf du bassin, et le rapport dirait alors « à grommer »
    /// d'un ticket refusé pour une autre raison.
    #[test]
    fn mika2700_le_motif_est_celui_du_bassin() {
        let milestone = vec![cand(3200, Some(1), false, &[])];
        let v = assess_pool(&[], &milestone, &[]);
        let relief = name_relief(&v, &milestone, &[]);

        assert_eq!(
            relief.next[0].motive, v.refusals[0].reason,
            "le motif de la relève est le motif du refus, au variant près"
        );
        assert_eq!(
            relief.next[0].motive.label(),
            "plan groomé absent",
            "et son libellé vit dans `RefusalReason`, donc un variant neuf ne \
             peut pas être rendu sans libellé"
        );
    }
}

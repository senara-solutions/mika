//! Pool — le bassin `ready` : sa profondeur, sa bande, et qui devrait y entrer.
//!
//! **LECTURE SEULE.** mika#2699 Phase 2 (a) **vol 1** : le manager *voit* le
//! bassin et *dit* qui devrait y entrer. Il ne pose aucun label, ne rattache
//! aucun milestone, ne déclare aucun interrupteur. `assess_pool` est une
//! fonction pure sans E/S ; le verdict entre dans le rapport et nulle part
//! ailleurs. L'écriture (`MIKA_MANAGER_POOL_WRITE` et le geste groupé) est le
//! vol 3, avec son exception nommée à `no_dispatch_test.rs`.
//!
//! # Le bassin n'est pas une propriété du milestone cible
//!
//! Le webhook `ready` dispatche à l'échelle du **dépôt**, pas du milestone. Un
//! bassin compté sur les seules `sub_issues` du milestone serait sous-compté, et
//! la recommandation fausse exactement le jour où elle compte. Mesuré le
//! 2026-10-15 : 2 issues `ready` ouvertes (mika#2699, mika#2687), **les deux**
//! portant `dispatch:loop` — profondeur disponible 0, bande *famine*. Un bassin
//! lu sur le seul milestone aurait affiché « 2, nominal ». D'où deux entrées
//! distinctes : `pool` vient d'une lecture dépôt-large
//! ([`super::reader::compose_ready_pool`]), `candidates` des `sub_issues` du
//! milestone.
//!
//! # Ce que le vol 1 ne dit pas
//!
//! Il **compte** les candidats écartés faute de plan groomé
//! ([`PoolVerdict::withheld_no_plan`]) sans les **nommer**, et ne connaît pas le
//! second motif de refus (estimation au-dessus de la borne par vol). Les deux
//! sont le vol 2, qui a de toute façon besoin d'élargir la lecture aux candidats
//! hors milestone cible.

use super::types::{IssueState, SubIssue};
use crate::webhook_dispatch::{DISPATCH_SEAT_LABEL_PREFIX, READY_LABEL};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

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

/// Le verdict du bassin pour ce cycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolVerdict {
    pub depth: PoolDepth,
    pub band: PoolBand,
    /// Numéros à faire entrer, dans l'ordre d'admission. Vide hors famine.
    pub admit: Vec<u64>,
    /// Combien de candidats du milestone ont été écartés faute de plan groomé.
    ///
    /// Existe pour une seule raison : un filtre muet est indistinguable d'un
    /// bassin sans candidat. Le vol 1 ne dit pas *lesquels* (c'est le vol 2),
    /// mais il dit **combien**, pour que l'opérateur sache qu'il y a eu filtre.
    pub withheld_no_plan: usize,
}

/// Juge le bassin et nomme les admissions — fonction **pure**, sans E/S.
///
/// Quatre décisions, chacune pour une raison nommable :
///
/// 1. **C'est `available` qui décide la bande**, pas `ready_total`. Un ticket
///    `ready` déjà en vol n'est pas du rail disponible.
/// 2. **Hystérésis.** On ne recommande une admission que **sous** [`POOL_MIN`],
///    et on recommande alors de remonter à [`POOL_MAX`]. Sans elle, à 3
///    disponibles le manager recommanderait un ticket à chaque cycle jusqu'à 4,
///    puis à nouveau dès qu'un vol démarre : un geste par cycle, soit exactement
///    le bruit que l'orchestrateur évite à la main.
/// 3. **Le tri est `(priority_rank.unwrap_or(u8::MAX), number)`** — la même clé
///    que les paliers de `assessor::pick_next_action`. Un ticket sans label de
///    priorité passe **en dernier** : l'absence de priorité n'est pas p0.
/// 4. **Candidats = les `sub_issues` ouvertes du milestone cible**, filtrées :
///    pas déjà `ready`, pas en vol, `plan_present == true`. Le « avant le
///    reste » du ticket (candidats hors milestone) est le vol 2. Ce que le vol 1
///    livre est donc la moitié haute de l'ordre demandé, et le rapport le dit.
pub fn assess_pool(pool: &[ReadyTicket], candidates: &[SubIssue]) -> PoolVerdict {
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

    let mut eligible: Vec<&SubIssue> = Vec::new();
    let mut withheld_no_plan = 0usize;
    for s in candidates {
        if s.state != IssueState::Open {
            continue;
        }
        if already_ready.contains(&s.number) || labels_carry_ready(&s.labels) {
            continue;
        }
        if labels_hold_dispatch_seat(&s.labels) {
            continue;
        }
        if !s.plan_present {
            withheld_no_plan += 1;
            continue;
        }
        eligible.push(s);
    }
    eligible.sort_by_key(|s| (s.priority_rank.unwrap_or(u8::MAX), s.number));

    // `POOL_MAX - available` ne peut pas déborder : la bande famine impose
    // `available < POOL_MIN`, et `POOL_MIN <= POOL_MAX`.
    let admit: Vec<u64> = match band {
        PoolBand::Starved => eligible
            .iter()
            .take(POOL_MAX - available)
            .map(|s| s.number)
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
        withheld_no_plan,
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
    use crate::milestone_manager::types::CiState;

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

    fn cand(number: u64, rank: Option<u8>, plan_present: bool, labels: &[&str]) -> SubIssue {
        SubIssue {
            number,
            title: format!("candidat #{number}"),
            state: IssueState::Open,
            priority_rank: rank,
            plan_present,
            branch_present: false,
            pr_number: None,
            pr_state: None,
            ci_state: CiState::None,
            blockers: vec![],
            updated_at: "2026-10-15T00:00:00Z".into(),
            labels: labels.iter().map(|s| s.to_string()).collect(),
        }
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
            let v = assess_pool(&pool_of(n, 0), &[]);
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
        let v = assess_pool(&pool_of(3, 2), &[]);
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
        let v = assess_pool(&pool, &[]);
        assert_eq!(v.depth.in_flight, 2, "les deux sièges comptent: {:?}", v);
        assert_eq!(v.depth.available, 1);
    }

    /// Cas 4 — hystérésis : muette entre 2 et 4, et en famine elle remonte à
    /// [`POOL_MAX`], pas d'un cran.
    #[test]
    fn mika2699_lhysteresis_ne_parle_quen_famine() {
        let candidates: Vec<SubIssue> = (1..=6).map(|n| cand(n, Some(1), true, &[])).collect();

        let nominal = assess_pool(&pool_of(3, 0), &candidates);
        assert!(
            nominal.admit.is_empty(),
            "à 3 disponibles, rien à recommander: {:?}",
            nominal.admit
        );

        let starved = assess_pool(&pool_of(1, 0), &candidates);
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
        let v = assess_pool(&pool_of(0, 0), &candidates);
        assert_eq!(
            v.admit,
            vec![10, 20, 30, 40],
            "ordre (priorité, numéro) attendu, obtenu {:?}",
            v.admit
        );
        assert!(
            !v.admit.contains(&50),
            "le non-priorisé ne passe pas devant un p2"
        );
    }

    /// Cas 6 — le filtre « plan groomé présent » : jamais dans `admit`, et
    /// compté, pour qu'un filtre muet ne se lise pas comme un bassin sans
    /// candidat.
    #[test]
    fn mika2699_un_candidat_sans_plan_est_ecarte_et_compte() {
        let candidates = vec![
            cand(10, Some(0), false, &[]),
            cand(11, Some(0), false, &[]),
            cand(12, Some(2), true, &[]),
        ];
        let v = assess_pool(&pool_of(0, 0), &candidates);
        assert_eq!(v.admit, vec![12], "seul le candidat groomé entre");
        assert_eq!(v.withheld_no_plan, 2, "les deux non-groomés sont comptés");
    }

    /// Cas 7 — déjà `ready` (par le bassin **ou** par ses propres labels), déjà
    /// en vol, ou fermé : jamais recommandé, et jamais compté comme écarté faute
    /// de plan (ce ne serait pas le motif).
    #[test]
    fn mika2699_deja_ready_deja_en_vol_ou_ferme_nest_pas_recommande() {
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
        let v = assess_pool(&pool, &candidates);
        assert_eq!(v.admit, vec![14], "seul #14 est admissible: {:?}", v.admit);
        assert_eq!(v.withheld_no_plan, 0);
    }

    /// Cas 8 — bassin vide et aucun candidat : famine, `admit` vide, pas de
    /// panique (et surtout pas de soustraction qui déborde).
    #[test]
    fn mika2699_bassin_vide_sans_candidat_ne_panique_pas() {
        let v = assess_pool(&[], &[]);
        assert_eq!(v.depth.available, 0);
        assert_eq!(v.band, PoolBand::Starved);
        assert!(v.admit.is_empty());
        assert_eq!(v.withheld_no_plan, 0);
    }
}

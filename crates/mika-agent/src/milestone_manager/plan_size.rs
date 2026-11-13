//! Plan size — le portage Rust du contrat de taille de plan de `dispatch-lib.sh`.
//!
//! **LECTURE SEULE, et pure.** Ce module ne connaît ni `gh`, ni le système de
//! fichiers : il prend le **texte** d'un plan et rend ce que le bash rend, plus
//! l'arithmétique de la borne par vol. Qui va chercher le texte est l'affaire de
//! [`super::reader`] ; qui en tire un refus est celle de [`super::pool`].
//!
//! # Ce qui est porté, et ce qui ne l'est pas
//!
//! Le modèle est `_plan_size_total_loc` et `_plan_size_per_flight_bound`
//! (`skills/bundled/_shared/dispatch-lib.sh`, porte 3 de mika#2701). Trois
//! décisions du bash survivent au portage parce qu'elles ont chacune été payées
//! par une mesure, et qu'un portage qui les perdrait rendrait un nombre
//! plausible et faux :
//!
//! 1. **Trois jetons distincts, et aucun n'est `0`.** [`PlanSizeTotal`] a autant
//!    de variantes que le bash a de sorties — `absent`, `unparsable`, `<n>` —
//!    parce que les remèdes sont opposés : groomer le plan, ou réparer le motif.
//!    Les confondre, ou rendre `0` pour l'un des deux, serait *un `null` pris
//!    pour un `0`* (mika#2331).
//! 2. **Le total est cherché DANS la section, jamais dans tout le document.** La
//!    règle de grooming ordonne, au-dessus du seuil, de découper en phases et de
//!    renvoyer la suite à un ticket de suivi : un plan **conforme** porte donc
//!    légitimement un second `Total estimé :` hors section. Mesuré côté bash :
//!    une section annonçant 850 et un § hors périmètre annonçant 1900 rendaient
//!    `1900`.
//! 3. **Les blocs clôturés sont retirés avant toute lecture.** Un plan qui
//!    *documente* le format dans un bloc ```` ```markdown ```` porte un titre
//!    `## Taille estimée` et une ligne `Total estimé :` qui sont l'exemple, pas
//!    la mesure. Sur nombre de fences **impair**, rien n'est retiré : le corps
//!    est ambigu et l'arbitrage va dans le sens du faux positif, comme
//!    mika#2120.
//!
//! Ce qui n'est **pas** porté, et pourquoi : le repli `disarmed_fallback` du
//! bash, qui couvre un plafond absent de l'argv. Ici la cascade de plafond rend
//! toujours un nombre ([`turn_ceiling_for_labels`]), donc une variante
//! `disarmed_fallback` serait une branche que rien ne peut atteindre — verte,
//! plausible, mesurant zéro (classe mika#2205).
//!
//! # Le plafond de tours : deux paliers sur quatre, et c'est nommé
//!
//! La cascade de `_pilot_max_turns` a quatre paliers ; celle d'ici en porte
//! deux — le label (palier 2) et le défaut in-file (palier 4). Les deux paliers
//! d'environnement sont ceux du **processus hôte du dispatch**, et le manager
//! est un autre processus : lire son propre `PILOT_MAX_TURNS` mesurerait
//! l'environnement du manager, pas celui du vol à venir.
//!
//! L'écart que cela laisse est borné, et dans le sens sûr :
//!
//! - palier 2 **domine** le palier 3, donc pour un ticket `loop-substrate` la
//!   valeur d'environnement est inerte **par construction** ;
//! - l'hôte de production porte `PILOT_MAX_TURNS=150`, c'est-à-dire exactement
//!   le défaut in-file, donc les deux cascades coïncident aujourd'hui sur toute
//!   la population ;
//! - reste le palier 1 (`PILOT_MAX_TURNS=""` ou `0`, le rollback d'incident).
//!   Pendant un rollback le manager surestime le plafond, donc la borne, donc il
//!   **sous-refuse**. Un refus manqué laisse un ticket entrer au bassin ; le
//!   contraire aurait gelé des admissions pendant un incident.
//!
//! # La table des constantes a deux sites, et un test les tient ensemble
//!
//! Le bash reste le site d'exécution ; ce module en est une copie, et une copie
//! est une divergence qui attend. Le garde est
//! `mika2699_le_portage_et_le_bash_saccordent_dans_les_deux_sens` : il lit
//! `dispatch-lib.sh` et compare **dans les deux sens** les quatre constantes
//! d'arithmétique, la table label → plafond, le défaut de flotte et les deux
//! motifs d'expression régulière. C'est le motif de
//! `scripts/check-dispatch-seats-declared.sh` (mika#2092) : les deux listes
//! doivent bouger dans le même commit, et le test dit laquelle a bougé seule.

use regex::Regex;
use std::sync::OnceLock;

/// Tours consommés par l'orientation d'un vol, avant la première ligne écrite.
pub const FLIGHT_ORIENTATION_TURNS: u32 = 27;

/// Tours consommés par la queue d'expédition (vérification, commit, PR).
pub const FLIGHT_TAIL_TURNS: u32 = 30;

/// Lignes réellement produites pour 100 tours de travail utile.
pub const FLIGHT_LOC_PER_100_TURNS: u32 = 900;

/// Facteur, en pourcent, entre l'estimation d'un plan et les lignes livrées.
pub const FLIGHT_ESTIMATE_FACTOR_PCT: u32 = 300;

/// Le défaut de flotte du plafond de tours — palier 4 de `_pilot_max_turns`.
pub const DEFAULT_TURN_CEILING: u32 = 150;

/// La table label → plafond de tours — palier 2 de `_pilot_max_turns`.
///
/// La première entrée qui apparie décide, comme en bash. L'appariement est
/// **exact** sur un label entier, jamais sur une sous-chaîne : un glob
/// apparierait `not-loop-substrate` ou `loop-substrate-v2`, tolérable pour un
/// préfixe de commit, faux pour un frein de coût.
pub const LABEL_TURN_CEILINGS: &[(&str, u32)] = &[("loop-substrate", 200)];

/// Le plafond de tours qu'un vol sur ce ticket recevrait, d'après ses labels.
pub fn turn_ceiling_for_labels(labels: &[String]) -> u32 {
    for (label, ceiling) in LABEL_TURN_CEILINGS {
        if labels.iter().any(|l| l == label) {
            return *ceiling;
        }
    }
    DEFAULT_TURN_CEILING
}

/// La borne par vol, en lignes estimées, dérivée d'un plafond de tours.
///
/// `(plafond − orientation − queue) × lignes_par_100_tours / facteur_pct`, soit
/// 429 pour 200 tours et 279 pour 150. Sur un plafond qui ne laisse aucun tour
/// de travail, la borne est **0** et jamais un nombre négatif : un plafond sous
/// le plancher du pipeline ne mesure pas un budget d'écriture, il dit qu'il n'y
/// en a pas.
pub fn per_flight_bound(ceiling: u32) -> u32 {
    let overhead = FLIGHT_ORIENTATION_TURNS + FLIGHT_TAIL_TURNS;
    if ceiling <= overhead {
        return 0;
    }
    (ceiling - overhead) * FLIGHT_LOC_PER_100_TURNS / FLIGHT_ESTIMATE_FACTOR_PCT
}

/// Ce que la section `## Taille estimée` d'un plan annonce — les trois jetons de
/// `_plan_size_total_loc`, et exactement eux.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanSizeTotal {
    /// Le total annoncé par la section.
    Total(u64),
    /// Jeton `absent` — le plan ne porte pas de section `## Taille estimée`.
    SectionAbsent,
    /// Jeton `unparsable` — la section est là, sa ligne `Total estimé :` non.
    Unparsable,
}

/// Le total annoncé par un plan, sous la forme du contrat à trois jetons.
///
/// `plan` est le **texte** du plan ; le bash rend `absent` aussi pour un fichier
/// illisible, cas qui n'existe pas ici — la lecture distante échoue chez son
/// appelant, qui la nomme autrement plutôt que de la confondre avec un plan non
/// dimensionné.
pub fn plan_size_total_loc(plan: &str) -> PlanSizeTotal {
    let Some(section) = section_body(plan) else {
        return PlanSizeTotal::SectionAbsent;
    };
    // **Le dernier match de la section gagne**, comme en bash (`tail -1`) : un
    // plan peut porter un sous-total par livrable avant le total qui conclut.
    match total_re().captures_iter(&section).last() {
        Some(c) => c[1]
            .parse::<u64>()
            .map_or(PlanSizeTotal::Unparsable, PlanSizeTotal::Total),
        None => PlanSizeTotal::Unparsable,
    }
}

/// Le corps de la section, titre compris, ou rien — site unique du fenêtrage.
///
/// La borne de fin est le prochain titre de niveau 2, exclu, exactement comme
/// l'`awk` du bash. Le titre d'ouverture est pris **avant** que la condition de
/// fin ne soit évaluée, sans quoi il se fermerait lui-même.
fn section_body(plan: &str) -> Option<String> {
    let body = strip_fences(plan);
    let mut out: Vec<&str> = Vec::new();
    for line in body.lines() {
        if out.is_empty() {
            if heading_re().is_match(line) {
                out.push(line);
            }
            continue;
        }
        if next_h2_re().is_match(line) {
            break;
        }
        out.push(line);
    }
    (!out.is_empty()).then(|| out.join("\n"))
}

/// Le corps privé de ses blocs clôturés — ```` ``` ```` comme `~~~`.
///
/// **Nombre de marqueurs impair ⇒ rien n'est retiré.** Le corps est alors
/// ambigu, et le sens de l'arbitrage est celui de mika#2120 : un faux positif
/// coûte une relance, un faux négatif a coûté quinze heures de boucle.
///
/// La bascule se fait sur **n'importe quel** marqueur, sans suivre lequel a
/// ouvert — c'est le comportement de l'`awk` porté, et il diffère de
/// [`crate::plan_callout::strip_fenced_blocks`], qui apparie les deux formes.
/// L'écart est **dit** plutôt que corrigé : ce module porte un contrat, il ne le
/// réécrit pas.
fn strip_fences(plan: &str) -> String {
    let fence = fence_re();
    if plan.lines().filter(|l| fence.is_match(l)).count() % 2 != 0 {
        return plan.to_string();
    }
    let mut inside = false;
    let mut out: Vec<&str> = Vec::new();
    for line in plan.lines() {
        if fence.is_match(line) {
            inside = !inside;
            continue;
        }
        if !inside {
            out.push(line);
        }
    }
    out.join("\n")
}

/// Le titre de section, casse repliée — `_PLAN_SIZE_HEADING_RE` avec `grep -i`.
///
/// `[ \t]` et non `[[:space:]]` : la classe POSIX contient `\n`, inoffensif pour
/// un `grep` ligne à ligne mais pas pour un moteur qui voit tout le texte.
fn heading_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)^##[ \t]+([0-9]+\.?[ \t]+)?Taille estimée")
            .expect("plan size heading regex must compile")
    })
}

/// La borne de fin de section : le prochain titre de niveau 2.
fn next_h2_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^##[ \t]").expect("h2 boundary regex must compile"))
}

/// La ligne de total — `_PLAN_SIZE_TOTAL_RE`, **sensible à la casse**.
///
/// Borne nommée, et c'est celle du bash : le titre replie la casse, le total
/// non. La forme prescrite est exacte sur cette ligne, et replier la casse ici
/// n'achèterait rien de mesuré.
fn total_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?m)^Total estimé[ \t]*:[ \t]*([0-9]+)([ \t]+[[:alpha:]].*)?$")
            .expect("plan size total regex must compile")
    })
}

/// Un marqueur de bloc clôturé en tête de ligne.
fn fence_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[ \t]*(```|~~~)").expect("fence regex must compile"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Le plan modèle : une section conforme, et un second total hors section
    /// — la population que la règle de grooming **prescrit**.
    fn plan_avec_suite_differee() -> String {
        "# Plan\n\
         \n\
         ## 6. Taille estimée\n\
         \n\
         | fichier | lignes |\n\
         |---|---|\n\
         Total estimé : 850 lignes\n\
         \n\
         ## 7. Hors périmètre, renvoyé à un ticket de suivi\n\
         \n\
         Total estimé : 1900 lignes\n"
            .to_string()
    }

    #[test]
    fn mika2699_le_total_est_celui_de_la_section_pas_celui_du_document() {
        assert_eq!(
            plan_size_total_loc(&plan_avec_suite_differee()),
            PlanSizeTotal::Total(850),
            "le § hors périmètre porte 1900 ; le lire rendrait over_bound sur un périmètre de 850"
        );
    }

    #[test]
    fn mika2699_dans_la_section_le_dernier_total_gagne() {
        let plan = "## Taille estimée\n\
                    Total estimé : 120 lignes\n\
                    Total estimé : 300 lignes\n";
        assert_eq!(
            plan_size_total_loc(plan),
            PlanSizeTotal::Total(300),
            "un sous-total par livrable précède le total qui conclut"
        );
    }

    #[test]
    fn mika2699_un_plan_qui_documente_le_format_nest_pas_un_plan_dimensionne() {
        // Le titre ET le total sont dans un bloc clôturé : l'exemple, pas la
        // mesure. Un lecteur naïf rendrait 395 pour un plan non dimensionné.
        let plan = "# Plan\n\
                    \n\
                    ```markdown\n\
                    ## Taille estimée\n\
                    Total estimé : 395 lignes\n\
                    ```\n\
                    \n\
                    ## Autre section\n";
        assert_eq!(plan_size_total_loc(plan), PlanSizeTotal::SectionAbsent);

        // `~~~` ferme exactement comme les accents graves — ne reconnaître
        // qu'une des deux formes laisserait un trou sur du Markdown valide.
        let tildes = plan.replace("```", "~~~");
        assert_eq!(plan_size_total_loc(&tildes), PlanSizeTotal::SectionAbsent);
    }

    #[test]
    fn mika2699_un_fence_non_ferme_ne_retire_rien() {
        // Nombre de marqueurs impair : le corps est ambigu, et l'arbitrage va
        // vers le faux positif (mika#2120).
        let plan = "## Taille estimée\n\
                    Total estimé : 410 lignes\n\
                    \n\
                    ```text\n\
                    une fence jamais refermée\n";
        assert_eq!(plan_size_total_loc(plan), PlanSizeTotal::Total(410));
    }

    #[test]
    fn mika2699_les_trois_jetons_restent_distincts() {
        // `absent` — aucune section.
        assert_eq!(
            plan_size_total_loc("# Plan\n\n## Approche\n\nRien de dimensionné.\n"),
            PlanSizeTotal::SectionAbsent
        );
        // `unparsable` — section présente, total non apparié.
        assert_eq!(
            plan_size_total_loc("## Taille estimée\n\nenviron quatre cents lignes\n"),
            PlanSizeTotal::Unparsable
        );
        // … et aucun des deux n'est un `Total(0)` : *un `null` n'est jamais un
        // `0`* (mika#2331).
        assert_ne!(
            plan_size_total_loc("## Taille estimée\n\nflou\n"),
            PlanSizeTotal::Total(0)
        );
    }

    #[test]
    fn mika2699_le_titre_replie_la_casse_le_total_non() {
        // Titre : numéroté, casse repliée, comme `grep -i` côté bash.
        assert_eq!(
            plan_size_total_loc("## 3. TAILLE ESTIMÉE\nTotal estimé : 77 lignes\n"),
            PlanSizeTotal::Total(77)
        );
        // Total : sensible à la casse — borne nommée, pas un oubli.
        assert_eq!(
            plan_size_total_loc("## Taille estimée\ntotal estimé : 77 lignes\n"),
            PlanSizeTotal::Unparsable
        );
    }

    #[test]
    fn mika2699_la_section_sarrete_au_prochain_titre_de_niveau_2() {
        let plan = "## Taille estimée\n\
                    Total estimé : 100 lignes\n\
                    ### Détail\n\
                    Total estimé : 200 lignes\n\
                    ## Suite\n\
                    Total estimé : 999 lignes\n";
        assert_eq!(
            plan_size_total_loc(plan),
            PlanSizeTotal::Total(200),
            "un `###` reste dans la section ; un `##` la ferme"
        );
    }

    #[test]
    fn mika2699_la_borne_par_vol_derive_du_plafond() {
        assert_eq!(per_flight_bound(200), 429, "(200 − 57) × 900 / 300");
        assert_eq!(per_flight_bound(150), 279, "(150 − 57) × 900 / 300");
        // Plancher : un plafond qui ne laisse aucun tour de travail rend 0, et
        // jamais un nombre négatif.
        assert_eq!(per_flight_bound(57), 0);
        assert_eq!(per_flight_bound(10), 0);
    }

    #[test]
    fn mika2699_le_plafond_vient_du_label_sinon_du_defaut() {
        assert_eq!(
            turn_ceiling_for_labels(&["loop-substrate".to_string()]),
            200
        );
        assert_eq!(turn_ceiling_for_labels(&["p1-important".to_string()]), 150);
        assert_eq!(turn_ceiling_for_labels(&[]), DEFAULT_TURN_CEILING);
        // Appariement exact : un voisin de nom ne relève pas le plafond.
        assert_eq!(
            turn_ceiling_for_labels(&["not-loop-substrate".to_string()]),
            150
        );
        assert_eq!(
            turn_ceiling_for_labels(&["loop-substrate-v2".to_string()]),
            150
        );
    }

    /// Le garde des deux sites : `dispatch-lib.sh` exécute, ce module copie, et
    /// une copie qui dérive en silence rend un nombre plausible et faux.
    ///
    /// Comparé **dans les deux sens** — le motif de
    /// `scripts/check-dispatch-seats-declared.sh` (mika#2092). Un sens seul
    /// laisserait passer l'ajout d'une entrée du côté non vérifié, c'est-à-dire
    /// exactement la façon dont les deux listes cessent de bouger ensemble.
    #[test]
    fn mika2699_le_portage_et_le_bash_saccordent_dans_les_deux_sens() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../skills/bundled/_shared/dispatch-lib.sh");
        let sh = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("dispatch-lib.sh illisible en {}: {e}", path.display()));

        // 1. Les quatre constantes d'arithmétique, par leur nom bash.
        let attendu: &[(&str, u32)] = &[
            (
                "_PLAN_SIZE_FLIGHT_ORIENTATION_TURNS",
                FLIGHT_ORIENTATION_TURNS,
            ),
            ("_PLAN_SIZE_FLIGHT_TAIL_TURNS", FLIGHT_TAIL_TURNS),
            (
                "_PLAN_SIZE_FLIGHT_LOC_PER_100_TURNS",
                FLIGHT_LOC_PER_100_TURNS,
            ),
            (
                "_PLAN_SIZE_FLIGHT_ESTIMATE_FACTOR_PCT",
                FLIGHT_ESTIMATE_FACTOR_PCT,
            ),
        ];
        let assign = Regex::new(r"(?m)^(_PLAN_SIZE_FLIGHT_[A-Z0-9_]+)=([0-9]+)$").unwrap();
        let mut vus: Vec<(String, u32)> = assign
            .captures_iter(&sh)
            .map(|c| (c[1].to_string(), c[2].parse::<u32>().unwrap()))
            .collect();
        vus.sort();
        let mut veut: Vec<(String, u32)> = attendu
            .iter()
            .map(|(n, v)| ((*n).to_string(), *v))
            .collect();
        veut.sort();
        assert_eq!(
            vus, veut,
            "les constantes d'arithmétique de la borne par vol ont divergé entre \
             dispatch-lib.sh et ce module — les deux listes doivent bouger dans le même commit"
        );

        // 2. La table label → plafond, dans les deux sens.
        let bloc = sh
            .split_once("PILOT_LABEL_TURN_CEILINGS=(")
            .and_then(|(_, reste)| reste.split_once(')'))
            .map(|(bloc, _)| bloc.to_string())
            .expect("PILOT_LABEL_TURN_CEILINGS introuvable sous sa forme de fil");
        let entree = Regex::new(r#""([^"=]+)=([0-9]+)""#).unwrap();
        let mut table_sh: Vec<(String, u32)> = entree
            .captures_iter(&bloc)
            .map(|c| (c[1].to_string(), c[2].parse::<u32>().unwrap()))
            .collect();
        table_sh.sort();
        let mut table_rs: Vec<(String, u32)> = LABEL_TURN_CEILINGS
            .iter()
            .map(|(l, c)| ((*l).to_string(), *c))
            .collect();
        table_rs.sort();
        assert_eq!(
            table_sh, table_rs,
            "la table label → plafond a divergé ; un label non porté ici fait \
             sous-estimer la borne par vol du ticket qui le porte"
        );

        // 3. Le défaut de flotte, lu dans le corps de `_pilot_max_turns` — pas
        //    ailleurs : c'est le site qui décide, et un `150` trouvé dans un
        //    commentaire voisin ne mesurerait rien.
        let corps = sh
            .split_once("_pilot_max_turns() {")
            .map(|(_, reste)| reste)
            .expect("_pilot_max_turns introuvable");
        let defaut = Regex::new(r#"local _default="([0-9]+)""#)
            .unwrap()
            .captures(corps)
            .map(|c| c[1].parse::<u32>().unwrap())
            .expect("le défaut de flotte de _pilot_max_turns est introuvable");
        assert_eq!(
            defaut, DEFAULT_TURN_CEILING,
            "le défaut de flotte a bougé côté bash"
        );

        // 4. Les deux motifs dont ce module est la traduction. Fil-piège plutôt
        //    que ré-implémentation : si le bash change sa grammaire, ce test
        //    nomme le portage à refaire au lieu de laisser les deux lecteurs
        //    répondre différemment au même plan.
        for motif in [
            r"_PLAN_SIZE_HEADING_RE='^##[[:space:]]+([0-9]+\.?[[:space:]]+)?Taille estimée'",
            r"_PLAN_SIZE_TOTAL_RE='^Total estimé[[:space:]]*:[[:space:]]*([0-9]+)([[:space:]]+[[:alpha:]].*)?$'",
        ] {
            assert!(
                sh.contains(motif),
                "le motif bash dont ce module est la traduction a changé :\n  {motif}\n\
                 Reporter le changement sur `heading_re`/`total_re` dans le même commit."
            );
        }
    }
}

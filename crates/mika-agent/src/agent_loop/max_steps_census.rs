//! mika#2877 — à l'épuisement du budget de steps, **nommer les outils** qui
//! l'ont consommé.
//!
//! Le WARN voisin (« agent exceeded max tool steps », `mod.rs:5110-5113`) porte
//! `label` et `max_steps` : il dit *que* le budget a été épuisé et *quel* était
//! le plafond, jamais *par quoi*. Or les trois corrections candidates — relever
//! le plafond, corriger un outil qui boucle, raccourcir le prompt — se
//! départagent exactement sur cette donnée. Ce module livre l'instrument, pas la
//! correction : ni le plafond ni le comportement à l'épuisement ne changent.
//!
//! L'événement est **distinct** du WARN voisin, qui reste inchangé octet pour
//! octet. Le constat du ticket est un `grep "agent exceeded max tool steps"` qui
//! a rendu 204 lignes sur dix jours : cette ligne est le dénominateur de la
//! mesure, et y ajouter des champs réécrirait la ligne même que la série suit.
//! Un événement voisin n'y touche pas — l'ancienne courbe reste lisible, la
//! nouvelle commence à la date du déploiement, et les deux se comparent.
//!
//! # Trois propriétés de lecture
//!
//! **« Absent n'est jamais zéro » (mika#2331).** Un outil non appelé est
//! **absent** de `per_tool`, jamais présent à `0`. La carte énumère ce qui a
//! consommé, pas l'univers des outils.
//!
//! **Une carte vide est une mesure, pas une absence.** Un tour qui épuise ses
//! steps sans aucun appel d'outil rend `per_tool = {}` et `total_calls = 0` — et
//! c'est peut-être *la* réponse (budget brûlé par les re-prompts de garde).
//! L'émission est donc **inconditionnelle** au site d'appel : aucun
//! `if !summaries.is_empty()`. Une garde là serait précisément celle qui rendrait
//! muet le cas le plus intéressant.
//!
//! **L'absence de la ligne signifie un binaire antérieur (mika#2340)**, et ici
//! cette lecture est exacte et unique, parce que le WARN voisin existe déjà :
//!
//! - `grep -c "agent exceeded"` > 0 et `grep -c max_steps_tool_census` == 0
//!   ⇒ binaire antérieur à ce vol. **Jamais** « aucun outil appelé ».
//! - les deux à 0 ⇒ aucun épuisement sur la fenêtre.
//!
//! Les deux lignes étant émises au même site **sans condition**, leurs comptes
//! sont égaux par construction sur tout binaire postérieur à ce vol : c'est la
//! sonde de conformité de l'instrument (`docs/agent-internals/observability.md`).

use std::collections::BTreeMap;

use tracing::warn;

use crate::tool_execution::types::ToolCallSummary;

/// Nom de l'événement, suivant `tool_defs_census` (mika#2783).
pub const EVENT_MAX_STEPS_TOOL_CENSUS: &str = "max_steps_tool_census";

/// Le recensement des appels d'outils d'un tour coupé au plafond de steps.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaxStepsToolCensus {
    /// Appels par nom d'outil. `BTreeMap` et non `HashMap` : l'ordre de
    /// sérialisation doit être déterministe, sinon le champ `per_tool` n'est ni
    /// assertable en test ni comparable entre deux lignes de log.
    /// Un outil non appelé est ABSENT, jamais à 0 (mika#2331).
    pub per_tool: BTreeMap<String, u32>,
    /// `summaries.len()`. Peut DÉPASSER `max_steps` : un step porte autant
    /// d'appels que le modèle a émis de blocs `tool_use`
    /// (`tool_execution/dispatch.rs`). Ce n'est pas une anomalie, c'est du
    /// parallélisme — et le distinguer est l'objet de `steps_with_tools`.
    pub total_calls: u32,
    /// Nombre de valeurs de `step` distinctes ayant produit au moins un appel.
    ///
    /// Porteur et non décoratif : sans lui, `total_calls = 20` recouvre deux
    /// populations non soustractibles (classe mika#2205) — « 20 steps × 1 appel »
    /// (le modèle travaille, le plafond est trop bas) et « 5 steps × 4 appels
    /// parallèles » (le budget brûle ailleurs). Ces deux lectures appellent des
    /// corrections opposées.
    pub steps_with_tools: u32,
    /// Nombre de noms d'outils distincts, soit `per_tool.len()`.
    pub distinct_tools: u32,
    /// Appels à `success == false`. Un échec consomme du budget et compte aussi
    /// dans `per_tool` : c'est le motif « un outil qui boucle sur son erreur ».
    pub failed_calls: u32,
}

/// Agrège une tranche de résumés d'appels d'outils en un recensement.
///
/// L'identité `per_tool.values().sum() == total_calls` est tenue **par
/// construction** : un seul parcours de la tranche construit les deux. Aucun
/// contrôle de réconciliation à l'exécution n'est ajouté — il n'aurait aucune
/// population, il surveillerait deux sorties du même `for`. L'identité est
/// assertée dans le test unitaire ci-dessous, où elle protège contre une refonte
/// future de la fonction, et nulle part ailleurs.
pub fn census(summaries: &[ToolCallSummary]) -> MaxStepsToolCensus {
    let mut per_tool: BTreeMap<String, u32> = BTreeMap::new();
    let mut steps: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    let mut failed_calls = 0u32;

    for summary in summaries {
        *per_tool.entry(summary.name.clone()).or_insert(0) += 1;
        steps.insert(summary.step);
        if !summary.success {
            failed_calls += 1;
        }
    }

    MaxStepsToolCensus {
        total_calls: summaries.len() as u32,
        steps_with_tools: steps.len() as u32,
        distinct_tools: per_tool.len() as u32,
        per_tool,
        failed_calls,
    }
}

/// Émet le recensement du tour coupé, au niveau **WARN**.
///
/// WARN pour accompagner le WARN voisin : un épuisement est anormal, et un
/// opérateur qui filtre son log au niveau WARN doit voir les deux lignes ou
/// aucune.
///
/// `label` porte la **même** valeur que le WARN voisin (`mode.label()`) : même
/// terme, donc les deux populations se soustraient ; un synonyme les rendrait
/// incomparables.
///
/// `per_tool` part en **JSON compact sous clés triées** — un objet et non une
/// liste de paires, parce que l'opérateur le lit au `jq` et que l'ordre par coût
/// est le travail de `jq`, pas celui de l'émetteur.
///
/// La carte ne porte que des **noms d'outils** — jamais `input_summary` ni
/// `output_summary`. Aucune valeur d'entrée ou de sortie n'y transite, donc aucun
/// passage par `secret_scrubber` n'est requis : c'est une propriété du choix de
/// champs, pas une vérification à refaire.
pub fn emit_max_steps_tool_census(
    summaries: &[ToolCallSummary],
    agent_id: &str,
    session_id: &str,
    trace_id: &str,
    label: &str,
    max_steps: usize,
) {
    let c = census(summaries);
    warn!(
        target: "mika::otel",
        event = EVENT_MAX_STEPS_TOOL_CENSUS,
        agent_id = %agent_id,
        session_id = %session_id,
        trace_id = %trace_id,
        label = %label,
        max_steps,
        total_calls = c.total_calls,
        steps_with_tools = c.steps_with_tools,
        distinct_tools = c.distinct_tools,
        failed_calls = c.failed_calls,
        per_tool = %serde_json::to_string(&c.per_tool).unwrap_or_default(),
        "max steps exhausted; tool call census for the turn"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un résumé d'appel réussi, au step donné.
    fn call(step: u32, name: &str) -> ToolCallSummary {
        ToolCallSummary {
            step,
            name: name.to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        }
    }

    /// Le même, en échec.
    fn failed_call(step: u32, name: &str) -> ToolCallSummary {
        ToolCallSummary {
            success: false,
            ..call(step, name)
        }
    }

    /// **U1** — décompte exact sur une tranche mélangée, ordre des clés
    /// déterministe, et l'identité `sum(per_tool) == total_calls`.
    ///
    /// La tranche est délibérément désordonnée par rapport à l'ordre alphabétique
    /// (`run_gh` apparaît avant `read_file`, et `search_memory` est semé entre les
    /// deux) : c'est ce qui fait de l'assertion sur l'ordre une assertion et non
    /// une tautologie.
    #[test]
    fn mika2877_u1_le_decompte_est_exact_et_lordre_des_cles_deterministe() {
        let summaries = vec![
            call(1, "search_memory"),
            call(2, "run_gh"),
            call(3, "search_memory"),
            call(4, "read_file"),
            call(5, "search_memory"),
            call(6, "run_gh"),
        ];

        let c = census(&summaries);

        assert_eq!(c.per_tool.get("search_memory"), Some(&3));
        assert_eq!(c.per_tool.get("run_gh"), Some(&2));
        assert_eq!(c.per_tool.get("read_file"), Some(&1));
        assert_eq!(c.distinct_tools, 3);
        assert_eq!(c.total_calls, 6);
        assert_eq!(c.steps_with_tools, 6);
        assert_eq!(c.failed_calls, 0);

        // L'ordre des clés est celui du `BTreeMap`, donc alphabétique, donc le
        // même d'une exécution à l'autre : c'est ce qui rend le champ `per_tool`
        // assertable en test et comparable entre deux lignes de log.
        assert_eq!(
            c.per_tool.keys().collect::<Vec<_>>(),
            vec!["read_file", "run_gh", "search_memory"]
        );
        assert_eq!(
            serde_json::to_string(&c.per_tool).unwrap(),
            r#"{"read_file":1,"run_gh":2,"search_memory":3}"#
        );

        // L'identité tenue par construction, assertée ici et nulle part ailleurs :
        // elle protège contre une refonte future de `census`, pas contre un état
        // atteignable aujourd'hui.
        assert_eq!(
            c.per_tool.values().sum::<u32>(),
            c.total_calls,
            "la somme de la carte doit être le total des appels"
        );
    }

    /// **U2** — tranche vide : carte vide, tous les scalaires à 0, aucune panique
    /// et aucun `Option` à déballer.
    ///
    /// C'est le cas que l'émission inconditionnelle existe pour rendre lisible —
    /// « le budget a brûlé hors appels d'outils » est peut-être *la* réponse, et un
    /// `if !summaries.is_empty()` au site d'appel le rendrait muet.
    #[test]
    fn mika2877_u2_une_tranche_vide_est_une_mesure_pas_une_absence() {
        let c = census(&[]);

        assert!(c.per_tool.is_empty());
        assert_eq!(c.total_calls, 0);
        assert_eq!(c.steps_with_tools, 0);
        assert_eq!(c.distinct_tools, 0);
        assert_eq!(c.failed_calls, 0);
        assert_eq!(c, MaxStepsToolCensus::default());

        // Et la carte vide se sérialise en objet vide, pas en `null` : la ligne de
        // log porte une mesure, `jq` la lit sans cas particulier.
        assert_eq!(serde_json::to_string(&c.per_tool).unwrap(), "{}");
    }

    /// **U3** — plusieurs appels au même `step` : `total_calls > steps_with_tools`,
    /// et `total_calls` peut dépasser `max_steps` sans que rien ne soit faux.
    ///
    /// C'est le parallélisme du modèle (plusieurs blocs `tool_use` dans une même
    /// réponse). Un décompte qui compterait les **steps** rendrait ici les deux
    /// nombres égaux ; c'est la seule chose qui sépare les deux implémentations.
    #[test]
    fn mika2877_u3_le_parallelisme_separe_les_appels_des_steps() {
        let mut summaries = Vec::new();
        // Trois steps, quatre appels parallèles chacun : 12 appels sur 3 steps.
        for step in 1..=3u32 {
            for _ in 0..4 {
                summaries.push(call(step, "run_gh"));
            }
        }

        let c = census(&summaries);

        assert_eq!(c.total_calls, 12);
        assert_eq!(c.steps_with_tools, 3);
        assert!(
            c.total_calls > c.steps_with_tools,
            "un step porte autant d'appels que le modèle en a émis"
        );

        // Et le total peut dépasser le plafond : `max_steps` borne les STEPS, pas
        // les appels. Un opérateur qui lirait `total_calls > max_steps` comme une
        // anomalie se tromperait, d'où la note sur le champ.
        let max_steps = 10usize;
        assert!(
            c.total_calls as usize > max_steps,
            "total_calls n'est pas borné par max_steps"
        );
    }

    /// **U4** — `failed_calls` compte les `success == false`, **et** ces appels
    /// restent comptés dans `per_tool`.
    ///
    /// La seconde moitié est la charge : un échec consomme du budget comme un
    /// succès. Les exclure de `per_tool` effacerait exactement le motif « un outil
    /// qui boucle sur son erreur » — le plus probable des trois hypothèses du
    /// ticket.
    #[test]
    fn mika2877_u4_un_echec_compte_dans_failed_calls_et_dans_per_tool() {
        let summaries = vec![
            call(1, "run_gh"),
            failed_call(2, "run_gh"),
            failed_call(3, "run_gh"),
            call(4, "search_memory"),
            failed_call(5, "read_file"),
        ];

        let c = census(&summaries);

        assert_eq!(c.failed_calls, 3);
        assert_eq!(
            c.per_tool.get("run_gh"),
            Some(&3),
            "les deux échecs de run_gh restent comptés dans per_tool"
        );
        assert_eq!(c.per_tool.get("read_file"), Some(&1));
        assert_eq!(c.per_tool.get("search_memory"), Some(&1));
        assert_eq!(c.total_calls, 5);
        assert_eq!(
            c.per_tool.values().sum::<u32>(),
            c.total_calls,
            "aucun appel n'est perdu par son échec"
        );

        // Un tour tout en échecs reste lisible : `failed_calls == total_calls`.
        let all_failed = census(&[failed_call(1, "run_gh"), failed_call(2, "run_gh")]);
        assert_eq!(all_failed.failed_calls, all_failed.total_calls);
    }
}

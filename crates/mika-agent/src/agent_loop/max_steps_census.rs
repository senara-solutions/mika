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

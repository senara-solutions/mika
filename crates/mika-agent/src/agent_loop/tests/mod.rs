//! Tests de la boucle d'agent — le `mod tests` qui vivait inline dans
//! `agent_loop/mod.rs`, extrait tel quel (mika#3023), puis réparti par thème
//! (mika#3024).
//!
//! # Pourquoi un répertoire, et pas un `agent_loop/tests.rs`
//!
//! `perimeter::rules` classe **Mechanical** tout chemin portant un segment
//! `/tests/`, mais **DecisionCore** un `some_module/tests.rs` isolé — par choix
//! fail-closed explicite et commenté (`perimeter/tests.rs`, « the file itself
//! lives next to production code and might be accidentally cfg-flipped »). Or
//! une PR DECISION-CORE n'est pas dé-draftée automatiquement (mika#2286) : un
//! fichier `agent_loop/tests.rs` ferait donc de **chaque PR future touchant un
//! test de la boucle** un draft en attente d'un geste humain, c'est-à-dire
//! exactement le coût que mika#3023 existe pour retirer, déplacé d'un check
//! rouge vers un draft. Le répertoire l'évite par construction.
//!
//! **Ne « simplifiez » donc pas ce répertoire en un fichier plat.**
//!
//! # La partition par thème
//!
//! Ce fichier ne porte plus de test. Il déclare les quinze tranches ci-dessous
//! et détient les imports qu'elles partagent ; les 418 `#[test]` y vivent,
//! chacun dans la tranche de son sujet :
//!
//! | tranche | ce qu'elle instruit |
//! |---|---|
//! | `callbacks_et_declencheurs` | cadrage de callback, santé de tâche, préfixes de déclencheur |
//! | `classification_derreurs_et_reprise` | erreurs d'outil, revue de PR, intention de reprise |
//! | `detection_dans_le_texte` | ce que la boucle lit dans le texte rendu par le modèle |
//! | `fenetre_de_contexte` | attribution, recensement par outil, plafond d'octets |
//! | `filtre_doutils_et_mode_de_boucle` | filtre structurel d'outils, `LoopMode`, plafonds de pas |
//! | `gardes_de_source` | les scans de source — délai, `turn_usage`, typographie, lecteur unique |
//! | `heritage_dequipe` | un enfant hérite des stop-topics de son parent |
//! | `injection_de_skills_et_historique` | le contrat de prompt et la préparation de l'historique |
//! | `onboarding_et_amorce` | `check_onboarding` et l'amorce de la personne |
//! | `outils_requis` | ce qui arme, augmente et désarme la porte des outils requis |
//! | `perimetre_mcp` | publier des outils MCP passe par le helper qui ensemence |
//! | `resume_et_usage_du_tour` | résumé plafonné en mode silencieux, usage déclaré du tour |
//! | `skills_outils_et_budgets` | carte outil→skill, délais, visibilité, contrat de preuve |
//! | `tours_webhook` | déclencheurs de webhook, gardes, dispatch autorisé |
//! | `verdicts_et_dispositions` | verdicts, dispositions attestées, budget de raisonnement |
//!
//! ## Où va un helper
//!
//! Une tranche est **enfant** de ce module, donc petite-fille d'`agent_loop` :
//! elle voit les items privés d'`agent_loop` et ceux de ce fichier, mais **pas**
//! ceux d'une tranche sœur. Un helper (ou une constante) utilisé par une seule
//! tranche reste dedans ; dès qu'un second lecteur apparaît, il **monte ici** et
//! les deux tranches l'atteignent par leur `use super::*;`. Monter un helper ne
//! coûte aucune visibilité élargie : c'est la propriété que le précédent
//! mika#2321 a dégagée sur `db/tests/`, et c'est elle qui rend ce découpage
//! mécanique. Si une répartition demandait un `pub(crate)`, c'est la coupure
//! qu'il faut refaire, pas la signature qu'il faut ouvrir.
//!
//! ## Conséquence sur le chemin de module, et la forme citable
//!
//! Un test porté par une tranche s'appelle désormais
//! `agent_loop::tests::<tranche>::<nom>` — un `mod` fichier crée un niveau de
//! chemin, il n'y a pas d'orthographe qui l'évite (`include!` a été refusé :
//! `rustfmt` ne le traverse pas, donc `cargo fmt --check` passerait au vert sur
//! un fichier qui dérive). **Le chemin long n'est donc pas la forme à citer.**
//! Un nom de test est unique dans tout le dépôt — il porte son numéro de
//! ticket — et `cargo test -p mika-agent <nom>` le trouve sans son chemin. Les
//! citations vivantes en prose et en commentaire ont été réécrites au **nom
//! nu** par mika#3024, pour cette raison exacte : le nom nu survit au prochain
//! redécoupage, le chemin long non.
//!
//! # Ce que la déclaration `mod tests;` sans visibilité achète
//!
//! Le parent déclare `#[cfg(test)] mod tests;` **sans** `pub(crate)`, et ce
//! n'est pas un détail :
//!
//! - **Le seul importateur est un frère, et il n'a pas besoin de plus.** Depuis
//!   mika#3004, `agent_loop::pr_review_groom::tests` nomme
//!   `super::super::tests::{TICKET_2750_CLOSED_MSG, make_registry_with_tool,
//!   make_resolved_skill_tool, served}` — quatre items portés en
//!   `pub(in crate::agent_loop)`, ouverts à ce frère et pas plus haut. Un frère
//!   voit `tests` parce qu'un enfant voit les items privés de son parent
//!   `agent_loop` ; aucune visibilité sur la déclaration n'est nécessaire pour
//!   cela, et `pub(crate)` n'ajouterait qu'un lecteur qui n'existe pas. C'est
//!   la différence de degré avec `db`, dont `tests/mod.rs` doit documenter que
//!   `skills::executor` importe `crate::db::tests::*`.
//!
//!   **Conséquence du découpage :** ces quatre items doivent rester dans *ce*
//!   fichier. Les descendre dans une tranche changerait leur chemin en
//!   `agent_loop::tests::<tranche>::<nom>` et casserait l'import du frère.
//! - **Cette orthographe garde l'ancre de `source_scan::production_half`
//!   satisfaite.** L'ancre est le littéral `"\n#[cfg(test)]\nmod tests"` ;
//!   `pub(crate) mod tests;` ne la matche pas. La garder vivante laisse la
//!   coupure production/test de `mod.rs` se comporter après le déplacement
//!   comme elle se comportait avant — une invariance de moins à vérifier.
//!
//! La même règle vaut pour les quinze `mod` ci-dessous : aucune visibilité,
//! puisque personne hors de ce répertoire ne les nomme.

use super::*;
use crate::skills::index::{ResolvedSkillTool, SkillEntry};
use crate::skills::manifest::{SkillInfo, SkillManifest, ToolHandler, Triggers};
use crate::test_utils::test_helpers::test_async_db;
use mika_common::claude::ToolDefinition;
use std::path::PathBuf;

mod callbacks_et_declencheurs;
mod classification_derreurs_et_reprise;
mod detection_dans_le_texte;
mod fenetre_de_contexte;
mod filtre_doutils_et_mode_de_boucle;
mod gardes_de_source;
mod heritage_dequipe;
mod injection_de_skills_et_historique;
mod onboarding_et_amorce;
mod outils_requis;
mod perimetre_mcp;
mod resume_et_usage_du_tour;
mod skills_outils_et_budgets;
mod tours_webhook;
mod verdicts_et_dispositions;

// ===========================================================================
// Helpers partagés — ceux-là seulement
//
// Un helper n'arrive ici qu'avec un second lecteur, et il y arrive avec la
// visibilité qu'il avait : rien n'est élargi par le découpage (mika#3024).
// ===========================================================================

fn tool_def(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: format!("test stub for {name}"),
        input_schema: serde_json::json!({}),
    }
}

fn full_tool_set() -> Vec<ToolDefinition> {
    vec![
        tool_def("run_claude_pilot"),
        tool_def("run_claude_pilot_groom"),
        tool_def("send_message"),
        tool_def("run_gh"),
        tool_def("update_task_status"),
    ]
}

fn make_summary(name: &str, output: &str, success: bool) -> ToolCallSummary {
    ToolCallSummary {
        step: 0,
        name: name.to_string(),
        input_summary: String::new(),
        output_summary: output.to_string(),
        success,
        non_zero_exit: !success,
    }
}

fn make_skill_entry(name: &str, timeout: u64, tool_names: &[&str]) -> SkillEntry {
    make_skill_entry_with_constraints(name, timeout, tool_names, &[])
}

fn make_skill_entry_with_constraints(
    name: &str,
    timeout: u64,
    tool_names: &[&str],
    required_tools: &[&str],
) -> SkillEntry {
    use crate::skills::manifest::Constraints;
    SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: name.to_string(),
                description: format!("{name} skill"),
                version: "0.1.0".to_string(),
                always_on: false,
                timeout_secs: timeout,
                dependencies: vec![],
                max_prompt_size: None,
                data_grade: Default::default(),
            },
            triggers: Triggers {
                keywords: vec![name.to_string()],
            },
            llm: Default::default(),
            constraints: Constraints {
                required_tools: required_tools.iter().map(|s| s.to_string()).collect(),
                required_fetches_for_quoted_resources: false,
            },
            output: Default::default(),
            context: std::collections::HashMap::new(),
            variants: Default::default(),
        },
        dir: PathBuf::from(format!("/skills/{name}")),
        keywords_lower: vec![name.to_lowercase()],
        prompt_snippet: String::new(),
        skill_tools: tool_names
            .iter()
            .map(|tn| ResolvedSkillTool {
                definition: ToolDefinition {
                    name: tn.to_string(),
                    description: format!("{tn} tool"),
                    input_schema: serde_json::json!({"type": "object"}),
                },
                handler: ToolHandler::Builtin {
                    function: tn.to_string(),
                },
                skill_dir: PathBuf::from(format!("/skills/{name}")),
            })
            .collect(),
        enabled: true,
        has_override: false,
        provider_overrides: std::collections::HashMap::new(),
        prompt_sources: SkillEntry::empty_prompt_sources(),
        model_overrides: std::collections::HashMap::new(),
    }
}

// Les quatre suivants sont nommés par le frère `agent_loop::pr_review_groom`
// (mika#3004) : leur chemin doit rester `agent_loop::tests::<nom>`, donc ils ne
// descendent dans aucune tranche. Voir l'en-tête, § `mod tests;` sans visibilité.

// mika#3004 — ouvert au frère `pr_review_groom`, et pas plus haut.
pub(in crate::agent_loop) fn make_resolved_skill_tool(name: &str) -> ResolvedSkillTool {
    ResolvedSkillTool {
        definition: ToolDefinition {
            name: name.to_string(),
            description: format!("{name} tool"),
            input_schema: serde_json::json!({"type": "object"}),
        },
        handler: ToolHandler::Builtin {
            function: name.to_string(),
        },
        skill_dir: PathBuf::from("/skills/test"),
    }
}

/// Create a ToolRegistry with a single dummy builtin tool named `name`.
pub(in crate::agent_loop) fn make_registry_with_tool(name: &'static str) -> ToolRegistry {
    struct DummyBuiltin(&'static str);
    #[async_trait::async_trait]
    impl crate::tools::Tool for DummyBuiltin {
        fn name(&self) -> &str {
            self.0
        }
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: self.0.to_string(),
                description: format!("{} tool", self.0),
                input_schema: serde_json::json!({"type": "object"}),
            }
        }
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &crate::tools::ToolContext<'_>,
        ) -> Result<crate::tools::ToolOutput> {
            Ok(crate::tools::ToolOutput::success("ok"))
        }
    }
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(DummyBuiltin(name)));
    registry
}

/// mika#2757 — le tableau d'outils servi (`enabled_tool_names`, #862) pour un
/// cas qui ne mesure pas le désarmement : tout ce qui est exigé est servi.
pub(in crate::agent_loop) fn served(names: &[&str]) -> HashSet<String> {
    names.iter().map(|n| (*n).to_string()).collect()
}

/// Le message mesuré du 2026-10-17 ≈ 01:01Z, reconstitué sur la forme que
/// `mika_gateway::github::format_event_text` émet pour `issues.closed` :
/// préfixe, référence, tiret cadratin, titre, puis le corps cité.
///
/// **Deux propriétés sont load-bearing, et seules elles.** Le préfixe
/// `[GitHub] Issue closed:` met le tour dans le domaine Fallthrough (ni
/// ready-label, ni PR, ni check-suite), donc
/// `webhook_no_unauthorized_dispatch` y interdit tout dispatch. Et le mot
/// `groom` est dans le **corps cité** — ni dans le titre, ni dans un slug de
/// branche : le masque du vol 1 (mika#2755,
/// `github_event_format::mask_cited_branch`) ne lit que le marqueur
/// `(branch: …)` de la première ligne, donc il ne le couvre pas. C'est
/// exactement la seconde occurrence que ce vol doit couvrir.
pub(in crate::agent_loop) const TICKET_2750_CLOSED_MSG: &str = "[GitHub] Issue closed: senara-solutions/mika#2750 — un dispatch ready-label qui échoue\n\nLe dispatch auto-groom s'est interrompu avant d'ouvrir la session.\n";

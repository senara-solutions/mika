//! Assemblage du `SkillRegistry` d'un agent vivant — le site unique (mika#2889).
//!
//! **La règle :** tout assemblage du registre d'un agent qui va *servir des tours*
//! passe la séquence complète de l'initialisation, dans l'ordre de
//! l'initialisation. Un rechargement à chaud est un assemblage : avant mika#2889
//! les quatre sites de hot-reload (A2A, HTTP, les deux de la TUI) s'arrêtaient
//! après le ban testimony-grade, donc une skill que `apply_load_safety_check` ou
//! `apply_required_tools_coherence_check` aurait écartée au démarrage revenait
//! vivante au premier `skills_dirty`.
//!
//! [`assemble_agent_registry`] est la seule réponse à « comment assemble-t-on un
//! registre d'agent vivant ». Une garde de source (`mika#2889` AC3) refuse qu'un
//! second site de production réponde la même chose, avec une allowlist livrée
//! vide : si elle rougit, **router le site par cette fonction**, ne pas l'inscrire
//! (doctrine mika#2201).
//!
//! Hors périmètre, et délibérément : les vues de diagnostic de
//! `mika-cli/src/commands/skills.rs`, qui lisent la surface *sur disque* sans
//! allowlist d'identité ni contrôles, et toute autre convergence de la TUI
//! (mika#1727).

use std::path::Path;

use super::SkillRegistry;
use crate::db::SkillOverride;
use crate::prompt::Identity;

/// Assemble le `SkillRegistry` d'un agent vivant.
///
/// La séquence, dans l'ordre que l'initialisation serveur a fixé :
///
/// 1. [`SkillRegistry::from_dir`] — scan du répertoire de skills
/// 2. [`SkillRegistry::apply_identity_allowlist`] — allowlist pilotée par
///    l'identité, quand l'identité en déclare une
/// 3. [`SkillRegistry::apply_overrides`] — overrides en base (désactivation,
///    `always_on`, fournisseur/modèle LLM)
/// 4. [`SkillRegistry::apply_testimony_grade_ban`] — ban structurel (mika#1798),
///    après allowlist et overrides pour qu'une ré-activation ne le contourne pas
/// 5. [`SkillRegistry::apply_load_safety_check`] — **contrôle de chargement** :
///    écarte une skill dont la validation rend un échec skip-worthy (handler
///    introuvable, manifeste incohérent) ; raison préfixée `validation:`
/// 6. [`SkillRegistry::apply_required_tools_coherence_check`] — **contrôle de
///    cohérence** : écarte une skill dont un `required_tools` ne se résout pas
///    dans la surface effective de cet agent ; raison préfixée `coherence:`
/// 7. [`SkillRegistry::log_summary`] — compte à trois états, après tout le reste
///
/// Les étapes 5 et 6 sont la règle de mika#2889 : **les deux contrôles, dans cet
/// ordre, à tout assemblage**. Elles viennent après le ban et les overrides parce
/// qu'elles jugent la surface *finale* — contrôler avant laisserait passer une
/// skill qu'un override vient de réactiver. L'ordre entre elles est celui de
/// l'initialisation serveur, et il porte : le contrôle de chargement ne rend
/// qu'un avertissement sur un `required_tools` inconnu (`validate_skill`
/// étape 5b), donc seul le second écarte cette population.
///
/// Prend l'[`Identity`] entière plutôt que l'allowlist nue : « j'ai oublié
/// l'allowlist » devient inexprimable au site d'appel.
///
/// Synchrone, et les `overrides` arrivent en paramètre plutôt que lus ici : le
/// site d'équipe (`teams::engine`) les lit sur une [`crate::db::Database`]
/// synchrone, avant que l'`AsyncDatabase` n'existe. Un appelant qui n'a pas pu
/// les lire passe une tranche vide — `apply_overrides(&[])` n'évince rien.
pub fn assemble_agent_registry(
    skills_dir: &Path,
    identity: &Identity,
    overrides: &[SkillOverride],
    agent_id: &str,
) -> SkillRegistry {
    let mut registry = SkillRegistry::from_dir(skills_dir);
    if let Some(ref allowlist) = identity.skills.allowlist {
        registry.apply_identity_allowlist(allowlist);
    }
    registry.apply_overrides(overrides);
    registry.apply_testimony_grade_ban();
    registry.apply_load_safety_check();
    registry.apply_required_tools_coherence_check(agent_id);
    registry.log_summary();
    registry
}

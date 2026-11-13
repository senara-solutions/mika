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

#[cfg(test)]
mod tests {
    use mika_common::source_guard::ProductionScanner;
    use std::path::Path;

    /// Les quatre marqueurs d'un assemblage de registre d'**agent vivant**.
    ///
    /// `.apply_overrides(` est délibérément hors du motif : c'est la seule des cinq
    /// étapes mutantes qu'une vue de diagnostic a une raison légitime d'appliquer,
    /// pour peupler `disabled()` sans rien juger.
    const ASSEMBLY_MARKERS: &[&str] = &[
        ".apply_identity_allowlist(",
        ".apply_testimony_grade_ban(",
        ".apply_load_safety_check(",
        ".apply_required_tools_coherence_check(",
    ];

    /// Le seul assemblage d'agent vivant de la production : `(suffixe de chemin,
    /// nom de fonction)`.
    ///
    /// La clé porte le **nom de fonction** et pas seulement le fichier, pour qu'un
    /// second assembleur déposé dans ce module-ci soit lui aussi signalé.
    const SOLE_ASSEMBLY_SITE: (&str, &str) = ("skills/assembly.rs", "assemble_agent_registry");

    /// **Livrée vide, et [`mika2889_lallowlist_de_la_garde_est_livree_vide`] l'assert.**
    ///
    /// Population pré-existante : **zéro, mesurée** — le commit qui route les six
    /// sites par [`super::assemble_agent_registry`] précède celui-ci, précisément
    /// pour que le vide soit atteint plutôt que souhaité. Il n'y a donc rien à
    /// excepter, ni de case où déposer la prochaine infraction (mika#2323).
    /// Quand le scan tire, **on route le site par l'assembleur**, on ne
    /// l'allowliste pas (doctrine mika#2201).
    const SECOND_ASSEMBLER_EXCEPTIONS: &[&str] = &[];

    /// Les corps de fonction de `production` qui assemblent un registre d'agent
    /// vivant, rendus sous la clé `<chemin relatif>::<fonction>`.
    ///
    /// **L'ancrage est le corps de fonction, pas le fichier**, et c'est ce qui rend
    /// la garde fidèle à AC3 (« hors outils de diagnostic »). Mesure du
    /// 2026-10-23 : `crates/mika-cli/src/commands/skills.rs` porte en pleine
    /// production `SkillRegistry::from_dir` (lignes 29, 57, 63, 1715) **et** deux
    /// des quatre marqueurs — `.apply_identity_allowlist(` ligne 313 et
    /// `.apply_testimony_grade_ban(` ligne 328, tous deux dans
    /// `apply_db_overrides_if_available`, bien avant la frontière `#[cfg(test)]`
    /// de la ligne 1907. Un prédicat **par fichier** le signalerait, alors que ces
    /// deux populations ne se touchent pas : ce site applique trois étapes et
    /// **jamais les deux contrôles**, ce qui est exactement la vue de diagnostic
    /// que mika#2889 déclare hors périmètre (mika#1727). Par corps de fonction, il
    /// n'y a plus de choix entre « allowlister un fichier légitime » et « livrer
    /// l'allowlist vide ».
    ///
    /// [`crate::source_scan::fn_bodies`] retire les lignes de commentaire avant de
    /// découper, ce qui porte deux fois : la prose *à propos* d'une méthode n'est
    /// pas un appel (`commands/skills.rs:1713` et `:1744` nomment
    /// `apply_required_tools_coherence_check` dans des commentaires — forme du
    /// Signal S, mika#2050), et sans ce retrait le corps d'une fonction avale le
    /// doc-comment de la suivante. Le motif reste ancré sur le **point d'appel**
    /// (`.apply_…(`), parce que le retrait est à la ligne et non lexical : un
    /// commentaire de fin de ligne survit au filtre.
    ///
    /// Angle mort nommé : le corps est borné au prochain `fn `, donc un assemblage
    /// réparti entre une fonction et une closure déclarée par une `fn` imbriquée
    /// échappe au motif. Sous-approximer rend des faux rouges, jamais des faux
    /// verts (mika#2761).
    fn second_assemblers(rel: &str, production: &str) -> Vec<String> {
        crate::source_scan::fn_bodies(production)
            .into_iter()
            .filter(|(name, body)| {
                body.contains("from_dir")
                    && ASSEMBLY_MARKERS.iter().any(|m| body.contains(m))
                    && !(rel.ends_with(SOLE_ASSEMBLY_SITE.0) && name == SOLE_ASSEMBLY_SITE.1)
            })
            .map(|(name, _)| format!("{rel}::{name}"))
            .collect()
    }

    /// Chemin relatif à la racine du workspace, séparateurs normalisés.
    fn relative_to(workspace: &Path, path: &Path) -> String {
        path.strip_prefix(workspace)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    /// Racine du workspace : `<…>/crates/mika-agent` → `<…>`.
    ///
    /// Le motif de `mika_common::home § mika2230_le_tier_a_un_seul_analyseur` : un
    /// seul analyseur, qui balaye **tous** les crates plutôt que les deux que
    /// mika#2889 touche. Un assembleur déposé demain dans `mika-gateway` est la
    /// même infraction.
    fn workspace_root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("le manifeste de mika-agent vit sous <workspace>/crates/")
    }

    /// AC3 de mika#2889 — **un seul** appel de production à
    /// `SkillRegistry::from_dir` assemble un registre d'agent vivant.
    ///
    /// La classe que cette garde couvre et qu'aucune assertion de comportement ne
    /// voit : un septième site qui recopie la séquence et **oublie les deux
    /// contrôles**. Il sert des tours avec une skill que l'initialisation aurait
    /// écartée, et toute la suite reste verte (classe `grooming_marker`,
    /// mika#2158).
    #[test]
    fn mika2889_un_seul_site_de_production_assemble_un_registre_dagent_vivant() {
        let workspace = workspace_root();
        let mut found: Vec<String> = Vec::new();
        let mut crates_scanned = 0usize;
        let mut files_scanned = 0usize;
        let mut saw_agent = false;
        let mut saw_cli = false;

        for entry in std::fs::read_dir(workspace.join("crates"))
            .expect("la garde doit pouvoir lire <workspace>/crates")
            .flatten()
        {
            let src = entry.path().join("src");
            if !src.is_dir() {
                continue;
            }
            crates_scanned += 1;
            let crate_name = entry.file_name().to_string_lossy().into_owned();
            saw_agent |= crate_name == "mika-agent";
            saw_cli |= crate_name == "mika-cli";

            let scanner = ProductionScanner::new(&src);
            scanner.for_each(|path, production| {
                files_scanned += 1;
                let rel = relative_to(workspace, path);
                found.extend(second_assemblers(&rel, production));
            });
        }

        // Garde-fou : un scan qui ne lit rien est vert par construction. Les deux
        // crates nommés par le ticket sont exigés *par leur nom* — un plancher
        // numérique seul resterait vert sur une racine qui a dérivé.
        assert!(
            saw_agent && saw_cli,
            "la garde n'a pas lu les deux crates de mika#2889 (mika-agent: {saw_agent}, mika-cli: {saw_cli})"
        );
        assert!(
            crates_scanned >= 4 && files_scanned >= 100,
            "la garde ne lit pas le workspace : {crates_scanned} crates, {files_scanned} fichiers"
        );

        let unexpected: Vec<&String> = found
            .iter()
            .filter(|k| !SECOND_ASSEMBLER_EXCEPTIONS.contains(&k.as_str()))
            .collect();
        assert!(
            unexpected.is_empty(),
            "ces fonctions de production assemblent un registre d'agent vivant hors de \
             `skills::assembly::assemble_agent_registry` (mika#2889 AC3) : {unexpected:?}\n\
             Router le site par l'assembleur — ne pas l'inscrire dans \
             SECOND_ASSEMBLER_EXCEPTIONS (doctrine mika#2201).",
        );

        // Auto-nettoyage : une exception que le scan ne signale plus est périmée,
        // et une allowlist qui garde ses morts est celle que mika#2309 condamne.
        let stale: Vec<&&str> = SECOND_ASSEMBLER_EXCEPTIONS
            .iter()
            .filter(|e| !found.iter().any(|k| k == *e))
            .collect();
        assert!(
            stale.is_empty(),
            "exceptions périmées, à retirer de SECOND_ASSEMBLER_EXCEPTIONS : {stale:?}"
        );
    }

    /// L'allowlist de la garde est **livrée vide**, comme AC3 l'exige.
    ///
    /// Assertion sœur plutôt que commentaire : une allowlist « vide à la livraison »
    /// qu'aucun test ne mesure est vide jusqu'au premier qui y ajoute une ligne
    /// (motif mika#2474).
    #[test]
    fn mika2889_lallowlist_de_la_garde_est_livree_vide() {
        assert!(
            SECOND_ASSEMBLER_EXCEPTIONS.is_empty(),
            "AC3 de mika#2889 exige une allowlist vide ; elle porte {:?}. \
             Router le site par `assemble_agent_registry` (doctrine mika#2201).",
            SECOND_ASSEMBLER_EXCEPTIONS,
        );
    }

    /// Contrôle négatif de bonne foi, sur du contenu **fabriqué**.
    ///
    /// Un scan vert sur un corpus conforme n'a rien montré : il est vert aussi quand
    /// le motif ne correspond à rien. Le contrôle passe au **même** prédicat une
    /// rechute et trois formes innocentes. Il ne lit aucun fichier —
    /// [`ProductionScanner::production_of_content`] accepte un contenu arbitraire,
    /// donc aucune fixture sur disque, aucun brouillon.
    #[test]
    fn mika2889_la_garde_rougit_sur_un_second_assembleur() {
        let scanner = ProductionScanner::new(workspace_root().join("crates/mika-agent/src"));
        let faux = Path::new("crates/mika-gateway/src/faux_site.rs");

        // Volet rechute : la forme exacte que mika#2889 vient de retirer des six
        // sites — la séquence recopiée, sans les deux contrôles.
        let rechute = "\
fn rebuild_registry(dir: &Path, identity: &Identity) -> SkillRegistry {
    let mut registry = SkillRegistry::from_dir(dir);
    if let Some(ref allowlist) = identity.skills.allowlist {
        registry.apply_identity_allowlist(allowlist);
    }
    registry.apply_testimony_grade_ban();
    registry
}
";
        let production = scanner.production_of_content(faux, rechute);
        let signale = second_assemblers("crates/mika-gateway/src/faux_site.rs", &production);
        assert_eq!(
            signale,
            vec!["crates/mika-gateway/src/faux_site.rs::rebuild_registry"],
            "la garde doit signaler un second assembleur"
        );

        // Volet innocent, trois formes qui ne doivent pas tirer :
        // 1. `from_dir` seul — un listing sur disque (`commands/skills.rs` run) ;
        // 2. les marqueurs seuls — `apply_db_overrides_if_available`, la vue de
        //    diagnostic réelle, qui est *pourquoi* l'ancrage est par fonction ;
        // 3. de la prose qui décrit le motif interdit, le piège de mika#2323.
        let innocent = "\
/// Ce commentaire cite SkillRegistry::from_dir et .apply_load_safety_check(),
/// et registry.apply_required_tools_coherence_check(agent_id) — ce n'est pas un
/// appel, c'est de la prose.
fn list_on_disk(dir: &Path) -> SkillRegistry {
    SkillRegistry::from_dir(dir)
}

fn apply_db_overrides_if_available(identity: &Identity, registry: &mut SkillRegistry) {
    if let Some(ref allowlist) = identity.skills.allowlist {
        registry.apply_identity_allowlist(allowlist);
    }
    registry.apply_overrides(&overrides);
    registry.apply_testimony_grade_ban();
}
";
        let production = scanner.production_of_content(faux, innocent);
        assert!(
            second_assemblers("crates/mika-gateway/src/faux_site.rs", &production).is_empty(),
            "la garde confond une vue de diagnostic, ou de la prose, avec un assemblage"
        );

        // Le site légitime reste exempté — par (chemin, fonction), donc un second
        // assembleur déposé dans ce fichier-ci est tout de même signalé.
        let jumeau = "\
fn assemble_agent_registry(dir: &Path) -> SkillRegistry {
    let mut r = SkillRegistry::from_dir(dir);
    r.apply_load_safety_check();
    r
}

fn assemble_agent_registry_bis(dir: &Path) -> SkillRegistry {
    let mut r = SkillRegistry::from_dir(dir);
    r.apply_load_safety_check();
    r
}
";
        let faux_assembly = Path::new("crates/mika-agent/src/skills/assembly.rs");
        let production = scanner.production_of_content(faux_assembly, jumeau);
        assert_eq!(
            second_assemblers("crates/mika-agent/src/skills/assembly.rs", &production),
            vec!["crates/mika-agent/src/skills/assembly.rs::assemble_agent_registry_bis"],
            "l'exemption doit porter sur une seule fonction, pas sur le fichier"
        );
    }
}

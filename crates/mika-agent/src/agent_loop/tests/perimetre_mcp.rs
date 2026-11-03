//! Le périmètre MCP : publier des outils passe par le helper qui ensemence.
//!
//! Le recensement des lecteurs qui lisent l'inventaire MCP **sans** le
//! publier au modèle, la garde qui refuse une publication hors helper
//! (mika#2814) et les grades MCP que le helper verse (mika#1959).
//!
//! Visibilité : enfant de `agent_loop::tests`, donc descendant
//! d'`agent_loop` — les items privés d'`agent_loop` et les helpers promus
//! dans `tests/mod.rs` restent visibles via `use super::*`.

use super::*;

// -- mika#2814 : publier des outils MCP passe par le helper qui ensemence --

/// Lecteurs de production qui lisent l'inventaire MCP **sans** le publier au
/// modèle, en `(fichier, aiguille de la ligne, raison du droit)`.
///
/// **Née non vide, et ce n'est pas un emplacement pour le prochain oubli.**
/// Ce n'est pas l'allowlist que mika#2309 condamne : `tool_definitions()`
/// est un getter, et lire l'inventaire sans le verser dans la requête LLM
/// est une chose qu'on a légitimement le droit de faire. Cette liste est
/// donc un **inventaire de droits**, et **aucun ticket de suite n'est dû
/// pour ses deux entrées** — un relecteur n'a pas à chercher le `mika#NNNN`
/// qui les retirerait, il n'y en a pas. Ce que la garde exige à la place,
/// c'est qu'elles restent **vraies** : son assertion auto-nettoyante rougit
/// sur une entrée que le balayage ne retrouve plus.
///
/// **L'aiguille est le texte de la ligne, pas le seul fichier**, parce que
/// `agent_loop/mod.rs` porte aussi [`publier_outils_mcp`] et les deux
/// boucles d'agent : exempter le fichier entier y rendrait un troisième site
/// de publication invisible, à l'endroit précis où il serait écrit. Et
/// keyer sur le texte échoue dans le bon sens — une ligne réécrite ne
/// retrouve plus son exemption, donc la garde **rougit**. C'est l'inverse
/// exact du mode de panne de l'ancienne garde, dont l'aiguille *détectait*
/// la faute par sa forme et la laissait passer dès qu'on l'écrivait
/// autrement.
const LECTEURS_NON_PUBLIANTS_MCP: &[(&str, &str, &str)] = &[
    (
        "agent_loop/mod.rs",
        "flat_map(|m| m.tool_definitions().iter().map(|d| d.name.clone()))",
        "collecte les NOMS des outils disponibles (`available_tool_names`) \
             pour les gardes de fin de tour ; aucune définition n'entre dans le \
             lot envoyé au modèle",
    ),
    (
        "kg/domain_builder.rs",
        "for tool_def in mcp.tool_definitions()",
        "projette les outils MCP en entités du graphe de connaissance ; la \
             destination est SQLite, pas la requête LLM",
    ),
];

/// L'entrée de [`LECTEURS_NON_PUBLIANTS_MCP`] qui couvre `(rel, texte)`.
///
/// Écrite une seule fois pour que le contrôle négatif ci-dessous exerce le
/// prédicat de la garde, et non une copie qui pourrait en diverger.
fn lecteur_declare(rel: &str, texte: &str) -> Option<usize> {
    LECTEURS_NON_PUBLIANTS_MCP
        .iter()
        .position(|(fichier, aiguille, _)| rel.contains(fichier) && texte.contains(aiguille))
}

/// Lignes de source retenues par un balayage, en `(ligne 1-based, texte)`.
type LignesRetenues = Vec<(usize, String)>;

/// Bornes `(début, fin)` 1-based inclusives du corps de
/// [`publier_outils_mcp`] dans `src`, s'il y figure.
///
/// L'exclusion du corps du helper est **de portée, pas de fichier** : c'est
/// elle qui laisse la garde mordre sur un troisième site ajouté dans
/// `agent_loop/mod.rs` lui-même — le fichier le plus probable de tous,
/// puisque c'est là que vivent les boucles d'agent. Les bornes tiennent à
/// deux invariants que `cargo fmt --check` maintient en CI : une fonction
/// libre s'ouvre en colonne 0, et son corps se ferme sur un `}` en
/// colonne 0. Si un jour l'un des deux tombe, la fonction rend `None`, le
/// corps cesse d'être exclu et la garde **rougit** : échec fermé.
fn bornes_du_helper_de_publication(src: &str) -> Option<(usize, usize)> {
    let lignes: Vec<&str> = src.lines().collect();
    let debut = lignes
        .iter()
        .position(|l| l.starts_with("fn publier_outils_mcp("))?;
    let fin = lignes[debut + 1..].iter().position(|l| *l == "}")? + debut + 1;
    Some((debut + 1, fin + 1))
}

/// Usages de production de l'inventaire MCP, en `(ligne 1-based, texte)` :
/// les lignes qui **lisent** `tool_definitions()`, et celles qui
/// **appellent** [`publier_outils_mcp`] hors de sa définition.
///
/// L'aiguille de lecture est `tool_definitions()` **nue**, là où mika#1959
/// visait la forme `extend_from_slice(mcp.tool_definitions())`. Ce
/// changement est le cœur du vol 2 : viser la forme, c'était laisser passer
/// une publication écrite autrement (une boucle, un `chain`, un `clone()`
/// intermédiaire). Viser l'inventaire n'est devenu possible qu'avec le
/// helper, qui crée **un** site où la lecture publiante est légitime ; son
/// corps est donc retiré de la liste, par portée, via
/// [`bornes_du_helper_de_publication`]. Les autres lecteurs légitimes sont
/// déclarés par [`LECTEURS_NON_PUBLIANTS_MCP`], que la garde confronte au
/// balayage.
///
/// L'aiguille d'appel exclut la **définition** (`fn publier_outils_mcp(`) :
/// la déclarer n'est pas l'appeler, et la compter rapprocherait le compte de
/// deux avec un seul appel réel.
///
/// Les deux exclusions de [`identity_history_reader_sites`] se composent
/// identiquement : un fichier dont le **chemin** est du test n'a pas de
/// moitié production (mika#2321), et partout ailleurs les régions de test
/// sont **masquées** plutôt que tronquées (mika#2398) — ce qui est
/// porteur ici, puisque les deux aiguilles figurent en clair dans les
/// fixtures du contrôle négatif ci-dessous, dans ce même fichier.
fn usages_des_definitions_mcp(
    path: &std::path::Path,
    src: &str,
) -> (LignesRetenues, LignesRetenues) {
    if crate::source_scan::is_test_source_path(path) {
        return (Vec::new(), Vec::new());
    }
    let masque = mika_common::source_guard::mask_test_regions(src);
    let corps_du_helper = bornes_du_helper_de_publication(&masque);
    let lignes: LignesRetenues = masque
        .lines()
        .enumerate()
        .map(|(i, line)| (i + 1, line.trim().to_string()))
        .filter(|(_, line)| !line.starts_with("//") && !line.starts_with('*'))
        .collect();
    let lectures = lignes
        .iter()
        .filter(|(n, line)| {
            line.contains("tool_definitions()")
                && !corps_du_helper.is_some_and(|(d, f)| *n >= d && *n <= f)
        })
        .cloned()
        .collect();
    let appels = lignes
        .iter()
        .filter(|(_, line)| {
            line.contains("publier_outils_mcp(") && !line.contains("fn publier_outils_mcp(")
        })
        .cloned()
        .collect();
    (lectures, appels)
}

/// **mika#2814** — publier des outils MCP au modèle passe par
/// [`publier_outils_mcp`], qui ensemence la map L4 dans le même appel.
///
/// **Le nom a changé parce que la propriété a changé.** mika#1959 appariait
/// deux **textes** — `extend_from_slice(mcp.tool_definitions())` d'un côté,
/// `seed_mcp_data_grades(` de l'autre — et une publication écrite autrement
/// (une boucle, un `chain`, un `clone()` intermédiaire) laissait la garde
/// verte en rouvrant le contournement F5 de mika#1798. Ce que cette garde
/// prouve désormais est plus fort : il n'existe **qu'un seul chemin**, et
/// deux sites l'empruntent.
///
/// Trois assertions, chacune fermant une moitié que les autres laissent
/// ouverte :
///
/// 1. **Le compte d'appels est deux** — la conversation et l'agent
///    d'équipe. Le site silencieux/callback/heartbeat (`run_silent_agent`)
///    est **hors population par construction** : il reçoit `None`, donc il
///    ne publie ni n'ensemence. C'est pourquoi le nombre attendu est deux et
///    non trois.
/// 2. **Aucune lecture de `tool_definitions()`** hors du corps du helper et
///    hors de [`LECTEURS_NON_PUBLIANTS_MCP`]. Sans elle, un troisième site
///    écrivant directement `defs.extend_from_slice(mcp.tool_definitions())`
///    laisserait le compte à deux et passerait. Cette assertion était
///    **inécrivable avant le helper** : il n'existait alors aucun site
///    unique où la lecture publiante était légitime, donc l'aiguille devait
///    viser la forme plutôt que l'inventaire.
/// 3. **L'inventaire des lecteurs est auto-nettoyant** — chaque entrée doit
///    correspondre à un usage que le balayage retrouve. Une entrée dont la
///    ligne a disparu ou a été réécrite rougit, donc la liste ne peut pas
///    survivre à ce qu'elle décrit (mika#2309 : le geste qui rend une
///    exception inutile est celui qui la fait expirer).
///
/// Ce que cette garde ne dit **pas** : que les deux sites publient le *bon*
/// lot. Le filtre L2 de `McpManager::connect_all`
/// ([`crate::mcp::retain_publiables`], vol 1 de mika#2814) est **en amont**
/// d'eux, donc aucun n'a bougé et la map L4 garde l'inventaire **complet**
/// (grades et routage ne sont pas amputés) — ce que l'injection
/// « passe-plat » de `todos/2814-injection-verification.md` § Cas 4 a mesuré
/// plutôt que supposé. Et un appel sur un nom non publié n'est **pas**
/// « outil inconnu » : voir § *La porte que ce helper tient est une porte*
/// sur [`publier_outils_mcp`], et § Cas 1 de
/// `todos/2814-vol2-rouge-mesure.md` qui l'a mesuré.
#[test]
fn mika2814_publier_des_outils_mcp_passe_par_le_helper_qui_ensemence() {
    let scanner =
        mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
    let src_root = scanner.src_root().to_path_buf();
    let mut appels: Vec<String> = Vec::new();
    let mut lectures_non_declarees: Vec<String> = Vec::new();
    let mut entrees_retrouvees = vec![false; LECTEURS_NON_PUBLIANTS_MCP.len()];

    scanner.for_each(|path, production| {
        let rel = path
            .strip_prefix(&src_root)
            .unwrap_or(path)
            .display()
            .to_string();
        let (lectures, appels_ici) = usages_des_definitions_mcp(path, production);
        for (line, text) in appels_ici {
            appels.push(format!("{rel}:{line}: {text}"));
        }
        for (line, text) in lectures {
            match lecteur_declare(&rel, &text) {
                Some(i) => entrees_retrouvees[i] = true,
                None => lectures_non_declarees.push(format!("{rel}:{line}: {text}")),
            }
        }
    });

    assert_eq!(
        appels.len(),
        2,
        "mika#2814 — exactement deux sites appellent `publier_outils_mcp` : la \
             conversation et l'agent d'équipe. Le mode silencieux est hors population \
             (il reçoit `None`). Trouvés :\n{}",
        appels.join("\n")
    );
    assert!(
        lectures_non_declarees.is_empty(),
        "mika#2814 — ces lignes de production lisent `mcp.tool_definitions()` hors \
             du corps de `publier_outils_mcp` et hors de `LECTEURS_NON_PUBLIANTS_MCP` \
             :\n{}\n\nSi c'est une publication au modèle, passez par \
             `publier_outils_mcp` : il ensemence la map L4 dans le même appel, et un \
             outil MCP testimony publié sans grade laisse `mcp.call_tool` partir sur \
             n'importe quel nom obtenu autrement que par l'inventaire publié — c'est le \
             contournement F5 de mika#1798, mesuré au § Cas 1 de \
             `todos/2814-vol2-rouge-mesure.md`. Si c'est une lecture qui ne publie pas, \
             déclarez-la dans `LECTEURS_NON_PUBLIANTS_MCP` avec la raison de son droit : \
             aucun ticket de suite n'est dû pour un droit légitime.",
        lectures_non_declarees.join("\n")
    );
    let perimees: Vec<String> = LECTEURS_NON_PUBLIANTS_MCP
        .iter()
        .zip(&entrees_retrouvees)
        .filter(|(_, retrouvee)| !**retrouvee)
        .map(|((fichier, aiguille, _), _)| format!("{fichier} :: {aiguille}"))
        .collect();
    assert!(
        perimees.is_empty(),
        "mika#2814 — ces entrées de `LECTEURS_NON_PUBLIANTS_MCP` ne correspondent \
             plus à aucune ligne que le balayage retrouve :\n{}\n\nLe lecteur a été \
             retiré, déplacé ou réécrit : retirez l'entrée, ou corrigez son aiguille. \
             Une exemption qui survit à ce qu'elle décrit est un droit que plus personne \
             ne possède, et c'est par là qu'une garde cesse de garder (mika#2309).",
        perimees.join("\n")
    );
}

/// **mika#2814** — un seul appel à [`publier_outils_mcp`] publie les outils
/// MCP au modèle **et** les verse dans la map L4.
///
/// C'est la propriété que le vol 2 achète : avant, les deux gestes étaient
/// deux lignes qu'une garde appariait par comparaison de chaînes, et
/// n'écrire que la première était un défaut écrivable. Ce test mesure les
/// deux moitiés sur le **même** appel — c'est-à-dire qu'il échouerait sur
/// un helper qui n'en ferait qu'une, là où la garde de source, elle, ne
/// verrait rien (elle compte des appels, pas des effets).
#[test]
fn mika2814_un_seul_appel_publie_et_ensemence() {
    use crate::skills::manifest::DataGrade;

    let outil_mcp = "mcp__journal-perso__read_entry";
    let mcp = McpManager::sans_connexion_pour_tests(
        HashMap::from([(
            outil_mcp.to_string(),
            ("journal-perso".to_string(), "read_entry".to_string()),
        )]),
        HashMap::from([(outil_mcp.to_string(), DataGrade::Testimony)]),
    );

    let mut defs: Vec<mika_common::claude::ToolDefinition> = Vec::new();
    let mut grades = HashMap::from([("web_search".to_string(), DataGrade::Operational)]);
    publier_outils_mcp(&mut defs, &mut grades, Some(&mcp));

    assert_eq!(
        defs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
        mcp.tool_definitions()
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>(),
        "l'appel doit verser au modèle exactement l'inventaire publiable du manager"
    );
    assert_eq!(
        grades.get(outil_mcp),
        Some(&DataGrade::Testimony),
        "et le même appel doit avoir ensemencé la map L4 — c'est la soudure que ce \
             helper existe pour rendre indissociable"
    );
    assert_eq!(
        grades.get("web_search"),
        Some(&DataGrade::Operational),
        "sans toucher aux entrées des skills"
    );
}

/// **mika#2814** — `None` laisse les deux arguments intacts.
///
/// Le mode silencieux / callback / heartbeat est **hors population par
/// construction**, et c'est pourquoi la garde de source attend deux appels
/// et non trois. Ce test est ce qui rend cette phrase vraie plutôt
/// qu'affirmée : sans manager, ni le lot d'outils ni la map ne bougent.
#[test]
fn mika2814_sans_manager_le_helper_ne_touche_a_rien() {
    use crate::skills::manifest::DataGrade;

    let map_skills = || HashMap::from([("web_search".to_string(), DataGrade::Operational)]);
    let mut defs: Vec<mika_common::claude::ToolDefinition> = Vec::new();
    let mut grades = map_skills();
    publier_outils_mcp(&mut defs, &mut grades, None);

    assert!(
        defs.is_empty(),
        "sans manager, aucune définition n'est publiée au modèle"
    );
    assert_eq!(grades, map_skills(), "et la map L4 reste celle des skills");
}

/// **mika#1959** — `seed_mcp_data_grades` verse les grades MCP dans la map
/// L4 sans toucher aux entrées des skills, ne rabat jamais un `Testimony`
/// sur collision de nom, et `None` (mode silencieux) laisse la map intacte.
///
/// **Ce test reste, et son objet s'est précisé** (mika#2814 vol 2). Il ne
/// tient plus la moitié « l'appel est présent au site » — c'est désormais
/// [`mika2814_publier_des_outils_mcp_passe_par_le_helper_qui_ensemence`] et
/// [`mika2814_un_seul_appel_publie_et_ensemence`] qui s'en chargent. Ce
/// qu'il tient, et que ni l'un ni l'autre ne mesure, c'est la **règle de
/// collision** : elle est une propriété de l'ensemencement seul, et c'est la
/// raison pour laquelle [`seed_mcp_data_grades`] n'est pas fondue dans
/// [`publier_outils_mcp`]. Un corps vidé laisserait les deux gardes vertes.
#[test]
fn mika1959_lensemencement_verse_les_grades_mcp_sans_ecraser_les_skills() {
    use crate::skills::manifest::DataGrade;

    let outil_mcp = "mcp__journal-perso__read_entry";
    let mcp = McpManager::sans_connexion_pour_tests(
        HashMap::from([(
            outil_mcp.to_string(),
            ("journal-perso".to_string(), "read_entry".to_string()),
        )]),
        HashMap::from([(outil_mcp.to_string(), DataGrade::Testimony)]),
    );
    let map_skills = || HashMap::from([("web_search".to_string(), DataGrade::Operational)]);

    let mut grades = map_skills();
    seed_mcp_data_grades(&mut grades, Some(&mcp));
    assert_eq!(
        grades.get(outil_mcp),
        Some(&DataGrade::Testimony),
        "l'outil MCP doit entrer dans la map L4 avec le grade de son serveur"
    );
    assert_eq!(
        grades.get("web_search"),
        Some(&DataGrade::Operational),
        "l'ensemencement MCP ne doit pas toucher aux entrées des skills"
    );
    assert_eq!(grades.len(), 2);

    // Collision : un outil de skill testimony au nom namespacé MCP n'est
    // pas rabattu par un serveur qui n'a rien déclaré.
    let operational = McpManager::sans_connexion_pour_tests(
        HashMap::from([(
            outil_mcp.to_string(),
            ("journal-perso".to_string(), "read_entry".to_string()),
        )]),
        HashMap::from([(outil_mcp.to_string(), DataGrade::Operational)]),
    );
    let mut grades = HashMap::from([(outil_mcp.to_string(), DataGrade::Testimony)]);
    seed_mcp_data_grades(&mut grades, Some(&operational));
    assert_eq!(
        grades.get(outil_mcp),
        Some(&DataGrade::Testimony),
        "sur collision, le grade le plus strict l'emporte — un serveur MCP \
             muet ne rabat jamais un `Testimony` de skill"
    );

    let mut grades = map_skills();
    seed_mcp_data_grades(&mut grades, None);
    assert_eq!(
        grades,
        map_skills(),
        "sans manager (mode silencieux), la map reste celle des skills"
    );
}

/// **Contrôle négatif de bonne foi** pour
/// [`mika2814_publier_des_outils_mcp_passe_par_le_helper_qui_ensemence`].
///
/// Une garde dont le prédicat ne regarde rien est verte, et c'est le mode de
/// défaillance qu'un contrôle négatif existe pour rendre visible. Trois cas,
/// les mêmes que ceux dont le rouge est enregistré dans
/// `todos/2814-vol2-rouge-mesure.md` :
///
/// 1. une publication écrite hors du helper — y compris **sous une forme que
///    l'ancienne aiguille ne voyait pas** ;
/// 2. une lecture nue dans un fichier non déclaré ;
/// 3. l'état livré — définition du helper et deux lecteurs déclarés — qui
///    doit être **vert**, et dont la verdeur repose sur les trois exclusions
///    (portée du helper, prose, régions et chemins de test).
#[test]
fn mika2814_la_garde_du_helper_rougit_sur_une_publication_hors_chemin() {
    let ailleurs = std::path::Path::new("src/ailleurs.rs");

    // --- Cas 1 : un troisième site publie sans passer par le helper. -----
    let hors_helper = "fn troisieme_site(mcp: &McpManager, defs: &mut Vec<ToolDefinition>) {\n    \
             defs.extend_from_slice(mcp.tool_definitions());\n}\n";
    let (lectures, appels) = usages_des_definitions_mcp(ailleurs, hors_helper);
    assert!(
        appels.is_empty(),
        "ce site ne passe pas par le helper — c'est tout le défaut"
    );
    assert_eq!(lectures.len(), 1, "et le prédicat doit voir sa lecture");
    assert!(
        lecteur_declare("ailleurs.rs", &lectures[0].1).is_none(),
        "qu'aucun lecteur déclaré ne couvre : c'est le rouge de l'assertion 2"
    );

    // La même publication écrite en boucle. L'aiguille de mika#1959
    // (`extend_from_slice(mcp.tool_definitions())`) ne la voyait pas, et
    // c'est précisément le trou que le vol 2 ferme : l'aiguille porte
    // désormais sur l'inventaire, pas sur la forme qui le verse.
    let en_boucle = "fn troisieme_site(mcp: &McpManager, defs: &mut Vec<ToolDefinition>) {\n    \
             for d in mcp.tool_definitions() {\n        defs.push(d.clone());\n    }\n}\n";
    assert!(
        !en_boucle.contains("extend_from_slice(mcp.tool_definitions())"),
        "la fixture doit bien être invisible à l'ancienne aiguille, sinon ce cas \
             ne mesure rien"
    );
    let (lectures, _) = usages_des_definitions_mcp(ailleurs, en_boucle);
    assert_eq!(
        lectures.len(),
        1,
        "le nouveau prédicat voit la publication en boucle, que l'ancien manquait"
    );

    // --- Cas 2 : une lecture nue dans un fichier non déclaré. ------------
    // Légitime ou non, elle doit être *déclarée* : c'est la déclaration qui
    // porte la raison du droit, et l'absence de raison est le défaut.
    let inconnu = std::path::Path::new("src/nouveau/module.rs");
    let lecture_nue =
        "let noms: Vec<_> = mcp.tool_definitions().iter().map(|d| d.name.clone()).collect();\n";
    let (lectures, _) = usages_des_definitions_mcp(inconnu, lecture_nue);
    assert_eq!(lectures.len(), 1, "le prédicat doit voir la lecture nue");
    assert!(
        lecteur_declare("nouveau/module.rs", &lectures[0].1).is_none(),
        "dans un fichier non déclaré, elle rougit"
    );

    // --- Cas 3 : l'état livré est vert. ---------------------------------
    // 3a. Le corps du helper est exclu par **portée**, et sa définition
    //     n'est pas un appel.
    let helper = "fn publier_outils_mcp(\n    \
             defs: &mut Vec<ToolDefinition>,\n    \
             grades: &mut HashMap<String, DataGrade>,\n    \
             mcp: Option<&McpManager>,\n) {\n    \
             if let Some(mcp) = mcp {\n        \
             defs.extend_from_slice(mcp.tool_definitions());\n    }\n    \
             seed_mcp_data_grades(grades, mcp);\n}\n";
    let (lectures, appels) = usages_des_definitions_mcp(ailleurs, helper);
    assert!(
        lectures.is_empty(),
        "le corps du helper est le seul site où lire pour publier est juste"
    );
    assert!(
        appels.is_empty(),
        "et sa définition n'est pas un appel — la compter rapprocherait le compte \
             de deux avec un seul appel réel"
    );

    // 3b. Les deux lecteurs déclarés sont couverts — chacun dans son
    //     fichier, et **pas** dans celui de l'autre : le droit est attaché
    //     à une ligne précise, pas accordé globalement.
    let collecte = ".flat_map(|m| m.tool_definitions().iter().map(|d| d.name.clone())),\n";
    let (lectures, _) =
        usages_des_definitions_mcp(std::path::Path::new("src/agent_loop/mod.rs"), collecte);
    assert_eq!(lectures.len(), 1);
    assert!(
        lecteur_declare("agent_loop/mod.rs", &lectures[0].1).is_some(),
        "la collecte de noms est un lecteur déclaré"
    );
    assert!(
        lecteur_declare("kg/domain_builder.rs", &lectures[0].1).is_none(),
        "mais son droit ne vaut que dans son fichier"
    );
    let projection = "for tool_def in mcp.tool_definitions() {\n";
    let (lectures, _) =
        usages_des_definitions_mcp(std::path::Path::new("src/kg/domain_builder.rs"), projection);
    assert_eq!(lectures.len(), 1);
    assert!(
        lecteur_declare("kg/domain_builder.rs", &lectures[0].1).is_some(),
        "la projection KG est un lecteur déclaré"
    );

    // 3c. Un appel réel au helper est compté.
    let appelant = "publier_outils_mcp(&mut defs, &mut grades, params.mcp_manager);\n";
    let (_, appels) = usages_des_definitions_mcp(ailleurs, appelant);
    assert_eq!(appels.len(), 1, "un appel réel doit être compté");

    // 3d. La prose qui nomme les aiguilles ne lit ni n'appelle rien — sans
    //     quoi les doc-comments de ce fichier, et les fixtures ci-dessus,
    //     rendraient la garde rouge (classe mika#2050 Signal S).
    let prose = "\
// Toute publication passe par `publier_outils_mcp(`, jamais par `mcp.tool_definitions()` nue.
/// `defs.extend_from_slice(mcp.tool_definitions())` est la forme d'avant mika#2814.
 * `publier_outils_mcp(` publie et ensemence dans le même appel.
";
    let (lectures, appels) = usages_des_definitions_mcp(ailleurs, prose);
    assert!(
        lectures.is_empty() && appels.is_empty(),
        "un commentaire ne publie ni n'appelle — prédicat non ancré, classe \
             mika#2050 Signal S"
    );

    // 3e. Une région de test est masquée, et un fichier dont le chemin est
    //     du test n'a pas de moitié production : les deux exclusions qui
    //     rendent les fixtures de ce test écrivables dans ce fichier même.
    let dans_les_tests = "fn prod() {}\n#[cfg(test)]\nmod tests {\n    \
             defs.extend_from_slice(mcp.tool_definitions());\n    \
             publier_outils_mcp(&mut d, &mut g, m);\n}\n";
    let (lectures, appels) = usages_des_definitions_mcp(ailleurs, dans_les_tests);
    assert!(
        lectures.is_empty() && appels.is_empty(),
        "mika#2398 — les régions de test sont masquées"
    );
    let chemin_test = std::path::Path::new("src/db/tests/harnais.rs");
    let (lectures, _) = usages_des_definitions_mcp(chemin_test, hors_helper);
    assert!(
        lectures.is_empty(),
        "mika#2321 — un fichier sous `tests/` est du test quel qu'en soit le contenu"
    );
}

//! Porte transverse (mika#2265) : **aucun livrable de `tests/eval/` n'existe
//! sans être compilé**.
//!
//! Un fichier `.rs` posé dans `tests/eval/` sans sa ligne `mod` correspondante
//! dans `tests/eval.rs` n'est pas compilé du tout. Il ne casse rien, ne rapporte
//! rien, et aucune CI ne le réclame — parce qu'une CI ne voit pas un test
//! *absent*, elle ne voit que les tests qui existent et échouent. Un livrable
//! oublié de cette façon rend une porte d'acceptation verte pour la mauvaise
//! raison.
//!
//! Cette porte est ici parce que mika#2265 ajoute trois fichiers d'un coup à ce
//! répertoire. Elle se déclare elle-même : si *elle* est oubliée, l'oubli est
//! visible dans le diff du commit qui la crée.
//!
//! # Fire-Disposition
//!
//! **(c) halte-et-remontée, gate CI bloquant.** Elle tire quand un fichier
//! existe dans `tests/eval/` sans sa ligne `mod`. Disposition : nommer le ou les
//! fichiers orphelins et échouer. **Pas de liste blanche** — une exception
//! nommée ici rouvrirait exactement le trou que la porte ferme. Violations
//! préexistantes au moment de sa création : aucune.

use std::collections::BTreeSet;
use std::path::Path;

/// La déclaration de modules, épinglée à la compilation : la porte ne peut pas
/// être satisfaite par une copie périmée sur le disque.
const EVAL_RS: &str = include_str!("../eval.rs");

/// Les `mod X;` / `pub mod X;` déclarés dans `tests/eval.rs`.
fn declared_modules(src: &str) -> BTreeSet<String> {
    src.lines()
        .map(str::trim)
        .filter_map(|line| {
            let rest = line
                .strip_prefix("pub mod ")
                .or_else(|| line.strip_prefix("mod "))?;
            // Seules les déclarations terminales — `mod eval {` ouvre un bloc.
            rest.strip_suffix(';').map(str::to_owned)
        })
        .collect()
}

#[test]
fn tout_fichier_de_tests_eval_est_declare_dans_eval_rs() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/eval");
    let declared = declared_modules(EVAL_RS);

    let mut orphans: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("lire tests/eval/") {
        let path = entry.expect("entrée de tests/eval/").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("nom de fichier UTF-8")
            .to_owned();
        if !declared.contains(&stem) {
            orphans.push(stem);
        }
    }
    orphans.sort();

    assert!(
        orphans.is_empty(),
        "fichier(s) présent(s) dans crates/mika-agent/tests/eval/ mais NON déclaré(s) dans \
         tests/eval.rs, donc jamais compilé(s) ni exécuté(s) : {orphans:?}\n\
         Ajouter `mod <nom>;` (ou `pub mod <nom>;` pour un module de support) dans le bloc \
         `mod eval` de tests/eval.rs. Ne pas mettre ce fichier en exception : c'est le trou \
         que cette porte ferme."
    );
}

/// Les `stems` qu'un `mod.rs` ne déclare pas.
///
/// Extrait pour que le contrôle négatif ci-dessous puisse exercer la décision
/// **sans** passer par le disque : sur une porte dont le régime attendu est
/// « rien à signaler », un vert ne distingue pas « elle a regardé et n'a rien
/// trouvé » de « elle n'a rien regardé » (classe mika#2205).
fn undeclared_stems(declared: &BTreeSet<String>, stems: &[&str]) -> Vec<String> {
    let mut orphans: Vec<String> = stems
        .iter()
        .filter(|stem| !declared.contains(**stem))
        .map(|s| (*s).to_owned())
        .collect();
    orphans.sort();
    orphans
}

/// mika#1960 D5 — **l'angle mort du premier niveau.** La porte ci-dessus fait
/// `read_dir(tests/eval)` puis filtre `extension == "rs"` ; un **répertoire**
/// n'a pas cette extension, donc `doctrine_regressions/` n'est jamais énuméré et
/// aucun de ses fichiers n'est vérifié. Or c'est là que vivent dix scénarios de
/// doctrine, et un `pub mod` oublié y donne : fichier non compilé, zéro test
/// exécuté, **critère d'acceptation vert pour la mauvaise raison**.
///
/// Deux tests et non un élargissement du premier : deux populations (premier
/// niveau vs sous-répertoires), deux messages, comptables séparément. Le parseur
/// `declared_modules` est **réutilisé** — il est déjà générique sur une source.
///
/// Différence assumée avec la porte du premier niveau : la déclaration est lue
/// **au runtime** et non épinglée par `include_str!`, parce que l'ensemble des
/// sous-répertoires est dynamique. Les deux moitiés (listing et `mod.rs`) venant
/// du même arbre que la compilation, elles sont cohérentes entre elles ; la seule
/// divergence possible est une édition pendant l'exécution.
#[test]
fn tout_fichier_dun_sous_repertoire_de_tests_eval_est_declare_dans_son_mod_rs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/eval");

    let mut reports: Vec<String> = Vec::new();
    let mut dirs_scanned = 0usize;
    let mut files_scanned = 0usize;

    let mut subdirs: Vec<std::path::PathBuf> = std::fs::read_dir(&root)
        .expect("lire tests/eval/")
        .map(|e| e.expect("entrée de tests/eval/").path())
        .filter(|p| p.is_dir() && p.join("mod.rs").is_file())
        .collect();
    subdirs.sort();

    for dir in subdirs {
        dirs_scanned += 1;
        let mod_rs = std::fs::read_to_string(dir.join("mod.rs")).expect("lire le mod.rs");
        let declared = declared_modules(&mod_rs);

        let mut stems: Vec<String> = std::fs::read_dir(&dir)
            .expect("lire un sous-répertoire de tests/eval/")
            .map(|e| e.expect("entrée de sous-répertoire").path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("rs"))
            .filter_map(|p| {
                p.file_stem()
                    .and_then(|s| s.to_str())
                    .filter(|s| *s != "mod")
                    .map(str::to_owned)
            })
            .collect();
        stems.sort();
        files_scanned += stems.len();

        let borrowed: Vec<&str> = stems.iter().map(String::as_str).collect();
        let orphans = undeclared_stems(&declared, &borrowed);
        if !orphans.is_empty() {
            let name = dir.file_name().and_then(|s| s.to_str()).unwrap_or("?");
            reports.push(format!("{name}/ → {orphans:?}"));
        }
    }

    // Anti-vacuité : sans ces deux bornes, un renommage de répertoire rendrait la
    // porte silencieusement inerte, ce qui se lit exactement comme un arbre
    // propre. Les seuils sont des planchers larges, pas des comptes exacts — la
    // porte ne doit pas rougir chaque fois qu'on ajoute un scénario.
    assert!(
        dirs_scanned >= 8,
        "la porte n'a trouvé que {dirs_scanned} sous-répertoire(s) portant un mod.rs \
         dans tests/eval/ — elle ne regarde plus ce qu'elle prétend regarder. \
         Réparer l'énumération, ne pas baisser ce plancher."
    );
    assert!(
        files_scanned >= 80,
        "la porte n'a examiné que {files_scanned} fichier(s) dans les \
         sous-répertoires de tests/eval/ — population invraisemblablement basse. \
         Réparer l'énumération, ne pas baisser ce plancher."
    );

    assert!(
        reports.is_empty(),
        "fichier(s) présent(s) dans un sous-répertoire de crates/mika-agent/tests/eval/ \
         mais NON déclaré(s) dans le mod.rs de ce répertoire, donc jamais compilé(s) ni \
         exécuté(s) : {reports:?}\n\
         Ajouter `pub mod <nom>;` dans le mod.rs concerné. Ne pas mettre ce fichier en \
         exception : c'est le trou que cette porte ferme."
    );
}

/// Contrôle négatif de la porte des sous-répertoires : la décision doit *voir*
/// un orphelin, et ne pas voir d'orphelin là où il n'y en a pas.
///
/// Sans lui, `tout_fichier_dun_sous_repertoire_…` pourrait passer en ne mesurant
/// rien — et son régime attendu étant « vide », le vert serait indistinguable.
#[test]
fn la_porte_des_sous_repertoires_voit_un_orphelin_synthetique() {
    let declared = declared_modules("pub mod scenario_a;\npub mod scenario_b;\n");

    assert_eq!(
        undeclared_stems(&declared, &["scenario_a", "scenario_b"]),
        Vec::<String>::new(),
        "contrôle positif : deux fichiers déclarés ne sont pas des orphelins"
    );
    assert_eq!(
        undeclared_stems(&declared, &["scenario_a", "scenario_oublie"]),
        vec!["scenario_oublie".to_owned()],
        "contrôle négatif : un fichier non déclaré DOIT être signalé"
    );
}

/// Contrôle négatif de la porte elle-même : le parseur doit *voir* un orphelin.
///
/// Sans lui, `declared_modules` pourrait rendre l'univers entier (ou l'assertion
/// être vacuously vraie) et la porte passerait sans rien mesurer.
#[test]
fn la_porte_voit_un_orphelin_synthetique() {
    let declared = declared_modules("mod eval {\n    pub mod harness;\n    mod test_a;\n}\n");
    assert!(
        declared.contains("harness"),
        "contrôle positif : `pub mod` reconnu"
    );
    assert!(
        declared.contains("test_a"),
        "contrôle positif : `mod` reconnu"
    );
    assert!(
        !declared.contains("test_orphelin"),
        "contrôle négatif : un module non déclaré ne doit pas être vu comme déclaré"
    );
    assert!(
        !declared.contains("eval"),
        "`mod eval {{` ouvre un bloc, ce n'est pas une déclaration de fichier"
    );
}

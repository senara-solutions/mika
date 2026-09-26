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

//! Classification des fichiers source pour les gardes structurelles (mika#2321).
//!
//! Plusieurs gardes de ce crate énumèrent `src/**/*.rs` et cherchent, dans « la
//! production », un motif qu'un site de production ne doit pas porter. Elles
//! doivent donc écarter le code de test, dont les fixtures posent légitimement
//! le motif interdit.
//!
//! ## La prémisse que mika#2310 a cessé de rendre vraie
//!
//! Deux d'entre elles — [`crate::db`] (mika#2335 F2a) et [`crate::agent_loop`]
//! (mika#2305) — écartaient le code de test en **tronquant** chaque fichier à
//! son premier littéral `#[cfg(test)]` :
//!
//! ```ignore
//! let production = match src.find("#[cfg(test)]") {
//!     Some(i) => &src[..i],
//!     None => &src[..],
//! };
//! ```
//!
//! Cela suppose que le code de test vit derrière un `#[cfg(test)]` **inline dans
//! le même fichier**. C'était vrai quand les deux gardes ont été écrites. Ça a
//! cessé de l'être avec mika#2310, qui a sorti `db/tests/harnais_porte.rs` dans
//! son propre fichier : un module de test extrait ne porte **aucun** littéral
//! `#[cfg(test)]` — l'attribut reste sur la déclaration `#[cfg(test)] mod …;` du
//! fichier parent. `find` rend alors `None`, et la garde scanne le fichier de
//! test **entier comme du code de production**.
//!
//! La prémisse était déjà fausse en deux endroits avant ce ticket, et bénigne
//! par chance — aucun des deux ne contenait les aiguilles :
//!
//! | fichier | occurrences de `cfg(test)` | statut |
//! |---|---|---|
//! | `db/tests/harnais_porte.rs` | **0** | scanné intégralement comme production |
//! | `perimeter/tests.rs` | 1, **dans un commentaire** | idem |
//!
//! mika#2321 fait passer 463 Ko et 431 tests par ce trou. Remplacer la chance
//! par un tirage n'était pas une option, d'où ce module.
//!
//! ## Ce que la réparation corrige, et ce qu'elle ne corrige pas
//!
//! Elle porte sur la **classification du fichier**, pas sur la troncature : un
//! fichier dont le chemin est un chemin de test **est** du code de test, qu'il
//! porte ou non un littéral `#[cfg(test)]`. Un fichier de production garde sa
//! troncature ; un fichier de test est écarté entièrement.
//!
//! Élargir les aiguilles ou poser une allowlist par fichier est **refusé** :
//! c'est exactement le mouvement que la doc de `production_half` anticipe
//! (« the natural repair would be to widen it until it caught nothing ») et que
//! `mika2335_no_production_dispatch_transitions_a_parent_without_stamping`
//! refuse en toutes lettres pour un quatrième site.
//!
//! ## Le prédicat
//!
//! Celui que le dépôt teste déjà ailleurs : [`crate::perimeter::rules`] classe
//! `Mechanical` tout chemin contenant un segment `/tests/`, et `DecisionCore`
//! un `tests.rs` isolé — par choix fail-closed explicite, parce qu'un
//! `some_module/tests.rs` « might be accidentally cfg-flipped ».
//!
//! Ici les **deux** branches sont nécessaires, et pour des raisons distinctes :
//! `db/tests/tasks.rs` relève de la première, `perimeter/tests.rs` — qui existe
//! et est déjà dans le trou — de la seconde. Le fail-closed du périmètre répond
//! à une autre question (« ce diff peut-il être mergé sans relecture humaine ? »)
//! et n'a pas à être transporté ici : une garde qui scanne un module de test
//! comme de la production ne protège rien, elle rougit sur du code sain.
//!
//! Un seul lecteur, pas une copie par garde — c'est la classe que
//! `grooming_marker` a dû graver une fois (mika#2158 : une regex copiée dont le
//! commentaire disait « Mirrors … » et qui a ensuite raté deux élargissements).

use std::path::Path;

/// `true` si `path` désigne du code de test par son **chemin** : un segment de
/// répertoire `tests`, ou un nom de fichier `tests.rs`.
///
/// Les deux branches sont nécessaires (cf. doc du module). La comparaison porte
/// sur les composants du chemin plutôt que sur une sous-chaîne, pour ne pas
/// dépendre du séparateur de la plateforme.
pub(crate) fn is_test_source_path(path: &Path) -> bool {
    if path
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("tests.rs"))
    {
        return true;
    }
    path.parent().is_some_and(|parent| {
        parent
            .components()
            .any(|c| c.as_os_str().eq_ignore_ascii_case("tests"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn src_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
    }

    #[test]
    fn a_tests_directory_segment_is_a_test_path() {
        assert!(is_test_source_path(Path::new(
            "/repo/crates/mika-agent/src/db/tests/mod.rs"
        )));
        assert!(is_test_source_path(Path::new(
            "/repo/crates/mika-agent/src/db/tests/tasks.rs"
        )));
        assert!(is_test_source_path(Path::new(
            "/repo/crates/mika-agent/src/db/tests/harnais_porte.rs"
        )));
    }

    #[test]
    fn a_bare_tests_rs_is_a_test_path() {
        assert!(is_test_source_path(Path::new(
            "/repo/crates/mika-agent/src/perimeter/tests.rs"
        )));
    }

    #[test]
    fn production_files_are_not_test_paths() {
        for p in [
            "/repo/crates/mika-agent/src/db.rs",
            "/repo/crates/mika-agent/src/db/migrations.rs",
            "/repo/crates/mika-agent/src/db/operational.rs",
            "/repo/crates/mika-agent/src/agent_loop/mod.rs",
            "/repo/crates/mika-agent/src/perimeter/rules.rs",
            // Le mot « tests » dans un nom de fichier ne suffit pas : seul le
            // nom exact `tests.rs` ou un segment de répertoire comptent.
            "/repo/crates/mika-agent/src/no_dispatch_tests.rs",
            "/repo/crates/mika-agent/src/testsuite.rs",
        ] {
            assert!(!is_test_source_path(Path::new(p)), "{p} classé comme test");
        }
    }

    /// **V4 / AC9 — le trou ouvert par mika#2310 est fermé.**
    ///
    /// Les deux fichiers que la troncature `#[cfg(test)]` laissait passer pour
    /// de la production sont écartés par leur chemin. L'assertion porte sur des
    /// chemins réels : si l'un des deux est renommé ou déplacé, ce test rougit
    /// plutôt que de laisser la garde se rendormir en silence.
    #[test]
    fn mika2321_the_two_files_mika2310_left_in_the_hole_are_classified_as_tests() {
        for rel in ["perimeter/tests.rs", "db/tests/harnais_porte.rs"] {
            let path = src_root().join(rel);
            assert!(
                path.exists(),
                "{} n'existe plus — la garde V4 vise un chemin mort",
                path.display()
            );
            assert!(
                is_test_source_path(&path),
                "{} est toujours scanné comme de la production",
                path.display()
            );
        }
    }
}

//! Garde (a) de mika#2735 — **aucun `VACUUM` dans le chemin de démarrage**.
//!
//! **Pourquoi une garde structurelle et pas seulement un test de comportement.**
//! Le déploiement du 2026-10-16 a joué la migration v55→v56 au démarrage de
//! mika-spirit. `v56: vacuum complete` à 08:50:38Z ; premier
//! `file is not a database` 25 s plus tard ; à 11:00Z `PRAGMA quick_check`
//! rendait 25 arbres abîmés, avec `Bad ptr map entry` et
//! `2nd reference to page` — deux signatures de *pointer map*. La base étant née
//! en `auto_vacuum=NONE` et `Database::open` demandant
//! `PRAGMA auto_vacuum = INCREMENTAL` (demande inopérante jusqu'au prochain
//! `VACUUM`), ce `VACUUM` n'était pas un compactage : c'était une **conversion
//! de format** qui ajoute des pages de pointer map au fichier vivant.
//! [`super::conversion_auto_vacuum`] épingle ce fait-là.
//!
//! La cause exacte n'est pas établie, et c'est précisément pourquoi la garde est
//! structurelle. Ce qu'on peut tenir sans connaître la cause, c'est que le
//! `VACUUM` ne revient pas. Un test de comportement dirait « cette migration-ci
//! n'en fait pas » ; celui-ci dit « aucun site de production de ce crate n'en
//! fait », et il rougit sur une réinsertion où qu'elle soit.
//!
//! **Le périmètre est le crate, pas `migrations.rs`.** Le chemin de démarrage
//! portait **deux** `VACUUM`, pas un : celui de `migrate_v55_to_v56`, et
//! `db.vacuum()` dans `startup_cleanup` (`server/mod.rs`) — ce dernier dans une
//! tâche fire-and-forget lancée *après* `ready`, donc en concurrence avec tout
//! le tick du moteur, et sans garde de version de schéma. Scanner le seul
//! `db/migrations.rs` demandé par l'AC1 du ticket aurait laissé le plus exposé
//! des deux en place. L'AC1 reste satisfaite au mot près : `migrations.rs` est
//! dans le périmètre.
//!
//! **Le registre est livré vide, et c'est le livrable.** Les deux méthodes
//! `Database::vacuum` et `AsyncDatabase::vacuum` ont été **supprimées**, pas
//! laissées sans appelant : une méthode orpheline aurait demandé une entrée de
//! registre, c'est-à-dire une exemption déguisée. Quand la commande opérateur
//! du vol 2 arrivera, elle déclarera son site ici — *on déclare, on
//! n'allowliste pas* (mika#2201), et
//! [`mika2735_le_registre_des_vacuum_na_pas_dentree_morte`] supprime l'entrée le
//! jour où le site disparaît.
//!
//! **`VACUUM INTO` n'est pas exempté par fichier mais par prédicat**, parce que
//! le vol 3 en posera un dans `db.rs` — sur le chemin de démarrage — pour la
//! sauvegarde de pré-migration. Ce n'est pas le même acte : il écrit un fichier
//! neuf et ne touche pas à la base vivante.

use mika_common::source_guard::ProductionScanner;
use std::collections::HashSet;
use std::path::Path;

/// Sites de production de `mika-agent` autorisés à porter un `VACUUM`.
///
/// `(chemin relatif à `src/`, raison lisible)`. **Livré vide.** C'est une
/// déclaration, pas une exemption : une entrée dit *quel* site compacte et
/// *sous quelle garantie d'exclusivité*, et le jour où ce site disparaît
/// [`mika2735_le_registre_des_vacuum_na_pas_dentree_morte`] exige qu'on
/// supprime la ligne.
const VACUUM_PRODUCTION_SITES: &[(&str, &str)] = &[];

/// Les `VACUUM` de production d'un fichier source, ligne par ligne.
///
/// Fonction pure sur `(chemin, source)` pour que les contrôles négatifs
/// tournent sur des sources synthétiques, sans toucher à l'arbre.
///
/// Trois filtres, dans cet ordre :
///
/// 1. [`crate::source_scan::is_test_source_path`] — un fichier de test ne rend
///    aucune violation. C'est la classification de chemin de mika#2321 : un
///    module de test extrait ne porte aucun `#[cfg(test)]` littéral, donc une
///    garde qui tronque au premier `#[cfg(test)]` scanne un fichier de test
///    entier comme de la production. Ce fichier-ci en est l'exemple : il écrit
///    `VACUUM` une dizaine de fois.
/// 2. [`mika_common::source_guard::mask_test_regions`] — les `mod tests` en
///    ligne d'un fichier de production (mika#2398).
/// 3. Les lignes de commentaire deviennent des lignes **vides**, pas rien : la
///    prose *sur* le `VACUUM` n'est pas un `VACUUM`, et il y en a plusieurs
///    paragraphes dans `db/migrations.rs` et `server/mod.rs`. Remplacer par
///    `""` au lieu de supprimer garde les numéros de ligne du fichier réel.
///
/// Puis chaque occurrence ASCII-insensible à la casse de `vacuum` est une
/// violation, **sauf** :
///
/// - caractère précédent alphanumérique, `_` ou `:` — exclut `auto_vacuum`,
///   `incremental_vacuum`, `crate::db::vacuum::…` ;
/// - caractère suivant alphanumérique, `_`, `:` ou `(` — exclut
///   `vacuum_exclusive`, `db_vacuum_refused`, `vacuum::run_exclusive`, et
///   l'appel d'une fonction nommée `vacuum` ;
/// - mot suivant `INTO` — `VACUUM INTO` écrit un fichier neuf et ne réécrit pas
///   la base vivante (vol 3 du ticket).
fn vacuum_sites(path: &Path, src: &str) -> Vec<(String, usize)> {
    if crate::source_scan::is_test_source_path(path) {
        return Vec::new();
    }

    let production = mika_common::source_guard::mask_test_regions(src);
    let code: String = production
        .lines()
        .map(|l| {
            if l.trim_start().starts_with("//") {
                ""
            } else {
                l
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    // `to_ascii_lowercase` préserve la longueur en octets, donc les index de la
    // copie minuscule sont ceux de `code`. `to_lowercase` ne le garantit pas.
    let haystack = code.to_ascii_lowercase();

    let display = path.display().to_string();
    let rel = display
        .split_once("/src/")
        .map(|(_, tail)| tail.to_string())
        .unwrap_or(display);

    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(off) = haystack[from..].find("vacuum") {
        let at = from + off;
        from = at + "vacuum".len();

        let before = code[..at].chars().next_back();
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == ':') {
            continue;
        }

        let rest = &code[from..];
        let after = rest.chars().next();
        if after.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == ':' || c == '(') {
            continue;
        }

        // Octets, pas tranche de `&str` : `next_word` peut commencer par un
        // caractère multi-octets, et `scripts/check-byte-slices.sh` refuse la
        // tranche à index fixe pour cette raison exacte.
        let next_word = rest.trim_start().as_bytes();
        if next_word.len() != rest.len()
            && next_word.len() >= 4
            && next_word[..4].eq_ignore_ascii_case(b"into")
            && !next_word
                .get(4)
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            continue;
        }

        out.push((rel.clone(), code[..at].matches('\n').count() + 1));
    }
    out
}

/// Tous les `VACUUM` de production du crate, registre non appliqué.
///
/// Les deux tests ci-dessous lisent la **même** liste : c'est ce qui garantit
/// qu'une entrée déclarée morte l'est pour la garde aussi.
fn tous_les_vacuum_du_crate() -> Vec<(String, usize)> {
    let scanner = ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
    let mut sites = Vec::new();
    let mut scanned = 0usize;
    scanner.for_each(|path, production| {
        scanned += 1;
        sites.extend(vacuum_sites(path, production));
    });
    assert!(
        scanned > 0,
        "la garde n'a scanné aucun fichier — chemin cassé"
    );
    sites
}

/// Garde (a) de mika#2735 : aucun `VACUUM` non déclaré dans la production.
#[test]
fn mika2735_aucun_vacuum_dans_la_production_de_mika_agent() {
    let violations: Vec<String> = tous_les_vacuum_du_crate()
        .into_iter()
        .filter(|(rel, _)| !VACUUM_PRODUCTION_SITES.iter().any(|(f, _)| f == rel))
        .map(|(rel, line)| format!("{rel}:{line}"))
        .collect();

    assert!(
        violations.is_empty(),
        "un `VACUUM` est de retour dans la production de mika-agent \
         (mika#2735) : {violations:?}\n\
         Un `VACUUM` réécrit le fichier vivant, et sur une base née en \
         `auto_vacuum=NONE` il en convertit le format sous les autres \
         connexions — c'est ce qui a abîmé 25 arbres le 2026-10-16. Si ce site \
         est un compactage sous exclusivité explicite, déclare-le dans \
         VACUUM_PRODUCTION_SITES avec sa garantie ; sinon, retire-le. \
         N'élargis pas le prédicat et n'ajoute pas d'exemption par fichier."
    );
}

/// Le registre ne garde pas d'entrée dont le site a disparu.
///
/// Sans cette assertion, une entrée survit à son site et devient une exemption
/// permanente que plus personne ne relit.
#[test]
fn mika2735_le_registre_des_vacuum_na_pas_dentree_morte() {
    let sites = tous_les_vacuum_du_crate();
    let vus: HashSet<&str> = sites.iter().map(|(rel, _)| rel.as_str()).collect();

    let mortes: Vec<&str> = VACUUM_PRODUCTION_SITES
        .iter()
        .filter(|(f, _)| !vus.contains(*f))
        .map(|(f, _)| *f)
        .collect();

    assert!(
        mortes.is_empty(),
        "VACUUM_PRODUCTION_SITES déclare des fichiers qui ne portent plus de \
         `VACUUM` : {mortes:?} — supprime les entrées (mika#2735)."
    );
}

/// Contrôle négatif : un `VACUUM` réinséré fait rougir le scan.
#[test]
fn mika2735_le_scan_rougit_sur_un_vacuum_reinsere() {
    let src = r#"
impl Migrations<'_> {
    fn migrate_v55_to_v56(&mut self) -> Result<()> {
        self.conn.execute_batch("VACUUM;")?;
        Ok(())
    }
}
"#;
    let sites = vacuum_sites(
        Path::new("/repo/crates/mika-agent/src/db/migrations.rs"),
        src,
    );
    assert_eq!(
        sites.len(),
        1,
        "le scan doit voir le `VACUUM` réinséré, et lui seul : {sites:?}"
    );
    assert_eq!(sites[0].0, "db/migrations.rs");
}

/// `VACUUM INTO` n'est pas un `VACUUM` : il écrit un fichier neuf.
///
/// Exempté **par prédicat**, pas par fichier, parce que le vol 3 du ticket en
/// posera un dans `db.rs` — sur le chemin de démarrage — pour la sauvegarde de
/// pré-migration.
#[test]
fn mika2735_vacuum_into_nest_pas_un_vacuum() {
    let src = r#"
impl Database {
    fn backup_to(&self, p: &str) -> Result<()> {
        self.conn.execute("VACUUM INTO ?1", [p])?;
        Ok(())
    }
}
"#;
    let sites = vacuum_sites(Path::new("/repo/crates/mika-agent/src/db.rs"), src);
    assert!(
        sites.is_empty(),
        "`VACUUM INTO` écrit un fichier neuf et ne réécrit pas la base vivante \
         — il ne doit pas être une violation : {sites:?}"
    );
}

/// La prose et `auto_vacuum` ne sont pas des violations.
///
/// Les deux faux positifs qui rendraient la garde intenable : le `PRAGMA
/// auto_vacuum = INCREMENTAL` de `Database::open` (hors périmètre, nommé), et
/// les paragraphes de commentaire qui *expliquent* le retrait.
#[test]
fn mika2735_la_prose_et_auto_vacuum_ne_sont_pas_des_violations() {
    let src = r#"
impl Database {
    /// Le `VACUUM` de mika#2658 a été retiré ici par mika#2735 : voir le
    /// runbook. Un VACUUM au démarrage réécrit le fichier vivant.
    fn open_pragmas(&self) -> Result<()> {
        // Hors périmètre de mika#2735 : voir le doc-comment sur le VACUUM.
        self.conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")?;
        Ok(())
    }
}
"#;
    let sites = vacuum_sites(Path::new("/repo/crates/mika-agent/src/db.rs"), src);
    assert!(
        sites.is_empty(),
        "ni la prose en commentaire ni `auto_vacuum` ne sont des `VACUUM` : {sites:?}"
    );
}

/// Un fichier de test n'est jamais une violation, quoi qu'il écrive.
///
/// Classification de chemin de mika#2321 : ce fichier-ci en est l'exemple
/// vivant. Sans ce filtre la garde se mordrait la queue.
#[test]
fn mika2735_un_fichier_de_test_nest_jamais_une_violation() {
    let src = "fn t() { conn.execute_batch(\"VACUUM;\").unwrap(); }\n";
    for p in [
        "/repo/crates/mika-agent/src/db/migrations/tests/aucun_vacuum_au_demarrage.rs",
        "/repo/crates/mika-agent/src/perimeter/tests.rs",
    ] {
        let sites = vacuum_sites(Path::new(p), src);
        assert!(sites.is_empty(), "{p} est un chemin de test : {sites:?}");
    }
}

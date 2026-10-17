//! Le fait que mika#2735 épingle : sur une base née en `auto_vacuum=NONE`, un
//! `VACUUM` ne compacte pas, il **convertit le format du fichier**.
//!
//! C'est le chaînon qui relie le `VACUUM` de la v56 aux signatures relevées le
//! 2026-10-16 — `Bad ptr map entry`, `2nd reference to page` — : la *pointer
//! map* n'existe que dans les fichiers `auto_vacuum` ≠ 0, et c'est le `VACUUM`
//! qui l'ajoute. Autrement dit, le `VACUUM` du démarrage n'écrivait pas
//! seulement les pages existantes ailleurs ; il changeait la structure du
//! fichier sous les autres connexions du processus.
//!
//! **Pourquoi ce test existe alors que l'AC1 comportementale du ticket n'est
//! pas livrée.** L'AC1 de la première liste demande un contrôle négatif sur un
//! pool de connexions : « sans le correctif, au moins une connexion échoue ».
//! Ce contrôle n'est pas livré, et le dire est préférable à le simuler — la
//! reproduction de l'incident dépend d'un entrelacement dont on n'a pas la
//! cause exacte, donc un test vert ne prouverait rien et un test rouge serait
//! instable. Ce qui le remplace est fait de deux pièces déterministes : la
//! garde structurelle de [`super::aucun_vacuum_au_demarrage`], dont le contrôle
//! négatif *est* déterministe, et le fait ci-dessous, qui établit que l'acte
//! retiré était bien une conversion de format et non un simple compactage.
//!
//! Le test tourne sur un fichier réel, pas sur `open_in_memory` : `auto_vacuum`
//! est une propriété de l'en-tête du fichier, et la conversion est l'objet même
//! de la mesure.

use rusqlite::Connection;

/// `PRAGMA auto_vacuum` de cette connexion, tel que le fichier le porte.
fn auto_vacuum(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))
        .expect("PRAGMA auto_vacuum doit répondre")
}

#[test]
fn mika2735_un_vacuum_convertit_le_format_du_fichier_sous_les_autres_connexions() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("conversion.db");
    let conn = Connection::open(&path).expect("ouverture du fichier");

    // Le fichier doit porter des pages avant qu'on demande la conversion :
    // sur une base encore vide, `PRAGMA auto_vacuum = …` prend effet tout de
    // suite et le fait qu'on veut épingler n'existe pas.
    conn.execute_batch(
        "CREATE TABLE t (id INTEGER PRIMARY KEY, payload TEXT);
         INSERT INTO t (payload) VALUES ('a'), ('b'), ('c');",
    )
    .expect("schéma initial");

    assert_eq!(
        auto_vacuum(&conn),
        0,
        "la base naît en `auto_vacuum=NONE` — c'est la population réelle de \
         mika.db, et la prémisse de tout ce qui suit"
    );

    // Exactement ce que fait `Database::open` (hors périmètre de mika#2735,
    // délibérément laissé en place).
    conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")
        .expect("PRAGMA auto_vacuum = INCREMENTAL");

    assert_eq!(
        auto_vacuum(&conn),
        0,
        "la demande est **inopérante** sur une base déjà peuplée : elle reste \
         en attente, et rien dans le fichier n'a changé. C'est pour cela que \
         le PRAGMA de `Database::open` était inoffensif pendant des mois"
    );

    conn.execute_batch("VACUUM;").expect("VACUUM");

    assert_eq!(
        auto_vacuum(&conn),
        2,
        "c'est le `VACUUM` qui applique la demande en attente : le fichier \
         passe en `auto_vacuum=INCREMENTAL` (2) et gagne des pages de pointer \
         map. L'acte retiré par mika#2735 n'était donc pas un compactage mais \
         une conversion de format du fichier vivant — et les erreurs relevées \
         le 2026-10-16 (`Bad ptr map entry`, `2nd reference to page`) portent \
         le nom de la structure qu'elle ajoute"
    );

    // Les données traversent la conversion : le défaut n'est pas « VACUUM perd
    // des lignes », il est dans la concurrence. Le dire évite de lire ce test
    // comme une accusation de corruption par VACUUM seul.
    let n: i64 = conn
        .query_row("SELECT count(*) FROM t", [], |r| r.get(0))
        .expect("count");
    assert_eq!(n, 3);
}

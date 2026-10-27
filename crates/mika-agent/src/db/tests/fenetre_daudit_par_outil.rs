//! `Database::list_audit_events_for_tool_since` — la lecture fenêtrée que le
//! compte de concordance de mika#2922 (vol 3) fait du registre des verdicts de
//! vol.
//!
//! Les trois voisines par `tool_name` de `db.rs` sont des helpers de test sans
//! borne basse et sans `created_at`. Les quatre propriétés que celle-ci ajoute —
//! la borne, la colonne, la portée par agent et l'ordre total — ne sont couvertes
//! par aucun de leurs tests : un prédicat `created_at` faux, un `agent_id`
//! oublié ou un `ORDER BY created_at` resteraient verts sans ceux-ci.

//! **Les noms d'outil de ces tests sont neutres, et c'est une décision.** La
//! requête est générique sur `tool_name` : aucune de ses quatre propriétés ne
//! dépend de la valeur passée, donc y recopier les `tool_name` du registre
//! n'apporterait rien et ferait de ce fichier un second endroit où ces noms
//! vivent. Le registre en est l'unique porteur
//! (`flight_verdict_ledger::MANAGER_FLIGHT_VERDICT_TOOL` et sa voisine), et
//! c'est ce que sa garde de lecteur unique dit du côté production.

use crate::db::Database;

const TOOL: &str = "outil_a";
const AUTRE_TOOL: &str = "outil_b";
const SESSION: &str = "session-de-test";
const SINCE: &str = "2026-10-01T00:00:00Z";

/// Une ligne du registre, sous l'agent nommé. `register_agent` et
/// `create_session` sont idempotents par l'échec : `audit_events` porte des clés
/// étrangères, et le second appel échoue sans effet.
fn ligne(db: &Database, agent: &str, tool: &str, key: &str, after: Option<&str>) {
    let _ = db.register_agent(agent, agent, "/tmp");
    let _ = db.create_session(SESSION, agent, "cli");
    db.log_audit_event(agent, SESSION, tool, key, None, after, None, None)
        .unwrap();
}

/// Reculer le `created_at` d'une ligne, désignée par sa clé. C'est le seul
/// moyen : la colonne a un DEFAULT et l'écrivain ne la passe pas.
fn reculer(db: &Database, key: &str, instant: &str) {
    db.conn
        .execute(
            "UPDATE audit_events SET created_at = ?1 WHERE target_key = ?2",
            rusqlite::params![instant, key],
        )
        .unwrap();
}

/// **La borne basse exclut, et `created_at` remonte.**
///
/// Les deux moitiés sont indissociables : sans la seconde, une fenêtre correcte
/// et une fenêtre qui rend tout seraient distinguables mais l'instrument qui
/// ordonne les vols n'aurait rien à ordonner ; sans la première, une lecture qui
/// ignore `since` lirait des verdicts que la rétention a déjà effacés en face
/// de jugements encore vivants, et compterait des désaccords inventés.
#[test]
fn mika2922_la_fenetre_exclut_une_ligne_anterieure_et_rend_created_at() {
    let db = Database::open_in_memory().unwrap();

    ligne(&db, "mika", TOOL, "task-vieux", Some("{\"a\":1}"));
    reculer(&db, "task-vieux", "2026-09-30T23:59:59Z");
    ligne(&db, "mika", TOOL, "task-dedans", Some("{\"a\":2}"));
    reculer(&db, "task-dedans", "2026-10-02T08:30:00Z");

    let lignes = db
        .list_audit_events_for_tool_since("mika", TOOL, SINCE)
        .unwrap();

    assert_eq!(
        lignes.len(),
        1,
        "une seule des deux lignes est dans la fenêtre : {lignes:?}"
    );
    let (key, after, reasoning, created_at) = &lignes[0];
    assert_eq!(key, "task-dedans");
    assert_eq!(after.as_deref(), Some("{\"a\":2}"));
    assert_eq!(reasoning.as_deref(), None, "un NULL reste absent");
    assert_eq!(
        created_at, "2026-10-02T08:30:00Z",
        "la colonne que les trois voisines ne projettent pas"
    );

    // Contrôle positif de la borne : la même lecture avec un `since` plus
    // ancien voit les deux. Sans lui, « une seule ligne » serait indistinguable
    // d'une requête qui n'en rend jamais plus d'une.
    let toutes = db
        .list_audit_events_for_tool_since("mika", TOOL, "2026-01-01T00:00:00Z")
        .unwrap();
    assert_eq!(toutes.len(), 2, "la ligne exclue existe bel et bien");
}

/// La portée par agent et par `tool_name`. Les deux prédicats sont dans la même
/// clause `WHERE` et aucun test voisin ne les couvre pour cette requête ; un
/// registre qui remonterait les lignes d'un autre agent mettrait en face deux
/// populations qui ne se jugent pas.
#[test]
fn mika2922_la_fenetre_est_bornee_par_lagent_et_par_loutil() {
    let db = Database::open_in_memory().unwrap();

    ligne(&db, "mika", TOOL, "task-a-nous", Some("{}"));
    ligne(&db, "mika-qa", TOOL, "task-dun-autre-agent", Some("{}"));
    ligne(&db, "mika", AUTRE_TOOL, "task-dun-autre-outil", Some("{}"));

    let lignes = db
        .list_audit_events_for_tool_since("mika", TOOL, SINCE)
        .unwrap();
    assert_eq!(
        lignes.iter().map(|(k, ..)| k.as_str()).collect::<Vec<_>>(),
        vec!["task-a-nous"],
        "l'agent et l'outil bornent tous les deux : {lignes:?}"
    );

    // Anti-vacuité des deux exclusions : chacune des deux lignes écartées est
    // bien lisible par la lecture qui la vise.
    assert_eq!(
        db.list_audit_events_for_tool_since("mika-qa", TOOL, SINCE)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db.list_audit_events_for_tool_since("mika", AUTRE_TOOL, SINCE)
            .unwrap()
            .len(),
        1
    );
}

/// **`ORDER BY id`, et non `ORDER BY created_at`.** `created_at` est un TEXT
/// ISO 8601 à la seconde : deux verdicts d'un même cycle sont à égalité, et un
/// tri sur cette colonne rendrait leur ordre arbitraire. Le compte de
/// concordance remonte les vols du plus récent au plus ancien en inversant
/// **cet** ordre, donc une égalité résolue au hasard déplacerait la borne du
/// compte consécutif.
#[test]
fn mika2922_lordre_est_celui_des_id_pas_celui_des_secondes() {
    let db = Database::open_in_memory().unwrap();

    for key in ["task-1", "task-2", "task-3"] {
        ligne(&db, "mika", TOOL, key, Some("{}"));
    }
    // La même seconde pour les trois : l'égalité que la colonne autorise.
    db.conn
        .execute(
            "UPDATE audit_events SET created_at = '2026-10-02T08:30:00Z'",
            [],
        )
        .unwrap();

    let lignes = db
        .list_audit_events_for_tool_since("mika", TOOL, SINCE)
        .unwrap();
    assert_eq!(
        lignes.iter().map(|(k, ..)| k.as_str()).collect::<Vec<_>>(),
        vec!["task-1", "task-2", "task-3"],
        "à `created_at` égal, l'ordre reste celui de l'inscription : {lignes:?}"
    );
}

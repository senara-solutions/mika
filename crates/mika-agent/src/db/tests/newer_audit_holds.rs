//! `Database::newer_audit_holds` — la question de la phase B2 de mika#2671 au
//! registre `qa_pr_sync_observed` : la retenue de CETTE identité a-t-elle un
//! successeur d'une autre identité sur la même clé ?
//!
//! Les tests du garde (`server::handlers`) passent par la requête mais ne
//! couvrent ni la portée par agent ni l'horizon `since` : un prédicat
//! `agent_id` ou `created_at` faux dans le SQL resterait vert sans ceux-ci.

use crate::db::Database;

const TOOL: &str = "qa_pr_sync_observed";
const HELD: &str = "stage=held";
const KEY: &str = "pr:senara-solutions/mika#2659";
const SINCE: &str = "2026-10-06T00:00:00Z";

fn hold(db: &Database, agent: &str, rid: &str) {
    let session = format!("s-{agent}");
    // `audit_events` et `sessions` portent des clés étrangères : l'agent et la
    // session doivent exister (idempotent : le second appel échoue sans effet).
    let _ = db.register_agent(agent, agent, "/tmp");
    let _ = db.create_session(&session, agent, "cli");
    db.log_audit_event(
        agent,
        &session,
        TOOL,
        KEY,
        Some("t"),
        Some(HELD),
        Some(&format!("request_id={rid}")),
        None,
    )
    .unwrap();
}

#[test]
fn mika2671_b2_une_retenue_dun_autre_agent_ne_compte_pas() {
    let db = Database::open_in_memory().unwrap();
    hold(&db, "mika-qa", "A");
    hold(&db, "mika-dev", "B");
    let (own, newer) = db
        .newer_audit_holds("mika-qa", TOOL, HELD, KEY, "request_id=A", SINCE)
        .unwrap();
    assert!(own.is_some());
    assert_eq!(
        newer, 0,
        "une retenue d'un autre agent n'est pas un successeur"
    );
    // Contrôle positif : la même retenue sous le même agent compte.
    hold(&db, "mika-qa", "B");
    let (_, newer) = db
        .newer_audit_holds("mika-qa", TOOL, HELD, KEY, "request_id=A", SINCE)
        .unwrap();
    assert_eq!(newer, 1);
}

#[test]
fn mika2671_b2_une_retenue_a_soi_hors_horizon_nest_pas_un_temoin() {
    let db = Database::open_in_memory().unwrap();
    hold(&db, "mika-qa", "A");
    db.conn
        .execute(
            "UPDATE audit_events SET created_at = '2026-10-01T00:00:00Z' WHERE reasoning = 'request_id=A'",
            [],
        )
        .unwrap();
    hold(&db, "mika-qa", "B");
    let (own, newer) = db
        .newer_audit_holds("mika-qa", TOOL, HELD, KEY, "request_id=A", SINCE)
        .unwrap();
    assert_eq!(own, None, "hors horizon, la retenue à soi ne situe rien");
    assert_eq!(newer, 0);
}

#[test]
fn mika2671_b2_la_derniere_retenue_a_soi_fait_foi() {
    // Deux retenues de la même identité encadrant une retenue d'autrui : la
    // dernière à soi est postérieure, donc aucun successeur.
    let db = Database::open_in_memory().unwrap();
    hold(&db, "mika-qa", "A");
    hold(&db, "mika-qa", "B");
    hold(&db, "mika-qa", "A");
    let (_, newer) = db
        .newer_audit_holds("mika-qa", TOOL, HELD, KEY, "request_id=A", SINCE)
        .unwrap();
    assert_eq!(newer, 0);
}

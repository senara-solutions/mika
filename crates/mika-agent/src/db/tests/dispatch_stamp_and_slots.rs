//! Tests de `crate::db` — thème `dispatch_stamp_and_slots` (mika#2321).
//!
//! Le stamp `fired_at` du parent (mika#2335 F2a) et sa garde de source,
//! l'arbitrage des slots d'exécution (mika#1948 Porte 2) et le budget de
//! re-drive d'`auto_pull_stats` (mika#2020).
//!
//! Enfant de `db::tests`, donc descendant de `db` : les items privés de `db` et
//! les helpers de `db::tests` restent visibles via `use super::*`.

use super::*;

// ── mika#2335 F2a: la transition de dispatch d'un parent a UN écrivain ──

/// Aucun chemin de production ne transitionne un parent avec
/// `update_manual_task_status(…, "in_progress")` — c'est
/// [`Database::mark_parent_dispatched`] qui le fait, stamp compris.
///
/// **Pourquoi un scan de source et pas un test comportemental.** La
/// régression que cette garde attrape ne rend aucune décision fausse sur
/// les chemins couverts : elle ajoute un *quatrième* chemin de dispatch,
/// muet sur `fired_at`, pendant que toutes les assertions existantes
/// restent vertes. Famille
/// `mika2205_periodic_scans_do_not_read_the_pat_field_directly` /
/// `mika2131_exclusion_skips_never_return_to_an_uncollected_debug`.
///
/// Les deux sites secondaires (`ready_label_handler`, `verdict_handler`)
/// existent parce qu'un chemin de dispatch a été ajouté en recopiant le
/// premier — leurs commentaires le disent en toutes lettres (« mirrors
/// execute_long_running's #525 transition »). Le quatrième arrivera de la
/// même façon.
///
/// **Allowlist vide, à dessein.** Exempter les trois sites du recensement
/// aurait rendu la garde verte en laissant `fired_at` NULL sur exactement
/// les trois chemins que le ticket décrit — donc en contredisant AC4. Ils
/// ont migré ; aucun n'est exempté.
///
/// **Disposition d'un quatrième site : halt-and-surface.** Pas d'entrée
/// d'allowlist, pas de `#[ignore]`. La réponse dépend d'une question que
/// personne ne peut pré-trancher ici : ce site dispatche-t-il un parent (→
/// il migre, et il était un quatrième visage du défaut) ou non (→ il est
/// légitime, et c'est cette garde qu'il faut affiner) ? Deviner en silence,
/// c'est soit poser un `fired_at` sur une ligne jamais dispatchée, soit
/// exempter un chemin de dispatch muet.
///
/// **Portée : lexicale sur le littéral `"in_progress"`, et c'est un coût
/// assumé.** Deux appelants de production passent le statut par variable et
/// échappent structurellement à ce scan — `rewind.rs` (`before_status`,
/// restaure un statut antérieur) et `tools/update_task_status.rs` (l'outil
/// agent). C'est le bon comportement : aucun des deux n'est un dispatch et
/// aucun ne doit stamper. Mais un futur chemin de dispatch qui
/// construirait son statut dans une variable passerait dessous. Un scan
/// sémantique demanderait une analyse de flot que cette famille de gardes
/// n'a pas.
///
/// Les lignes de commentaire sont neutralisées avant le scan — sans quoi la
/// doc de `mark_parent_dispatched`, qui doit nommer l'appel qu'elle
/// remplace, ferait rougir la garde. Les commentaires de bloc `/* … */` ne
/// le sont pas : ce dépôt n'en écrit pas, et une prose qui en emploierait
/// un pour citer l'appel proscrit se signalerait d'elle-même au premier run.
///
/// **Le code de test est écarté par son CHEMIN depuis mika#2321, plus par sa
/// troncature.** La version d'origine coupait au premier littéral
/// `#[cfg(test)]`, ce qui suppose que le code de test vit inline dans le
/// fichier qu'il teste. mika#2310 a cessé de rendre cette prémisse vraie en
/// sortant `db/tests/harnais_porte.rs` dans son propre fichier — un module
/// de test extrait ne porte aucun littéral `#[cfg(test)]`, donc `find` rend
/// `None` et la garde scanne le fichier **entier comme de la production**.
/// mika#2321 sort 463 Ko et 431 tests par ce même trou, d'où la réparation.
/// Voir [`crate::source_scan`] pour le prédicat et pourquoi il porte sur la
/// classification et non sur les aiguilles.
#[test]
fn mika2335_no_production_dispatch_transitions_a_parent_without_stamping() {
    let src_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations: Vec<String> = Vec::new();
    let mut scanned = 0usize;

    for path in rust_sources_under(&src_root) {
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("la garde doit pouvoir lire {}: {e}", path.display()));
        scanned += 1;
        violations.extend(dispatch_stamp_violations(&path, &src));
    }

    assert!(
        scanned > 0,
        "la garde n'a scanné aucun fichier — chemin cassé"
    );
    assert!(
        violations.is_empty(),
        "mika#2335 F2a — ces sites de production transitionnent un parent \
             sans stamper `fired_at` : {violations:?}\n\
             Si le site dispatche un parent, il doit appeler \
             `mark_parent_dispatched`. S'il ne dispatche pas, c'est la garde \
             qu'il faut affiner — et dans les deux cas c'est une décision à \
             prendre explicitement, pas une exemption à poser en passant."
    );
}

/// **Contrôle négatif de la garde ci-dessus (mika#2321 V3-b).**
///
/// Une réparation de classification trop large rendrait la garde *vacuous*
/// sans rien casser : elle resterait verte parce qu'elle ne regarde plus
/// rien. Ce test prouve les deux moitiés sur la même source fautive —
/// attrapée à un chemin de production, écartée à un chemin de test.
///
/// Sans lui, mika#2321 aurait pu faire passer 463 Ko de fixtures devant la
/// garde et on ne l'aurait su qu'au quatrième site de dispatch muet.
#[test]
fn mika2321_the_dispatch_stamp_guard_still_catches_a_production_site() {
    let offending = r#"
            pub async fn dispatch_something(db: &AsyncDatabase, id: &str) {
                db.update_manual_task_status(id, "in_progress", None)
                    .await
                    .ok();
            }
        "#;

    let production = std::path::Path::new("/repo/crates/mika-agent/src/server/new_path.rs");
    assert_eq!(
        dispatch_stamp_violations(production, offending).len(),
        1,
        "la garde ne détecte plus un site de production fautif — elle est \
             devenue vacuous, ce qui ne casse rien et ne protège plus rien"
    );

    // Neutralisé par un commentaire : la prose doit pouvoir décrire l'appel.
    let commented = "// db.update_manual_task_status(id, \"in_progress\", None)";
    assert!(dispatch_stamp_violations(production, commented).is_empty());

    // La même source, à un chemin de test, est écartée entièrement.
    for test_path in [
        "/repo/crates/mika-agent/src/db/tests/tasks.rs",
        "/repo/crates/mika-agent/src/perimeter/tests.rs",
    ] {
        assert!(
            dispatch_stamp_violations(std::path::Path::new(test_path), offending).is_empty(),
            "{test_path} est scanné comme de la production"
        );
    }
}

// ── mika#1948 Porte 2: exec-slot arbitration ──

/// A fresh DB must carry the v51 surface without any migration running —
/// `migrate_v1` builds the schema directly, so a column added only to the
/// migration would be missing on every new install.
#[test]
fn test_fresh_db_has_dispatcher_source_and_lease_table() {
    let db = db();
    let cols: Vec<String> = db
        .conn
        .prepare("PRAGMA table_info(tasks)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    assert!(
        cols.iter().any(|c| c == "dispatcher_source"),
        "fresh DB must have tasks.dispatcher_source — got {cols:?}"
    );
    db.conn
        .query_row("SELECT COUNT(*) FROM dispatch_slot_leases", [], |r| {
            r.get::<_, i64>(0)
        })
        .expect("fresh DB must have the dispatch_slot_leases table");
}

// ---- mika#2160: the lease cap becomes choosable ----

/// A fresh DB must carry the v52 surface without any migration running —
/// `migrate_v1` builds the schema directly, so a key column added only to
/// the migration would be missing on every new install.
#[test]
fn test_fresh_db_lease_table_has_slot_index_in_its_key() {
    let db = db();
    let cols: Vec<(String, i64)> = db
        .conn
        .prepare("PRAGMA table_info(dispatch_slot_leases)")
        .unwrap()
        .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(5)?)))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    let slot = cols.iter().find(|(n, _)| n == "slot_index");
    assert!(
        slot.is_some_and(|(_, pk)| *pk > 0),
        "fresh DB must carry slot_index inside the PRIMARY KEY — got {cols:?}"
    );
}

/// mika#2189 D5: the column exists on a **fresh** install, not only after
/// the ALTER. A migration that adds a column the `CREATE TABLE` forgot
/// leaves every new database silently missing it — and the write path,
/// which names its columns, would then fail on exactly the deployments
/// that have no history to migrate.
#[test]
fn fresh_db_has_llm_calls_request_bytes() {
    let db = db();
    assert!(
        db.column_exists("llm_calls", "request_bytes").unwrap(),
        "a fresh database must carry llm_calls.request_bytes"
    );
}

/// AC4, lease half. Two concurrent `implement` claimants each get a lease
/// at a cap of two, and a third is refused.
///
/// The assertion is on **two `Acquired`**, not on the guard passing: before
/// mika#2160 the lease PRIMARY KEY was itself a cap of one, so a cap that
/// only moved the count predicate would read as configured and serialize
/// anyway. This is the test that makes the setting non-decorative (KTD1).
///
/// No environment is mutated: the cap is a parameter, so the module's
/// tests keep running in parallel.
#[test]
fn test_two_implement_claims_each_take_a_lease_at_cap_two() {
    let mut db = db();
    let a = db
        .try_acquire_dispatch_slot("mika", "implement", "task-a", Some("mika_dev"), 120, 2)
        .unwrap();
    assert_eq!(a, SlotClaim::Acquired, "first claimant takes a slot");

    let b = db
        .try_acquire_dispatch_slot("mika", "implement", "task-b", Some("mika_manager"), 120, 2)
        .unwrap();
    assert_eq!(
        b,
        SlotClaim::Acquired,
        "second claimant must take the SECOND slot — a cap of 2 that only \
             refuses is the decorative-setting regression KTD1 names"
    );

    // Two distinct holders, two distinct indices.
    let holders: Vec<(String, i64)> = db
        .conn
        .prepare(
            "SELECT holder_task_id, slot_index FROM dispatch_slot_leases
                  WHERE agent_id = 'mika' AND dispatch_class = 'implement'
                  ORDER BY slot_index",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    assert_eq!(
        holders,
        vec![("task-a".to_string(), 0), ("task-b".to_string(), 1)],
        "each dispatch must hold its own indexed slot"
    );

    let c = db
        .try_acquire_dispatch_slot("mika", "implement", "task-c", None, 120, 2)
        .unwrap();
    assert!(
        matches!(c, SlotClaim::Held { .. }),
        "a third claimant must be refused at a cap of two — got {c:?}"
    );
}

/// AC5, lease half. At the default cap of 1 the lease behaves exactly as it
/// did before v52: one holder, one row, at index 0.
#[test]
fn test_cap_of_one_is_the_pre_v52_behaviour() {
    let mut db = db();
    assert_eq!(
        db.try_acquire_dispatch_slot("mika", "implement", "task-a", None, 120, 1)
            .unwrap(),
        SlotClaim::Acquired
    );
    assert!(
        matches!(
            db.try_acquire_dispatch_slot("mika", "implement", "task-b", None, 120, 1)
                .unwrap(),
            SlotClaim::Held { .. }
        ),
        "the default cap must still serialize the class to one"
    );
    let (rows, max_idx): (i64, i64) = db
        .conn
        .query_row(
            "SELECT COUNT(*), COALESCE(MAX(slot_index), -1) FROM dispatch_slot_leases",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((rows, max_idx), (1, 0), "exactly one row, at index 0");
}

/// A holder that already owns a slot must not consume a second one when it
/// re-claims. Before v52 the upsert's `OR holder_task_id = ?` arm gave this
/// for free; with several slots it has to be checked first.
#[test]
fn test_re_claim_by_same_holder_does_not_consume_a_second_slot() {
    let mut db = db();
    db.try_acquire_dispatch_slot("mika", "implement", "task-a", None, 120, 3)
        .unwrap();
    assert_eq!(
        db.try_acquire_dispatch_slot("mika", "implement", "task-a", None, 120, 3)
            .unwrap(),
        SlotClaim::Acquired,
        "re-claim by the same holder stays idempotent"
    );
    let rows: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM dispatch_slot_leases", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(rows, 1, "one dispatch must never hold two slots");
}

/// A cap lowered while leases are live must not leave the class
/// over-subscribed. Found in review: scanning `0..max_slots` alone cannot
/// see a live row ABOVE the new ceiling, so a freed low index would be
/// handed out while a high one is still held.
#[test]
fn test_cap_downshift_does_not_over_subscribe_the_class() {
    let mut db = db();
    // Two live leases at a cap of two: slots 0 and 1.
    db.try_acquire_dispatch_slot("mika", "implement", "task-a", None, 120, 2)
        .unwrap();
    db.try_acquire_dispatch_slot("mika", "implement", "task-b", None, 120, 2)
        .unwrap();

    // Slot 0 frees (its holder released); slot 1 is still live.
    assert!(
        db.release_dispatch_slot("mika", "implement", "task-a")
            .unwrap()
    );

    // The operator reverts the cap to the default. A new claim must be
    // refused: one lease is already live and the cap is one — even though
    // index 0 is now free.
    let c = db
        .try_acquire_dispatch_slot("mika", "implement", "task-c", None, 120, 1)
        .unwrap();
    assert!(
        matches!(c, SlotClaim::Held { .. }),
        "a cap lowered to 1 with one live lease must refuse — got {c:?}"
    );

    let live: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM dispatch_slot_leases WHERE expires_at > \
                 strftime('%Y-%m-%dT%H:%M:%SZ','now')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        live, 1,
        "the class must never hold more leases than its cap"
    );
}

/// With the cap disabled, an expired index must be reclaimed rather than a
/// new one minted. Found in review: `release_dispatch_slot` has no
/// production caller, so without reclaim the table would grow by one row
/// per dispatch, forever — something the pre-v52 single-row PRIMARY KEY
/// made impossible to express.
#[test]
fn test_cap_zero_reclaims_an_expired_index_instead_of_leaking() {
    let mut db = db();
    // Born expired (ttl 0), so it is reclaimable immediately.
    db.try_acquire_dispatch_slot("mika", "implement", "dead-1", None, 0, 0)
        .unwrap();
    db.try_acquire_dispatch_slot("mika", "implement", "dead-2", None, 0, 0)
        .unwrap();

    // Two fresh claims must land on the two expired indices, not above them.
    db.try_acquire_dispatch_slot("mika", "implement", "live-1", None, 120, 0)
        .unwrap();
    db.try_acquire_dispatch_slot("mika", "implement", "live-2", None, 120, 0)
        .unwrap();

    let (rows, max_idx): (i64, i64) = db
        .conn
        .query_row(
            "SELECT COUNT(*), COALESCE(MAX(slot_index), -1) FROM dispatch_slot_leases",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (rows, max_idx),
        (2, 1),
        "the table must be bounded by live leases, not by total dispatches"
    );
}

/// The explicit disable sentinel (KTD3): `0` lifts the cap rather than
/// meaning "zero dispatches".
#[test]
fn test_cap_zero_lifts_the_lease_cap() {
    let mut db = db();
    for holder in ["t1", "t2", "t3", "t4"] {
        assert_eq!(
            db.try_acquire_dispatch_slot("mika", "implement", holder, None, 120, 0)
                .unwrap(),
            SlotClaim::Acquired,
            "cap 0 disables the cap — {holder} must not be refused"
        );
    }
    let rows: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM dispatch_slot_leases", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(rows, 4, "each holder gets its own appended index");
}

#[test]
fn test_set_task_dispatcher_source_roundtrips() {
    let db = db();
    let id = db
        .create_task(&new_task("mika", "op work", "manual", "none"))
        .unwrap();

    // Default is NULL — the pre-v51 / autonomous-loop case.
    let t = db.get_task(&id, "mika").unwrap().unwrap();
    assert_eq!(t.dispatcher_source, None, "default must stay NULL");

    assert!(
        db.set_task_dispatcher_source(&id, "mika", "operator")
            .unwrap()
    );
    let t = db.get_task(&id, "mika").unwrap().unwrap();
    assert_eq!(t.dispatcher_source.as_deref(), Some("operator"));
}

#[test]
fn test_set_task_dispatcher_source_rejects_unknown_value() {
    let db = db();
    let id = db
        .create_task(&new_task("mika", "op work", "manual", "none"))
        .unwrap();
    let err = db
        .set_task_dispatcher_source(&id, "mika", "zorglub")
        .unwrap_err();
    assert!(
        err.to_string().contains("unknown dispatcher_source"),
        "must name the rejected value — got: {err}"
    );
}

#[test]
fn test_has_pending_operator_task_for_class() {
    let db = db();
    // Anti-vacuity: no operator task → false. Without this the assertion
    // below is satisfied by a function that always returns true.
    assert!(
        !db.has_pending_operator_task_for_class("mika", "implement")
            .unwrap(),
        "no operator task seeded — must be false"
    );

    let id = db
        .create_task(&new_task("mika", "operator work", "manual", "none"))
        .unwrap();
    db.set_task_dispatcher_source(&id, "mika", "operator")
        .unwrap();
    assert!(
        db.has_pending_operator_task_for_class("mika", "implement")
            .unwrap(),
        "pending operator task in the implement class must be seen"
    );
    // Class-scoped: the same task must not register in the other class.
    assert!(
        !db.has_pending_operator_task_for_class("mika", "groom")
            .unwrap(),
        "operator task defaults to the implement class, not groom"
    );
}

#[test]
fn test_has_pending_operator_task_ignores_non_operator_sources() {
    let db = db();
    let id = db
        .create_task(&new_task("mika", "manager work", "manual", "none"))
        .unwrap();
    db.set_task_dispatcher_source(&id, "mika", "mika_manager")
        .unwrap();
    assert!(
        !db.has_pending_operator_task_for_class("mika", "implement")
            .unwrap(),
        "only 'operator' confers priority — mika_manager must not"
    );
}

// --- the atomic claim itself ---

#[test]
fn test_slot_claim_first_wins_second_is_refused() {
    let mut db = db();
    let a = db
        .try_acquire_dispatch_slot("mika", "implement", "task-a", Some("mika_dev"), 120, 1)
        .unwrap();
    assert_eq!(a, SlotClaim::Acquired, "first claimant takes the slot");

    let b = db
        .try_acquire_dispatch_slot("mika", "implement", "task-b", Some("mika_manager"), 120, 1)
        .unwrap();
    match b {
        SlotClaim::Held {
            holder_task_id,
            dispatcher_source,
            ..
        } => {
            assert_eq!(holder_task_id, "task-a");
            assert_eq!(
                dispatcher_source.as_deref(),
                Some("mika_dev"),
                "the refusal must name WHO holds the slot"
            );
        }
        other => panic!("second claimant must be refused, got {other:?}"),
    }
}

/// Anti-vacuity twin of the test above: a guard that refused everything
/// would satisfy "second claimant refused". The slot split must still hold,
/// so a different class is free while `implement` is taken.
#[test]
fn test_slot_claim_is_per_class_not_global() {
    let mut db = db();
    db.try_acquire_dispatch_slot("mika", "implement", "task-a", None, 120, 1)
        .unwrap();
    let groom = db
        .try_acquire_dispatch_slot("mika", "groom", "task-b", None, 120, 1)
        .unwrap();
    assert_eq!(
        groom,
        SlotClaim::Acquired,
        "the groom slot is a separate slot — taking implement must not close it"
    );
}

/// And per-agent: one agent's busy slot must not block another's.
#[test]
fn test_slot_claim_is_per_agent() {
    let mut db = db();
    db.try_acquire_dispatch_slot("mika", "implement", "task-a", None, 120, 1)
        .unwrap();
    let other = db
        .try_acquire_dispatch_slot("mika-qa", "implement", "task-b", None, 120, 1)
        .unwrap();
    assert_eq!(other, SlotClaim::Acquired, "slots are scoped per agent");
}

/// Re-entrancy: the same task re-validating refreshes its own lease rather
/// than deadlocking against itself. Without this, any retry of a dispatch
/// that already claimed the slot would be permanently refused.
#[test]
fn test_slot_claim_is_reentrant_for_same_holder() {
    let mut db = db();
    db.try_acquire_dispatch_slot("mika", "implement", "task-a", None, 120, 1)
        .unwrap();
    let again = db
        .try_acquire_dispatch_slot("mika", "implement", "task-a", None, 120, 1)
        .unwrap();
    assert_eq!(
        again,
        SlotClaim::Acquired,
        "a task must be able to re-claim the slot it already holds"
    );
}

/// Expiry is what keeps fail-closed from becoming loop-breaking: a
/// dispatcher that dies mid-claim must stall its class for one TTL, not
/// forever. A zero TTL makes the lease born expired, so the next claimant
/// reclaims it.
#[test]
fn test_expired_lease_is_reclaimable() {
    let mut db = db();
    db.try_acquire_dispatch_slot("mika", "implement", "dead-task", None, 0, 1)
        .unwrap();
    let next = db
        .try_acquire_dispatch_slot("mika", "implement", "live-task", None, 120, 1)
        .unwrap();
    assert_eq!(
        next,
        SlotClaim::Acquired,
        "an expired lease must not strand the slot"
    );
    let holder = db.dispatch_slot_lease_holder("mika", "implement").unwrap();
    assert_eq!(
        holder.map(|h| h.0),
        Some("live-task".to_string()),
        "the reclaiming task becomes the holder"
    );
}

#[test]
fn test_expired_lease_reads_as_free() {
    let mut db = db();
    db.try_acquire_dispatch_slot("mika", "implement", "dead-task", None, 0, 1)
        .unwrap();
    assert_eq!(
        db.dispatch_slot_lease_holder("mika", "implement").unwrap(),
        None,
        "an expired lease must read as free, not as held"
    );
}

#[test]
fn test_release_frees_the_slot_for_another_dispatcher() {
    let mut db = db();
    db.try_acquire_dispatch_slot("mika", "implement", "task-a", None, 120, 1)
        .unwrap();
    assert!(
        db.release_dispatch_slot("mika", "implement", "task-a")
            .unwrap()
    );
    let b = db
        .try_acquire_dispatch_slot("mika", "implement", "task-b", None, 120, 1)
        .unwrap();
    assert_eq!(b, SlotClaim::Acquired, "released slot must be claimable");
}

/// A release from a task that no longer owns the lease must not free it.
/// Otherwise a dispatcher whose lease expired and was re-taken could, on a
/// late cleanup, hand the slot away from its current holder.
#[test]
fn test_release_by_non_holder_is_a_noop() {
    let mut db = db();
    db.try_acquire_dispatch_slot("mika", "implement", "task-a", None, 120, 1)
        .unwrap();
    assert!(
        !db.release_dispatch_slot("mika", "implement", "task-b")
            .unwrap(),
        "a non-holder must not be able to release the lease"
    );
    let holder = db.dispatch_slot_lease_holder("mika", "implement").unwrap();
    assert_eq!(
        holder.map(|h| h.0),
        Some("task-a".to_string()),
        "the original holder must still hold it"
    );
}

/// The blocking-dispatch report must carry the source so a rejection can
/// name who holds the slot (AC2).
#[test]
fn test_blocking_dispatch_carries_dispatcher_source() {
    let db = db();
    let parent = db
        .create_task(&new_task("mika", "blocking parent", "manual", "none"))
        .unwrap();
    db.set_task_dispatcher_source(&parent, "mika", "mika_dev")
        .unwrap();
    let mut cb = new_task(
        "mika",
        "long_running:run_claude_pilot",
        "callback",
        "resume_agent",
    );
    cb.parent_task_id = Some(parent.clone());
    cb.dispatch_class = Some("implement".to_string());
    db.create_task(&cb).unwrap();

    let blocking = db
        .has_active_callback_tasks_excluding("other-task", "mika", "implement")
        .unwrap()
        .expect("the active callback must be seen as blocking");
    assert_eq!(blocking.parent_task_id, parent);
    assert_eq!(
        blocking.dispatcher_source.as_deref(),
        Some("mika_dev"),
        "the blocker's dispatcher must be reported"
    );
}

/// A pre-v51 blocker (NULL source) must report `None`, NOT a defaulted
/// "mika_dev". The read site applies the COALESCE; manufacturing the
/// default this deep would turn "we don't know" into a positive claim.
#[test]
fn test_blocking_dispatch_null_source_is_reported_as_unknown() {
    let db = db();
    let parent = db
        .create_task(&new_task("mika", "legacy parent", "manual", "none"))
        .unwrap();
    let mut cb = new_task(
        "mika",
        "long_running:run_claude_pilot",
        "callback",
        "resume_agent",
    );
    cb.parent_task_id = Some(parent.clone());
    cb.dispatch_class = Some("implement".to_string());
    db.create_task(&cb).unwrap();

    let blocking = db
        .has_active_callback_tasks_excluding("other-task", "mika", "implement")
        .unwrap()
        .expect("blocking callback must be seen");
    assert_eq!(
        blocking.dispatcher_source, None,
        "a pre-v51 row must stay unknown rather than be defaulted here"
    );
}

#[test]
fn test_lease_ttl_env_override_rejects_non_positive() {
    // A zero or negative TTL would make every lease born expired, silently
    // disabling the guard — the override must fall back instead.
    unsafe { std::env::set_var("MIKA_DISPATCH_SLOT_LEASE_TTL_SECS", "0") };
    assert_eq!(dispatch_slot_lease_ttl_secs(), DISPATCH_SLOT_LEASE_TTL_SECS);
    unsafe { std::env::set_var("MIKA_DISPATCH_SLOT_LEASE_TTL_SECS", "-5") };
    assert_eq!(dispatch_slot_lease_ttl_secs(), DISPATCH_SLOT_LEASE_TTL_SECS);
    unsafe { std::env::set_var("MIKA_DISPATCH_SLOT_LEASE_TTL_SECS", "45") };
    assert_eq!(dispatch_slot_lease_ttl_secs(), 45);
    unsafe { std::env::remove_var("MIKA_DISPATCH_SLOT_LEASE_TTL_SECS") };
}

// ── mika#2020: re-drive budget on auto_pull_stats ──

#[test]
fn test_auto_pull_redrive_state_defaults_to_zero() {
    let db = db();
    assert_eq!(
        db.get_auto_pull_redrive_state("senara-solutions/mika", 1901)
            .unwrap(),
        (0, false),
        "a ticket with no row has spent no budget and is not abandoned"
    );
}

#[test]
fn test_auto_pull_redrive_increment_and_reset() {
    let db = db();
    let repo = "senara-solutions/mika";
    for _ in 0..3 {
        db.increment_auto_pull_redrive(repo, 1901).unwrap();
    }
    assert_eq!(
        db.get_auto_pull_redrive_state(repo, 1901).unwrap(),
        (3, false)
    );

    db.reset_auto_pull_redrive(repo, 1901).unwrap();
    assert_eq!(
        db.get_auto_pull_redrive_state(repo, 1901).unwrap(),
        (0, false)
    );
}

#[test]
fn test_auto_pull_redrive_survives_failure_counter_reset() {
    // The load-bearing test for mika#2020 KTD1: `reset_auto_pull_failure`
    // runs on EVERY successful rescue. If it also cleared the re-drive
    // budget, the 16-requeue loop on mika#1901 would be rebuilt exactly.
    let db = db();
    let repo = "senara-solutions/mika";

    db.increment_auto_pull_failure(repo, 1901).unwrap();
    db.increment_auto_pull_redrive(repo, 1901).unwrap();
    db.increment_auto_pull_redrive(repo, 1901).unwrap();

    db.reset_auto_pull_failure(repo, 1901).unwrap();

    assert_eq!(db.get_auto_pull_failure_count(repo, 1901).unwrap(), 0);
    assert_eq!(
        db.get_auto_pull_redrive_state(repo, 1901).unwrap(),
        (2, false),
        "the re-drive budget must NOT be cleared by the failure-counter reset"
    );
}

#[test]
fn test_auto_pull_redrive_abandonment_stamp_round_trip() {
    let db = db();
    let repo = "senara-solutions/mika";

    db.increment_auto_pull_redrive(repo, 1901).unwrap();
    db.mark_auto_pull_redrive_abandoned(repo, 1901).unwrap();
    assert_eq!(
        db.get_auto_pull_redrive_state(repo, 1901).unwrap(),
        (1, true)
    );

    // The operator lifts the abandonment.
    db.reset_auto_pull_redrive(repo, 1901).unwrap();
    assert_eq!(
        db.get_auto_pull_redrive_state(repo, 1901).unwrap(),
        (0, false),
        "re-entry clears both the count and the stamp"
    );
}

#[test]
fn test_auto_pull_redrive_abandon_on_a_ticket_with_no_prior_row() {
    // The plan-ownership abandonment path stamps without ever incrementing.
    let db = db();
    let repo = "senara-solutions/mika";
    db.mark_auto_pull_redrive_abandoned(repo, 1887).unwrap();
    assert_eq!(
        db.get_auto_pull_redrive_state(repo, 1887).unwrap(),
        (0, true)
    );
}

/// **T8 (mika#2361)** — the abandonment *instant*, which is the bound of the
/// re-entry-blocked comment's dedup window (D5).
///
/// Three states, and the third is the one that matters: after a re-entry the
/// accessor must read `None` again, or a ticket abandoned a second time
/// would be bounded by a stale window and the comment would never be posted
/// twice — an idempotence that outlives the thing it is idempotent about.
#[test]
fn mika2361_abandoned_at_is_readable_and_none_when_not_abandoned() {
    let db = db();
    let repo = "senara-solutions/mika";

    assert_eq!(
        db.get_auto_pull_redrive_abandoned_at(repo, 2360).unwrap(),
        None,
        "a ticket with no row at all is not abandoned"
    );

    db.increment_auto_pull_redrive(repo, 2360).unwrap();
    assert_eq!(
        db.get_auto_pull_redrive_abandoned_at(repo, 2360).unwrap(),
        None,
        "a row exists but nothing was abandoned — NULL is not an instant"
    );

    db.mark_auto_pull_redrive_abandoned(repo, 2360).unwrap();
    let at = db
        .get_auto_pull_redrive_abandoned_at(repo, 2360)
        .unwrap()
        .expect("an abandoned ticket carries its instant");
    assert!(
        crate::timestamp::parse(&at).is_ok(),
        "the instant must be parseable ISO 8601 — it is compared as a string \
             against `audit_events.created_at`, so a different shape would make \
             the dedup window meaningless rather than wrong-looking: {at}"
    );
    assert_eq!(
        db.get_auto_pull_redrive_state(repo, 2360).unwrap(),
        (1, true),
        "the boolean the decision reads and the instant the comment reads \
             must agree — they are two readings of one column"
    );

    db.reset_auto_pull_redrive(repo, 2360).unwrap();
    assert_eq!(
        db.get_auto_pull_redrive_abandoned_at(repo, 2360).unwrap(),
        None,
        "re-entry clears the instant, so a later abandonment gets a fresh window"
    );
}

#[test]
fn test_open_in_memory_creates_schema() {
    let db = db();
    let version = db.schema_version().unwrap();
    assert_eq!(version, CURRENT_SCHEMA_VERSION);
}

#[test]
fn test_default_agent_registered() {
    let db = db();
    let agents = db.list_agents_db().unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].id, "mika");
}

#[test]
fn test_v3_tables_exist() {
    let db = db();
    let count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table'
                  AND name IN ('agents','teams','tasks','sessions','messages','core_memory',
                               'people','commitments','preferences','events',
                               'audit_events','audit_event_summaries','search_content',
                               'team_runs','team_workspace','heartbeat_sends',
                               'reflection_runs','customer_config','failed_sends')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 19);
}

#[test]
fn test_no_reminders_table() {
    let db = db();
    let exists: bool = db
        .conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='reminders'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!exists);
}

#[test]
fn test_save_and_load_messages() {
    let (db, sid) = db_with_session();
    db.save_message("mika", &sid, "user", "Hello!", None)
        .unwrap();
    db.save_message("mika", &sid, "assistant", "Hi!", None)
        .unwrap();
    let msgs = db.load_recent_messages("mika", 10).unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].role, "user");
    assert_eq!(msgs[1].role, "assistant");
    assert_eq!(msgs[0].session_id, sid);
}

#[test]
fn test_load_recent_messages_limit() {
    let (db, sid) = db_with_session();
    for i in 0..5 {
        db.save_message("mika", &sid, "user", &format!("msg {i}"), None)
            .unwrap();
    }
    let msgs = db.load_recent_messages("mika", 3).unwrap();
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[2].content, "msg 4");
}

#[test]
fn test_load_messages_after() {
    let (db, sid) = db_with_session();
    db.save_message("mika", &sid, "user", "msg 1", None)
        .unwrap();
    db.save_message("mika", &sid, "user", "msg 2", None)
        .unwrap();
    db.save_message("mika", &sid, "user", "msg 3", None)
        .unwrap();

    let all = db.load_messages_after("mika", 0).unwrap();
    assert_eq!(all.len(), 3);

    let first_id = all[0].id;
    let after = db.load_messages_after("mika", first_id).unwrap();
    assert_eq!(after.len(), 2);
    for msg in &after {
        assert!(msg.id > first_id);
    }
}

#[test]
fn test_session_channel_type_via_join() {
    let db = db();
    db.create_session("tg-session", "mika", "telegram").unwrap();
    db.create_session("cli-session", "mika", "cli").unwrap();
    db.save_message("mika", "tg-session", "user", "telegram msg", None)
        .unwrap();
    db.save_message("mika", "cli-session", "user", "cli msg", None)
        .unwrap();

    let msgs = db.load_recent_messages("mika", 10).unwrap();
    assert_eq!(msgs.len(), 2);
    // Channel type comes from session JOIN
    assert!(msgs.iter().any(|m| m.channel_type == "telegram"));
    assert!(msgs.iter().any(|m| m.channel_type == "cli"));
}

#[test]
fn test_load_recent_messages_excludes_team() {
    let db = db();
    db.create_session("team-session", "mika", "team").unwrap();
    db.create_session("cli-session", "mika", "cli").unwrap();
    db.save_message("mika", "team-session", "assistant", "team msg", None)
        .unwrap();
    db.save_message("mika", "cli-session", "user", "cli msg", None)
        .unwrap();

    let msgs = db.load_recent_messages("mika", 10).unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content, "cli msg");
    assert_eq!(msgs[0].channel_type, "cli");
}

#[test]
fn test_load_recent_messages_includes_telegram() {
    let db = db();
    db.create_session("tg-session", "mika", "telegram").unwrap();
    db.save_message("mika", "tg-session", "user", "hello from telegram", None)
        .unwrap();

    let msgs = db.load_recent_messages("mika", 10).unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content, "hello from telegram");
    assert_eq!(msgs[0].channel_type, "telegram");
}

#[test]
fn test_load_recent_messages_mixed_channels() {
    let db = db();
    db.create_session("cli-session", "mika", "cli").unwrap();
    db.create_session("tg-session", "mika", "telegram").unwrap();
    db.create_session("team-session", "mika", "team").unwrap();

    db.save_message("mika", "cli-session", "user", "cli 1", None)
        .unwrap();
    db.save_message("mika", "team-session", "assistant", "team 1", None)
        .unwrap();
    db.save_message("mika", "tg-session", "user", "tg 1", None)
        .unwrap();
    db.save_message("mika", "team-session", "assistant", "team 2", None)
        .unwrap();
    db.save_message("mika", "cli-session", "user", "cli 2", None)
        .unwrap();

    let msgs = db.load_recent_messages("mika", 10).unwrap();
    assert_eq!(msgs.len(), 3);
    // Team messages excluded, cli and telegram present in chronological order
    assert!(msgs.iter().all(|m| m.channel_type != "team"));
    assert!(msgs.iter().any(|m| m.channel_type == "cli"));
    assert!(msgs.iter().any(|m| m.channel_type == "telegram"));
    // Chronological order (reversed from DESC query)
    assert_eq!(msgs[0].content, "cli 1");
    assert_eq!(msgs[1].content, "tg 1");
    assert_eq!(msgs[2].content, "cli 2");
}

#[test]
fn test_get_messages_since() {
    let (db, sid) = db_with_session();
    db.conn
        .execute(
            "INSERT INTO messages (session_id, agent_id, role, content, created_at)
                  VALUES (?1, 'mika', 'user', 'old', '2020-01-01T00:00:00Z')",
            params![sid],
        )
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO messages (session_id, agent_id, role, content, created_at)
                  VALUES (?1, 'mika', 'user', 'new', '2025-01-01T00:00:00Z')",
            params![sid],
        )
        .unwrap();
    let msgs = db
        .get_messages_since("mika", "2024-01-01T00:00:00Z")
        .unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content, "new");
}

#[test]
fn test_get_messages_by_trace_id() {
    let (db, sid) = db_with_session();
    let trace = "aaaa0000bbbb1111cccc2222dddd3333";
    db.save_message_with_metadata("mika", &sid, "user", "traced msg", None, Some(trace), false)
        .unwrap();
    db.save_message("mika", &sid, "assistant", "no trace", None)
        .unwrap();

    let msgs = db.get_messages_by_trace_id(trace).unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content, "traced msg");
    assert_eq!(msgs[0].trace_id.as_deref(), Some(trace));

    let empty = db.get_messages_by_trace_id("nonexistent").unwrap();
    assert!(empty.is_empty());
}

#[test]
fn test_last_user_message_time() {
    let (db, sid) = db_with_session();
    assert!(db.last_user_message_time("mika").unwrap().is_none());
    db.save_message("mika", &sid, "user", "hello", None)
        .unwrap();
    let ts = db.last_user_message_time("mika").unwrap();
    assert!(ts.is_some());
    assert!(!ts.unwrap().is_empty());
}

#[test]
fn test_replace_with_summary() {
    let (mut db, sid) = db_with_session();
    let id1 = db.save_message("mika", &sid, "user", "msg1", None).unwrap();
    db.save_message("mika", &sid, "assistant", "reply1", None)
        .unwrap();
    db.replace_with_summary("mika", "Summary text", id1)
        .unwrap();
    let summary = db.load_conversation_summary("mika").unwrap().unwrap();
    assert_eq!(summary.role, "summary");
    assert_eq!(summary.content, "Summary text");
    let count = db.count_messages("mika").unwrap();
    assert_eq!(count, 1); // only the second message remains + summary excluded
}

#[test]
fn test_count_messages_excludes_summary() {
    let (db, sid) = db_with_session();
    db.save_message("mika", &sid, "user", "a", None).unwrap();
    db.save_message("mika", &sid, "assistant", "b", None)
        .unwrap();
    let sys_session = db.get_or_create_system_session("mika").unwrap();
    db.conn
        .execute(
            "INSERT INTO messages (session_id, agent_id, role, content)
                  VALUES (?1, 'mika', 'summary', 'S')",
            params![sys_session],
        )
        .unwrap();
    assert_eq!(db.count_messages("mika").unwrap(), 2);
}

#[test]
fn test_core_memory_set_and_get() {
    let db = db();
    db.set_core_memory("mika", "user_summary", "Alice").unwrap();
    let entry = db.get_core_memory("mika", "user_summary").unwrap().unwrap();
    assert_eq!(entry.value, "Alice");
}

#[test]
fn test_seed_core_memory() {
    let db = db();
    db.seed_core_memory("mika", None).unwrap();
    let entries = db.get_all_core_memory("mika").unwrap();
    assert_eq!(entries.len(), CORE_MEMORY_SECTIONS.len());
}

#[test]
fn test_seed_core_memory_custom_user_summary() {
    let db = db();
    db.seed_core_memory("mika", Some("Bob")).unwrap();
    let entry = db.get_core_memory("mika", "user_summary").unwrap().unwrap();
    assert_eq!(entry.value, "Bob");
}

#[test]
fn test_upsert_and_get_person() {
    let db = db();
    db.upsert_person("mika", "Alice", Some("colleague"), Some("Works at Acme"))
        .unwrap();
    let p = db.get_person("mika", "Alice").unwrap().unwrap();
    assert_eq!(p.canonical_name, "Alice");
    assert_eq!(p.relationship.unwrap(), "colleague");
}

#[test]
fn test_add_commitment_and_list() {
    let db = db();
    db.add_commitment("mika", "Write report", Some("2026-04-01"), None)
        .unwrap();
    let items = db.list_commitments("mika", "pending").unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].description, "Write report");
}

#[test]
fn test_update_commitment_status() {
    let db = db();
    let id = db.add_commitment("mika", "Task A", None, None).unwrap();
    assert!(
        db.update_commitment_status("mika", id, "completed")
            .unwrap()
    );
    let status = db.get_commitment_status("mika", id).unwrap().unwrap();
    assert_eq!(status, "completed");
}

#[test]
fn test_set_and_get_preference() {
    let db = db();
    db.set_preference("mika", "timezone", "UTC").unwrap();
    let v = db.get_preference("mika", "timezone").unwrap().unwrap();
    assert_eq!(v, "UTC");
}

#[test]
fn test_add_and_list_events() {
    let db = db();
    db.add_event("mika", "Team meeting", Some("2026-04-15"), None)
        .unwrap();
    let evts = db.list_events("mika").unwrap();
    assert_eq!(evts.len(), 1);
    assert_eq!(evts[0].description, "Team meeting");
}

#[test]
fn test_record_and_count_heartbeat_sends() {
    let db = db();
    db.record_heartbeat_send("mika").unwrap();
    db.record_heartbeat_send("mika").unwrap();
    let count = db.count_heartbeat_sends_last_hour("mika").unwrap();
    assert_eq!(count, 2);
}

#[test]
fn test_prune_old_heartbeat_sends() {
    let db = db();
    // Insert old record manually
    db.conn
        .execute(
            "INSERT INTO heartbeat_sends (agent_id, sent_at) VALUES ('mika', 1000)",
            [],
        )
        .unwrap();
    db.record_heartbeat_send("mika").unwrap();
    db.prune_old_heartbeat_sends("mika", 30).unwrap();
    let count = db.count_heartbeat_sends_last_hour("mika").unwrap();
    // Old entry gone, recent one stays
    assert!(count <= 1);
}

#[test]
fn test_record_reflection_run() {
    let db = db();
    db.record_reflection_run("mika", "completed", 3, Some("Updated 3 keys"))
        .unwrap();
}

#[test]
fn test_save_and_get_failed_send() {
    let db = db();
    let id = db
        .save_failed_send("mika", "Failed message", Some("req-1"))
        .unwrap();
    let pending = db.get_pending_failed_sends("mika", 10).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].text, "Failed message");
    db.delete_failed_send("mika", id).unwrap();
    let pending = db.get_pending_failed_sends("mika", 10).unwrap();
    assert!(pending.is_empty());
}

#[test]
fn test_customer_config() {
    let db = db();
    db.set_customer_config("mika", "timezone", "America/New_York")
        .unwrap();
    let v = db.get_customer_config("mika", "timezone").unwrap().unwrap();
    assert_eq!(v, "America/New_York");
    let all = db.list_customer_config("mika").unwrap();
    assert_eq!(all.len(), 1);
}

#[test]
fn test_team_runs_insert_and_load() {
    let db = db();
    db.insert_team_run(
        "run-001",
        "engineering",
        "Build feature X",
        3,
        "2023-11-14T22:13:20Z",
        None,
    )
    .unwrap();
    db.update_team_run(
        "run-001",
        "completed",
        None,
        1,
        Some("Done!"),
        Some("2023-11-14T22:30:00Z"),
        0,
        false,
        None,
    )
    .unwrap();
    let runs = db.load_team_runs("engineering", 10).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].goal, "Build feature X");
    assert_eq!(runs[0].status, "completed");
}

#[test]
fn test_team_workspace_insert_and_load() {
    let db = db();
    db.insert_team_run("run-001", "eng", "Goal", 3, "2020-01-01T00:00:00Z", None)
        .unwrap();
    let id = db
        .insert_team_workspace_entry("run-001", None, Some("mika"), "plan", "Do this", 1, None)
        .unwrap();
    let entries = db.load_team_workspace("run-001").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, id);
    assert_eq!(entries[0].entry_type, "plan");
}

#[test]
fn test_create_and_get_task() {
    let db = db();
    let task = NewTask {
        agent_id: "mika".to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: "Send reminder".to_string(),
        trigger_type: "time".to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: Some("2286-11-20T17:46:39Z".to_string()),
        timeout_at: None,
        action_type: "send_message".to_string(),
        action_config: r#"{"message":"hello"}"#.to_string(),
        input_context: None,
        created_by_session: None,
        created_trace_id: None,
        reference_url: None,
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: None,
    };
    let id = db.create_task(&task).unwrap();
    let t = db.get_task(&id, "mika").unwrap().unwrap();
    assert_eq!(t.label, "Send reminder");
    assert_eq!(t.trigger_type, "time");
    assert_eq!(t.status, "pending");
}

#[test]
fn test_cancel_task() {
    let db = db();
    let task = NewTask {
        agent_id: "mika".to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: "Cancelable".to_string(),
        trigger_type: "time".to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: None,
        timeout_at: None,
        action_type: "send_message".to_string(),
        action_config: "{}".to_string(),
        input_context: None,
        created_by_session: None,
        created_trace_id: None,
        reference_url: None,
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: None,
    };
    let id = db.create_task(&task).unwrap();
    assert!(db.cancel_task(&id, "mika").unwrap());
    let t = db.get_task(&id, "mika").unwrap().unwrap();
    assert_eq!(t.status, "cancelled");
    // Cancelling again returns false
    assert!(!db.cancel_task(&id, "mika").unwrap());
}

#[test]
fn test_resolve_task_id_by_prefix_unique_match() {
    let db = db();
    let task = NewTask {
        agent_id: "mika".to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: "Prefix test".to_string(),
        trigger_type: "manual".to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: None,
        timeout_at: None,
        action_type: "none".to_string(),
        action_config: "{}".to_string(),
        input_context: None,
        created_by_session: None,
        created_trace_id: None,
        reference_url: None,
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: None,
    };
    let id = db.create_task(&task).unwrap();
    // Use the first 12 chars as prefix (same as tasks list display)
    let prefix = &id[..12];
    let matches = db.resolve_task_id_by_prefix(prefix, "mika").unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0], id);
}

#[test]
fn test_resolve_task_id_by_prefix_exact_match() {
    let db = db();
    let task = NewTask {
        agent_id: "mika".to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: "Full UUID test".to_string(),
        trigger_type: "manual".to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: None,
        timeout_at: None,
        action_type: "none".to_string(),
        action_config: "{}".to_string(),
        input_context: None,
        created_by_session: None,
        created_trace_id: None,
        reference_url: None,
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: None,
    };
    let id = db.create_task(&task).unwrap();
    // Full UUID also works via prefix match
    let matches = db.resolve_task_id_by_prefix(&id, "mika").unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0], id);
}

#[test]
fn test_resolve_task_id_by_prefix_no_match() {
    let db = db();
    let matches = db
        .resolve_task_id_by_prefix("nonexistent-prefix", "mika")
        .unwrap();
    assert!(matches.is_empty());
}

#[test]
fn test_resolve_task_id_by_prefix_scoped_to_agent() {
    let db = db();
    db.register_agent("agent-a", "Agent A", "/tmp/a").unwrap();
    db.register_agent("agent-b", "Agent B", "/tmp/b").unwrap();
    let task = NewTask {
        agent_id: "agent-a".to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: "Agent A task".to_string(),
        trigger_type: "manual".to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: None,
        timeout_at: None,
        action_type: "none".to_string(),
        action_config: "{}".to_string(),
        input_context: None,
        created_by_session: None,
        created_trace_id: None,
        reference_url: None,
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: None,
    };
    let id = db.create_task(&task).unwrap();
    let prefix = &id[..12];
    // Same prefix, different agent — should not match
    let matches = db.resolve_task_id_by_prefix(prefix, "agent-b").unwrap();
    assert!(matches.is_empty());
    // Correct agent — should match
    let matches = db.resolve_task_id_by_prefix(prefix, "agent-a").unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0], id);
}

#[test]
fn test_register_agent() {
    let db = db();
    db.register_agent("agent2", "Secondary Agent", "/home/agent2")
        .unwrap();
    let agents = db.list_agents_db().unwrap();
    assert_eq!(agents.len(), 2);
}

#[test]
fn test_register_agent_upserts_home_dir() {
    let db = db();
    // "mika" was pre-registered with empty home_dir by schema init
    let agents = db.list_agents_db().unwrap();
    let mika = agents.iter().find(|a| a.id == "mika").unwrap();
    assert_eq!(mika.home_dir, "");

    // Re-register with a real path — should update
    db.register_agent("mika", "Mika", "/home/user/.mika/agents/mika")
        .unwrap();
    let agents = db.list_agents_db().unwrap();
    let mika = agents.iter().find(|a| a.id == "mika").unwrap();
    assert_eq!(mika.home_dir, "/home/user/.mika/agents/mika");

    // Re-register with empty string — should NOT overwrite home_dir
    db.register_agent("mika", "Mika", "").unwrap();
    let agents = db.list_agents_db().unwrap();
    let mika = agents.iter().find(|a| a.id == "mika").unwrap();
    assert_eq!(mika.home_dir, "/home/user/.mika/agents/mika");
}

#[test]
fn test_register_agent_upserts_name() {
    let db = db();
    // "mika" was pre-registered by schema init with name = "Mika"
    let name = db.get_agent_display_name("mika");
    assert_eq!(name, "Mika");

    // Re-register with a proper display name — should update
    db.register_agent("mika", "Mika ✨", "/home/user/.mika/agents/mika")
        .unwrap();
    let name = db.get_agent_display_name("mika");
    assert_eq!(name, "Mika ✨");

    // Re-register with a different name — should update again
    db.register_agent("mika", "My Assistant", "").unwrap();
    let name = db.get_agent_display_name("mika");
    assert_eq!(name, "My Assistant");
}

#[test]
fn test_register_team() {
    let db = db();
    db.register_team("eng", "Engineering", "/teams/eng.toml")
        .unwrap();
    let teams = db.list_teams_db().unwrap();
    assert_eq!(teams.len(), 1);
    assert_eq!(teams[0].id, "eng");
}

#[test]
fn test_fts_search_agent_isolation() {
    let db = db();
    db.register_agent("other", "Other", "").unwrap();
    db.index_content("mika", "person", Some(1), "Alice in Wonderland")
        .unwrap();
    db.index_content("other", "person", Some(1), "Bob the Builder")
        .unwrap();
    let results = db.fts_search("mika", "Alice", 10, None).unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].content.contains("Alice"));
}

#[test]
fn test_get_unembedded_content() {
    let db = db();
    // Index two content rows — both start with embedding_json = NULL
    let id1 = db
        .index_content("mika", "person", Some(1), "Alice in Wonderland")
        .unwrap();
    let id2 = db
        .index_content("mika", "person", Some(2), "Bob the Builder")
        .unwrap();

    // Both should be returned as unembedded
    let unembedded = db.get_unembedded_content("mika").unwrap();
    assert_eq!(unembedded.len(), 2);

    // Store an embedding for the first one
    db.index_embedding(id1, &[0.1; 512]).unwrap();

    // Now only the second should be unembedded
    let unembedded = db.get_unembedded_content("mika").unwrap();
    assert_eq!(unembedded.len(), 1);
    assert_eq!(unembedded[0].0, id2);
    assert_eq!(unembedded[0].1, "Bob the Builder");

    // Store embedding for the second — none left
    db.index_embedding(id2, &[0.2; 512]).unwrap();
    let unembedded = db.get_unembedded_content("mika").unwrap();
    assert!(unembedded.is_empty());
}

#[test]
fn test_get_unembedded_content_agent_isolation() {
    let db = db();
    db.register_agent("other", "Other", "").unwrap();
    db.index_content("mika", "person", Some(1), "Alice")
        .unwrap();
    db.index_content("other", "person", Some(1), "Bob").unwrap();

    // Each agent sees only its own unembedded content
    let mika_unembedded = db.get_unembedded_content("mika").unwrap();
    assert_eq!(mika_unembedded.len(), 1);
    assert_eq!(mika_unembedded[0].1, "Alice");

    let other_unembedded = db.get_unembedded_content("other").unwrap();
    assert_eq!(other_unembedded.len(), 1);
    assert_eq!(other_unembedded[0].1, "Bob");
}

#[test]
fn test_get_all_facts_for_indexing() {
    let db = db();
    db.upsert_person("mika", "Alice", Some("friend"), None)
        .unwrap();
    db.add_commitment("mika", "Write docs", None, None).unwrap();
    db.set_preference("mika", "theme", "dark").unwrap();
    let facts = db.get_all_facts_for_indexing("mika").unwrap();
    assert_eq!(facts.len(), 3);
    let types: Vec<&str> = facts.iter().map(|(t, _, _)| t.as_str()).collect();
    assert!(types.contains(&"person"));
    assert!(types.contains(&"commitment"));
    assert!(types.contains(&"preference"));
}

#[test]
fn test_load_messages_before_window() {
    let (db, sid) = db_with_session();
    for i in 0..5 {
        db.save_message("mika", &sid, "user", &format!("msg {i}"), None)
            .unwrap();
    }
    let before = db.load_messages_before_window("mika", 2).unwrap();
    // Window is last 2, so before window is first 3
    assert_eq!(before.len(), 3);
    assert_eq!(before[0].content, "msg 0");
}

#[test]
fn test_count_pending_tasks() {
    let db = db();
    let task = NewTask {
        agent_id: "mika".to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: "T1".to_string(),
        trigger_type: "time".to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: None,
        timeout_at: None,
        action_type: "send_message".to_string(),
        action_config: "{}".to_string(),
        input_context: None,
        created_by_session: None,
        created_trace_id: None,
        reference_url: None,
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: None,
    };
    db.create_task(&task).unwrap();
    assert_eq!(db.count_pending_tasks("mika").unwrap(), 1);
}

#[test]
fn test_sibling_completion_all_done_fires_parent() {
    let db = db();
    let parent_id = db.create_task(&make_task("parent")).unwrap();

    let mut c1 = make_task("child1");
    c1.parent_task_id = Some(parent_id.clone());
    c1.depth = 1;
    let c1_id = db.create_task(&c1).unwrap();

    let mut c2 = make_task("child2");
    c2.parent_task_id = Some(parent_id.clone());
    c2.depth = 1;
    let c2_id = db.create_task(&c2).unwrap();

    let mut c3 = make_task("child3");
    c3.parent_task_id = Some(parent_id.clone());
    c3.depth = 1;
    let c3_id = db.create_task(&c3).unwrap();

    // Complete 2 of 3 — parent should NOT fire
    db.update_task_completed(&c1_id, "mika", Some("done"))
        .unwrap();
    db.update_task_completed(&c2_id, "mika", Some("done"))
        .unwrap();
    assert_eq!(
        db.try_complete_parent_on_sibling_done(&c2_id).unwrap(),
        None
    );

    // Complete the 3rd — parent should fire
    db.update_task_completed(&c3_id, "mika", Some("done"))
        .unwrap();
    let result = db.try_complete_parent_on_sibling_done(&c3_id).unwrap();
    assert_eq!(result, Some(parent_id));
}

#[test]
fn test_sibling_completion_no_parent_returns_none() {
    let db = db();
    let task_id = db.create_task(&make_task("orphan")).unwrap();
    assert_eq!(
        db.try_complete_parent_on_sibling_done(&task_id).unwrap(),
        None
    );
}

#[test]
fn test_sibling_completion_failed_child_counts_as_done() {
    let db = db();
    let parent_id = db.create_task(&make_task("parent")).unwrap();

    let mut c1 = make_task("child1");
    c1.parent_task_id = Some(parent_id.clone());
    let c1_id = db.create_task(&c1).unwrap();

    let mut c2 = make_task("child2");
    c2.parent_task_id = Some(parent_id.clone());
    let c2_id = db.create_task(&c2).unwrap();

    // One completed, one failed — both are "done"
    db.update_task_completed(&c1_id, "mika", Some("ok"))
        .unwrap();
    db.update_task_failed(&c2_id, "mika", "error").unwrap();
    let result = db.try_complete_parent_on_sibling_done(&c2_id).unwrap();
    assert_eq!(result, Some(parent_id));
}

#[test]
fn test_get_child_tasks() {
    let db = db();
    let parent_id = db.create_task(&make_task("parent")).unwrap();

    let mut c1 = make_task("child1");
    c1.parent_task_id = Some(parent_id.clone());
    db.create_task(&c1).unwrap();

    let mut c2 = make_task("child2");
    c2.parent_task_id = Some(parent_id.clone());
    db.create_task(&c2).unwrap();

    let children = db.get_child_tasks(&parent_id).unwrap();
    assert_eq!(children.len(), 2);
}

#[test]
fn test_get_task_descendants_three_levels() {
    let db = db();
    let root_id = db.create_task(&make_task("root")).unwrap();

    let mut child = make_task("child");
    child.parent_task_id = Some(root_id.clone());
    child.depth = 1;
    let child_id = db.create_task(&child).unwrap();

    let mut grandchild = make_task("grandchild");
    grandchild.parent_task_id = Some(child_id.clone());
    grandchild.depth = 2;
    let grandchild_id = db.create_task(&grandchild).unwrap();

    let mut great_grandchild = make_task("great-grandchild");
    great_grandchild.parent_task_id = Some(grandchild_id.clone());
    great_grandchild.depth = 3;
    db.create_task(&great_grandchild).unwrap();

    let descendants = db.get_task_descendants(&root_id).unwrap();
    assert_eq!(descendants.len(), 3);
    // Verify all descendants are present (ordering depends on created_at which may be identical)
    let labels: Vec<&str> = descendants.iter().map(|t| t.label.as_str()).collect();
    assert!(labels.contains(&"child"));
    assert!(labels.contains(&"grandchild"));
    assert!(labels.contains(&"great-grandchild"));
}

#[test]
fn test_get_task_descendants_excludes_root() {
    let db = db();
    let root_id = db.create_task(&make_task("root")).unwrap();

    let mut child = make_task("child");
    child.parent_task_id = Some(root_id.clone());
    child.depth = 1;
    db.create_task(&child).unwrap();

    let descendants = db.get_task_descendants(&root_id).unwrap();
    assert_eq!(descendants.len(), 1);
    assert!(descendants.iter().all(|t| t.id != root_id));
}

#[test]
fn test_get_task_descendants_no_children() {
    let db = db();
    let root_id = db.create_task(&make_task("root")).unwrap();

    let descendants = db.get_task_descendants(&root_id).unwrap();
    assert!(descendants.is_empty());
}

#[test]
fn test_get_task_descendants_multi_branch() {
    let db = db();
    let root_id = db.create_task(&make_task("root")).unwrap();

    // 3 children, each with 2 grandchildren = 9 descendants total
    for i in 0..3 {
        let mut child = make_task(&format!("child-{i}"));
        child.parent_task_id = Some(root_id.clone());
        child.depth = 1;
        let child_id = db.create_task(&child).unwrap();

        for j in 0..2 {
            let mut gc = make_task(&format!("gc-{i}-{j}"));
            gc.parent_task_id = Some(child_id.clone());
            gc.depth = 2;
            db.create_task(&gc).unwrap();
        }
    }

    let descendants = db.get_task_descendants(&root_id).unwrap();
    assert_eq!(descendants.len(), 9);
}

#[test]
fn test_get_task_descendants_cross_agent() {
    let db = Database::open_in_memory().unwrap();
    db.register_agent("mika", "Mika", "/tmp").unwrap();
    db.register_agent("other-agent", "Other", "/tmp").unwrap();

    let root_id = db.create_task(&make_task("root")).unwrap();

    let mut child = make_task("child");
    child.parent_task_id = Some(root_id.clone());
    child.depth = 1;
    child.agent_id = "other-agent".to_string();
    db.create_task(&child).unwrap();

    let descendants = db.get_task_descendants(&root_id).unwrap();
    assert_eq!(descendants.len(), 1);
    assert_eq!(descendants[0].agent_id, "other-agent");
}

#[test]
fn test_count_pending_callback_tasks_by_team_run() {
    let db = Database::open_in_memory().unwrap();
    db.register_agent("mika", "Mika", "/tmp").unwrap();

    // Insert team + team runs so FK constraints are satisfied
    db.conn
        .execute(
            "INSERT OR IGNORE INTO teams (id, name) VALUES ('team1', 'team1')",
            [],
        )
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO team_runs (id, team_id, goal, started_at)
                 VALUES ('run123', 'team1', 'test goal', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
            [],
        )
        .unwrap();

    // Create a pending callback task with matching team_run_id and depth > 1
    let mut t = make_task("grandchild");
    t.trigger_type = "callback".to_string();
    t.depth = 2;
    t.team_run_id = Some("run123".to_string());
    db.create_task(&t).unwrap();

    // Create a completed callback task (should NOT be counted)
    let mut t2 = make_task("done-grandchild");
    t2.trigger_type = "callback".to_string();
    t2.depth = 2;
    t2.team_run_id = Some("run123".to_string());
    let t2_id = db.create_task(&t2).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'completed' WHERE id = ?1",
            params![t2_id],
        )
        .unwrap();

    // Create a depth=1 callback (should NOT be counted -- only depth > 1)
    let mut t3 = make_task("child-not-grandchild");
    t3.trigger_type = "callback".to_string();
    t3.depth = 1;
    t3.team_run_id = Some("run123".to_string());
    db.create_task(&t3).unwrap();

    // Create a pending callback for a DIFFERENT team run (should NOT be counted)
    db.conn
        .execute(
            "INSERT INTO team_runs (id, team_id, goal, started_at)
                 VALUES ('other-run', 'team1', 'other goal', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
            [],
        )
        .unwrap();
    let mut t4 = make_task("other-run-grandchild");
    t4.trigger_type = "callback".to_string();
    t4.depth = 2;
    t4.team_run_id = Some("other-run".to_string());
    db.create_task(&t4).unwrap();

    let count = db
        .count_pending_callback_tasks_by_team_run("run123")
        .unwrap();
    assert_eq!(count, 1); // only the pending depth=2 grandchild for run123
}

#[test]
fn test_get_expired_child_task_ids() {
    let db = Database::open_in_memory().unwrap();
    db.register_agent("mika", "Mika", "/tmp").unwrap();

    // Parent still pending — its expired child SHOULD appear
    let parent_id = db.create_task(&make_task("parent")).unwrap();

    let mut c = make_task("expired-child");
    c.parent_task_id = Some(parent_id.clone());
    let c_id = db.create_task(&c).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'expired' WHERE id = ?1",
            params![c_id],
        )
        .unwrap();

    // Expired task without parent (should NOT appear)
    let o_id = db.create_task(&make_task("expired-orphan")).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'expired' WHERE id = ?1",
            params![o_id],
        )
        .unwrap();

    // Parent already completed — its expired child should NOT appear
    let done_parent_id = db.create_task(&make_task("done-parent")).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'completed' WHERE id = ?1",
            params![done_parent_id],
        )
        .unwrap();

    let mut c2 = make_task("expired-child-done-parent");
    c2.parent_task_id = Some(done_parent_id.clone());
    let c2_id = db.create_task(&c2).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'expired' WHERE id = ?1",
            params![c2_id],
        )
        .unwrap();

    let ids = db.get_expired_child_task_ids("mika").unwrap();
    assert_eq!(
        ids.len(),
        1,
        "should only return expired children whose parent is still pending"
    );
    assert_eq!(ids[0], c_id);
}

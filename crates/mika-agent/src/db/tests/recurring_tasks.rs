//! Tests de `crate::db` — thème `recurring_tasks` (mika#2321).
//!
//! Le veto anti-zombie des récurrences (mika#1742 Problem B), sa levée pour la
//! classe « trigger inconnu » (mika#2337) et le registre en lecture seule
//! (mika#2360).
//!
//! Enfant de `db::tests`, donc descendant de `db` : les items privés de `db` et
//! les helpers de `db::tests` restent visibles via `use super::*`.

use super::*;

/// Fresh install (no prior rows) → registration succeeds.
#[test]
fn zombie_guard_fresh_install_registers() {
    let db = db();
    let id = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
        .unwrap();
    assert!(id.is_some());
}

/// Existing `recurring_active` row → returns `None` (unchanged
/// idempotency, driven by the partial unique index).
#[test]
fn zombie_guard_active_row_still_idempotent() {
    let db = db();
    db.create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
        .unwrap();
    let second = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
        .unwrap();
    assert!(
        second.is_none(),
        "second call must NOT create a duplicate active row"
    );
}

/// Recent `failed` row within the grace window → refuse to re-register.
/// This IS the mika#1742 root-cause fix.
#[test]
fn zombie_guard_recent_failed_refuses_registration() {
    let db = db();
    let first = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
        .unwrap()
        .unwrap();
    // Mark it failed 1 hour ago.
    db.conn
        .execute(
            "UPDATE tasks SET status = 'failed',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hour')
                 WHERE id = ?1",
            params![first],
        )
        .unwrap();

    let retry = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
        .unwrap();
    assert!(
        retry.is_none(),
        "recent-failed sibling must block fresh registration (mika#1742)"
    );
}

/// mika#2271 — un cancel de config *reverté* n'est pas une mort : la row
/// porte le marqueur, la garde la saute, la ré-inscription passe.
#[test]
fn zombie_guard_reverted_config_cancel_allows_registration() {
    let db = db();
    let first = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "auto_pull_groomed"))
        .unwrap()
        .unwrap();
    db.cancel_recurring_task_by_label("mika", "auto_pull_groomed")
        .unwrap();

    let marked = db
        .revert_config_cancel_recurring_task("mika", "auto_pull_groomed")
        .unwrap();
    assert_eq!(marked, 1, "la row cancelled doit être marquée");

    let retry = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "auto_pull_groomed"))
        .unwrap();
    assert!(
        retry.is_some(),
        "un cancel de config reverté ne doit plus bloquer (mika#2271)"
    );
    assert_ne!(retry.unwrap(), first, "une row fraîche doit être créée");
}

/// L'exemption ne s'étend pas aux morts accidentelles : `revert` ne touche
/// que `cancelled`, et un `failed` récent continue de bloquer (mika#1742).
#[test]
fn revert_config_cancel_leaves_failed_rows_blocking() {
    let db = db();
    let first = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "auto_pull_groomed"))
        .unwrap()
        .unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'failed',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hour')
                 WHERE id = ?1",
            params![first],
        )
        .unwrap();

    let marked = db
        .revert_config_cancel_recurring_task("mika", "auto_pull_groomed")
        .unwrap();
    assert_eq!(marked, 0, "revert ne doit marquer aucune row `failed`");

    let retry = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "auto_pull_groomed"))
        .unwrap();
    assert!(
        retry.is_none(),
        "un échec terminal récent doit toujours bloquer (mika#1742)"
    );
}

/// Le marqueur porte l'exemption ; il ne falsifie pas l'horodatage de
/// l'annulation réelle (`updated_at` intact — piste d'audit préservée).
#[test]
fn revert_config_cancel_preserves_updated_at() {
    let db = db();
    db.create_recurring_task_if_absent(zombie_recurring_task("mika", "auto_pull_groomed"))
        .unwrap();
    db.cancel_recurring_task_by_label("mika", "auto_pull_groomed")
        .unwrap();

    let before: String = db
        .conn
        .query_row(
            "SELECT updated_at FROM tasks WHERE label = 'auto_pull_groomed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.revert_config_cancel_recurring_task("mika", "auto_pull_groomed")
        .unwrap();
    let after: String = db
        .conn
        .query_row(
            "SELECT updated_at FROM tasks WHERE label = 'auto_pull_groomed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(before, after, "updated_at doit rester celui du cancel réel");
}

/// Idempotence : un second boot ne re-marque pas une row déjà exemptée.
#[test]
fn revert_config_cancel_is_idempotent() {
    let db = db();
    db.create_recurring_task_if_absent(zombie_recurring_task("mika", "auto_pull_groomed"))
        .unwrap();
    db.cancel_recurring_task_by_label("mika", "auto_pull_groomed")
        .unwrap();
    assert_eq!(
        db.revert_config_cancel_recurring_task("mika", "auto_pull_groomed")
            .unwrap(),
        1
    );
    assert_eq!(
        db.revert_config_cancel_recurring_task("mika", "auto_pull_groomed")
            .unwrap(),
        0,
        "une row déjà marquée ne doit pas être ré-écrite"
    );
}

/// Recent `cancelled` row within the grace window → refuse.
#[test]
fn zombie_guard_recent_cancelled_refuses_registration() {
    let db = db();
    let first = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
        .unwrap()
        .unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'cancelled',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-2 hours')
                 WHERE id = ?1",
            params![first],
        )
        .unwrap();
    let retry = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
        .unwrap();
    assert!(retry.is_none());
}

/// Old dead row (outside the grace window) → allow fresh registration.
/// The grace has elapsed; operator has had time to notice / act.
#[test]
fn zombie_guard_expired_grace_allows_registration() {
    let db = db();
    let first = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
        .unwrap()
        .unwrap();
    // Push the failure well outside the 24h window.
    db.conn
        .execute(
            "UPDATE tasks SET status = 'failed',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-72 hours')
                 WHERE id = ?1",
            params![first],
        )
        .unwrap();
    let retry = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
        .unwrap();
    assert!(
        retry.is_some(),
        "outside grace window must allow re-registration"
    );
}

/// Dead row for a DIFFERENT label must not block registration.
#[test]
fn zombie_guard_scoped_to_label() {
    let db = db();
    let first = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "other_label"))
        .unwrap()
        .unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'failed',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hour')
                 WHERE id = ?1",
            params![first],
        )
        .unwrap();
    let curator = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
        .unwrap();
    assert!(curator.is_some(), "guard must scope to (agent_id, label)");
}

/// Dead row for a DIFFERENT agent must not block registration.
#[test]
fn zombie_guard_scoped_to_agent() {
    let db = db();
    // Test fixture's migrate_v1 pre-registers 'mika' only; register the
    // second agent explicitly so the tasks FK holds.
    db.register_agent("mika-arch", "Mika Arch", "/tmp/mika-arch")
        .unwrap();
    let mika_first = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
        .unwrap()
        .unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'failed',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hour')
                 WHERE id = ?1",
            params![mika_first],
        )
        .unwrap();
    // Different agent — must succeed.
    let arch = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika-arch", "curator_review"))
        .unwrap();
    assert!(arch.is_some(), "guard must scope to agent_id");
}

/// AC4 — une mort par trigger inconnu **survenue sous le binaire porteur du
/// marquage** n'arme pas le veto : le redémarrage suivant ré-inscrit sans
/// attendre les 24 h.
///
/// C'est le cœur du hotfix mika#2337 : la ligne `failed` de `fb425f89` a
/// gelé la réconciliation de revue (#2334) pendant toute la fenêtre, et
/// redéployer — le remède naturel — était précisément ce que cet état
/// neutralisait.
#[test]
fn mika2337_unknown_trigger_death_does_not_arm_the_veto() {
    let db = db();
    kill_recurring(&db, "qa_review_reconcile", "-1 hour", true);

    let retry = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "qa_review_reconcile"))
        .unwrap();
    assert!(
        retry.is_some(),
        "une mort marquée « trigger inconnu » ne doit pas empêcher la \
             ré-inscription : le binaire qui écrit le nom porte le bras qui le route"
    );
}

/// AC5 — le contrôle négatif d'AC4, et la seule chose qui l'empêche de
/// signifier « mika#1742 est désarmée ». Même scénario, même fenêtre, même
/// label : seule la cause de la mort change.
#[test]
fn mika2337_any_other_death_still_arms_the_veto() {
    let db = db();
    kill_recurring(&db, "qa_review_reconcile", "-1 hour", false);

    let retry = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "qa_review_reconcile"))
        .unwrap();
    assert!(
        retry.is_none(),
        "une mort de toute autre cause doit continuer d'armer le veto \
             mika#1742 — la levée est une exception nommée, pas un désarmement"
    );
}

/// AC6 — la levée est à usage unique. Une **seconde** mort marquée sur le
/// même label dans la fenêtre retrouve un veto armé.
///
/// C'est l'assertion auto-nettoyante au sens du gate : elle rougit le jour
/// où la levée cesserait d'être bornée, c'est-à-dire le jour où une boucle
/// réelle — un binaire qui ré-enregistre en continu un trigger qu'il ne
/// sait pas router — pourrait s'auto-absoudre indéfiniment.
#[test]
fn mika2337_the_lift_is_single_use_within_the_window() {
    let db = db();
    kill_recurring(&db, "qa_review_reconcile", "-2 hours", true);

    let first_retry = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "qa_review_reconcile"))
        .unwrap();
    assert!(first_retry.is_some(), "la première levée doit passer (AC4)");

    // La ligne ré-inscrite meurt à son tour, même cause, même fenêtre.
    let second_id = first_retry.unwrap();
    assert_eq!(
        db.mark_recurring_unknown_trigger(&second_id).unwrap(),
        1,
        "la seconde mort porte le marqueur, comme la première"
    );
    db.conn
        .execute(
            "UPDATE tasks SET status = 'failed',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hour')
                 WHERE id = ?1",
            params![second_id],
        )
        .unwrap();

    let second_retry = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "qa_review_reconcile"))
        .unwrap();
    assert!(
        second_retry.is_none(),
        "la levée achète UN redémarrage, pas une immunité : la seconde mort \
             marquée dans la fenêtre doit retrouver le veto armé"
    );
}

/// Le marqueur de levée consommée sort de la fenêtre avec la ligne qui le
/// porte : passé la grâce, l'exemption se ré-arme. Sans cela, un `(agent,
/// label)` ayant consommé sa levée une fois la perdrait pour toujours —
/// une dispense négative permanente, l'exact miroir du défaut que la levée
/// corrige.
#[test]
fn mika2337_the_lift_rearms_once_the_grace_window_has_elapsed() {
    let db = db();
    kill_recurring(&db, "qa_review_reconcile", "-2 hours", true);
    let reregistered = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "qa_review_reconcile"))
        .unwrap()
        .expect("première levée");

    // La ligne consommée ET la nouvelle mort sortent toutes deux de la
    // fenêtre de 24 h.
    db.conn
        .execute(
            "UPDATE tasks SET updated_at =
                 strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-72 hours')
                 WHERE agent_id = 'mika' AND label = 'qa_review_reconcile'",
            [],
        )
        .unwrap();
    db.mark_recurring_unknown_trigger(&reregistered).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'failed',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hour')
                 WHERE id = ?1",
            params![reregistered],
        )
        .unwrap();

    let retry = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "qa_review_reconcile"))
        .unwrap();
    assert!(
        retry.is_some(),
        "une levée consommée hors fenêtre ne doit plus désarmer l'exemption"
    );
}

/// R2 — `trigger_type = 'recurring'` est un littéral : `manual`,
/// `callback` et `time` ne sortent jamais, quel que soit le filtre.
#[test]
fn mika2360_registry_excludes_non_recurring_rows() {
    let db = db();
    db.create_task(&make_task("callback-row")).unwrap();
    let mut manual = make_task("manual-row");
    manual.trigger_type = "manual".to_string();
    db.create_task(&manual).unwrap();
    let mut time = make_task("time-row");
    time.trigger_type = "time".to_string();
    time.next_fire_at = Some("2286-11-20T17:46:39Z".to_string());
    db.create_task(&time).unwrap();
    db.create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
        .unwrap()
        .unwrap();

    let (rows, total) = db.list_recurring_registry(None, 200, 0).unwrap();
    assert_eq!(total, 1);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].label, "curator_review");
    assert_eq!(rows[0].trigger_type, "recurring");
    assert_eq!(rows[0].status, "recurring_active");
    assert!(!rows[0].zombie_veto_active);
}

/// AC4 — testée sur le JSON rendu, pas sur les champs de la struct : un
/// `#[serde(flatten)]` ajouté plus tard passerait un test sur les champs.
#[test]
fn mika2360_registry_projection_carries_no_message_content() {
    let db = db();
    registry_row(
        &db,
        "rappel-medicaments",
        "recurring_active",
        "2026-09-01T08:00:00Z",
    );

    let (rows, _) = db.list_recurring_registry(None, 200, 0).unwrap();
    let json = serde_json::to_string(&rows).unwrap();
    assert!(
        !json.contains("SECRET-MEDICATION-REMINDER"),
        "action_config leaked into the registry projection: {json}"
    );
    assert!(
        !json.contains("SECRET-METADATA-BLOB"),
        "metadata leaked into the registry projection: {json}"
    );
    assert!(!json.contains("action_config"));
    assert!(!json.contains("\"metadata\""));
    assert!(!json.contains("\"result\""));
    assert!(!json.contains("input_context"));
    assert!(json.contains("\"label\":\"rappel-medicaments\""));
    assert!(json.contains("\"zombie_veto_active\""));
}

/// T2 — aucun filtre de statut : la ligne morte qui bloque est visible.
#[test]
fn mika2360_registry_lists_cancelled_and_failed_rows() {
    let db = db();
    registry_row(&db, "a-cancelled", "cancelled", "2026-09-01T08:00:00Z");
    registry_row(&db, "b-failed", "failed", "2026-09-01T08:00:00Z");
    registry_row(&db, "c-expired", "expired", "2026-09-01T08:00:00Z");
    registry_row(&db, "d-active", "recurring_active", "2026-09-01T08:00:00Z");

    let (rows, total) = db.list_recurring_registry(None, 200, 0).unwrap();
    assert_eq!(total, 4);
    let statuses: Vec<&str> = rows.iter().map(|r| r.status.as_str()).collect();
    assert_eq!(
        statuses,
        vec!["cancelled", "failed", "expired", "recurring_active"]
    );
}

/// T4 — les doublons d'un label sont contigus et en ordre de naissance,
/// quel que soit `updated_at`.
#[test]
fn mika2360_registry_orders_by_label_then_created_at() {
    let db = db();
    let newer = registry_row(&db, "rappel", "cancelled", "2026-09-02T08:00:00Z");
    registry_row(&db, "aaa-autre", "recurring_active", "2026-09-01T12:00:00Z");
    let older = registry_row(&db, "rappel", "recurring_active", "2026-09-01T08:00:00Z");
    // updated_at in the *opposite* order of created_at: the sort must
    // not follow it.
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = '2026-09-10T00:00:00Z' WHERE id = ?1",
            params![older],
        )
        .unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = '2026-09-03T00:00:00Z' WHERE id = ?1",
            params![newer],
        )
        .unwrap();

    let (rows, _) = db.list_recurring_registry(None, 200, 0).unwrap();
    let seq: Vec<(&str, &str)> = rows
        .iter()
        .map(|r| (r.label.as_str(), r.created_at.as_str()))
        .collect();
    assert_eq!(
        seq,
        vec![
            ("aaa-autre", "2026-09-01T12:00:00Z"),
            ("rappel", "2026-09-01T08:00:00Z"),
            ("rappel", "2026-09-02T08:00:00Z"),
        ]
    );
}

/// T6 b — `Rappel` et `rappel` sont le même label pour le veto ; le tri
/// doit les rapprocher, pas les séparer par la ligne intermédiaire.
#[test]
fn mika2360_registry_orders_labels_case_insensitively() {
    let db = db();
    registry_row(&db, "Rappel", "cancelled", "2026-09-01T08:00:00Z");
    registry_row(
        &db,
        "aaa-autre-label",
        "recurring_active",
        "2026-09-01T09:00:00Z",
    );
    registry_row(&db, "rappel", "recurring_active", "2026-09-01T10:00:00Z");

    assert_eq!(
        registry_labels(&db),
        vec!["aaa-autre-label", "Rappel", "rappel"],
        "a binary sort would put `Rappel` before `aaa-…` and split the pair"
    );
}

/// `?agent_id=` restreint le registre à un agent.
#[test]
fn mika2360_registry_filters_by_agent_id() {
    let db = db();
    db.register_agent("other", "Other", "/tmp/other").unwrap();
    registry_row(&db, "mika-only", "recurring_active", "2026-09-01T08:00:00Z");
    let mut t = zombie_recurring_task("other", "other-only");
    db.create_task(&t).unwrap();
    t.label = "other-second".to_string();
    db.create_task(&t).unwrap();

    let (all, total_all) = db.list_recurring_registry(None, 200, 0).unwrap();
    assert_eq!(total_all, 3);
    assert_eq!(all.len(), 3);

    let (mika, total_mika) = db.list_recurring_registry(Some("mika"), 200, 0).unwrap();
    assert_eq!(total_mika, 1);
    assert_eq!(mika[0].agent_id, "mika");

    let (other, total_other) = db.list_recurring_registry(Some("other"), 200, 0).unwrap();
    assert_eq!(total_other, 2);
    assert!(other.iter().all(|r| r.agent_id == "other"));

    // Pagination: total counts the whole population, data is the page.
    let (page2, total_p) = db.list_recurring_registry(None, 2, 2).unwrap();
    assert_eq!(total_p, 3);
    assert_eq!(page2.len(), 1);
}

/// Test d'accord (§3.1 option b) — sur chaque état du veto,
/// `zombie_veto_active` concorde avec le refus réel de
/// `create_recurring_task_if_absent`. Chaque cas vit dans sa propre base :
/// un seul label mort par base, donc « une ligne arme le veto » et « la
/// ré-inscription est refusée » sont la même proposition.
#[test]
fn mika2360_zombie_veto_flag_matches_registration_refusal() {
    // (nom du cas, préparation, veto attendu)
    type Prepare = Box<dyn Fn(&Database)>;
    let cases: Vec<(&str, Prepare, bool)> = vec![
        (
            "recent cancelled",
            Box::new(|db| {
                let id = db
                    .create_recurring_task_if_absent(zombie_recurring_task("mika", "lbl"))
                    .unwrap()
                    .unwrap();
                db.conn
                    .execute(
                        "UPDATE tasks SET status = 'cancelled',
                             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-2 hours')
                             WHERE id = ?1",
                        params![id],
                    )
                    .unwrap();
            }),
            true,
        ),
        (
            "cancelled + config-cancel reverted (mika#2271)",
            Box::new(|db| {
                db.create_recurring_task_if_absent(zombie_recurring_task("mika", "lbl"))
                    .unwrap()
                    .unwrap();
                db.cancel_recurring_task_by_label("mika", "lbl").unwrap();
                assert_eq!(
                    db.revert_config_cancel_recurring_task("mika", "lbl")
                        .unwrap(),
                    1
                );
            }),
            false,
        ),
        (
            "failed outside the grace window",
            Box::new(|db| {
                kill_recurring(db, "lbl", "-72 hours", false);
            }),
            false,
        ),
        (
            "failed inside the window, any other cause",
            Box::new(|db| {
                kill_recurring(db, "lbl", "-1 hour", false);
            }),
            true,
        ),
        (
            "unknown-trigger death, lift not yet spent (mika#2337)",
            Box::new(|db| {
                kill_recurring(db, "lbl", "-1 hour", true);
            }),
            false,
        ),
        (
            "failed inside the window, lifted by the operator (mika#2446)",
            Box::new(|db| {
                kill_recurring(db, "lbl", "-1 hour", false);
                assert_eq!(db.mark_recurring_operator_rearm("mika", "lbl").unwrap(), 1);
            }),
            false,
        ),
    ];

    for (name, prepare, expected_veto) in cases {
        let db = db();
        prepare(&db);

        let (rows, _) = db.list_recurring_registry(None, 200, 0).unwrap();
        assert_eq!(rows.len(), 1, "{name}: one row expected");
        assert_eq!(
            rows[0].zombie_veto_active, expected_veto,
            "{name}: zombie_veto_active"
        );

        let registered = db
            .create_recurring_task_if_absent(zombie_recurring_task("mika", "lbl"))
            .unwrap();
        assert_eq!(
            registered.is_none(),
            expected_veto,
            "{name}: the flag must agree with the real guard"
        );
    }
}

/// T6 a — le lift est une propriété de `(agent_id, label)`, pas de la
/// ligne. Deux lignes du même label (casse mêlée) : l'une porte
/// `lift_consumed`, l'autre `unknown_trigger_death`. Une lecture par ligne
/// verrait la seconde exemptée ; la sous-requête corrélée rend le même
/// verdict que la garde : veto armé.
#[test]
fn mika2360_zombie_veto_flag_reads_lift_spent_on_a_sibling_row() {
    let db = db();
    let first = kill_recurring(&db, "Qa_Review_Reconcile", "-2 hours", true);
    // The lift is spent on `first` by this re-registration (same label,
    // different case — the guard compares NOCASE, so must the flag).
    let second = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "qa_review_reconcile"))
        .unwrap()
        .expect("first lift must pass");
    assert_eq!(db.mark_recurring_unknown_trigger(&second).unwrap(), 1);
    db.conn
        .execute(
            "UPDATE tasks SET status = 'failed',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hour')
                 WHERE id = ?1",
            params![second],
        )
        .unwrap();

    let (rows, _) = db.list_recurring_registry(None, 200, 0).unwrap();
    assert_eq!(rows.len(), 2);
    // Sorted NOCASE then created_at: `first` (older) before `second`.
    assert_eq!(rows[0].label, "Qa_Review_Reconcile");
    assert_eq!(rows[1].label, "qa_review_reconcile");
    assert!(
        rows[0].zombie_veto_active,
        "the row whose lift was spent is a plain recent death now"
    );
    assert!(
        rows[1].zombie_veto_active,
        "a per-row read would exempt this row: its own metadata carries \
             the marker and no lift_consumed — the sibling carries that"
    );

    let retry = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "qa_review_reconcile"))
        .unwrap();
    assert!(retry.is_none(), "the guard must agree: veto armed");
    let _ = first;
}

/// AC3 — lire le registre n'écrit rien : même nombre de lignes, mêmes
/// `status` / `updated_at` / `metadata` avant et après.
#[test]
fn mika2360_registry_is_read_only() {
    let db = db();
    kill_recurring(&db, "dead", "-1 hour", true);
    registry_row(&db, "alive", "recurring_active", "2026-09-01T08:00:00Z");
    db.create_task(&make_task("callback-row")).unwrap();

    let snapshot = |db: &Database| -> Vec<(String, String, String, Option<String>)> {
        let mut stmt = db
            .conn
            .prepare("SELECT id, status, updated_at, metadata FROM tasks ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };

    let before = snapshot(&db);
    assert_eq!(before.len(), 3);
    for _ in 0..3 {
        db.list_recurring_registry(None, 200, 0).unwrap();
        db.list_recurring_registry(Some("mika"), 1, 0).unwrap();
    }
    assert_eq!(snapshot(&db), before, "a read must leave `tasks` untouched");
}

/// Le marqueur est écrit en **entier**, pas en texte. SQLite ordonne
/// INTEGER avant TEXT, donc `'1' = 1` est faux : un marqueur textuel serait
/// visible dans la ligne et invisible à la garde — la pire des formes,
/// puisque l'inspection manuelle le confirmerait tout en le laissant inerte.
#[test]
fn mika2337_the_marker_is_written_as_an_integer() {
    let db = db();
    let id = kill_recurring(&db, "qa_review_reconcile", "-1 hour", true);
    let typ: String = db
        .conn
        .query_row(
            "SELECT json_type(metadata, ?2) FROM tasks WHERE id = ?1",
            params![id, RECURRING_UNKNOWN_TRIGGER_PATH],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        typ, "integer",
        "le marqueur doit être l'entier 1 — la garde compare avec `= 1`"
    );
}

/// Consts stay in sync: the SQL modifier's magnitude must match
/// [`RECURRING_ZOMBIE_GRACE_HOURS`] so operators reading logs and the
/// SQL query agree on the same window.
#[test]
fn zombie_guard_grace_consts_stay_in_sync() {
    let expected = format!("-{RECURRING_ZOMBIE_GRACE_HOURS} hours");
    assert_eq!(
        RECURRING_ZOMBIE_GRACE_SQL, expected,
        "RECURRING_ZOMBIE_GRACE_SQL must match RECURRING_ZOMBIE_GRACE_HOURS"
    );
}

#[test]
fn test_cancelled_recurring_task_allows_re_creation() {
    let db = db();

    // Step 1: Create a recurring task
    let task = NewTask {
        agent_id: "mika".to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: "heartbeat".to_string(),
        trigger_type: "recurring".to_string(),
        cron_expr: Some("0 0 * * * *".to_string()),
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: Some("2286-11-20T17:46:39Z".to_string()),
        timeout_at: None,
        action_type: "inject_context".to_string(),
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
    let id1 = db.create_task(&task).unwrap();
    assert!(!id1.is_empty());

    // Step 2: Cancel it
    assert!(db.cancel_task(&id1, "mika").unwrap());
    let t = db.get_task(&id1, "mika").unwrap().unwrap();
    assert_eq!(t.status, "cancelled");

    // Step 3: Create another recurring task with the same label — should succeed
    let task2 = NewTask {
        agent_id: "mika".to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: "heartbeat".to_string(),
        trigger_type: "recurring".to_string(),
        cron_expr: Some("0 0 * * * *".to_string()),
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: Some("2286-11-20T17:46:39Z".to_string()),
        timeout_at: None,
        action_type: "inject_context".to_string(),
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
    let id2 = db.create_task(&task2).unwrap();
    assert!(!id2.is_empty());
    assert_ne!(id1, id2, "should be a new task, not the old cancelled one");

    // Verify the new task is pending
    let t2 = db.get_task(&id2, "mika").unwrap().unwrap();
    assert_eq!(t2.status, "pending");
    assert_eq!(t2.label, "heartbeat");
}

// ── mika#2446 — la levée opérateur (`mika tasks rearm`) ──────────────────
//
// Jumeaux des tests mika#2337 : le premier dit ce que l'acte achète, les
// suivants ce qu'il ne touche pas. Pris isolément, le premier passerait sur
// un désarmement général de mika#1742.

/// AC6 — le marqueur opérateur lève le veto pour son label : une mort de
/// cause quelconque, dans la fenêtre, ne bloque plus la ré-inscription.
#[test]
fn mika2446_operator_rearm_lifts_the_veto_for_its_label() {
    let db = db();
    kill_recurring(&db, "worktree_reap", "-1 hour", false);

    assert!(
        db.create_recurring_task_if_absent(zombie_recurring_task("mika", "worktree_reap"))
            .unwrap()
            .is_none(),
        "précondition : sans l'acte, le veto mika#1742 est armé"
    );

    assert_eq!(
        db.mark_recurring_operator_rearm("mika", "worktree_reap")
            .unwrap(),
        1
    );
    assert!(
        db.create_recurring_task_if_absent(zombie_recurring_task("mika", "worktree_reap"))
            .unwrap()
            .is_some(),
        "une ligne morte levée par l'opérateur ne doit plus bloquer la ré-inscription"
    );
}

/// AC8 — la levée est per-label : un autre label mort dans la même fenêtre
/// garde son veto armé.
#[test]
fn mika2446_operator_rearm_is_scoped_to_its_label() {
    let db = db();
    kill_recurring(&db, "worktree_reap", "-1 hour", false);
    kill_recurring(&db, "curator_review", "-1 hour", false);

    db.mark_recurring_operator_rearm("mika", "worktree_reap")
        .unwrap();

    assert!(
        db.create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
            .unwrap()
            .is_none(),
        "la levée d'un label ne doit rien lever ailleurs — mika#1742 reste armé"
    );
}

/// AC8 — la levée absout les morts qui existaient au moment de l'acte,
/// jamais une mort postérieure : la ligne ré-inscrite qui meurt à son tour
/// retrouve un veto armé.
#[test]
fn mika2446_a_death_after_the_rearm_still_arms_the_veto() {
    let db = db();
    kill_recurring(&db, "worktree_reap", "-2 hours", false);
    db.mark_recurring_operator_rearm("mika", "worktree_reap")
        .unwrap();
    let revived = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", "worktree_reap"))
        .unwrap()
        .expect("la levée doit laisser passer la ré-inscription");

    db.conn
        .execute(
            "UPDATE tasks SET status = 'failed',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hour')
                 WHERE id = ?1",
            params![revived],
        )
        .unwrap();

    assert!(
        db.create_recurring_task_if_absent(zombie_recurring_task("mika", "worktree_reap"))
            .unwrap()
            .is_none(),
        "le ré-armement est un acte, pas une immunité : la mort suivante arme le veto"
    );
}

/// La levée doit couvrir TOUTES les morts du label : la garde prend la plus
/// récente non exemptée, donc n'absoudre que la dernière exposerait celle
/// d'avant et l'acte serait refusé par son propre historique.
#[test]
fn mika2446_operator_rearm_covers_every_dead_row_of_the_label() {
    let db = db();
    let older = kill_recurring(&db, "worktree_reap", "-3 hours", false);
    // Seconde mort : on contourne la garde pour poser une seconde ligne.
    db.conn
        .execute(
            "UPDATE tasks SET metadata = json_set(COALESCE(metadata, '{}'), '$.config_cancel_reverted', 1)
                 WHERE id = ?1",
            params![older],
        )
        .unwrap();
    let newer = kill_recurring(&db, "worktree_reap", "-1 hour", false);
    db.conn
        .execute(
            "UPDATE tasks SET metadata = json_remove(metadata, '$.config_cancel_reverted')
                 WHERE id = ?1",
            params![older],
        )
        .unwrap();
    assert_ne!(older, newer);

    assert_eq!(
        db.mark_recurring_operator_rearm("mika", "worktree_reap")
            .unwrap(),
        2,
        "les deux lignes mortes doivent porter le marqueur"
    );
    assert!(
        db.create_recurring_task_if_absent(zombie_recurring_task("mika", "worktree_reap"))
            .unwrap()
            .is_some()
    );
}

/// La cible est la mort la plus récente, et la recherche est insensible à la
/// casse comme la garde — mais rend l'orthographe stockée.
#[test]
fn mika2446_rearm_target_is_the_latest_dead_row() {
    let db = db();
    assert!(
        db.find_recurring_rearm_target("mika", "worktree_reap")
            .unwrap()
            .is_none(),
        "aucune ligne morte → aucune cible (jamais de création ex nihilo)"
    );
    let id = kill_recurring(&db, "worktree_reap", "-1 hour", false);

    let target = db
        .find_recurring_rearm_target("mika", "WORKTREE_REAP")
        .unwrap()
        .expect("la ligne morte doit être trouvée");
    assert_eq!(target.task_id, id);
    assert_eq!(target.label, "worktree_reap");
    assert_eq!(target.status, "failed");
    assert_eq!(target.cron_expr.as_deref(), Some("0 0 * * * *"));
}

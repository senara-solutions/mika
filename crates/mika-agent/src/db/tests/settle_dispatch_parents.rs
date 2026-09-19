//! Tests de `crate::db` — la requête du fermeur de parents de dispatch
//! (mika#2405, U1).
//!
//! Sa population est le **complément** de celle du couple self_dev voisin
//! (`find_orphaned_parent_tasks` / `find_completable_parent_tasks_on_pr_url`),
//! pas un troisième membre : d'où un contrôle négatif `self_dev` explicite
//! plutôt qu'une simple couverture du chemin nominal.
//!
//! Enfant de `db::tests`, donc descendant de `db` : les items privés de `db` et
//! les helpers de `db::tests` restent visibles via `use super::*`.

use super::*;

/// La forme nominale : une row `manual` de suivi sans `source`, un enfant
/// callback `delivered` vieilli au-delà de la fenêtre de grâce.
#[test]
fn mika2405_settleable_parent_with_terminal_child_is_selected() {
    let db = db();
    let (parent_id, _child_id) = create_settleable_parent_setup(&db, 700);

    let rows = db.find_settleable_dispatch_parents("mika", 600).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, parent_id);
    assert_eq!(
        rows[0].child_count, 1,
        "child_count porte les enfants callback joints"
    );
    assert!(
        !rows[0].last_child_at.is_empty(),
        "last_child_at doit porter MAX(child.updated_at)"
    );
}

/// **Contrôle négatif du remède qui saute aux yeux** (refus n°1 du plan). La
/// population self_dev garde ses propres faucheuses, dont le verdict est plus
/// riche (`failed` sans `pr_url`, `completed` avec). Retirer ce terme — la
/// correction d'une ligne — enverrait tout le trafic QA sur leur branche
/// `failed`.
#[test]
fn mika2405_self_dev_parent_is_never_selected() {
    let db = db();
    let (parent_id, _child_id) = create_settleable_parent_setup(&db, 700);
    db.conn
        .execute(
            "UPDATE tasks SET source = 'self_dev' WHERE id = ?1",
            params![parent_id],
        )
        .unwrap();

    let rows = db.find_settleable_dispatch_parents("mika", 600).unwrap();
    assert!(
        rows.is_empty(),
        "un parent self_dev relève du couple #871/#1162, pas de ce fermeur"
    );
}

/// Le `COALESCE` porte sur le cas NULL (couvert par le test nominal) ; celui-ci
/// couvre l'autre moitié — une `source` non nulle mais simplement différente de
/// `self_dev` est bien dans la population. Le terme discrimine donc sur la
/// valeur, pas sur la nullité.
#[test]
fn mika2405_non_self_dev_source_is_selected() {
    let db = db();
    let (parent_id, _child_id) = create_settleable_parent_setup(&db, 700);
    db.conn
        .execute(
            "UPDATE tasks SET source = 'qa_review' WHERE id = ?1",
            params![parent_id],
        )
        .unwrap();

    let rows = db.find_settleable_dispatch_parents("mika", 600).unwrap();
    assert_eq!(
        rows.len(),
        1,
        "seul 'self_dev' est exclu, pas toute source non nulle"
    );
    assert_eq!(rows[0].id, parent_id);
}

/// Grâce non écoulée : l'enfant vient de bouger.
#[test]
fn mika2405_grace_not_elapsed_is_not_selected() {
    let db = db();
    let (_parent_id, _child_id) = create_settleable_parent_setup(&db, 0);

    let rows = db.find_settleable_dispatch_parents("mika", 600).unwrap();
    assert!(
        rows.is_empty(),
        "un enfant qui vient de bouger est dans la grâce"
    );
}

/// La grâce porte sur `MAX(child.updated_at)`, jamais sur l'âge du parent — un
/// parent réutilisé (mika#920) est ancien par construction, donc une grâce sur
/// `parent.created_at` fermerait une row dont le dispatch vient de revenir.
#[test]
fn mika2405_grace_keys_on_last_child_not_on_parent_age() {
    let db = db();
    let (parent_id, _child_id) = create_settleable_parent_setup(&db, 0);
    // Parent bien plus vieux que toute grâce ; enfant frais.
    db.conn
        .execute(
            "UPDATE tasks SET created_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-30 days') \
             WHERE id = ?1",
            params![parent_id],
        )
        .unwrap();

    let rows = db.find_settleable_dispatch_parents("mika", 600).unwrap();
    assert!(
        rows.is_empty(),
        "un parent ancien avec un enfant frais n'est pas fermable"
    );
}

/// **Le terme le plus facile à « simplifier » par erreur.** Sur une row
/// callback, `completed` signifie « le pilote est revenu, la livraison n'a pas
/// encore eu lieu » ; `delivered` est l'état terminal. Fermer ici fermerait le
/// parent avant que son tour de verdict ait tourné.
#[test]
fn mika2405_sibling_completed_defers() {
    let db = db();
    let (parent_id, _child_id) = create_settleable_parent_setup(&db, 700);

    let mut sibling = callback_task("mika");
    sibling.parent_task_id = Some(parent_id.clone());
    let sibling_id = db.create_task(&sibling).unwrap();
    assert!(
        db.update_task_completed(&sibling_id, "mika", Some("returned"))
            .unwrap()
    );
    backdate_task_updated_at(&db, &sibling_id, 700);

    let rows = db.find_settleable_dispatch_parents("mika", 600).unwrap();
    assert!(
        rows.is_empty(),
        "un enfant callback `completed` n'a pas été livré — le parent doit attendre"
    );
}

/// Les trois autres statuts non terminaux, chacun sur sa propre base pour
/// qu'un échec nomme celui qui a cessé de mordre.
#[test]
fn mika2405_each_non_terminal_sibling_status_defers() {
    for status in ["pending", "in_progress", "blocked"] {
        let db = db();
        let (parent_id, _child_id) = create_settleable_parent_setup(&db, 700);

        let mut sibling = callback_task("mika");
        sibling.parent_task_id = Some(parent_id.clone());
        let sibling_id = db.create_task(&sibling).unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = ?2 WHERE id = ?1",
                params![sibling_id, status],
            )
            .unwrap();
        backdate_task_updated_at(&db, &sibling_id, 700);

        let rows = db.find_settleable_dispatch_parents("mika", 600).unwrap();
        assert!(
            rows.is_empty(),
            "un frère en `{status}` doit différer la fermeture du parent"
        );
    }
}

/// L'enfant joint ne peut pas s'auto-exclure : la liste de statuts du
/// `NOT EXISTS` ne contient aucun état terminal, ce qui est précisément
/// pourquoi la garde n'a pas besoin du `sibling.id != child.id` de ses deux
/// voisines. Un enfant `delivered` sélectionne donc bien son parent (test
/// nominal ci-dessus), et un enfant `completed` l'exclut même quand il est le
/// seul.
#[test]
fn mika2405_sole_child_still_completed_excludes_its_own_parent() {
    let db = db();
    let parent = new_task("mika", "long_running:build_mika", "manual", "none");
    let parent_id = db.create_task(&parent).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
            params![parent_id],
        )
        .unwrap();

    let mut child = callback_task("mika");
    child.parent_task_id = Some(parent_id.clone());
    let child_id = db.create_task(&child).unwrap();
    assert!(
        db.update_task_completed(&child_id, "mika", Some("returned"))
            .unwrap()
    );
    backdate_task_updated_at(&db, &child_id, 700);

    let rows = db.find_settleable_dispatch_parents("mika", 600).unwrap();
    assert!(
        rows.is_empty(),
        "le seul enfant est `completed`, donc le parent n'est pas fermable"
    );
}

/// Fail-safe : un parent sans aucun enfant callback est hors population. Un
/// dispatch refusé avant création de l'enfant (`dispatch_limit_exceeded`) n'a
/// pas reçu de dispatch, et ce fermeur n'a rien à en dire.
#[test]
fn mika2405_childless_parent_is_out_of_population() {
    let db = db();
    let parent = new_task("mika", "long_running:build_mika", "manual", "none");
    let parent_id = db.create_task(&parent).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
            params![parent_id],
        )
        .unwrap();
    backdate_task_updated_at(&db, &parent_id, 7000);

    let rows = db.find_settleable_dispatch_parents("mika", 600).unwrap();
    assert!(rows.is_empty(), "le JOIN exclut un parent sans enfant");
}

/// Un enfant qui n'est pas un callback `resume_agent` ne rend pas son parent
/// fermable : les deux termes du maillon 4 de l'AC1 sont requis ensemble.
#[test]
fn mika2405_non_callback_child_does_not_select_the_parent() {
    let db = db();
    let parent = new_task("mika", "long_running:build_mika", "manual", "none");
    let parent_id = db.create_task(&parent).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
            params![parent_id],
        )
        .unwrap();

    let mut child = new_task("mika", "note", "manual", "none");
    child.parent_task_id = Some(parent_id.clone());
    let child_id = db.create_task(&child).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'cancelled' WHERE id = ?1",
            params![child_id],
        )
        .unwrap();
    backdate_task_updated_at(&db, &child_id, 700);

    let rows = db.find_settleable_dispatch_parents("mika", 600).unwrap();
    assert!(
        rows.is_empty(),
        "seul un enfant callback/resume_agent met un parent dans la population"
    );
}

/// Cadrage par agent, comme toutes les requêtes voisines.
#[test]
fn mika2405_excludes_other_agents() {
    let db = db();
    db.register_agent("agent_a", "Agent A", "/tmp/a").unwrap();
    let (_parent_id, _child_id) = create_settleable_parent_setup(&db, 700);

    let rows = db.find_settleable_dispatch_parents("agent_a", 600).unwrap();
    assert!(rows.is_empty(), "les rows d'un autre agent sont invisibles");
}

/// Un parent ayant déjà quitté `in_progress` est hors population — le fermeur
/// ne revisite jamais une row terminale.
#[test]
fn mika2405_terminal_parent_is_out_of_population() {
    let db = db();
    let (parent_id, _child_id) = create_settleable_parent_setup(&db, 700);
    db.conn
        .execute(
            "UPDATE tasks SET status = 'cancelled' WHERE id = ?1",
            params![parent_id],
        )
        .unwrap();

    let rows = db.find_settleable_dispatch_parents("mika", 600).unwrap();
    assert!(rows.is_empty(), "un parent annulé n'est pas fermable");
}

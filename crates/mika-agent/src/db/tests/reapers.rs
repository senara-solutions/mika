//! Tests de `crate::db` — thème `reapers` (mika#2321).
//!
//! Les requêtes de sélection des faucheuses et de leurs compléments : parents
//! orphelins, balayage des phantoms NULL-PID, parents `pending` bloqués,
//! wrappers différés, parents sans enfant, auto-complétion sur `pr_url`.
//!
//! Enfant de `db::tests`, donc descendant de `db` : les items privés de `db` et
//! les helpers de `db::tests` restent visibles via `use super::*`.

use super::*;

#[test]
fn test_get_undelivered_callback_tasks_returns_completed() {
    let db = db();
    let id = db.create_task(&callback_task("mika")).unwrap();

    // Pending task should not appear
    let results = db
        .get_undelivered_callback_tasks("mika", "1970-01-01T00:00:00Z")
        .unwrap();
    assert!(results.is_empty());

    // Complete it
    assert!(db.update_task_completed(&id, "mika", Some("done")).unwrap());

    // Now it should appear
    let results = db
        .get_undelivered_callback_tasks("mika", "1970-01-01T00:00:00Z")
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, id);
}

#[test]
fn test_get_undelivered_callback_tasks_since_boundary() {
    let db = db();
    let id = db.create_task(&callback_task("mika")).unwrap();
    assert!(db.update_task_completed(&id, "mika", Some("done")).unwrap());

    // Get the completed_at value
    let task = db.get_task(&id, "mika").unwrap().unwrap();
    let completed_at = task.completed_at.unwrap();

    // since = completed_at means "after this time", so the task at exactly
    // that time should still be included (query uses >)
    // But since completed_at == since, and query is >, it should NOT appear
    let results = db
        .get_undelivered_callback_tasks("mika", &completed_at)
        .unwrap();
    assert!(results.is_empty());

    // since before completed_at should include it
    let before = {
        let dt = timestamp::parse(&completed_at).unwrap();
        timestamp::format(&(dt - Duration::seconds(1)))
    };
    let results = db.get_undelivered_callback_tasks("mika", &before).unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn test_get_undelivered_callback_tasks_excludes_other_agents() {
    let db = db();
    db.register_agent("agent_a", "Agent A", "/tmp/a").unwrap();
    db.register_agent("agent_b", "Agent B", "/tmp/b").unwrap();
    let id = db.create_task(&callback_task("agent_a")).unwrap();
    assert!(db.update_task_completed(&id, "agent_a", Some("x")).unwrap());

    let results = db
        .get_undelivered_callback_tasks("agent_b", "1970-01-01T00:00:00Z")
        .unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_get_undelivered_callback_tasks_for_session_scoped() {
    let db = db();
    // Create a task in session_a
    let mut task = callback_task("mika");
    task.created_by_session = Some("session_a".to_string());
    let id = db.create_task(&task).unwrap();
    assert!(db.update_task_completed(&id, "mika", Some("done")).unwrap());

    // Session A should see it
    let results = db
        .get_undelivered_callback_tasks_for_session("mika", "1970-01-01T00:00:00Z", "session_a")
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, id);

    // Session B should NOT see it
    let results = db
        .get_undelivered_callback_tasks_for_session("mika", "1970-01-01T00:00:00Z", "session_b")
        .unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_get_undelivered_callback_tasks_for_session_excludes_no_session() {
    let db = db();
    // Task with no session (created_by_session = None) should not appear
    let id = db.create_task(&callback_task("mika")).unwrap();
    assert!(db.update_task_completed(&id, "mika", Some("done")).unwrap());

    let results = db
        .get_undelivered_callback_tasks_for_session("mika", "1970-01-01T00:00:00Z", "session_a")
        .unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_mark_task_delivered_claims_completed_task() {
    let db = db();
    let id = db.create_task(&callback_task("mika")).unwrap();
    assert!(
        db.update_task_completed(&id, "mika", Some("result"))
            .unwrap()
    );

    // First claim succeeds
    assert!(db.mark_task_delivered(&id).unwrap());

    let task = db.get_task(&id, "mika").unwrap().unwrap();
    assert_eq!(task.status, "delivered");
}

#[test]
fn test_mark_task_delivered_double_claim_rejected() {
    let db = db();
    let id = db.create_task(&callback_task("mika")).unwrap();
    assert!(
        db.update_task_completed(&id, "mika", Some("result"))
            .unwrap()
    );

    // First claim
    assert!(db.mark_task_delivered(&id).unwrap());
    // Second claim returns false (already delivered)
    assert!(!db.mark_task_delivered(&id).unwrap());
}

#[test]
fn test_mark_task_delivered_rejects_non_completed_task() {
    let db = db();
    let id = db.create_task(&callback_task("mika")).unwrap();

    // Task is still pending, should not be claimable
    assert!(!db.mark_task_delivered(&id).unwrap());
}

#[test]
fn test_get_undelivered_callback_tasks_returns_failed_tasks() {
    let db = db();
    let id = db.create_task(&callback_task("mika")).unwrap();

    // Mark it as failed (simulates background monitor detecting non-zero exit)
    db.update_task_failed(&id, "mika", "Process exited with code 1: error output")
        .unwrap();

    // Failed task should appear in undelivered callbacks
    let results = db
        .get_undelivered_callback_tasks("mika", "1970-01-01T00:00:00Z")
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, id);
    assert_eq!(results[0].status, "failed");
    assert_eq!(
        results[0].result.as_deref(),
        Some("Process exited with code 1: error output")
    );
}

#[test]
fn test_get_undelivered_callback_tasks_returns_both_completed_and_failed() {
    let db = db();
    let id1 = db.create_task(&callback_task("mika")).unwrap();
    let id2 = db.create_task(&callback_task("mika")).unwrap();

    // Complete one, fail the other
    assert!(
        db.update_task_completed(&id1, "mika", Some("success"))
            .unwrap()
    );
    db.update_task_failed(&id2, "mika", "Process exited with code 128: fatal error")
        .unwrap();

    // Both should appear, ordered by completed_at
    let results = db
        .get_undelivered_callback_tasks("mika", "1970-01-01T00:00:00Z")
        .unwrap();
    assert_eq!(results.len(), 2);
    // Both tasks present (order depends on completed_at which is set to 'now' for both)
    let statuses: Vec<&str> = results.iter().map(|t| t.status.as_str()).collect();
    assert!(statuses.contains(&"completed"));
    assert!(statuses.contains(&"failed"));
}

#[test]
fn test_get_undelivered_callback_tasks_for_session_returns_failed() {
    let db = db();
    let mut task = callback_task("mika");
    task.created_by_session = Some("session_a".to_string());
    let id = db.create_task(&task).unwrap();
    db.update_task_failed(&id, "mika", "crash").unwrap();

    // Session A should see the failed task
    let results = db
        .get_undelivered_callback_tasks_for_session("mika", "1970-01-01T00:00:00Z", "session_a")
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, "failed");

    // Session B should not
    let results = db
        .get_undelivered_callback_tasks_for_session("mika", "1970-01-01T00:00:00Z", "session_b")
        .unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_get_active_background_task_count_counts_pending_and_in_progress() {
    let db = db();
    // No callback tasks → count is 0
    assert_eq!(db.get_active_background_task_count("mika").unwrap(), 0);

    // Create a pending callback task → count is 1
    let id1 = db.create_task(&callback_task("mika")).unwrap();
    assert_eq!(db.get_active_background_task_count("mika").unwrap(), 1);

    // Create a second pending callback task → count is 2
    let mut task2 = callback_task("mika");
    task2.label = "build_project".to_string();
    let _id2 = db.create_task(&task2).unwrap();
    assert_eq!(db.get_active_background_task_count("mika").unwrap(), 2);

    // Complete the first task → count drops to 1
    assert!(
        db.update_task_completed(&id1, "mika", Some("done"))
            .unwrap()
    );
    assert_eq!(db.get_active_background_task_count("mika").unwrap(), 1);
}

#[test]
fn test_get_active_background_task_count_excludes_other_agents() {
    let db = db();
    db.register_agent("agent_a", "Agent A", "/tmp/a").unwrap();
    db.register_agent("agent_b", "Agent B", "/tmp/b").unwrap();

    let _id = db.create_task(&callback_task("agent_a")).unwrap();

    // agent_a has 1, agent_b has 0
    assert_eq!(db.get_active_background_task_count("agent_a").unwrap(), 1);
    assert_eq!(db.get_active_background_task_count("agent_b").unwrap(), 0);
}

#[test]
fn test_get_active_background_task_count_excludes_non_callback_tasks() {
    let db = db();
    // Create a non-callback task (e.g., a reminder)
    let reminder = NewTask {
        agent_id: "mika".to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: "morning reminder".to_string(),
        trigger_type: "time".to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: None,
        timeout_at: None,
        action_type: "send_message".to_string(),
        action_config: r#"{"text":"hello"}"#.to_string(),
        input_context: None,
        created_by_session: None,
        created_trace_id: None,
        reference_url: None,
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: None,
    };
    let _id = db.create_task(&reminder).unwrap();

    // Reminder should NOT count as a background task
    assert_eq!(db.get_active_background_task_count("mika").unwrap(), 0);
}

#[test]
fn test_get_background_task_counts_splits_executing_and_queued() {
    let db = db();

    // No callback tasks → both zero
    let counts = db.get_background_task_counts("mika").unwrap();
    assert_eq!(
        counts,
        BackgroundTaskCounts {
            executing: 0,
            queued: 0
        }
    );

    // Create two pending callback tasks (no process_id) → both queued
    let id1 = db.create_task(&callback_task("mika")).unwrap();
    let mut task2 = callback_task("mika");
    task2.label = "build_project".to_string();
    let _id2 = db.create_task(&task2).unwrap();
    let counts = db.get_background_task_counts("mika").unwrap();
    assert_eq!(
        counts,
        BackgroundTaskCounts {
            executing: 0,
            queued: 2
        }
    );

    // Set process_id on task1 → 1 executing, 1 queued
    db.set_task_process_id(&id1, Some(12345)).unwrap();
    let counts = db.get_background_task_counts("mika").unwrap();
    assert_eq!(
        counts,
        BackgroundTaskCounts {
            executing: 1,
            queued: 1
        }
    );

    // Backward compat: total still matches
    assert_eq!(db.get_active_background_task_count("mika").unwrap(), 2);
}

#[test]
fn test_get_background_task_counts_all_executing() {
    let db = db();

    let id1 = db.create_task(&callback_task("mika")).unwrap();
    let mut task2 = callback_task("mika");
    task2.label = "build_project".to_string();
    let id2 = db.create_task(&task2).unwrap();

    db.set_task_process_id(&id1, Some(100)).unwrap();
    db.set_task_process_id(&id2, Some(200)).unwrap();

    let counts = db.get_background_task_counts("mika").unwrap();
    assert_eq!(
        counts,
        BackgroundTaskCounts {
            executing: 2,
            queued: 0
        }
    );
}

#[test]
fn test_get_background_task_counts_excludes_terminal_status() {
    let db = db();

    let id1 = db.create_task(&callback_task("mika")).unwrap();
    db.set_task_process_id(&id1, Some(100)).unwrap();

    // Complete the task — should not appear in either bucket
    db.update_task_completed(&id1, "mika", Some("done"))
        .unwrap();
    let counts = db.get_background_task_counts("mika").unwrap();
    assert_eq!(
        counts,
        BackgroundTaskCounts {
            executing: 0,
            queued: 0
        }
    );
}

#[test]
fn test_mark_task_delivered_claims_failed_task() {
    let db = db();
    let id = db.create_task(&callback_task("mika")).unwrap();
    db.update_task_failed(&id, "mika", "exit code 1").unwrap();

    // Claim the failed task
    assert!(db.mark_task_delivered(&id).unwrap());

    let task = db.get_task(&id, "mika").unwrap().unwrap();
    assert_eq!(task.status, "delivered");
}

#[test]
fn test_mark_task_delivered_failed_double_claim_rejected() {
    let db = db();
    let id = db.create_task(&callback_task("mika")).unwrap();
    db.update_task_failed(&id, "mika", "exit code 1").unwrap();

    // First claim
    assert!(db.mark_task_delivered(&id).unwrap());
    // Second claim returns false (already delivered)
    assert!(!db.mark_task_delivered(&id).unwrap());
}

#[test]
fn test_find_orphaned_parent_tasks_failure_path() {
    let db = db();
    let (parent_id, child_id) = create_orphaned_parent_setup(&db);

    // Backdate the child's updated_at so it's past the grace period (700s > 600s)
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    let orphans = db.find_orphaned_parent_tasks("mika", 600).unwrap();
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0].id, parent_id);
    assert_eq!(orphans[0].callback_task_id, child_id);
}

#[test]
fn test_find_orphaned_parent_tasks_happy_path_pr_url_present() {
    let db = db();
    let (parent_id, child_id) = create_orphaned_parent_setup(&db);

    // Set pr_url on parent metadata — reaper should NOT match
    let meta = r#"{"claude_pilot": {"pr_url": "https://github.com/x/y/pull/1"}}"#;
    db.conn
        .execute(
            "UPDATE tasks SET metadata = ?1 WHERE id = ?2",
            params![meta, parent_id],
        )
        .unwrap();

    // Backdate child past grace
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    let orphans = db.find_orphaned_parent_tasks("mika", 600).unwrap();
    assert!(
        orphans.is_empty(),
        "parent with pr_url should not be reaped"
    );
}

#[test]
fn test_find_orphaned_parent_tasks_grace_period_not_elapsed() {
    let db = db();
    let (_parent_id, _child_id) = create_orphaned_parent_setup(&db);

    // Child was just delivered (updated_at = now), well within 600s grace
    let orphans = db.find_orphaned_parent_tasks("mika", 600).unwrap();
    assert!(
        orphans.is_empty(),
        "parent within grace period should not be reaped"
    );
}

#[test]
fn test_find_orphaned_parent_tasks_active_sibling_defers() {
    let db = db();
    let (parent_id, child_id) = create_orphaned_parent_setup(&db);

    // Backdate the delivered child past grace
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    // Add a sibling callback child that's still in_progress (e.g., #870 retry)
    let mut sibling = callback_task("mika");
    sibling.parent_task_id = Some(parent_id.clone());
    let sibling_id = db.create_task(&sibling).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
            params![sibling_id],
        )
        .unwrap();

    let orphans = db.find_orphaned_parent_tasks("mika", 600).unwrap();
    assert!(
        orphans.is_empty(),
        "parent with active sibling callback should not be reaped"
    );
}

#[test]
fn test_find_orphaned_parent_tasks_excludes_other_agents() {
    let db = db();
    db.register_agent("agent_a", "Agent A", "/tmp/a").unwrap();
    db.register_agent("agent_b", "Agent B", "/tmp/b").unwrap();

    // Create orphaned parent under agent_a
    let mut parent = new_task("agent_a", "Implement task", "manual", "none");
    parent.source = Some("self_dev".to_string());
    let parent_id = db.create_task(&parent).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
            params![parent_id],
        )
        .unwrap();

    let mut child = NewTask {
        agent_id: "agent_a".to_string(),
        ..callback_task("agent_a")
    };
    child.parent_task_id = Some(parent_id.clone());
    let child_id = db.create_task(&child).unwrap();
    assert!(
        db.update_task_completed(&child_id, "agent_a", Some("done"))
            .unwrap()
    );
    assert!(db.mark_task_delivered(&child_id).unwrap());
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    // agent_b should see nothing
    let orphans = db.find_orphaned_parent_tasks("agent_b", 600).unwrap();
    assert!(orphans.is_empty());

    // agent_a should see the orphan
    let orphans = db.find_orphaned_parent_tasks("agent_a", 600).unwrap();
    assert_eq!(orphans.len(), 1);
}

/// mika#1118 v2 — groom-class CALLBACKS must NOT trigger reaping of their
/// parent. The class is keyed off the child (per-dispatch) because reused
/// parents (mika#920) carry stale class data.
#[test]
fn test_find_orphaned_parent_tasks_groom_class_not_reaped() {
    let db = db();
    let (_parent_id, child_id) = create_orphaned_parent_setup(&db);

    // Set CHILD callback dispatch_class to 'groom' (the per-dispatch class)
    db.conn
        .execute(
            "UPDATE tasks SET dispatch_class = 'groom' WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    // Backdate child past grace (would otherwise trigger reaping)
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    let orphans = db.find_orphaned_parent_tasks("mika", 600).unwrap();
    assert!(
        orphans.is_empty(),
        "parent with groom-class callback should not be reaped — grooming produces plan commits, not PRs"
    );
}

/// mika#1118 v2 — implement-class callbacks still trigger reaping when the
/// dispatch completes without producing a PR. Confirms the per-child filter
/// does not regress the original #871 behavior.
#[test]
fn test_find_orphaned_parent_tasks_implement_class_still_reaped() {
    let db = db();
    let (parent_id, child_id) = create_orphaned_parent_setup(&db);

    // Set CHILD callback dispatch_class explicitly to 'implement'
    db.conn
        .execute(
            "UPDATE tasks SET dispatch_class = 'implement' WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    // Backdate child past grace
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    let orphans = db.find_orphaned_parent_tasks("mika", 600).unwrap();
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0].id, parent_id);
    assert_eq!(orphans[0].callback_task_id, child_id);
}

/// mika#1118 v2 — NULL dispatch_class on the CHILD callback (pre-v34 rows)
/// is treated as 'implement' via COALESCE. Preserves backward compatibility
/// with rows created before the v33→v34 migration added the column.
#[test]
fn test_find_orphaned_parent_tasks_null_dispatch_class_treated_as_implement() {
    let db = db();
    let (parent_id, child_id) = create_orphaned_parent_setup(&db);

    // Helper does NOT set dispatch_class — defaults to NULL on both parent
    // and child. Verify the CHILD's column is actually NULL.
    let class: Option<String> = db
        .conn
        .query_row(
            "SELECT dispatch_class FROM tasks WHERE id = ?1",
            params![child_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        class.is_none(),
        "test setup invariant: child dispatch_class should be NULL pre-v34-style"
    );

    // Backdate child past grace
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    let orphans = db.find_orphaned_parent_tasks("mika", 600).unwrap();
    assert_eq!(
        orphans.len(),
        1,
        "NULL child dispatch_class must still be reaped (COALESCE -> 'implement')"
    );
    assert_eq!(orphans[0].id, parent_id);
}

/// mika#1118 v2 — the key regression case: reused parent (mika#920) with
/// NULL `parent.dispatch_class` but groom-class `child.dispatch_class`.
/// v1 would have reaped because it checked the parent's class. v2 must NOT
/// reap because the child's class is the per-dispatch authority.
#[test]
fn test_find_orphaned_parent_tasks_stale_parent_class_doesnt_drive_reaping() {
    let db = db();
    let (_parent_id, child_id) = create_orphaned_parent_setup(&db);

    // Parent stays NULL (simulates reused parent from pre-v34 or mika#920).
    // Child is the fresh groom-class dispatch.
    db.conn
        .execute(
            "UPDATE tasks SET dispatch_class = 'groom' WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    // Backdate child past grace
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    let orphans = db.find_orphaned_parent_tasks("mika", 600).unwrap();
    assert!(
        orphans.is_empty(),
        "stale parent class must not override the fresh child class — v1 regression"
    );
}

/// mika#1126 — H3 scenario: parent with two children where one has NULL
/// dispatch_class (treated as 'implement') and one has 'groom'. The NULL
/// child matches the reaper filter, so the parent IS reaped.
#[test]
fn test_find_orphaned_parent_tasks_mixed_children_groom_and_null() {
    let db = db();
    let (parent_id, child_a_id) = create_orphaned_parent_setup(&db);

    // child_a: NULL dispatch_class (default from helper), callback, delivered
    // Already set up by helper. Backdate past grace.
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_a_id],
        )
        .unwrap();

    // child_b: groom dispatch_class, callback, delivered
    let mut child_b = callback_task("mika");
    child_b.parent_task_id = Some(parent_id.clone());
    child_b.dispatch_class = Some("groom".to_string());
    let child_b_id = db.create_task(&child_b).unwrap();
    assert!(
        db.update_task_completed(&child_b_id, "mika", Some("done"))
            .unwrap()
    );
    assert!(db.mark_task_delivered(&child_b_id).unwrap());
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_b_id],
        )
        .unwrap();

    let orphans = db.find_orphaned_parent_tasks("mika", 600).unwrap();
    assert_eq!(
        orphans.len(),
        1,
        "parent with mixed children (NULL + groom) should be reaped — the NULL child matches"
    );
    assert_eq!(orphans[0].id, parent_id);
}

/// mika#1126 — parent with ONLY groom-class children should NOT be reaped.
/// Both children have dispatch_class='groom'; no implement-class child exists.
#[test]
fn test_find_orphaned_parent_tasks_only_groom_children_not_reaped() {
    let db = db();
    let (parent_id, child_a_id) = create_orphaned_parent_setup(&db);

    // Set child_a to groom class
    db.conn
        .execute(
            "UPDATE tasks SET dispatch_class = 'groom' WHERE id = ?1",
            params![child_a_id],
        )
        .unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_a_id],
        )
        .unwrap();

    // child_b: also groom class
    let mut child_b = callback_task("mika");
    child_b.parent_task_id = Some(parent_id.clone());
    child_b.dispatch_class = Some("groom".to_string());
    let child_b_id = db.create_task(&child_b).unwrap();
    assert!(
        db.update_task_completed(&child_b_id, "mika", Some("done"))
            .unwrap()
    );
    assert!(db.mark_task_delivered(&child_b_id).unwrap());
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_b_id],
        )
        .unwrap();

    let orphans = db.find_orphaned_parent_tasks("mika", 600).unwrap();
    assert!(
        orphans.is_empty(),
        "parent with only groom-class children should not be reaped"
    );
}

#[test]
fn test_find_dispatch_children_with_pid_ignores_pidless_children() {
    let db = db();
    // Accented label: this repo's real tracking rows are written in
    // French, so the fixture exercises the population we actually have.
    let parent = create_phantom_tracking_row(
        &db,
        "mika",
        "ready-label: mika#2156 — suivi différé",
        "in_progress",
        7200,
    );
    create_recall_child(&db, "mika", &parent, "note sans procédé", None, None);
    let with_pid = create_recall_child(
        &db,
        "mika",
        &parent,
        "long_running:run_claude_pilot",
        Some(365_667),
        Some("52755192"),
    );

    let children = db.find_dispatch_children_with_pid(&parent).unwrap();
    assert_eq!(
        children.len(),
        1,
        "only the PID-carrying child is a liveness candidate"
    );
    assert_eq!(children[0].id, with_pid);
    assert_eq!(children[0].process_id, 365_667);
    assert_eq!(
        children[0].process_start_time,
        Some(52_755_192),
        "the executor stores start_time as a JSON string — it must parse back to a number"
    );
}

#[test]
fn test_find_dispatch_children_with_pid_start_time_absent_is_none() {
    let db = db();
    let parent = create_phantom_tracking_row(
        &db,
        "mika",
        "ready-label: mika#2156 — rappel sans horodatage",
        "in_progress",
        7200,
    );
    create_recall_child(
        &db,
        "mika",
        &parent,
        "long_running:run_claude_pilot",
        Some(4242),
        None,
    );

    let children = db.find_dispatch_children_with_pid(&parent).unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(
        children[0].process_start_time, None,
        "a missing start_time must surface as None so the caller sweeps (D-3), \
             never as a value that would make a recycled PID read as alive"
    );
}

#[test]
fn test_find_dispatch_children_with_pid_start_time_value_shapes() {
    let db = db();
    let parent = create_phantom_tracking_row(
        &db,
        "mika",
        "ready-label: mika#2156 — formes de métadonnées",
        "in_progress",
        7200,
    );

    // The executor writes a string; the integer arm exists so a
    // hand-written or future-shaped row is not silently treated as
    // missing. Both must parse.
    let as_text = create_recall_child(
        &db,
        "mika",
        &parent,
        "long_running: chaîne",
        Some(101),
        Some("52755192"),
    );
    let as_int = create_recall_child(
        &db,
        "mika",
        &parent,
        "long_running: entier",
        Some(102),
        None,
    );
    db.conn
        .execute(
            "UPDATE tasks SET metadata = json('{\"process_start_time\": 52755192}') WHERE id = ?1",
            params![as_int],
        )
        .unwrap();
    // Garbage and negatives must degrade to None, never to a number.
    let as_garbage = create_recall_child(
        &db,
        "mika",
        &parent,
        "long_running: pas un nombre",
        Some(103),
        Some("pas-un-nombre"),
    );
    let as_negative = create_recall_child(
        &db,
        "mika",
        &parent,
        "long_running: négatif",
        Some(104),
        None,
    );
    db.conn
        .execute(
            "UPDATE tasks SET metadata = json('{\"process_start_time\": -1}') WHERE id = ?1",
            params![as_negative],
        )
        .unwrap();

    let children = db.find_dispatch_children_with_pid(&parent).unwrap();
    let got = |id: &str| {
        children
            .iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("child {id} missing"))
            .process_start_time
    };
    assert_eq!(got(&as_text), Some(52_755_192), "string form must parse");
    assert_eq!(got(&as_int), Some(52_755_192), "integer form must parse");
    assert_eq!(got(&as_garbage), None, "non-numeric must degrade to None");
    assert_eq!(got(&as_negative), None, "negative must degrade to None");
}

#[test]
fn test_find_dispatch_children_with_pid_survives_malformed_metadata_sibling() {
    let db = db();
    let parent = create_phantom_tracking_row(
        &db,
        "mika",
        "ready-label: mika#2156 — un frère au JSON cassé",
        "in_progress",
        7200,
    );
    let broken = create_recall_child(&db, "mika", &parent, "long_running: cassé", Some(201), None);
    // Not JSON at all. SQLite's json_extract raises a hard error on this,
    // which without the json_valid guard would fail the lookup for the
    // WHOLE parent — and the caller would then have no answer about a
    // parent whose other child is alive. Removing the CASE WHEN
    // json_valid(...) wrapper in find_dispatch_children_with_pid turns
    // this test red.
    db.conn
        .execute(
            "UPDATE tasks SET metadata = 'pas du json' WHERE id = ?1",
            params![broken],
        )
        .unwrap();
    let healthy = create_recall_child(
        &db,
        "mika",
        &parent,
        "long_running: sain",
        Some(202),
        Some("52755192"),
    );

    let children = db
        .find_dispatch_children_with_pid(&parent)
        .expect("one malformed sibling must not fail the whole lookup");
    assert_eq!(children.len(), 2);
    let healthy_row = children.iter().find(|c| c.id == healthy).unwrap();
    assert_eq!(
        healthy_row.process_start_time,
        Some(52_755_192),
        "the healthy sibling must still be readable"
    );
    let broken_row = children.iter().find(|c| c.id == broken).unwrap();
    assert_eq!(
        broken_row.process_start_time, None,
        "only the malformed row degrades — it simply cannot spare (D-3)"
    );
}

#[test]
fn test_find_dispatch_children_with_pid_no_children_is_empty() {
    let db = db();
    let parent = create_phantom_tracking_row(
        &db,
        "mika",
        "ready-label: mika#2140 — fiche vraiment orpheline",
        "in_progress",
        7200,
    );
    assert!(
        db.find_dispatch_children_with_pid(&parent)
            .unwrap()
            .is_empty(),
        "a genuine orphan has no dispatch child — the sweeper's reason to exist"
    );
}

#[test]
fn test_find_phantom_tracking_tasks_matching_row_returned() {
    let db = db();
    let id = create_phantom_tracking_row(&db, "mika", "track mika#1583", "in_progress", 3700);

    let phantoms = db.find_phantom_tracking_tasks("mika", 3600).unwrap();
    assert_eq!(phantoms.len(), 1);
    assert_eq!(phantoms[0].id, id);
    assert_eq!(phantoms[0].agent_id, "mika");
    assert_eq!(phantoms[0].status, "in_progress");
    assert_eq!(phantoms[0].label, "track mika#1583");
}

#[test]
fn test_find_phantom_tracking_tasks_blocked_status_returned() {
    let db = db();
    create_phantom_tracking_row(&db, "mika", "blocked track", "blocked", 3700);

    let phantoms = db.find_phantom_tracking_tasks("mika", 3600).unwrap();
    assert_eq!(phantoms.len(), 1);
    assert_eq!(phantoms[0].status, "blocked");
}

#[test]
fn test_find_phantom_tracking_tasks_mismatched_action_type_excluded() {
    let db = db();
    // action_type='send_message' should NOT match the phantom shape
    let task = new_task("mika", "reminder", "manual", "send_message");
    let id = db.create_task(&task).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress',
                 process_id = NULL,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-3700 seconds')
                 WHERE id = ?1",
            params![id],
        )
        .unwrap();

    let phantoms = db.find_phantom_tracking_tasks("mika", 3600).unwrap();
    assert!(
        phantoms.is_empty(),
        "action_type != 'none' should be excluded"
    );
}

#[test]
fn test_find_phantom_tracking_tasks_non_null_process_id_excluded() {
    let db = db();
    let id = create_phantom_tracking_row(&db, "mika", "live track", "in_progress", 3700);
    db.conn
        .execute(
            "UPDATE tasks SET process_id = 12345 WHERE id = ?1",
            params![id],
        )
        .unwrap();

    let phantoms = db.find_phantom_tracking_tasks("mika", 3600).unwrap();
    assert!(
        phantoms.is_empty(),
        "non-NULL process_id should be excluded (watchdog owns that path)"
    );
}

#[test]
fn test_find_phantom_tracking_tasks_terminal_status_excluded() {
    let db = db();
    let id = create_phantom_tracking_row(&db, "mika", "done track", "in_progress", 3700);
    db.conn
        .execute(
            "UPDATE tasks SET status = 'completed' WHERE id = ?1",
            params![id],
        )
        .unwrap();

    let phantoms = db.find_phantom_tracking_tasks("mika", 3600).unwrap();
    assert!(
        phantoms.is_empty(),
        "terminal-status rows should be excluded"
    );
}

/// T-5 (2026-08-21): defensively verify every non-`in_progress`/`blocked`
/// status is excluded by the SQL predicate. Guards against a
/// well-meaning refactor that changes `IN ('in_progress','blocked')` to
/// `NOT IN (terminal_statuses)` — which would sweep `pending` rows (out
/// of scope per plan §7 D1 + ADV-2 deferral to mika#1934) and any other
/// non-terminal status the schema ever adds.
#[test]
fn test_find_phantom_tracking_tasks_all_non_matching_statuses_excluded() {
    // pending: newly-created tracking row awaiting agent transition — must NOT be swept.
    // failed/cancelled/expired: already terminal — no-op even if matched.
    // delivered: callback-lifecycle status — inapplicable, must not be selected.
    for status in ["pending", "failed", "cancelled", "expired", "delivered"] {
        let db = db();
        create_phantom_tracking_row(&db, "mika", "shape check", status, 7200);
        let phantoms = db.find_phantom_tracking_tasks("mika", 3600).unwrap();
        assert!(
            phantoms.is_empty(),
            "status '{status}' must be excluded from the sweep predicate — \
                 SQL is `status IN ('in_progress', 'blocked')` per mika#1712 AC3"
        );
    }
}

#[test]
fn test_find_phantom_tracking_tasks_age_guard_fresh_row_not_swept() {
    let db = db();
    // updated_at = now (age 0); grace = 60s -> should NOT match
    create_phantom_tracking_row(&db, "mika", "fresh", "in_progress", 0);

    let phantoms = db.find_phantom_tracking_tasks("mika", 60).unwrap();
    assert!(
        phantoms.is_empty(),
        "row within grace window should not be selected"
    );
}

#[test]
fn test_find_phantom_tracking_tasks_age_guard_aged_row_swept() {
    let db = db();
    // Row aged 3700s (past 3600s grace) -> should match
    create_phantom_tracking_row(&db, "mika", "aged", "in_progress", 3700);

    let phantoms = db.find_phantom_tracking_tasks("mika", 3600).unwrap();
    assert_eq!(phantoms.len(), 1);
}

#[test]
fn test_find_phantom_tracking_tasks_age_zero_selects_any_age() {
    let db = db();
    // Fresh row (age 0) MUST be selected when age_seconds=0 (AC5 startup)
    create_phantom_tracking_row(&db, "mika", "fresh", "in_progress", 0);
    // Aged row also selected
    create_phantom_tracking_row(&db, "mika", "aged", "blocked", 7200);

    let phantoms = db.find_phantom_tracking_tasks("mika", 0).unwrap();
    assert_eq!(
        phantoms.len(),
        2,
        "age_seconds=0 (AC5 startup sweep) must select all matching rows"
    );
}

#[test]
fn test_find_phantom_tracking_tasks_excludes_other_agents() {
    let db = db();
    db.register_agent("agent_a", "Agent A", "/tmp/a").unwrap();
    db.register_agent("agent_b", "Agent B", "/tmp/b").unwrap();

    create_phantom_tracking_row(&db, "agent_a", "a-only", "in_progress", 3700);

    let a_phantoms = db.find_phantom_tracking_tasks("agent_a", 3600).unwrap();
    assert_eq!(a_phantoms.len(), 1);
    let b_phantoms = db.find_phantom_tracking_tasks("agent_b", 3600).unwrap();
    assert!(b_phantoms.is_empty());
}

#[test]
fn test_find_orphaned_pending_selects_qualifying() {
    let db = db();
    let parent_id = create_pending_issue_parent(&db, 2013, 3600);

    let orphans = db
        .find_orphaned_pending_issue_tasks("mika", 2700, 2700)
        .unwrap();
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0].id, parent_id);
    assert_eq!(
        orphans[0].reference_url,
        "https://github.com/x/y/issues/2013"
    );
    assert!(orphans[0].age_seconds >= 3600);
    assert_eq!(orphans[0].rearm_count, 0);
}

/// R2 anti-vacuity: a task waiting behind a busy dispatch slot is old AND
/// healthy. Age alone would kill it. This test fails if the wrapper clause
/// is dropped from the predicate.
#[test]
fn test_find_orphaned_pending_excludes_task_with_pending_wrapper() {
    let db = db();
    let parent_id = create_pending_issue_parent(&db, 2013, 3600);
    attach_deferred_wrapper(&db, &parent_id, "pending");

    let orphans = db
        .find_orphaned_pending_issue_tasks("mika", 2700, 2700)
        .unwrap();
    assert!(
        orphans.is_empty(),
        "a task with a pending deferred wrapper is queued, not orphaned"
    );
}

/// The wrapper was promoted (`completed`) and its turn never dispatched —
/// exactly the mika#2045 shape.
///
/// AC3 (mika#2181), non-regression: this is the case the reaper exists to
/// catch, and it must survive the widened predicate. It does, and it now
/// also covers the fail-safe branch: `attach_deferred_wrapper` leaves
/// `completed_at` NULL, and a wrapper that cannot prove it is recent is not
/// counted as live. The body is deliberately unchanged.
#[test]
fn test_find_orphaned_pending_selects_when_wrapper_was_consumed() {
    let db = db();
    let parent_id = create_pending_issue_parent(&db, 2013, 3600);
    attach_deferred_wrapper(&db, &parent_id, "completed");

    let orphans = db
        .find_orphaned_pending_issue_tasks("mika", 2700, 2700)
        .unwrap();
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0].id, parent_id);
}

/// Second anti-vacuity clause: the real dispatch is in flight. Without the
/// non-deferred `NOT EXISTS`, this parent is classified orphaned during the
/// window before it reaches `in_progress`.
#[test]
fn test_find_orphaned_pending_excludes_task_with_active_real_callback() {
    let db = db();
    let parent_id = create_pending_issue_parent(&db, 2013, 3600);
    attach_real_callback(&db, &parent_id, "in_progress");

    let orphans = db
        .find_orphaned_pending_issue_tasks("mika", 2700, 2700)
        .unwrap();
    assert!(
        orphans.is_empty(),
        "a task whose real dispatch is in flight is not orphaned"
    );
}

#[test]
fn test_find_orphaned_pending_excludes_younger_than_grace() {
    let db = db();
    create_pending_issue_parent(&db, 2013, 600);

    let orphans = db
        .find_orphaned_pending_issue_tasks("mika", 2700, 2700)
        .unwrap();
    assert!(orphans.is_empty(), "10 minutes is inside the normal window");
}

#[test]
fn test_find_orphaned_pending_excludes_non_pending_status() {
    let db = db();
    let parent_id = create_pending_issue_parent(&db, 2013, 3600);
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
            params![parent_id],
        )
        .unwrap();

    let orphans = db
        .find_orphaned_pending_issue_tasks("mika", 2700, 2700)
        .unwrap();
    assert!(
        orphans.is_empty(),
        "in_progress parents belong to the #871/#1687 reapers"
    );
}

#[test]
fn test_find_orphaned_pending_excludes_other_agents() {
    let db = db();
    db.register_agent("agent_b", "Agent B", "/tmp/b").unwrap();
    create_pending_issue_parent(&db, 2013, 3600);

    assert!(
        db.find_orphaned_pending_issue_tasks("agent_b", 2700, 2700)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        db.find_orphaned_pending_issue_tasks("mika", 2700, 2700)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn test_find_orphaned_pending_reads_rearm_count_from_metadata() {
    let db = db();
    let parent_id = create_pending_issue_parent(&db, 2013, 3600);
    db.set_task_metadata_field(&parent_id, "stuck_rearm_count", "2")
        .unwrap();

    let orphans = db
        .find_orphaned_pending_issue_tasks("mika", 2700, 2700)
        .unwrap();
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0].rearm_count, 2);
}

/// Tolerant read: unreadable metadata must not raise, it reads as 0.
#[test]
fn test_find_orphaned_pending_tolerates_malformed_metadata() {
    let db = db();
    let parent_id = create_pending_issue_parent(&db, 2013, 3600);
    db.conn
        .execute(
            "UPDATE tasks SET metadata = 'not json at all' WHERE id = ?1",
            params![parent_id],
        )
        .unwrap();

    let orphans = db
        .find_orphaned_pending_issue_tasks("mika", 2700, 2700)
        .unwrap();
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0].rearm_count, 0);
}

/// Backs the `loop_stuck_pending_tasks` event and the probe: several stuck
/// issues are reported together, each with its own url and age.
#[test]
fn test_find_orphaned_pending_reports_every_stuck_issue() {
    let db = db();
    create_pending_issue_parent(&db, 2013, 3600);
    create_pending_issue_parent(&db, 1887, 4200);
    // A healthy neighbour must not inflate the count.
    let queued = create_pending_issue_parent(&db, 1664, 3600);
    attach_deferred_wrapper(&db, &queued, "pending");

    let orphans = db
        .find_orphaned_pending_issue_tasks("mika", 2700, 2700)
        .unwrap();
    assert_eq!(orphans.len(), 2);
    let urls: Vec<&str> = orphans.iter().map(|o| o.reference_url.as_str()).collect();
    assert!(urls.contains(&"https://github.com/x/y/issues/2013"));
    assert!(urls.contains(&"https://github.com/x/y/issues/1887"));
    assert!(!urls.contains(&"https://github.com/x/y/issues/1664"));
}

// -- mika#2181: a promoted wrapper is still alive --

/// AC2 — verbatim replay of the mika#2181 trace.
///
/// Parent `pending` born 12:10:08Z, reaper tick at 15:31:03Z: age 11 455 s,
/// well past the 2700 s grace, so it is a candidate *by age*. Its deferred
/// wrapper `f5eebf48` was promoted at that very tick — `completed`,
/// `completed_at` = now, never `delivered`. The turn answered at 15:34:43Z,
/// three and a half minutes later.
///
/// On `main` the narrow `status = 'pending'` clause calls this parent
/// unrepresented and the reaper re-arms it; two ticks later the re-arm budget
/// is spent and a healthy parent is `failed`. With the fix the parent is not
/// returned at all.
#[test]
fn test_find_orphaned_pending_excludes_parent_whose_wrapper_was_just_promoted() {
    let db = db();
    let parent_id = create_pending_issue_parent(&db, 2158, 11_455);
    attach_deferred_wrapper_at(&db, &parent_id, "completed", 0);

    let orphans = db
        .find_orphaned_pending_issue_tasks("mika", 2700, 2700)
        .unwrap();
    assert!(
        orphans.is_empty(),
        "a wrapper promoted this tick still represents its parent — the turn \
             it feeds has not had a chance to answer yet (mika#2181)"
    );
}

/// The liveness window is **bounded**, and this test is the guard that keeps
/// it that way (mika#2181).
///
/// On the silent-turn error path the wrapper is re-armed but never marked
/// `delivered`, so it stays `completed` forever. If a future refactor drops
/// the `completed_at` bound "to simplify", that corpse becomes a permanent
/// shield and the reaper never touches the parent again. Here the promotion
/// is 3000 s old against a 2700 s window: dead, and the parent is returned.
#[test]
fn test_find_orphaned_pending_selects_when_promoted_wrapper_is_stale() {
    let db = db();
    let parent_id = create_pending_issue_parent(&db, 2158, 11_455);
    attach_deferred_wrapper_at(&db, &parent_id, "completed", 3000);

    let orphans = db
        .find_orphaned_pending_issue_tasks("mika", 2700, 2700)
        .unwrap();
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0].id, parent_id);
}

/// AC3 to the letter (mika#2181): a `delivered` wrapper is a consumed one.
///
/// The pre-existing `..._when_wrapper_was_consumed` test carries `completed`,
/// not `delivered`, so the exact wording of AC3 had no test. `delivered` must
/// never count as live however recent it is — the turn is over and nothing
/// dispatched.
#[test]
fn test_find_orphaned_pending_selects_when_wrapper_is_delivered() {
    let db = db();
    let parent_id = create_pending_issue_parent(&db, 2158, 11_455);
    attach_deferred_wrapper_at(&db, &parent_id, "delivered", 0);

    let orphans = db
        .find_orphaned_pending_issue_tasks("mika", 2700, 2700)
        .unwrap();
    assert_eq!(
        orphans.len(),
        1,
        "a delivered wrapper is spent — its parent is orphaned"
    );
    assert_eq!(orphans[0].id, parent_id);
}

/// `failed` and `cancelled` wrappers are dead, not live, whatever their
/// `completed_at` says (mika#2181). This is why the widened clause names
/// `completed` explicitly instead of `status NOT IN ('delivered', ...)`.
#[test]
fn test_find_orphaned_pending_selects_when_wrapper_is_failed_or_cancelled() {
    for status in ["failed", "cancelled"] {
        let db = db();
        let parent_id = create_pending_issue_parent(&db, 2158, 11_455);
        attach_deferred_wrapper_at(&db, &parent_id, status, 0);

        let orphans = db
            .find_orphaned_pending_issue_tasks("mika", 2700, 2700)
            .unwrap();
        assert_eq!(orphans.len(), 1, "a {status} wrapper is dead, not live");
    }
}

/// The two windows must not be transposable (mika#2181).
///
/// Every other call site passes `(2700, 2700)`, so a swap of `grace_seconds`
/// and `promoted_liveness_seconds` — in the signature, in the `params!`
/// binding, or at a caller — would pass the entire suite. This is the one
/// test that separates them: the same fixture must give opposite answers
/// under the two orderings.
#[test]
fn test_find_orphaned_pending_distinguishes_grace_from_liveness() {
    let db = db();
    let parent_id = create_pending_issue_parent(&db, 2158, 3600);
    attach_deferred_wrapper_at(&db, &parent_id, "completed", 1200);

    // Promotion 1200 s ago, liveness 3600 s: the wrapper is still live, so
    // the parent is sheltered.
    assert!(
        db.find_orphaned_pending_issue_tasks("mika", 2700, 3600)
            .unwrap()
            .is_empty(),
        "grace 2700 / liveness 3600 must shelter a wrapper promoted 1200 s ago"
    );

    // Same fixture, windows swapped: liveness 2700 still shelters it, so a
    // transposition is only detectable with a liveness SHORTER than the
    // promotion age. 300 s expires the shield; the parent is orphaned.
    assert_eq!(
        db.find_orphaned_pending_issue_tasks("mika", 2700, 300)
            .unwrap()
            .len(),
        1,
        "grace 2700 / liveness 300 must NOT shelter a wrapper promoted 1200 s ago"
    );

    // And the mirror: swapping the arguments of the sheltering call flips
    // its answer, which is exactly what a transposed binding would do.
    assert_eq!(
        db.find_orphaned_pending_issue_tasks("mika", 300, 2700)
            .unwrap()
            .len(),
        0,
        "grace 300 / liveness 2700 shelters — the two parameters are not interchangeable"
    );
}

/// The window's edge is `>`, strictly (mika#2181). At exactly the boundary a
/// wrapper is NOT live, and that direction is the safe one: an off-by-one
/// here costs one tick of detection latency, never a wrongly-sheltered
/// parent. Only the exclusive side is pinned — `-2699` would race the
/// fixture and the query into the same second and flake.
#[test]
fn test_find_orphaned_pending_liveness_boundary_is_exclusive() {
    let db = db();
    let parent_id = create_pending_issue_parent(&db, 2158, 11_455);
    attach_deferred_wrapper_at(&db, &parent_id, "completed", 2700);

    let orphans = db
        .find_orphaned_pending_issue_tasks("mika", 2700, 2700)
        .unwrap();
    assert_eq!(
        orphans.len(),
        1,
        "completed_at == now - liveness is not `>` — the wrapper is spent"
    );
    assert_eq!(orphans[0].id, parent_id);
}

/// A `pending` wrapper is live regardless of `completed_at` (mika#2181).
///
/// The first disjunct must not be made conditional on the timestamp by a
/// later edit: a wrapper still waiting for its slot is represented however
/// old any stale timestamp on the row happens to be.
#[test]
fn test_find_orphaned_pending_shelters_pending_wrapper_with_stale_completed_at() {
    let db = db();
    let parent_id = create_pending_issue_parent(&db, 2158, 11_455);
    attach_deferred_wrapper_at(&db, &parent_id, "pending", 999_999);

    assert!(
        db.find_orphaned_pending_issue_tasks("mika", 2700, 2700)
            .unwrap()
            .is_empty(),
        "a pending wrapper is live on its status alone"
    );
}

/// D7's whole reason to exist: `has_live_deferred_wrapper_child` and clause
/// (1) of `find_orphaned_pending_issue_tasks` answer the SAME question, and
/// the repo's own rule (docs/solutions/.../asymmetric-perimeter-predicate-drift)
/// says a deliberate fork ships its parity test in the same commit. The two
/// clauses are hand-synced SQL; this is what makes a one-sided edit fail.
#[test]
fn test_live_wrapper_predicate_agrees_with_orphan_clause() {
    // (wrapper status, completed_at offset or None) -> is the wrapper live?
    let cases: &[(&str, Option<i64>)] = &[
        ("pending", None),
        ("pending", Some(999_999)),
        ("completed", Some(0)),
        ("completed", Some(1200)),
        ("completed", Some(3000)),
        ("completed", None),
        ("delivered", Some(0)),
        ("failed", Some(0)),
        ("cancelled", Some(0)),
    ];

    for (status, offset) in cases {
        let db = db();
        let parent_id = create_pending_issue_parent(&db, 2158, 11_455);
        match offset {
            Some(o) => attach_deferred_wrapper_at(&db, &parent_id, status, *o),
            None => attach_deferred_wrapper(&db, &parent_id, status),
        };

        let twin_says_live = db
            .has_live_deferred_wrapper_child("mika", &parent_id, 2700)
            .unwrap();
        // The parent is orphaned exactly when no wrapper shelters it.
        let clause_says_live = db
            .find_orphaned_pending_issue_tasks("mika", 2700, 2700)
            .unwrap()
            .is_empty();

        assert_eq!(
            twin_says_live, clause_says_live,
            "predicates diverged on ({status}, {offset:?}): \
                 has_live_deferred_wrapper_child={twin_says_live}, \
                 orphan clause sheltered={clause_says_live}"
        );
    }
}

#[test]
fn test_has_live_deferred_wrapper_child() {
    let db = db();
    let parent_id = create_pending_issue_parent(&db, 2013, 100);
    assert!(
        !db.has_live_deferred_wrapper_child("mika", &parent_id, 2700)
            .unwrap()
    );

    let wrapper_id = attach_deferred_wrapper(&db, &parent_id, "pending");
    assert!(
        db.has_live_deferred_wrapper_child("mika", &parent_id, 2700)
            .unwrap()
    );

    // Promotion writes `completed` + `completed_at`. The wrapper is still
    // live: the silent turn it feeds has not returned yet (mika#2181).
    db.conn
        .execute(
            "UPDATE tasks SET status = 'completed',
                        completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                 WHERE id = ?1",
            params![wrapper_id],
        )
        .unwrap();
    assert!(
        db.has_live_deferred_wrapper_child("mika", &parent_id, 2700)
            .unwrap(),
        "a wrapper promoted inside the window still represents its parent"
    );

    // Past the window it is a corpse, not a shield.
    db.conn
        .execute(
            "UPDATE tasks SET completed_at =
                        strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-3000 seconds')
                 WHERE id = ?1",
            params![wrapper_id],
        )
        .unwrap();
    assert!(
        !db.has_live_deferred_wrapper_child("mika", &parent_id, 2700)
            .unwrap()
    );

    // `delivered` is spent, however recent.
    db.conn
        .execute(
            "UPDATE tasks SET status = 'delivered',
                        completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                 WHERE id = ?1",
            params![wrapper_id],
        )
        .unwrap();
    assert!(
        !db.has_live_deferred_wrapper_child("mika", &parent_id, 2700)
            .unwrap()
    );
}

/// mika#2413 — the boolean sibling is the exclusion-free case of the finder, so
/// the two cannot drift. Asserted over the same status/age matrix the parity
/// test above uses, because a delegation that silently stopped delegating would
/// reopen exactly the fork `has_live_deferred_wrapper_child`'s own doc warns
/// about.
#[test]
fn mika2413_the_boolean_sibling_is_the_finder_without_an_exclusion() {
    let cases: &[(&str, Option<i64>)] = &[
        ("pending", None),
        ("completed", Some(0)),
        ("completed", Some(3000)),
        ("delivered", Some(0)),
        ("cancelled", Some(0)),
    ];

    for (status, offset) in cases {
        let db = db();
        let parent_id = create_pending_issue_parent(&db, 2413, 11_455);
        match offset {
            Some(o) => attach_deferred_wrapper_at(&db, &parent_id, status, *o),
            None => attach_deferred_wrapper(&db, &parent_id, status),
        };

        assert_eq!(
            db.has_live_deferred_wrapper_child("mika", &parent_id, 2700)
                .unwrap(),
            db.find_live_deferred_wrapper_child("mika", &parent_id, 2700, None)
                .unwrap()
                .is_some(),
            "the two entry points diverged on ({status}, {offset:?})"
        );
    }
}

/// mika#2413 — the exclusion takes exactly one wrapper out, and only that one.
///
/// This is what lets `rearm_deferred_callback` ask *"is this parent represented
/// by anything OTHER than the wrapper whose sterility I am treating?"*. Without
/// it, a wrapper still `completed` on the `silent_turn_error` path counts itself
/// as live and that repair path dies for the whole liveness window.
#[test]
fn mika2413_the_exclusion_removes_only_the_named_wrapper() {
    let db = db();
    let parent_id = create_pending_issue_parent(&db, 2413, 100);
    let consumed = attach_deferred_wrapper(&db, &parent_id, "completed");
    db.conn
        .execute(
            "UPDATE tasks SET completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            params![consumed],
        )
        .unwrap();

    // On its own, the consumed wrapper would shelter its own parent.
    assert!(
        db.find_live_deferred_wrapper_child("mika", &parent_id, 2700, None)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        db.find_live_deferred_wrapper_child("mika", &parent_id, 2700, Some(&consumed))
            .unwrap(),
        None,
        "excluded, it shelters nothing"
    );

    // A genuine sibling is found, and is the one reported.
    let sibling = attach_deferred_wrapper(&db, &parent_id, "pending");
    assert_eq!(
        db.find_live_deferred_wrapper_child("mika", &parent_id, 2700, Some(&consumed))
            .unwrap(),
        Some(sibling.clone()),
        "the exclusion must not hide the other wrappers"
    );
    // And excluding the sibling instead brings the consumed one back.
    assert_eq!(
        db.find_live_deferred_wrapper_child("mika", &parent_id, 2700, Some(&sibling))
            .unwrap(),
        Some(consumed),
        "only the named id leaves the population"
    );
}

/// mika#2413 U3 — the reset clears a spent budget and writes nothing otherwise.
///
/// The `> 0` guard is what keeps the nominal dispatch free of a write and makes
/// the return value mean "there was something to clear", so the caller can log
/// only the resets that carry information.
#[test]
fn mika2413_reset_stuck_rearm_count_only_writes_when_there_is_something_to_clear() {
    let db = db();
    let parent_id = create_pending_issue_parent(&db, 2413, 100);

    assert!(
        !db.reset_stuck_rearm_count(&parent_id).unwrap(),
        "a counter already at zero is not a reset"
    );

    db.increment_stuck_rearm_count(&parent_id).unwrap();
    db.increment_stuck_rearm_count(&parent_id).unwrap();
    assert_eq!(db.get_stuck_rearm_count(&parent_id).unwrap(), 2);

    assert!(db.reset_stuck_rearm_count(&parent_id).unwrap());
    assert_eq!(db.get_stuck_rearm_count(&parent_id).unwrap(), 0);
    assert!(
        !db.reset_stuck_rearm_count(&parent_id).unwrap(),
        "the reset is idempotent"
    );

    // Unreadable metadata is left alone rather than replaced: `increment`
    // rebuilds a fresh object because it must still bound a corrupt row, but a
    // reset has nothing to bound and no reason to overwrite.
    db.conn
        .execute(
            "UPDATE tasks SET metadata = 'not json' WHERE id = ?1",
            params![parent_id],
        )
        .unwrap();
    assert!(!db.reset_stuck_rearm_count(&parent_id).unwrap());
}

/// AC4 (mika#2181): the audit rendering names every wrapper and its status,
/// oldest first, and says so plainly when there is none.
#[test]
fn test_summarize_deferred_wrappers_renders_ids_and_statuses() {
    let db = db();
    let parent_id = create_pending_issue_parent(&db, 2158, 11_455);
    assert_eq!(
        DeferredWrapperSummary::render(
            &db.summarize_deferred_wrappers_of_parent("mika", &parent_id)
                .unwrap()
        ),
        "wrappers:none"
    );

    let promoted = attach_deferred_wrapper_at(&db, &parent_id, "completed", 0);
    let queued = attach_deferred_wrapper(&db, &parent_id, "pending");

    let summaries = db
        .summarize_deferred_wrappers_of_parent("mika", &parent_id)
        .unwrap();
    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries[0].id, promoted, "oldest first");
    assert_eq!(summaries[0].status, "completed");
    assert!(summaries[0].completed_at.is_some());
    assert_eq!(summaries[1].id, queued);
    assert_eq!(summaries[1].status, "pending");
    assert_eq!(summaries[1].completed_at, None);

    // Pin the whole layout, not a substring. `contains("completed@2")`
    // passes for any string whose timestamp merely starts with a `2` — it
    // asserts the century, not the format.
    let promoted_at = summaries[0].completed_at.as_deref().unwrap();
    assert_eq!(
        DeferredWrapperSummary::render(&summaries),
        format!(
            "wrappers:{}:completed@{promoted_at},{}:pending@-",
            &promoted[..8],
            &queued[..8]
        )
    );
}

#[test]
fn mika2040_stamped_finished_dispatch_is_selected() {
    let db = db();
    let task_id = create_stamped_dispatch(&db, "mika", "delivered", 600);

    let found = db
        .find_dispatches_expecting_transcripts("mika", 300)
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].task_id, task_id);
    assert_eq!(found[0].status, "delivered");
    assert!(
        found[0]
            .expected_path
            .ends_with(&format!("{task_id}.jsonl")),
        "the detector must re-read the path the producer stamped, got {}",
        found[0].expected_path
    );
}

#[test]
fn mika2040_unstamped_dispatch_is_invisible_to_the_detector() {
    // The premise is a fact the producer records. A dispatch nobody asked a
    // transcript of must never be reported as having failed to produce one
    // — that is the whole reason the key is stamped rather than inferred
    // from the skill name plus the current gate.
    let db = db();
    let task_id = db.create_task(&callback_task("mika")).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'delivered',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-600 seconds')
                 WHERE id = ?1",
            params![task_id],
        )
        .unwrap();

    assert!(
        db.find_dispatches_expecting_transcripts("mika", 300)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn mika2040_unfinished_dispatch_is_not_selected() {
    // `pending` / `in_progress` mean the pilot may still be writing. The
    // finished boundary here must stay byte-for-byte the one the ingestion
    // uses, or the detector reports transcripts that are merely late.
    for status in ["pending", "in_progress"] {
        // A fresh DB per status: leaving the previous iteration's row in
        // place would let a passing assertion rest on the wrong row.
        let db = db();
        let _ = create_stamped_dispatch(&db, "mika", status, 600);
        assert!(
            db.find_dispatches_expecting_transcripts("mika", 300)
                .unwrap()
                .is_empty(),
            "a {status} dispatch must not be reported"
        );
    }
}

#[test]
fn mika2040_dispatch_inside_the_grace_window_is_not_selected() {
    let db = db();
    create_stamped_dispatch(&db, "mika", "delivered", 10);

    assert!(
        db.find_dispatches_expecting_transcripts("mika", 300)
            .unwrap()
            .is_empty(),
        "a dispatch that finished seconds ago may still have a file in flight"
    );
}

#[test]
fn mika2040_reported_dispatch_is_not_reported_twice() {
    let db = db();
    let task_id = create_stamped_dispatch(&db, "mika", "delivered", 600);
    assert_eq!(
        db.find_dispatches_expecting_transcripts("mika", 300)
            .unwrap()
            .len(),
        1
    );

    db.set_task_metadata_field(
        &task_id,
        crate::task_engine::engine::PILOT_TRANSCRIPT_REPORTED_KEY,
        "2026-09-08T10:00:00Z",
    )
    .unwrap();

    assert!(
        db.find_dispatches_expecting_transcripts("mika", 300)
            .unwrap()
            .is_empty(),
        "the warning fires once per dispatch, not once per scan"
    );
}

#[test]
fn mika2040_detector_is_agent_scoped() {
    let db = db();
    // Les tasks référencent agents(id) (FK NOT NULL) — créer les agents d'abord.
    db.register_agent("agent_a", "Agent A", "/tmp/a").unwrap();
    db.register_agent("agent_b", "Agent B", "/tmp/b").unwrap();
    create_stamped_dispatch(&db, "agent_a", "delivered", 600);

    assert_eq!(
        db.find_dispatches_expecting_transcripts("agent_a", 300)
            .unwrap()
            .len(),
        1
    );
    assert!(
        db.find_dispatches_expecting_transcripts("agent_b", 300)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn mika2040_one_corrupt_metadata_row_does_not_blind_the_detector() {
    // `json_extract` raises a hard error on non-JSON metadata, and that
    // error propagates out of the whole query_map. Unguarded, ONE bad row
    // would silence the detector for every other dispatch — the exact
    // failure class mika#2040 exists to end.
    let db = db();
    let good = create_stamped_dispatch(&db, "mika", "delivered", 600);

    let corrupt = db.create_task(&callback_task("mika")).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'delivered', metadata = 'not json at all',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-600 seconds')
                 WHERE id = ?1",
            params![corrupt],
        )
        .unwrap();

    let found = db
        .find_dispatches_expecting_transcripts("mika", 300)
        .unwrap();
    assert_eq!(found.len(), 1, "the corrupt row drops out, alone");
    assert_eq!(found[0].task_id, good);
}

#[test]
fn mika2040_ingested_transcript_is_countable_for_its_task() {
    // AC4, end of the reader chain: a v1 line parses, inserts, and is
    // findable by the task id the detector keys on.
    let mut db = db();
    let task_id = db.create_task(&callback_task("mika")).unwrap();

    let row =
        crate::task_engine::pilot_transcript::parse_pilot_transcript_line(&serde_json::json!({
            "schema_version": "v1",
            "timestamp": "2026-09-07T12:00:00Z",
            "model": "claude-sonnet-4-6",
            "response_body": "hello",
            "tokens_out": 7,
        }))
        .expect("v1 line must parse");

    assert_eq!(
        db.insert_pilot_transcripts_batch(&task_id, &[row]).unwrap(),
        1
    );
    assert_eq!(db.count_pilot_transcripts_for_task(&task_id).unwrap(), 1);
}

#[test]
fn test_find_childless_stuck_parent_selects_qualifying() {
    let db = db();
    let parent_id = create_childless_stuck_parent(&db, 2000);

    let stuck = db.find_childless_stuck_parent_tasks("mika", 1800).unwrap();
    assert_eq!(stuck.len(), 1);
    assert_eq!(stuck[0].id, parent_id);
    assert_eq!(stuck[0].agent_id, "mika");
    assert!(!stuck[0].created_at.is_empty());
    assert!(!stuck[0].updated_at.is_empty());
}

#[test]
fn test_find_childless_stuck_parent_excludes_parent_with_any_child() {
    let db = db();
    let parent_id = create_childless_stuck_parent(&db, 2000);

    // Add a callback child that is merely `pending` (not delivered). The
    // NOT EXISTS predicate excludes on ANY child row — this is the exact
    // complement of the orphan reaper's `delivered`-child INNER JOIN.
    let mut child = callback_task("mika");
    child.parent_task_id = Some(parent_id.clone());
    db.create_task(&child).unwrap();

    let stuck = db.find_childless_stuck_parent_tasks("mika", 1800).unwrap();
    assert!(
        stuck.is_empty(),
        "parent with any child (even pending) must not be reaped"
    );
}

#[test]
fn test_find_childless_stuck_parent_excludes_younger_than_grace() {
    let db = db();
    create_childless_stuck_parent(&db, 100);

    let stuck = db.find_childless_stuck_parent_tasks("mika", 1800).unwrap();
    assert!(
        stuck.is_empty(),
        "parent younger than grace must not be reaped"
    );
}

#[test]
fn test_find_childless_stuck_parent_excludes_milestone_and_project() {
    let db = db();

    for task_type in ["milestone", "project"] {
        let mut parent = new_task("mika", "Milestone parent", "manual", "none");
        parent.source = Some("self_dev".to_string());
        parent.r#type = Some(task_type.to_string());
        let parent_id = db.create_task(&parent).unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'in_progress',
                     updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-2000 seconds')
                     WHERE id = ?1",
                params![parent_id],
            )
            .unwrap();
    }

    let stuck = db.find_childless_stuck_parent_tasks("mika", 1800).unwrap();
    assert!(
        stuck.is_empty(),
        "milestone/project parents are out of scope for v1 (D2)"
    );
}

#[test]
fn test_find_childless_stuck_parent_excludes_wrong_status_source_trigger() {
    let db = db();

    // (a) pending (never reached in_progress): backdate but leave pending.
    let mut pending = new_task("mika", "Pending", "manual", "none");
    pending.source = Some("self_dev".to_string());
    let pending_id = db.create_task(&pending).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-2000 seconds')
                 WHERE id = ?1",
            params![pending_id],
        )
        .unwrap();

    // (b) non-self_dev source, in_progress, aged.
    let mut other_source = new_task("mika", "Other source", "manual", "none");
    other_source.source = Some("webhook".to_string());
    let other_source_id = db.create_task(&other_source).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-2000 seconds')
                 WHERE id = ?1",
            params![other_source_id],
        )
        .unwrap();

    // (c) non-manual trigger (recurring), self_dev, in_progress, aged.
    let mut recurring = new_task("mika", "Recurring", "recurring", "none");
    recurring.source = Some("self_dev".to_string());
    let recurring_id = db.create_task(&recurring).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-2000 seconds')
                 WHERE id = ?1",
            params![recurring_id],
        )
        .unwrap();

    let stuck = db.find_childless_stuck_parent_tasks("mika", 1800).unwrap();
    assert!(
        stuck.is_empty(),
        "pending / non-self_dev / non-manual parents must not be reaped"
    );
}

#[test]
fn test_find_childless_stuck_parent_excludes_other_agents() {
    let db = db();
    db.register_agent("agent_a", "Agent A", "/tmp/a").unwrap();
    db.register_agent("agent_b", "Agent B", "/tmp/b").unwrap();

    let mut parent = new_task("agent_a", "Implement task", "manual", "none");
    parent.source = Some("self_dev".to_string());
    let parent_id = db.create_task(&parent).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-2000 seconds')
                 WHERE id = ?1",
            params![parent_id],
        )
        .unwrap();

    // agent_b sees nothing; agent_a sees its own stuck parent.
    assert!(
        db.find_childless_stuck_parent_tasks("agent_b", 1800)
            .unwrap()
            .is_empty()
    );
    let stuck = db
        .find_childless_stuck_parent_tasks("agent_a", 1800)
        .unwrap();
    assert_eq!(stuck.len(), 1);
    assert_eq!(stuck[0].id, parent_id);
}

/// Terminal-state race: once the childless parent is transitioned out of
/// `in_progress`, the guarded `update_task_failed` no-ops (Ok(false)) and
/// the query stops selecting it — the reaper never double-writes (R7).
#[test]
fn test_find_childless_stuck_parent_terminal_state_race() {
    let db = db();
    let parent_id = create_childless_stuck_parent(&db, 2000);

    // First reap succeeds.
    assert!(
        db.update_task_failed(&parent_id, "mika", "stuck_in_progress_no_callback_child")
            .unwrap()
    );

    // No longer selected (left in_progress).
    assert!(
        db.find_childless_stuck_parent_tasks("mika", 1800)
            .unwrap()
            .is_empty()
    );

    // A second guarded write no-ops rather than overwriting.
    assert!(
        !db.update_task_failed(&parent_id, "mika", "should not overwrite")
            .unwrap()
    );
    let task = db.get_task(&parent_id, "mika").unwrap().unwrap();
    assert_eq!(task.status, "failed");
    assert_eq!(
        task.result.as_deref(),
        Some("stuck_in_progress_no_callback_child")
    );
}

/// mika#1126 — get_reaper_child_snapshot returns ALL children of a parent.
#[test]
fn test_get_reaper_child_snapshot_returns_all_children() {
    let db = db();
    let (parent_id, child_a_id) = create_orphaned_parent_setup(&db);

    // Add a second groom-class child
    let mut child_b = callback_task("mika");
    child_b.parent_task_id = Some(parent_id.clone());
    child_b.dispatch_class = Some("groom".to_string());
    let child_b_id = db.create_task(&child_b).unwrap();

    let snapshot = db.get_reaper_child_snapshot(&parent_id).unwrap();
    assert_eq!(snapshot.len(), 2, "snapshot should return all children");

    let ids: Vec<&str> = snapshot.iter().map(|s| s.id.as_str()).collect();
    assert!(ids.contains(&child_a_id.as_str()));
    assert!(ids.contains(&child_b_id.as_str()));

    // Verify the groom child has dispatch_class populated
    let groom_child = snapshot.iter().find(|s| s.id == child_b_id).unwrap();
    assert_eq!(groom_child.dispatch_class.as_deref(), Some("groom"));
    assert_eq!(groom_child.trigger_type, "callback");
    assert_eq!(groom_child.action_type, "resume_agent");
}

#[test]
fn test_find_completable_parent_tasks_happy_path() {
    let db = db();
    let pr_url = "https://github.com/owner/repo/pull/1234";
    let (parent_id, child_id) = create_completable_parent_setup(&db, pr_url);

    // Backdate the child's updated_at past the grace period
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    let candidates = db
        .find_completable_parent_tasks_on_pr_url("mika", 600)
        .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].id, parent_id);
    assert_eq!(candidates[0].callback_task_id, child_id);
    assert_eq!(candidates[0].pr_url, pr_url);
}

#[test]
fn test_find_completable_parent_tasks_no_pr_url_excluded() {
    // Mirror of the reaper's happy path — when pr_url is absent the
    // completer must NOT match. (The reaper handles that case.)
    let db = db();
    let (_parent_id, child_id) = create_orphaned_parent_setup(&db);

    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    let candidates = db
        .find_completable_parent_tasks_on_pr_url("mika", 600)
        .unwrap();
    assert!(
        candidates.is_empty(),
        "parent without pr_url is reaper territory, not completer"
    );
}

#[test]
fn test_find_completable_parent_tasks_empty_pr_url_excluded() {
    let db = db();
    let (_parent_id, child_id) = create_completable_parent_setup(&db, "");

    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    let candidates = db
        .find_completable_parent_tasks_on_pr_url("mika", 600)
        .unwrap();
    assert!(
        candidates.is_empty(),
        "empty pr_url string must not trip the completer"
    );
}

#[test]
fn test_find_completable_parent_tasks_grace_period_not_elapsed() {
    let db = db();
    let pr_url = "https://github.com/owner/repo/pull/1";
    let (_parent_id, _child_id) = create_completable_parent_setup(&db, pr_url);

    // Child was just delivered, well within the 600s grace window
    let candidates = db
        .find_completable_parent_tasks_on_pr_url("mika", 600)
        .unwrap();
    assert!(
        candidates.is_empty(),
        "parent within grace period should not be auto-completed yet"
    );
}

#[test]
fn test_find_completable_parent_tasks_child_in_progress_excluded() {
    let db = db();
    let pr_url = "https://github.com/owner/repo/pull/1";

    // Custom setup: parent ready, but child is `in_progress` (not delivered).
    let mut parent = new_task("mika", "Implement mika#1162", "manual", "none");
    parent.source = Some("self_dev".to_string());
    let parent_id = db.create_task(&parent).unwrap();
    let meta = format!(r#"{{"claude_pilot":{{"pr_url":"{pr_url}"}}}}"#);
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress', metadata = ?1 WHERE id = ?2",
            params![meta, parent_id],
        )
        .unwrap();

    let mut child = callback_task("mika");
    child.parent_task_id = Some(parent_id.clone());
    let child_id = db.create_task(&child).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    let candidates = db
        .find_completable_parent_tasks_on_pr_url("mika", 600)
        .unwrap();
    assert!(
        candidates.is_empty(),
        "child must be `delivered`, not `in_progress`"
    );
}

#[test]
fn test_find_completable_parent_tasks_groom_class_excluded() {
    // Defense-in-depth: groom-class callbacks don't emit `PR:` lines, but
    // the dispatch_class filter must still exclude them. Mirror of the
    // reaper's same-named guard.
    let db = db();
    let pr_url = "https://github.com/owner/repo/pull/1";
    let (_parent_id, child_id) = create_completable_parent_setup(&db, pr_url);

    db.conn
        .execute(
            "UPDATE tasks SET dispatch_class = 'groom' WHERE id = ?1",
            params![child_id],
        )
        .unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    let candidates = db
        .find_completable_parent_tasks_on_pr_url("mika", 600)
        .unwrap();
    assert!(
        candidates.is_empty(),
        "groom-class callbacks must not trip the success-side completer"
    );
}

#[test]
fn test_find_completable_parent_tasks_implement_class_matched() {
    let db = db();
    let pr_url = "https://github.com/owner/repo/pull/1";
    let (parent_id, child_id) = create_completable_parent_setup(&db, pr_url);

    db.conn
        .execute(
            "UPDATE tasks SET dispatch_class = 'implement' WHERE id = ?1",
            params![child_id],
        )
        .unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    let candidates = db
        .find_completable_parent_tasks_on_pr_url("mika", 600)
        .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].id, parent_id);
}

#[test]
fn test_find_completable_parent_tasks_null_dispatch_class_treated_as_implement() {
    let db = db();
    let pr_url = "https://github.com/owner/repo/pull/1";
    let (parent_id, child_id) = create_completable_parent_setup(&db, pr_url);

    // Helper leaves dispatch_class NULL (pre-v34 row shape)
    let class: Option<String> = db
        .conn
        .query_row(
            "SELECT dispatch_class FROM tasks WHERE id = ?1",
            params![child_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(class.is_none(), "test invariant: pre-v34 shape");

    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    let candidates = db
        .find_completable_parent_tasks_on_pr_url("mika", 600)
        .unwrap();
    assert_eq!(
        candidates.len(),
        1,
        "NULL dispatch_class must still match (COALESCE -> 'implement')"
    );
    assert_eq!(candidates[0].id, parent_id);
}

#[test]
fn test_find_completable_parent_tasks_parent_not_in_progress() {
    let db = db();
    let pr_url = "https://github.com/owner/repo/pull/1";
    let (parent_id, child_id) = create_completable_parent_setup(&db, pr_url);

    // Parent is already completed (race with the inline path).
    db.conn
        .execute(
            "UPDATE tasks SET status = 'completed' WHERE id = ?1",
            params![parent_id],
        )
        .unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    let candidates = db
        .find_completable_parent_tasks_on_pr_url("mika", 600)
        .unwrap();
    assert!(
        candidates.is_empty(),
        "parent not in `in_progress` must not be re-completed"
    );
}

#[test]
fn test_find_completable_parent_tasks_active_sibling_defers() {
    let db = db();
    let pr_url = "https://github.com/owner/repo/pull/1";
    let (parent_id, child_id) = create_completable_parent_setup(&db, pr_url);

    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    // Another callback child is still in_progress
    let mut sibling = callback_task("mika");
    sibling.parent_task_id = Some(parent_id.clone());
    let sibling_id = db.create_task(&sibling).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
            params![sibling_id],
        )
        .unwrap();

    let candidates = db
        .find_completable_parent_tasks_on_pr_url("mika", 600)
        .unwrap();
    assert!(
        candidates.is_empty(),
        "parent with active sibling callback should wait for sibling completion"
    );
}

#[test]
fn test_find_completable_parent_tasks_excludes_other_agents() {
    let db = db();
    db.register_agent("agent_a", "Agent A", "/tmp/a").unwrap();
    db.register_agent("agent_b", "Agent B", "/tmp/b").unwrap();

    let mut parent = new_task("agent_a", "Implement mika#1162", "manual", "none");
    parent.source = Some("self_dev".to_string());
    let parent_id = db.create_task(&parent).unwrap();
    let meta = r#"{"claude_pilot":{"pr_url":"https://github.com/x/y/pull/1"}}"#;
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress', metadata = ?1 WHERE id = ?2",
            params![meta, parent_id],
        )
        .unwrap();

    let mut child = NewTask {
        agent_id: "agent_a".to_string(),
        ..callback_task("agent_a")
    };
    child.parent_task_id = Some(parent_id.clone());
    let child_id = db.create_task(&child).unwrap();
    assert!(
        db.update_task_completed(&child_id, "agent_a", Some("done"))
            .unwrap()
    );
    assert!(db.mark_task_delivered(&child_id).unwrap());
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    // agent_b should see nothing
    let candidates = db
        .find_completable_parent_tasks_on_pr_url("agent_b", 600)
        .unwrap();
    assert!(candidates.is_empty());

    // agent_a should see the candidate
    let candidates = db
        .find_completable_parent_tasks_on_pr_url("agent_a", 600)
        .unwrap();
    assert_eq!(candidates.len(), 1);
}

#[test]
fn test_find_completable_parent_tasks_stale_parent_class_doesnt_drive_selection() {
    // mika#1162 v2 — symmetric to test_find_orphaned_parent_tasks_stale_parent_class_*.
    // mika#920 task-reuse pattern means a parent's dispatch_class can be
    // stale relative to the most recent child callback. The selection MUST
    // key off the CHILD's dispatch_class, not the parent's. Set parent class
    // to 'groom' (stale, would have caused false-negative if we keyed off
    // parent) and child to 'implement' (the fresh dispatch). Parent must be
    // selected.
    let db = db();
    let pr_url = "https://github.com/x/y/pull/1";
    let (parent_id, child_id) = create_completable_parent_setup(&db, pr_url);

    // Set PARENT class to 'groom' (stale data from a prior dispatch)
    db.conn
        .execute(
            "UPDATE tasks SET dispatch_class = 'groom' WHERE id = ?1",
            params![parent_id],
        )
        .unwrap();
    // Set CHILD class to 'implement' (the fresh per-dispatch authority)
    db.conn
        .execute(
            "UPDATE tasks SET dispatch_class = 'implement' WHERE id = ?1",
            params![child_id],
        )
        .unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    let candidates = db
        .find_completable_parent_tasks_on_pr_url("mika", 600)
        .unwrap();
    assert_eq!(
        candidates.len(),
        1,
        "stale parent class must not override the fresh child class — must select on child.dispatch_class"
    );
    assert_eq!(candidates[0].id, parent_id);
}

#[test]
fn test_find_completable_parent_tasks_returns_pr_url_field() {
    // Defense-in-depth: confirm the pr_url comes through unchanged from
    // json_extract. The engine completer relies on this for its audit log.
    let db = db();
    let pr_url = "https://github.com/senara-solutions/mika/pull/1158";
    let (_parent_id, child_id) = create_completable_parent_setup(&db, pr_url);

    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    let candidates = db
        .find_completable_parent_tasks_on_pr_url("mika", 600)
        .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].pr_url, pr_url);
}

#[test]
fn test_update_task_failed_guards_terminal_states() {
    let db = db();
    let id = db.create_task(&callback_task("mika")).unwrap();

    // Complete the task first
    assert!(db.update_task_completed(&id, "mika", Some("done")).unwrap());

    // Attempting to fail an already-completed task should return Ok(false)
    let updated = db
        .update_task_failed(&id, "mika", "should not overwrite")
        .unwrap();
    assert!(!updated);

    // Verify the task status is still 'completed' (not overwritten to 'failed')
    let task = db.get_task(&id, "mika").unwrap().unwrap();
    assert_eq!(task.status, "completed");
    assert_eq!(task.result.as_deref(), Some("done"));
}

#[test]
fn test_promote_task_completed_from_failed() {
    let db = db();
    let id = db.create_task(&callback_task("mika")).unwrap();

    // First fail the task
    assert!(
        db.update_task_failed(&id, "mika", "initial_failure")
            .unwrap()
    );
    let task = db.get_task(&id, "mika").unwrap().unwrap();
    assert_eq!(task.status, "failed");

    // Promote from failed → completed
    let promoted = db
        .promote_task_completed(&id, "mika", "retry_success (pr_url: https://example.com)")
        .unwrap();
    assert!(promoted);

    let task = db.get_task(&id, "mika").unwrap().unwrap();
    assert_eq!(task.status, "completed");
    assert_eq!(
        task.result.as_deref(),
        Some("retry_success (pr_url: https://example.com)")
    );
    assert!(task.completed_at.is_some());
}

#[test]
fn test_promote_task_completed_noop_for_non_failed() {
    let db = db();
    let id = db.create_task(&callback_task("mika")).unwrap();

    // Task starts in pending — promote should be a no-op
    let promoted = db
        .promote_task_completed(&id, "mika", "should not fire")
        .unwrap();
    assert!(!promoted);
    let task = db.get_task(&id, "mika").unwrap().unwrap();
    assert_eq!(task.status, "pending");

    // Complete the task — promote should still be a no-op
    assert!(db.update_task_completed(&id, "mika", Some("done")).unwrap());
    let promoted = db
        .promote_task_completed(&id, "mika", "should not fire")
        .unwrap();
    assert!(!promoted);
    let task = db.get_task(&id, "mika").unwrap().unwrap();
    assert_eq!(task.status, "completed");
    assert_eq!(task.result.as_deref(), Some("done"));
}

#[test]
fn test_promote_task_completed_wrong_agent() {
    let db = db();
    let id = db.create_task(&callback_task("mika")).unwrap();
    assert!(db.update_task_failed(&id, "mika", "failure").unwrap());

    // Wrong agent_id — should return false
    let promoted = db
        .promote_task_completed(&id, "wrong-agent", "retry_success")
        .unwrap();
    assert!(!promoted);
    let task = db.get_task(&id, "mika").unwrap().unwrap();
    assert_eq!(task.status, "failed");
}

#[test]
fn test_promote_task_completed_nonexistent() {
    let db = db();
    let promoted = db
        .promote_task_completed("nonexistent-id", "mika", "retry_success")
        .unwrap();
    assert!(!promoted);
}

#[test]
fn test_is_unique_violation_only_catches_unique() {
    use rusqlite::ffi;

    // UNIQUE violation should match
    let unique_err = rusqlite::Error::SqliteFailure(
        ffi::Error::new(ffi::SQLITE_CONSTRAINT_UNIQUE),
        Some("UNIQUE constraint failed".to_string()),
    );
    assert!(is_unique_violation(&anyhow::Error::from(unique_err)));

    // NOT NULL violation should NOT match
    let notnull_err = rusqlite::Error::SqliteFailure(
        ffi::Error::new(ffi::SQLITE_CONSTRAINT_NOTNULL),
        Some("NOT NULL constraint failed".to_string()),
    );
    assert!(!is_unique_violation(&anyhow::Error::from(notnull_err)));
}

#[test]
fn test_trace_id_propagation() {
    let (db, sid) = db_with_session();
    let trace = "abcd1234abcd1234abcd1234abcd1234";

    // Save message with trace_id
    db.save_message("mika", &sid, "user", "traced msg", Some(trace))
        .unwrap();

    // Log audit event with trace_id
    db.log_audit_event(
        "mika",
        &sid,
        "store_fact",
        "person:Alice",
        None,
        Some("new"),
        None,
        Some(trace),
    )
    .unwrap();

    // Create task with trace_id
    let task = NewTask {
        agent_id: "mika".to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: "traced-task".to_string(),
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
        created_by_session: Some(sid.clone()),
        created_trace_id: Some(trace.to_string()),
        reference_url: None,
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: None,
    };
    db.create_task(&task).unwrap();

    // Query unified_timeline for this trace_id
    let rows: Vec<(String, String)> = db
        .conn
        .prepare("SELECT event_type, summary FROM unified_timeline WHERE trace_id = ?1 ORDER BY event_type")
        .unwrap()
        .query_map([trace], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(
        rows.len(),
        3,
        "expected message + audit + task in unified_timeline"
    );
    let types: Vec<&str> = rows.iter().map(|(t, _)| t.as_str()).collect();
    assert!(types.contains(&"audit"), "missing audit event");
    assert!(types.contains(&"message"), "missing message");
    assert!(types.contains(&"task"), "missing task");
}

#[test]
fn test_unified_timeline_includes_null_trace_id() {
    let (db, sid) = db_with_session();

    // Save message without trace_id (legacy behavior)
    db.save_message("mika", &sid, "user", "legacy msg", None)
        .unwrap();

    // Query for NULL trace_id rows
    let count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM unified_timeline WHERE trace_id IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();

    assert!(
        count >= 1,
        "legacy rows with NULL trace_id should appear in unified_timeline"
    );
}

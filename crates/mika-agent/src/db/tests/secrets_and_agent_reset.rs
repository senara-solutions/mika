//! Tests de `crate::db` — thème `secrets_and_agent_reset` (mika#2321).
//!
//! Le scrubbing des secrets à la frontière `save_tool_call` (#908), la
//! réinitialisation d'agent (#964) et les entités/résolutions du graphe sujet.
//!
//! Enfant de `db::tests`, donc descendant de `db` : les items privés de `db` et
//! les helpers de `db::tests` restent visibles via `use super::*`.

use super::*;

// ===== Secret scrubbing at save_tool_call boundary (#908) =====

#[test]
fn test_save_tool_call_scrubs_secrets_in_output() {
    let db = db();
    db.save_tool_call(
        "tc-1",
        "mika",
        "test-session",
        None,
        None,
        0,
        "read_agent_file",
        "builtin",
        None,
        Some(r#"{"path":".env"}"#),
        Some("MIKA_GITHUB_TOKEN=github_pat_11CBQ5ABC1234567890abcdef\nMIKA_LOG_FORMAT=json"),
        true,
        false,
        100,
        None,
    )
    .unwrap();

    let (input, output): (Option<String>, Option<String>) = db
        .conn
        .query_row(
            "SELECT input, output FROM tool_calls WHERE id = 'tc-1'",
            [],
            |row: &rusqlite::Row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();

    // Output should have secret redacted but non-secret preserved
    let output = output.unwrap();
    assert!(
        !output.contains("github_pat_11CBQ5"),
        "secret should be redacted in output: {output}"
    );
    assert!(
        output.contains("MIKA_GITHUB_TOKEN=<REDACTED>"),
        "env var assignment should be redacted: {output}"
    );
    assert!(
        output.contains("MIKA_LOG_FORMAT=json"),
        "non-secret env var should be preserved: {output}"
    );

    // Input should be unchanged (no secrets)
    let input = input.unwrap();
    assert_eq!(input, r#"{"path":".env"}"#);
}

#[test]
fn test_save_tool_call_scrubs_secrets_in_input() {
    let db = db();
    db.save_tool_call(
        "tc-2",
        "mika",
        "test-session",
        None,
        None,
        0,
        "run_shell",
        "builtin",
        None,
        Some(r#"{"command":"echo ghp_ABCDEFghij1234567890"}"#),
        Some("ghp_ABCDEFghij1234567890"),
        true,
        false,
        50,
        None,
    )
    .unwrap();

    let (input, output): (Option<String>, Option<String>) = db
        .conn
        .query_row(
            "SELECT input, output FROM tool_calls WHERE id = 'tc-2'",
            [],
            |row: &rusqlite::Row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();

    let input = input.unwrap();
    assert!(
        !input.contains("ghp_ABCDEFghij"),
        "secret should be redacted in input: {input}"
    );
    assert!(
        input.contains("ghp_<REDACTED>"),
        "should have redaction marker: {input}"
    );

    let output = output.unwrap();
    assert_eq!(output, "ghp_<REDACTED>");
}

#[test]
fn test_save_tool_call_preserves_clean_content() {
    let db = db();
    let clean_output = "File contents:\nname = mika\nversion = 0.5.0";
    db.save_tool_call(
        "tc-3",
        "mika",
        "test-session",
        None,
        None,
        0,
        "read_agent_file",
        "builtin",
        None,
        Some(r#"{"path":"Cargo.toml"}"#),
        Some(clean_output),
        true,
        false,
        30,
        None,
    )
    .unwrap();

    let output: String = db
        .conn
        .query_row(
            "SELECT output FROM tool_calls WHERE id = 'tc-3'",
            [],
            |row: &rusqlite::Row| row.get(0),
        )
        .unwrap();

    assert_eq!(output, clean_output, "clean content should be unchanged");
}

#[test]
fn test_save_tool_call_scrubs_secrets_in_error_message() {
    let db = db();
    // When a tool fails, error_message carries the same content as output.
    // Both must be scrubbed.
    let secret_error = "Error reading .env: MIKA_GITHUB_TOKEN=github_pat_11CBQ5ABC1234567890abcdef";
    db.save_tool_call(
        "tc-err",
        "mika",
        "test-session",
        None,
        None,
        0,
        "read_agent_file",
        "builtin",
        None,
        Some(r#"{"path":".env"}"#),
        Some(secret_error),
        false,
        false,
        100,
        Some(secret_error),
    )
    .unwrap();

    let (output, err_msg): (Option<String>, Option<String>) = db
        .conn
        .query_row(
            "SELECT output, error_message FROM tool_calls WHERE id = 'tc-err'",
            [],
            |row: &rusqlite::Row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();

    let output = output.unwrap();
    let err_msg = err_msg.unwrap();
    assert!(
        !output.contains("github_pat_11CBQ5"),
        "secret in output should be redacted: {output}"
    );
    assert!(
        !err_msg.contains("github_pat_11CBQ5"),
        "secret in error_message should be redacted: {err_msg}"
    );
}

#[test]
fn test_get_active_callback_tasks_with_pid() {
    let db = db();
    let (_parent_id, child_id) = create_callback_task_pair(&db, "mika", "callback-task");
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress', process_id = 12345 WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    let results = db.get_active_callback_tasks_with_pid("mika").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, child_id);
    assert_eq!(results[0].process_id, Some(12345));
}

#[test]
fn test_get_active_callback_tasks_with_pid_excludes_completed() {
    let db = db();
    let (_parent_id, child_id) = create_callback_task_pair(&db, "mika", "done-callback");
    db.conn
        .execute(
            "UPDATE tasks SET status = 'completed', process_id = 12345 WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    let results = db.get_active_callback_tasks_with_pid("mika").unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_get_active_callback_tasks_with_pid_excludes_no_pid() {
    let db = db();
    let (_parent_id, child_id) = create_callback_task_pair(&db, "mika", "no-pid-callback");
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
            params![child_id],
        )
        .unwrap();

    let results = db.get_active_callback_tasks_with_pid("mika").unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_set_task_metadata_field_new() {
    let db = db();
    let task = new_task("mika", "meta-test", "manual", "none");
    let id = db.create_task(&task).unwrap();

    db.set_task_metadata_field(&id, "process_start_time", "999888")
        .unwrap();

    let val = db
        .get_task_metadata_field(&id, "process_start_time")
        .unwrap();
    assert_eq!(val, Some("999888".to_string()));
}

#[test]
fn test_set_task_metadata_field_existing_metadata() {
    let db = db();
    let task = new_task("mika", "meta-test-2", "manual", "none");
    let id = db.create_task(&task).unwrap();

    // Set initial metadata
    db.update_task_metadata(&id, r#"{"existing":"value"}"#)
        .unwrap();

    // Add a new field
    db.set_task_metadata_field(&id, "first_dead_at", "2026-05-05T00:00:00Z")
        .unwrap();

    // Both fields should exist
    let val = db.get_task_metadata_field(&id, "existing").unwrap();
    assert_eq!(val, Some("value".to_string()));
    let val = db.get_task_metadata_field(&id, "first_dead_at").unwrap();
    assert_eq!(val, Some("2026-05-05T00:00:00Z".to_string()));
}

#[test]
fn test_get_task_metadata_field_missing_key() {
    let db = db();
    let task = new_task("mika", "meta-test-3", "manual", "none");
    let id = db.create_task(&task).unwrap();

    let val = db.get_task_metadata_field(&id, "nonexistent").unwrap();
    assert_eq!(val, None);
}

// -- mika#1011: cancel_task cascade tests --

#[test]
fn test_cancel_task_cascades_to_active_callback_children() {
    let db = db();
    // Create parent task
    let parent = new_task("mika", "parent-work-item", "manual", "none");
    let parent_id = db.create_task(&parent).unwrap();

    // Create callback child (pending)
    let mut child = new_task(
        "mika",
        "long_running:run_claude_pilot",
        "callback",
        "resume_agent",
    );
    child.parent_task_id = Some(parent_id.clone());
    let child_id = db.create_task(&child).unwrap();

    // Create deferred callback child (pending)
    let mut deferred = new_task(
        "mika",
        "long_running:run_claude_pilot:deferred",
        "callback",
        "resume_agent",
    );
    deferred.parent_task_id = Some(parent_id.clone());
    let deferred_id = db.create_task(&deferred).unwrap();

    // Create a non-callback child (manual sub-task) — should NOT be cancelled
    let mut manual_child = new_task("mika", "sub-task", "manual", "none");
    manual_child.parent_task_id = Some(parent_id.clone());
    let manual_child_id = db.create_task(&manual_child).unwrap();

    // Cancel parent
    assert!(db.cancel_task(&parent_id, "mika").unwrap());

    // Callback children should be cancelled
    let child_task = db.get_task_unscoped(&child_id).unwrap().unwrap();
    assert_eq!(
        child_task.status, "cancelled",
        "callback child should be cascaded"
    );

    let deferred_task = db.get_task_unscoped(&deferred_id).unwrap().unwrap();
    assert_eq!(
        deferred_task.status, "cancelled",
        "deferred callback should be cascaded"
    );

    // Non-callback child should NOT be cancelled
    let manual_task = db.get_task_unscoped(&manual_child_id).unwrap().unwrap();
    assert_eq!(
        manual_task.status, "pending",
        "manual child should not be affected"
    );
}

#[test]
fn test_cancel_task_does_not_cascade_to_completed_callback() {
    let db = db();
    let parent = new_task("mika", "parent", "manual", "none");
    let parent_id = db.create_task(&parent).unwrap();

    let mut child = new_task(
        "mika",
        "long_running:run_claude_pilot",
        "callback",
        "resume_agent",
    );
    child.parent_task_id = Some(parent_id.clone());
    let child_id = db.create_task(&child).unwrap();
    // Mark child as completed
    db.update_task_completed(&child_id, "mika", Some("done"))
        .unwrap();

    // Cancel parent
    assert!(db.cancel_task(&parent_id, "mika").unwrap());

    // Completed child should NOT be affected
    let child_task = db.get_task_unscoped(&child_id).unwrap().unwrap();
    assert_eq!(
        child_task.status, "completed",
        "completed callback should not be cancelled"
    );
}

// -- mika#1011: deferred callback DB helper tests --

#[test]
fn test_count_pending_deferred_callbacks() {
    let db = db();
    // No deferred callbacks initially
    assert_eq!(db.count_pending_deferred_callbacks("mika").unwrap(), 0);

    // Create parent tasks (FK requirement)
    let p1_id = db
        .create_task(&new_task("mika", "p1", "manual", "none"))
        .unwrap();
    let p2_id = db
        .create_task(&new_task("mika", "p2", "manual", "none"))
        .unwrap();
    let p3_id = db
        .create_task(&new_task("mika", "p3", "manual", "none"))
        .unwrap();

    // Create a deferred callback
    let mut task = new_task(
        "mika",
        "long_running:run_claude_pilot:deferred",
        "callback",
        "resume_agent",
    );
    task.parent_task_id = Some(p1_id.clone());
    db.create_task(&task).unwrap();

    assert_eq!(db.count_pending_deferred_callbacks("mika").unwrap(), 1);

    // Create another
    let mut task2 = new_task(
        "mika",
        "long_running:run_claude_pilot:deferred",
        "callback",
        "resume_agent",
    );
    task2.parent_task_id = Some(p2_id.clone());
    db.create_task(&task2).unwrap();

    assert_eq!(db.count_pending_deferred_callbacks("mika").unwrap(), 2);

    // Non-deferred callback should not count
    let mut regular = new_task(
        "mika",
        "long_running:run_claude_pilot",
        "callback",
        "resume_agent",
    );
    regular.parent_task_id = Some(p3_id.clone());
    db.create_task(&regular).unwrap();

    assert_eq!(db.count_pending_deferred_callbacks("mika").unwrap(), 2);
}

#[test]
fn test_promote_next_deferred_callback_fifo() {
    let db = db();

    // No deferred callbacks → returns None
    assert!(db.promote_next_deferred_callback("mika").unwrap().is_none());

    // Create parent tasks (FK requirement)
    let p1_id = db
        .create_task(&new_task("mika", "p1", "manual", "none"))
        .unwrap();
    let p2_id = db
        .create_task(&new_task("mika", "p2", "manual", "none"))
        .unwrap();

    // Create two deferred callbacks
    let mut task1 = new_task(
        "mika",
        "long_running:run_claude_pilot:deferred",
        "callback",
        "resume_agent",
    );
    task1.parent_task_id = Some(p1_id.clone());
    let id1 = db.create_task(&task1).unwrap();

    let mut task2 = new_task(
        "mika",
        "long_running:run_claude_pilot:deferred",
        "callback",
        "resume_agent",
    );
    task2.parent_task_id = Some(p2_id.clone());
    let id2 = db.create_task(&task2).unwrap();

    // Promote first (FIFO)
    assert!(db.promote_next_deferred_callback("mika").unwrap().is_some());

    // First should be completed, second still pending
    let t1 = db.get_task_unscoped(&id1).unwrap().unwrap();
    assert_eq!(
        t1.status, "completed",
        "first deferred should be promoted to completed"
    );
    assert!(t1.result.is_some(), "promoted task should have a result");

    let t2 = db.get_task_unscoped(&id2).unwrap().unwrap();
    assert_eq!(
        t2.status, "pending",
        "second deferred should still be pending"
    );

    // Promote second
    assert!(db.promote_next_deferred_callback("mika").unwrap().is_some());
    let t2 = db.get_task_unscoped(&id2).unwrap().unwrap();
    assert_eq!(t2.status, "completed");

    // No more → returns None
    assert!(db.promote_next_deferred_callback("mika").unwrap().is_none());
}

/// mika#1175 — Class-scoped sibling of `test_promote_next_deferred_callback_fifo`.
/// Verifies that `promote_next_deferred_callback_for_class` filters by
/// `dispatch_class`, that NULL-class rows are treated as `'implement'` via
/// `COALESCE`, and that FIFO is preserved within a class.
#[test]
fn test_promote_next_deferred_callback_for_class_filters_by_class() {
    let db = db();

    // No deferred callbacks → both class predicates return None.
    assert!(
        db.promote_next_deferred_callback_for_class("mika", "implement")
            .unwrap()
            .is_none()
    );
    assert!(
        db.promote_next_deferred_callback_for_class("mika", "groom")
            .unwrap()
            .is_none()
    );

    // Parent tasks (FK requirement).
    let p_impl = db
        .create_task(&new_task("mika", "p_impl", "manual", "none"))
        .unwrap();
    let p_groom = db
        .create_task(&new_task("mika", "p_groom", "manual", "none"))
        .unwrap();
    let p_null = db
        .create_task(&new_task("mika", "p_null", "manual", "none"))
        .unwrap();

    // Three deferred wrappers: implement, groom, NULL (pre-v34 row).
    let mut w_impl = new_task(
        "mika",
        "long_running:run_claude_pilot:deferred",
        "callback",
        "resume_agent",
    );
    w_impl.parent_task_id = Some(p_impl.clone());
    w_impl.dispatch_class = Some("implement".to_string());
    let id_impl = db.create_task(&w_impl).unwrap();

    let mut w_groom = new_task(
        "mika",
        "long_running:run_claude_pilot:deferred",
        "callback",
        "resume_agent",
    );
    w_groom.parent_task_id = Some(p_groom.clone());
    w_groom.dispatch_class = Some("groom".to_string());
    let id_groom = db.create_task(&w_groom).unwrap();

    let mut w_null = new_task(
        "mika",
        "long_running:run_claude_pilot:deferred",
        "callback",
        "resume_agent",
    );
    w_null.parent_task_id = Some(p_null.clone());
    w_null.dispatch_class = None; // pre-v34 NULL row
    let id_null = db.create_task(&w_null).unwrap();

    // Promote groom: only the groom wrapper transitions.
    assert!(
        db.promote_next_deferred_callback_for_class("mika", "groom")
            .unwrap()
            .is_some()
    );
    assert_eq!(
        db.get_task_unscoped(&id_groom).unwrap().unwrap().status,
        "completed"
    );
    assert_eq!(
        db.get_task_unscoped(&id_impl).unwrap().unwrap().status,
        "pending",
        "implement wrapper must not transition on a groom promotion"
    );
    assert_eq!(
        db.get_task_unscoped(&id_null).unwrap().unwrap().status,
        "pending",
        "NULL-class wrapper must not transition on a groom promotion"
    );

    // No more groom wrappers pending.
    assert!(
        db.promote_next_deferred_callback_for_class("mika", "groom")
            .unwrap()
            .is_none()
    );

    // First implement promotion: one of (implement, NULL) transitions
    // (FIFO within the implement+NULL class group via COALESCE).
    assert!(
        db.promote_next_deferred_callback_for_class("mika", "implement")
            .unwrap()
            .is_some()
    );
    let after_first_impl = (
        db.get_task_unscoped(&id_impl).unwrap().unwrap().status,
        db.get_task_unscoped(&id_null).unwrap().unwrap().status,
    );
    assert!(
        matches!(
            after_first_impl,
            (ref a, ref b) if (a == "completed" && b == "pending") || (a == "pending" && b == "completed")
        ),
        "exactly one of (implement, NULL) must transition on first implement promotion, got {after_first_impl:?}"
    );

    // Second implement promotion: the remaining wrapper transitions
    // (NULL is matched by 'implement' via COALESCE).
    assert!(
        db.promote_next_deferred_callback_for_class("mika", "implement")
            .unwrap()
            .is_some()
    );
    assert_eq!(
        db.get_task_unscoped(&id_impl).unwrap().unwrap().status,
        "completed",
        "implement wrapper must be completed after second implement promotion"
    );
    assert_eq!(
        db.get_task_unscoped(&id_null).unwrap().unwrap().status,
        "completed",
        "NULL-class wrapper must be completed after second implement \
             promotion (COALESCE treats NULL as 'implement')"
    );

    // Drained.
    assert!(
        db.promote_next_deferred_callback_for_class("mika", "implement")
            .unwrap()
            .is_none()
    );
    assert!(
        db.promote_next_deferred_callback_for_class("mika", "groom")
            .unwrap()
            .is_none()
    );
}

/// mika#1070 — Regression test: chain promotion works after anti-cascade
/// guard removal. Simulates the full lifecycle:
/// 1. Blocking callback completes → promotes wrapper W1
/// 2. W1's DeferredDispatch turn completes (mark delivered) → promotes W2
#[test]
fn test_deferred_dispatch_chain_promotion() {
    let db = db();

    // Create parent tasks
    let p1 = db
        .create_task(&new_task("mika", "host1", "manual", "none"))
        .unwrap();
    let p2 = db
        .create_task(&new_task("mika", "host2", "manual", "none"))
        .unwrap();

    // Create two deferred wrappers
    let mut w1 = new_task(
        "mika",
        "long_running:run_claude_pilot:deferred",
        "callback",
        "resume_agent",
    );
    w1.parent_task_id = Some(p1.clone());
    let w1_id = db.create_task(&w1).unwrap();

    let mut w2 = new_task(
        "mika",
        "long_running:run_claude_pilot:deferred",
        "callback",
        "resume_agent",
    );
    w2.parent_task_id = Some(p2.clone());
    let w2_id = db.create_task(&w2).unwrap();

    // Step 1: Promote W1 (simulates blocking callback completion)
    assert!(db.promote_next_deferred_callback("mika").unwrap().is_some());
    let t1 = db.get_task_unscoped(&w1_id).unwrap().unwrap();
    assert_eq!(t1.status, "completed");
    assert!(t1.completed_at.is_some());

    // W1 should be returned by get_undelivered_callback_tasks
    let since = crate::timestamp::now_minus(chrono::Duration::hours(1));
    let undelivered = db.get_undelivered_callback_tasks("mika", &since).unwrap();
    assert!(
        undelivered.iter().any(|t| t.id == w1_id),
        "promoted W1 should be in undelivered callbacks"
    );

    // Step 2: Mark W1 as delivered (simulates DeferredDispatch turn completion)
    assert!(db.mark_task_delivered(&w1_id).unwrap());

    // Step 3: Chain promotion — promote W2 (this was blocked by the
    // anti-cascade guard before mika#1070)
    assert!(db.promote_next_deferred_callback("mika").unwrap().is_some());
    let t2 = db.get_task_unscoped(&w2_id).unwrap().unwrap();
    assert_eq!(
        t2.status, "completed",
        "W2 should be promoted via chain promotion"
    );

    // W2 should be in undelivered callbacks
    let undelivered = db.get_undelivered_callback_tasks("mika", &since).unwrap();
    assert!(
        undelivered.iter().any(|t| t.id == w2_id),
        "promoted W2 should be in undelivered callbacks"
    );
}

/// mika#1070 — Regression test: the occupancy predicate correctly identifies
/// active non-deferred callbacks and excludes deferred wrappers.
///
/// **mika#2162 rebased this onto the class-scoped form.** It used to exercise
/// `has_any_active_callback`, the agent-wide sibling, whose own doc-comment
/// declared it kept "as a regression-test baseline" with no production caller.
/// A fifth hand-written copy of a clause being unified is exactly where the
/// next divergence settles, so the method is gone and its coverage lives here.
/// The rows below carry no `dispatch_class`, so they reach the predicate
/// through the `COALESCE(…, 'implement')` term — which is part of what this
/// asserts.
#[test]
fn test_has_any_active_callback() {
    let db = db();

    // No callbacks at all → false
    assert!(
        !db.has_any_active_callback_for_class("mika", "implement")
            .unwrap()
    );

    // Create parent task
    let p1 = db
        .create_task(&new_task("mika", "host1", "manual", "none"))
        .unwrap();

    // Add a deferred wrapper (should NOT count as active)
    let mut deferred = new_task(
        "mika",
        "long_running:run_claude_pilot:deferred",
        "callback",
        "resume_agent",
    );
    deferred.parent_task_id = Some(p1.clone());
    db.create_task(&deferred).unwrap();
    assert!(
        !db.has_any_active_callback_for_class("mika", "implement")
            .unwrap(),
        "deferred wrapper should not count as active callback"
    );

    // Add a regular callback (SHOULD count as active)
    let p2 = db
        .create_task(&new_task("mika", "host2", "manual", "none"))
        .unwrap();
    let mut regular = new_task(
        "mika",
        "long_running:run_claude_pilot",
        "callback",
        "resume_agent",
    );
    regular.parent_task_id = Some(p2.clone());
    let reg_id = db.create_task(&regular).unwrap();
    assert!(
        db.has_any_active_callback_for_class("mika", "implement")
            .unwrap(),
        "regular pending callback should count as active"
    );

    // Complete then deliver → no more active
    db.update_task_completed(&reg_id, "mika", Some("done"))
        .unwrap();
    db.mark_task_delivered(&reg_id).unwrap();
    assert!(
        !db.has_any_active_callback_for_class("mika", "implement")
            .unwrap(),
        "delivered callback should not count as active"
    );
}

/// mika#1175 — Class-scoped sibling of `test_has_any_active_callback`.
/// Verifies that `has_any_active_callback_for_class` is scoped to the given
/// `dispatch_class`, that `:deferred` wrappers are excluded in both classes
/// (parity with mika#1163), and that NULL-class rows are matched by the
/// `'implement'` predicate via `COALESCE`.
#[test]
fn test_has_any_active_callback_for_class_class_scoped() {
    let db = db();

    // Empty DB → both predicates false.
    assert!(
        !db.has_any_active_callback_for_class("mika", "implement")
            .unwrap()
    );
    assert!(
        !db.has_any_active_callback_for_class("mika", "groom")
            .unwrap()
    );

    // Active non-deferred implement callback.
    let p_impl = db
        .create_task(&new_task("mika", "p_impl", "manual", "none"))
        .unwrap();
    let mut active_impl = new_task(
        "mika",
        "long_running:run_claude_pilot",
        "callback",
        "resume_agent",
    );
    active_impl.parent_task_id = Some(p_impl.clone());
    active_impl.dispatch_class = Some("implement".to_string());
    db.create_task(&active_impl).unwrap();

    assert!(
        db.has_any_active_callback_for_class("mika", "implement")
            .unwrap(),
        "implement-class predicate must detect the active implement callback"
    );
    assert!(
        !db.has_any_active_callback_for_class("mika", "groom")
            .unwrap(),
        "groom-class predicate must not see the implement callback"
    );

    // Add `:deferred` wrappers in BOTH classes — must not flip either predicate.
    let p_def_impl = db
        .create_task(&new_task("mika", "p_def_impl", "manual", "none"))
        .unwrap();
    let mut def_impl = new_task(
        "mika",
        "long_running:run_claude_pilot:deferred",
        "callback",
        "resume_agent",
    );
    def_impl.parent_task_id = Some(p_def_impl.clone());
    def_impl.dispatch_class = Some("implement".to_string());
    db.create_task(&def_impl).unwrap();

    let p_def_groom = db
        .create_task(&new_task("mika", "p_def_groom", "manual", "none"))
        .unwrap();
    let mut def_groom = new_task(
        "mika",
        "long_running:run_claude_pilot:deferred",
        "callback",
        "resume_agent",
    );
    def_groom.parent_task_id = Some(p_def_groom.clone());
    def_groom.dispatch_class = Some("groom".to_string());
    db.create_task(&def_groom).unwrap();

    assert!(
        db.has_any_active_callback_for_class("mika", "implement")
            .unwrap(),
        "implement predicate unchanged by :deferred wrappers (mika#1163 parity)"
    );
    assert!(
        !db.has_any_active_callback_for_class("mika", "groom")
            .unwrap(),
        "groom predicate unchanged by :deferred wrappers (mika#1163 parity)"
    );

    // NULL-class active callback must be matched by `"implement"` via COALESCE.
    // Use a fresh agent so the assertion is independent of the rows above.
    db.register_agent("mika-null", "mika-null", "").unwrap();
    let p_null = db
        .create_task(&new_task("mika-null", "p_null", "manual", "none"))
        .unwrap();
    let mut active_null = new_task(
        "mika-null",
        "long_running:run_claude_pilot",
        "callback",
        "resume_agent",
    );
    active_null.parent_task_id = Some(p_null.clone());
    active_null.dispatch_class = None;
    db.create_task(&active_null).unwrap();

    assert!(
        db.has_any_active_callback_for_class("mika-null", "implement")
            .unwrap(),
        "NULL-class active callback must be matched by 'implement' predicate \
             (COALESCE matches mika#1163 slot-guard semantics)"
    );
    assert!(
        !db.has_any_active_callback_for_class("mika-null", "groom")
            .unwrap(),
        "NULL-class active callback must NOT be matched by 'groom' predicate"
    );
}

/// mika#1163 — Regression test: `has_active_callback_tasks_excluding` must
/// exclude `:deferred` wrappers when looking for "slot occupied" evidence.
///
/// The sibling predicate `has_any_active_callback` (mika#1070) already
/// excludes `:deferred` rows; this test pins the same semantics on the
/// per-class slot predicate used by `validate_dispatch_readiness`. Without
/// the `label NOT LIKE '%:deferred'` clause, two parents each holding a
/// pending deferred wrapper deadlock: every dispatch attempt from one
/// wrapper sees the OTHER wrapper as an active dispatch, so neither ever
/// promotes through `run_claude_pilot`.
#[test]
fn test_has_active_callback_tasks_excluding_ignores_deferred_wrappers() {
    let db = db();

    // Parent A with one pending deferred wrapper as a callback child.
    let p_a = db
        .create_task(&new_task("mika", "host_a", "manual", "none"))
        .unwrap();
    let mut w_a = new_task(
        "mika",
        "long_running:run_claude_pilot:deferred",
        "callback",
        "resume_agent",
    );
    w_a.parent_task_id = Some(p_a.clone());
    db.create_task(&w_a).unwrap();

    // Querying for any other parent: a `:deferred` wrapper is NOT an
    // active dispatch, so the predicate must return None.
    let result = db
        .has_active_callback_tasks_excluding("other-task", "mika", "implement")
        .unwrap();
    assert!(
        result.is_none(),
        "pending :deferred wrapper must not count as an active dispatch \
             (mika#1163 — was previously detected as slot-occupied, deadlocking \
             every cross-parent dispatch attempt)"
    );

    // Add Parent B with a REAL (non-deferred) pending callback. This IS
    // an active dispatch and MUST be detected.
    let p_b = db
        .create_task(&new_task("mika", "host_b", "manual", "none"))
        .unwrap();
    let mut real_b = new_task(
        "mika",
        "long_running:run_claude_pilot",
        "callback",
        "resume_agent",
    );
    real_b.parent_task_id = Some(p_b.clone());
    let real_b_id = db.create_task(&real_b).unwrap();

    // Querying from a third parent: should find Parent B's real callback,
    // proving the exclusion is wrapper-only, not blanket.
    let result = db
        .has_active_callback_tasks_excluding("other-task", "mika", "implement")
        .unwrap();
    let blocking = result.expect(
        "real (non-deferred) pending callback MUST still be detected as an \
             active dispatch — exclusion is narrowly scoped to :deferred wrappers",
    );
    assert_eq!(
        blocking.parent_task_id, p_b,
        "should match Parent B (the real dispatch)"
    );
    assert_eq!(blocking.callback_task_id, real_b_id);
    assert_eq!(blocking.label, "long_running:run_claude_pilot");

    // Mixed-state: querying from Parent B itself excludes B's own callback
    // via the parent_task_id != ?1 clause, and the only remaining row is
    // A's deferred wrapper. Result must be None.
    let result = db
        .has_active_callback_tasks_excluding(&p_b, "mika", "implement")
        .unwrap();
    assert!(
        result.is_none(),
        "Parent B's query: own callback excluded by parent filter, A's \
             wrapper excluded by :deferred filter — no blocking dispatch"
    );

    // Suffix-anchor pin: the `%:deferred` LIKE pattern matches END of the
    // label, not arbitrary substring. A hypothetical future label that
    // contains `:deferred` mid-string (e.g., `:deferred:retry`) is NOT
    // excluded by the wildcard — it would be counted as an active
    // dispatch. This pins the convention so a refactor that shifts the
    // suffix convention has to update the SQL clause deliberately.
    let p_d = db
        .create_task(&new_task("mika", "host_d", "manual", "none"))
        .unwrap();
    let mut suffix_variant = new_task(
        "mika",
        "long_running:run_claude_pilot:deferred:retry",
        "callback",
        "resume_agent",
    );
    suffix_variant.parent_task_id = Some(p_d.clone());
    let suffix_variant_id = db.create_task(&suffix_variant).unwrap();
    // Delete the existing real callback for Parent B so this assertion
    // can isolate the suffix-variant row's behavior.
    db.cancel_task(&real_b_id, "mika").unwrap();
    let result = db
        .has_active_callback_tasks_excluding("other-task", "mika", "implement")
        .unwrap();
    let blocking = result.expect(
        "label `:deferred:retry` is NOT a suffix match for `%:deferred` — \
             must still be counted as an active dispatch",
    );
    assert_eq!(
        blocking.parent_task_id, p_d,
        "suffix-variant row should be the blocker"
    );
    assert_eq!(blocking.callback_task_id, suffix_variant_id);

    // Forward-compat for in_progress deferred wrappers: today no code path
    // sets a `:deferred` row to `in_progress`, but the SQL `status IN
    // ('pending', 'in_progress')` clause catches both. Verify the
    // exclusion also holds for in_progress wrappers so a future code path
    // that flips a wrapper to in_progress doesn't accidentally reintroduce
    // the deadlock.
    db.update_task_status(&suffix_variant_id, "cancelled")
        .unwrap(); // clean up suffix-variant first
    let p_e = db
        .create_task(&new_task("mika", "host_e", "manual", "none"))
        .unwrap();
    let mut deferred_in_progress = new_task(
        "mika",
        "long_running:run_claude_pilot:deferred",
        "callback",
        "resume_agent",
    );
    deferred_in_progress.parent_task_id = Some(p_e.clone());
    let dip_id = db.create_task(&deferred_in_progress).unwrap();
    db.update_task_status(&dip_id, "in_progress").unwrap();
    let result = db
        .has_active_callback_tasks_excluding("other-task", "mika", "implement")
        .unwrap();
    assert!(
        result.is_none(),
        "in_progress :deferred wrapper must also be excluded (forward-compat \
             — today no code path sets a wrapper to in_progress, but the SQL \
             status filter covers both pending+in_progress, so the exclusion \
             contract must hold for both)"
    );
}

/// mika#1070 — Regression test: AgentBusy recovery keeps callback in
/// 'completed' status so dispatch_undelivered_callbacks can find it.
/// The old behavior reset to 'pending', which stranded the callback.
#[test]
fn test_agent_busy_callback_stays_completed() {
    let db = db();

    let p1 = db
        .create_task(&new_task("mika", "host1", "manual", "none"))
        .unwrap();

    let mut cb = new_task(
        "mika",
        "long_running:run_claude_pilot",
        "callback",
        "resume_agent",
    );
    cb.parent_task_id = Some(p1.clone());
    let cb_id = db.create_task(&cb).unwrap();

    // Simulate external completion (webhook handler sets completed)
    db.update_task_completed(&cb_id, "mika", Some("done"))
        .unwrap();
    let t = db.get_task_unscoped(&cb_id).unwrap().unwrap();
    assert_eq!(t.status, "completed");

    // Simulate AgentBusy: keep completed, just set next_fire_at for retry
    let retry_at = crate::timestamp::now_plus(chrono::Duration::seconds(30));
    db.update_task_next_fire_at(&cb_id, &retry_at).unwrap();

    // The task should still be found by get_undelivered_callback_tasks
    let since = crate::timestamp::now_minus(chrono::Duration::hours(1));
    let undelivered = db.get_undelivered_callback_tasks("mika", &since).unwrap();
    assert!(
        undelivered.iter().any(|t| t.id == cb_id),
        "AgentBusy callback should remain in completed status and be findable"
    );

    // Verify it has next_fire_at set (for retry delay guard in engine)
    let t = db.get_task_unscoped(&cb_id).unwrap().unwrap();
    assert!(
        t.next_fire_at.is_some(),
        "AgentBusy callback should have next_fire_at for retry delay"
    );
}

#[test]
fn test_reset_agent_empty() {
    // Reset an agent with no state — should be idempotent, all counts 0
    let db = db();
    let agent_id = "test-agent";
    db.register_agent(agent_id, "Test Agent", "/tmp/test")
        .unwrap();

    // First reset: all zeros
    let counts = db.reset_agent_state(agent_id).unwrap();
    assert_eq!(counts.total(), 0);

    // Agent row still exists
    let agents = db.list_agents_db().unwrap();
    assert!(agents.iter().any(|a| a.id == agent_id));

    // Second reset: still idempotent
    let counts2 = db.reset_agent_state(agent_id).unwrap();
    assert_eq!(counts2.total(), 0);
}

#[test]
fn test_reset_agent_populated() {
    let db = db();
    let agent_id = "test-agent";
    db.register_agent(agent_id, "Test Agent", "/tmp/test")
        .unwrap();

    // Insert data into multiple child tables
    let sid = "test-reset-session";
    db.create_session(sid, agent_id, "cli").unwrap();
    db.save_message(agent_id, sid, "user", "hello", None)
        .unwrap();
    db.save_message(agent_id, sid, "assistant", "hi", None)
        .unwrap();
    db.set_core_memory(agent_id, "user_summary", "test user")
        .unwrap();

    // Create a task
    let task = make_manual_task(agent_id, "test task");
    db.create_task(&task).unwrap();

    // Create a person fact
    db.conn
        .execute(
            "INSERT INTO people (agent_id, canonical_name, relationship, notes)
                 VALUES (?1, 'Alice', 'colleague', 'test')",
            params![agent_id],
        )
        .unwrap();

    // Create a heartbeat_sends entry
    db.conn
        .execute(
            "INSERT INTO heartbeat_sends (agent_id, sent_at)
                 VALUES (?1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
            params![agent_id],
        )
        .unwrap();

    // Create a customer_config entry
    db.conn
        .execute(
            "INSERT INTO customer_config (agent_id, key, value)
                 VALUES (?1, 'test_key', 'test_value')",
            params![agent_id],
        )
        .unwrap();

    // Verify data exists before reset
    let pre_counts = db.count_agent_state(agent_id).unwrap();
    assert!(pre_counts.sessions > 0);
    assert!(pre_counts.messages > 0);
    assert!(pre_counts.core_memory > 0);
    assert!(pre_counts.tasks > 0);
    assert!(pre_counts.people > 0);
    assert!(pre_counts.heartbeat_sends > 0);
    assert!(pre_counts.customer_config > 0);

    // Reset
    let counts = db.reset_agent_state(agent_id).unwrap();
    assert!(counts.total() > 0, "Should have deleted rows");

    // Verify all child tables are empty for this agent
    let post_counts = db.count_agent_state(agent_id).unwrap();
    assert_eq!(post_counts.total(), 0, "All child tables should be empty");

    // Agent row still exists
    let agents = db.list_agents_db().unwrap();
    assert!(
        agents.iter().any(|a| a.id == agent_id),
        "Agent row must survive reset"
    );
}

#[test]
fn test_reset_agent_active_task_guard() {
    let db = db();
    let agent_id = "test-agent";
    db.register_agent(agent_id, "Test Agent", "/tmp/test")
        .unwrap();

    // Create an in_progress task
    let mut task = make_manual_task(agent_id, "active work");
    task.trigger_type = "manual".to_string();
    let task_id = db.create_task(&task).unwrap();

    // Transition to in_progress
    db.update_task_status(&task_id, "in_progress").unwrap();

    // Active-task guard should find it
    let active = db.get_active_tasks_for_agent(agent_id).unwrap();
    assert!(!active.is_empty(), "Should detect active task");
    assert!(
        active.iter().any(|(id, _)| id == &task_id),
        "Should include the in_progress task"
    );
}

#[test]
fn test_reset_agent_nonexistent() {
    let db = db();
    let result = db.reset_agent_state("nonexistent-agent");
    assert!(result.is_err(), "Should fail for nonexistent agent");
}

#[test]
fn test_count_agent_state_nonexistent() {
    let db = db();
    let result = db.count_agent_state("nonexistent-agent");
    assert!(result.is_err(), "Should fail for nonexistent agent");
}

#[test]
fn test_kg_outcome_stats_empty_returns_zeros() {
    let db = db();
    let stats = db.kg_resolution_outcome_stats("mika", None, 7).unwrap();
    assert_eq!(stats.total, 0);
    assert_eq!(stats.attempted, 0);
    assert_eq!(stats.no_match, 0);
    assert_eq!(stats.no_match_rate(), 0.0);
}

#[test]
fn test_kg_outcome_stats_agent_wide() {
    let db = db();
    let hash = "testhash1234";
    let agent = "mika"; // Default agent registered by schema init.

    // Seed entities
    let e1 = seed_subject_entity(&db, hash, "entity-1");
    let e2 = seed_subject_entity(&db, hash, "entity-2");
    let e3 = seed_subject_entity(&db, hash, "entity-3");
    let e4 = seed_subject_entity(&db, hash, "entity-4");
    let e5 = seed_subject_entity(&db, hash, "entity-5");

    // Seed outcomes within 7-day window
    seed_resolution_log(&db, agent, e1, "no_match", 1);
    seed_resolution_log(&db, agent, e2, "matched_exact", 2);
    seed_resolution_log(&db, agent, e3, "matched_llm", 3);
    seed_resolution_log(&db, agent, e4, "skipped_no_llm", 1);
    seed_resolution_log(&db, agent, e5, "error", 1);

    let stats = db.kg_resolution_outcome_stats(agent, None, 7).unwrap();

    assert_eq!(stats.total, 5);
    assert_eq!(stats.attempted, 3); // no_match + matched_exact + matched_llm
    assert_eq!(stats.no_match, 1);
    assert_eq!(stats.matched_exact, 1);
    assert_eq!(stats.matched_llm, 1);
    assert_eq!(stats.skipped, 1);
    assert_eq!(stats.errors, 1);
    // no_match_rate = 1/3 ≈ 0.333
    assert!(
        (stats.no_match_rate() - 1.0 / 3.0).abs() < 1e-10,
        "Expected ~0.333, got {}",
        stats.no_match_rate()
    );
}

#[test]
fn test_kg_outcome_stats_per_corpus() {
    let db = db();
    let hash_a = "hash_corpus_a";
    let hash_b = "hash_corpus_b";
    let agent = "mika";

    // Corpus A: 2 no_match, 1 matched
    let a1 = seed_subject_entity(&db, hash_a, "a-entity-1");
    let a2 = seed_subject_entity(&db, hash_a, "a-entity-2");
    let a3 = seed_subject_entity(&db, hash_a, "a-entity-3");
    seed_resolution_log(&db, agent, a1, "no_match", 1);
    seed_resolution_log(&db, agent, a2, "no_match", 2);
    seed_resolution_log(&db, agent, a3, "matched_exact", 1);

    // Corpus B: 0 no_match, 2 matched
    let b1 = seed_subject_entity(&db, hash_b, "b-entity-1");
    let b2 = seed_subject_entity(&db, hash_b, "b-entity-2");
    seed_resolution_log(&db, agent, b1, "matched_llm", 1);
    seed_resolution_log(&db, agent, b2, "matched_exact", 3);

    // Per-corpus A
    let stats_a = db
        .kg_resolution_outcome_stats(agent, Some(hash_a), 7)
        .unwrap();
    assert_eq!(stats_a.attempted, 3);
    assert_eq!(stats_a.no_match, 2);
    assert!((stats_a.no_match_rate() - 2.0 / 3.0).abs() < 1e-10);

    // Per-corpus B
    let stats_b = db
        .kg_resolution_outcome_stats(agent, Some(hash_b), 7)
        .unwrap();
    assert_eq!(stats_b.attempted, 2);
    assert_eq!(stats_b.no_match, 0);
    assert_eq!(stats_b.no_match_rate(), 0.0);

    // Agent-wide includes both corpora
    let stats_all = db.kg_resolution_outcome_stats(agent, None, 7).unwrap();
    assert_eq!(stats_all.attempted, 5);
    assert_eq!(stats_all.no_match, 2);
}

#[test]
fn test_kg_outcome_stats_window_excludes_old_rows() {
    let db = db();
    let hash = "windowhash";
    let agent = "mika";

    // 3-day-old row — inside 7-day window
    let e1 = seed_subject_entity(&db, hash, "recent-entity");
    seed_resolution_log(&db, agent, e1, "no_match", 3);

    // 8-day-old row — outside 7-day window
    let e2 = seed_subject_entity(&db, hash, "old-entity");
    seed_resolution_log(&db, agent, e2, "no_match", 8);

    let stats = db.kg_resolution_outcome_stats(agent, None, 7).unwrap();
    assert_eq!(stats.total, 1, "8-day-old row should be excluded");
    assert_eq!(stats.no_match, 1);
}

#[test]
fn test_kg_outcome_stats_all_outcome_types() {
    let db = db();
    let hash = "alloutcomes";

    let outcomes = [
        "matched_exact",
        "matched_llm",
        "matched_llm_db_fallback",
        "no_match",
        "no_candidate_of_type",
        "skipped_no_llm",
        "skipped_discovered_type",
        "skipped_discovered_subject",
        "error",
    ];
    let agent = "mika";
    for (i, outcome) in outcomes.iter().enumerate() {
        let eid = seed_subject_entity(&db, hash, &format!("e-{i}"));
        seed_resolution_log(&db, agent, eid, outcome, 1);
    }

    let stats = db.kg_resolution_outcome_stats(agent, None, 7).unwrap();
    assert_eq!(stats.total, 9);
    assert_eq!(stats.attempted, 5); // matched_exact + matched_llm + matched_llm_db_fallback + no_match + no_candidate_of_type
    assert_eq!(stats.skipped, 3); // skipped_no_llm + skipped_discovered_type + skipped_discovered_subject
    assert_eq!(stats.errors, 1);
    assert_eq!(stats.matched_exact, 1);
    assert_eq!(stats.matched_llm, 1);
    assert_eq!(stats.matched_llm_db_fallback, 1);
    assert_eq!(stats.no_match, 1);
    assert_eq!(stats.no_candidate_of_type, 1);
}

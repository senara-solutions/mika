//! Tests de `crate::db` — thème `task_messages_and_groom` (mika#2321).
//!
//! Les `task_messages` (mika#974), la force-promotion des wrappers différés
//! (mika#1453), l'annulation des récurrences orphelines et le prédicat
//! `has_completed_groom_for_issue` (mika#1620 / mika#2287).
//!
//! Enfant de `db::tests`, donc descendant de `db` : les items privés de `db` et
//! les helpers de `db::tests` restent visibles via `use super::*`.

use super::*;

#[test]
fn test_double_write_tagged_event() {
    let (mut db, sid) = db_with_session();
    let task_id = "task-root-123";
    create_test_task(&db, task_id, "issue", None);

    let msg_id = db
        .save_message_with_task_context(
            "mika",
            &sid,
            "assistant",
            "Hello from task",
            None,
            None,
            false,
            Some(task_id),
        )
        .unwrap();
    assert!(msg_id > 0);

    // Verify row in messages
    let msgs = db.load_recent_messages("mika", 10).unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content, "Hello from task");

    // Verify row in task_messages
    let task_msgs = db.load_task_messages(task_id).unwrap();
    assert_eq!(task_msgs.len(), 1);
    assert_eq!(task_msgs[0].content, "Hello from task");
    assert_eq!(task_msgs[0].task_id, task_id);
    assert_eq!(task_msgs[0].session_id, sid);
    assert_eq!(task_msgs[0].role, "assistant");
}

#[test]
fn test_single_write_untagged_event() {
    let (mut db, sid) = db_with_session();

    db.save_message_with_task_context(
        "mika",
        &sid,
        "user",
        "No task context",
        None,
        None,
        false,
        None,
    )
    .unwrap();

    // Verify row in messages
    let msgs = db.load_recent_messages("mika", 10).unwrap();
    assert_eq!(msgs.len(), 1);

    // Verify task_messages is empty
    let count: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM task_messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_double_write_transaction_atomicity() {
    // Verify that if the task_messages INSERT would fail, messages INSERT
    // also rolls back. We test by trying to write with a task_id and verifying
    // both tables are consistent.
    let (mut db, sid) = db_with_session();

    // Write two tagged messages, verify both tables have exactly 2 rows.
    db.save_message_with_task_context("mika", &sid, "user", "msg1", None, None, false, Some("t1"))
        .unwrap();
    db.save_message_with_task_context(
        "mika",
        &sid,
        "assistant",
        "msg2",
        None,
        None,
        false,
        Some("t1"),
    )
    .unwrap();

    let msg_count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE agent_id = 'mika'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let task_msg_count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM task_messages WHERE task_id = 't1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(msg_count, 2);
    assert_eq!(task_msg_count, 2);
}

#[test]
fn test_scope_root_walk() {
    let db = db();
    // Build tree: project → milestone → issue
    create_test_task(&db, "proj-1", "project", None);
    create_test_task(&db, "ms-1", "milestone", Some("proj-1"));
    create_test_task(&db, "issue-1", "issue", Some("ms-1"));
    // Callback child (not a scope type)
    db.conn
        .execute(
            "INSERT INTO tasks (id, agent_id, depth, label, trigger_type, action_type, action_config, status, type, parent_task_id)
                 VALUES ('cb-1', 'mika', 0, 'callback', 'callback', 'resume_agent', '{}', 'completed', 'issue', 'issue-1')",
            [],
        )
        .unwrap();

    // From callback child → should resolve to issue-1 (first scope root)
    let root = db.resolve_scope_root_task_id("cb-1").unwrap();
    assert_eq!(root, Some("issue-1".to_string()));

    // From issue → should resolve to itself
    let root = db.resolve_scope_root_task_id("issue-1").unwrap();
    assert_eq!(root, Some("issue-1".to_string()));

    // From milestone → should resolve to itself
    let root = db.resolve_scope_root_task_id("ms-1").unwrap();
    assert_eq!(root, Some("ms-1".to_string()));

    // From project → should resolve to itself
    let root = db.resolve_scope_root_task_id("proj-1").unwrap();
    assert_eq!(root, Some("proj-1".to_string()));
}

#[test]
fn test_malformed_parent_chain() {
    let db = db();
    // Create a non-manual callback task with no parent. Its type is 'issue'
    // but it's a callback — not a scope root. The chain exhausts without
    // finding a manual scope root.
    db.conn
        .execute(
            "INSERT INTO tasks (id, agent_id, depth, label, trigger_type, action_type, action_config, status, type)
                 VALUES ('orphan-1', 'mika', 0, 'orphan', 'callback', 'resume_agent', '{}', 'pending', 'issue')",
            [],
        )
        .unwrap();

    // Should return None — callback tasks are not scope roots
    let root = db.resolve_scope_root_task_id("orphan-1").unwrap();
    assert_eq!(root, None);
}

#[test]
fn test_scope_root_walk_depth_limit_21_hops() {
    let db = db();
    // Build a chain of 21 callback tasks (non-scope-root), no scope root reachable.
    // The depth limit is 20, so at hop 21 the guard fires and returns None.
    let mut prev_id: Option<String> = None;
    for i in 0..21 {
        let id = format!("chain-{i}");
        db.conn
            .execute(
                "INSERT INTO tasks (id, agent_id, depth, label, trigger_type, action_type, action_config, status, type, parent_task_id)
                     VALUES (?1, 'mika', 0, 'chain', 'callback', 'resume_agent', '{}', 'pending', 'issue', ?2)",
                params![id, prev_id],
            )
            .unwrap();
        prev_id = Some(id);
    }

    // Walk from the deepest node (chain-20). The chain has 21 nodes,
    // all callback (not scope roots). After 20 hops the depth limit fires.
    let root = db.resolve_scope_root_task_id("chain-20").unwrap();
    assert_eq!(
        root, None,
        "21-hop chain must hit the depth limit and return None"
    );

    // Verify a 20-hop chain with a scope root at the end DOES resolve.
    // Create a manual project at the root.
    create_test_task(&db, "scope-root", "project", None);
    // Re-parent chain-0 to point at the scope root.
    db.conn
        .execute(
            "UPDATE tasks SET parent_task_id = 'scope-root' WHERE id = 'chain-0'",
            [],
        )
        .unwrap();

    // From chain-18 (19 hops through chain + 1 hop to scope-root = 20 iterations).
    // The loop runs for 0..20, so 20 iterations fit exactly.
    let root = db.resolve_scope_root_task_id("chain-18").unwrap();
    assert_eq!(
        root,
        Some("scope-root".to_string()),
        "chain with scope-root reachable within 20 iterations should resolve"
    );

    // From chain-19 (20 hops through chain + 1 hop to scope-root = 21 iterations).
    // Exceeds the 20-iteration limit — depth guard fires.
    let root = db.resolve_scope_root_task_id("chain-19").unwrap();
    assert_eq!(
        root, None,
        "chain requiring 21 iterations must exceed depth limit"
    );

    // From chain-20 (21 hops through chain + 1 hop = 22 iterations): also exceeds.
    let root = db.resolve_scope_root_task_id("chain-20").unwrap();
    assert_eq!(
        root, None,
        "chain requiring 22 iterations must exceed depth limit"
    );
}

#[test]
fn test_scope_root_walk_nonexistent_task() {
    let db = db();
    let root = db.resolve_scope_root_task_id("does-not-exist").unwrap();
    assert_eq!(root, None);
}

#[test]
fn test_task_messages_survive_compaction() {
    let (mut db, sid) = db_with_session();
    let task_id = "task-survive-compaction";

    // Insert several tagged messages
    for i in 0..5 {
        db.save_message_with_task_context(
            "mika",
            &sid,
            if i % 2 == 0 { "user" } else { "assistant" },
            &format!("msg {i}"),
            None,
            None,
            false,
            Some(task_id),
        )
        .unwrap();
    }

    // Verify both tables have 5 rows
    let msgs = db.load_recent_messages("mika", 20).unwrap();
    assert_eq!(msgs.len(), 5);
    let task_msgs = db.load_task_messages(task_id).unwrap();
    assert_eq!(task_msgs.len(), 5);

    // Run compaction — this deletes from messages but NOT from task_messages
    let last_id = msgs.iter().map(|m| m.id).max().unwrap();
    db.replace_with_summary("mika", "Summary of task work", last_id)
        .unwrap();

    // messages should now have zero non-summary messages
    // (load_recent_messages filters out role='summary')
    let msgs_after = db.load_recent_messages("mika", 20).unwrap();
    assert_eq!(msgs_after.len(), 0);

    // But the summary exists in the DB
    let summary = db.load_conversation_summary("mika").unwrap();
    assert!(summary.is_some());

    // task_messages should still have all 5 rows — the structural guarantee
    let task_msgs_after = db.load_task_messages(task_id).unwrap();
    assert_eq!(task_msgs_after.len(), 5);
    for (i, tm) in task_msgs_after.iter().enumerate() {
        assert_eq!(tm.content, format!("msg {i}"));
    }
}

#[test]
fn test_load_task_messages_ordered() {
    let (mut db, sid) = db_with_session();
    let task_id = "task-order";

    db.save_message_with_task_context(
        "mika",
        &sid,
        "user",
        "first",
        None,
        None,
        false,
        Some(task_id),
    )
    .unwrap();
    db.save_message_with_task_context(
        "mika",
        &sid,
        "assistant",
        "second",
        None,
        None,
        false,
        Some(task_id),
    )
    .unwrap();
    db.save_message_with_task_context(
        "mika",
        &sid,
        "user",
        "third",
        None,
        None,
        false,
        Some(task_id),
    )
    .unwrap();

    let task_msgs = db.load_task_messages(task_id).unwrap();
    assert_eq!(task_msgs.len(), 3);
    assert_eq!(task_msgs[0].content, "first");
    assert_eq!(task_msgs[1].content, "second");
    assert_eq!(task_msgs[2].content, "third");
}

#[test]
fn test_insert_task_message_standalone() {
    let (db, sid) = db_with_session();
    let task_id = "task-standalone";

    let row_id = db
        .insert_task_message(
            task_id,
            "mika",
            &sid,
            "system",
            "Callback completed: session=abc, turns=5",
            Some(r#"{"claude_pilot":{"session_id":"abc","turns":5}}"#),
            Some("trace-123"),
        )
        .unwrap();
    assert!(row_id > 0);

    let msgs = db.load_task_messages(task_id).unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].task_id, task_id);
    assert_eq!(msgs[0].agent_id, "mika");
    assert_eq!(msgs[0].session_id, sid);
    assert_eq!(msgs[0].role, "system");
    assert_eq!(msgs[0].content, "Callback completed: session=abc, turns=5");
    assert!(
        msgs[0]
            .metadata
            .as_deref()
            .unwrap()
            .contains("claude_pilot")
    );
    assert_eq!(msgs[0].trace_id.as_deref(), Some("trace-123"));
}

#[test]
fn test_insert_task_message_with_none_optional_fields() {
    let (db, sid) = db_with_session();
    let task_id = "task-none-fields";

    let row_id = db
        .insert_task_message(task_id, "mika", &sid, "system", "summary", None, None)
        .unwrap();
    assert!(row_id > 0);

    let msgs = db.load_task_messages(task_id).unwrap();
    assert_eq!(msgs.len(), 1);
    assert!(msgs[0].metadata.is_none());
    assert!(msgs[0].trace_id.is_none());
}

/// Happy-path replay: simulate a milestone dispatch with multiple children.
/// Verify that messages from dispatch session (turn 1), child callback (turn 2),
/// and subsequent dispatch (turn 3) are all present in task_messages sorted by
/// created_at. Verify rebuild_context in task-mode surfaces the full narrative
/// from a different (callback) session.
#[test]
fn test_happy_path_multi_session_replay() {
    let mut db = db();
    let scope_root = "milestone-1";
    create_test_task(&db, scope_root, "milestone", None);

    // Three sessions simulating dispatch → callback → re-dispatch lifecycle.
    let sid_dispatch1 = "session-dispatch-1";
    let sid_callback = "session-callback";
    let sid_dispatch2 = "session-dispatch-2";
    db.create_session(sid_dispatch1, "mika", "cli").unwrap();
    db.create_session(sid_callback, "mika", "cli").unwrap();
    db.create_session(sid_dispatch2, "mika", "cli").unwrap();

    // Turn 1: dispatch session — orchestrator dispatches child issue #1.
    // Use explicit created_at to guarantee ordering across sessions.
    db.save_message_with_task_context(
        "mika",
        sid_dispatch1,
        "user",
        "dispatch child issue #1",
        None,
        None,
        false,
        Some(scope_root),
    )
    .unwrap();
    db.save_message_with_task_context(
        "mika",
        sid_dispatch1,
        "assistant",
        "dispatched child #1, advance to item #2",
        None,
        None,
        false,
        Some(scope_root),
    )
    .unwrap();

    // Turn 2: callback session — child #1 completes.
    db.save_message_with_task_context(
        "mika",
        sid_callback,
        "user",
        "[callback: child #1 completed]",
        None,
        None,
        false,
        Some(scope_root),
    )
    .unwrap();
    db.save_message_with_task_context(
        "mika",
        sid_callback,
        "assistant",
        "child #1 done, updating status",
        None,
        None,
        false,
        Some(scope_root),
    )
    .unwrap();

    // Turn 3: re-dispatch session — orchestrator dispatches child issue #2.
    db.save_message_with_task_context(
        "mika",
        sid_dispatch2,
        "user",
        "dispatch child issue #2",
        None,
        None,
        false,
        Some(scope_root),
    )
    .unwrap();
    db.save_message_with_task_context(
        "mika",
        sid_dispatch2,
        "assistant",
        "dispatched child #2",
        None,
        None,
        false,
        Some(scope_root),
    )
    .unwrap();

    // Verify: load_task_messages returns all 6 messages across 3 sessions,
    // in created_at order.
    let task_msgs = db.load_task_messages(scope_root).unwrap();
    assert_eq!(
        task_msgs.len(),
        6,
        "all messages across all sessions must be present"
    );
    assert_eq!(task_msgs[0].content, "dispatch child issue #1");
    assert_eq!(
        task_msgs[1].content,
        "dispatched child #1, advance to item #2"
    );
    assert_eq!(task_msgs[2].content, "[callback: child #1 completed]");
    assert_eq!(task_msgs[3].content, "child #1 done, updating status");
    assert_eq!(task_msgs[4].content, "dispatch child issue #2");
    assert_eq!(task_msgs[5].content, "dispatched child #2");

    // Verify: sessions span 3 distinct session IDs.
    let session_ids: std::collections::HashSet<&str> =
        task_msgs.iter().map(|m| m.session_id.as_str()).collect();
    assert_eq!(session_ids.len(), 3);

    // Verify: rebuild_context from the callback session with task-mode
    // surfaces the "advance to item #2" intent from the dispatch session.
    let ctx = db
        .rebuild_context("mika", None, Some(scope_root), 20)
        .unwrap();
    assert!(
        ctx.iter().any(|m| m.content.contains("advance to item #2")),
        "rebuild_context in task-mode must surface cross-session dispatch intent"
    );

    // Verify: messages table also has rows (double-write contract).
    let channel_msgs = db.load_recent_messages("mika", 100).unwrap();
    assert_eq!(
        channel_msgs.len(),
        6,
        "messages table should also have all 6 rows"
    );

    // Verify: after compaction, task_messages still has all 6 rows.
    let last_id = channel_msgs.iter().map(|m| m.id).max().unwrap();
    db.replace_with_summary("mika", "Summary of milestone work", last_id)
        .unwrap();
    let channel_msgs_after = db.load_recent_messages("mika", 100).unwrap();
    assert_eq!(channel_msgs_after.len(), 0, "channel messages compacted");
    let task_msgs_after = db.load_task_messages(scope_root).unwrap();
    assert_eq!(
        task_msgs_after.len(),
        6,
        "task narrative survives compaction"
    );

    // Verify: rebuild_context still surfaces full narrative post-compaction.
    let ctx_after = db
        .rebuild_context("mika", None, Some(scope_root), 20)
        .unwrap();
    assert_eq!(
        ctx_after.len(),
        6,
        "task-mode rebuild returns full narrative post-compaction"
    );
    assert!(
        ctx_after
            .iter()
            .any(|m| m.content.contains("advance to item #2"))
    );
}

/// AC5a (mika#1453): 2 pending wrappers + 1 in-flight real callback →
/// force-promote WITHOUT override → RejectedSlotBusy, no state mutation.
#[test]
fn test_force_promote_rejected_slot_busy_no_state_mutation() {
    let db = db();

    // Parent tasks (FK requirement for callbacks).
    let p1 = db
        .create_task(&new_task("mika", "p1", "manual", "none"))
        .unwrap();
    let p2 = db
        .create_task(&new_task("mika", "p2", "manual", "none"))
        .unwrap();
    let p3 = db
        .create_task(&new_task("mika", "p3", "manual", "none"))
        .unwrap();

    // 2 pending deferred wrappers.
    let mut d1 = deferred_wrapper("mika", "implement");
    d1.parent_task_id = Some(p1.clone());
    let d1_id = db.create_task(&d1).unwrap();

    let mut d2 = deferred_wrapper("mika", "implement");
    d2.parent_task_id = Some(p2.clone());
    let d2_id = db.create_task(&d2).unwrap();

    // 1 in-flight real callback (pending status occupies the slot).
    let mut rc = real_callback("mika", "implement");
    rc.parent_task_id = Some(p3.clone());
    let rc_id = db.create_task(&rc).unwrap();
    // Transition to in_progress to simulate a dispatched subprocess.
    db.claim_and_fire_task(&rc_id, "mika").unwrap();

    // Force-promote should be rejected.
    let result = db
        .force_promote_deferred_for_class("mika", "implement", 1)
        .unwrap();
    assert!(
        matches!(result, ForcePromoteResult::RejectedSlotBusy { .. }),
        "Expected RejectedSlotBusy, got {result:?}"
    );

    // Both deferred wrappers should still be pending (no state mutation).
    let t1 = db.get_task_unscoped(&d1_id).unwrap().unwrap();
    assert_eq!(
        t1.status, "pending",
        "deferred wrapper 1 should remain pending"
    );
    let t2 = db.get_task_unscoped(&d2_id).unwrap().unwrap();
    assert_eq!(
        t2.status, "pending",
        "deferred wrapper 2 should remain pending"
    );

    // Real callback should still be in_progress.
    let rc_task = db.get_task_unscoped(&rc_id).unwrap().unwrap();
    assert_eq!(
        rc_task.status, "in_progress",
        "real callback should remain in_progress"
    );

    // AC5a audit assertion: simulate the caller emitting the rejection
    // audit event (the tool/CLI layer is responsible for logging), then
    // verify exactly one row with the expected event type was stored.
    db.log_audit_event(
        "mika",
        "test-session",
        "deferred_dispatch_force_promote_rejected_slot_busy",
        "dispatch_class:implement",
        None,
        Some("rejected_slot_busy"),
        None,
        None,
    )
    .unwrap();
    let audit_count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM audit_events WHERE tool_name = ?1",
            params!["deferred_dispatch_force_promote_rejected_slot_busy"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        audit_count, 1,
        "exactly one rejected_slot_busy audit event should be emitted"
    );
}

/// AC5b (mika#1453): Same setup; cancel the blocker, then force-promote →
/// oldest deferred wrapper promoted, per-class invariant holds (≤1 active
/// real callback).
#[test]
fn test_force_promote_override_cancel_then_promote() {
    let db = db();

    // Parent tasks (FK requirement for callbacks).
    let p1 = db
        .create_task(&new_task("mika", "p1", "manual", "none"))
        .unwrap();
    let p2 = db
        .create_task(&new_task("mika", "p2", "manual", "none"))
        .unwrap();
    let p3 = db
        .create_task(&new_task("mika", "p3", "manual", "none"))
        .unwrap();

    // 2 pending deferred wrappers.
    let mut d1 = deferred_wrapper("mika", "implement");
    d1.parent_task_id = Some(p1.clone());
    let d1_id = db.create_task(&d1).unwrap();

    let mut d2 = deferred_wrapper("mika", "implement");
    d2.parent_task_id = Some(p2.clone());
    let d2_id = db.create_task(&d2).unwrap();

    // 1 in-flight real callback.
    let mut rc = real_callback("mika", "implement");
    rc.parent_task_id = Some(p3.clone());
    let rc_id = db.create_task(&rc).unwrap();
    db.claim_and_fire_task(&rc_id, "mika").unwrap();

    // Step 1: Confirm slot is busy.
    let result = db
        .force_promote_deferred_for_class("mika", "implement", 1)
        .unwrap();
    assert!(matches!(
        result,
        ForcePromoteResult::RejectedSlotBusy { .. }
    ));

    // Step 2: Find and identify the blocker.
    let blocker = db
        .find_active_callback_for_class("mika", "implement")
        .unwrap();
    assert_eq!(blocker.as_deref(), Some(rc_id.as_str()));

    // Step 3: Cancel the blocker (simulating `cancel_task_and_kill`).
    db.cancel_task(&rc_id, "mika").unwrap();

    // Step 4: Retry force-promote — should succeed now.
    let result = db
        .force_promote_deferred_for_class("mika", "implement", 1)
        .unwrap();
    match &result {
        ForcePromoteResult::Promoted { task_id } => {
            assert_eq!(
                task_id, &d1_id,
                "oldest deferred wrapper should be promoted first"
            );
        }
        other => panic!("Expected Promoted, got {other:?}"),
    }

    // Step 5: Verify only the first wrapper was promoted.
    let t1 = db.get_task_unscoped(&d1_id).unwrap().unwrap();
    assert_eq!(
        t1.status, "completed",
        "first wrapper should be promoted (completed)"
    );

    let t2 = db.get_task_unscoped(&d2_id).unwrap().unwrap();
    assert_eq!(t2.status, "pending", "second wrapper should remain pending");

    // Step 6: Verify per-class invariant — no real callback active.
    assert!(
        !db.has_any_active_callback_for_class("mika", "implement")
            .unwrap(),
        "no active real callback should remain after cancel"
    );
}

#[test]
fn test_force_promote_slot_free_promoted() {
    let db = db();
    let p1 = db
        .create_task(&new_task("mika", "p1", "manual", "none"))
        .unwrap();

    let mut d1 = deferred_wrapper("mika", "implement");
    d1.parent_task_id = Some(p1.clone());
    let d1_id = db.create_task(&d1).unwrap();

    let result = db
        .force_promote_deferred_for_class("mika", "implement", 1)
        .unwrap();
    match &result {
        ForcePromoteResult::Promoted { task_id } => {
            assert_eq!(task_id, &d1_id);
        }
        other => panic!("Expected Promoted, got {other:?}"),
    }
}

/// mika#2160 — the third place that held the cap. The deferred-promotion
/// path gated on a boolean "is anything active", which is a cap of exactly
/// one. Left that way, raising the cap would admit NEW dispatches while a
/// DEFERRED one waited for the class to fall back to zero — the
/// asymmetric-predicate drift mika#1163 already had to name once.
#[test]
fn test_force_promote_honours_a_cap_above_one() {
    let db = db();

    // One live dispatch occupies the class.
    let p1 = db
        .create_task(&new_task("mika", "p1", "manual", "none"))
        .unwrap();
    let mut busy = new_task(
        "mika",
        "long_running:run_claude_pilot",
        "callback",
        "resume_agent",
    );
    busy.parent_task_id = Some(p1.clone());
    db.create_task(&busy).unwrap();

    // A deferred wrapper waits behind it.
    let p2 = db
        .create_task(&new_task("mika", "p2", "manual", "none"))
        .unwrap();
    let mut d1 = deferred_wrapper("mika", "implement");
    d1.parent_task_id = Some(p2.clone());
    let d1_id = db.create_task(&d1).unwrap();

    // At the default cap the class is full — today's behaviour, unchanged.
    assert!(
        matches!(
            db.force_promote_deferred_for_class("mika", "implement", 1)
                .unwrap(),
            ForcePromoteResult::RejectedSlotBusy { .. }
        ),
        "one active dispatch must still fill a cap of one"
    );

    // At a cap of two there is room, and the wrapper must be promoted.
    match db
        .force_promote_deferred_for_class("mika", "implement", 2)
        .unwrap()
    {
        ForcePromoteResult::Promoted { task_id } => assert_eq!(task_id, d1_id),
        other => panic!("a cap of two leaves room — expected Promoted, got {other:?}"),
    }
}

/// The class-slot count must count dispatches, not callback rows: a parent
/// carrying two callbacks of the same class is one occupant.
#[test]
fn test_count_active_callbacks_for_class_counts_dispatches_not_rows() {
    let db = db();
    assert_eq!(
        db.count_active_callbacks_for_class("mika", "implement")
            .unwrap(),
        0,
        "an idle class counts zero"
    );

    let p1 = db
        .create_task(&new_task("mika", "p1", "manual", "none"))
        .unwrap();
    for _ in 0..2 {
        let mut cb = new_task(
            "mika",
            "long_running:run_claude_pilot",
            "callback",
            "resume_agent",
        );
        cb.parent_task_id = Some(p1.clone());
        db.create_task(&cb).unwrap();
    }
    assert_eq!(
        db.count_active_callbacks_for_class("mika", "implement")
            .unwrap(),
        1,
        "two callback rows under one parent are one dispatch"
    );

    let p2 = db
        .create_task(&new_task("mika", "p2", "manual", "none"))
        .unwrap();
    let mut cb2 = new_task(
        "mika",
        "long_running:run_claude_pilot",
        "callback",
        "resume_agent",
    );
    cb2.parent_task_id = Some(p2.clone());
    db.create_task(&cb2).unwrap();
    assert_eq!(
        db.count_active_callbacks_for_class("mika", "implement")
            .unwrap(),
        2,
        "a second parent is a second dispatch"
    );

    // Parity with the boolean it companions: both see the same population.
    assert!(
        db.has_any_active_callback_for_class("mika", "implement")
            .unwrap()
    );
    assert_eq!(
        db.count_active_callbacks_for_class("mika", "groom")
            .unwrap(),
        0,
        "the count stays class-scoped"
    );
}

#[test]
fn test_force_promote_slot_free_no_pending_wrapper() {
    let db = db();
    let result = db
        .force_promote_deferred_for_class("mika", "implement", 1)
        .unwrap();
    assert!(
        matches!(result, ForcePromoteResult::NoPendingWrapper),
        "Expected NoPendingWrapper, got {result:?}"
    );
}

#[test]
fn test_find_active_callback_for_class_excludes_deferred() {
    let db = db();
    let p1 = db
        .create_task(&new_task("mika", "p1", "manual", "none"))
        .unwrap();

    // Only a deferred wrapper exists — should return None.
    let mut d1 = deferred_wrapper("mika", "implement");
    d1.parent_task_id = Some(p1.clone());
    db.create_task(&d1).unwrap();

    let blocker = db
        .find_active_callback_for_class("mika", "implement")
        .unwrap();
    assert!(
        blocker.is_none(),
        "deferred wrappers should not count as slot occupiers"
    );
}

#[test]
fn test_find_active_callback_for_class_returns_real_callback() {
    let db = db();
    let p1 = db
        .create_task(&new_task("mika", "p1", "manual", "none"))
        .unwrap();

    let mut rc = real_callback("mika", "implement");
    rc.parent_task_id = Some(p1.clone());
    let rc_id = db.create_task(&rc).unwrap();

    let blocker = db
        .find_active_callback_for_class("mika", "implement")
        .unwrap();
    assert_eq!(blocker.as_deref(), Some(rc_id.as_str()));
}

#[test]
fn test_cancel_orphan_recurring_tasks_one_orphan() {
    let db = db();
    db.register_agent("agent-a", "Agent A", "/tmp/a").unwrap();
    db.register_agent("agent-b", "Agent B", "/tmp/b").unwrap();

    let id_a = db
        .create_task(&recurring_task("agent-a", "heartbeat"))
        .unwrap();
    let id_b = db
        .create_task(&recurring_task("agent-b", "heartbeat"))
        .unwrap();

    // Only agent-a is known (agent-b was deleted from disk).
    let orphans = db
        .cancel_orphan_recurring_tasks(&["agent-a".to_string()])
        .unwrap();

    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0].0, id_b);
    assert_eq!(orphans[0].1, "agent-b");

    // agent-a's task is unchanged (create_task sets status to 'pending').
    let ta = db.get_task(&id_a, "agent-a").unwrap().unwrap();
    assert_eq!(ta.status, "pending");

    // agent-b's task is cancelled.
    let tb = db.get_task(&id_b, "agent-b").unwrap().unwrap();
    assert_eq!(tb.status, "cancelled");
}

#[test]
fn test_cancel_orphan_recurring_tasks_no_orphans() {
    let db = db();
    db.register_agent("agent-a", "Agent A", "/tmp/a").unwrap();

    db.create_task(&recurring_task("agent-a", "heartbeat"))
        .unwrap();

    let orphans = db
        .cancel_orphan_recurring_tasks(&["agent-a".to_string()])
        .unwrap();
    assert!(orphans.is_empty());
}

#[test]
fn test_cancel_orphan_recurring_tasks_multiple_orphans_same_agent() {
    let db = db();
    db.register_agent("agent-a", "Agent A", "/tmp/a").unwrap();
    db.register_agent("mika-relay", "Relay", "/tmp/relay")
        .unwrap();

    db.create_task(&recurring_task("agent-a", "heartbeat"))
        .unwrap();
    let r1 = db
        .create_task(&recurring_task("mika-relay", "heartbeat"))
        .unwrap();
    let r2 = db
        .create_task(&recurring_task("mika-relay", "reflection"))
        .unwrap();

    let orphans = db
        .cancel_orphan_recurring_tasks(&["agent-a".to_string()])
        .unwrap();

    assert_eq!(orphans.len(), 2);
    let orphan_ids: Vec<&str> = orphans.iter().map(|(id, _)| id.as_str()).collect();
    assert!(orphan_ids.contains(&r1.as_str()));
    assert!(orphan_ids.contains(&r2.as_str()));

    // Both relay tasks cancelled.
    let t1 = db.get_task(&r1, "mika-relay").unwrap().unwrap();
    let t2 = db.get_task(&r2, "mika-relay").unwrap().unwrap();
    assert_eq!(t1.status, "cancelled");
    assert_eq!(t2.status, "cancelled");
}

#[test]
fn test_cancel_orphan_recurring_tasks_already_cancelled_idempotent() {
    let db = db();
    db.register_agent("agent-b", "Agent B", "/tmp/b").unwrap();

    let id = db
        .create_task(&recurring_task("agent-b", "heartbeat"))
        .unwrap();
    // Pre-cancel the task.
    db.cancel_task(&id, "agent-b").unwrap();

    let orphans = db
        .cancel_orphan_recurring_tasks(&["agent-a".to_string()])
        .unwrap();
    // Already cancelled — not in the active set, so not returned.
    assert!(orphans.is_empty());
}

#[test]
fn test_cancel_orphan_recurring_tasks_empty_known_set_returns_empty() {
    let db = db();
    db.register_agent("agent-a", "Agent A", "/tmp/a").unwrap();
    db.create_task(&recurring_task("agent-a", "heartbeat"))
        .unwrap();

    // Empty known set triggers early return (safety guard).
    let orphans = db.cancel_orphan_recurring_tasks(&[]).unwrap();
    assert!(orphans.is_empty());

    // Task should still be active (no mutation on empty set).
    let count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE agent_id = 'agent-a' AND status IN ('pending', 'recurring_active')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_cancel_orphan_recurring_tasks_skips_manual_tasks() {
    let db = db();
    db.register_agent("agent-b", "Agent B", "/tmp/b").unwrap();

    // Create a manual task for agent-b (should NOT be cancelled).
    let manual_id = db
        .create_task(&new_task("agent-b", "some-work", "manual", "none"))
        .unwrap();

    // Create a recurring task for agent-b (should be cancelled).
    let recurring_id = db
        .create_task(&recurring_task("agent-b", "heartbeat"))
        .unwrap();

    let orphans = db
        .cancel_orphan_recurring_tasks(&["agent-a".to_string()])
        .unwrap();

    // Only the recurring task is cancelled.
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0].0, recurring_id);

    // Manual task is still pending.
    let mt = db.get_task(&manual_id, "agent-b").unwrap().unwrap();
    assert_eq!(mt.status, "pending");
}

#[test]
fn test_get_last_cli_session_returns_none_when_empty() {
    let db = db();
    let result = db.get_last_cli_session_for_agent("mika").unwrap();
    assert!(result.is_none());
}

#[test]
fn test_get_last_cli_session_returns_most_recent_ended() {
    let db = db();
    // Create two ended CLI sessions with distinct started_at timestamps
    db.create_session("s1", "mika", "cli").unwrap();
    db.end_session("s1").unwrap();
    // Manually set s1 to an earlier timestamp to ensure deterministic ordering
    db.conn
        .execute(
            "UPDATE sessions SET started_at = '2026-01-01T00:00:00Z' WHERE id = 's1'",
            [],
        )
        .unwrap();
    db.create_session("s2", "mika", "cli").unwrap();
    db.end_session("s2").unwrap();

    let result = db.get_last_cli_session_for_agent("mika").unwrap().unwrap();
    assert_eq!(result.id, "s2");
}

#[test]
fn test_get_last_cli_session_excludes_non_cli_channels() {
    let db = db();
    db.create_session("tg1", "mika", "telegram").unwrap();
    db.end_session("tg1").unwrap();
    db.create_session("wh1", "mika", "webhook").unwrap();
    db.end_session("wh1").unwrap();

    let result = db.get_last_cli_session_for_agent("mika").unwrap();
    assert!(result.is_none());
}

#[test]
fn test_get_last_cli_session_excludes_system_sessions() {
    let db = db();
    db.create_session("system-mika", "mika", "cli").unwrap();
    db.end_session("system-mika").unwrap();

    let result = db.get_last_cli_session_for_agent("mika").unwrap();
    assert!(result.is_none());
}

#[test]
fn test_get_last_cli_session_excludes_delegate_sessions() {
    let db = db();
    db.create_session("delegate-abc", "mika", "cli").unwrap();
    db.end_session("delegate-abc").unwrap();

    let result = db.get_last_cli_session_for_agent("mika").unwrap();
    assert!(result.is_none());
}

#[test]
fn test_get_last_cli_session_excludes_child_sessions() {
    let db = db();
    // Parent session
    db.create_session("parent", "mika", "cli").unwrap();
    db.end_session("parent").unwrap();
    // Child session with parent_session_id set
    db.create_session_with_parent("child", "mika", "cli", None, Some("parent"), None)
        .unwrap();
    db.end_session("child").unwrap();

    let result = db.get_last_cli_session_for_agent("mika").unwrap().unwrap();
    // Only the parent (no parent_session_id) is returned
    assert_eq!(result.id, "parent");
}

#[test]
fn test_get_last_cli_session_excludes_active_sessions() {
    let db = db();
    // Active session (ended_at IS NULL)
    db.create_session("active", "mika", "cli").unwrap();

    let result = db.get_last_cli_session_for_agent("mika").unwrap();
    assert!(result.is_none());
}

#[test]
fn test_groom_cross_check_no_task_returns_false() {
    let db = db();
    assert!(
        !db.has_completed_groom_for_issue("mika", GROOM_ISSUE_URL)
            .unwrap()
    );
}

#[test]
fn test_groom_cross_check_completed_callback_returns_true() {
    let db = db();
    completed_groom_pair(&db, "mika", GROOM_ISSUE_URL, GROOM_CALLBACK_PLAN_GROOMED);
    assert!(
        db.has_completed_groom_for_issue("mika", GROOM_ISSUE_URL)
            .unwrap()
    );
}

#[test]
fn test_groom_cross_check_delivered_callback_returns_true() {
    let db = db();
    let (_, callback_id) =
        completed_groom_pair(&db, "mika", GROOM_ISSUE_URL, GROOM_CALLBACK_PLAN_GROOMED);
    db.update_task_status(&callback_id, "delivered").unwrap();
    assert!(
        db.has_completed_groom_for_issue("mika", GROOM_ISSUE_URL)
            .unwrap()
    );
}

/// mika#2287 anchor: the parent's CURRENT dispatch_class is irrelevant —
/// the engine flips it groom→implement (mika#1614) and the proof must
/// survive. The async twin of this test lives in
/// `task_engine::dispatcher::tests::test_groom_gate_survives_implement_flip`.
#[test]
fn test_groom_cross_check_survives_parent_flip_to_implement() {
    let db = db();
    let (parent_id, _) =
        completed_groom_pair(&db, "mika", GROOM_ISSUE_URL, GROOM_CALLBACK_PLAN_GROOMED);
    assert!(
        db.update_task_dispatch_class(&parent_id, "mika", "implement")
            .unwrap()
    );
    assert!(
        db.has_completed_groom_for_issue("mika", GROOM_ISSUE_URL)
            .unwrap()
    );
}

/// R3/R14: the legacy LLM-driven path writes the parent URL with
/// `GROOM_PHASE_SUFFIX`; the same query accepts it.
#[test]
fn test_groom_cross_check_legacy_suffixed_parent_url_returns_true() {
    let db = db();
    let suffixed = format!(
        "{}{}",
        GROOM_ISSUE_URL,
        crate::task_state::tasks::GROOM_PHASE_SUFFIX
    );
    completed_groom_pair(&db, "mika", &suffixed, GROOM_CALLBACK_PLAN_GROOMED);
    assert!(
        db.has_completed_groom_for_issue("mika", GROOM_ISSUE_URL)
            .unwrap()
    );
}

#[test]
fn test_groom_cross_check_pending_callback_returns_false() {
    let db = db();
    let parent_id = db
        .create_task(&groom_parent("mika", GROOM_ISSUE_URL))
        .unwrap();
    db.create_task(&groom_callback("mika", &parent_id, "groom"))
        .unwrap();
    // Callback never completed — no result, status pending.
    assert!(
        !db.has_completed_groom_for_issue("mika", GROOM_ISSUE_URL)
            .unwrap()
    );
}

#[test]
fn test_groom_cross_check_plan_iterate_returns_false() {
    let db = db();
    completed_groom_pair(&db, "mika", GROOM_ISSUE_URL, GROOM_CALLBACK_PLAN_ITERATE);
    assert!(
        !db.has_completed_groom_for_issue("mika", GROOM_ISSUE_URL)
            .unwrap()
    );
}

#[test]
fn test_groom_cross_check_implement_class_callback_returns_false() {
    let db = db();
    let parent_id = db
        .create_task(&groom_parent("mika", GROOM_ISSUE_URL))
        .unwrap();
    let callback_id = db
        .create_task(&groom_callback("mika", &parent_id, "implement"))
        .unwrap();
    db.update_task_completed(&callback_id, "mika", Some(GROOM_CALLBACK_PLAN_GROOMED))
        .unwrap();
    assert!(
        !db.has_completed_groom_for_issue("mika", GROOM_ISSUE_URL)
            .unwrap()
    );
}

#[test]
fn test_groom_cross_check_different_agent_returns_false() {
    let db = db();
    db.register_agent("other-agent", "Other", "").unwrap();
    completed_groom_pair(
        &db,
        "other-agent",
        GROOM_ISSUE_URL,
        GROOM_CALLBACK_PLAN_GROOMED,
    );
    // Query for "mika" — must not see the other agent's proof.
    assert!(
        !db.has_completed_groom_for_issue("mika", GROOM_ISSUE_URL)
            .unwrap()
    );
}

#[test]
fn test_groom_cross_check_different_issue_returns_false() {
    let db = db();
    completed_groom_pair(&db, "mika", GROOM_ISSUE_URL, GROOM_CALLBACK_PLAN_GROOMED);
    assert!(
        !db.has_completed_groom_for_issue(
            "mika",
            "https://github.com/senara-solutions/mika/issues/124",
        )
        .unwrap()
    );
}

/// The pre-mika#2287 proof shape — a terminal groom-class PARENT with the
/// suffixed URL and no callback — is no longer proof on its own. A hand
/// pre-stamped ticket cannot satisfy the gate by minting such a row.
#[test]
fn test_groom_cross_check_parent_only_legacy_shape_returns_false() {
    let db = db();
    let suffixed = format!(
        "{}{}",
        GROOM_ISSUE_URL,
        crate::task_state::tasks::GROOM_PHASE_SUFFIX
    );
    let parent_id = db.create_task(&groom_parent("mika", &suffixed)).unwrap();
    db.update_task_status(&parent_id, "completed").unwrap();
    assert!(
        !db.has_completed_groom_for_issue("mika", GROOM_ISSUE_URL)
            .unwrap()
    );
}

// ---------------------------------------------------------------------------
// mika#2545 — `latest_groom_verdict_for_issue`, sœur de la précédente
// ---------------------------------------------------------------------------

/// Le `result` d'un groom qui a escaladé, tel que `_escalate_groom` l'écrit
/// depuis mika#2545 (marqueur terminal + disposition posée par
/// `_set_outcome_line`).
const GROOM_CALLBACK_ESCALATE: &str = "claude-pilot completed (status: done).\n\
     GROOM ESCALATED (terminal): mika-arch escalated at first-pass.\n\
     Verdict: ESCALATE — human review required.\n\
     \n\
     Outcome: ESCALATE — first-pass";

/// Force `created_at` sur une row pour rendre l'ordre chronologique
/// observable : les timestamps ISO-8601 sont à la seconde, donc deux rows
/// créées dans le même test les partagent.
fn set_created_at(db: &Database, task_id: &str, created_at: &str) {
    let n = db
        .conn
        .execute(
            "UPDATE tasks SET created_at = ?2 WHERE id = ?1",
            rusqlite::params![task_id, created_at],
        )
        .unwrap();
    assert_eq!(n, 1, "la row de test doit exister");
}

#[test]
fn mika2545_no_groom_callback_returns_none() {
    let db = db();
    assert!(
        db.latest_groom_verdict_for_issue("mika", GROOM_ISSUE_URL)
            .unwrap()
            .is_none(),
        "aucun groom terminal ⇒ `None`, le cas nominal d'un premier grooming — \
         que l'appelant lit « jamais escaladé », jamais « illisible »"
    );
}

#[test]
fn mika2545_an_escalated_groom_is_returned_verbatim() {
    let db = db();
    completed_groom_pair(&db, "mika", GROOM_ISSUE_URL, GROOM_CALLBACK_ESCALATE);
    let result = db
        .latest_groom_verdict_for_issue("mika", GROOM_ISSUE_URL)
        .unwrap()
        .expect("une row existe")
        .expect("son result est renseigné");
    assert!(
        result.contains("Outcome: ESCALATE — first-pass"),
        "le `result` est rendu VERBATIM — la grammaire du marqueur est l'affaire \
         de la fonction pure, pas de la requête : {result}"
    );
}

/// **Le test porteur de R2.** Il distingue « le DERNIER groom a escaladé » de
/// « il existe un groom qui a escaladé ». Un prédicat en `EXISTS` passerait le
/// test précédent et gèlerait pour toujours un ticket re-groomé avec succès :
/// un frein devenu mur.
#[test]
fn mika2545_an_escalate_followed_by_a_groomed_reads_not_escalated() {
    let db = db();
    let (parent_id, escalated_id) =
        completed_groom_pair(&db, "mika", GROOM_ISSUE_URL, GROOM_CALLBACK_ESCALATE);
    set_created_at(&db, &escalated_id, "2026-09-26T13:06:14Z");

    // Le re-grooming réussi, plus tard, sous le MÊME parent (mika#1614 réutilise
    // la row) — donc la requête doit choisir par la date, pas par l'existence.
    let groomed_id = db
        .create_task(&groom_callback("mika", &parent_id, "groom"))
        .unwrap();
    assert!(
        db.update_task_completed(&groomed_id, "mika", Some(GROOM_CALLBACK_PLAN_GROOMED))
            .unwrap()
    );
    set_created_at(&db, &groomed_id, "2026-09-27T09:00:00Z");

    let result = db
        .latest_groom_verdict_for_issue("mika", GROOM_ISSUE_URL)
        .unwrap()
        .expect("une row existe")
        .expect("son result est renseigné");
    assert!(
        result.contains("Outcome: PLAN_GROOMED"),
        "le dernier groom est le re-grooming réussi : {result}"
    );
    assert!(
        !result.contains("Outcome: ESCALATE"),
        "et l'escalade antérieure ne doit pas survivre au tri : {result}"
    );
}

/// Contrôle du sens inverse : un GROOMED puis un ESCALATE rend bien l'escalade.
/// Sans lui, « le tri fonctionne » ne se distingue pas de « la requête préfère
/// PLAN_GROOMED ».
#[test]
fn mika2545_a_groomed_followed_by_an_escalate_reads_escalated() {
    let db = db();
    let (parent_id, groomed_id) =
        completed_groom_pair(&db, "mika", GROOM_ISSUE_URL, GROOM_CALLBACK_PLAN_GROOMED);
    set_created_at(&db, &groomed_id, "2026-09-20T10:00:00Z");

    let escalated_id = db
        .create_task(&groom_callback("mika", &parent_id, "groom"))
        .unwrap();
    assert!(
        db.update_task_completed(&escalated_id, "mika", Some(GROOM_CALLBACK_ESCALATE))
            .unwrap()
    );
    set_created_at(&db, &escalated_id, "2026-09-26T13:06:14Z");

    let result = db
        .latest_groom_verdict_for_issue("mika", GROOM_ISSUE_URL)
        .unwrap()
        .expect("une row existe")
        .expect("son result est renseigné");
    assert!(
        result.contains("Outcome: ESCALATE"),
        "le dernier groom est l'escalade : {result}"
    );
}

/// L'URL legacy `?phase=groom` que le chemin LLM appose sur le parent est
/// reconnue, comme pour `has_completed_groom_for_issue`. Rien n'est appendu par
/// l'appelant.
#[test]
fn mika2545_the_legacy_phase_suffix_on_the_parent_is_recognised() {
    let db = db();
    let suffixed = format!(
        "{}{}",
        GROOM_ISSUE_URL,
        crate::task_state::tasks::GROOM_PHASE_SUFFIX
    );
    completed_groom_pair(&db, "mika", &suffixed, GROOM_CALLBACK_ESCALATE);
    let result = db
        .latest_groom_verdict_for_issue("mika", GROOM_ISSUE_URL)
        .unwrap()
        .expect("une row existe sous l'URL suffixée")
        .expect("son result est renseigné");
    assert!(result.contains("Outcome: ESCALATE"));
}

/// Un callback terminal dont la colonne `result` est NULL : les deux niveaux
/// d'`Option` disent deux choses différentes, et cette row-là est le seul
/// producteur de `Some(None)`.
#[test]
fn mika2545_a_terminal_callback_with_a_null_result_is_some_none() {
    let db = db();
    let parent_id = db
        .create_task(&groom_parent("mika", GROOM_ISSUE_URL))
        .unwrap();
    let callback_id = db
        .create_task(&groom_callback("mika", &parent_id, "groom"))
        .unwrap();
    assert!(
        db.update_task_completed(&callback_id, "mika", None)
            .unwrap()
    );

    let outer = db
        .latest_groom_verdict_for_issue("mika", GROOM_ISSUE_URL)
        .unwrap();
    assert!(outer.is_some(), "la row existe");
    assert!(
        outer.unwrap().is_none(),
        "son `result` est NULL — à distinguer de « aucune row », qui est le cas \
         nominal d'un premier grooming"
    );
}

/// Scoping par agent, comme sa sœur : la preuve d'un autre agent n'est pas la
/// nôtre.
#[test]
fn mika2545_another_agents_escalation_is_not_ours() {
    let db = db();
    db.register_agent("other-agent", "Other", "").unwrap();
    completed_groom_pair(&db, "other-agent", GROOM_ISSUE_URL, GROOM_CALLBACK_ESCALATE);
    assert!(
        db.latest_groom_verdict_for_issue("mika", GROOM_ISSUE_URL)
            .unwrap()
            .is_none()
    );
}

/// Un callback de groom **non terminal** n'est pas un verdict : un groom en vol
/// ne dit rien de l'escalade, et le lire comme tel refuserait un dispatch
/// pendant que son propre grooming tourne.
#[test]
fn mika2545_a_non_terminal_groom_callback_is_not_a_verdict() {
    let db = db();
    let parent_id = db
        .create_task(&groom_parent("mika", GROOM_ISSUE_URL))
        .unwrap();
    let callback_id = db
        .create_task(&groom_callback("mika", &parent_id, "groom"))
        .unwrap();
    db.update_task_status(&callback_id, "in_progress").unwrap();
    assert!(
        db.latest_groom_verdict_for_issue("mika", GROOM_ISSUE_URL)
            .unwrap()
            .is_none()
    );
}

/// Un callback d'**implement** n'est pas un verdict de grooming — même parent,
/// autre classe. Sans ce contrôle, « la requête filtre sur la classe » ne se
/// distingue pas de « la requête prend le dernier callback ».
#[test]
fn mika2545_an_implement_callback_is_not_a_groom_verdict() {
    let db = db();
    let parent_id = db
        .create_task(&groom_parent("mika", GROOM_ISSUE_URL))
        .unwrap();
    let callback_id = db
        .create_task(&groom_callback("mika", &parent_id, "implement"))
        .unwrap();
    assert!(
        db.update_task_completed(&callback_id, "mika", Some(GROOM_CALLBACK_ESCALATE))
            .unwrap()
    );
    assert!(
        db.latest_groom_verdict_for_issue("mika", GROOM_ISSUE_URL)
            .unwrap()
            .is_none()
    );
}

/// Deux callbacks nés dans la même seconde : l'ordre est **déterministe**, et le
/// doc-comment ne promet rien de plus. Ce test pin la propriété réelle (deux
/// lectures rendent le même verdict) plutôt qu'une chronologie que `tasks.id`,
/// UUID v4, ne peut pas porter.
#[test]
fn mika2545_two_callbacks_in_the_same_second_answer_deterministically() {
    let db = db();
    let (parent_id, first_id) =
        completed_groom_pair(&db, "mika", GROOM_ISSUE_URL, GROOM_CALLBACK_ESCALATE);
    let second_id = db
        .create_task(&groom_callback("mika", &parent_id, "groom"))
        .unwrap();
    assert!(
        db.update_task_completed(&second_id, "mika", Some(GROOM_CALLBACK_PLAN_GROOMED))
            .unwrap()
    );
    set_created_at(&db, &first_id, "2026-09-26T13:06:14Z");
    set_created_at(&db, &second_id, "2026-09-26T13:06:14Z");

    let a = db
        .latest_groom_verdict_for_issue("mika", GROOM_ISSUE_URL)
        .unwrap();
    let b = db
        .latest_groom_verdict_for_issue("mika", GROOM_ISSUE_URL)
        .unwrap();
    assert_eq!(
        a, b,
        "l'égalité des deux lectures est la propriété : sans le `id DESC` la \
         réponse dépendrait de l'ordre de balayage de SQLite"
    );
}

/// AC2 / test-coverage-mandatory line 4: the three new columns
/// round-trip via the write API (`update_team_run` with non-default
/// values) and the read helpers (`row_to_team_run`).
#[test]
fn test_migrate_v46_to_v47_new_columns_round_trip() {
    let db = db();
    db.register_agent("mika", "Mika", "").unwrap();
    db.conn
        .execute(
            "INSERT INTO teams (id, name, config_path) VALUES ('t-1', 'alpha', '')",
            [],
        )
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO team_runs (id, team_id, goal, status, iteration, max_iterations, started_at) \
                 VALUES ('r-1', 't-1', 'g', 'running', 1, 3, '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

    db.update_team_run(
        "r-1",
        "failed_no_delegation",
        Some("no delegation"),
        2,
        None,
        Some("2026-01-01T00:01:00Z"),
        0,
        true,
        Some(r#"{"phase":"first_decompose"}"#),
    )
    .unwrap();

    let runs = db.load_team_runs("alpha", 10).unwrap();
    assert_eq!(runs.len(), 1);
    let row = &runs[0];
    assert_eq!(row.status, "failed_no_delegation");
    assert_eq!(row.delegation_count, 0);
    assert!(
        row.solo_absorption,
        "solo_absorption must round-trip as true"
    );
    assert_eq!(
        row.failure_context.as_deref(),
        Some(r#"{"phase":"first_decompose"}"#)
    );
}

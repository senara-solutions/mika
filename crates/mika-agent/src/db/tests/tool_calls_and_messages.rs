//! Tests de `crate::db` — thème `tool_calls_and_messages` (mika#2321).
//!
//! Les `tool_calls` (dont la détection de répétition destructive de mika#1646),
//! les messages internes (#494) et la fenêtre de conversation (mika#2295).
//!
//! Enfant de `db::tests`, donc descendant de `db` : les items privés de `db` et
//! les helpers de `db::tests` restent visibles via `use super::*`.

use super::*;

/// The founding incident: two closes of PR #1644 seven minutes apart, from
/// two DIFFERENT sessions (the second came from a deferred webhook replay).
/// The query is agent-scoped precisely so the second one can see the first.
#[test]
fn test_repeat_close_found_across_sessions() {
    let db = db();
    db.create_session("s1", "mika", "cli").unwrap();
    db.create_session("s2", "mika", "cli").unwrap();

    let earlier = timestamp::format(&(Utc::now() - Duration::seconds(415)));
    insert_run_gh(&db, "mika", "s1", &["pr", "close", "1644"], &earlier);

    let found = db
        .find_recent_destructive_actions("mika", "pr", "1644", 1800, None)
        .unwrap();
    assert_eq!(found.len(), 1, "second execution must see the first");
}

#[test]
fn test_repeat_close_outside_window_not_found() {
    let db = db();
    db.create_session("s1", "mika", "cli").unwrap();

    let long_ago = timestamp::format(&(Utc::now() - Duration::seconds(7200)));
    insert_run_gh(&db, "mika", "s1", &["pr", "close", "1644"], &long_ago);

    let found = db
        .find_recent_destructive_actions("mika", "pr", "1644", 1800, None)
        .unwrap();
    assert!(
        found.is_empty(),
        "a close 2h old is outside the 30min window"
    );
}

/// A read of the target is not a close of it.
#[test]
fn test_view_of_target_is_not_a_repeat() {
    let db = db();
    db.create_session("s1", "mika", "cli").unwrap();

    let recent = timestamp::format(&(Utc::now() - Duration::seconds(60)));
    insert_run_gh(
        &db,
        "mika",
        "s1",
        &["pr", "view", "1644", "--json", "files"],
        &recent,
    );

    let found = db
        .find_recent_destructive_actions("mika", "pr", "1644", 1800, None)
        .unwrap();
    assert!(found.is_empty());
}

/// Closing issue #164 must not read as a repeat of closing issue #1644.
#[test]
fn test_substring_number_is_not_a_repeat() {
    let db = db();
    db.create_session("s1", "mika", "cli").unwrap();

    let recent = timestamp::format(&(Utc::now() - Duration::seconds(60)));
    insert_run_gh(&db, "mika", "s1", &["issue", "close", "1644"], &recent);

    let found = db
        .find_recent_destructive_actions("mika", "issue", "164", 1800, None)
        .unwrap();
    assert!(found.is_empty(), "#164 must not match #1644");
}

/// A PR close and an issue close of the same number are different actions.
#[test]
fn test_pr_close_is_not_an_issue_close_repeat() {
    let db = db();
    db.create_session("s1", "mika", "cli").unwrap();

    let recent = timestamp::format(&(Utc::now() - Duration::seconds(60)));
    insert_run_gh(&db, "mika", "s1", &["pr", "close", "1644"], &recent);

    let found = db
        .find_recent_destructive_actions("mika", "issue", "1644", 1800, None)
        .unwrap();
    assert!(found.is_empty());
}

/// Another agent's close is not this agent's repeat.
#[test]
fn test_other_agent_close_is_not_a_repeat() {
    let db = db();
    db.create_session("s1", "mika", "cli").unwrap();

    let recent = timestamp::format(&(Utc::now() - Duration::seconds(60)));
    insert_run_gh(&db, "mika", "s1", &["pr", "close", "1644"], &recent);

    let found = db
        .find_recent_destructive_actions("mika-qa", "pr", "1644", 1800, None)
        .unwrap();
    assert!(found.is_empty());
}

#[test]
fn test_dispatch_failures_below_threshold_no_anomaly() {
    let db = db();
    let session_id = "test-session";
    db.create_session(session_id, "mika", "cli").unwrap();

    // 2 recent failures — below threshold of 3
    let recent = timestamp::format(&(Utc::now() - Duration::seconds(600)));
    insert_tool_call(&db, "mika", session_id, "run_claude_pilot", false, &recent);
    insert_tool_call(&db, "mika", session_id, "run_claude_pilot", false, &recent);

    let summary = db.get_task_health_summary("mika").unwrap();
    assert!(
        summary
            .anomalies
            .iter()
            .all(|a| a.anomaly_type != "dispatch_failures"),
        "should not fire dispatch_failures with only 2 failures"
    );
}

#[test]
fn test_dispatch_failures_at_threshold() {
    let db = db();
    let session_id = "test-session";
    db.create_session(session_id, "mika", "cli").unwrap();

    // 3 recent failures — at threshold
    let recent = timestamp::format(&(Utc::now() - Duration::seconds(600)));
    for _ in 0..3 {
        insert_tool_call(&db, "mika", session_id, "run_claude_pilot", false, &recent);
    }

    let summary = db.get_task_health_summary("mika").unwrap();
    let anomaly = summary
        .anomalies
        .iter()
        .find(|a| a.anomaly_type == "dispatch_failures");
    assert!(
        anomaly.is_some(),
        "should fire dispatch_failures at threshold 3"
    );
    assert_eq!(anomaly.unwrap().age_description, "3 failures in last 2h");
}

#[test]
fn test_dispatch_failures_above_threshold_shows_count() {
    let db = db();
    let session_id = "test-session";
    db.create_session(session_id, "mika", "cli").unwrap();

    // 6 recent failures — above threshold
    let recent = timestamp::format(&(Utc::now() - Duration::seconds(600)));
    for _ in 0..6 {
        insert_tool_call(&db, "mika", session_id, "run_claude_pilot", false, &recent);
    }

    let summary = db.get_task_health_summary("mika").unwrap();
    let anomaly = summary
        .anomalies
        .iter()
        .find(|a| a.anomaly_type == "dispatch_failures")
        .expect("should fire dispatch_failures");
    assert_eq!(anomaly.age_description, "6 failures in last 2h");
}

#[test]
fn test_dispatch_failures_outside_window_no_anomaly() {
    let db = db();
    let session_id = "test-session";
    db.create_session(session_id, "mika", "cli").unwrap();

    // 3 failures older than 2h window
    let old = timestamp::format(&(Utc::now() - Duration::seconds(8000)));
    for _ in 0..3 {
        insert_tool_call(&db, "mika", session_id, "run_claude_pilot", false, &old);
    }

    let summary = db.get_task_health_summary("mika").unwrap();
    assert!(
        summary
            .anomalies
            .iter()
            .all(|a| a.anomaly_type != "dispatch_failures"),
        "failures outside 2h window should not trigger dispatch_failures"
    );
}

#[test]
fn test_dispatch_failures_mixed_success_counts_only_failures() {
    let db = db();
    let session_id = "test-session";
    db.create_session(session_id, "mika", "cli").unwrap();

    let recent = timestamp::format(&(Utc::now() - Duration::seconds(600)));
    // 2 failures + 3 successes — only 2 failures, below threshold
    insert_tool_call(&db, "mika", session_id, "run_claude_pilot", false, &recent);
    insert_tool_call(&db, "mika", session_id, "run_claude_pilot", true, &recent);
    insert_tool_call(&db, "mika", session_id, "run_claude_pilot", false, &recent);
    insert_tool_call(&db, "mika", session_id, "run_claude_pilot", true, &recent);
    insert_tool_call(&db, "mika", session_id, "run_claude_pilot", true, &recent);

    let summary = db.get_task_health_summary("mika").unwrap();
    assert!(
        summary
            .anomalies
            .iter()
            .all(|a| a.anomaly_type != "dispatch_failures"),
        "should count only failures, not successes"
    );
}

#[test]
fn test_dispatch_failures_task_correlation_via_session_join() {
    let db = db();
    let session_id = "task-session";
    db.create_session(session_id, "mika", "cli").unwrap();

    // Create an in_progress manual task and link session to it
    let task_id = db
        .create_task(&new_task("mika", "Fix issue #42", "manual", "none"))
        .unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
            params![task_id],
        )
        .unwrap();
    db.conn
        .execute(
            "UPDATE sessions SET task_id = ?1 WHERE id = ?2",
            params![task_id, session_id],
        )
        .unwrap();

    // 3 failures in that session
    let recent = timestamp::format(&(Utc::now() - Duration::seconds(600)));
    for _ in 0..3 {
        insert_tool_call(&db, "mika", session_id, "run_claude_pilot", false, &recent);
    }

    let summary = db.get_task_health_summary("mika").unwrap();
    let anomaly = summary
        .anomalies
        .iter()
        .find(|a| a.anomaly_type == "dispatch_failures")
        .expect("should fire dispatch_failures with task correlation");
    assert_eq!(anomaly.task_id, task_id);
    assert_eq!(anomaly.label, "Fix issue #42");
}

#[test]
fn test_dispatch_stale_fires_when_no_recent_dispatch() {
    let db = db();

    // Create an in_progress manual task
    let task_id = db
        .create_task(&new_task("mika", "Stale work item", "manual", "none"))
        .unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
            params![task_id],
        )
        .unwrap();

    // No run_claude_pilot calls at all → stale dispatch
    let summary = db.get_task_health_summary("mika").unwrap();
    let anomaly = summary
        .anomalies
        .iter()
        .find(|a| a.anomaly_type == "dispatch_stale");
    assert!(
        anomaly.is_some(),
        "should fire dispatch_stale when no dispatch attempt in >1h"
    );
    assert_eq!(anomaly.unwrap().task_id, task_id);
    assert_eq!(
        anomaly.unwrap().age_description,
        "no dispatch attempt in >1h"
    );
}

#[test]
fn test_dispatch_stale_not_fired_when_recent_dispatch_exists() {
    let db = db();
    let session_id = "test-session";
    db.create_session(session_id, "mika", "cli").unwrap();

    // Create an in_progress manual task
    let task_id = db
        .create_task(&new_task("mika", "Active work item", "manual", "none"))
        .unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
            params![task_id],
        )
        .unwrap();

    // Recent successful dispatch within 1h
    let recent = timestamp::format(&(Utc::now() - Duration::seconds(1800)));
    insert_tool_call(&db, "mika", session_id, "run_claude_pilot", true, &recent);

    let summary = db.get_task_health_summary("mika").unwrap();
    assert!(
        summary
            .anomalies
            .iter()
            .all(|a| a.anomaly_type != "dispatch_stale"),
        "should not fire dispatch_stale when recent dispatch exists"
    );
}

#[test]
fn test_dispatch_signal_a_suppresses_signal_b() {
    let db = db();
    let session_id = "test-session";
    db.create_session(session_id, "mika", "cli").unwrap();

    // Create an in_progress manual task
    let task_id = db
        .create_task(&new_task("mika", "Work item", "manual", "none"))
        .unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
            params![task_id],
        )
        .unwrap();

    // 3 recent failures (Signal A fires) but also no recent dispatch (Signal B would fire)
    // Only Signal A should appear
    let recent = timestamp::format(&(Utc::now() - Duration::seconds(600)));
    for _ in 0..3 {
        insert_tool_call(&db, "mika", session_id, "run_claude_pilot", false, &recent);
    }

    let summary = db.get_task_health_summary("mika").unwrap();
    let dispatch_anomalies: Vec<_> = summary
        .anomalies
        .iter()
        .filter(|a| a.anomaly_type == "dispatch_failures" || a.anomaly_type == "dispatch_stale")
        .collect();
    assert_eq!(
        dispatch_anomalies.len(),
        1,
        "Signal A should suppress Signal B"
    );
    assert_eq!(dispatch_anomalies[0].anomaly_type, "dispatch_failures");
}

#[test]
fn test_dispatch_stale_fires_when_failures_aged_out() {
    let db = db();
    let session_id = "test-session";
    db.create_session(session_id, "mika", "cli").unwrap();

    // Create an in_progress manual task
    let task_id = db
        .create_task(&new_task("mika", "Aged out work", "manual", "none"))
        .unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
            params![task_id],
        )
        .unwrap();

    // 3 failures older than 2h window (aged out of Signal A)
    // AND older than 1h (stale for Signal B)
    let old = timestamp::format(&(Utc::now() - Duration::seconds(8000)));
    for _ in 0..3 {
        insert_tool_call(&db, "mika", session_id, "run_claude_pilot", false, &old);
    }

    let summary = db.get_task_health_summary("mika").unwrap();
    // Signal A should NOT fire (aged out of 2h window)
    assert!(
        summary
            .anomalies
            .iter()
            .all(|a| a.anomaly_type != "dispatch_failures"),
        "Signal A should not fire for aged-out failures"
    );
    // Signal B SHOULD fire (no recent dispatch in >1h, in_progress task exists)
    let stale = summary
        .anomalies
        .iter()
        .find(|a| a.anomaly_type == "dispatch_stale");
    assert!(
        stale.is_some(),
        "Signal B (dispatch_stale) should fire when failures aged out — this is the aging defense"
    );
    assert_eq!(stale.unwrap().task_id, task_id);
}

// ===== Internal message tests (#494) =====

#[test]
fn test_internal_column_exists() {
    let db = db();
    assert!(db.column_exists("messages", "internal").unwrap());
}

#[test]
fn test_save_internal_message() {
    let (db, sid) = db_with_session();
    db.save_message_with_metadata("mika", &sid, "assistant", "internal msg", None, None, true)
        .unwrap();
    let msgs = db.load_recent_messages("mika", 10).unwrap();
    assert_eq!(msgs.len(), 1);
    assert!(msgs[0].internal);
    assert_eq!(msgs[0].content, "internal msg");
}

#[test]
fn test_save_internal_message_with_metadata() {
    let (db, sid) = db_with_session();
    db.save_message_with_metadata(
        "mika",
        &sid,
        "assistant",
        "internal with meta",
        Some(r#"{"tool_calls":[]}"#),
        None,
        true,
    )
    .unwrap();
    let msgs = db.load_recent_messages("mika", 10).unwrap();
    assert_eq!(msgs.len(), 1);
    assert!(msgs[0].internal);
    assert!(msgs[0].metadata.is_some());
}

#[test]
fn test_save_message_defaults_not_internal() {
    let (db, sid) = db_with_session();
    db.save_message("mika", &sid, "user", "hello", None)
        .unwrap();
    let msgs = db.load_recent_messages("mika", 10).unwrap();
    assert_eq!(msgs.len(), 1);
    assert!(!msgs[0].internal);
}

#[test]
fn test_load_recent_messages_filtered_excludes_internal() {
    let (db, sid) = db_with_session();
    db.save_message("mika", &sid, "user", "visible 1", None)
        .unwrap();
    db.save_message_with_metadata("mika", &sid, "assistant", "hidden", None, None, true)
        .unwrap();
    db.save_message("mika", &sid, "assistant", "visible 2", None)
        .unwrap();

    // Without filter: all 3, hidden count 0
    let (all, hidden) = db
        .load_recent_messages_filtered("mika", None, 10, false)
        .unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(hidden, 0);

    // With filter: only 2 visible, 1 hidden
    let (visible, hidden) = db
        .load_recent_messages_filtered("mika", None, 10, true)
        .unwrap();
    assert_eq!(visible.len(), 2);
    assert_eq!(hidden, 1);
    assert!(visible.iter().all(|m| !m.internal));
}

// ===== mika#2295: conversation-window scope =====

/// AC2 — the default is unchanged, asserted on the query itself.
///
/// The behavioural tests around it would all still pass if the unscoped path
/// had quietly grown an `AND (?3 IS NULL OR …)`; this is the assertion that
/// says "byte for byte" and means it. It also pins what the scoped form adds:
/// one equality on `m.session_id`, and nothing else moved.
#[test]
fn mika2295_unscoped_query_is_byte_for_byte_the_pre_fix_query() {
    let cols = Database::SESSION_MESSAGE_COLUMNS;
    let unscoped = Database::recent_messages_sql(cols, false);

    let pre_fix = format!(
        "SELECT {cols} FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.agent_id = ?1 AND m.role != 'summary' AND s.channel_type != 'team'
              ORDER BY m.created_at DESC, m.id DESC LIMIT ?2"
    );
    assert_eq!(
        unscoped, pre_fix,
        "the unscoped window query must be the pre-mika#2295 query verbatim"
    );

    let scoped = Database::recent_messages_sql(cols, true);
    assert!(
        scoped.contains("AND m.session_id = ?3"),
        "the scoped form must restrict in SQL, not after loading: {scoped}"
    );
    assert_ne!(unscoped, scoped);
}

/// AC2 (behaviour) — `None` still returns every session's messages.
#[test]
fn mika2295_unscoped_window_still_crosses_sessions() {
    let (db, sid_a) = db_with_session();
    let sid_b = "other-ticket-session".to_string();
    db.create_session(&sid_b, "mika", "cli").unwrap();

    db.save_message("mika", &sid_a, "user", "ticket A", None)
        .unwrap();
    db.save_message("mika", &sid_b, "user", "ticket B", None)
        .unwrap();

    let (msgs, _) = db
        .load_recent_messages_filtered("mika", None, 20, false)
        .unwrap();
    assert_eq!(msgs.len(), 2, "the default window is agent-wide");
}

/// AC3 — the session filter runs in the query, proven by interleaving.
///
/// This is the test the plan asks for, and its shape is the whole argument:
/// the other session's messages are written **past the limit**, so a filter
/// applied after loading would have spent all 5 slots on session B and
/// returned an amputated window — zero or one of A's four messages. Only a
/// filter that runs in SQL returns all four. That is the exact difference
/// between "correct at rest" and "correct under the 59-grooms-a-day load that
/// produced the incident".
#[test]
fn mika2295_session_scope_survives_interleaving_past_the_limit() {
    let (db, sid_a) = db_with_session();
    let sid_b = "other-ticket-session".to_string();
    db.create_session(&sid_b, "mika", "cli").unwrap();

    for i in 0..4 {
        db.save_message("mika", &sid_a, "user", &format!("A{i}"), None)
            .unwrap();
    }
    // Ten newer messages from another ticket — more than the limit below.
    for i in 0..10 {
        db.save_message("mika", &sid_b, "user", &format!("B{i}"), None)
            .unwrap();
    }

    let (scoped, _) = db
        .load_recent_messages_filtered("mika", Some(&sid_a), 5, false)
        .unwrap();
    assert_eq!(
        scoped.len(),
        4,
        "every message of the current session must survive, however many \
             newer messages other sessions interleaved: {scoped:?}"
    );
    assert!(scoped.iter().all(|m| m.session_id == sid_a));

    // The control: the same call unscoped sees only the other ticket.
    let (unscoped, _) = db
        .load_recent_messages_filtered("mika", None, 5, false)
        .unwrap();
    assert!(
        unscoped.iter().all(|m| m.session_id == sid_b),
        "the agent-wide window is exactly what buries the current session"
    );
}

/// AC3 — `rebuild_context` propagates the scope on the channel-mode path.
#[test]
fn mika2295_rebuild_context_propagates_session_scope() {
    let (db, sid_a) = db_with_session();
    let sid_b = "other-ticket-session".to_string();
    db.create_session(&sid_b, "mika", "cli").unwrap();

    db.save_message("mika", &sid_a, "user", "mine", None)
        .unwrap();
    db.save_message("mika", &sid_b, "user", "theirs", None)
        .unwrap();

    let scoped = db.rebuild_context("mika", Some(&sid_a), None, 20).unwrap();
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].content, "mine");

    let unscoped = db.rebuild_context("mika", None, None, 20).unwrap();
    assert_eq!(unscoped.len(), 2);
}

#[test]
fn test_load_recent_messages_filtered_hidden_count_empty() {
    let (db, _sid) = db_with_session();
    let (msgs, hidden) = db
        .load_recent_messages_filtered("mika", None, 20, true)
        .unwrap();
    assert!(msgs.is_empty());
    assert_eq!(hidden, 0);
}

#[test]
fn test_load_recent_messages_filtered_hidden_count_limit() {
    let (db, sid) = db_with_session();
    // Create 5 internal + 5 visible messages
    for i in 0..5 {
        db.save_message_with_metadata(
            "mika",
            &sid,
            "user",
            &format!("internal {i}"),
            None,
            None,
            true,
        )
        .unwrap();
        db.save_message("mika", &sid, "assistant", &format!("visible {i}"), None)
            .unwrap();
    }

    // Limit 10 fetches all 10 rows from DB; 5 visible returned, 5 hidden counted
    let (visible, hidden) = db
        .load_recent_messages_filtered("mika", None, 10, true)
        .unwrap();
    assert_eq!(visible.len(), 5);
    assert_eq!(hidden, 5);

    // Without filter: all 10 returned, 0 hidden
    let (all, hidden) = db
        .load_recent_messages_filtered("mika", None, 10, false)
        .unwrap();
    assert_eq!(all.len(), 10);
    assert_eq!(hidden, 0);
}

// -- find_active_task_by_pr_url tests --

#[test]
fn test_find_active_task_by_pr_url_found() {
    let db = db();
    let pr_url = "https://github.com/senara-solutions/mika/pull/42";
    let task = new_task("mika", "Implement feature", "manual", "none");
    let id = db.create_task(&task).unwrap();
    let meta = r#"{"claude_pilot":{"pr_url":"https://github.com/senara-solutions/mika/pull/42"}}"#;
    db.update_task_metadata(&id, meta).unwrap();

    let found = db.find_active_task_by_pr_url("mika", pr_url).unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, id);
}

#[test]
fn test_find_active_task_by_pr_url_not_found() {
    let db = db();
    let task = new_task("mika", "Implement feature", "manual", "none");
    let id = db.create_task(&task).unwrap();
    let meta = r#"{"claude_pilot":{"pr_url":"https://github.com/senara-solutions/mika/pull/42"}}"#;
    db.update_task_metadata(&id, meta).unwrap();

    let found = db
        .find_active_task_by_pr_url("mika", "https://github.com/senara-solutions/mika/pull/99")
        .unwrap();
    assert!(found.is_none());
}

#[test]
fn test_find_active_task_by_pr_url_completed_not_returned() {
    let db = db();
    let task = new_task("mika", "Done feature", "manual", "none");
    let id = db.create_task(&task).unwrap();
    let meta = r#"{"claude_pilot":{"pr_url":"https://github.com/senara-solutions/mika/pull/42"}}"#;
    db.update_task_metadata(&id, meta).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'completed' WHERE id = ?1",
            params![id],
        )
        .unwrap();

    let found = db
        .find_active_task_by_pr_url("mika", "https://github.com/senara-solutions/mika/pull/42")
        .unwrap();
    assert!(found.is_none());
}

#[test]
fn test_find_active_task_by_pr_url_wrong_metadata_path() {
    let db = db();
    let task = new_task("mika", "Wrong path", "manual", "none");
    let id = db.create_task(&task).unwrap();
    // pr_url in a different metadata path (not under claude_pilot)
    let meta = r#"{"other":{"pr_url":"https://github.com/senara-solutions/mika/pull/42"}}"#;
    db.update_task_metadata(&id, meta).unwrap();

    let found = db
        .find_active_task_by_pr_url("mika", "https://github.com/senara-solutions/mika/pull/42")
        .unwrap();
    assert!(found.is_none());
}

// -- find_active_task_by_branch tests --

#[test]
fn test_find_active_task_by_branch_found() {
    let db = db();
    let branch = "feat/test";
    let task = new_task("mika", "Implement feature", "manual", "none");
    let id = db.create_task(&task).unwrap();
    let meta = r#"{"claude_pilot":{"branch":"feat/test"}}"#;
    db.update_task_metadata(&id, meta).unwrap();

    let found = db.find_active_task_by_branch("mika", branch).unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, id);
}

#[test]
fn test_find_active_task_by_branch_not_found() {
    let db = db();
    let task = new_task("mika", "Implement feature", "manual", "none");
    let id = db.create_task(&task).unwrap();
    let meta = r#"{"claude_pilot":{"branch":"feat/test"}}"#;
    db.update_task_metadata(&id, meta).unwrap();

    let found = db
        .find_active_task_by_branch("mika", "feat/other-branch")
        .unwrap();
    assert!(found.is_none());
}

#[test]
fn test_find_active_task_by_branch_completed_not_returned() {
    let db = db();
    let task = new_task("mika", "Done feature", "manual", "none");
    let id = db.create_task(&task).unwrap();
    let meta = r#"{"claude_pilot":{"branch":"feat/test"}}"#;
    db.update_task_metadata(&id, meta).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'completed' WHERE id = ?1",
            params![id],
        )
        .unwrap();

    let found = db.find_active_task_by_branch("mika", "feat/test").unwrap();
    assert!(found.is_none());
}

#[test]
fn test_find_active_task_by_branch_wrong_path() {
    let db = db();
    let task = new_task("mika", "Wrong path", "manual", "none");
    let id = db.create_task(&task).unwrap();
    // branch in a different metadata path (not under claude_pilot)
    let meta = r#"{"other":{"branch":"feat/test"}}"#;
    db.update_task_metadata(&id, meta).unwrap();

    let found = db.find_active_task_by_branch("mika", "feat/test").unwrap();
    assert!(found.is_none());
}

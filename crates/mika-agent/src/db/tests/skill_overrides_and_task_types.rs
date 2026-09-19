//! Tests de `crate::db` — thème `skill_overrides_and_task_types` (mika#2321).
//!
//! Les overrides de skills (état `enabled` tri-état, overrides LLM, curateur) et
//! la colonne `tasks.type` (issue / milestone / project, schéma v23).
//!
//! Enfant de `db::tests`, donc descendant de `db` : les items privés de `db` et
//! les helpers de `db::tests` restent visibles via `use super::*`.

use super::*;

// ===== Skill Override Tests =====

#[test]
fn test_skill_override_crud() {
    let db = db();

    // Initially empty
    let overrides = db.get_skill_overrides("mika").unwrap();
    assert!(overrides.is_empty());

    // Set an override
    db.set_skill_override("mika", "web-search", true).unwrap();
    let overrides = db.get_skill_overrides("mika").unwrap();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].skill_name, "web-search");
    assert_eq!(overrides[0].always_on, Some(true));

    // Update the override
    db.set_skill_override("mika", "web-search", false).unwrap();
    let overrides = db.get_skill_overrides("mika").unwrap();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].always_on, Some(false));

    // Delete the override
    db.delete_skill_override("mika", "web-search").unwrap();
    let overrides = db.get_skill_overrides("mika").unwrap();
    assert!(overrides.is_empty());
}

#[test]
fn test_skill_llm_override_round_trip() {
    let mut db = db();

    // Set LLM override on a skill with no existing row.
    db.set_skill_llm_override("mika", "qa-review", "anthropic", "claude-sonnet-4-6")
        .unwrap();
    let overrides = db.get_skill_overrides("mika").unwrap();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].skill_name, "qa-review");
    assert_eq!(overrides[0].always_on, None);
    assert_eq!(overrides[0].llm_provider.as_deref(), Some("anthropic"));
    assert_eq!(overrides[0].llm_model.as_deref(), Some("claude-sonnet-4-6"));

    // Upsert to a different model.
    db.set_skill_llm_override("mika", "qa-review", "deepseek", "deepseek-chat")
        .unwrap();
    let overrides = db.get_skill_overrides("mika").unwrap();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].llm_provider.as_deref(), Some("deepseek"));
    assert_eq!(overrides[0].llm_model.as_deref(), Some("deepseek-chat"));

    // Clearing LLM columns prunes a row with no other overrides.
    db.delete_skill_llm_override("mika", "qa-review").unwrap();
    let overrides = db.get_skill_overrides("mika").unwrap();
    assert!(overrides.is_empty(), "fully-NULL row should be pruned");
}

#[test]
fn test_skill_llm_override_preserves_always_on() {
    let mut db = db();

    // Start with always_on set.
    db.set_skill_override("mika", "qa-review", true).unwrap();
    // Layer an LLM override on top.
    db.set_skill_llm_override("mika", "qa-review", "anthropic", "claude-sonnet-4-6")
        .unwrap();

    let overrides = db.get_skill_overrides("mika").unwrap();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].always_on, Some(true));
    assert_eq!(overrides[0].llm_provider.as_deref(), Some("anthropic"));

    // Clearing LLM columns must NOT delete the row — always_on still set.
    db.delete_skill_llm_override("mika", "qa-review").unwrap();
    let overrides = db.get_skill_overrides("mika").unwrap();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].always_on, Some(true));
    assert_eq!(overrides[0].llm_provider, None);
    assert_eq!(overrides[0].llm_model, None);
}

#[test]
fn test_skill_llm_override_case_insensitive_and_per_agent() {
    let db = db();
    db.register_agent("agent-a", "Agent A", "").unwrap();
    db.register_agent("agent-b", "Agent B", "").unwrap();

    db.set_skill_llm_override("agent-a", "QA-Review", "anthropic", "claude-sonnet-4-6")
        .unwrap();
    db.set_skill_llm_override("agent-b", "qa-review", "deepseek", "deepseek-chat")
        .unwrap();

    let a = db.get_skill_overrides("AGENT-A").unwrap();
    let b = db.get_skill_overrides("agent-b").unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].llm_provider.as_deref(), Some("anthropic"));
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].llm_provider.as_deref(), Some("deepseek"));
}

#[test]
fn test_skill_override_per_agent_isolation() {
    let db = db();
    db.register_agent("agent-a", "Agent A", "").unwrap();
    db.register_agent("agent-b", "Agent B", "").unwrap();

    db.set_skill_override("agent-a", "shell-exec", true)
        .unwrap();
    db.set_skill_override("agent-b", "shell-exec", false)
        .unwrap();

    let a_overrides = db.get_skill_overrides("agent-a").unwrap();
    let b_overrides = db.get_skill_overrides("agent-b").unwrap();
    assert_eq!(a_overrides[0].always_on, Some(true));
    assert_eq!(b_overrides[0].always_on, Some(false));
}

#[test]
fn test_skill_override_case_insensitive() {
    let db = db();

    db.set_skill_override("mika", "Web-Search", true).unwrap();

    // Query with different case should find it
    let overrides = db.get_skill_overrides("MIKA").unwrap();
    assert_eq!(overrides.len(), 1);

    // Upsert with different case should update (not create duplicate)
    db.set_skill_override("MIKA", "web-search", false).unwrap();
    let overrides = db.get_skill_overrides("mika").unwrap();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].always_on, Some(false));
}

#[test]
fn test_skill_override_delete_nonexistent_is_noop() {
    let db = db();
    // Should not error
    db.delete_skill_override("mika", "nonexistent").unwrap();
}

#[test]
fn test_set_skill_enabled_disable() {
    let mut db = db();
    db.set_skill_enabled("mika", "foo", false).unwrap();
    let overrides = db.get_skill_overrides("mika").unwrap();
    let ov = overrides.iter().find(|o| o.skill_name == "foo").unwrap();
    assert_eq!(ov.enabled, Some(false));
}

#[test]
fn test_set_skill_enabled_enable_deletes_row() {
    let mut db = db();
    // Disable first
    db.set_skill_enabled("mika", "foo", false).unwrap();
    assert_eq!(db.get_skill_overrides("mika").unwrap().len(), 1);
    // Enable (default) — row should be deleted (default-equals-delete)
    db.set_skill_enabled("mika", "foo", true).unwrap();
    assert!(db.get_skill_overrides("mika").unwrap().is_empty());
}

#[test]
fn test_set_skill_enabled_preserves_always_on() {
    let mut db = db();
    // Set always_on first
    db.set_skill_override("mika", "foo", true).unwrap();
    // Now disable
    db.set_skill_enabled("mika", "foo", false).unwrap();
    let overrides = db.get_skill_overrides("mika").unwrap();
    let ov = overrides.iter().find(|o| o.skill_name == "foo").unwrap();
    assert_eq!(ov.always_on, Some(true));
    assert_eq!(ov.enabled, Some(false));
}

#[test]
fn test_v43_migration_adds_curator_columns() {
    // Mechanically enforce the schema the curator candidate query depends
    // on: migrate_v42_to_v43 must add lifecycle_state, use_count, and
    // last_used_at to skill_overrides. This dependency is self-contained in
    // mika#1584 — the migration adds all three columns here, so the query is
    // not coupled to any other PR's schema. (qa#1624: schema dependency
    // mechanically enforced.)
    let db = db();
    let cols: Vec<String> = db
        .conn
        .prepare("PRAGMA table_info('skill_overrides')")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    for required in ["lifecycle_state", "use_count", "last_used_at"] {
        assert!(
            cols.iter().any(|c| c == required),
            "skill_overrides missing curator column '{required}' (v43 migration); have: {cols:?}"
        );
    }
}

#[test]
fn test_archival_candidates_fresh_agent_returns_zero() {
    // AC9: a fresh agent with no agent-authored skills yields zero
    // candidates from the curator candidate query.
    let db = db();
    let candidates = db.get_archival_candidates("mika", 30).unwrap();
    assert!(candidates.is_empty());
}

#[test]
fn test_archival_candidates_staged_skill_excluded() {
    // AC10: a staged (not yet promoted) skill is excluded even when idle —
    // the query only considers lifecycle_state = 'active'.
    let db = db();
    let idle = crate::timestamp::now_minus(chrono::Duration::days(60));
    seed_curator_skill_row(&db, "mika", "staged-skill", Some("staged"), 0, Some(&idle));
    let candidates = db.get_archival_candidates("mika", 30).unwrap();
    assert!(candidates.is_empty());
}

#[test]
fn test_archival_candidates_idle_active_skill_returned() {
    // AC11: a promoted+active skill idle beyond the threshold (last used 40
    // days ago, threshold 30) is returned as exactly one candidate.
    let db = db();
    let idle = crate::timestamp::now_minus(chrono::Duration::days(40));
    seed_curator_skill_row(&db, "mika", "idle-skill", Some("active"), 3, Some(&idle));
    let candidates = db.get_archival_candidates("mika", 30).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].skill_name, "idle-skill");
    assert_eq!(candidates[0].lifecycle_state.as_deref(), Some("active"));
}

#[test]
fn test_archival_candidates_bundled_null_lifecycle_excluded() {
    // AC12: a bundled/marketplace skill (NULL lifecycle_state, never used)
    // is excluded by construction — NULL never equals 'active'.
    let db = db();
    seed_curator_skill_row(&db, "mika", "bundled-skill", None, 0, None);
    let candidates = db.get_archival_candidates("mika", 30).unwrap();
    assert!(candidates.is_empty());
}

#[test]
fn test_set_skill_enabled_enable_with_always_on_keeps_row() {
    let mut db = db();
    // Set both always_on and disabled
    db.set_skill_override("mika", "foo", true).unwrap();
    db.set_skill_enabled("mika", "foo", false).unwrap();
    // Now enable — row should remain because always_on is non-NULL
    db.set_skill_enabled("mika", "foo", true).unwrap();
    let overrides = db.get_skill_overrides("mika").unwrap();
    let ov = overrides.iter().find(|o| o.skill_name == "foo").unwrap();
    assert_eq!(ov.always_on, Some(true));
    assert_eq!(ov.enabled, None); // Cleared to NULL
}

#[test]
fn test_set_skill_enabled_preserves_llm_override() {
    let mut db = db();
    db.set_skill_llm_override("mika", "foo", "anthropic", "claude-sonnet-4-6")
        .unwrap();
    db.set_skill_enabled("mika", "foo", false).unwrap();
    let overrides = db.get_skill_overrides("mika").unwrap();
    let ov = overrides.iter().find(|o| o.skill_name == "foo").unwrap();
    assert_eq!(ov.llm_provider.as_deref(), Some("anthropic"));
    assert_eq!(ov.enabled, Some(false));
}

#[test]
fn test_set_skill_enabled_round_trip() {
    let mut db = db();
    // Disable → enable → state is clean
    db.set_skill_enabled("mika", "bar", false).unwrap();
    assert_eq!(db.get_skill_overrides("mika").unwrap().len(), 1);
    db.set_skill_enabled("mika", "bar", true).unwrap();
    assert!(db.get_skill_overrides("mika").unwrap().is_empty());
}

#[test]
fn test_enabled_column_exists_in_skill_overrides() {
    let db = db();
    assert!(db.column_exists("skill_overrides", "enabled").unwrap());
}

#[test]
fn test_delete_skill_llm_override_with_enabled_keeps_row() {
    let mut db = db();
    db.set_skill_llm_override("mika", "foo", "anthropic", "claude-sonnet-4-6")
        .unwrap();
    db.set_skill_enabled("mika", "foo", false).unwrap();
    // Delete LLM override — row should remain because enabled is non-NULL
    db.delete_skill_llm_override("mika", "foo").unwrap();
    let overrides = db.get_skill_overrides("mika").unwrap();
    let ov = overrides.iter().find(|o| o.skill_name == "foo").unwrap();
    assert_eq!(ov.llm_provider, None);
    assert_eq!(ov.enabled, Some(false));
}

#[test]
fn test_schema_version_is_current() {
    let db = db();
    assert_eq!(db.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
}

#[test]
fn test_execution_trace_id_column_exists() {
    let db = db();
    assert!(db.column_exists("tasks", "execution_trace_id").unwrap());
}

#[test]
fn test_parent_session_id_column_exists() {
    let db = db();
    assert!(db.column_exists("sessions", "parent_session_id").unwrap());
}

// ===== `tasks.type` column tests (issue #595, schema v23) =====

#[test]
fn test_tasks_type_column_exists() {
    let db = db();
    assert!(db.column_exists("tasks", "type").unwrap());
}

#[test]
fn test_tasks_type_defaults_to_issue() {
    // Inserting via NewTask with r#type: None should backfill to 'issue'
    // via the SQL DEFAULT.
    let db = db();
    let task = NewTask {
        agent_id: "mika".to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: "default-type".to_string(),
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
    let t = db.get_task(&id, "mika").unwrap().unwrap();
    assert_eq!(t.r#type, "issue");
}

#[test]
fn test_tasks_type_round_trips_milestone() {
    let db = db();
    let task = NewTask {
        agent_id: "mika".to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: "milestone-type".to_string(),
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
        r#type: Some("milestone".to_string()),
        dispatch_class: None,
    };
    let id = db.create_task(&task).unwrap();
    let t = db.get_task(&id, "mika").unwrap().unwrap();
    assert_eq!(t.r#type, "milestone");
}

#[test]
fn test_tasks_type_check_constraint_rejects_invalid() {
    // Direct INSERT bypassing the tool boundary should still be blocked by
    // the SQLite CHECK constraint.
    let db = db();
    let result = db.conn.execute(
        "INSERT INTO tasks (id, agent_id, label, trigger_type, action_type, type)
             VALUES ('00000000-0000-0000-0000-000000000099', 'mika', 'bad', 'manual', 'none', 'epic')",
        [],
    );
    assert!(result.is_err(), "CHECK should reject 'epic'");
}

#[test]
fn test_update_task_execution_trace_id() {
    let db = db();
    let task = NewTask {
        agent_id: "mika".to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: "test-exec-trace".to_string(),
        trigger_type: "time".to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: Some("2286-11-20T17:46:39Z".to_string()),
        timeout_at: None,
        action_type: "send_message".to_string(),
        action_config: "{}".to_string(),
        input_context: None,
        created_by_session: None,
        created_trace_id: Some("created-trace-aaa".to_string()),
        reference_url: None,
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: None,
    };
    let id = db.create_task(&task).unwrap();

    // Initially execution_trace_id is None
    let t = db.get_task(&id, "mika").unwrap().unwrap();
    assert!(t.execution_trace_id.is_none());

    // Write execution trace_id
    db.update_task_execution_trace_id(&id, "exec-trace-bbb")
        .unwrap();

    let t = db.get_task(&id, "mika").unwrap().unwrap();
    assert_eq!(t.execution_trace_id.as_deref(), Some("exec-trace-bbb"));
    // created_trace_id should be unchanged
    assert_eq!(t.created_trace_id.as_deref(), Some("created-trace-aaa"));
}

#[test]
fn test_update_task_execution_trace_id_cross_agent() {
    // Verify execution_trace_id update does NOT scope by agent_id
    let db = db();
    db.register_agent("other", "Other", "").unwrap();

    let task = NewTask {
        agent_id: "other".to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: "cross-agent-test".to_string(),
        trigger_type: "time".to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: Some("2286-11-20T17:46:39Z".to_string()),
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

    // The "mika" agent can write execution_trace_id on a task owned by "other"
    db.update_task_execution_trace_id(&id, "cross-trace-123")
        .unwrap();

    let t = db.get_task(&id, "other").unwrap().unwrap();
    assert_eq!(t.execution_trace_id.as_deref(), Some("cross-trace-123"));
}

#[test]
fn test_create_session_with_parent() {
    let db = db();
    // Create a parent session first
    db.create_session("parent-sess", "mika", "cli").unwrap();

    // Create a child session with parent reference and task linkage
    db.create_session_with_parent(
        "child-sess",
        "mika",
        "system",
        Some(r#"{"trigger": "callback"}"#),
        Some("parent-sess"),
        Some("task-123"),
    )
    .unwrap();

    let session = db.get_session("child-sess").unwrap().unwrap();
    assert_eq!(session.parent_session_id.as_deref(), Some("parent-sess"));
    assert_eq!(session.task_id.as_deref(), Some("task-123"));
    assert_eq!(
        session.metadata.as_deref(),
        Some(r#"{"trigger": "callback"}"#)
    );
}

#[test]
fn test_create_session_with_parent_none() {
    let db = db();
    db.create_session_with_parent(
        "no-parent-sess",
        "mika",
        "system",
        Some(r#"{"trigger": "heartbeat"}"#),
        None,
        None,
    )
    .unwrap();

    let session = db.get_session("no-parent-sess").unwrap().unwrap();
    assert!(session.parent_session_id.is_none());
    assert!(session.task_id.is_none());
}

#[test]
fn test_create_session_with_metadata_and_task_id() {
    let db = db();
    db.create_session_with_metadata(
        "meta-sess",
        "mika",
        "cli",
        Some(r#"{"task_id": "task-abc"}"#),
        Some("task-abc"),
    )
    .unwrap();

    let session = db.get_session("meta-sess").unwrap().unwrap();
    assert_eq!(session.task_id.as_deref(), Some("task-abc"));
    assert_eq!(
        session.metadata.as_deref(),
        Some(r#"{"task_id": "task-abc"}"#)
    );
}

#[test]
fn test_get_sessions_for_task_tree() {
    let db = db();

    // Create a parent task
    let parent_id = db
        .create_task(&new_task("mika", "parent-work-item", "manual", "none"))
        .unwrap();

    // Create a child task
    let mut child = new_task("mika", "child-callback", "callback", "resume_agent");
    child.parent_task_id = Some(parent_id.clone());
    child.depth = 1;
    let child_id = db.create_task(&child).unwrap();

    // Create sessions linked to the task tree
    db.create_session_with_parent("sess-parent", "mika", "cli", None, None, Some(&parent_id))
        .unwrap();
    db.create_session_with_parent(
        "sess-child",
        "mika",
        "system",
        Some(r#"{"trigger": "callback"}"#),
        Some("sess-parent"),
        Some(&child_id),
    )
    .unwrap();
    // Unrelated session (no task_id)
    db.create_session("sess-unrelated", "mika", "cli").unwrap();

    let sessions = db.get_sessions_for_task_tree(&parent_id).unwrap();
    assert_eq!(sessions.len(), 2);

    // Both sessions should be found
    let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
    assert!(ids.contains(&"sess-parent"));
    assert!(ids.contains(&"sess-child"));

    // Verify task labels are joined
    let parent_sess = sessions.iter().find(|s| s.id == "sess-parent").unwrap();
    assert_eq!(parent_sess.task_label.as_deref(), Some("parent-work-item"));
    let child_sess = sessions.iter().find(|s| s.id == "sess-child").unwrap();
    assert_eq!(child_sess.task_label.as_deref(), Some("child-callback"));
}

#[test]
fn test_sessions_for_task_tree_backfill_compat() {
    let db = db();

    // Simulate a pre-v19 session with task_id only in metadata JSON
    let task_id = db
        .create_task(&new_task("mika", "legacy-task", "manual", "none"))
        .unwrap();

    // Create session with task_id only in metadata (legacy path)
    db.create_session_with_metadata(
        "legacy-sess",
        "mika",
        "cli",
        Some(&format!(r#"{{"task_id": "{task_id}"}}"#)),
        None, // no task_id column
    )
    .unwrap();

    // The COALESCE in the query should still find it
    let sessions = db.get_sessions_for_task_tree(&task_id).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "legacy-sess");
}

#[test]
fn test_sessions_for_task_tree_includes_deep_descendants() {
    let db = db();

    // Create a 3-level tree: root → child → grandchild
    let root_id = db
        .create_task(&new_task("mika", "root-task", "manual", "none"))
        .unwrap();
    let mut child = new_task("mika", "child-task", "callback", "resume_agent");
    child.parent_task_id = Some(root_id.clone());
    child.depth = 1;
    let child_id = db.create_task(&child).unwrap();
    let mut grandchild = new_task("mika", "grandchild-task", "callback", "resume_agent");
    grandchild.parent_task_id = Some(child_id.clone());
    grandchild.depth = 2;
    let grandchild_id = db.create_task(&grandchild).unwrap();

    // Sessions at each level
    db.create_session_with_parent("sess-root", "mika", "cli", None, None, Some(&root_id))
        .unwrap();
    db.create_session_with_parent("sess-child", "mika", "system", None, None, Some(&child_id))
        .unwrap();
    db.create_session_with_parent(
        "sess-grandchild",
        "mika",
        "system",
        None,
        None,
        Some(&grandchild_id),
    )
    .unwrap();
    // Unrelated session
    db.create_session("sess-unrelated", "mika", "cli").unwrap();

    let sessions = db.get_sessions_for_task_tree(&root_id).unwrap();
    assert_eq!(sessions.len(), 3);
    let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
    assert!(ids.contains(&"sess-root"));
    assert!(ids.contains(&"sess-child"));
    assert!(ids.contains(&"sess-grandchild"));
}

#[test]
fn test_list_sessions_paginated_coalesce_task_id() {
    let db = db();

    let task_id = db
        .create_task(&new_task("mika", "coalesce-test", "manual", "none"))
        .unwrap();

    // Session with task_id in column
    db.create_session_with_metadata("col-sess", "mika", "cli", None, Some(&task_id))
        .unwrap();
    // Session with task_id only in metadata (legacy)
    db.create_session_with_metadata(
        "meta-sess-2",
        "mika",
        "cli",
        Some(&format!(r#"{{"task_id": "{task_id}"}}"#)),
        None,
    )
    .unwrap();
    // Unrelated session
    db.create_session("other-sess", "mika", "cli").unwrap();

    let sessions = db
        .list_sessions_paginated(None, None, None, Some(&task_id), None, None, 50, 0)
        .unwrap();
    assert_eq!(sessions.len(), 2);
    let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
    assert!(ids.contains(&"col-sess"));
    assert!(ids.contains(&"meta-sess-2"));
}

#[test]
fn test_sessions_time_range_filter() {
    let db = db();
    // Create sessions with known timestamps
    db.create_session("s1", "mika", "cli").unwrap();
    db.create_session("s2", "mika", "cli").unwrap();

    // Query with from/to set to a window that includes all sessions (now-1h to now+1h)
    let from_ts = crate::timestamp::now_minus(chrono::Duration::seconds(3600));
    let to_ts = crate::timestamp::now_plus(chrono::Duration::seconds(3600));
    let sessions = db
        .list_sessions_paginated(None, None, None, None, Some(&from_ts), Some(&to_ts), 50, 0)
        .unwrap();
    assert_eq!(sessions.len(), 2);

    // Query with from set to the far future — should return no sessions
    let future_ts = "2099-01-01T00:00:00Z";
    let sessions = db
        .list_sessions_paginated(None, None, None, None, Some(future_ts), None, 50, 0)
        .unwrap();
    assert!(sessions.is_empty());

    // Count should also respect the filter
    let count = db
        .count_sessions(None, None, None, None, Some(&from_ts), Some(&to_ts))
        .unwrap();
    assert_eq!(count, 2);
    let count = db
        .count_sessions(None, None, None, None, Some(future_ts), None)
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_tasks_time_range_filter() {
    let db = db();
    db.create_task(&new_task("mika", "task-a", "manual", "none"))
        .unwrap();
    db.create_task(&new_task("mika", "task-b", "manual", "none"))
        .unwrap();

    // Build filters with a wide time range
    let from_ts = crate::timestamp::now_minus(chrono::Duration::seconds(3600));
    let to_ts = crate::timestamp::now_plus(chrono::Duration::seconds(3600));
    let filters = TaskFilters {
        from: Some(from_ts),
        to: Some(to_ts),
        ..Default::default()
    };
    let (tasks, count) = db.list_tasks_paginated_with_count(&filters, 50, 0).unwrap();
    assert_eq!(tasks.len(), 2);
    assert_eq!(count, 2);

    // Narrow to the far future — no matches
    let filters = TaskFilters {
        from: Some("2099-01-01T00:00:00Z".to_string()),
        ..Default::default()
    };
    let (tasks, count) = db.list_tasks_paginated_with_count(&filters, 50, 0).unwrap();
    assert!(tasks.is_empty());
    assert_eq!(count, 0);
}

#[test]
fn test_dev_runs_time_range_filter() {
    let db = db();
    // Create dev-run-shaped tasks (trigger_type=manual, source=self_dev)
    let mut t = new_task("mika", "dev-run-1", "manual", "none");
    t.source = Some("self_dev".to_string());
    db.create_task(&t).unwrap();
    let mut t2 = new_task("mika", "dev-run-2", "manual", "none");
    t2.source = Some("self_dev".to_string());
    db.create_task(&t2).unwrap();

    // Wide time range — should get both
    let from_ts = crate::timestamp::now_minus(chrono::Duration::seconds(3600));
    let to_ts = crate::timestamp::now_plus(chrono::Duration::seconds(3600));
    let (runs, count) = db
        .list_dev_runs_paginated_with_count(None, Some(&from_ts), Some(&to_ts), 50, 0)
        .unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(count, 2);

    // Far future — no matches
    let (runs, count) = db
        .list_dev_runs_paginated_with_count(None, Some("2099-01-01T00:00:00Z"), None, 50, 0)
        .unwrap();
    assert!(runs.is_empty());
    assert_eq!(count, 0);
}

#[test]
fn test_unified_timeline_uses_execution_trace_id() {
    let db = db();
    let task = NewTask {
        agent_id: "mika".to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: "timeline-test".to_string(),
        trigger_type: "time".to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: Some("2286-11-20T17:46:39Z".to_string()),
        timeout_at: None,
        action_type: "send_message".to_string(),
        action_config: "{}".to_string(),
        input_context: None,
        created_by_session: None,
        created_trace_id: Some("created-trace-111".to_string()),
        reference_url: None,
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: None,
    };
    let id = db.create_task(&task).unwrap();

    // Before execution: unified_timeline should show created_trace_id
    let rows: Vec<(Option<String>,)> = db
        .conn
        .prepare("SELECT trace_id FROM unified_timeline WHERE event_type = 'task' AND summary LIKE 'timeline-test%'")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?,)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0.as_deref(), Some("created-trace-111"));

    // After execution: unified_timeline should prefer execution_trace_id
    db.update_task_execution_trace_id(&id, "exec-trace-222")
        .unwrap();

    let rows: Vec<(Option<String>,)> = db
        .conn
        .prepare("SELECT trace_id FROM unified_timeline WHERE event_type = 'task' AND summary LIKE 'timeline-test%'")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?,)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0.as_deref(), Some("exec-trace-222"));
}

#[test]
fn test_get_last_completed_team_run_no_runs() {
    let db = db();
    let result = db.get_last_completed_team_run("nonexistent-team").unwrap();
    assert!(result.is_none());
}

#[test]
fn test_get_last_completed_team_run_filters_running() {
    let db = db();
    let run_id = "run-filter-1";
    db.insert_team_run(
        run_id,
        "filter-team",
        "test goal",
        3,
        "2020-01-01T00:00:00Z",
        None,
    )
    .unwrap();

    // Run is in "running" status — should not be returned
    let result = db.get_last_completed_team_run("filter-team").unwrap();
    assert!(result.is_none());

    // Complete the run
    db.update_team_run(
        run_id,
        "completed",
        None,
        2,
        Some("deliverable"),
        Some("2020-01-01T00:01:00Z"),
        0,
        false,
        None,
    )
    .unwrap();

    let result = db.get_last_completed_team_run("filter-team").unwrap();
    assert!(result.is_some());
    assert_eq!(result.unwrap().id, run_id);
}

#[test]
fn test_get_last_finished_team_run_excludes_suspended() {
    let db = db();
    // Insert a running run
    db.insert_team_run(
        "run-finished-1",
        "finished-team",
        "running goal",
        3,
        "2020-01-01T00:00:00Z",
        None,
    )
    .unwrap();

    // Running → should not be returned
    let result = db.get_last_finished_team_run("finished-team").unwrap();
    assert!(result.is_none());

    // Suspend it → still should not be returned
    db.update_team_run(
        "run-finished-1",
        "suspended",
        None,
        1,
        None,
        None,
        0,
        false,
        None,
    )
    .unwrap();
    let result = db.get_last_finished_team_run("finished-team").unwrap();
    assert!(result.is_none());

    // Insert a completed run
    db.insert_team_run(
        "run-finished-2",
        "finished-team",
        "completed goal",
        3,
        "2020-01-01T00:01:00Z",
        None,
    )
    .unwrap();
    db.update_team_run(
        "run-finished-2",
        "completed",
        None,
        2,
        Some("done"),
        Some("2020-01-01T00:02:00Z"),
        0,
        false,
        None,
    )
    .unwrap();

    let result = db.get_last_finished_team_run("finished-team").unwrap();
    assert!(result.is_some());
    assert_eq!(result.unwrap().id, "run-finished-2");

    // Insert a cancelled run (newer) — should also be returned
    db.insert_team_run(
        "run-finished-3",
        "finished-team",
        "cancelled goal",
        3,
        "2020-01-01T00:03:00Z",
        None,
    )
    .unwrap();
    db.update_team_run(
        "run-finished-3",
        "cancelled",
        None,
        0,
        None,
        Some("2020-01-01T00:03:30Z"),
        0,
        false,
        None,
    )
    .unwrap();

    let result = db.get_last_finished_team_run("finished-team").unwrap();
    assert!(result.is_some());
    assert_eq!(result.unwrap().id, "run-finished-3");
}

#[test]
fn test_team_run_failed_transport_round_trips() {
    // mika#1671 (AC3/AC5): the all-transport-failed short-circuit persists the
    // team_run as `failed_transport`. This proves the v49 CHECK constraint accepts
    // the new status (composed with the v46→v47 `failed_no_delegation` addition)
    // and the read path round-trips it (the terminal state that AC5 asserts the
    // run reaches without entering review/deliver).
    let db = db();
    db.insert_team_run(
        "run-transport-1",
        "transport-team",
        "odds engine goal",
        3,
        "2020-01-01T00:00:00Z",
        None,
    )
    .unwrap();

    db.update_team_run(
        "run-transport-1",
        "failed_transport",
        Some("all-delegations-transport-failed"),
        1,
        None,
        Some("2020-01-01T00:00:05Z"),
        0,
        false,
        None,
    )
    .unwrap();

    let row = db.load_team_run_by_id("run-transport-1").unwrap().unwrap();
    assert_eq!(row.status, "failed_transport");
    assert_eq!(
        row.failure_reason.as_deref(),
        Some("all-delegations-transport-failed")
    );

    // A failed_transport run is finished (terminal), not suspended/running.
    let finished = db.get_last_finished_team_run("transport-team").unwrap();
    assert_eq!(finished.unwrap().id, "run-transport-1");
}

#[test]
fn test_get_team_run_summary_basic() {
    let db = db();
    let run_id = "run-summary-1";
    db.insert_team_run(
        run_id,
        "summary-team",
        "test goal for summary",
        3,
        "2020-01-01T00:00:00Z",
        None,
    )
    .unwrap();
    db.update_team_run(
        run_id,
        "completed",
        None,
        1,
        Some("final output"),
        Some("2020-01-01T00:01:00Z"),
        0,
        false,
        None,
    )
    .unwrap();

    let summary = db.get_team_run_summary(run_id).unwrap().unwrap();
    assert_eq!(summary.run.id, run_id);
    assert_eq!(summary.run.goal, "test goal for summary");
    assert_eq!(summary.run.deliverable.as_deref(), Some("final output"));
    assert!(summary.agent_results.is_empty());
    assert!(summary.task_statuses.is_empty());
    assert!(summary.pending_tasks.is_empty());
    assert!(summary.critic_feedback.is_none());
}

#[test]
fn test_get_team_run_summary_with_critic() {
    let db = db();
    let run_id = "run-critic-1";
    db.insert_team_run(
        run_id,
        "critic-team",
        "critic test",
        3,
        "2020-01-01T00:00:00Z",
        None,
    )
    .unwrap();

    // Add critic feedback
    db.insert_team_workspace_entry(
        run_id,
        None,
        Some("critic"),
        "critic",
        "Needs improvement in error handling",
        1,
        None,
    )
    .unwrap();

    let summary = db.get_team_run_summary(run_id).unwrap().unwrap();
    assert_eq!(
        summary.critic_feedback.as_deref(),
        Some("Needs improvement in error handling")
    );
}

#[test]
fn test_get_team_run_summary_not_found() {
    let db = db();
    let result = db.get_team_run_summary("nonexistent-run-id").unwrap();
    assert!(result.is_none());
}

#[test]
fn test_truncate_chars_short() {
    assert_eq!(super::truncate_chars("hello", 10), "hello");
}

#[test]
fn test_truncate_chars_exact() {
    assert_eq!(super::truncate_chars("hello", 5), "hello");
}

#[test]
fn test_truncate_chars_long() {
    assert_eq!(super::truncate_chars("hello world", 5), "hello...");
}

#[test]
fn test_create_session_if_not_exists() {
    let db = db();
    // First call creates the session
    db.create_session_if_not_exists(
        "test-idempotent",
        "mika",
        "team",
        Some(r#"{"trigger":"team"}"#),
    )
    .unwrap();
    // Second call should not error (INSERT OR IGNORE)
    db.create_session_if_not_exists(
        "test-idempotent",
        "mika",
        "team",
        Some(r#"{"trigger":"team"}"#),
    )
    .unwrap();
    // Verify session exists
    let count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = 'test-idempotent'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_end_session_sets_ended_at() {
    let db = db();
    db.create_session("end-test", "mika", "system").unwrap();
    // ended_at should be NULL initially
    let ended: Option<String> = db
        .conn
        .query_row(
            "SELECT ended_at FROM sessions WHERE id = 'end-test'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(ended.is_none());
    // End the session
    db.end_session("end-test").unwrap();
    let ended: Option<String> = db
        .conn
        .query_row(
            "SELECT ended_at FROM sessions WHERE id = 'end-test'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(ended.is_some());
}

#[test]
fn test_get_or_create_canonical_session_is_idempotent() {
    // mika#1401: repeated calls reuse the same row (INSERT OR IGNORE), so every
    // `mika ask` to a singleton agent lands on one canonical session.
    // Uses the test DB's registered agent ("mika") to satisfy the sessions FK.
    let db = db();
    let id = "canonical-mika";
    let r1 = db
        .get_or_create_canonical_session(id, "mika", "cli")
        .unwrap();
    assert_eq!(r1, id);
    // Write a message to the canonical session.
    db.save_message("mika", id, "user", "first ask", None)
        .unwrap();
    // Second invocation must not fail on duplicate PK and must not clobber.
    let r2 = db
        .get_or_create_canonical_session(id, "mika", "cli")
        .unwrap();
    assert_eq!(r2, id);
    db.save_message("mika", id, "user", "second ask", None)
        .unwrap();

    // Exactly one session row exists, accumulating both messages.
    let session_count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(session_count, 1);
    let msg_count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(msg_count, 2);
}

#[test]
fn test_end_session_unless_canonical_noops_on_canonical() {
    // mika#1401: the canonical session must never be ended (pruning-safe).
    let db = db();
    let id = "canonical-mika";
    db.get_or_create_canonical_session(id, "mika", "cli")
        .unwrap();
    // Attempt to end it while passing itself as the canonical id → no-op.
    db.end_session_unless_canonical(id, Some(id)).unwrap();
    let ended: Option<String> = db
        .conn
        .query_row(
            "SELECT ended_at FROM sessions WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(ended.is_none(), "canonical session must stay open");
}

#[test]
fn test_end_session_unless_canonical_ends_non_canonical() {
    // A non-canonical session (or None canonical_id) ends normally.
    let db = db();
    db.create_session("per-ask", "mika", "cli").unwrap();
    db.end_session_unless_canonical("per-ask", Some("canonical-mika"))
        .unwrap();
    let ended: Option<String> = db
        .conn
        .query_row(
            "SELECT ended_at FROM sessions WHERE id = 'per-ask'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(ended.is_some(), "non-canonical session should be ended");

    // None canonical_id (non-singleton agent) also ends normally.
    db.create_session("per-ask-2", "mika", "cli").unwrap();
    db.end_session_unless_canonical("per-ask-2", None).unwrap();
    let ended: Option<String> = db
        .conn
        .query_row(
            "SELECT ended_at FROM sessions WHERE id = 'per-ask-2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(ended.is_some());
}

#[test]
fn test_canonical_session_exempt_from_pruning() {
    // mika#1401 + D5: the `canonical-` prefix is not in prune_old_sessions's
    // LIKE list, so even an (erroneously) ended canonical session survives.
    let db = db();
    let id = "canonical-mika";
    db.get_or_create_canonical_session(id, "mika", "cli")
        .unwrap();
    // Force-end and backdate to simulate a worst-case stale row.
    db.end_session(id).unwrap();
    db.conn
        .execute(
            "UPDATE sessions SET ended_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-30 days') WHERE id = ?1",
            params![id],
        )
        .unwrap();
    let pruned = db.prune_old_sessions(7 * 24 * 60 * 60).unwrap();
    assert_eq!(pruned, 0, "canonical prefix must be exempt from pruning");
    let count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_prune_old_sessions() {
    let db = db();
    // Create two ended sessions and one active (not ended)
    db.create_session("heartbeat-old", "mika", "system")
        .unwrap();
    db.create_session("heartbeat-new", "mika", "system")
        .unwrap();
    db.create_session("heartbeat-active", "mika", "system")
        .unwrap();
    // Save a message to heartbeat-old to verify cascade delete
    db.save_message("mika", "heartbeat-old", "user", "test msg", None)
        .unwrap();

    // End two sessions, but make one "old" by backdating ended_at
    db.end_session("heartbeat-old").unwrap();
    db.end_session("heartbeat-new").unwrap();
    // Backdate heartbeat-old to 10 days ago
    db.conn
        .execute(
            "UPDATE sessions SET ended_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-10 days') WHERE id = 'heartbeat-old'",
            [],
        )
        .unwrap();

    // Prune with 7-day retention
    let pruned = db.prune_old_sessions(7 * 24 * 60 * 60).unwrap();
    assert_eq!(pruned, 1); // Only heartbeat-old should be pruned

    // Verify heartbeat-old is gone (and its message cascaded)
    let count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = 'heartbeat-old'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);

    let msg_count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = 'heartbeat-old'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(msg_count, 0);

    // Verify heartbeat-new still exists (ended recently)
    let count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = 'heartbeat-new'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);

    // Verify heartbeat-active still exists (not ended)
    let count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = 'heartbeat-active'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_prune_targets_correct_prefixes() {
    let db = db();
    // Create sessions with various prefixes
    for prefix in &[
        "heartbeat-1",
        "callback-1",
        "skill-test-1",
        "reflection-2026-01-01",
        "team-run1-agent1",
    ] {
        db.create_session(prefix, "mika", "system").unwrap();
        db.end_session(prefix).unwrap();
        // Backdate to 10 days ago
        db.conn
            .execute(
                &format!(
                    "UPDATE sessions SET ended_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-10 days') WHERE id = '{prefix}'"
                ),
                [],
            )
            .unwrap();
    }
    // Delegate session uses "delegate" channel (not "system")
    db.create_session("delegate-task-1", "mika", "delegate")
        .unwrap();
    db.end_session("delegate-task-1").unwrap();
    db.conn
        .execute(
            "UPDATE sessions SET ended_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-10 days') WHERE id = 'delegate-task-1'",
            [],
        )
        .unwrap();
    // Also create a regular CLI session that should NOT be pruned
    db.create_session("cli-session", "mika", "cli").unwrap();
    db.end_session("cli-session").unwrap();
    db.conn
        .execute(
            "UPDATE sessions SET ended_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-10 days') WHERE id = 'cli-session'",
            [],
        )
        .unwrap();

    let pruned = db.prune_old_sessions(7 * 24 * 60 * 60).unwrap();
    assert_eq!(pruned, 6); // All prefixed sessions pruned, CLI session preserved

    // CLI session should still exist
    let count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = 'cli-session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_health_summary_empty_no_anomalies() {
    let db = db();
    let summary = db.get_task_health_summary("mika").unwrap();
    assert!(summary.active_tasks.is_empty());
    assert!(summary.anomalies.is_empty());
}

#[test]
fn test_health_summary_active_tasks() {
    let db = db();
    let task = NewTask {
        reference_url: Some("https://github.com/org/repo/issues/1".to_string()),
        ..new_task("mika", "Fix bug", "manual", "none")
    };
    db.create_task(&task).unwrap();
    let summary = db.get_task_health_summary("mika").unwrap();
    assert_eq!(summary.active_tasks.len(), 1);
    assert_eq!(summary.active_tasks[0].label, "Fix bug");
    // Also detected as github_linked anomaly
    assert!(
        summary
            .anomalies
            .iter()
            .any(|a| a.anomaly_type == "github_linked")
    );
}

#[test]
fn test_health_summary_stuck_callback() {
    let db = db();
    let id = db
        .create_task(&new_task(
            "mika",
            "Build deploy",
            "callback",
            "resume_agent",
        ))
        .unwrap();
    // Mark as completed with a timestamp >10 min ago
    let old_time = timestamp::format(&(Utc::now() - Duration::seconds(700)));
    db.conn
        .execute(
            "UPDATE tasks SET status = 'completed', updated_at = ?1 WHERE id = ?2",
            params![old_time, id],
        )
        .unwrap();

    let summary = db.get_task_health_summary("mika").unwrap();
    assert_eq!(summary.anomalies.len(), 1);
    assert_eq!(summary.anomalies[0].anomaly_type, "stuck_callback");
    assert_eq!(summary.anomalies[0].label, "Build deploy");
    assert!(summary.anomalies[0].age_description.starts_with("stuck "));
}

#[test]
fn test_health_summary_stuck_callback_not_triggered_within_threshold() {
    let db = db();
    let id = db
        .create_task(&new_task(
            "mika",
            "Recent callback",
            "callback",
            "resume_agent",
        ))
        .unwrap();
    // Mark as completed just 2 minutes ago (within 10 min threshold)
    let recent_time = timestamp::format(&(Utc::now() - Duration::seconds(120)));
    db.conn
        .execute(
            "UPDATE tasks SET status = 'completed', updated_at = ?1 WHERE id = ?2",
            params![recent_time, id],
        )
        .unwrap();

    let summary = db.get_task_health_summary("mika").unwrap();
    // Should NOT appear as stuck_callback
    assert!(
        summary
            .anomalies
            .iter()
            .all(|a| a.anomaly_type != "stuck_callback")
    );
}

#[test]
fn test_health_summary_failed_recurring() {
    let db = db();
    let id = db
        .create_task(&new_task("mika", "Heartbeat", "recurring", "run_skill"))
        .unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'failed' WHERE id = ?1",
            params![id],
        )
        .unwrap();

    let summary = db.get_task_health_summary("mika").unwrap();
    assert!(
        summary
            .anomalies
            .iter()
            .any(|a| a.anomaly_type == "failed_recurring")
    );
}

#[test]
fn test_health_summary_long_running() {
    let db = db();
    let id = db
        .create_task(&new_task("mika", "Slow task", "time", "run_skill"))
        .unwrap();
    let old_time = timestamp::format(&(Utc::now() - Duration::seconds(7200)));
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress', fired_at = ?1 WHERE id = ?2",
            params![old_time, id],
        )
        .unwrap();

    let summary = db.get_task_health_summary("mika").unwrap();
    assert!(
        summary
            .anomalies
            .iter()
            .any(|a| a.anomaly_type == "long_running")
    );
}

#[test]
fn test_health_summary_stale_blocked() {
    let db = db();
    let id = db
        .create_task(&new_task("mika", "Blocked item", "manual", "none"))
        .unwrap();
    let old_time = timestamp::format(&(Utc::now() - Duration::seconds(90_000)));
    db.conn
        .execute(
            "UPDATE tasks SET status = 'blocked', updated_at = ?1 WHERE id = ?2",
            params![old_time, id],
        )
        .unwrap();

    let summary = db.get_task_health_summary("mika").unwrap();
    assert!(
        summary
            .anomalies
            .iter()
            .any(|a| a.anomaly_type == "stale_blocked")
    );
}

#[test]
fn test_health_summary_agent_scoping() {
    let db = db();
    // Register a second agent
    db.conn
        .execute(
            "INSERT OR IGNORE INTO agents (id, name) VALUES ('other', 'Other')",
            [],
        )
        .unwrap();
    // Create a stuck callback for "other" agent
    let id = db
        .create_task(&new_task(
            "other",
            "Other build",
            "callback",
            "resume_agent",
        ))
        .unwrap();
    let old_time = timestamp::format(&(Utc::now() - Duration::seconds(700)));
    db.conn
        .execute(
            "UPDATE tasks SET status = 'completed', updated_at = ?1 WHERE id = ?2",
            params![old_time, id],
        )
        .unwrap();

    // Mika should see no anomalies
    let summary = db.get_task_health_summary("mika").unwrap();
    assert!(summary.anomalies.is_empty());

    // Other should see the stuck callback
    let summary = db.get_task_health_summary("other").unwrap();
    assert_eq!(summary.anomalies.len(), 1);
    assert_eq!(summary.anomalies[0].anomaly_type, "stuck_callback");
}

#[test]
fn test_health_summary_anomaly_cap() {
    let db = db();
    // Create 15 stuck callbacks — only 10 should be returned
    for i in 0..15 {
        let id = db
            .create_task(&new_task(
                "mika",
                &format!("Build {i}"),
                "callback",
                "resume_agent",
            ))
            .unwrap();
        let old_time = timestamp::format(&(Utc::now() - Duration::seconds(700 + i * 60)));
        db.conn
            .execute(
                "UPDATE tasks SET status = 'completed', updated_at = ?1 WHERE id = ?2",
                params![old_time, id],
            )
            .unwrap();
    }

    let summary = db.get_task_health_summary("mika").unwrap();
    assert!(summary.anomalies.len() <= 10);
}

#[test]
fn test_format_age_hours_minutes() {
    let now = Utc::now();
    let ts = timestamp::format(&(now - Duration::seconds(7200 + 1320)));
    assert_eq!(format_age(&ts, now), "2h 22m");
}

#[test]
fn test_format_age_days() {
    let now = Utc::now();
    let ts = timestamp::format(&(now - Duration::seconds(86_400 * 5)));
    assert_eq!(format_age(&ts, now), "5d");
}

#[test]
fn test_format_age_invalid_timestamp() {
    let now = Utc::now();
    assert_eq!(format_age("not-a-timestamp", now), "unknown");
}

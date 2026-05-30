//! Regression tests for the `mika ask --task-id` correlation path (#752).
//!
//! A caller issues `mika ask --task-id <uuid>` for tasks
//! owned by a different agent (e.g. `mika-dev`). The correlation branch must
//! use `get_task_unscoped` so the lookup succeeds across agent boundaries.
//! The contrast test proves an agent-scoped lookup on the same task returns
//! `None`, which is exactly the failure mode the fix eliminates.

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{Database, NewTask};

async fn open_shared_db(agent_a: &str, agent_b: &str) -> AsyncDatabase {
    let db = AsyncDatabase::new_with_agent(Database::open_in_memory().unwrap(), agent_a);
    db.register_agent(agent_a, agent_a, "").await.unwrap();
    db.register_agent(agent_b, agent_b, "").await.unwrap();
    db
}

fn new_task(agent_id: &str, label: &str) -> NewTask {
    NewTask {
        agent_id: agent_id.to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: label.to_string(),
        trigger_type: "callback".to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: None,
        timeout_at: None,
        action_type: "resume_agent".to_string(),
        action_config: "{}".to_string(),
        input_context: None,
        created_by_session: None,
        created_trace_id: None,
        reference_url: None,
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: None,
    }
}

#[tokio::test]
async fn unscoped_lookup_finds_cross_agent_task() {
    let db_a = open_shared_db("agent-a", "agent-b").await;
    let task_id = db_a
        .create_task(new_task("agent-a", "agent-a owned task"))
        .await
        .unwrap();

    let db_b = db_a.with_agent("agent-b");
    let task = db_b
        .get_task_unscoped(&task_id)
        .await
        .expect("unscoped lookup must not error")
        .expect("unscoped lookup must find cross-agent task");

    assert_eq!(task.id, task_id);
    assert_eq!(task.agent_id, "agent-a");
    assert_eq!(task.label, "agent-a owned task");
}

#[tokio::test]
async fn scoped_lookup_rejects_cross_agent_task() {
    let db_a = open_shared_db("agent-a", "agent-b").await;
    let task_id = db_a
        .create_task(new_task("agent-a", "agent-a owned task"))
        .await
        .unwrap();

    let db_b = db_a.with_agent("agent-b");
    let task = db_b
        .get_task(&task_id)
        .await
        .expect("scoped lookup must not error");

    assert!(task.is_none(), "scoped lookup must reject cross-agent task");
}

#[tokio::test]
async fn unscoped_lookup_returns_none_for_nonexistent_task() {
    let db = AsyncDatabase::new_with_agent(Database::open_in_memory().unwrap(), "agent-a");
    let task = db
        .get_task_unscoped("00000000-0000-0000-0000-000000000000")
        .await
        .expect("unscoped lookup must not error");

    assert!(task.is_none());
}

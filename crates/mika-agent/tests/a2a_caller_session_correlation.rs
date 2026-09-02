//! Concurrent-separation proof for the caller session id (mika#2070).
//!
//! `turn_usage` is RT-005's primary measurement channel, and until mika#2070 it
//! carried spirit's own `a2a-<task_id>` session: a run's turns could only be
//! recovered by time slice, over a log holding 21 084 events from every origin
//! on the box. Slice attribution mixes in turns the run never paid for, and the
//! resulting error tracks load — so it can track the experimental cell, which is
//! the one contamination a 2x2 design cannot absorb.
//!
//! What these tests pin is the property that filtering depends on: interleaved
//! A2A tasks each resolve to their *own* caller's session, and an unclaimable id
//! lands on a minted session disjoint from both. The remaining hop — from the
//! resolved session to the `session_id` field of the log line — is an unbranched
//! argument pass: `a2a_create_task`'s return value becomes
//! `AgentParams.session_id`, which `emit_turn_usage` prints verbatim.
//!
//! Plan: `docs/plans/2026-08-30-001-fix-2070-caller-session-id-reaches-agent-params-plan.md`
//! Unit: U4.

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;

/// One shared container database, the way the CLI and the spirit daemon see it:
/// `mika ask` writes its session row here, and spirit reads it back over A2A.
async fn shared_db(agent: &str) -> AsyncDatabase {
    let db = AsyncDatabase::new_with_agent(Database::open_in_memory().unwrap(), agent);
    db.register_agent(agent, agent, "").await.unwrap();
    db
}

#[tokio::test]
async fn interleaved_tasks_keep_their_own_caller_sessions() {
    let db = shared_db("mika").await;
    db.create_session("probe-a", "mika", "cli").await.unwrap();
    db.create_session("probe-b", "mika", "cli").await.unwrap();

    // Interleave the way two concurrent invocations reach spirit: both sessions
    // exist before either task is created.
    let session_a = db
        .a2a_create_task("task-a", None, Some("probe-a"))
        .await
        .unwrap();
    let session_b = db
        .a2a_create_task("task-b", None, Some("probe-b"))
        .await
        .unwrap();
    let session_c = db.a2a_create_task("task-c", None, None).await.unwrap();

    assert_eq!(session_a, "probe-a");
    assert_eq!(session_b, "probe-b");
    // The uncorrelated caller still runs — it just keeps spirit's minted session.
    assert_eq!(session_c, "a2a-task-c");

    // Disjoint: a filter on any one of the three catches only its own turns.
    assert_ne!(session_a, session_b);
    assert_ne!(session_a, session_c);
    assert_ne!(session_b, session_c);
}

#[tokio::test]
async fn each_task_maps_back_to_its_own_session() {
    let db = shared_db("mika").await;
    db.create_session("probe-a", "mika", "cli").await.unwrap();
    db.create_session("probe-b", "mika", "cli").await.unwrap();

    db.a2a_create_task("task-a", None, Some("probe-a"))
        .await
        .unwrap();
    db.a2a_create_task("task-b", None, Some("probe-b"))
        .await
        .unwrap();

    assert_eq!(
        db.a2a_get_session_id("task-a").await.unwrap(),
        Some("probe-a".to_string())
    );
    assert_eq!(
        db.a2a_get_session_id("task-b").await.unwrap(),
        Some("probe-b".to_string())
    );
}

#[tokio::test]
async fn a_foreign_session_cannot_be_claimed_by_another_agent() {
    let db = shared_db("mika").await;
    db.register_agent("mika-dev", "mika-dev", "").await.unwrap();
    db.create_session("dev-owned", "mika-dev", "cli")
        .await
        .unwrap();

    let session = db
        .a2a_create_task("task-a", None, Some("dev-owned"))
        .await
        .unwrap();

    // Refused, not adopted — otherwise one agent's turns would be logged under
    // another agent's session and both measurements would be wrong.
    assert_eq!(session, "a2a-task-a");
}

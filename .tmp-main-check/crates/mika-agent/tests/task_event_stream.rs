//! Integration tests for `TaskEventFrame` emission from `AsyncDatabase`
//! wrapper methods (mika#1758).
//!
//! These tests exercise the public wrapper surface end-to-end: attach a real
//! `TaskEventsChannel`, subscribe, drive lifecycle transitions through
//! `AsyncDatabase`, and assert the exact frame sequence delivered on the wire.
//!
//! Companion to the unit tests in `async_db.rs::tests` which cover the finer
//! per-method invariants (no-frame-on-Ok(false), truncation).

use std::sync::Arc;
use std::time::Duration;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{Database, NewTask};
use mika_agent::server::tasks_stream::{TaskEventFrame, TaskEventsChannel};
use tokio::sync::broadcast::Receiver;

/// Agent id used across these tests. Must be registered in the FK-owning
/// `agents` table before any task INSERT (tasks.agent_id FKs to agents.id).
const TEST_AGENT: &str = "mika-test";

/// Build an in-memory AsyncDatabase wired to a fresh TaskEventsChannel, and
/// return a subscriber receiver alongside. All lifecycle transitions on the
/// returned `AsyncDatabase` will emit on the returned receiver.
async fn setup_db_with_channel() -> (AsyncDatabase, Receiver<TaskEventFrame>) {
    let db = Database::open_in_memory().unwrap();
    let async_db = AsyncDatabase::new_with_agent(db, TEST_AGENT);
    // FK: tasks.agent_id → agents.id. Register the agent so create_task
    // does not fail with FOREIGN KEY constraint failed.
    async_db
        .register_agent(TEST_AGENT, TEST_AGENT, "/tmp/mika-test-home")
        .await
        .unwrap();
    let channel = Arc::new(TaskEventsChannel::new());
    // Subscribe BEFORE attaching so the first broadcast reaches this receiver.
    let rx = channel.subscribe();
    async_db.set_task_events_channel(channel);
    (async_db, rx)
}

fn make_test_task(label: &str) -> NewTask {
    NewTask {
        agent_id: TEST_AGENT.to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: label.to_string(),
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
    }
}

/// Timeout-bounded recv so a missing frame fails fast rather than hanging CI.
async fn recv_next(rx: &mut Receiver<TaskEventFrame>) -> TaskEventFrame {
    tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timed out waiting for TaskEventFrame")
        .expect("channel closed unexpectedly")
}

/// AC3: full lifecycle `create → claim → complete → mark_delivered` yields
/// exactly four frames in the correct order with the correct fields.
#[tokio::test]
async fn full_lifecycle_create_claim_complete_deliver_emits_four_frames_in_order() {
    let (db, mut rx) = setup_db_with_channel().await;

    // Callback task (mark_task_delivered gates on trigger_type='callback',
    // action_type='resume_agent' — set those here).
    let mut task = make_test_task("lifecycle test");
    task.trigger_type = "callback".to_string();
    task.action_type = "resume_agent".to_string();
    let id = db.create_task(task).await.unwrap();

    // Frame 1: TaskCreated
    match recv_next(&mut rx).await {
        TaskEventFrame::TaskCreated {
            task_id,
            agent_id,
            kind,
            action_type,
            label,
            ..
        } => {
            assert_eq!(task_id, id);
            assert_eq!(agent_id, TEST_AGENT);
            assert_eq!(kind, "callback");
            assert_eq!(action_type, "resume_agent");
            assert_eq!(label.as_deref(), Some("lifecycle test"));
        }
        other => panic!("expected TaskCreated, got {other:?}"),
    }

    // Claim
    assert!(db.claim_and_fire_task(&id).await.unwrap());

    // Frame 2: TaskClaimed
    match recv_next(&mut rx).await {
        TaskEventFrame::TaskClaimed {
            task_id, agent_id, ..
        } => {
            assert_eq!(task_id, id);
            assert_eq!(agent_id, TEST_AGENT);
        }
        other => panic!("expected TaskClaimed, got {other:?}"),
    }

    // Complete
    assert!(
        db.update_task_completed(&id, Some("result body"))
            .await
            .unwrap()
    );

    // Frame 3: TaskCompleted
    match recv_next(&mut rx).await {
        TaskEventFrame::TaskCompleted {
            task_id,
            agent_id,
            result_preview,
            ..
        } => {
            assert_eq!(task_id, id);
            assert_eq!(agent_id, TEST_AGENT);
            assert_eq!(result_preview.as_deref(), Some("result body"));
        }
        other => panic!("expected TaskCompleted, got {other:?}"),
    }

    // Mark delivered
    assert!(db.mark_task_delivered(&id).await.unwrap());

    // Frame 4: TaskDelivered
    match recv_next(&mut rx).await {
        TaskEventFrame::TaskDelivered {
            task_id, agent_id, ..
        } => {
            assert_eq!(task_id, id);
            assert_eq!(agent_id, TEST_AGENT);
        }
        other => panic!("expected TaskDelivered, got {other:?}"),
    }

    // No stray frames: try_recv should now report empty.
    assert!(rx.try_recv().is_err(), "unexpected extra frame");
}

/// `create → claim → fail` emits three frames in order (create/claim/fail).
#[tokio::test]
async fn create_claim_fail_emits_three_frames_in_order() {
    let (db, mut rx) = setup_db_with_channel().await;

    let id = db.create_task(make_test_task("fail path")).await.unwrap();
    let _ = recv_next(&mut rx).await; // TaskCreated

    assert!(db.claim_and_fire_task(&id).await.unwrap());
    let _ = recv_next(&mut rx).await; // TaskClaimed

    assert!(db.update_task_failed(&id, "boom").await.unwrap());
    match recv_next(&mut rx).await {
        TaskEventFrame::TaskFailed {
            task_id,
            error_preview,
            ..
        } => {
            assert_eq!(task_id, id);
            assert_eq!(error_preview.as_deref(), Some("boom"));
        }
        other => panic!("expected TaskFailed, got {other:?}"),
    }
    assert!(rx.try_recv().is_err());
}

/// `create → cancel_task` emits two frames in order.
#[tokio::test]
async fn create_then_cancel_emits_two_frames_in_order() {
    let (db, mut rx) = setup_db_with_channel().await;

    let id = db.create_task(make_test_task("cancel path")).await.unwrap();
    let _ = recv_next(&mut rx).await; // TaskCreated

    assert!(db.cancel_task(&id).await.unwrap());
    match recv_next(&mut rx).await {
        TaskEventFrame::TaskCancelled {
            task_id,
            agent_id,
            reason,
            ..
        } => {
            assert_eq!(task_id, id);
            assert_eq!(agent_id, TEST_AGENT);
            assert_eq!(reason.as_deref(), Some("cancelled_via_cancel_task"));
        }
        other => panic!("expected TaskCancelled, got {other:?}"),
    }
    assert!(rx.try_recv().is_err());
}

/// AC2: `update_task_completed` returning `Ok(false)` (task already in terminal
/// state) MUST NOT emit a frame. This is the "no phantom frame on no-op DB
/// write" invariant.
#[tokio::test]
async fn no_frame_when_update_task_completed_returns_false() {
    let (db, mut rx) = setup_db_with_channel().await;

    let id = db.create_task(make_test_task("no-op")).await.unwrap();
    let _ = recv_next(&mut rx).await; // drain TaskCreated

    // Cancel to move out of pending → completion attempt should return Ok(false)
    assert!(db.cancel_task(&id).await.unwrap());
    let _ = recv_next(&mut rx).await; // drain TaskCancelled

    // Now try to complete — DB guard rejects (status IN ('pending','in_progress'))
    let transitioned = db.update_task_completed(&id, Some("late")).await.unwrap();
    assert!(!transitioned);

    // No TaskCompleted frame should have fired.
    assert!(rx.try_recv().is_err(), "no-op should not emit frame");
}

/// AC2: `claim_and_fire_task` returning `Ok(false)` (task no longer claimable)
/// MUST NOT emit a frame.
#[tokio::test]
async fn no_frame_when_claim_returns_false() {
    let (db, mut rx) = setup_db_with_channel().await;

    let id = db.create_task(make_test_task("claim-race")).await.unwrap();
    let _ = recv_next(&mut rx).await; // TaskCreated

    // Cancel first to make the task un-claimable.
    assert!(db.cancel_task(&id).await.unwrap());
    let _ = recv_next(&mut rx).await; // TaskCancelled

    let claimed = db.claim_and_fire_task(&id).await.unwrap();
    assert!(!claimed);
    assert!(rx.try_recv().is_err());
}

/// AC2: `mark_task_delivered` returning `Ok(false)` (already delivered / not
/// completed) MUST NOT emit a frame.
#[tokio::test]
async fn no_frame_when_mark_delivered_returns_false() {
    let (db, mut rx) = setup_db_with_channel().await;

    // Callback task shape so mark_task_delivered can succeed later.
    let mut task = make_test_task("deliver-noop");
    task.trigger_type = "callback".to_string();
    task.action_type = "resume_agent".to_string();
    let id = db.create_task(task).await.unwrap();
    let _ = recv_next(&mut rx).await; // TaskCreated

    // Try to mark_delivered before the task completes — guarded UPDATE returns false.
    let delivered = db.mark_task_delivered(&id).await.unwrap();
    assert!(!delivered);
    assert!(rx.try_recv().is_err());
}

/// AC2 (fire-and-forget): DB writes succeed even with zero subscribers.
#[tokio::test]
async fn db_writes_succeed_with_zero_subscribers() {
    let db_inner = Database::open_in_memory().unwrap();
    let async_db = AsyncDatabase::new_with_agent(db_inner, TEST_AGENT);
    async_db
        .register_agent(TEST_AGENT, TEST_AGENT, "/tmp/mika-test-home")
        .await
        .unwrap();
    let channel = Arc::new(TaskEventsChannel::new());
    async_db.set_task_events_channel(channel);
    // NOTE: never call `.subscribe()` — zero subscribers.

    let id = async_db
        .create_task(make_test_task("no-subs"))
        .await
        .unwrap();
    assert!(async_db.claim_and_fire_task(&id).await.unwrap());
    assert!(
        async_db
            .update_task_completed(&id, Some("done"))
            .await
            .unwrap()
    );
}

/// AC2 (unattached-channel): DB writes succeed and never panic when no channel
/// is attached. This is the CLI/test-default state and the safety-net for any
/// non-server caller.
#[tokio::test]
async fn db_writes_succeed_when_channel_unattached() {
    let db_inner = Database::open_in_memory().unwrap();
    let async_db = AsyncDatabase::new_with_agent(db_inner, TEST_AGENT);
    async_db
        .register_agent(TEST_AGENT, TEST_AGENT, "/tmp/mika-test-home")
        .await
        .unwrap();
    // NOTE: never call `set_task_events_channel` — channel is `None`.

    let id = async_db
        .create_task(make_test_task("no-channel"))
        .await
        .unwrap();
    assert!(async_db.claim_and_fire_task(&id).await.unwrap());
    assert!(
        async_db
            .update_task_completed(&id, Some("done"))
            .await
            .unwrap()
    );
    assert!(!async_db.cancel_task(&id).await.unwrap()); // already terminal
}

/// R6/AC4-adjacent: matched-drain-rate test. Producer emits N frames, single
/// subscriber drains them in real time; assert zero `OverflowMarker` observed
/// under this normal (not overload) load. Deterministic, CI-safe. The full
/// 30-second 10-subscribers 100-eps soak is documented as a manual/#[ignore]
/// invocation elsewhere.
#[tokio::test]
async fn matched_drain_emits_no_overflow_marker() {
    let (db, mut rx) = setup_db_with_channel().await;

    // Emit 50 lifecycle transitions in a row — each create+claim+complete is
    // 3 frames = 150 total, comfortably under CHANNEL_CAP (256) even without
    // draining. A concurrent drain task consumes each frame as it arrives.
    let drain = tokio::spawn(async move {
        let mut count = 0usize;
        let mut overflow = 0usize;
        while let Ok(Ok(frame)) = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await
        {
            if matches!(frame, TaskEventFrame::OverflowMarker { .. }) {
                overflow += 1;
            }
            count += 1;
        }
        (count, overflow)
    });

    for i in 0..50 {
        let id = db
            .create_task(make_test_task(&format!("burst-{i}")))
            .await
            .unwrap();
        assert!(db.claim_and_fire_task(&id).await.unwrap());
        assert!(db.update_task_completed(&id, None).await.unwrap());
    }

    let (count, overflow) = drain.await.unwrap();
    assert_eq!(overflow, 0, "matched-drain load must not overflow");
    assert!(count >= 150, "expected >=150 frames, got {count}");
}

// ── Additional emission-site + no-op tests (post-review) ──────────────────

/// `promote_task_completed` (failed→completed) MUST emit `TaskCompleted`
/// (plan KTD5). Otherwise the wire hides a real terminal transition.
#[tokio::test]
async fn promote_task_completed_emits_task_completed_frame() {
    let (db, mut rx) = setup_db_with_channel().await;

    let id = db
        .create_task(make_test_task("retry candidate"))
        .await
        .unwrap();
    let _ = recv_next(&mut rx).await; // TaskCreated
    assert!(db.claim_and_fire_task(&id).await.unwrap());
    let _ = recv_next(&mut rx).await; // TaskClaimed
    assert!(db.update_task_failed(&id, "flaky ci").await.unwrap());
    let _ = recv_next(&mut rx).await; // TaskFailed

    // failed → completed promotion (mika#958 retry-promoter path).
    assert!(db.promote_task_completed(&id, "retry_ok").await.unwrap());
    match recv_next(&mut rx).await {
        TaskEventFrame::TaskCompleted {
            task_id,
            agent_id,
            result_preview,
            ..
        } => {
            assert_eq!(task_id, id);
            assert_eq!(agent_id, TEST_AGENT);
            assert_eq!(result_preview.as_deref(), Some("retry_ok"));
        }
        other => panic!("expected TaskCompleted after promotion, got {other:?}"),
    }
    assert!(rx.try_recv().is_err());
}

/// `promote_task_completed` on a non-failed task returns `Ok(false)` and MUST
/// NOT emit. Guards the transition-only invariant for the retry-promoter path.
#[tokio::test]
async fn no_frame_when_promote_task_completed_returns_false() {
    let (db, mut rx) = setup_db_with_channel().await;

    let id = db.create_task(make_test_task("not-failed")).await.unwrap();
    let _ = recv_next(&mut rx).await; // drain TaskCreated

    // Task is `pending`, not `failed` — promote should return Ok(false).
    let promoted = db.promote_task_completed(&id, "wrong-state").await.unwrap();
    assert!(!promoted, "promotion must no-op on a non-failed task");
    assert!(rx.try_recv().is_err(), "no frame expected for no-op");
}

/// `update_task_status(id, "cancelled")` MUST emit `TaskCancelled` with the
/// distinctive reason string. Real caller: `teams/engine.rs` cancelling a
/// team-parent task.
#[tokio::test]
async fn update_task_status_to_cancelled_emits_frame() {
    let (db, mut rx) = setup_db_with_channel().await;

    let id = db.create_task(make_test_task("team-parent")).await.unwrap();
    let _ = recv_next(&mut rx).await; // TaskCreated

    db.update_task_status(&id, "cancelled").await.unwrap();
    match recv_next(&mut rx).await {
        TaskEventFrame::TaskCancelled {
            task_id, reason, ..
        } => {
            assert_eq!(task_id, id);
            assert_eq!(reason.as_deref(), Some("cancelled_via_update_task_status"));
        }
        other => panic!("expected TaskCancelled, got {other:?}"),
    }
    assert!(rx.try_recv().is_err());
}

/// `update_task_status(id, non_cancel_status)` MUST NOT emit any frame — the
/// generic status-write path only fires for the cancel branch.
#[tokio::test]
async fn no_frame_when_update_task_status_is_not_cancel() {
    let (db, mut rx) = setup_db_with_channel().await;

    let id = db
        .create_task(make_test_task("non-cancel status"))
        .await
        .unwrap();
    let _ = recv_next(&mut rx).await; // TaskCreated

    db.update_task_status(&id, "in_progress").await.unwrap();
    assert!(rx.try_recv().is_err());
    db.update_task_status(&id, "failed").await.unwrap();
    assert!(rx.try_recv().is_err());
}

/// `update_task_status(id, "cancelled")` MUST NOT emit a spurious frame when
/// the row is already `cancelled` (idempotent re-cancel). Guards the F2
/// (post-review) fix that pre-fetches prior status.
#[tokio::test]
async fn no_frame_on_redundant_update_task_status_cancel() {
    let (db, mut rx) = setup_db_with_channel().await;

    let id = db
        .create_task(make_test_task("double-cancel"))
        .await
        .unwrap();
    let _ = recv_next(&mut rx).await; // TaskCreated

    db.update_task_status(&id, "cancelled").await.unwrap();
    let _ = recv_next(&mut rx).await; // First TaskCancelled

    // Second cancel on the same row — DB write silently updates updated_at,
    // no logical transition; wire must stay silent.
    db.update_task_status(&id, "cancelled").await.unwrap();
    assert!(rx.try_recv().is_err(), "re-cancel must not fire a frame");
}

/// `update_task_status` on a non-existent task MUST NOT emit — the underlying
/// UPDATE affects zero rows, no transition.
#[tokio::test]
async fn no_frame_when_update_task_status_hits_no_row() {
    let (db, mut rx) = setup_db_with_channel().await;

    // No `create_task` first — the id is unknown.
    db.update_task_status("00000000-0000-0000-0000-000000000000", "cancelled")
        .await
        .unwrap();
    assert!(rx.try_recv().is_err());
}

/// `update_manual_task_status(id, "cancelled")` MUST emit `TaskCancelled` with
/// its distinctive reason string. Third of the three cancel emission paths.
#[tokio::test]
async fn update_manual_task_status_to_cancelled_emits_frame() {
    let (db, mut rx) = setup_db_with_channel().await;

    let id = db.create_task(make_test_task("manual-task")).await.unwrap();
    let _ = recv_next(&mut rx).await; // TaskCreated

    let prior = db
        .update_manual_task_status(&id, "cancelled")
        .await
        .unwrap();
    assert!(prior.is_some(), "wrapper must report the transition");
    match recv_next(&mut rx).await {
        TaskEventFrame::TaskCancelled {
            task_id, reason, ..
        } => {
            assert_eq!(task_id, id);
            assert_eq!(
                reason.as_deref(),
                Some("cancelled_via_update_manual_task_status")
            );
        }
        other => panic!("expected TaskCancelled, got {other:?}"),
    }
    assert!(rx.try_recv().is_err());
}

/// `update_manual_task_status` returning `Ok(None)` (no matching row) MUST NOT
/// emit. Guards the same transition-only invariant as `cancel_task`.
#[tokio::test]
async fn no_frame_when_update_manual_task_status_returns_none() {
    let (db, mut rx) = setup_db_with_channel().await;

    let outcome = db
        .update_manual_task_status("00000000-0000-0000-0000-000000000000", "cancelled")
        .await
        .unwrap();
    assert!(outcome.is_none());
    assert!(rx.try_recv().is_err());
}

/// AC2 sibling: `update_task_failed` returning `Ok(false)` (task already
/// terminal) MUST NOT emit `TaskFailed`.
#[tokio::test]
async fn no_frame_when_update_task_failed_returns_false() {
    let (db, mut rx) = setup_db_with_channel().await;

    let id = db.create_task(make_test_task("fail-noop")).await.unwrap();
    let _ = recv_next(&mut rx).await; // TaskCreated
    assert!(db.cancel_task(&id).await.unwrap());
    let _ = recv_next(&mut rx).await; // TaskCancelled

    // Task is `cancelled` — update_task_failed's guarded UPDATE must not fire.
    let transitioned = db.update_task_failed(&id, "late").await.unwrap();
    assert!(!transitioned);
    assert!(rx.try_recv().is_err());
}

/// AC2 sibling: `cancel_task` returning `Ok(false)` on a subsequent call
/// (already-terminal state) MUST NOT emit a second `TaskCancelled` frame.
/// Exercises the guard with an attached subscriber (companion to the
/// unattached-channel test above).
#[tokio::test]
async fn no_frame_on_second_cancel_task_with_subscriber() {
    let (db, mut rx) = setup_db_with_channel().await;

    let id = db
        .create_task(make_test_task("double-cancel"))
        .await
        .unwrap();
    let _ = recv_next(&mut rx).await; // TaskCreated
    assert!(db.cancel_task(&id).await.unwrap());
    let _ = recv_next(&mut rx).await; // TaskCancelled #1

    // Second cancel returns Ok(false) — task is already terminal.
    let cancelled = db.cancel_task(&id).await.unwrap();
    assert!(!cancelled);
    assert!(rx.try_recv().is_err(), "no second TaskCancelled expected");
}

/// End-to-end truncation invariant: the wrapper drives
/// `TaskEventFrame::completed`/`::failed`, which UTF-8-safely truncates the
/// preview at 500 chars. This test catches a regression that swaps the helper
/// for a struct literal (dropping truncation).
#[tokio::test]
async fn wrapper_truncates_result_and_error_previews() {
    let (db, mut rx) = setup_db_with_channel().await;

    let id = db
        .create_task(make_test_task("truncation lifecycle"))
        .await
        .unwrap();
    let _ = recv_next(&mut rx).await; // TaskCreated
    assert!(db.claim_and_fire_task(&id).await.unwrap());
    let _ = recv_next(&mut rx).await; // TaskClaimed

    // ≥1000-char result → preview must be ≤501 chars (500 + ellipsis).
    let long_result = "x".repeat(1000);
    assert!(
        db.update_task_completed(&id, Some(&long_result))
            .await
            .unwrap()
    );
    match recv_next(&mut rx).await {
        TaskEventFrame::TaskCompleted { result_preview, .. } => {
            let preview = result_preview.expect("preview should be Some");
            assert!(
                preview.chars().count() <= 501,
                "preview len {} > 501",
                preview.chars().count()
            );
            assert!(preview.ends_with('…'));
        }
        other => panic!("expected TaskCompleted, got {other:?}"),
    }

    // Second lifecycle for the failed path.
    let id2 = db.create_task(make_test_task("fail-trunc")).await.unwrap();
    let _ = recv_next(&mut rx).await; // TaskCreated
    assert!(db.claim_and_fire_task(&id2).await.unwrap());
    let _ = recv_next(&mut rx).await; // TaskClaimed

    let long_error = "e".repeat(1000);
    assert!(db.update_task_failed(&id2, &long_error).await.unwrap());
    match recv_next(&mut rx).await {
        TaskEventFrame::TaskFailed { error_preview, .. } => {
            let preview = error_preview.expect("preview should be Some");
            assert!(preview.chars().count() <= 501);
            assert!(preview.ends_with('…'));
        }
        other => panic!("expected TaskFailed, got {other:?}"),
    }
}

/// `AsyncDatabase::with_agent()` returns a handle scoped to a different
/// `agent_id` but sharing the same `Inner` (and therefore the same
/// `TaskEventsChannel`). Emissions from the clone MUST carry the CLONE's
/// agent_id — this proves the design invariant that supports the multi-tenant
/// v2 upgrade path documented in the ticket body.
#[tokio::test]
async fn with_agent_clone_emits_with_new_agent_id() {
    let (db, mut rx) = setup_db_with_channel().await;

    // Register the second agent so its create_task FK check passes.
    db.register_agent("other-agent", "other-agent", "/tmp/other-home")
        .await
        .unwrap();

    let cloned = db.with_agent("other-agent");
    let mut task = make_test_task("agent-b-task");
    task.agent_id = "other-agent".to_string();
    let id = cloned.create_task(task).await.unwrap();

    match recv_next(&mut rx).await {
        TaskEventFrame::TaskCreated {
            task_id, agent_id, ..
        } => {
            assert_eq!(task_id, id);
            assert_eq!(
                agent_id, "other-agent",
                "frame must carry the write's actual agent, not the parent handle's"
            );
        }
        other => panic!("expected TaskCreated, got {other:?}"),
    }

    // Claim uses the wrapper's self.agent_id — the cloned handle carries
    // "other-agent", so the TaskClaimed frame agent_id must also match.
    assert!(cloned.claim_and_fire_task(&id).await.unwrap());
    match recv_next(&mut rx).await {
        TaskEventFrame::TaskClaimed {
            task_id, agent_id, ..
        } => {
            assert_eq!(task_id, id);
            assert_eq!(agent_id, "other-agent");
        }
        other => panic!("expected TaskClaimed, got {other:?}"),
    }
}

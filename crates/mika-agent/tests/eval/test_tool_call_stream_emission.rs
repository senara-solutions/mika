//! Integration tests: A2A tool-call stream frame emission (mika#1757).
//!
//! These tests exercise the end-to-end thread — `AgentParams.stream_ctx` →
//! `run_agent` → `run_loop` → `process_tool_calls` → broadcast subscriber —
//! by driving `EvalHarness::run` with a `MockLlmProvider` and asserting the
//! exact `StreamEvent::ToolCallStart` / `ToolCallResult` frames a subscriber
//! observes on the wire.
//!
//! Companion to the emit-helper unit tests in
//! `crates/mika-a2a/src/streaming.rs::tests`.
//!
//! **Structural invariant for `stream_ctx = None`.** The plan's R6 ("without
//! ctx, no frames") is enforced at compile time by the emit-site guards
//! `if let Some(sc) = stream_ctx { sc.emit_start(...) }` in
//! `crates/mika-agent/src/tool_execution/dispatch.rs`. There is no code path
//! that can produce a frame when the pattern binds `None`. A dynamic test
//! for this would need to attach a *harness-internal* probe on the emit
//! sites (the harness has no such hook); attaching an orphaned
//! `broadcast::Receiver` that is never wired to any producer is tautological
//! — the assertion holds regardless of engine behaviour, and would still
//! hold after a regression. The invariant is therefore covered by the
//! `if let Some(sc)` code shape plus the positive assertions below (which
//! would fail if the guard was inverted).

use std::sync::Arc;
use std::time::Duration;

use mika_a2a::streaming::{StreamEvent, ToolCallStreamContext};
use mika_common::llm::mock::*;
use serde_json::json;
use tokio::sync::broadcast;

use super::harness::EvalHarness;

/// Build a `ToolCallStreamContext` wired to a broadcast channel and return the
/// context (owned Arc) alongside a subscriber. Capacity is generous enough
/// that all frames from these small-scale tests stay resident until drain.
fn make_stream_ctx() -> (Arc<ToolCallStreamContext>, broadcast::Receiver<StreamEvent>) {
    let (tx, rx) = broadcast::channel::<StreamEvent>(64);
    let ctx = Arc::new(ToolCallStreamContext::new(
        Arc::new(tx),
        "task-under-test".to_string(),
        Some("ctx-under-test".to_string()),
    ));
    (ctx, rx)
}

/// Drain a subscriber until it's empty (or timeout expires), returning every
/// tool-call frame observed. Used by tests that assert on the exact sequence
/// and count.
async fn drain_tool_frames(
    rx: &mut broadcast::Receiver<StreamEvent>,
    max: usize,
) -> Vec<StreamEvent> {
    let mut out = Vec::new();
    for _ in 0..max {
        match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Ok(ev @ StreamEvent::ToolCallStart(_))) => out.push(ev),
            Ok(Ok(ev @ StreamEvent::ToolCallResult(_))) => out.push(ev),
            Ok(Ok(_)) => { /* non-tool-call frame — ignore */ }
            Ok(Err(_)) => break,
            Err(_) => break,
        }
    }
    out
}

// ────────────────────────────────────────────────────────────────────────────
// AC1 + AC4 — Physical dispatch emits one Start + one Result per call
// ────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn tool_call_emits_start_then_result_in_order() {
    let (ctx, mut rx) = make_stream_ctx();
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response(
                "store_fact",
                json!({
                    "category": "preference",
                    "key": "test_key",
                    "value": "hello store fact"
                }),
            ),
            text_response("Stored."),
        ])
        .stream_ctx(ctx)
        .build()
        .await
        .unwrap();

    let _ = harness.run("Please remember something").await.unwrap();

    let frames = drain_tool_frames(&mut rx, 8).await;
    assert_eq!(
        frames.len(),
        2,
        "expected one Start + one Result frame, got {}: {frames:?}",
        frames.len()
    );

    let start_step = match &frames[0] {
        StreamEvent::ToolCallStart(e) => {
            assert_eq!(e.task_id, "task-under-test");
            assert_eq!(e.context_id.as_deref(), Some("ctx-under-test"));
            assert_eq!(e.tool_name, "store_fact");
            assert!(e.args_summary.contains("hello store fact"));
            assert!(!e.timestamp.is_empty());
            e.step
        }
        other => panic!("frame[0] expected ToolCallStart, got {other:?}"),
    };
    match &frames[1] {
        StreamEvent::ToolCallResult(e) => {
            assert_eq!(e.task_id, "task-under-test");
            assert_eq!(e.context_id.as_deref(), Some("ctx-under-test"));
            assert_eq!(e.tool_name, "store_fact");
            // Success-path assertions: store_fact returns Ok, no exit-code
            // prefix, so the Result frame must carry success=true and
            // non_zero_exit=false. Failing to bind these fields tightly is
            // exactly the P1 gap the reviewer flagged.
            assert!(e.success, "success-path Result must have success=true");
            assert!(
                !e.non_zero_exit,
                "success-path Result must have non_zero_exit=false"
            );
            assert!(!e.output_summary.is_empty(), "output_summary populated");
            assert_eq!(
                e.step, start_step,
                "Result step must match its paired Start"
            );
        }
        other => panic!("frame[1] expected ToolCallResult, got {other:?}"),
    }
}

// ────────────────────────────────────────────────────────────────────────────
// AC3 — Per-turn dedup replay path stays silent
// ────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn dedup_replay_emits_only_one_start_result_pair() {
    // Two identical tool_use blocks in one LLM response — the mika#582 dedup
    // guard runs execute_tool once and reuses the cached ToolOutput for the
    // second block. Per AC3, the wire must reflect the physical dispatch
    // shape, not the LLM-emitted-block count.
    let (ctx, mut rx) = make_stream_ctx();
    let harness = EvalHarness::builder()
        .responses(vec![
            multi_tool_response(vec![
                ("search_memory", json!({"query": "sprint status"})),
                ("search_memory", json!({"query": "sprint status"})),
            ]),
            text_response("Done."),
        ])
        .stream_ctx(ctx)
        .build()
        .await
        .unwrap();

    let _ = harness.run("check sprint").await.unwrap();

    let frames = drain_tool_frames(&mut rx, 8).await;
    assert_eq!(
        frames.len(),
        2,
        "dedup replay must yield ONE Start + ONE Result (not two), got {} frames: {frames:?}",
        frames.len()
    );
    assert!(matches!(&frames[0], StreamEvent::ToolCallStart(_)));
    assert!(matches!(&frames[1], StreamEvent::ToolCallResult(_)));
}

// ────────────────────────────────────────────────────────────────────────────
// AC2 (fire-and-forget) — Zero-subscriber emit never fails the tool call
// ────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn zero_subscribers_does_not_break_agent_loop() {
    // Attach a ctx whose sender has zero live subscribers; the send() call
    // returns Err, but process_tool_calls swallows it and the agent loop
    // completes normally.
    let (tx, rx) = broadcast::channel::<StreamEvent>(4);
    drop(rx); // ← zero subscribers now
    let ctx = Arc::new(ToolCallStreamContext::new(
        Arc::new(tx),
        "orphan-task".to_string(),
        None,
    ));

    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("search_memory", json!({"query": "anything"})),
            text_response("Nothing found."),
        ])
        .stream_ctx(ctx)
        .build()
        .await
        .unwrap();

    // The run must complete without panicking. Broadcast errors are the
    // fire-and-forget invariant per AgentParams.stream_ctx docstring.
    let trace = harness.run("orphan run").await.unwrap();
    assert!(!trace.calls_for_tool("search_memory").is_empty());
}

// ────────────────────────────────────────────────────────────────────────────
// R4 — Suppressed send_message boundary path stays silent
// ────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn suppressed_send_message_boundary_emits_no_frames() {
    // Two `send_message` calls in one LLM response (conversation mode). The
    // mika#771 intra-step boundary gate lets the first through and
    // suppresses the second via `continue` before it reaches execute_tool —
    // so the wire must show exactly ONE Start + ONE Result pair (from the
    // physical dispatch of the first), not two. A regression that moved
    // the emit sites above the suppression gate would double the frame
    // count and fail here.
    let (ctx, mut rx) = make_stream_ctx();
    let harness = EvalHarness::builder()
        .responses(vec![
            multi_tool_response(vec![
                ("send_message", json!({"text": "First message"})),
                (
                    "send_message",
                    json!({"text": "Duplicate — should be suppressed"}),
                ),
            ]),
            text_response("Done."),
        ])
        .stream_ctx(ctx)
        .build()
        .await
        .unwrap();

    let _ = harness.run("test suppression").await.unwrap();

    let frames = drain_tool_frames(&mut rx, 8).await;
    assert_eq!(
        frames.len(),
        2,
        "suppressed send_message must not emit — expected ONE pair from the first \
         physical dispatch, got {} frames: {frames:?}",
        frames.len()
    );
    assert!(matches!(&frames[0], StreamEvent::ToolCallStart(_)));
    assert!(matches!(&frames[1], StreamEvent::ToolCallResult(_)));
}

// ────────────────────────────────────────────────────────────────────────────
// R2 — Failure-path emits Result with success=false
// ────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn error_path_emits_result_with_success_false() {
    // Call a tool that does not exist. execute_tool's fallthrough branch
    // returns ToolOutput::error("Unknown tool: X"), which sets is_error=true
    // and therefore tool_succeeded=false on the wire frame. This test binds
    // the success flag on the failure branch — the sibling to the success
    // assertion in `tool_call_emits_start_then_result_in_order`. A
    // regression that hard-coded `success: true` on the emit call would
    // fail here even though the success-path test kept passing.
    let (ctx, mut rx) = make_stream_ctx();
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("nonexistent_tool_zzz", json!({})),
            text_response("Failed."),
        ])
        .stream_ctx(ctx)
        .build()
        .await
        .unwrap();

    let _ = harness.run("please error").await.unwrap();

    let frames = drain_tool_frames(&mut rx, 8).await;
    assert_eq!(
        frames.len(),
        2,
        "failing dispatch still emits one Start + one Result pair, got {} frames: {frames:?}",
        frames.len()
    );
    assert!(matches!(&frames[0], StreamEvent::ToolCallStart(_)));
    match &frames[1] {
        StreamEvent::ToolCallResult(e) => {
            assert_eq!(e.tool_name, "nonexistent_tool_zzz");
            assert!(
                !e.success,
                "failing dispatch must emit Result with success=false, got success=true"
            );
            // Unknown-tool errors are is_error=true, not non_zero_exit — the
            // non_zero_exit flag is reserved for `Exit code:`-prefixed
            // outputs from exec-handler skills, which are hard to exercise
            // without a skill fixture. Assert the discriminator holds.
            assert!(
                !e.non_zero_exit,
                "non_zero_exit is false for is_error=true dispatches"
            );
        }
        other => panic!("frame[1] expected ToolCallResult, got {other:?}"),
    }
}

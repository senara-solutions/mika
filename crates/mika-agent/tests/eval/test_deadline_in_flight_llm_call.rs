//! mika#848 — agent total timeout deadline-exceeded path persists `llm_calls`.
//!
//! Before the fix, an outer `tokio::time::timeout` wrapped `run_agent_inner`. When
//! the 5-minute deadline fired while a `reqwest` HTTP request was in-flight, the
//! inner future was dropped, the LLM response was lost, and no `llm_calls` row
//! was ever written — the operator saw the canned `"that took too long"` fallback
//! with no diagnostic trail.
//!
//! The fix replaces the outer timeout with an `Instant`-based deadline checked
//! between steps in `run_loop`. In-flight LLM calls are allowed to complete (or
//! transport-timeout via the provider's own 120s `reqwest` timeout) before the
//! loop exits gracefully via `LoopResult::DeadlineExceeded`. The contract this
//! test enforces: **at least one `llm_calls` row is persisted even when the
//! deadline fires during an LLM call.**

use std::time::Duration;

use mika_common::llm::mock::*;
use serde_json::json;
use tokio::time::Instant;

use super::harness::EvalHarness;

/// The LLM call is in-flight when the deadline fires. The provider's response
/// (or transport-timeout) governs cancellation, so the `llm_calls` row is
/// persisted before `run_loop` returns `DeadlineExceeded` on the NEXT iteration.
///
/// Uses a tool-call response so the loop continues to a second iteration where
/// the deadline check fires. A pure EndTurn response would exit the loop via
/// `LoopResult::Done` before the deadline check ever ran — that's the correct
/// behavior, but it's not the path we're trying to test here.
///
/// Uses `tokio::time::pause()` to drive the clock without wall-clock waits.
/// `MockResponse::Delayed` sleeps virtually for 6 minutes; the deadline is set
/// to 1s; the runtime advances time until the mock resolves.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn deadline_during_llm_call_persists_llm_calls_row() {
    // Step 0: a tool_call response delayed by 6 minutes virtual. When it
    // resolves, the `llm_calls` row is persisted, the (no-op) tool runs, and
    // the loop iterates to step 1.
    // Step 1: deadline check fires (now far past the 1s deadline), loop exits
    // via DeadlineExceeded. No second LLM call is made.
    let responses = vec![
        delayed_response(
            360_000, // 6 minutes virtual
            tool_call_response("search_memory", json!({"query": "anything"})),
        ),
        // Sentinel: this should never be consumed because the deadline fires
        // before the next LLM call. If it IS consumed, the test would still
        // find a fallback because the second response would also crash into
        // the deadline gate before producing useful output.
        text_response("(should never be returned)"),
    ];

    let harness = EvalHarness::builder()
        .responses(responses)
        .build()
        .await
        .expect("harness build");

    // Set a deadline 1 second into the future. Virtual clock will advance past
    // it once the mock's sleep resolves.
    let deadline = Instant::now() + Duration::from_secs(1);

    let trace = harness
        .run_with_deadline("Trigger a slow tool-use LLM call.", deadline)
        .await
        .expect("agent run");

    // Contract A: the slow LLM call's row IS persisted, even though it crossed
    // the deadline. Pre-fix, this would be 0 (the in-flight reqwest got
    // cancelled at socket layer when the outer tokio::time::timeout fired).
    assert!(
        trace.llm_call_count >= 1,
        "expected at least one llm_calls row when deadline fires during an LLM call, got {}",
        trace.llm_call_count
    );

    // Contract B: the conversation-mode fallback message is emitted (the loop
    // exited via DeadlineExceeded, not Done). Same user-visible string as
    // pre-fix, but now it's reached via the graceful-exit path with full
    // observability rather than the silent-drop path.
    let text = trace
        .output
        .text
        .expect("agent output should have fallback text");
    assert!(
        text.contains("took too long"),
        "expected conversation fallback message, got: {text}"
    );

    // Contract C: at most one LLM call was made (the one that crossed the
    // deadline). The deadline check at iteration top must have prevented a
    // second call. Pre-fix this could be 0 (in-flight call cancelled before
    // persist) or, with a buggy fix, > 1 (deadline didn't gate next iteration).
    assert_eq!(
        trace.llm_call_count, 1,
        "expected exactly 1 llm_calls row — the deadline check at iteration top must have prevented a second call"
    );
}

/// **Plan AC F3c contract:** an LLM call inside `attempt_continuation_turn`
/// must persist its `llm_calls` row in every outcome (success, API error,
/// deadline-clamped timeout). Pre-fix, the continuation path called
/// `llm.send_message` directly with no DB write — the silent-drop variant of
/// the in-flight-cancel bug at smaller scale.
///
/// We test the API-error arm to lock in the persistence contract without the
/// virtual-time complications of the deadline-clamp arm (which would require
/// driving 20+ tool dispatches under `tokio::time::pause()`, where each tool's
/// 30s `tokio::time::timeout` interacts non-trivially with auto-advance).
///
/// The key invariant: `attempt_continuation_turn` writes a row before
/// returning, regardless of which arm fires. The success/error arms exercise
/// the same `save_continuation_llm_call` helper as the timeout arm, so
/// covering one structurally proves the others.
#[tokio::test]
async fn continuation_llm_call_persists_row_on_api_error() {
    use mika_common::llm::LlmError;
    use mika_common::llm::mock::tool_call_response;

    let mut responses: Vec<MockResponse> = (0..20)
        .map(|i| tool_call_response("search_memory", json!({"query": format!("q{i}")})))
        .collect();
    // 21st response is the continuation turn — fails with an HTTP 500.
    // Pre-F3c, the continuation made the LLM call and discarded the error
    // without persisting anything. Post-F3c, the error arm calls
    // `save_continuation_llm_call` with status='error'.
    responses.push(MockResponse::Error(LlmError::HttpError {
        status: 500,
        message: "Internal server error during continuation".into(),
        retryable: false,
    }));

    let harness = EvalHarness::builder()
        .responses(responses)
        .build()
        .await
        .expect("harness build");

    let trace = harness
        .run("Drive max-steps then continuation.")
        .await
        .expect("agent run");

    // 20 in-loop calls + 1 continuation error = 21 rows. Pre-F3c this would
    // have been 20 — the continuation error was never persisted.
    assert_eq!(
        trace.llm_call_count, 21,
        "F3c: continuation LLM call must persist its row even on API error (got {})",
        trace.llm_call_count
    );

    // The 21st row should be status='error' with a non-empty error message
    // pointing at the continuation, not at the in-loop steps.
    let last_call = trace.llm_calls.last().expect("expected continuation row");
    assert_eq!(
        last_call.status, "error",
        "continuation row should be status='error' on API failure; got '{}'",
        last_call.status
    );
    assert!(
        last_call
            .error_message
            .as_ref()
            .is_some_and(|e| e.contains("Internal server error")),
        "continuation error_message should propagate the API error; got {:?}",
        last_call.error_message
    );

    // Output should be the structured fallback (not a panic, not silence).
    let text = trace.output.text.expect("output text");
    assert!(
        text.contains("search_memory"),
        "fallback should mention attempted tools; got: {text}"
    );
}

/// Sanity: when the deadline is generous, the same setup runs to completion
/// (the slow response resolves, the agent loop exits with `Done`, no fallback
/// is emitted). Confirms that the deadline-exceeded path is what's being
/// exercised in the assertion above, not a separate failure mode.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn slow_llm_call_completes_when_deadline_is_generous() {
    let responses = vec![delayed_response(
        10_000, // 10 seconds virtual — well within deadline
        text_response("Response arrived in time."),
    )];

    let harness = EvalHarness::builder()
        .responses(responses)
        .build()
        .await
        .expect("harness build");

    // Generous 5-minute deadline.
    let deadline = Instant::now() + Duration::from_secs(300);

    let trace = harness
        .run_with_deadline("Trigger a slow but in-budget LLM call.", deadline)
        .await
        .expect("agent run");

    assert_eq!(
        trace.llm_call_count, 1,
        "expected exactly one llm_calls row when call completes within deadline"
    );
    let text = trace.output.text.expect("output text");
    assert!(
        text.contains("Response arrived"),
        "expected the actual response text, got: {text}"
    );
    assert!(
        !text.contains("took too long"),
        "should NOT see fallback text when deadline is generous"
    );
}

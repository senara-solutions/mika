//! End-to-end wiring of the `[context.history]` window bounds (mika#2295).
//!
//! The unit tests around this one prove that the SQL restricts, that the ceiling
//! elides oldest-first, and that mika-arch's identity carries both fields. **None
//! of them would fail if the agent loop read the wrong section of the identity,
//! or read it and did nothing with it** — the whole chain would stay green while
//! the window stayed unbounded. That is the shape mika#2205 measured on a
//! different guard: the predicate was right and the caller never called it. These
//! tests run the real loop and read the window the model actually received.
//!
//! Sibling of `test_context_summary_inject.rs`, which does the same for the other
//! `[context.*]` block, and deliberately written in its shape.

use mika_common::llm::mock::*;

use super::harness::EvalHarness;
use super::trace::AgentTrace;

/// Concatenated content of the window sent on the first LLM call.
fn window_text(trace: &AgentTrace) -> String {
    assert!(
        !trace.captured_requests.is_empty(),
        "the turn must have reached the provider"
    );
    trace.captured_requests[0]
        .messages
        .iter()
        .map(|m| format!("{:?}", m.content))
        .collect::<Vec<_>>()
        .join("\n")
}

/// AC3/AC7 — under `scope = "session"`, another ticket's plan is not in the window.
///
/// This is the contamination half of mika#2295, and the reason the ticket is more
/// than a cost problem: an architect reviewing ticket A was reading the plans of
/// B, C and D, because the window filtered on `m.agent_id` and never on
/// `m.session_id`.
#[tokio::test]
async fn mika2295_session_scope_keeps_another_tickets_plan_out_of_the_window() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Verdict noted.")])
        .build()
        .await
        .unwrap();

    std::fs::write(
        harness.home_dir.path().join("identity.toml"),
        r#"
name = "Architect"

[context.history]
scope = "session"
"#,
    )
    .unwrap();

    // A plan reviewed under another ticket, in its own session.
    harness
        .db
        .create_session("other-ticket-session", "mika", "test")
        .await
        .unwrap();
    harness
        .db
        .save_message(
            "other-ticket-session",
            "user",
            "PLAN FOR TICKET B: rewrite the dispatcher",
            None,
        )
        .await
        .unwrap();

    let trace = harness.run("Review the plan for ticket A.").await.unwrap();
    let window = window_text(&trace);

    assert!(
        !window.contains("TICKET B"),
        "another ticket's plan must not reach an architect reviewing this one:\n{window}"
    );
    assert!(
        window.contains("ticket A"),
        "the question itself must survive"
    );
}

/// AC2 — without the block, the window still crosses sessions.
///
/// The negative control for the test above. Without it, a `scope` field that was
/// never read would pass that test for the wrong reason on an empty database.
#[tokio::test]
async fn mika2295_without_the_block_the_window_still_crosses_sessions() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Noted.")])
        .build()
        .await
        .unwrap();

    // The harness writes a `name`/`emoji`-only identity.toml (mika#2027 makes an
    // absent file fail-closed), so no `[context.history]` block: the default.

    harness
        .db
        .create_session("other-ticket-session", "mika", "test")
        .await
        .unwrap();
    harness
        .db
        .save_message(
            "other-ticket-session",
            "user",
            "PLAN FOR TICKET B: rewrite the dispatcher",
            None,
        )
        .await
        .unwrap();

    let trace = harness.run("Review the plan for ticket A.").await.unwrap();

    assert!(
        window_text(&trace).contains("TICKET B"),
        "the pre-mika#2295 window is agent-wide, and stays so unless an identity says otherwise"
    );
}

/// AC4 — the ceiling fires on a session that iterates, and says so.
///
/// Session scope bounds contamination *between* tickets; it does not bound one
/// session that grooms in a loop (plan v1, review, plan v2 …). That is why both
/// fields exist and neither makes the other redundant.
#[tokio::test]
async fn mika2295_ceiling_elides_an_iterating_session_and_leaves_a_marker() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Verdict noted.")])
        .build()
        .await
        .unwrap();

    std::fs::write(
        harness.home_dir.path().join("identity.toml"),
        r#"
name = "Architect"

[context.history]
scope = "session"
max_tokens = 100
"#,
    )
    .unwrap();

    // Four iterations in the SAME session, ~2 KB each — far past 100 tokens
    // (= 400 bytes).
    for i in 1..=4 {
        harness
            .db
            .save_message(
                &harness.session_id,
                "user",
                &format!("PLAN REVISION {i}: {}", "detail ".repeat(280)),
                None,
            )
            .await
            .unwrap();
    }

    let trace = harness.run("Second pass, please.").await.unwrap();
    let window = window_text(&trace);

    assert!(
        window.contains("elided to fit the context-window budget"),
        "the model must be told history is missing, not left to assume it saw everything:\n\
         (window was {} chars)",
        window.len()
    );
    assert!(
        !window.contains("PLAN REVISION 1"),
        "the oldest revision is the first to go"
    );
    assert!(
        window.contains("Second pass, please."),
        "the turn's own question is never elided"
    );
}

/// AC4 — no ceiling configured, nothing elided, no marker.
#[tokio::test]
async fn mika2295_without_a_ceiling_nothing_is_elided() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Noted.")])
        .build()
        .await
        .unwrap();

    std::fs::write(
        harness.home_dir.path().join("identity.toml"),
        r#"
name = "Architect"

[context.history]
scope = "session"
"#,
    )
    .unwrap();

    harness
        .db
        .save_message(
            &harness.session_id,
            "user",
            &format!("PLAN REVISION 1: {}", "detail ".repeat(280)),
            None,
        )
        .await
        .unwrap();

    let trace = harness.run("Second pass, please.").await.unwrap();
    let window = window_text(&trace);

    assert!(!window.contains("elided to fit the context-window budget"));
    assert!(window.contains("PLAN REVISION 1"));
}

//! mika#2305 — the scope in force is readable in production, and it is the one
//! the identity declared.
//!
//! # What this covers that its neighbours do not
//!
//! `test_context_window_budget_2295.rs` already runs the real loop and asserts
//! the **window** the model received under each scope. The unit tests in
//! `agent_loop::tests` already assert that `build_context_window_fields` renders
//! the scope it is handed. Neither would fail if the emission site passed a
//! constant instead of `history_config.scope` — the window would still be
//! filtered, the label would still read `"session"`, and the instrument would be
//! saying something it had not measured.
//!
//! That is the mika#2205 shape, one notch further in: there, a correct predicate
//! was never called; here, a correct renderer would be called with the wrong
//! argument. Only a test that reads the **emitted event** after a real turn
//! closes it.
//!
//! # Why the two fields are asserted together
//!
//! `distinct_sessions` alone cannot be read — `> 1` means either the scope fell
//! back to `agent` (the mika#2305 defect) or one session iterated legitimately.
//! The pair is the deliverable, so the pair is what is asserted, including in the
//! negative control.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mika_common::llm::mock::*;

use super::harness::EvalHarness;

// -- tracing capture (same shape as `test_llm_watchdog_2342.rs`) --

type Captured = Arc<Mutex<Vec<(String, HashMap<String, String>)>>>;

struct CapturingLayer {
    events: Captured,
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CapturingLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = HashMap::new();
        let mut visitor = FieldVisitor(&mut fields);
        event.record(&mut visitor);
        let message = fields.remove("message").unwrap_or_default();
        if let Ok(mut events) = self.events.lock() {
            events.push((message, fields));
        }
    }
}

struct FieldVisitor<'a>(&'a mut HashMap<String, String>);

impl tracing::field::Visit for FieldVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
}

/// A dispatcher registered for the life of the test process, so the scoped one
/// below is never the only one.
///
/// tracing-core caches a callsite's interest at its first hit. While exactly one
/// dispatcher is registered, that first computation consults the *hitting
/// thread's* default — so another `eval` test that first reaches
/// `emit_context_window_assembled` on its own thread, while this guard is the
/// lone registered dispatcher, caches `never` for the whole process and this test
/// captures zero events. With two registered, the computation iterates the
/// registry instead and sees the capturing subscriber.
static PINNED_DISPATCH: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();

fn capture() -> (tracing::subscriber::DefaultGuard, Captured) {
    use tracing_subscriber::layer::SubscriberExt;
    PINNED_DISPATCH.get_or_init(|| tracing::Dispatch::new(tracing_subscriber::registry()));
    let events: Captured = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CapturingLayer {
        events: Arc::clone(&events),
    });
    let guard = tracing::subscriber::set_default(subscriber);
    (guard, events)
}

/// The one `context_window_assembled` event of the turn.
///
/// Asserting there is exactly one is part of the contract: the operator command
/// in the plan filters by `agent_id` and reads the lines it gets, so a turn that
/// emitted the event twice — once per scope, say — would make that command
/// ambiguous without failing anything.
fn the_window_event(events: &Captured) -> HashMap<String, String> {
    let captured = events.lock().expect("capture mutex");
    let mut matching: Vec<_> = captured
        .iter()
        .filter(|(_, f)| f.get("event").map(String::as_str) == Some("context_window_assembled"))
        .map(|(_, f)| f.clone())
        .collect();

    assert_eq!(
        matching.len(),
        1,
        "expected exactly one context_window_assembled event for the turn, got {}",
        matching.len()
    );
    matching.pop().expect("checked non-empty")
}

/// Text of the window sent on the first LLM call — the same reader as
/// `test_context_window_budget_2295.rs`, so the two files are comparable.
fn window_text(trace: &super::trace::AgentTrace) -> String {
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

/// Seed one message belonging to another ticket, in its own session.
async fn seed_another_tickets_plan(harness: &EvalHarness) {
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
}

/// **T1 + T3** — under a declared `scope = "session"`, the event reports
/// `session`, the count is 1, and the other ticket is genuinely absent.
///
/// The three assertions are one statement: the instrument says what the window
/// did, and the window did it.
#[tokio::test]
async fn mika2305_a_session_scoped_turn_reports_session_and_one_distinct_session() {
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

    seed_another_tickets_plan(&harness).await;

    let (_guard, events) = capture();
    let trace = harness.run("Review the plan for ticket A.").await.unwrap();

    let fields = the_window_event(&events);
    assert_eq!(
        fields.get("history_scope").map(String::as_str),
        Some("session"),
        "the emission site must report the scope the identity declared, not a constant: {fields:?}"
    );
    assert_eq!(
        fields.get("distinct_sessions").map(String::as_str),
        Some("1"),
        "a session-scoped window draws from one session: {fields:?}"
    );

    let window = window_text(&trace);
    assert!(
        !window.contains("TICKET B"),
        "the reported scope must describe a window that really was filtered:\n{window}"
    );
}

/// **T2 + T3, negative control** — without the block, the event reports `agent`
/// and counts the sessions the window really crossed.
///
/// This is the load-bearing half. Without it the test above passes on a hard-coded
/// `"session"` label and on a `rebuild_context` that returns nothing — the two
/// ways a green suite can describe a broken system. It is also what makes the
/// `agent` row of the reading table in `emit_context_window_assembled` a measured
/// claim rather than a comment.
#[tokio::test]
async fn mika2305_without_the_block_the_event_reports_agent_and_counts_the_crossing() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Noted.")])
        .build()
        .await
        .unwrap();

    // The harness writes a `name`/`emoji`-only identity.toml (mika#2027 makes an
    // absent file fail-closed), so no `[context.history]` block: the default.

    seed_another_tickets_plan(&harness).await;

    let (_guard, events) = capture();
    let trace = harness.run("Review the plan for ticket A.").await.unwrap();

    let fields = the_window_event(&events);
    assert_eq!(
        fields.get("history_scope").map(String::as_str),
        Some("agent"),
        "the default scope must be reported as such — a label that always reads \
         \"session\" would hide exactly the mika#2305 defect: {fields:?}"
    );
    assert_eq!(
        fields.get("distinct_sessions").map(String::as_str),
        Some("2"),
        "the agent-wide window crossed two sessions and must say so: {fields:?}"
    );

    assert!(
        window_text(&trace).contains("TICKET B"),
        "the negative control must really cross sessions, otherwise the positive \
         test above proves nothing about the filter"
    );
}

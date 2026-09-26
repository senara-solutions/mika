//! mika#1951 — a caller may isolate one turn, and the isolation really happens.
//!
//! # What these tests cover that their neighbours do not
//!
//! `test_context_scope_observability_2305.rs` proves the **identity-declared**
//! scope reaches the window and the instrument. It says nothing about a
//! *per-call* lever, because there was none. `well_known_agents::tests` proves
//! mika-test's constant and that it lands on disk, but mika-test is one agent:
//! the 2026-08-22 battery could as easily have run on mika-dev, and an operator
//! wanting one isolated pass on a tenant still had nothing.
//!
//! # Why the negative control is load-bearing here too
//!
//! Without it, `mika1951_an_isolated_turn_reads_only_its_own_session` passes on
//! a `rebuild_context` that returns nothing and on a `history_scope` field that
//! is a constant — the two ways a green suite describes a broken system. The
//! pair is the deliverable, so the pair is asserted.
//!
//! # The two channels are tested apart, on purpose
//!
//! §1 of the plan: `load_conversation_summary` filters on `agent_id` alone, so
//! the compaction summary crosses sessions by construction and is a *second*
//! leak of the same shape. A test that only seeded history would pass on an
//! isolation that closed the window and left the summary open — and the
//! partiality would be invisible until a bench ran long enough to compact.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mika_common::llm::mock::*;

use super::harness::EvalHarness;

// -- tracing capture (same shape as `test_context_scope_observability_2305.rs`) --

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

/// See the identically-named helper in `test_context_scope_observability_2305.rs`
/// for why a second live dispatcher is required for the capture to see anything.
fn keep_registry_multi_dispatcher() {
    static KEEPER: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
    KEEPER.get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
}

fn capture() -> (tracing::subscriber::DefaultGuard, Captured) {
    use tracing_subscriber::layer::SubscriberExt;
    keep_registry_multi_dispatcher();
    let events: Captured = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CapturingLayer {
        events: Arc::clone(&events),
    });
    let guard = tracing::subscriber::set_default(subscriber);
    (guard, events)
}

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

/// Everything the model was shown on the first call — system prompt included.
///
/// Deliberately wider than its `test_context_scope_observability_2305.rs`
/// cousin, which reads `messages` only: the compaction summary is injected into
/// the **system prompt**, so a reader that skipped it would find the summary
/// channel untestable and quietly report it clean.
fn everything_the_model_saw(trace: &super::trace::AgentTrace) -> String {
    assert!(
        !trace.captured_requests.is_empty(),
        "the turn must have reached the provider"
    );
    let req = &trace.captured_requests[0];
    let mut out = req.system.clone().unwrap_or_default();
    for m in &req.messages {
        out.push('\n');
        out.push_str(&format!("{:?}", m.content));
    }
    out
}

/// Seed one message belonging to another bench call, in its own session.
async fn seed_another_bench_call(harness: &EvalHarness) {
    harness
        .db
        .create_session("other-bench-session", "mika", "test")
        .await
        .unwrap();
    harness
        .db
        .save_message(
            "other-bench-session",
            "assistant",
            "ANSWER FROM CALL ONE: six",
            None,
        )
        .await
        .unwrap();
}

/// **Positive control, window channel** — with the per-call lever, an agent
/// under the *conversational default* reads its own session only.
///
/// This is the lever the founding ticket lacked: `mika-test` is closed by its
/// identity (U1), but the battery could have run on any agent, and editing an
/// `identity.toml` plus restarting the daemon between measurements is not a
/// bench workflow.
#[tokio::test]
async fn mika1951_an_isolated_turn_reads_only_its_own_session() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Seven.")])
        .session_isolated(true)
        .build()
        .await
        .unwrap();

    // No `[context.history]` block: the agent is under `HistoryScope::Agent`,
    // the fleet default. The isolation must come from the caller alone.
    seed_another_bench_call(&harness).await;

    let (_guard, events) = capture();
    let trace = harness
        .run("How many sides does a hexagon have?")
        .await
        .unwrap();

    let fields = the_window_event(&events);
    assert_eq!(
        fields.get("history_scope").map(String::as_str),
        Some("session"),
        "the instrument must report the scope that DECIDED the window — for an \
         isolated turn that is the caller's, not the identity's: {fields:?}"
    );
    assert_eq!(
        fields.get("distinct_sessions").map(String::as_str),
        Some("1"),
        "an isolated window draws from one session: {fields:?}"
    );

    let seen = everything_the_model_saw(&trace);
    assert!(
        !seen.contains("ANSWER FROM CALL ONE"),
        "the reported scope must describe a window that really was filtered — \
         this is the *« Six. Answer unchanged. »* of 2026-08-22:\n{seen}"
    );
}

/// **Negative control** — the same agent, the same seeded history, without the
/// lever: the leak is reproduced.
///
/// AC1 of the ticket, as a deterministic test rather than a replay: without this
/// the test above proves nothing about the filter, only that the harness can
/// produce an empty window.
#[tokio::test]
async fn mika1951_without_the_lever_the_turn_still_crosses_sessions() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Six. Answer unchanged.")])
        .build()
        .await
        .unwrap();

    seed_another_bench_call(&harness).await;

    let (_guard, events) = capture();
    let trace = harness
        .run("How many sides does a hexagon have?")
        .await
        .unwrap();

    let fields = the_window_event(&events);
    assert_eq!(
        fields.get("history_scope").map(String::as_str),
        Some("agent"),
        "the default must still be reported as such: {fields:?}"
    );
    assert_eq!(
        fields.get("distinct_sessions").map(String::as_str),
        Some("2"),
        "the unisolated window crossed two sessions and must say so: {fields:?}"
    );

    assert!(
        everything_the_model_saw(&trace).contains("ANSWER FROM CALL ONE"),
        "the negative control must really leak, or the positive test proves nothing"
    );
}

/// **Positive control, summary channel** — the second leak of §1, tested alone.
///
/// History is left empty and only a compaction summary is seeded, so this test
/// fails if the isolation closes the window and nothing else. That is the trap
/// the plan names: an isolation that looks complete on the probe
/// (`context_window_assembled` would read a clean `"session"` / `1`) while a
/// longer bench still bleeds through `load_conversation_summary`, which filters
/// on `agent_id` alone.
#[tokio::test]
async fn mika1951_an_isolated_turn_injects_no_compaction_summary() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Seven.")])
        .session_isolated(true)
        .build()
        .await
        .unwrap();

    harness
        .db
        .replace_with_summary("SUMMARY OF EARLIER CALLS: the answer given was six", 0)
        .await
        .unwrap();

    let trace = harness
        .run("How many sides does a hexagon have?")
        .await
        .unwrap();

    let seen = everything_the_model_saw(&trace);
    assert!(
        !seen.contains("SUMMARY OF EARLIER CALLS"),
        "an isolated turn must close BOTH cross-session channels; the summary is \
         keyed on agent_id alone and crosses every session by construction:\n{seen}"
    );
}

/// **Negative control, summary channel** — without the lever the summary is
/// injected, so the test above is about the lever and not about an agent that
/// never had a summary.
#[tokio::test]
async fn mika1951_without_the_lever_the_compaction_summary_is_injected() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Six.")])
        .build()
        .await
        .unwrap();

    harness
        .db
        .replace_with_summary("SUMMARY OF EARLIER CALLS: the answer given was six", 0)
        .await
        .unwrap();

    let trace = harness
        .run("How many sides does a hexagon have?")
        .await
        .unwrap();

    assert!(
        everything_the_model_saw(&trace).contains("SUMMARY OF EARLIER CALLS"),
        "the default must still inject the summary — otherwise the positive test \
         above is green for a reason unrelated to the isolation"
    );
}

/// **Non-widening** — `session_isolated` cannot restore an agent-wide window.
///
/// The property is asserted, not assumed. It is the reason the wire key is a
/// bool rather than a scope name: `/a2a/{agent}` is reachable by any
/// authenticated caller, and a key carrying `"agent"` would let one widen
/// mika-arch's window from the network and make it read other tickets' plans —
/// mika#2295 and mika#2305 reopened through the door.
///
/// `false` is what a caller that declared nothing produces, so this also pins
/// that the absent-key path leaves a session-scoped agent alone.
#[tokio::test]
async fn mika1951_the_lever_can_never_widen_a_session_scoped_agent() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Verdict noted.")])
        .session_isolated(false)
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

    seed_another_bench_call(&harness).await;

    let (_guard, events) = capture();
    let trace = harness.run("Review the plan for ticket A.").await.unwrap();

    let fields = the_window_event(&events);
    assert_eq!(
        fields.get("history_scope").map(String::as_str),
        Some("session"),
        "a caller declaring nothing must not widen a session-scoped agent: {fields:?}"
    );
    assert!(
        !everything_the_model_saw(&trace).contains("ANSWER FROM CALL ONE"),
        "the declared scope must still filter when the caller declared nothing"
    );
}

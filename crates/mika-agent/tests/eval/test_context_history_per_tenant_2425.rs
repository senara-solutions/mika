//! mika#2425 — the per-tenant narrowing reaches the window, and the instrument
//! reports what really happened.
//!
//! # What this covers that its neighbours do not
//!
//! `test_context_scope_observability_2305.rs` proves the **identity-declared**
//! scope reaches the emitted event; `test_context_window_budget_2295.rs` proves
//! the window itself is filtered. Neither says anything about the
//! `customer_config` half, which is the whole of mika#2425 — and neither would
//! fail if `agent_loop` kept reading `ctx.identity.context.history` and dropped
//! the cascade on the floor. That is precisely the shape the structural guard
//! (`mika2425_identity_context_history_has_a_single_reader`) cannot see either:
//! a site that reads the *right* field and simply never consults the database
//! violates no source predicate.
//!
//! The unit tests in `agent_loop::context_history` assert the truth table on a
//! pure function. They would stay green if nothing ever called it. Only a test
//! that runs the real loop, with a real row in `customer_config`, and reads the
//! **emitted** `context_window_assembled` closes that gap.
//!
//! # Why the pair is asserted, and why there are three tests
//!
//! `distinct_sessions` alone has two causes of opposite sign (mika#2305), so the
//! pair `(history_scope, distinct_sessions)` is the deliverable and the pair is
//! what is asserted. The negative control is not optional decoration: without
//! it, a `resolve` that returned `Session` unconditionally would pass the
//! positive test, and so would a `rebuild_context` that returned nothing. The
//! third test is the one AC3 rests on — with **no** row posed, the turn is
//! byte-for-byte the pre-mika#2425 turn.

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
/// for why a second live dispatcher is required for this capture to be reliable.
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

/// The one `context_window_assembled` event of the turn.
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

/// Text of the window sent on the **last** LLM call of the trace.
///
/// Deliberately `.last()` where the mika#2305 and mika#2295 files read `[0]`:
/// `MockLlmProvider::captured_requests()` accumulates for the life of the mock,
/// so in a test that runs two turns against one harness `[0]` is the *first*
/// turn's window. Reading it there does not fail loudly — it silently asserts
/// the wrong turn, which is how a two-turn test reports that a setting change
/// had no effect when it had one, or the reverse. Single-turn tests capture one
/// request, so the two readers agree there.
fn window_text(trace: &super::trace::AgentTrace) -> String {
    let request = trace
        .captured_requests
        .last()
        .expect("the turn must have reached the provider");
    request
        .messages
        .iter()
        .map(|m| format!("{:?}", m.content))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Seed one message belonging to another ticket, in its own session.
///
/// The agent is read off the harness rather than written as `"mika"`: a test
/// that isolates itself on its own agent (see `EvalHarnessBuilder::agent_id`)
/// would otherwise seed the neighbouring session under a *different* agent, and
/// the agent-scoped window would legitimately never contain it — a negative
/// control that passes for the wrong reason.
async fn seed_another_tickets_plan(harness: &EvalHarness) {
    harness
        .db
        .create_session("other-ticket-session", &harness.db.agent_id, "test")
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

/// **V5** — a `customer_config` narrowing reaches the window, and the event says so.
///
/// The identity declares nothing: the harness writes a `name`/`emoji`-only
/// `identity.toml` (mika#2027 makes an absent file fail-closed), so the declared
/// scope is the default `agent`. Everything asserted here therefore comes from
/// the database half, which is exactly the population mika#2425 adds.
#[tokio::test]
async fn mika2425_a_customer_config_narrowing_reaches_the_window() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Noted.")])
        .build()
        .await
        .unwrap();

    seed_another_tickets_plan(&harness).await;
    harness
        .db
        .set_customer_config(
            mika_agent::config_keys::CONTEXT_HISTORY_SCOPE_KEY,
            "session",
        )
        .await
        .unwrap();

    let (_guard, events) = capture();
    let trace = harness.run("Review the plan for ticket A.").await.unwrap();

    let fields = the_window_event(&events);
    assert_eq!(
        fields.get("history_scope").map(String::as_str),
        Some("session"),
        "the emission site must report the RESOLVED scope — reporting the declared \
         one would make the instrument say `agent` about a window that really was \
         filtered, which is the mika#2305 defect with the sign flipped: {fields:?}"
    );
    assert_eq!(
        fields.get("distinct_sessions").map(String::as_str),
        Some("1"),
        "a session-scoped window draws from one session: {fields:?}"
    );

    let window = window_text(&trace);
    assert!(
        !window.contains("TICKET B"),
        "the reported scope must describe a window that really was filtered — \
         otherwise the cascade is a label and not a mechanism:\n{window}"
    );
}

/// **V5, negative control** — without the row, the same identity crosses sessions.
///
/// This is the load-bearing half. Without it the test above passes on a
/// `resolve` that returns `Session` unconditionally, on a decision site that
/// hard-codes the label, and on a `rebuild_context` that returns nothing — three
/// ways a green suite describes a broken system.
#[tokio::test]
async fn mika2425_without_the_row_the_window_still_crosses_sessions() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Noted.")])
        .build()
        .await
        .unwrap();

    seed_another_tickets_plan(&harness).await;
    // No `set_customer_config` — this is the only difference with the test above.

    let (_guard, events) = capture();
    let trace = harness.run("Review the plan for ticket A.").await.unwrap();

    let fields = the_window_event(&events);
    assert_eq!(
        fields.get("history_scope").map(String::as_str),
        Some("agent"),
        "with no row posed, the default must be reported as such: {fields:?}"
    );
    assert_eq!(
        fields.get("distinct_sessions").map(String::as_str),
        Some("2"),
        "the agent-wide window crossed two sessions and must say so: {fields:?}"
    );
    assert!(
        window_text(&trace).contains("TICKET B"),
        "the negative control must really cross sessions, otherwise the positive \
         test above proves nothing about the cascade"
    );
}

/// **AC4 / V6 on the production path** — the database cannot widen a declared
/// `session`, and the refusal is said.
///
/// The unit test asserts the truth table; this one asserts that the *turn* obeys
/// it. A cascade that resolved correctly in isolation and were wired backwards
/// at the decision site would leave that unit test green while reopening
/// mika#2295 from a database row.
#[tokio::test]
async fn mika2425_the_database_cannot_widen_a_declared_session_scope() {
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
    harness
        .db
        .set_customer_config(mika_agent::config_keys::CONTEXT_HISTORY_SCOPE_KEY, "agent")
        .await
        .unwrap();

    let (_guard, events) = capture();
    let trace = harness.run("Review the plan for ticket A.").await.unwrap();

    let fields = the_window_event(&events);
    assert_eq!(
        fields.get("history_scope").map(String::as_str),
        Some("session"),
        "`agent` in the database is the neutral and never releases a declared \
         `session` — otherwise mika#2295 is reopenable from a DB row: {fields:?}"
    );
    assert!(
        !window_text(&trace).contains("TICKET B"),
        "the refusal must be real, not merely reported"
    );

    let captured = events.lock().expect("capture mutex");
    assert!(
        captured.iter().any(|(_, f)| {
            f.get("event").map(String::as_str) == Some("context_history_widening_refused")
        }),
        "a refused widening must be SAID: an operator who believes they widened \
         the window and did not has no other way to learn it. Captured events: {:?}",
        captured
            .iter()
            .filter_map(|(_, f)| f.get("event").cloned())
            .collect::<Vec<_>>()
    );
}

/// **AC1 + AC5** — posing the neutral cancels the narrowing, on the next turn,
/// with no restart; and the provenance event says the tenant door decided nothing.
///
/// Two things are asserted at once because they are one gesture. `delete_customer_config`
/// does not exist, so *posing `agent`* **is** the cancellation — and the whole
/// "narrow only" rule is what makes that safe. That it takes effect on the very
/// next turn is R2: `load_agent_context` re-reads the identity and this table on
/// every turn, which is what makes a per-tenant knob usable at all on a daemon
/// shared by a fleet.
///
/// **The two turns plus the dedicated agent are what make this test
/// order-independent**, and neither half is sufficient alone.
/// `context_history_resolved` is deduplicated on the resolved state in a
/// process-global map keyed by agent. Two turns guarantee a *state change* —
/// turn 1 puts `session` in the map, turn 2 asks for `agent` — but they
/// guarantee nothing while the key is shared: every other eval test in this
/// binary runs as agent `mika` and resolves the default `(agent, default)`,
/// which is precisely the state turn 2 resolves. A sibling landing between the
/// two turns writes that state first, turn 2 then matches it, the announcement
/// is deduplicated away and the assertion below fails on a system that is
/// working. That is not a hypothesis: it is how this test failed once main
/// added enough eval tests to make the window likely (mika#2425, 2026-09-24),
/// while passing in isolation every time.
///
/// The agent is therefore the fix, because the agent is the dedup key. Under
/// `tenant-2425-neutral` no sibling can write this map entry, so turn 2's
/// re-announcement is guaranteed by construction rather than by luck. The
/// captured event is filtered on the same agent for the same reason — the
/// capture is thread-local, but reading "the last `context_history_resolved`"
/// without checking whose it is would be the same class of accident one layer up.
#[tokio::test]
async fn mika2425_posing_the_neutral_cancels_the_narrowing_and_says_so() {
    let agent = "tenant-2425-neutral";
    let harness = EvalHarness::builder()
        .agent_id(agent)
        .responses(vec![text_response("Narrowed."), text_response("Widened.")])
        .build()
        .await
        .unwrap();

    seed_another_tickets_plan(&harness).await;
    let scope_key = mika_agent::config_keys::CONTEXT_HISTORY_SCOPE_KEY;

    // Turn 1 — the narrowing is in force.
    harness
        .db
        .set_customer_config(scope_key, "session")
        .await
        .unwrap();
    let narrowed = harness.run("Review the plan for ticket A.").await.unwrap();
    assert!(
        !window_text(&narrowed).contains("TICKET B"),
        "precondition: the narrowing must actually be in force before it is cancelled"
    );

    // Turn 2 — the neutral cancels it, same process, no restart.
    harness
        .db
        .set_customer_config(scope_key, "agent")
        .await
        .unwrap();
    let (_guard, events) = capture();
    let widened = harness.run("Review the plan for ticket A.").await.unwrap();

    assert!(
        window_text(&widened).contains("TICKET B"),
        "posing the neutral is the cancellation — `delete_customer_config` does \
         not exist, so if this does not restore the window nothing does"
    );

    let captured = events.lock().expect("capture mutex");
    let resolved = captured
        .iter()
        .rev()
        .find(|(_, f)| {
            f.get("event").map(String::as_str) == Some("context_history_resolved")
                && f.get("agent_id").map(String::as_str) == Some(agent)
        })
        .map(|(_, f)| f.clone())
        .expect("a state CHANGE must be re-announced — dedup silences repetition, not change");

    assert_eq!(resolved.get("scope").map(String::as_str), Some("agent"));
    assert_eq!(
        resolved.get("scope_source").map(String::as_str),
        Some("default"),
        "the neutral leaves the tenant door having decided nothing — crediting \
         `customer_config` for an untouched default would make the S1 probe \
         unreadable on the exact measurement it exists to make: {resolved:?}"
    );
    assert_eq!(
        resolved.get("max_tokens_source").map(String::as_str),
        Some("default"),
        "same on the ceiling axis, which was never posed at all: {resolved:?}"
    );
    assert_eq!(
        resolved.get("session_minting").map(String::as_str),
        Some("per_message"),
        "R3 must be readable off the line: this agent declares no `[session] \
         singleton`, so a \"session\" is a message for it: {resolved:?}"
    );
    assert!(
        !captured.iter().any(|(_, f)| {
            f.get("event").map(String::as_str) == Some("context_history_widening_refused")
        }),
        "`agent` against a declared `agent` widens nothing — it is the neutral, \
         and warning here would bury the signal under the state of the fleet"
    );
}

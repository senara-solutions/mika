//! mika#2342 — the `run_loop` LLM-call safety net (D1/V3, AC1/AC2).
//!
//! # The failure
//!
//! `run_loop`'s LLM call was made bare: no `tokio::time::timeout` around it,
//! and the agent envelope is tested at the **top of the iteration**, which a
//! future that never returns never reaches. So the provider's own `reqwest`
//! timeout was the only thing that could end the call — and when it did not
//! fire (n=2, mika-arch on kimi/OpenRouter), the call ran **27+ minutes**
//! leaving no `llm_call completed`, no `turn_usage` and no error. The task
//! stayed `in_progress` for ever.
//!
//! # What is asserted here
//!
//! Under virtual time, so the incident is covered without being reproduced:
//! the call is bounded (AC1), and the overrun produces an error and a trace
//! rather than a silence (AC2).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use mika_common::llm::mock::*;
use tokio::time::Instant;

use super::harness::EvalHarness;

// -- tracing capture (same shape as `skills::builtin_handlers::tests`) --

/// Captured `(message, fields)` pairs, shared between the layer and the test.
type Captured = Arc<Mutex<Vec<(String, std::collections::HashMap<String, String>)>>>;

struct CapturingLayer {
    events: Captured,
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CapturingLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = std::collections::HashMap::new();
        let mut visitor = FieldVisitor(&mut fields);
        event.record(&mut visitor);
        let message = fields.remove("message").unwrap_or_default();
        if let Ok(mut events) = self.events.lock() {
            events.push((message, fields));
        }
    }
}

struct FieldVisitor<'a>(&'a mut std::collections::HashMap<String, String>);

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

fn capture() -> (tracing::subscriber::DefaultGuard, Captured) {
    use tracing_subscriber::layer::SubscriberExt;
    let events: Captured = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CapturingLayer {
        events: Arc::clone(&events),
    });
    let guard = tracing::subscriber::set_default(subscriber);
    (guard, events)
}

/// The test geometry, spelled out because every bound below depends on it.
///
/// `MockLlmProvider` inherits the default `timeout_budget()` (120 s plafond /
/// 300 s envelope), so `worst_case_failure_secs = floor(300/120) × 120 = 240`
/// and the watchdog is `240 + LLM_WATCHDOG_MARGIN_SECS (60) = 300 s`.
const WATCHDOG_SECS: u64 = 300;

/// **AC1 + AC2 — positive control.** A call that outlives the rail's declared
/// worst case is cut, and says so.
///
/// The deadline is deliberately far away (1 h): what bounds this call must be
/// the watchdog, not the envelope. Without the fix the mock's 20-minute sleep
/// would simply be waited out — the iteration-top deadline check cannot
/// interrupt a call in flight, which is the whole defect.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn mika2342_a_call_outliving_the_rails_worst_case_is_cut_and_traced() {
    let responses = vec![
        delayed_response(
            20 * 60 * 1000, // 20 virtual minutes — the founding incident's order of magnitude
            text_response("(never arrives)"),
        ),
        // Sentinel: the turn must end on the watchdog error, not roll on to a
        // second call.
        text_response("(should never be returned)"),
    ];

    let harness = EvalHarness::builder()
        .responses(responses)
        .build()
        .await
        .expect("harness build");

    let (_guard, events) = capture();
    let deadline = Instant::now() + Duration::from_secs(3600);
    let result = harness
        .run_with_deadline("Trigger a call that never returns.", deadline)
        .await;

    // AC1: the turn ends. Pre-fix it did not — the task stayed `in_progress`.
    // (`AgentTrace` is not `Debug`, so the error is unwrapped by match rather
    // than by `expect_err`.)
    let rendered = match result {
        Ok(_) => panic!(
            "the watchdog must surface as an error: the point of mika#2342 is that the \
             overrun stops being a silence"
        ),
        Err(e) => format!("{e:#}"),
    };
    assert!(
        rendered.contains("mika#2342 watchdog"),
        "the error must name the net rather than mimic a reqwest failure — an operator \
         who reads a transport message must be able to keep believing reqwest said it. \
         Got: {rendered}"
    );

    // AC2, first half: the named WARN event, sole writer of this name.
    let fired: Vec<_> = events
        .lock()
        .unwrap()
        .iter()
        .filter(|(msg, _)| msg == "llm_call_watchdog_fired")
        .cloned()
        .collect();
    assert_eq!(
        fired.len(),
        1,
        "exactly one `llm_call_watchdog_fired` — its absence in nominal operation is \
         what makes it informative"
    );
    let (_, fields) = &fired[0];
    assert_eq!(
        fields.get("watchdog_secs").map(String::as_str),
        Some(WATCHDOG_SECS.to_string().as_str()),
        "the watchdog is the rail's declared worst case plus the margin, not the \
         remaining deadline (D2 — bounding on `remaining` would re-open mika#848)"
    );
    // The size of a request that never came back — the one measurement the hang
    // destroys (E5), which is why it rides on the event and not only on the row.
    let request_bytes: u64 = fields["request_bytes"]
        .parse()
        .expect("request_bytes is numeric");
    assert!(request_bytes > 0, "a real turn's request is not empty");

    // AC2, second half: the `llm_calls` row exists, in `error`, with the real
    // latency. It is written by the pre-existing `Err` branch — the watchdog
    // routes into it rather than duplicating the persistence path.
    let rows = harness
        .db
        .query_llm_calls_by_trace(&harness.trace_id)
        .await
        .expect("llm_calls readable");
    assert_eq!(
        rows.len(),
        1,
        "one call, one row — and the sentinel response must not have been consumed"
    );
    assert_eq!(rows[0].status, "error");
    assert!(
        rows[0]
            .error_message
            .as_ref()
            .is_some_and(|m| m.contains("mika#2342 watchdog")),
        "the row must carry the watchdog's own message, got {:?}",
        rows[0].error_message
    );
    // NOT asserted here, and the reason is worth stating rather than leaving as
    // a gap: `latency_ms` is measured with `std::time::Instant`, which tokio
    // does **not** virtualise. Under `start_paused` no wall-clock time passes,
    // so the row reads 0 ms however long the virtual call took. The value is
    // real in production — it comes from the same `llm_call_start` the success
    // path uses, untouched by mika#2342 — but this harness cannot witness it,
    // and an assertion that passed here by measuring nothing would be worse
    // than none.
}

/// The margin covers a **full legitimate retry chain**, backoffs included
/// (verification contract item 5 — AC4's arithmetic half).
///
/// Without this, D3's margin is an assertion rather than a property: the net is
/// sized on `max_attempts × plafond`, which deliberately excludes the retry
/// loop's own `500 ms × 2^(n-1)` sleeps. Over the hard cap of four attempts
/// those sum to 0.5 + 1 + 2 = 3.5 s — so the margin has to exceed that, on
/// every geometry a provider can legally be built with, or the net would cut a
/// chain that was still running normally.
#[test]
fn mika2342_the_margin_covers_a_full_retry_chain_with_its_backoffs() {
    use mika_common::llm::budget::LlmTimeoutBudget;
    use mika_common::llm::{DEFAULT_ATTEMPTS_HARD_CAP, MIN_HTTP_TIMEOUT_SECS};

    // The backoff series the retry loop sleeps between attempts, rounded up to
    // whole seconds: 0.5 + 1 + 2 = 3.5 s over four attempts.
    let worst_backoff_secs = 4u64;

    // `LLM_WATCHDOG_MARGIN_SECS` is private to `agent_loop`; the value is
    // pinned here from the outside, which is also what makes a silent change to
    // it visible to this contract.
    let margin_secs = 60u64;

    for (plafond, envelope) in [
        (MIN_HTTP_TIMEOUT_SECS, 300u64), // the smallest plafond an operator may set
        (120, 300),                      // the fleet default
        (240, 900),                      // mika-arch since mika#2189
        (420, 600),                      // the geometry of the founding incident
    ] {
        let budget = LlmTimeoutBudget::new(plafond, envelope).expect("valid geometry");
        let worst_case = budget.worst_case_failure_secs(DEFAULT_ATTEMPTS_HARD_CAP);
        let chain = worst_case + worst_backoff_secs;
        assert!(
            worst_case + margin_secs > chain,
            "at {plafond}/{envelope} a complete retry chain ({chain}s including backoffs) \
             reaches the watchdog ({}s) — the margin no longer covers legitimate work",
            worst_case + margin_secs
        );
    }
}

/// **AC4 — negative control, the deliberate one.** A slow call that stays
/// under the watchdog is not cut.
///
/// 280 s virtual: over twice the per-call plafond (120 s) and over the
/// deadline, yet under the 300 s net. If the watchdog were a second deadline
/// rather than an anti-hang device, this would fail — and so would the two
/// mika#848 / mika#2276 tests that live at 200 s for the same reason.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn mika2342_a_slow_but_bounded_call_is_not_cut() {
    let responses = vec![delayed_response(
        280_000,
        text_response("Slow, but it arrived."),
    )];

    let harness = EvalHarness::builder()
        .responses(responses)
        .build()
        .await
        .expect("harness build");

    let (_guard, events) = capture();
    let deadline = Instant::now() + Duration::from_secs(3600);
    let trace = harness
        .run_with_deadline("Trigger a slow but bounded call.", deadline)
        .await
        .expect("a call under the watchdog must complete normally");

    assert!(
        !events
            .lock()
            .unwrap()
            .iter()
            .any(|(msg, _)| msg == "llm_call_watchdog_fired"),
        "the net must not fire on a call the rail would have returned — a false \
         positive costs a whole turn, which is the asymmetry that sizes the margin"
    );
    assert_eq!(
        trace.output.text.as_deref(),
        Some("Slow, but it arrived."),
        "the response must be delivered intact"
    );
    assert_eq!(trace.llm_calls[0].status, "success");
}

//! mika#2342 — per-attempt instrumentation of the LLM retry chain (D4, AC3).
//!
//! # What this closes
//!
//! `llm_call started` is emitted **once, before** the retry loop. So N attempts
//! produced one line, with no per-attempt latency and no request size — and
//! "one unbounded call" and "N bounded silent calls" read identically in the
//! log. That ambiguity is hypothesis 1 of the founding ticket, and nothing in
//! the log could settle it.
//!
//! # Why the N > 1 case is the test that matters
//!
//! A single-attempt assertion proves nothing here: emitting one event per call
//! is **exactly the pre-fix behaviour**. Only a chain that actually retries can
//! tell a per-attempt event apart from a per-call one, which is why this file
//! stands up a real HTTP server that fails twice before succeeding.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mika_common::llm::budget::LlmTimeoutBudget;
use mika_common::llm::openai::OpenAiCompatibleProvider;
use mika_common::llm::{LlmMessage, LlmProvider, LlmRequest, LlmRole, ProviderKind};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// -- tracing capture (same shape as `skills::builtin_handlers::tests`) --

#[derive(Debug, Clone)]
struct CapturedEvent {
    message: String,
    fields: HashMap<String, String>,
}

struct CapturingLayer {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
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
            events.push(CapturedEvent { message, fields });
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

fn capture() -> (
    tracing::subscriber::DefaultGuard,
    Arc<Mutex<Vec<CapturedEvent>>>,
) {
    use tracing_subscriber::layer::SubscriberExt;
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CapturingLayer {
        events: Arc::clone(&events),
    });
    let guard = tracing::subscriber::set_default(subscriber);
    (guard, events)
}

fn attempts(events: &Arc<Mutex<Vec<CapturedEvent>>>) -> Vec<CapturedEvent> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e.message == "llm_call_attempt")
        .cloned()
        .collect()
}

// -- fixtures --

fn ok_body() -> serde_json::Value {
    json!({
        "choices": [{
            "message": {"role": "assistant", "content": "ok"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1}
    })
}

fn provider(base_url: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(
        base_url,
        Some("test-key".into()),
        "test-model".into(),
        128,
        ProviderKind::OpenAi,
        false,
        LlmTimeoutBudget::default(),
    )
}

fn request() -> LlmRequest {
    LlmRequest {
        model: "test-model".into(),
        system: Some("a system prompt".into()),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: mika_common::llm::LlmContent::Text("hello".into()),
        }],
        tools: None,
        max_tokens: 128,
        thinking: None,
    }
}

// -- AC3 --

/// Positive control, single attempt: the event exists and carries the three
/// fields an operator needs to read a silence (`attempt`, `max_attempts`,
/// `request_bytes`).
#[tokio::test(flavor = "current_thread")]
async fn mika2342_one_attempt_emits_one_event_carrying_the_request_size() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;

    let (_guard, events) = capture();
    provider(server.uri())
        .send_message(&request())
        .await
        .expect("the mock server answers 200");

    let attempts = attempts(&events);
    assert_eq!(attempts.len(), 1, "one attempt, one event");
    assert_eq!(
        attempts[0].fields.get("attempt").map(String::as_str),
        Some("0")
    );
    assert!(attempts[0].fields.contains_key("max_attempts"));

    // `request_bytes` on the *start* of the attempt is the whole point of
    // carrying it here (E5): it is the only way to know the size of a request
    // that never comes back, since `llm_calls.request_bytes` is written only
    // once the call returns.
    let bytes: u64 = attempts[0].fields["request_bytes"]
        .parse()
        .expect("request_bytes is numeric");
    assert!(bytes > 0, "a non-empty request must report a non-zero size");
}

/// The test that separates a per-attempt event from a per-call one: two
/// retryable 500s, then a 200. Three attempts, three events, numbered.
#[tokio::test(flavor = "current_thread")]
async fn mika2342_a_retrying_chain_emits_one_event_per_attempt() {
    let server = MockServer::start().await;
    // wiremock consumes mounted expectations in order, so the first two calls
    // get the retryable 500 and the third gets the success.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;

    let (_guard, events) = capture();
    // No deadline: the chain runs to the full hard cap, so two retries fit.
    provider(server.uri())
        .send_message(&request())
        .await
        .expect("the third attempt succeeds");

    let attempts = attempts(&events);
    assert_eq!(
        attempts.len(),
        3,
        "three attempts must produce three events — one event for a three-attempt \
         chain is the pre-mika#2342 behaviour this test exists to refuse"
    );
    let numbering: Vec<&str> = attempts
        .iter()
        .map(|e| e.fields["attempt"].as_str())
        .collect();
    assert_eq!(
        numbering,
        vec!["0", "1", "2"],
        "attempts must be numbered, or an operator cannot tell how far a silent \
         chain got"
    );
}

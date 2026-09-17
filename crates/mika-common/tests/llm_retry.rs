//! The retry loop, tested against a real HTTP server (mika#2331 §3.4 / AC4).
//!
//! # Why this file did not exist before
//!
//! There was **no test of the retry loop on any of the three rails**.
//! `MockLlmProvider` cannot produce one: it does not override
//! `send_message_with_deadline` and returns its `MockResponse::Error` without a
//! loop, so it can assert what an error *is* and never how many times the
//! client asked. The question mika#2331 has to settle — *did the chain run once
//! or four times?* — is invisible to it by construction.
//!
//! # Why a raw TCP server and not `wiremock`
//!
//! The plan named `wiremock`. It cannot produce the signal these tests exist to
//! observe. The failure under test is a body **cut mid-stream**: the headers
//! announce a length, fewer bytes arrive, and the connection closes — the
//! `unexpected EOF` whose cause chain mika#2015 walks. `wiremock` serves through
//! `hyper`, which computes `Content-Length` from the body it is given and keeps
//! the connection open; a mismatched header would at best hang the client until
//! its HTTP timeout, which is a different failure (and a two-minute test). So
//! the server here speaks HTTP/1.1 by hand over a `TcpListener`: ~120 lines,
//! fully deterministic, and it counts the requests it received — which is the
//! assertion every case below rests on. AC4 asks for "a test HTTP server, not a
//! provider mock"; this is one.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use mika_common::claude::{ClaudeClient, Message, MessageContent, MessagesRequest};
use mika_common::llm::error::LlmError;
use mika_common::llm::ollama::OllamaProvider;
use mika_common::llm::openai::OpenAiCompatibleProvider;
use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};
use mika_common::llm::{LlmProvider, LlmTimeoutBudget, ProviderKind};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

// ── the test server ───────────────────────────────────────────────────────

/// What the server answers for one request.
#[derive(Clone, Debug)]
enum Reply {
    /// Announce more bytes than are sent, then close the connection: the client
    /// sees `unexpected EOF` while reading the body. This is the mika#2015
    /// signature and the only thing that separates "the bytes never arrived"
    /// from "the bytes arrived and are unreadable".
    TruncatedBody,
    /// A complete, valid 200 response.
    Ok(String),
    /// A complete response with a non-2xx status.
    Status(u16, String),
    /// A complete 200 response whose body is not the expected schema — a
    /// genuine parse failure, which must stay terminal.
    Unparseable,
}

struct FakeApi {
    addr: SocketAddr,
    hits: Arc<AtomicUsize>,
}

impl FakeApi {
    /// Replies are consumed in order; once the script is spent the last entry
    /// repeats, so a chain that runs longer than expected is caught by the hit
    /// count rather than by a connection error that could be mistaken for the
    /// failure under test.
    async fn start(script: Vec<Reply>) -> Self {
        assert!(!script.is_empty(), "a script needs at least one reply");
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_for_task = Arc::clone(&hits);

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let n = hits_for_task.fetch_add(1, Ordering::SeqCst);
                let reply = script
                    .get(n)
                    .cloned()
                    .unwrap_or_else(|| script.last().expect("non-empty").clone());

                // Drain the request head (and, best effort, its body) so the
                // client never sees a reset before it finished writing.
                let mut buf = vec![0u8; 64 * 1024];
                let _ = socket.read(&mut buf).await;

                let wire = match reply {
                    Reply::TruncatedBody => {
                        let partial = br#"{"id":"msg_trunc","cont"#;
                        // The announced length is deliberately far larger than
                        // what is written: that gap, plus the close below, is
                        // the whole fixture.
                        let mut out = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                             Content-Length: 4096\r\nConnection: close\r\n\r\n"
                            .to_vec();
                        out.extend_from_slice(partial);
                        out
                    }
                    Reply::Ok(body) => http_response(200, &body),
                    Reply::Status(code, body) => http_response(code, &body),
                    Reply::Unparseable => http_response(200, r#"{"not":"the expected schema"}"#),
                };

                let _ = socket.write_all(&wire).await;
                let _ = socket.flush().await;
                // Closing here is what turns a short body into an EOF rather
                // than a hang: without it the client would wait out its full
                // HTTP timeout for bytes that never come.
                let _ = socket.shutdown().await;
            }
        });

        Self { addr, hits }
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

fn http_response(status: u16, body: &str) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}

// ── fixtures ──────────────────────────────────────────────────────────────

/// The shipped fleet geometry: 120 s per call inside a 300 s envelope, which
/// `max_attempts` turns into exactly **2** attempts (F2). The whole point of
/// case 3 below is that this is 2 and not `MAX_RETRIES + 1 = 4`.
fn fleet_budget() -> LlmTimeoutBudget {
    LlmTimeoutBudget::unvalidated(120, 300)
}

fn deadline_in(secs: u64) -> Instant {
    Instant::now() + Duration::from_secs(secs)
}

fn llm_request() -> LlmRequest {
    LlmRequest {
        model: "test-model".into(),
        max_tokens: 64,
        system: None,
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text("hi".into()),
        }],
        tools: None,
        thinking: None,
    }
}

fn openai_body() -> String {
    r#"{"choices":[{"message":{"role":"assistant","content":"hi"},
        "finish_reason":"stop"}],
        "usage":{"prompt_tokens":3,"completion_tokens":1}}"#
        .to_string()
}

fn ollama_body() -> String {
    r#"{"model":"test-model","message":{"role":"assistant","content":"hi"},
        "done":true,"eval_count":1,"prompt_eval_count":3}"#
        .to_string()
}

fn anthropic_body() -> String {
    r#"{"id":"msg_1","content":[{"type":"text","text":"hi"}],
        "stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":1}}"#
        .to_string()
}

fn openai_provider(base_url: String, budget: LlmTimeoutBudget) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(
        base_url,
        Some("test-key".into()),
        "test-model".into(),
        64,
        ProviderKind::OpenRouter,
        false,
        budget,
    )
}

fn ollama_provider(base_url: String, budget: LlmTimeoutBudget) -> OllamaProvider {
    OllamaProvider::new(base_url, None, "test-model".into(), 64, false, budget)
}

fn anthropic_request() -> MessagesRequest {
    MessagesRequest {
        model: "claude-test".into(),
        max_tokens: 64,
        system: None,
        messages: vec![Message {
            role: "user".into(),
            content: MessageContent::Text("hi".into()),
        }],
        tools: None,
        thinking: None,
    }
}

// ── case 1 — the ticket's own test ────────────────────────────────────────

/// The negative test mika#2331 asks for, on the rail that carries 100 % of the
/// measured hangs: first response cut mid-body, second one valid ⇒ the call
/// succeeds and the server saw **exactly two** requests.
///
/// T1: this passes on the pre-fix tree. It is a **characterisation** test — it
/// freezes behaviour mika#2015 already shipped, which is the finding the ticket
/// body did not have: the retry it asks for was already there.
#[tokio::test]
#[serial]
async fn openai_retries_once_after_a_body_cut_mid_stream() {
    let api = FakeApi::start(vec![Reply::TruncatedBody, Reply::Ok(openai_body())]).await;
    let provider = openai_provider(api.base_url(), fleet_budget());

    let response = provider
        .send_message_with_deadline(&llm_request(), Some(deadline_in(300)))
        .await
        .expect("second attempt should succeed");

    assert_eq!(response.usage.input_tokens, 3);
    assert_eq!(api.hits(), 2, "exactly one retry, no more");
}

// ── case 2 — no infinite loop ─────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn openai_propagates_the_error_after_a_bounded_number_of_attempts() {
    let api = FakeApi::start(vec![Reply::TruncatedBody]).await;
    let provider = openai_provider(api.base_url(), fleet_budget());

    let err = provider
        .send_message_with_deadline(&llm_request(), Some(deadline_in(300)))
        .await
        .expect_err("a permanently broken server must propagate");

    assert!(
        err.is_transport(),
        "a body cut mid-stream is a transport failure, got {err:?}"
    );
    assert_eq!(api.hits(), 2, "the chain must stop at max_attempts");
}

// ── case 3 — max_attempts is the bound, not MAX_RETRIES ───────────────────

/// Pins F2, on which the whole diagnostic procedure of §3.5 rests: reading
/// `attempt` against `max_attempts` only means something if the budget — not
/// the provider's hard cap — is what bounds the chain.
///
/// **If this one is red, halt and report.** It would mean F2 is false, and a
/// plan whose measurement premise has just been disproved is not repaired by
/// adjusting the assertion to whatever was observed.
#[tokio::test]
#[serial]
async fn openai_chain_length_follows_the_budget_not_the_hard_cap() {
    let api = FakeApi::start(vec![Reply::Status(
        500,
        r#"{"error":{"message":"boom"}}"#.into(),
    )])
    .await;
    let provider = openai_provider(api.base_url(), fleet_budget());

    let _ = provider
        .send_message_with_deadline(&llm_request(), Some(deadline_in(300)))
        .await
        .expect_err("HTTP 500 forever must propagate");

    assert_eq!(
        api.hits(),
        2,
        "120/300 gives floor(300/120) = 2 attempts — not the hard cap of 4"
    );
}

// ── case 4 — the class is the one the ledger already knows ────────────────

/// Restricted to this rail on purpose (F8-2): on the Anthropic rail every error
/// leaves `LlmProvider` as `ProviderError`, so asserting `transport_timeout`
/// there would assert something false.
#[tokio::test]
#[serial]
async fn openai_body_cut_carries_the_transport_class_the_ledger_groups_by() {
    let api = FakeApi::start(vec![Reply::TruncatedBody]).await;
    let provider = openai_provider(api.base_url(), fleet_budget());

    let err = provider
        .send_message_with_deadline(&llm_request(), Some(deadline_in(300)))
        .await
        .expect_err("permanent truncation");

    // The class is `transport`, not `transport_timeout`: the cause chain of a
    // cut body says `unexpected EOF`, not `timed out`. Both are transport, and
    // keeping them apart is why `classify_transport_message` exists.
    assert_eq!(err.error_class(), "transport");
}

// ── case 5 — non-retryable stays non-retryable ────────────────────────────

#[tokio::test]
#[serial]
async fn openai_http_400_is_not_retried() {
    let api = FakeApi::start(vec![Reply::Status(
        400,
        r#"{"error":{"message":"bad"}}"#.into(),
    )])
    .await;
    let provider = openai_provider(api.base_url(), fleet_budget());

    let err = provider
        .send_message_with_deadline(&llm_request(), Some(deadline_in(300)))
        .await
        .expect_err("400 must propagate");

    assert!(matches!(err, LlmError::HttpError { status: 400, .. }));
    assert_eq!(api.hits(), 1, "a client error is not retried");
}

/// The other half of the §3.3 split, on the rail that already had it: a body
/// that **arrived** and does not parse is terminal. Without this, "make body
/// reads retryable" could be satisfied by making everything retryable.
#[tokio::test]
#[serial]
async fn openai_unparseable_body_is_terminal() {
    let api = FakeApi::start(vec![Reply::Unparseable]).await;
    let provider = openai_provider(api.base_url(), fleet_budget());

    let err = provider
        .send_message_with_deadline(&llm_request(), Some(deadline_in(300)))
        .await
        .expect_err("a schema mismatch must propagate");

    assert!(matches!(err, LlmError::ParseError(_)), "got {err:?}");
    assert_eq!(api.hits(), 1, "a parse failure is not retried");
}

// ── case 6a — the Ollama rail (T2: red before §3.3) ───────────────────────

/// Before §3.3 this rail mapped a read failure to `ParseError`, so the chain
/// stopped at **1** request. This is one of the two detectors that fire on the
/// pre-fix tree, and it lands in the same commit as its fix.
#[tokio::test]
#[serial]
async fn ollama_retries_once_after_a_body_cut_mid_stream() {
    let api = FakeApi::start(vec![Reply::TruncatedBody, Reply::Ok(ollama_body())]).await;
    let provider = ollama_provider(api.base_url(), fleet_budget());

    let response = provider
        .send_message_with_deadline(&llm_request(), Some(deadline_in(300)))
        .await
        .expect("second attempt should succeed");

    assert_eq!(response.usage.input_tokens, 3);
    assert_eq!(api.hits(), 2, "exactly one retry, no more");
}

#[tokio::test]
#[serial]
async fn ollama_unparseable_body_is_terminal() {
    let api = FakeApi::start(vec![Reply::Unparseable]).await;
    let provider = ollama_provider(api.base_url(), fleet_budget());

    let err = provider
        .send_message_with_deadline(&llm_request(), Some(deadline_in(300)))
        .await
        .expect_err("a schema mismatch must propagate");

    assert!(matches!(err, LlmError::ParseError(_)), "got {err:?}");
    assert_eq!(
        api.hits(),
        1,
        "the split must not make everything retryable"
    );
}

// ── case 6b — the Anthropic rail (T2: red before §3.3) ────────────────────

/// **The assertion is the request count, never the class** (F8-2):
/// `llm::anthropic` flattens every error off this rail into `ProviderError`, so
/// a class assertion at that boundary would assert something false. It is also
/// independent of `MAX_RETRIES`, which this rail still consumes instead of
/// `max_attempts` — a success on the second try stops at 2 whatever the bound.
#[tokio::test]
#[serial]
async fn anthropic_retries_once_after_a_body_cut_mid_stream() {
    let api = FakeApi::start(vec![Reply::TruncatedBody, Reply::Ok(anthropic_body())]).await;
    let client = ClaudeClient::for_test(api.base_url(), "claude-test".into(), 10);

    let response = client
        .send_message_with_deadline(&anthropic_request(), Some(deadline_in(300)), None)
        .await
        .expect("second attempt should succeed");

    assert_eq!(response.usage.input_tokens, 3);
    assert_eq!(api.hits(), 2, "exactly one retry, no more");
}

#[tokio::test]
#[serial]
async fn anthropic_unparseable_body_is_terminal() {
    let api = FakeApi::start(vec![Reply::Unparseable]).await;
    let client = ClaudeClient::for_test(api.base_url(), "claude-test".into(), 10);

    let err = client
        .send_message_with_deadline(&anthropic_request(), Some(deadline_in(300)), None)
        .await
        .expect_err("a schema mismatch must propagate");

    assert!(
        err.to_string().contains("unexpected response")
            || format!("{err:?}").contains("parse error"),
        "expected a parse-class failure, got {err:?}"
    );
    assert_eq!(api.hits(), 1, "a parse failure is not retried on this rail");
}

/// The mika#1744 threshold, on the rail where forgetting `BodyRead` would have
/// been silent (§3.3).
///
/// The deadline is set **between** the two thresholds this rail uses: 90 s
/// remaining clears the transport threshold (60 s) and not the default one
/// (90 + 30 = 120 s). So the retry happens only if a cut body counts as
/// transport. Had `is_transport_class` kept its old `Transport`-only shape, the
/// chain would abandon after one request — no error, no symptom, just a retry
/// that silently stops being attempted.
#[tokio::test]
#[serial]
async fn anthropic_body_cut_uses_the_transport_retry_threshold() {
    let api = FakeApi::start(vec![Reply::TruncatedBody, Reply::Ok(anthropic_body())]).await;
    let client = ClaudeClient::for_test(api.base_url(), "claude-test".into(), 10);

    let response = client
        .send_message_with_deadline(&anthropic_request(), Some(deadline_in(90)), None)
        .await
        .expect("the retry must be allowed under the transport threshold");

    assert_eq!(response.usage.input_tokens, 3);
    assert_eq!(
        api.hits(),
        2,
        "90 s remaining clears the 60 s transport threshold but not the 120 s default one"
    );
}

// ── mika#2362 — `retrying` may not describe a retry that will not run ─────
//
// The founding trace, 2026-09-17, mika-arch under a `300/600` geometry:
//
// ```
// 14:11:36.940Z  llm_call_attempt  attempt=0  max_attempts=2
// 14:16:36.941Z  llm_call_attempt  attempt=0  max_attempts=2  elapsed_ms=300000  outcome="retrying"
// 14:16:36.959Z  turn_usage        stop_reason=error  latency_ms=300001  status=error
// ```
//
// `attempt=0` cut at exactly the cap, announcing a retry — and 18 ms later the
// turn errored with no `attempt=1` anywhere. `600 − 300 = 300` is exactly the
// non-transport threshold, so the margin was zero and the deadline guard
// refused what the outcome line had just promised.
//
// The geometry below is that one, scaled to `20/40` so the test runs in
// milliseconds: the arithmetic that matters is `envelope = 2 × cap`, not the
// absolute seconds. The deadline is posed at `cap` from now, which is exactly
// the state "attempt 0 has consumed the full cap under a `2 × cap` envelope" —
// the same measurement without the wait. `ε > 0` (any real call consumes some
// wall-clock) is what makes the margin fall strictly below the threshold, and
// that is the whole of the off-by-one.

/// The incident's shape: cap `P`, envelope `2 × P`.
fn incident_budget() -> LlmTimeoutBudget {
    LlmTimeoutBudget::unvalidated(20, 40)
}

/// One `llm_call_attempt` line, reduced to the fields these assertions read.
#[derive(Debug)]
struct Attempt {
    attempt: u64,
    outcome: String,
    elapsed_ms: u64,
}

/// Why **every** test in this file carries `#[serial]`, not just the capturing
/// ones.
///
/// `tracing::subscriber::set_default` is thread-local, but two things a
/// capturing test depends on are process-global: the count of live scoped
/// subscribers (which feeds the max-level hint the `info!` macros consult) and
/// the per-callsite `Interest` cache (see `capture::start`). A test running in
/// parallel — **including one that installs no subscriber at all** — can move
/// either one out from under a capture in progress, and the symptom is an empty
/// capture, i.e. a failure that reads as "the rail emitted nothing".
///
/// Serializing only the capturing tests was tried first and was not enough: the
/// failure moved to a different test on each run. The whole file is therefore
/// serial. It costs a couple of seconds and buys a deterministic suite; a
/// flaky assertion about a log line is worse than a slow one, because the
/// natural response to it is to stop believing the line.
use serial_test::serial;

mod capture {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::Attempt;

    #[derive(Default)]
    pub struct Sink(pub Arc<Mutex<Vec<HashMap<String, String>>>>);

    struct Layer(Arc<Mutex<Vec<HashMap<String, String>>>>);
    struct Visitor<'a>(&'a mut HashMap<String, String>);

    impl tracing::field::Visit for Visitor<'_> {
        fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
            self.0.insert(f.name().into(), format!("{v:?}"));
        }
        fn record_str(&mut self, f: &tracing::field::Field, v: &str) {
            self.0.insert(f.name().into(), v.into());
        }
        fn record_u64(&mut self, f: &tracing::field::Field, v: u64) {
            self.0.insert(f.name().into(), v.to_string());
        }
        fn record_i64(&mut self, f: &tracing::field::Field, v: i64) {
            self.0.insert(f.name().into(), v.to_string());
        }
        fn record_bool(&mut self, f: &tracing::field::Field, v: bool) {
            self.0.insert(f.name().into(), v.to_string());
        }
    }

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Layer {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut fields = HashMap::new();
            event.record(&mut Visitor(&mut fields));
            if let Ok(mut seen) = self.0.lock() {
                seen.push(fields);
            }
        }
    }

    /// Install a capturing subscriber for the current thread.
    ///
    /// `rebuild_interest_cache` is **not** optional here, and the reason is the
    /// trap this file is most likely to re-lose: a `tracing` callsite caches
    /// its `Interest` **globally**, decided by whichever thread reaches it
    /// first. In a test binary that is routinely a test with no subscriber
    /// installed, which answers `never` — after which the capturing test on
    /// another thread observes nothing at all, and the failure looks like "the
    /// rail did not emit" rather than "the callsite was disabled before we got
    /// there". Measured here as an empty capture moving from one test to
    /// another between runs. Rebuilding on install re-asks every callsite under
    /// the subscriber now in place.
    ///
    /// It composes with the `#[serial]` on every test in this file (see the
    /// note above `Attempt`): the rebuild fixes the cache, and the
    /// serialization is what stops a parallel test from re-deciding it mid
    /// capture.
    pub fn start() -> (tracing::subscriber::DefaultGuard, Sink) {
        use tracing_subscriber::layer::SubscriberExt;
        let sink = Sink::default();
        let subscriber = tracing_subscriber::registry().with(Layer(Arc::clone(&sink.0)));
        let guard = tracing::subscriber::set_default(subscriber);
        tracing::callsite::rebuild_interest_cache();
        (guard, sink)
    }

    impl Sink {
        /// The `llm_call_attempt` lines, in order.
        ///
        /// Filtered on `event == "llm_call_attempt"`, the same guard the
        /// operator procedure requires (mika#2331): the name is a homonym of
        /// the mika#2342 pre-call line, which carries no `outcome`.
        pub fn attempts(&self) -> Vec<Attempt> {
            self.0
                .lock()
                .expect("sink")
                .iter()
                .filter(|f| f.get("event").map(String::as_str) == Some("llm_call_attempt"))
                .map(|f| Attempt {
                    attempt: f["attempt"].parse().expect("attempt is a number"),
                    outcome: f["outcome"].clone(),
                    elapsed_ms: f["elapsed_ms"].parse().expect("elapsed_ms is a number"),
                })
                .collect()
        }
    }
}

/// T1 / AC1 / AC2 — the test the ticket asks for, with its positive control in
/// the same call.
///
/// Negative side (the incident): under `envelope = 2 × cap`, attempt 0 fails on
/// a **retryable non-transport** error (HTTP 429) with the full cap consumed.
/// The line must say `exhausted`, never `retrying`; the attempt that did not
/// happen must carry `deadline_abort` with `elapsed_ms = 0`; and the server
/// must have seen exactly one request.
///
/// Positive side (the fleet geometry): the same failure with a real margin must
/// still say `retrying` and really retry. Without it, a predicate answering
/// `exhausted` unconditionally would pass — and would have destroyed the retry
/// this ticket must not touch.
#[tokio::test]
#[serial]
async fn mika2362_openai_zero_margin_says_exhausted_and_a_real_margin_retries() {
    // -- negative control: the incident's geometry --
    let api = FakeApi::start(vec![Reply::Status(
        429,
        r#"{"error":{"message":"slow"}}"#.into(),
    )])
    .await;
    let provider = openai_provider(api.base_url(), incident_budget());

    let (guard, sink) = capture::start();
    let _ = provider
        .send_message_with_deadline(&llm_request(), Some(deadline_in(20)))
        .await
        .expect_err("the chain must fail");
    let lines = sink.attempts();
    drop(guard);

    assert_eq!(api.hits(), 1, "no second attempt may run: {lines:?}");
    assert_eq!(lines[0].attempt, 0, "{lines:?}");
    assert_eq!(
        lines[0].outcome, "exhausted",
        "attempt 0 announced a retry that cannot run: {lines:?}"
    );
    assert_eq!(lines[1].attempt, 1, "{lines:?}");
    assert_eq!(
        lines[1].outcome, "deadline_abort",
        "the attempt that did not happen keeps its own name: {lines:?}"
    );
    assert_eq!(
        lines[1].elapsed_ms, 0,
        "`deadline_abort` measures nothing, by contract"
    );

    // -- positive control: the fleet geometry, where the margin is real --
    let api = FakeApi::start(vec![
        Reply::Status(429, r#"{"error":{"message":"slow"}}"#.into()),
        Reply::Ok(openai_body()),
    ])
    .await;
    let provider = openai_provider(api.base_url(), fleet_budget());

    let (guard, sink) = capture::start();
    provider
        // 180 s is what a 120/300 geometry has left after a full-cap attempt.
        .send_message_with_deadline(&llm_request(), Some(deadline_in(180)))
        .await
        .expect("the retry must run and succeed");
    let lines = sink.attempts();
    drop(guard);

    assert_eq!(api.hits(), 2, "the real retry must still happen: {lines:?}");
    assert_eq!(
        lines[0].outcome, "retrying",
        "a retry that will run must still be announced: {lines:?}"
    );
}

/// T2 / AC6 — the transport class is untouched.
///
/// A body cut mid-stream clears the smaller `0.50 × cap` threshold (mika#1744),
/// so the very margin that refuses a 429 lets this through. This is the control
/// that proves mika#2362 did not tighten the chain the mika#2342 watchdog is
/// sized on — the most expensive regression this change could produce.
#[tokio::test]
#[serial]
async fn mika2362_openai_transport_still_retries_at_the_same_margin() {
    let api = FakeApi::start(vec![Reply::TruncatedBody, Reply::Ok(openai_body())]).await;
    let provider = openai_provider(api.base_url(), incident_budget());

    let (guard, sink) = capture::start();
    let response = provider
        .send_message_with_deadline(&llm_request(), Some(deadline_in(20)))
        .await
        .expect("a transport failure must still get its second attempt");
    let lines = sink.attempts();
    drop(guard);

    assert_eq!(response.usage.input_tokens, 3);
    assert_eq!(api.hits(), 2, "the transport chain must not shorten");
    assert_eq!(
        lines[0].outcome, "retrying",
        "and it must still be announced as one: {lines:?}"
    );
}

/// T3 — the same correction, on the ollama rail.
///
/// Three rails and not one: a fix on the single measured rail would leave two
/// copies free to diverge, and an absent line on an uncorrected rail reads as
/// "this rail did not retry" (mika#2331's own argument for instrumenting all
/// three).
#[tokio::test]
#[serial]
async fn mika2362_ollama_zero_margin_says_exhausted_and_a_real_margin_retries() {
    let api = FakeApi::start(vec![Reply::Status(429, r#"{"error":"slow"}"#.into())]).await;
    let provider = ollama_provider(api.base_url(), incident_budget());

    let (guard, sink) = capture::start();
    let _ = provider
        .send_message_with_deadline(&llm_request(), Some(deadline_in(20)))
        .await
        .expect_err("the chain must fail");
    let lines = sink.attempts();
    drop(guard);

    assert_eq!(api.hits(), 1, "no second attempt may run: {lines:?}");
    assert_eq!(lines[0].outcome, "exhausted", "{lines:?}");
    assert_eq!(lines[1].outcome, "deadline_abort", "{lines:?}");

    // Positive control, same rail.
    let api = FakeApi::start(vec![
        Reply::Status(429, r#"{"error":"slow"}"#.into()),
        Reply::Ok(ollama_body()),
    ])
    .await;
    let provider = ollama_provider(api.base_url(), fleet_budget());

    let (guard, sink) = capture::start();
    provider
        .send_message_with_deadline(&llm_request(), Some(deadline_in(180)))
        .await
        .expect("the retry must run and succeed");
    let lines = sink.attempts();
    drop(guard);

    assert_eq!(api.hits(), 2, "{lines:?}");
    assert_eq!(lines[0].outcome, "retrying", "{lines:?}");
}

/// T3 — and on the Anthropic rail.
///
/// This rail bounds its loop on `MAX_RETRIES` rather than on the budget, so
/// `max_attempts` is 4 whatever the geometry: D2's `floor(E/P)` arithmetic does
/// not apply to it. D1 does — the deadline guard is the same, and an outcome
/// line ignoring it lied here exactly as it did elsewhere. The margin is posed
/// below the default-cap threshold (120 s) and above the transport one (60 s),
/// which is this rail's equivalent of a zero margin.
#[tokio::test]
#[serial]
async fn mika2362_anthropic_zero_margin_says_exhausted_and_a_real_margin_retries() {
    let api = FakeApi::start(vec![Reply::Status(
        429,
        r#"{"error":{"message":"slow"}}"#.into(),
    )])
    .await;
    let client = ClaudeClient::for_test(api.base_url(), "claude-test".into(), 10);

    let (guard, sink) = capture::start();
    let _ = client
        // 90 s: under the 120 s non-transport threshold, over the 60 s
        // transport one — a 429 is not transport, so no second attempt.
        .send_message_with_deadline(&anthropic_request(), Some(deadline_in(90)), None)
        .await
        .expect_err("the chain must fail");
    let lines = sink.attempts();
    drop(guard);

    assert_eq!(api.hits(), 1, "no second attempt may run: {lines:?}");
    assert_eq!(lines[0].outcome, "exhausted", "{lines:?}");
    assert_eq!(lines[1].outcome, "deadline_abort", "{lines:?}");

    // Positive control: a margin above the threshold retries and succeeds.
    let api = FakeApi::start(vec![
        Reply::Status(429, r#"{"error":{"message":"slow"}}"#.into()),
        Reply::Ok(anthropic_body()),
    ])
    .await;
    let client = ClaudeClient::for_test(api.base_url(), "claude-test".into(), 10);

    let (guard, sink) = capture::start();
    client
        .send_message_with_deadline(&anthropic_request(), Some(deadline_in(300)), None)
        .await
        .expect("the retry must run and succeed");
    let lines = sink.attempts();
    drop(guard);

    assert_eq!(api.hits(), 2, "{lines:?}");
    assert_eq!(lines[0].outcome, "retrying", "{lines:?}");
}

/// T3b — the post-loop error message follows the same verdict.
///
/// This is the third site of the divergence: it chose between "deadline budget
/// insufficient" and "max retries exceeded" by **re-deriving** the guard's
/// threshold, under a comment saying it did so "so the two branches agree".
///
/// It is asserted on the Anthropic rail because that is where the message is
/// *observable*: it wraps the real error as context. On the OpenAI-shaped rails
/// the same decision feeds an `unwrap_or_else` whose `None` branch their loop
/// cannot reach (every iteration either returns or records `last_error`), so
/// the message is unreachable there — the predicate is shared all the same, and
/// `retry_gate`'s unit tests cover it directly.
///
/// Negative control in the same test: with the margin wide and the error
/// non-retryable, the chain must not claim the deadline stopped it.
#[tokio::test]
#[serial]
async fn mika2362_anthropic_post_loop_message_names_the_deadline_not_the_retries() {
    let api = FakeApi::start(vec![Reply::Status(
        429,
        r#"{"error":{"message":"slow"}}"#.into(),
    )])
    .await;
    let client = ClaudeClient::for_test(api.base_url(), "claude-test".into(), 10);

    let err = client
        .send_message_with_deadline(&anthropic_request(), Some(deadline_in(90)), None)
        .await
        .expect_err("the chain must fail");
    let chain = format!("{err:#}");
    assert!(
        chain.contains("deadline budget insufficient"),
        "a chain stopped by the deadline must say so, got: {chain}"
    );

    // Negative control: a terminal error with all the margin in the world must
    // not be reported as a deadline abort.
    let api = FakeApi::start(vec![Reply::Status(
        400,
        r#"{"error":{"message":"bad"}}"#.into(),
    )])
    .await;
    let client = ClaudeClient::for_test(api.base_url(), "claude-test".into(), 10);

    let err = client
        .send_message_with_deadline(&anthropic_request(), Some(deadline_in(3_000)), None)
        .await
        .expect_err("400 must propagate");
    let chain = format!("{err:#}");
    assert!(
        !chain.contains("deadline budget insufficient"),
        "the deadline did not stop this one, got: {chain}"
    );
}

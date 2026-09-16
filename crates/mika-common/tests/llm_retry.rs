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
async fn anthropic_retries_once_after_a_body_cut_mid_stream() {
    let api = FakeApi::start(vec![Reply::TruncatedBody, Reply::Ok(anthropic_body())]).await;
    let client = ClaudeClient::for_test(api.base_url(), "claude-test".into(), 10);

    let response = client
        .send_message_with_deadline(&anthropic_request(), Some(deadline_in(300)))
        .await
        .expect("second attempt should succeed");

    assert_eq!(response.usage.input_tokens, 3);
    assert_eq!(api.hits(), 2, "exactly one retry, no more");
}

#[tokio::test]
async fn anthropic_unparseable_body_is_terminal() {
    let api = FakeApi::start(vec![Reply::Unparseable]).await;
    let client = ClaudeClient::for_test(api.base_url(), "claude-test".into(), 10);

    let err = client
        .send_message_with_deadline(&anthropic_request(), Some(deadline_in(300)))
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
async fn anthropic_body_cut_uses_the_transport_retry_threshold() {
    let api = FakeApi::start(vec![Reply::TruncatedBody, Reply::Ok(anthropic_body())]).await;
    let client = ClaudeClient::for_test(api.base_url(), "claude-test".into(), 10);

    let response = client
        .send_message_with_deadline(&anthropic_request(), Some(deadline_in(90)))
        .await
        .expect("the retry must be allowed under the transport threshold");

    assert_eq!(response.usage.input_tokens, 3);
    assert_eq!(
        api.hits(),
        2,
        "90 s remaining clears the 60 s transport threshold but not the 120 s default one"
    );
}

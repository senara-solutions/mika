//! E4 adversarial no-log runtime test (mika#1810).
//!
//! Complements the E1 `CapturingLayer` test in `super::tests` — that test
//! visits the tracing *events* and asserts the field-shape invariant. This
//! test operates on the FORMATTED output stream — the bytes a real
//! `tracing_subscriber::fmt` layer would write to stdout / a log file — with
//! the max-verbose filter (`trace`) engaged.
//!
//! Rationale: the E1 test is structural — it catches a query field appearing
//! on a substrate event. The adversarial test catches format-level leakage
//! that a structural visitor could miss: a formatter that shipped a bug
//! rendering a field it shouldn't, a third-party crate that emitted via a
//! path the field visitor doesn't cover, or a substrate change that adds a
//! new event whose fields the E1 allowlist doesn't yet reject.
//!
//! Together, structural + adversarial = the "vérifié" half of "no-log par
//! construction, vérifié" (Prime 2026-07-19).

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use secrecy::SecretString;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{EnvFilter, Layer};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{BraveConfig, SearchEgressClient, SearchRequest, SearchUpstream};

/// A `MakeWriter` that appends every formatted line to a shared buffer.
/// Wraps a `Arc<Mutex<Vec<u8>>>` — clones share the same underlying buffer.
#[derive(Clone, Default)]
struct SharedBuffer {
    inner: Arc<Mutex<Vec<u8>>>,
}

impl SharedBuffer {
    fn snapshot(&self) -> Vec<u8> {
        self.inner.lock().unwrap().clone()
    }
}

impl Write for SharedBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut guard = self.inner.lock().unwrap();
        guard.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for SharedBuffer {
    type Writer = SharedBuffer;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn brave_client_pointing_at(server: &MockServer) -> SearchEgressClient {
    SearchEgressClient::new(SearchUpstream::Brave(BraveConfig {
        api_key: SecretString::from("adversarial-token-do-not-log"),
        endpoint: format!("{}/res/v1/web/search", server.uri()),
    }))
}

/// **Adversarial E4 no-log test — mika#1810.**
///
/// Runs a successful search against a wiremocked Brave endpoint with the
/// tracing subscriber wired to a `fmt` layer at TRACE filter level and a
/// capturing buffer. Asserts that the sensitive query bytes appear NOWHERE
/// in the captured formatted output.
///
/// If this test fails, the Q4 STRIP TOTAL invariant is broken at the
/// format-render level — do NOT weaken the assertion. Fix the emit side.
#[tokio::test]
async fn adversarial_no_query_leak_in_formatted_output() {
    let buffer = SharedBuffer::default();

    // fmt layer at TRACE — the most verbose realistic setting. If any log
    // line ANYWHERE in the stack renders the query, this catches it.
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(buffer.clone())
        .with_ansi(false)
        .with_target(true)
        .with_level(true)
        .with_filter(EnvFilter::new("trace"));
    let subscriber = tracing_subscriber::registry().with(fmt_layer);

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/res/v1/web/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "web": {
                "results": [
                    {"title": "T", "url": "https://ex.com", "description": "d"}
                ]
            }
        })))
        .mount(&server)
        .await;

    // Sentinel strings that MUST NOT appear in the formatted output stream.
    // The `sensitive_query` is the primary target — a substrate that ever
    // logs the query would flip this test red. `sensitive_secret_token` is
    // the API-key sentinel — the same test protects against the SecretString
    // discipline being weakened.
    let sensitive_query = "TENANT-1810-E4-ADVERSARIAL-QUERY-do-not-leak";
    let sensitive_secret_token = "adversarial-token-do-not-log";

    let client = brave_client_pointing_at(&server);
    let _guard = set_default(subscriber);

    let req = SearchRequest {
        query: sensitive_query.to_string(),
        max_results: 3,
    };
    let _ = client.search(req).await;
    drop(_guard);

    let bytes = buffer.snapshot();
    let text = String::from_utf8_lossy(&bytes);

    assert!(
        !text.contains(sensitive_query),
        "query bytes leaked into formatted tracing output — Q4 STRIP TOTAL violated. \
         Captured output ({} bytes):\n{text}",
        bytes.len(),
    );

    assert!(
        !text.contains(sensitive_secret_token),
        "API key leaked into formatted tracing output — SecretString discipline broken. \
         Captured output ({} bytes):\n{text}",
        bytes.len(),
    );

    // Positive shape check — the two audit events must be present in the
    // captured stream, so a bug that dropped both events (and hence made
    // the no-leak assertion trivially pass) would be caught. Use loose
    // fmt-formatted contains — the exact rendering differs by fmt version.
    assert!(
        text.contains("search_requested") || text.contains(r#"event="search_requested""#),
        "expected 'search_requested' event in formatted output, got:\n{text}"
    );
    assert!(
        text.contains("search_egress") || text.contains(r#"event="search_egress""#),
        "expected 'search_egress' event in formatted output, got:\n{text}"
    );
}

/// The failure-path variant of the adversarial test — same shape, but the
/// substrate returns a taxonomy error (offline endpoint via wiremock returning
/// 500 → retry → 500). Ensures the failure emission path also holds the
/// no-leak invariant at the format level.
#[tokio::test]
async fn adversarial_no_query_leak_on_failure_path() {
    let buffer = SharedBuffer::default();
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(buffer.clone())
        .with_ansi(false)
        .with_target(true)
        .with_level(true)
        .with_filter(EnvFilter::new("trace"));
    let subscriber = tracing_subscriber::registry().with(fmt_layer);

    let server = MockServer::start().await;
    // Persistent 500 — the substrate retries once and then returns
    // UpstreamStatus(500). Two round-trips through the format pipeline.
    Mock::given(method("GET"))
        .and(path("/res/v1/web/search"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let sensitive_query = "TENANT-1810-E4-FAILURE-PATH-QUERY-still-not-a-leak";
    let client = brave_client_pointing_at(&server);
    let _guard = set_default(subscriber);
    let _ = client
        .search(SearchRequest {
            query: sensitive_query.to_string(),
            max_results: 5,
        })
        .await;
    drop(_guard);

    let bytes = buffer.snapshot();
    let text = String::from_utf8_lossy(&bytes);

    assert!(
        !text.contains(sensitive_query),
        "query bytes leaked into formatted tracing output on FAILURE path — Q4 violated. \
         Captured output ({} bytes):\n{text}",
        bytes.len(),
    );

    // Failure path must still emit the audit event with taxonomy status.
    assert!(
        text.contains("search_egress"),
        "expected 'search_egress' audit event on failure path, got:\n{text}"
    );
}

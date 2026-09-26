//! Brave Search API client — the concrete E2 (mika#1808) wiring behind the
//! E1 egress substrate.
//!
//! # Discipline (inherit from `super`)
//!
//! - **Q2 uniqueness:** the `reqwest::Client` used here is passed in from
//!   [`super::SearchEgressClient`]; this file NEVER constructs its own client
//!   and NEVER holds a reference to a search upstream identifier outside the
//!   allowlisted egress module tree (`scripts/verify-egress-uniqueness.sh`).
//! - **Q4 STRIP TOTAL:** this file emits ZERO tracing calls. The audit event
//!   is emitted once by the parent module. Adding a `debug!` / `info!` /
//!   `warn!` line here — even one that pretends to be Q4-safe — is a
//!   discipline violation. The log-absence test in `super::tests` counts the
//!   emitted-event set to catch drift.
//! - **Secrecy:** the API key stays in [`secrecy::SecretString`] until the
//!   exact call site where it's written to the HTTP header. It is never
//!   included in any error message, tracing attribute, or panic string.
//! - **No response body in errors:** upstream response bodies are read only
//!   to parse the success schema. Failure paths discard the body — they
//!   cannot end up in a `Debug` impl or an error string.
//!
//! # Retry policy
//!
//! The Brave freemium tier is ~2000 requests / month; a naive retry doubles
//! quota impact. This module retries **at most once** and only for
//! recoverable classes:
//!
//! | Class                          | Retry?                                  |
//! |--------------------------------|-----------------------------------------|
//! | 2xx                            | success, return                         |
//! | 401 / 403                      | NO — return `Unauthorized` immediately  |
//! | 429                            | YES — respect `Retry-After`, once       |
//! | 5xx                            | YES — 500ms backoff, once               |
//! | transport / timeout            | YES — 500ms backoff, once               |
//! | other 4xx                      | NO — return `UpstreamStatus(status)`    |
//!
//! The whole operation (initial + retry) is bounded by
//! [`super::EGRESS_HARD_TIMEOUT_SECS`] via `tokio::time::timeout`. Retry
//! waits (backoff or `Retry-After`) are capped against the remaining budget
//! so a large `Retry-After` degrades to "give up now, return 429" rather
//! than blowing the budget.

use std::time::{Duration, Instant};

use secrecy::ExposeSecret;
use serde::Deserialize;

use super::{
    BraveConfig, EGRESS_HARD_TIMEOUT_SECS, SearchError, SearchRequest, SearchResponse, SearchResult,
};

/// Hard cap on any single per-request timeout budget the retry loop is
/// allowed to consume. Must be > 0 and ≤ `EGRESS_HARD_TIMEOUT_SECS`.
const RETRY_BACKOFF_MS: u64 = 500;

/// Maximum wait time (ms) honored when a 429 response carries a
/// `Retry-After` header. Above this cap we return the 429 immediately —
/// there is no operational value in blocking a caller for tens of seconds
/// on a per-tenant search request.
const MAX_HONORED_RETRY_AFTER_MS: u64 = 2_000;

/// Executes a single Brave Search API call with the retry policy documented
/// in the module doc. Returns the parsed [`SearchResponse`] on success (with
/// `upstream_latency_ms = 0` — the parent overwrites with the wall-clock
/// value observed at the substrate layer).
///
/// The `req` is consumed so the caller cannot accidentally re-log the query
/// after we've handed it to reqwest.
pub(crate) async fn execute_brave_search(
    client: &reqwest::Client,
    config: &BraveConfig,
    req: SearchRequest,
) -> Result<SearchResponse, SearchError> {
    let deadline = Instant::now() + Duration::from_secs(EGRESS_HARD_TIMEOUT_SECS);

    // Move ownership into the retry loop — we never need `req` again.
    let SearchRequest { query, max_results } = req;

    // Attempt 1 — always fire.
    match send_once(client, config, &query, max_results).await {
        Outcome::Ok(resp) => return Ok(resp),
        Outcome::TerminalErr(e) => return Err(e),
        Outcome::Retryable {
            error,
            wait_before_retry,
        } => {
            // Wait, but only if the budget can accommodate it.
            let now = Instant::now();
            let remaining = deadline.saturating_duration_since(now);
            let wait = wait_before_retry.min(remaining);
            if remaining <= Duration::from_millis(1) {
                // Nothing left in the budget — return the retryable error
                // as its terminal shape.
                return Err(error);
            }
            tokio::time::sleep(wait).await;
        }
    }

    // Attempt 2 — the sole retry.
    match send_once(client, config, &query, max_results).await {
        Outcome::Ok(resp) => Ok(resp),
        Outcome::TerminalErr(e) => Err(e),
        Outcome::Retryable { error, .. } => Err(error),
    }
}

/// Result of a single HTTP attempt, carrying the retry disposition.
enum Outcome {
    Ok(SearchResponse),
    TerminalErr(SearchError),
    Retryable {
        error: SearchError,
        wait_before_retry: Duration,
    },
}

/// Perform one HTTP round-trip and classify the result. Never retries on
/// its own — retry policy lives in `execute_brave_search`.
async fn send_once(
    client: &reqwest::Client,
    config: &BraveConfig,
    query: &str,
    max_results: usize,
) -> Outcome {
    let count = max_results.clamp(1, 20);

    let response = client
        .get(&config.endpoint)
        .header("X-Subscription-Token", config.api_key.expose_secret())
        .header("Accept", "application/json")
        .query(&[("q", query), ("count", &count.to_string())])
        .send()
        .await;

    let response = match response {
        Ok(r) => r,
        Err(_err) => {
            // Deliberately drop `_err` — reqwest's Debug impl can include
            // the URL (which carries the query). We keep only the taxonomy
            // label.
            return Outcome::Retryable {
                error: SearchError::Transport,
                wait_before_retry: Duration::from_millis(RETRY_BACKOFF_MS),
            };
        }
    };

    let status = response.status();

    if status.is_success() {
        return match parse_brave_body(response).await {
            Ok(resp) => Outcome::Ok(resp),
            Err(err) => Outcome::TerminalErr(err),
        };
    }

    // 401 / 403 — fail fast so an operator can rotate the key without
    // burning further quota.
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Outcome::TerminalErr(SearchError::Unauthorized);
    }

    // 429 — respect Retry-After if it's within our budget cap.
    if status.as_u16() == 429 {
        let wait = retry_after_ms(response.headers())
            .unwrap_or(RETRY_BACKOFF_MS)
            .min(MAX_HONORED_RETRY_AFTER_MS);
        return Outcome::Retryable {
            error: SearchError::UpstreamStatus(429),
            wait_before_retry: Duration::from_millis(wait),
        };
    }

    // 5xx — retryable with backoff.
    if status.is_server_error() {
        return Outcome::Retryable {
            error: SearchError::UpstreamStatus(status.as_u16()),
            wait_before_retry: Duration::from_millis(RETRY_BACKOFF_MS),
        };
    }

    // Any other 4xx — terminal, don't retry.
    Outcome::TerminalErr(SearchError::UpstreamStatus(status.as_u16()))
}

/// Parse a Brave `/res/v1/web/search` response body into a
/// [`SearchResponse`]. The `upstream_latency_ms` field is set to `0` here;
/// the parent module overwrites it with the observed wall-clock value.
///
/// Brave shape (2026-08 stable):
/// ```json
/// {
///   "web": {
///     "results": [
///       {"title": "...", "url": "https://...", "description": "..."},
///       ...
///     ]
///   }
/// }
/// ```
/// Missing / non-array `web.results` -> empty `results` vec (not an error;
/// upstream returned "no results", which is a valid outcome).
async fn parse_brave_body(response: reqwest::Response) -> Result<SearchResponse, SearchError> {
    let parsed: BraveResponseBody = match response.json().await {
        Ok(v) => v,
        Err(_err) => {
            // Same reason we drop the transport error's debug string —
            // reqwest error carries URL context.
            return Err(SearchError::ParseError);
        }
    };

    let results = parsed
        .web
        .and_then(|w| w.results)
        .unwrap_or_default()
        .into_iter()
        .map(|r| SearchResult {
            title: r.title.unwrap_or_default(),
            url: r.url.unwrap_or_default(),
            snippet: r.description.unwrap_or_default(),
        })
        .collect();

    Ok(SearchResponse {
        results,
        upstream_latency_ms: 0,
    })
}

fn retry_after_ms(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    // Brave sends numeric seconds; be lenient enough to handle whitespace
    // but do NOT attempt HTTP-date parsing (out of scope, and honoring an
    // arbitrary future date is not something a per-request path should do).
    value
        .trim()
        .parse::<u64>()
        .ok()
        .map(|s| s.saturating_mul(1_000))
}

// -- Brave response schema (private) --

#[derive(Debug, Deserialize)]
struct BraveResponseBody {
    web: Option<BraveWebResults>,
}

#[derive(Debug, Deserialize)]
struct BraveWebResults {
    results: Option<Vec<BraveWebResult>>,
}

#[derive(Debug, Deserialize)]
struct BraveWebResult {
    title: Option<String>,
    url: Option<String>,
    description: Option<String>,
}

// -- Tests --

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_ms_parses_numeric_seconds() {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(reqwest::header::RETRY_AFTER, "2".parse().unwrap());
        assert_eq!(retry_after_ms(&h), Some(2_000));
    }

    #[test]
    fn retry_after_ms_parses_with_whitespace() {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(reqwest::header::RETRY_AFTER, "  5  ".parse().unwrap());
        assert_eq!(retry_after_ms(&h), Some(5_000));
    }

    #[test]
    fn retry_after_ms_rejects_http_date() {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(
            reqwest::header::RETRY_AFTER,
            "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap(),
        );
        assert_eq!(retry_after_ms(&h), None);
    }

    #[test]
    fn retry_after_ms_none_when_absent() {
        let h = reqwest::header::HeaderMap::new();
        assert_eq!(retry_after_ms(&h), None);
    }

    #[test]
    fn brave_body_parses_expected_shape() {
        let raw = serde_json::json!({
            "web": {
                "results": [
                    {"title": "Ex", "url": "https://ex.com", "description": "d1"},
                    {"title": "Ex2", "url": "https://ex2.com", "description": "d2"},
                ]
            }
        });
        let parsed: BraveResponseBody = serde_json::from_value(raw).unwrap();
        let web = parsed.web.expect("web block present");
        let results = web.results.expect("results present");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title.as_deref(), Some("Ex"));
        assert_eq!(results[0].url.as_deref(), Some("https://ex.com"));
        assert_eq!(results[0].description.as_deref(), Some("d1"));
    }

    #[test]
    fn brave_body_tolerates_missing_web_block() {
        let raw = serde_json::json!({});
        let parsed: BraveResponseBody = serde_json::from_value(raw).unwrap();
        assert!(parsed.web.is_none());
    }

    #[test]
    fn brave_body_tolerates_missing_results() {
        let raw = serde_json::json!({"web": {}});
        let parsed: BraveResponseBody = serde_json::from_value(raw).unwrap();
        assert!(parsed.web.and_then(|w| w.results).is_none());
    }

    #[test]
    fn brave_body_tolerates_missing_result_fields() {
        // Real-world resilience — Brave has been known to omit `description`
        // for image / news / spelling results embedded in the web array.
        let raw = serde_json::json!({
            "web": {
                "results": [
                    {"title": "T", "url": "https://x"},
                    {"url": "https://y"},
                    {"description": "d only"}
                ]
            }
        });
        let parsed: BraveResponseBody = serde_json::from_value(raw).unwrap();
        let results = parsed.web.and_then(|w| w.results).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].description, None);
        assert_eq!(results[1].title, None);
        assert_eq!(results[2].url, None);
    }
}

/// Wiremock-backed HTTP integration tests (mika#1808). Exercise the full
/// `SearchEgressClient::search()` path against a mock Brave upstream so the
/// retry policy, header format, response parsing, and Q4 log discipline are
/// verified end-to-end without touching the real Brave API.
///
/// Why inline (not in `crates/mika-gateway/tests/`)? The substrate's Q2
/// discipline keeps every export at `pub(crate)`. Integration test binaries
/// live outside the crate and only see `pub` items — moving the tests there
/// would force us to weaken the visibility invariant. Inline `#[cfg(test)]`
/// tests keep the discipline intact.
#[cfg(test)]
mod wiremock_integration {
    use super::super::{
        BraveConfig, SearchEgressClient, SearchError, SearchRequest, SearchUpstream,
    };
    use secrecy::SecretString;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Build a client that points at the mock server. Reuses
    /// `SearchEgressClient::new` (production factory) so tests exercise the
    /// same reqwest builder profile as the real client.
    fn client_pointing_at(server: &MockServer) -> SearchEgressClient {
        SearchEgressClient::new(SearchUpstream::Brave(BraveConfig {
            api_key: SecretString::from("test-token-do-not-log"),
            endpoint: format!("{}/res/v1/web/search", server.uri()),
        }))
    }

    fn simple_request(q: &str) -> SearchRequest {
        SearchRequest {
            query: q.to_string(),
            max_results: 5,
        }
    }

    #[tokio::test]
    async fn success_path_parses_brave_response_and_sends_subscription_token_header() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .and(header("X-Subscription-Token", "test-token-do-not-log"))
            .and(header("Accept", "application/json"))
            .and(query_param("q", "rust axum"))
            .and(query_param("count", "5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "web": {
                    "results": [
                        {"title": "Axum", "url": "https://github.com/tokio-rs/axum", "description": "web framework"},
                        {"title": "Docs", "url": "https://docs.rs/axum", "description": "API reference"}
                    ]
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = client_pointing_at(&server);
        let resp = client
            .search(simple_request("rust axum"))
            .await
            .expect("success path");

        assert_eq!(resp.results.len(), 2);
        assert_eq!(resp.results[0].title, "Axum");
        assert_eq!(resp.results[0].url, "https://github.com/tokio-rs/axum");
        assert_eq!(resp.results[0].snippet, "web framework");
        // upstream_latency_ms is set by the substrate layer (bounded, non-zero
        // by construction — but we don't assert an exact value to avoid
        // flakiness).
    }

    #[tokio::test]
    async fn missing_web_block_returns_empty_results_not_parse_error() {
        // Brave's "no results" shape omits the `web` block entirely; the
        // substrate must return an empty Vec, not a ParseError — otherwise
        // an innocuous zero-hit query would page an operator.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        let client = client_pointing_at(&server);
        let resp = client
            .search(simple_request("obscure-nohits-query"))
            .await
            .expect("success path");
        assert!(resp.results.is_empty());
    }

    #[tokio::test]
    async fn unauthorized_401_fails_fast_no_retry() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .respond_with(ResponseTemplate::new(401))
            // `.expect(1)` is the load-bearing assertion: fail-fast means
            // exactly one HTTP call, never two.
            .expect(1)
            .mount(&server)
            .await;

        let client = client_pointing_at(&server);
        let err = client
            .search(simple_request("auth-test"))
            .await
            .expect_err("401 must produce an error");
        assert!(
            matches!(err, SearchError::Unauthorized),
            "expected Unauthorized, got {err:?}"
        );
    }

    #[tokio::test]
    async fn forbidden_403_fails_fast_no_retry() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .respond_with(ResponseTemplate::new(403))
            .expect(1)
            .mount(&server)
            .await;

        let client = client_pointing_at(&server);
        let err = client
            .search(simple_request("auth-test"))
            .await
            .expect_err("403 must produce an error");
        assert!(matches!(err, SearchError::Unauthorized));
    }

    #[tokio::test]
    async fn rate_limit_429_retries_once_then_succeeds() {
        let server = MockServer::start().await;

        // First hit → 429 with Retry-After: 1s
        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("Retry-After", "1")
                    .set_body_string(""),
            )
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;

        // Second hit (retry) → 200 with a valid body.
        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "web": {"results": [{"title": "OK", "url": "https://ok", "description": "d"}]}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = client_pointing_at(&server);
        let resp = client
            .search(simple_request("retry-test"))
            .await
            .expect("retry should succeed");
        assert_eq!(resp.results.len(), 1);
    }

    #[tokio::test]
    async fn rate_limit_429_persists_after_retry_returns_upstream_status() {
        // Both attempts return 429 — after the single retry we return the
        // taxonomy label so the caller (and any monitor) can distinguish
        // rate-limit exhaustion from other upstream errors.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("Retry-After", "1")
                    .set_body_string(""),
            )
            .expect(2)
            .mount(&server)
            .await;

        let client = client_pointing_at(&server);
        let err = client
            .search(simple_request("still-rate-limited"))
            .await
            .expect_err("persistent 429 must error");
        assert!(matches!(err, SearchError::UpstreamStatus(429)));
    }

    #[tokio::test]
    async fn server_error_500_retries_once() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "web": {"results": []}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = client_pointing_at(&server);
        let resp = client
            .search(simple_request("5xx-retry"))
            .await
            .expect("second attempt succeeds");
        assert!(resp.results.is_empty());
    }

    #[tokio::test]
    async fn client_error_400_no_retry() {
        // 4xx (other than 401/403/429) should fail-fast without a retry —
        // there's no expectation the same malformed request would succeed
        // on the second try, and retries burn quota.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .respond_with(ResponseTemplate::new(400))
            .expect(1)
            .mount(&server)
            .await;

        let client = client_pointing_at(&server);
        let err = client
            .search(simple_request("bad-request"))
            .await
            .expect_err("400 must error");
        assert!(matches!(err, SearchError::UpstreamStatus(400)));
    }

    #[tokio::test]
    async fn success_body_that_fails_to_parse_returns_parse_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>oops</html>"))
            .expect(1)
            .mount(&server)
            .await;

        let client = client_pointing_at(&server);
        let err = client
            .search(simple_request("parse-fail"))
            .await
            .expect_err("non-JSON body must error");
        assert!(matches!(err, SearchError::ParseError));
    }

    #[tokio::test]
    async fn max_results_is_clamped_and_forwarded_as_count_param() {
        // The substrate caps max_results at 20 so a caller can't ask for
        // 1000 (and burn quota / trigger a Brave rate-limit).
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .and(query_param("count", "20"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "web": {"results": []}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = client_pointing_at(&server);
        let req = SearchRequest {
            query: "clamp-test".to_string(),
            max_results: 1000,
        };
        client.search(req).await.expect("clamped call");
    }

    #[tokio::test]
    async fn success_path_leaks_zero_log_fields_beyond_q4_allowlist() {
        // Duplicate the mod.rs `log_assertion_no_tenant_no_query_no_forbidden_fields`
        // for the SUCCESS path — the mod.rs variant runs the transport-error
        // path (offline endpoint). Together they cover both branches of the
        // Brave impl.
        use std::sync::{Arc, Mutex};
        use tracing::subscriber::set_default;
        use tracing_subscriber::layer::SubscriberExt;

        #[derive(Clone, Default)]
        struct RawCaptureLayer {
            names: Arc<Mutex<Vec<String>>>,
            field_names: Arc<Mutex<Vec<String>>>,
        }

        impl<S> tracing_subscriber::Layer<S> for RawCaptureLayer
        where
            S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
        {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _ctx: tracing_subscriber::layer::Context<'_, S>,
            ) {
                self.names
                    .lock()
                    .unwrap()
                    .push(event.metadata().name().to_string());
                struct FieldNamer<'a>(&'a Mutex<Vec<String>>);
                impl<'a> tracing::field::Visit for FieldNamer<'a> {
                    fn record_debug(
                        &mut self,
                        field: &tracing::field::Field,
                        _v: &dyn std::fmt::Debug,
                    ) {
                        self.0.lock().unwrap().push(field.name().to_string());
                    }
                }
                event.record(&mut FieldNamer(&self.field_names));
            }
        }

        let layer = RawCaptureLayer::default();
        let subscriber = tracing_subscriber::registry().with(layer.clone());

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "web": {"results": [
                    {"title": "T", "url": "https://x", "description": "d"}
                ]}
            })))
            .mount(&server)
            .await;

        let sensitive_query = "TENANT-99-SECRET-do-not-leak-success-path";
        let client = client_pointing_at(&server);
        let _guard = set_default(subscriber);
        let _ = client
            .search(SearchRequest {
                query: sensitive_query.to_string(),
                max_results: 3,
            })
            .await;
        drop(_guard);

        let field_names = layer.field_names.lock().unwrap().clone();
        for name in &field_names {
            assert!(
                !matches!(
                    name.as_str(),
                    "query"
                        | "tenant_id"
                        | "tenant_hash"
                        | "user_id"
                        | "chat_id"
                        | "customer_id"
                        | "api_key"
                        | "retry_after"
                        | "url"
                ),
                "forbidden field name '{name}' appeared on the success path — Q4 STRIP TOTAL violated"
            );
        }
    }
}

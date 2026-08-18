//! Egress search substrate — the ONLY code path in the platform that talks
//! to a search upstream (Brave, SearXNG-future, etc.).
//!
//! This module is the E1 keystone of milestone #1806 (search backend
//! egress-controlled). See `docs/plans/2026-08-18-1807-e1-egress-substrate-plan.md`
//! for the design tranchage, and `crates/mika-gateway/docs/egress-search.md`
//! for the architecture doc (AC5). All mika-spirit agents call this substrate
//! via `POST /internal/search` — never the upstream API directly.
//!
//! # Discipline
//!
//! **Q1 — Placement (sami-tranchée 2026-08-18):** module-in-gateway, not
//! separate service. Non-souverain deploy (no new Helm chart, no new K8s
//! service). Compensated by strict isolation (see Q2 below).
//!
//! **Q2 — Isolation (quadruple discipline, mirroring mika#1796 voice
//! testimony non-transit invariant):**
//!   1. **Marker types** — `SearchEgressClient` wraps `reqwest::Client`
//!      privately. No conversion from a general `reqwest::Client` is
//!      possible. Handlers accept only `&SearchEgressClient`.
//!   2. **Module visibility** — every export is `pub(crate)`. No
//!      cross-crate import possible.
//!   3. **CI lint** — `scripts/verify-egress-uniqueness.sh` grep-fails
//!      the build if any file OTHER than `crates/mika-gateway/src/egress_search*`
//!      references a search-upstream identifier (Brave/SearXNG URLs, etc.).
//!   4. **Runtime egress firewall (E4 scope, NOT in this PR)** —
//!      iptables/nft rules at the container level. See ticket #1810.
//!
//! **Q3 — Multi-tenant sharing (Prime 2026-07-19):** one shared
//! `SearchEgressClient` instance across every tenant. No per-tenant state
//! (session, cache, rate-limit counter). Centrality ≠ visibility; the
//! no-log invariant below prevents ex-post correlation.
//!
//! **Q4 — Instrumentation (sami STRIP TOTAL v1, 2026-08-18):** ZERO
//! tenant identifier of any kind — not raw, not hashed, not bucketed.
//! ZERO query content. Traces carry only `{upstream, latency_ms, status}`.
//! See `#tests::log_assertion` for enforcement.
//!
//! # Scope of E2 (mika#1808)
//!
//! Brave Search API client wired behind the E1 substrate. The concrete
//! HTTP call lives in [`brave`]; this module owns the marker types, the
//! shared client, the audit-event emitter, and the HTTP handler.

use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;

use crate::routes::AppState;

pub(crate) mod brave;

#[cfg(test)]
mod tests_e3_request_shape;

/// Default endpoint for Brave Search API. Overridable via
/// `MIKA_BRAVE_ENDPOINT` for E2 integration tests / self-hosted mirrors.
pub(crate) const DEFAULT_BRAVE_ENDPOINT: &str = "https://api.search.brave.com/res/v1/web/search";

/// Hard cap on any single search egress call (retry attempts included).
/// The `reqwest::Client` per-request timeout is smaller (see [`build_client`])
/// so the retry loop cannot exceed this budget.
pub(crate) const EGRESS_HARD_TIMEOUT_SECS: u64 = 5;

// -- Public (crate) surface --

/// Search egress client — the SINGLE code path that talks to a search upstream.
///
/// Constructed once at gateway startup (see `main.rs`) and shared across every
/// tenant via `Arc`. Q3 partagé no-log — see module doc.
///
/// The inner `reqwest::Client` is intentionally NOT exposed, and this type is
/// intentionally NOT `pub` (only `pub(crate)`). No other crate can import,
/// construct, or extract it — the sole entry point is the HTTP endpoint
/// registered by `handler` below.
pub(crate) struct SearchEgressClient {
    /// Private HTTP client. Do NOT expose. Do NOT return by reference.
    inner: reqwest::Client,
    upstream: SearchUpstream,
}

/// Marker enum bounding the upstream this client may reach. Enforced at
/// construction time — the enum has no runtime-mutable variant, so a
/// constructed `SearchEgressClient` cannot be re-pointed at a different
/// upstream after the fact.
pub(crate) enum SearchUpstream {
    /// Brave Search API. Wired for real network I/O in E2 (mika#1808).
    Brave(BraveConfig),
    // Future: SearXNG(SearXNGConfig) — E6 contingency only, if E4 fails.
}

impl SearchUpstream {
    /// Stable identifier used in Q4 instrumentation. NEVER include tenant/user
    /// context here.
    pub(crate) fn provider_name(&self) -> &'static str {
        match self {
            SearchUpstream::Brave(_) => "brave",
        }
    }
}

/// Brave Search API configuration. `api_key` is the `X-Subscription-Token`
/// value sent on every request (mika#1808 wires it into the concrete HTTP
/// call). `endpoint` overrides the canonical Brave URL for tests /
/// self-hosted mirrors.
pub(crate) struct BraveConfig {
    pub(crate) api_key: SecretString,
    pub(crate) endpoint: String,
}

/// Wire request from an in-container mika-spirit agent.
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct SearchRequest {
    /// User query. NEVER log this field. Never persist. Never emit as tracing
    /// attribute or Prometheus label. See Q4 STRIP TOTAL invariant.
    pub query: String,
    /// Number of results the agent wants back (upstream may cap).
    #[serde(default = "default_max_results")]
    pub max_results: usize,
}

fn default_max_results() -> usize {
    5
}

/// A single search result the substrate returns to the calling agent.
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Substrate-shaped response. The `upstream_latency_ms` field is the ONLY
/// side-channel exposed to the calling agent — it is bounded to a small
/// integer and carries no tenant-correlated bits (upstream response time
/// depends on the upstream, not the tenant).
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub upstream_latency_ms: u32,
}

/// Error taxonomy for `search()`. Each variant maps to a stable HTTP status
/// via `IntoResponse`. Q4 STRIP TOTAL: error variants NEVER carry query
/// content, tenant identifiers, or upstream response bodies — only status
/// classes and taxonomy labels the audit event can safely record.
#[derive(Debug, Error)]
pub(crate) enum SearchError {
    /// Reserved for future upstream variants that don't yet have a concrete
    /// client wired. Held in the taxonomy so downstream monitors relying on
    /// the `not_implemented` label stay stable when a new `SearchUpstream`
    /// arm is added ahead of its client.
    #[error("search upstream not yet implemented")]
    #[allow(dead_code)]
    NotImplemented,

    /// Upstream returned a non-success HTTP status. The wrapped code is the
    /// upstream's status, NOT a mika-side classification. Rate-limit
    /// exhaustion (429 after retry) and generic 5xx both land here.
    #[error("search upstream returned HTTP {0}")]
    UpstreamStatus(u16),

    /// Auth failure (401/403) from the upstream — invalid API key, revoked
    /// credentials. Fail-fast (no retry) so an operator can rotate the key
    /// without burning quota. Kept as its own taxonomy label so aggregate
    /// dashboards can page on unauthorized bursts distinctly from other
    /// upstream errors.
    #[error("search upstream rejected credentials")]
    Unauthorized,

    /// Transport-level failure talking to upstream (timeout, connect error,
    /// TLS failure). Retryable at the caller layer if desired; the substrate
    /// has already exhausted its internal retry budget when this returns.
    #[error("search upstream transport error")]
    Transport,

    /// Upstream returned a 2xx but the body couldn't be parsed into the
    /// expected schema. Distinct from `Transport` so operators can spot
    /// upstream schema drift (Brave shipping a breaking change) without
    /// misreading it as network flakiness.
    #[error("search upstream response could not be parsed")]
    ParseError,
}

impl SearchError {
    fn http_status(&self) -> StatusCode {
        match self {
            SearchError::NotImplemented => StatusCode::NOT_IMPLEMENTED,
            SearchError::UpstreamStatus(_) => StatusCode::BAD_GATEWAY,
            SearchError::Unauthorized => StatusCode::BAD_GATEWAY,
            SearchError::Transport => StatusCode::BAD_GATEWAY,
            SearchError::ParseError => StatusCode::BAD_GATEWAY,
        }
    }

    /// Machine-readable classification for Q4 tracing / audit event. Never
    /// includes tenant identifiers, query content, or upstream response bodies.
    fn tracing_status(&self) -> &'static str {
        match self {
            SearchError::NotImplemented => "not_implemented",
            SearchError::UpstreamStatus(_) => "upstream_error",
            SearchError::Unauthorized => "unauthorized",
            SearchError::Transport => "transport_error",
            SearchError::ParseError => "parse_error",
        }
    }
}

/// Build the shared `reqwest::Client` used by the substrate. Kept as a free
/// function so tests can construct a client with the same timeout profile
/// without paying for `SearchEgressClient::new`'s validation.
///
/// Q2 discipline: this is the ONE place in the module tree that constructs
/// the `reqwest::Client` reaching a search upstream. The CI lint
/// (`scripts/verify-egress-uniqueness.sh`) enforces at the file level; this
/// factory keeps the timeout budget in a single spot so per-retry math in
/// [`brave`] can reference it symbolically.
pub(crate) fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        // Per-request cap — must be < EGRESS_HARD_TIMEOUT_SECS so the retry
        // loop in `brave` can respect the overall budget.
        .timeout(std::time::Duration::from_secs(3))
        .connect_timeout(std::time::Duration::from_secs(2))
        .pool_max_idle_per_host(4)
        .pool_idle_timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("reqwest client build cannot fail with valid defaults")
}

impl SearchEgressClient {
    /// Construct the substrate client. Called once from `main.rs` when a
    /// search upstream is configured.
    pub(crate) fn new(upstream: SearchUpstream) -> Self {
        // Dedicated client — bounded connection pool + short timeouts.
        // Kept SEPARATE from the general-purpose `AppState.http_client` so
        // the CI-lint (verify-egress-uniqueness.sh) has an unambiguous
        // ownership claim: search-upstream HTTP work lives ONLY here.
        Self {
            inner: build_client(),
            upstream,
        }
    }

    /// Handle a search request. Instrumented per Q4 (upstream + status only —
    /// NEVER query content, NEVER tenant identifier).
    pub(crate) async fn search(&self, req: SearchRequest) -> Result<SearchResponse, SearchError> {
        let upstream = self.upstream.provider_name();

        // Q4 log — the ONE structured field that leaves this module. NO
        // tenant_hash, NO tenant_id, NO query, NO user identifier. Reviewer
        // check: `git diff` on this line — any additional field is a Q4
        // violation.
        info!(
            event = "search_requested",
            upstream = upstream,
            "search egress requested"
        );

        let start = std::time::Instant::now();

        let outcome: Result<SearchResponse, SearchError> = match &self.upstream {
            SearchUpstream::Brave(config) => {
                brave::execute_brave_search(&self.inner, config, req).await
            }
        };

        let latency_ms = u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX);
        emit_audit_event(upstream, latency_ms, outcome_status(&outcome));

        // Stamp the observed upstream latency onto the successful response so
        // callers see a bounded, tenant-independent side-channel — the
        // per-call latency depends on the upstream, not on the tenant.
        outcome.map(|mut resp| {
            resp.upstream_latency_ms = latency_ms;
            resp
        })
    }
}

/// Audit event shape (Q4 STRIP TOTAL): `{type, upstream, latency_ms, status}`.
/// Consumer contract: cm#99 audit_events emitter (E3/E4 ticket wires
/// persistence). This module writes a structured `tracing::info!` line that
/// downstream log-shippers can turn into an audit row — no `audit_events`
/// table access here.
fn emit_audit_event(upstream: &'static str, latency_ms: u32, status: &'static str) {
    info!(
        event = "search_egress",
        upstream = upstream,
        latency_ms = latency_ms,
        status = status,
        "search egress audit event"
    );
}

fn outcome_status<T>(outcome: &Result<T, SearchError>) -> &'static str {
    match outcome {
        Ok(_) => "ok",
        Err(err) => err.tracing_status(),
    }
}

// -- HTTP handler --

/// Handler for `POST /internal/search`. Auth is applied by the existing
/// `require_bearer_token` middleware layer registered in `routes.rs`.
///
/// Returns:
///   - 200 with `SearchResponse` on Ok
///   - 404 when no `SearchEgressClient` is configured (upstream not enabled)
///   - 501 with `SearchError::NotImplemented` for a future upstream variant
///     with no wired client
///   - 502 for upstream / transport / auth / parse errors
pub(crate) async fn handle_internal_search(
    State(state): State<AppState>,
    Json(payload): Json<SearchRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let client = match state.search_egress_client.as_ref() {
        Some(c) => c,
        None => {
            // No upstream configured — the substrate is compiled in but not
            // wired for this deploy. Q4-safe response: no config detail leaks.
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": "search_upstream_not_configured",
                })),
            );
        }
    };

    match client.search(payload).await {
        Ok(response) => (
            StatusCode::OK,
            Json(serde_json::to_value(response).expect("SearchResponse serializes")),
        ),
        Err(err) => {
            let status = err.http_status();
            // Response body carries only the taxonomy label — no query,
            // no tenant, no upstream response body. Q4 STRIP TOTAL.
            (
                status,
                Json(serde_json::json!({
                    "error": err.tracing_status(),
                })),
            )
        }
    }
}

/// Type-alias used by `AppState` and `main.rs` so the ownership story is
/// explicit: the client is Arc-shared across every request handler.
pub(crate) type SharedSearchEgressClient = Arc<SearchEgressClient>;

// -- Tests --

// E4 adversarial no-log test (mika#1810) — inline sibling of the `tests`
// module below. Kept in its own file so the format-stream harness stays
// separable from the field-visitor harness.
#[cfg(test)]
mod tests_e4_no_log;

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;
    use std::sync::Mutex;
    use tracing::subscriber::set_default;
    use tracing_subscriber::layer::SubscriberExt;

    /// Custom tracing layer that records every emitted event's field names
    /// + string values into a shared vector.
    ///
    /// Used to assert Q4 discipline on `search_requested` and
    /// `search_egress` events.
    #[derive(Clone, Default)]
    struct CapturingLayer {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
    }

    #[derive(Debug, Clone, Default)]
    struct CapturedEvent {
        target: String,
        event_name: Option<String>,
        upstream: Option<String>,
        status: Option<String>,
        latency_ms: Option<i64>,
        // Any field we do NOT expect — collected here so tests can assert
        // emptiness. (Only applied to events from our module's target.)
        forbidden_fields: Vec<String>,
        /// All field values (any source), serialized to a string. Used by
        /// the query-leakage assertion — reqwest/hyper emit their own events
        /// that could theoretically carry URL content (which contains the
        /// query). We assert the sensitive query bytes appear NOWHERE.
        all_field_values: Vec<String>,
    }

    impl<S> tracing_subscriber::Layer<S> for CapturingLayer
    where
        S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let target = event.metadata().target().to_string();
            let mut visitor = FieldVisitor::default();
            event.record(&mut visitor);
            self.events
                .lock()
                .unwrap()
                .push(visitor.into_captured(target));
        }
    }

    #[derive(Default)]
    struct FieldVisitor {
        event_name: Option<String>,
        upstream: Option<String>,
        status: Option<String>,
        latency_ms: Option<i64>,
        forbidden_fields: Vec<String>,
        all_field_values: Vec<String>,
    }

    impl FieldVisitor {
        fn into_captured(self, target: String) -> CapturedEvent {
            CapturedEvent {
                target,
                event_name: self.event_name,
                upstream: self.upstream,
                status: self.status,
                latency_ms: self.latency_ms,
                forbidden_fields: self.forbidden_fields,
                all_field_values: self.all_field_values,
            }
        }
    }

    // Q4 allowlist — fields we DO expect on the two egress events. Any
    // other field is a discipline violation and lands in `forbidden_fields`
    // for the assertion below to catch.
    const ALLOWED_FIELDS: &[&str] = &["event", "upstream", "status", "latency_ms", "message"];

    impl tracing::field::Visit for FieldVisitor {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            let name = field.name();
            self.all_field_values.push(value.to_string());
            match name {
                "event" => self.event_name = Some(value.to_string()),
                "upstream" => self.upstream = Some(value.to_string()),
                "status" => self.status = Some(value.to_string()),
                "message" => { /* message is the log msg text, allowed */ }
                other if ALLOWED_FIELDS.contains(&other) => {}
                other => self.forbidden_fields.push(other.to_string()),
            }
        }

        fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
            let name = field.name();
            self.all_field_values.push(value.to_string());
            match name {
                "latency_ms" => self.latency_ms = Some(value),
                other if ALLOWED_FIELDS.contains(&other) => {}
                other => self.forbidden_fields.push(other.to_string()),
            }
        }

        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            let name = field.name();
            self.all_field_values.push(value.to_string());
            match name {
                "latency_ms" => self.latency_ms = Some(value as i64),
                other if ALLOWED_FIELDS.contains(&other) => {}
                other => self.forbidden_fields.push(other.to_string()),
            }
        }

        fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
            let name = field.name();
            self.all_field_values.push(value.to_string());
            if !ALLOWED_FIELDS.contains(&name) {
                self.forbidden_fields.push(name.to_string());
            }
        }

        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            let name = field.name();
            let formatted = format!("{value:?}");
            self.all_field_values.push(formatted.clone());
            if name == "message" {
                // debug-formatted message text — allowed
                return;
            }
            if ALLOWED_FIELDS.contains(&name) {
                // fallback path — capture as string for the allowed fields
                match name {
                    "event" => self.event_name = Some(formatted),
                    "upstream" => self.upstream = Some(formatted),
                    "status" => self.status = Some(formatted),
                    _ => {}
                }
            } else {
                self.forbidden_fields.push(name.to_string());
            }
        }
    }

    fn brave_client_with_endpoint(endpoint: String) -> SearchEgressClient {
        SearchEgressClient::new(SearchUpstream::Brave(BraveConfig {
            api_key: SecretString::from("test-api-key"),
            endpoint,
        }))
    }

    fn brave_client_offline() -> SearchEgressClient {
        // Point at an RFC-5737 documentation address that will fail to
        // connect fast (or a bogus port on localhost). Used by tests that
        // only care about the log-emit / audit shape, not upstream success.
        // Any transport failure gives us a stable audit-event status without
        // opening a real network socket.
        brave_client_with_endpoint("http://127.0.0.1:1/does-not-exist".to_string())
    }

    #[test]
    fn provider_name_stable() {
        let up = SearchUpstream::Brave(BraveConfig {
            api_key: SecretString::from("k"),
            endpoint: DEFAULT_BRAVE_ENDPOINT.to_string(),
        });
        assert_eq!(up.provider_name(), "brave");
    }

    #[test]
    fn search_request_deserializes_with_default_max_results() {
        let raw = r#"{"query": "example"}"#;
        let parsed: SearchRequest = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.query, "example");
        assert_eq!(parsed.max_results, 5);
    }

    #[test]
    fn search_request_roundtrips() {
        let req = SearchRequest {
            query: "example".to_string(),
            max_results: 10,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: SearchRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.query, "example");
        assert_eq!(back.max_results, 10);
    }

    #[test]
    fn search_response_serializes_stable_shape() {
        let resp = SearchResponse {
            results: vec![SearchResult {
                title: "T".to_string(),
                url: "https://example.com".to_string(),
                snippet: "S".to_string(),
            }],
            upstream_latency_ms: 42,
        };
        let json = serde_json::to_value(&resp).unwrap();
        // Public wire contract — mika-agent side depends on these field names.
        assert!(json.get("results").is_some());
        assert!(json.get("upstream_latency_ms").is_some());
        let first = &json["results"][0];
        assert_eq!(first["title"], "T");
        assert_eq!(first["url"], "https://example.com");
        assert_eq!(first["snippet"], "S");
    }

    #[test]
    fn search_error_status_mapping_is_stable() {
        assert_eq!(
            SearchError::NotImplemented.http_status(),
            StatusCode::NOT_IMPLEMENTED
        );
        assert_eq!(
            SearchError::UpstreamStatus(500).http_status(),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(
            SearchError::Unauthorized.http_status(),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(
            SearchError::Transport.http_status(),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(
            SearchError::ParseError.http_status(),
            StatusCode::BAD_GATEWAY
        );
    }

    #[test]
    fn search_error_tracing_status_is_stable() {
        assert_eq!(
            SearchError::NotImplemented.tracing_status(),
            "not_implemented"
        );
        assert_eq!(
            SearchError::UpstreamStatus(500).tracing_status(),
            "upstream_error"
        );
        assert_eq!(SearchError::Unauthorized.tracing_status(), "unauthorized");
        assert_eq!(SearchError::Transport.tracing_status(), "transport_error");
        assert_eq!(SearchError::ParseError.tracing_status(), "parse_error");
    }

    /// E2 replaces the E1 `NotImplemented` check: `search()` now performs a
    /// real HTTP call. Pointing at an unreachable endpoint returns
    /// `Transport` (no retry can save a hard-refused connect).
    #[tokio::test]
    async fn search_returns_transport_error_on_unreachable_upstream() {
        let client = brave_client_offline();
        let req = SearchRequest {
            query: "sensitive-looking-query-that-must-not-leak".to_string(),
            max_results: 3,
        };
        let result = client.search(req).await;
        assert!(
            matches!(result, Err(SearchError::Transport)),
            "expected Transport error, got {result:?}"
        );
    }

    /// **Load-bearing Q4 discipline test.** Runs a search against an
    /// unreachable Brave endpoint (transport failure, deterministic), captures
    /// every tracing event emitted by `search()`, and asserts:
    ///   * Only two events are emitted: `search_requested` and `search_egress`.
    ///   * Each event has only the allowed fields (upstream, status,
    ///     latency_ms, message, event).
    ///   * No field named `tenant_hash`, `tenant_id`, `user_id`, `query`,
    ///     `chat_id`, `customer_id`, `api_key`, or `retry_after` appears
    ///     anywhere.
    ///   * The captured field values contain none of the query-content bytes.
    ///
    /// If this test fails, the Q4 STRIP TOTAL invariant is broken. Do NOT
    /// weaken the assertion — fix the emit-side.
    ///
    /// E2 note: the assertion works for BOTH success and failure paths — the
    /// Brave impl in `brave::execute_brave_search` must not add any log line
    /// that names query content, credentials, or tenant.
    #[tokio::test]
    async fn log_assertion_no_tenant_no_query_no_forbidden_fields() {
        let layer = CapturingLayer::default();
        let subscriber = tracing_subscriber::registry().with(layer.clone());

        let sensitive_query = "TENANT-42-SECRET-QUERY-do-not-leak-into-logs";
        let client = brave_client_offline();

        // `set_default` returns a per-thread guard that scopes the subscriber
        // to the current thread; the returned guard is dropped after the
        // async work completes.
        let _guard = set_default(subscriber);
        let req = SearchRequest {
            query: sensitive_query.to_string(),
            max_results: 7,
        };
        let _ = client.search(req).await;
        drop(_guard);

        let all_events = layer.events.lock().unwrap().clone();

        // Partition: events from our module (`crate::egress_search::*`) vs
        // events from third-party crates (reqwest, hyper, rustls, etc.).
        // The Q4 STRIP TOTAL invariant is scoped to OUR emissions — we don't
        // control third-party log lines. But the query-leakage assertion
        // below MUST cover every captured event regardless of source, since
        // a URL logged by reqwest/hyper would carry the query bytes.
        let our_target_prefix = "mika_gateway::egress_search";
        let our_events: Vec<_> = all_events
            .iter()
            .filter(|e| e.target.starts_with(our_target_prefix))
            .cloned()
            .collect();

        // Exactly two events from OUR module (search_requested + search_egress).
        // The Brave impl in `brave.rs` must never emit its own log line — the
        // audit event carries the taxonomy; the query never appears anywhere.
        assert_eq!(
            our_events.len(),
            2,
            "unexpected event count from our module (target prefix {our_target_prefix}); \
             events: {our_events:?}. The Brave impl MUST NOT add its own tracing calls — \
             Q4 STRIP TOTAL."
        );

        let event_names: Vec<_> = our_events
            .iter()
            .filter_map(|e| e.event_name.clone())
            .collect();
        assert!(event_names.contains(&"search_requested".to_string()));
        assert!(event_names.contains(&"search_egress".to_string()));

        // Every OUR event's `upstream` field is exactly "brave" — nothing else.
        for e in &our_events {
            assert_eq!(e.upstream.as_deref(), Some("brave"));
            assert!(
                e.forbidden_fields.is_empty(),
                "search() emitted forbidden fields: {:?}. Q4 STRIP TOTAL violated.",
                e.forbidden_fields
            );
        }

        // Load-bearing cross-source assertion: the sensitive query bytes
        // must NOT appear in ANY captured field value across ALL events
        // (our module + reqwest + hyper + etc). If reqwest ever ships a
        // release that logs the full request URL at INFO/DEBUG (unlikely
        // but possible), this test catches it.
        for e in &all_events {
            for value in &e.all_field_values {
                assert!(
                    !value.contains(sensitive_query),
                    "query bytes leaked into a tracing field value from target={}: {value}",
                    e.target
                );
            }
        }

        // The audit event carries a status (transport_error on this offline
        // path — the Brave impl retried and gave up).
        let audit = our_events
            .iter()
            .find(|e| e.event_name.as_deref() == Some("search_egress"))
            .expect("search_egress event present");
        assert_eq!(audit.status.as_deref(), Some("transport_error"));
        assert!(audit.latency_ms.is_some());
    }

    /// Marker-type discipline: BraveConfig cannot be constructed with a bare
    /// String api_key — the field is a `SecretString` and drop-zeroizes.
    #[test]
    fn brave_config_uses_secret_string() {
        let cfg = BraveConfig {
            api_key: SecretString::from("test"),
            endpoint: DEFAULT_BRAVE_ENDPOINT.to_string(),
        };
        // ExposeSecret is required to see the value — direct field access
        // would surface the SecretString wrapper, not the raw string. This
        // test is a shape guard, not a security assertion: if a future PR
        // changes `api_key` to `String`, this line stops compiling.
        assert_eq!(cfg.api_key.expose_secret(), "test");
    }
}

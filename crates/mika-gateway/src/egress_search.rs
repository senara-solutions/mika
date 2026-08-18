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
//! # Scope of E1 (this PR)
//!
//! Substrate + interface + discipline. `search()` returns
//! `SearchError::NotImplemented` for now — E2 (#1808) wires the concrete
//! Brave API client into the `SearchUpstream::Brave` arm. The gateway
//! endpoint proves the substrate is in place and correctly reachable
//! from mika-spirit; it does not perform network I/O against the
//! upstream yet.

use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;

use crate::routes::AppState;

/// Default endpoint for Brave Search API. Overridable via
/// `MIKA_BRAVE_ENDPOINT` for E2 integration tests / self-hosted mirrors.
pub(crate) const DEFAULT_BRAVE_ENDPOINT: &str = "https://api.search.brave.com/res/v1/web/search";

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
    #[allow(dead_code)] // E2 (#1808) wires the concrete network call
    inner: reqwest::Client,
    upstream: SearchUpstream,
}

/// Marker enum bounding the upstream this client may reach. Enforced at
/// construction time — the enum has no runtime-mutable variant, so a
/// constructed `SearchEgressClient` cannot be re-pointed at a different
/// upstream after the fact.
pub(crate) enum SearchUpstream {
    /// Brave Search API. Config carried but not read until E2 (#1808) wires
    /// the concrete HTTP call.
    Brave(#[allow(dead_code)] BraveConfig),
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
/// value sent on every request (E2 will wire this into the concrete HTTP
/// call). `endpoint` overrides the canonical Brave URL for tests /
/// self-hosted mirrors.
pub(crate) struct BraveConfig {
    #[allow(dead_code)] // E2 (#1808) — carries the credential for the concrete call
    pub(crate) api_key: SecretString,
    #[allow(dead_code)] // E2 (#1808) — resolved from settings or DEFAULT_BRAVE_ENDPOINT
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
/// via `IntoResponse`.
#[derive(Debug, Error)]
pub(crate) enum SearchError {
    /// E1 substrate is live but E2 (#1808) has not yet wired the concrete
    /// upstream call. Returned by every `search()` invocation in v1.
    #[error("search upstream not yet implemented (E2 #1808 pending)")]
    NotImplemented,

    /// Reserved for E2 — upstream returned a non-success HTTP status. The
    /// wrapped code is the upstream's status, NOT a mika-side classification.
    #[error("search upstream returned HTTP {0}")]
    #[allow(dead_code)] // E2 will populate on non-2xx upstream responses
    UpstreamStatus(u16),

    /// Reserved for E2 — transport-level failure talking to upstream.
    #[error("search upstream transport error")]
    #[allow(dead_code)] // E2 will populate on reqwest send/timeout errors
    Transport,
}

impl SearchError {
    fn http_status(&self) -> StatusCode {
        match self {
            SearchError::NotImplemented => StatusCode::NOT_IMPLEMENTED,
            SearchError::UpstreamStatus(_) => StatusCode::BAD_GATEWAY,
            SearchError::Transport => StatusCode::BAD_GATEWAY,
        }
    }

    /// Machine-readable classification for Q4 tracing / audit event. Never
    /// includes tenant identifiers, query content, or upstream response bodies.
    fn tracing_status(&self) -> &'static str {
        match self {
            SearchError::NotImplemented => "not_implemented",
            SearchError::UpstreamStatus(_) => "upstream_error",
            SearchError::Transport => "transport_error",
        }
    }
}

impl SearchEgressClient {
    /// Construct the substrate client. Called once from `main.rs` when a
    /// search upstream is configured.
    pub(crate) fn new(upstream: SearchUpstream) -> Self {
        // Dedicated client — bounded connection pool + short timeouts.
        // Kept SEPARATE from the general-purpose `AppState.http_client` so
        // the CI-lint (verify-egress-uniqueness.sh) has an unambiguous
        // ownership claim: search-upstream HTTP work lives ONLY here.
        let inner = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .connect_timeout(std::time::Duration::from_secs(2))
            .pool_max_idle_per_host(4)
            .pool_idle_timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("reqwest client build cannot fail with valid defaults");
        Self { inner, upstream }
    }

    /// Handle a search request. Instrumented per Q4 (upstream + status only —
    /// NEVER query content, NEVER tenant identifier).
    ///
    /// v1 always returns `SearchError::NotImplemented`. E2 (#1808) replaces
    /// the inner body with the concrete Brave call using `self.inner` and
    /// the credential in `self.upstream`.
    pub(crate) async fn search(&self, req: SearchRequest) -> Result<SearchResponse, SearchError> {
        // Consume `req` explicitly so the borrow of the query text is scoped
        // to the local frame and the field's name never appears in an event
        // attribute. Q4 STRIP TOTAL — the count is a bounded integer, not a
        // per-query fingerprint.
        let requested_results = req.max_results;
        drop(req);

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
        let outcome: Result<SearchResponse, SearchError> = Err(SearchError::NotImplemented);
        let latency_ms = u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX);

        emit_audit_event(upstream, latency_ms, outcome_status(&outcome));

        // requested_results is intentionally unused in v1; E2 will forward it.
        let _ = requested_results;

        outcome
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
///   - 200 with `SearchResponse` on Ok (E2 onward)
///   - 404 when no `SearchEgressClient` is configured (upstream not enabled)
///   - 501 with `SearchError::NotImplemented` while E1 substrate is live but
///     E2 hasn't wired the concrete upstream
///   - 502 for upstream / transport errors (E2 onward)
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
        event_name: Option<String>,
        upstream: Option<String>,
        status: Option<String>,
        latency_ms: Option<i64>,
        // Any field we do NOT expect — collected here so tests can assert
        // emptiness.
        forbidden_fields: Vec<String>,
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
            let mut visitor = FieldVisitor::default();
            event.record(&mut visitor);
            self.events.lock().unwrap().push(visitor.into_captured());
        }
    }

    #[derive(Default)]
    struct FieldVisitor {
        event_name: Option<String>,
        upstream: Option<String>,
        status: Option<String>,
        latency_ms: Option<i64>,
        forbidden_fields: Vec<String>,
    }

    impl FieldVisitor {
        fn into_captured(self) -> CapturedEvent {
            CapturedEvent {
                event_name: self.event_name,
                upstream: self.upstream,
                status: self.status,
                latency_ms: self.latency_ms,
                forbidden_fields: self.forbidden_fields,
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
            match name {
                "latency_ms" => self.latency_ms = Some(value),
                other if ALLOWED_FIELDS.contains(&other) => {}
                other => self.forbidden_fields.push(other.to_string()),
            }
        }

        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            let name = field.name();
            match name {
                "latency_ms" => self.latency_ms = Some(value as i64),
                other if ALLOWED_FIELDS.contains(&other) => {}
                other => self.forbidden_fields.push(other.to_string()),
            }
        }

        fn record_bool(&mut self, field: &tracing::field::Field, _value: bool) {
            let name = field.name();
            if !ALLOWED_FIELDS.contains(&name) {
                self.forbidden_fields.push(name.to_string());
            }
        }

        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            let name = field.name();
            if name == "message" {
                // debug-formatted message text — allowed
                return;
            }
            if ALLOWED_FIELDS.contains(&name) {
                // fallback path — capture as string for the allowed fields
                let formatted = format!("{value:?}");
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

    fn brave_client() -> SearchEgressClient {
        SearchEgressClient::new(SearchUpstream::Brave(BraveConfig {
            api_key: SecretString::from("test-api-key"),
            endpoint: DEFAULT_BRAVE_ENDPOINT.to_string(),
        }))
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
            SearchError::Transport.http_status(),
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
        assert_eq!(SearchError::Transport.tracing_status(), "transport_error");
    }

    #[tokio::test]
    async fn search_v1_returns_not_implemented() {
        let client = brave_client();
        let req = SearchRequest {
            query: "sensitive-looking-query-that-must-not-leak".to_string(),
            max_results: 3,
        };
        let result = client.search(req).await;
        assert!(matches!(result, Err(SearchError::NotImplemented)));
    }

    /// **Load-bearing Q4 discipline test.** Runs a search, captures every
    /// tracing event emitted by `search()`, and asserts:
    ///   * Only two events are emitted: `search_requested` and `search_egress`.
    ///   * Each event has only the allowed fields (upstream, status,
    ///     latency_ms, message, event).
    ///   * No field named `tenant_hash`, `tenant_id`, `user_id`, `query`,
    ///     `chat_id`, or `customer_id` appears anywhere.
    ///   * The captured field values contain none of the query-content bytes.
    ///
    /// If this test fails, the Q4 STRIP TOTAL invariant is broken. Do NOT
    /// weaken the assertion — fix the emit-side.
    #[tokio::test]
    async fn log_assertion_no_tenant_no_query_no_forbidden_fields() {
        let layer = CapturingLayer::default();
        let subscriber = tracing_subscriber::registry().with(layer.clone());

        let sensitive_query = "TENANT-42-SECRET-QUERY-do-not-leak-into-logs";
        let client = brave_client();

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

        let events = layer.events.lock().unwrap().clone();

        // Exactly two events (search_requested + search_egress).
        assert_eq!(
            events.len(),
            2,
            "unexpected event count from search(); events: {events:?}"
        );

        let event_names: Vec<_> = events.iter().filter_map(|e| e.event_name.clone()).collect();
        assert!(event_names.contains(&"search_requested".to_string()));
        assert!(event_names.contains(&"search_egress".to_string()));

        // Every event's `upstream` field is exactly "brave" — nothing else.
        for e in &events {
            assert_eq!(e.upstream.as_deref(), Some("brave"));
            assert!(
                e.forbidden_fields.is_empty(),
                "search() emitted forbidden fields: {:?}. Q4 STRIP TOTAL violated.",
                e.forbidden_fields
            );
        }

        // No captured field value contains the sensitive query bytes.
        for e in &events {
            let concat = format!(
                "{:?}{:?}{:?}",
                e.event_name.as_deref().unwrap_or_default(),
                e.upstream.as_deref().unwrap_or_default(),
                e.status.as_deref().unwrap_or_default(),
            );
            assert!(
                !concat.contains(sensitive_query),
                "query content leaked into structured log fields: {concat}"
            );
        }

        // The audit event carries a status.
        let audit = events
            .iter()
            .find(|e| e.event_name.as_deref() == Some("search_egress"))
            .expect("search_egress event present");
        assert_eq!(audit.status.as_deref(), Some("not_implemented"));
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

//! Egress fetch substrate — the ONLY code path in the platform that talks
//! to a GET-only URL upstream (currently: gouv.fr allowlist).
//!
//! This module is the second controlled-egress class (after
//! `egress_search`), landed in mika#1969 as a mirror of the E1 substrate
//! pattern established by milestone #1806. The mirror-module doctrine is
//! captured in `docs/solutions/best-practices/mirror-substrate-module-for-new-egress-class-2026-08-23.md`.
//! All mika-spirit agents call this substrate via `POST /internal/fetch` —
//! never the upstream host directly.
//!
//! # Discipline
//!
//! **Q1 — Placement (mika#1969, mirroring the 2026-08-18 sami-tranchée):**
//! module-in-gateway, not separate service. Non-souverain deploy (no new
//! Helm chart, no new K8s service). Compensated by strict isolation
//! (see Q2 below).
//!
//! **Q2 — Isolation (quadruple discipline, mirroring `egress_search`):**
//!   1. **Marker types** — `FetchEgressClient` wraps `reqwest::Client`
//!      privately. No conversion from a general `reqwest::Client` is
//!      possible. Handlers accept only `&FetchEgressClient`.
//!   2. **Module visibility** — every export is `pub(crate)`. No
//!      cross-crate import possible.
//!   3. **CI lint** — `scripts/verify-egress-uniqueness.sh` grep-fails
//!      the build if any file OTHER than the authorized module tree
//!      references a gouv.fr host substring (`service-public.fr`,
//!      `ants.gouv.fr`, `impots.gouv.fr`, `data.gouv.fr`).
//!   4. **Runtime egress firewall (E4 scope, cross-repo follow-up)** —
//!      iptables/nft rules at the container level. Tracked as a sibling
//!      mika-cloud issue extending mika#1810.
//!
//! **Q3 — Multi-tenant sharing:** one shared `FetchEgressClient` instance
//! across every tenant. No per-tenant state (session, cookie, cache).
//! Centrality ≠ visibility; the STRIP TOTAL invariant below prevents
//! ex-post correlation.
//!
//! **Q4 — Instrumentation (STRIP TOTAL, mirroring `egress_search`):**
//! ZERO tenant identifier of any kind — not raw, not hashed, not
//! bucketed. ZERO URL bytes (a `host_class` label collapses the matched
//! allowlist entry to a bounded four-value taxonomy). ZERO response
//! bytes. Traces carry only `{event, upstream, host_class, status,
//! latency_ms, message}`. See `#tests::log_assertion` for enforcement.
//!
//! # Scope
//!
//! GET-only lecture-seule (KTD6). No POST/PUT/DELETE. No cookies. No
//! JavaScript. No session state. The wire payload carries a `url`
//! field and nothing else — the type system enforces the incapacity
//! rather than promising the restraint.

use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;

use crate::routes::AppState;

pub(crate) mod gouv_fr;

/// Hard cap on any single fetch egress call (retry attempts included).
/// The per-request timeout is smaller (see [`build_client`]) so the
/// upstream call cannot exceed this budget. Currently the substrate
/// does zero in-substrate retries — the per-request 10 s timeout in
/// [`build_client`] is the sole cancellation mechanism — so this
/// constant is a spec anchor for the KTD5 budget, referenced by the
/// module doc-comment.
#[allow(dead_code)]
pub(crate) const FETCH_HARD_TIMEOUT_SECS: u64 = 15;

/// Response-body size cap. Gouv.fr pages routinely run 100–500 KB of
/// prose + markup; 1 MiB is comfortable headroom without opening a
/// memory-exhaustion vector. Enforced twice: `Content-Length` check
/// pre-body-pull, then `.bytes().await` length re-check for upstreams
/// that omit the header.
pub(crate) const MAX_FETCH_RESPONSE_BYTES: usize = 1_048_576;

/// Compile-time allowlist of upstream hosts. Any request whose parsed
/// URL host does NOT suffix-match one of these entries (with a dot or
/// start-of-string preceding the match) is rejected with
/// [`FetchError::HostNotAllowed`].
///
/// Extension is a code change + deploy — never a runtime knob (KTD2).
/// Same reasoning as `INTERNAL_REPOS` in `crates/mika-gateway/src/github.rs`:
/// the allowlist is security-adjacent, so operator-mutable env vars
/// would widen the security envelope past reviewer-mutable code.
pub(crate) const ALLOWED_HOSTS: &[&str] = &[
    "service-public.fr",
    "ants.gouv.fr",
    "impots.gouv.fr",
    "data.gouv.fr",
];

// -- Public (crate) surface --

/// Fetch egress client — the SINGLE code path that talks to a GET-only
/// URL upstream.
///
/// Constructed once at gateway startup (see `main.rs`) and shared across
/// every tenant via `Arc`. The inner `reqwest::Client` is intentionally
/// NOT exposed, and this type is intentionally NOT `pub` (only
/// `pub(crate)`). No other crate can import, construct, or extract it —
/// the sole entry point is the HTTP endpoint registered by
/// [`handle_internal_fetch`] below.
pub(crate) struct FetchEgressClient {
    /// Private HTTP client. Do NOT expose. Do NOT return by reference.
    inner: reqwest::Client,
    upstream: FetchUpstream,
}

/// Marker enum bounding the upstream this client may reach. Enforced at
/// construction time — the enum has no runtime-mutable variant, so a
/// constructed `FetchEgressClient` cannot be re-pointed at a different
/// upstream after the fact.
pub(crate) enum FetchUpstream {
    /// Gouv.fr allowlist bundle. Wired for real network I/O in
    /// [`gouv_fr::execute_gouv_fr_fetch`].
    GouvFr(GouvFrConfig),
    // Future: additional egress classes (webhook-fetch, DNS lookup) get
    // their own variants + own config structs. The mirror-module pattern
    // in `docs/solutions/best-practices/mirror-substrate-module-for-new-egress-class-2026-08-23.md`
    // captures when to add a sibling module vs extend the enum.
}

impl FetchUpstream {
    /// Stable identifier used in Q4 instrumentation. NEVER include
    /// tenant/user context here — this is a per-substrate-variant label,
    /// not a per-request bit.
    pub(crate) fn provider_name(&self) -> &'static str {
        match self {
            FetchUpstream::GouvFr(_) => "gouv_fr",
        }
    }
}

/// Configuration for the gouv.fr allowlist upstream. Currently empty —
/// no per-tenant fields, no runtime knobs. Kept as a named struct so
/// future config additions (per-host timeout overrides, etc.) are
/// namespaced under the variant instead of leaking into the enum.
pub(crate) struct GouvFrConfig {}

/// Wire request from an in-container mika-spirit agent.
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct FetchRequest {
    /// Target URL. NEVER log this field. Never persist. Never emit as
    /// tracing attribute or Prometheus label. See Q4 STRIP TOTAL
    /// invariant.
    pub url: String,
}

/// Substrate-shaped response returned to the calling agent. `bytes_read`
/// is a bounded side-channel (u32, capped at
/// [`MAX_FETCH_RESPONSE_BYTES`]) — like `upstream_latency_ms` in the
/// search substrate, it carries no per-tenant bits beyond what the
/// caller already chose.
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct FetchResponse {
    /// UTF-8 decoded body (lossy — see [`gouv_fr::execute_gouv_fr_fetch`]).
    /// text/plain or text/html, un-parsed.
    pub body: String,
    /// Upstream `Content-Type` header, un-parsed. Default
    /// `application/octet-stream` when the header is absent.
    pub content_type: String,
    /// Response body length in bytes (post-cap, pre-UTF-8-decode).
    pub bytes_read: u32,
}

/// Error taxonomy for [`FetchEgressClient::fetch`]. Each variant maps
/// to a stable HTTP status via [`FetchError::http_status`]. Q4 STRIP
/// TOTAL: error variants NEVER carry URL bytes, tenant identifiers, or
/// upstream response bodies — only status classes and taxonomy labels
/// the audit event can safely record.
#[derive(Debug, Error)]
pub(crate) enum FetchError {
    /// URL host is not on the compile-time allowlist. Security-taxonomy
    /// rejection — the caller MUST NOT be trusted to have chosen a
    /// legitimate host. Maps to HTTP 403 (distinct from the 502 shape
    /// upstream errors take, so aggregate monitors can page on
    /// allowlist bypass attempts separately).
    #[error("fetch upstream host not allowed")]
    HostNotAllowed,

    /// URL failed to parse, is missing a host, or uses a non-HTTPS
    /// scheme. Never carries the URL bytes.
    #[error("fetch URL is invalid")]
    InvalidUrl,

    /// Response body exceeded [`MAX_FETCH_RESPONSE_BYTES`]. Detected
    /// either at `Content-Length` header time or after `.bytes().await`
    /// re-check for headerless upstreams. Same label either way — the
    /// operator dashboard keys off "response_too_large", not "which
    /// detector".
    #[error("fetch response too large")]
    ResponseTooLarge,

    /// Upstream returned a non-2xx HTTP status. Wraps the upstream's
    /// status verbatim. Rate-limit (429) and generic 5xx both land
    /// here — the caller (LLM) can back off naturally.
    #[error("fetch upstream returned HTTP {0}")]
    UpstreamStatus(u16),

    /// Transport-level failure talking to upstream (connect error, TLS
    /// failure, DNS). Distinct from timeout so operators can spot
    /// upstream reachability regressions.
    #[error("fetch upstream transport error")]
    Transport,

    /// Upstream did not respond within the per-request timeout. Kept as
    /// its own label so dashboards can distinguish "upstream slow" from
    /// "upstream unreachable".
    #[error("fetch upstream timeout")]
    Timeout,
}

impl FetchError {
    /// Map to the HTTP status returned by [`handle_internal_fetch`].
    /// Consumer contract: the operator dashboard keys off these
    /// codes — do NOT rename without updating the dashboard config.
    pub(crate) fn http_status(&self) -> StatusCode {
        match self {
            // Security-taxonomy rejection — 403 is the load-bearing
            // distinction from the 502 shape used for upstream errors.
            // Operators paging on 403 spikes see allowlist bypass
            // attempts distinctly.
            FetchError::HostNotAllowed => StatusCode::FORBIDDEN,
            FetchError::InvalidUrl => StatusCode::BAD_REQUEST,
            FetchError::ResponseTooLarge => StatusCode::BAD_GATEWAY,
            FetchError::UpstreamStatus(_) => StatusCode::BAD_GATEWAY,
            FetchError::Transport => StatusCode::BAD_GATEWAY,
            FetchError::Timeout => StatusCode::GATEWAY_TIMEOUT,
        }
    }

    /// Machine-readable classification for Q4 tracing / audit event.
    /// Never includes URL bytes, tenant identifiers, or upstream
    /// response bodies.
    pub(crate) fn tracing_status(&self) -> &'static str {
        match self {
            FetchError::HostNotAllowed => "host_not_allowed",
            FetchError::InvalidUrl => "invalid_url",
            FetchError::ResponseTooLarge => "response_too_large",
            FetchError::UpstreamStatus(_) => "upstream_error",
            FetchError::Transport => "transport_error",
            FetchError::Timeout => "timeout",
        }
    }
}

/// Build the shared `reqwest::Client` used by the substrate. Kept as a
/// free function so tests can construct a client with the same timeout
/// profile without paying for [`FetchEgressClient::new`]'s validation.
///
/// Q2 discipline: this is the ONE place in the module tree that
/// constructs the `reqwest::Client` reaching a fetch upstream. The CI
/// lint (`scripts/verify-egress-uniqueness.sh`) enforces at the file
/// level; this factory keeps the timeout budget in one spot so
/// per-request math in [`gouv_fr`] can reference it symbolically.
///
/// Timeout budget (KTD5): per-request 10 s, hard cap
/// [`FETCH_HARD_TIMEOUT_SECS`] 15 s. Government sites can be slower
/// than a general web upstream (heavier assets, older origin infra);
/// the 15 s hard cap mirrors the `web_search` builtin's budget while
/// preserving headroom for one retry inside the hard cap. The
/// per-request timeout is the sole cancellation mechanism for the
/// upstream `.get(...)` call.
pub(crate) fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        // Per-request cap — must be < FETCH_HARD_TIMEOUT_SECS.
        .timeout(std::time::Duration::from_secs(10))
        .connect_timeout(std::time::Duration::from_secs(3))
        .pool_max_idle_per_host(4)
        .pool_idle_timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("reqwest client build cannot fail with valid defaults")
}

impl FetchEgressClient {
    /// Construct the substrate client. Called once from `main.rs` when
    /// the substrate is wired.
    pub(crate) fn new(upstream: FetchUpstream) -> Self {
        // Dedicated client — bounded connection pool + short timeouts.
        // Kept SEPARATE from the general-purpose `AppState.http_client`
        // and from `egress_search`'s client so the CI lint has an
        // unambiguous ownership claim per substrate variant.
        Self {
            inner: build_client(),
            upstream,
        }
    }

    /// Handle a fetch request. Instrumented per Q4 (upstream +
    /// host_class + status only — NEVER URL content, NEVER tenant
    /// identifier).
    pub(crate) async fn fetch(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
        let upstream = self.upstream.provider_name();

        // Q4 log — the two structured fields that leave this module on
        // the request side. NO tenant_hash, NO tenant_id, NO url, NO
        // user identifier. Reviewer check: `git diff` on this line —
        // any additional field is a Q4 violation.
        info!(
            event = "fetch_requested",
            upstream = upstream,
            "fetch egress requested"
        );

        let start = std::time::Instant::now();

        // Compute `host_class` BEFORE dispatch. When the URL fails to
        // parse or the host cannot be extracted, the label collapses
        // to `"unknown"` — the audit event still records the taxonomy
        // label. See KTD3 for why this is emitted from the substrate
        // shell rather than from the inner `execute_*_fetch` scope
        // (leaking upstream-side timing correlation).
        let host_class = classify_host(&req.url);

        let outcome: Result<FetchResponse, FetchError> = match &self.upstream {
            FetchUpstream::GouvFr(config) => {
                gouv_fr::execute_gouv_fr_fetch(&self.inner, config, req).await
            }
        };

        let latency_ms = u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX);
        emit_audit_event(upstream, host_class, latency_ms, outcome_status(&outcome));

        outcome
    }
}

/// Collapse a URL's host to one of the four bounded allowlist labels
/// (or `"unknown"` when parsing / host extraction fails). Never emits
/// the raw host bytes — this is the primary Q4 side-channel guard.
///
/// Extending [`ALLOWED_HOSTS`] requires extending this classifier;
/// tests below assert per-entry parity.
pub(crate) fn classify_host(url: &str) -> &'static str {
    let parsed = match url::Url::parse(url) {
        Ok(p) => p,
        Err(_) => return "unknown",
    };
    let host = match parsed.host_str() {
        Some(h) => h.to_lowercase(),
        None => return "unknown",
    };
    if host_matches_allowlist_entry(&host, "service-public.fr") {
        "service_public"
    } else if host_matches_allowlist_entry(&host, "ants.gouv.fr") {
        "ants"
    } else if host_matches_allowlist_entry(&host, "impots.gouv.fr") {
        "impots"
    } else if host_matches_allowlist_entry(&host, "data.gouv.fr") {
        "data_gouv"
    } else {
        "unknown"
    }
}

/// Suffix-match with a boundary guard: `host` matches `entry` when the
/// host equals the entry OR when the char immediately preceding the
/// suffix match is `.` — catching subdomains cleanly without
/// false-positive matches like `evilservice-public.fr` (the leading
/// char is neither `.` nor start-of-string).
///
/// Both inputs must already be lowercased.
pub(crate) fn host_matches_allowlist_entry(host: &str, entry: &str) -> bool {
    if host == entry {
        return true;
    }
    let Some(prefix_len) = host.len().checked_sub(entry.len()) else {
        return false;
    };
    if prefix_len == 0 {
        // Equal-length case handled by the `host == entry` branch.
        return false;
    }
    if !host.ends_with(entry) {
        return false;
    }
    // Boundary check: the char before the suffix match must be '.'.
    // We slice by byte index because `.` is a single-byte ASCII char
    // and `entry` is guaranteed ASCII by ALLOWED_HOSTS content.
    host.as_bytes().get(prefix_len - 1) == Some(&b'.')
}

/// Q4 audit event shape: `{event, upstream, host_class, latency_ms,
/// status}`. Consumer contract: cm audit_events emitter (E3/E4 ticket
/// wires persistence). This module writes a structured `tracing::info!`
/// line that downstream log-shippers can turn into an audit row — no
/// `audit_events` table access here.
fn emit_audit_event(
    upstream: &'static str,
    host_class: &'static str,
    latency_ms: u32,
    status: &'static str,
) {
    info!(
        event = "fetch_egress",
        upstream = upstream,
        host_class = host_class,
        latency_ms = latency_ms,
        status = status,
        "fetch egress audit event"
    );
}

fn outcome_status<T>(outcome: &Result<T, FetchError>) -> &'static str {
    match outcome {
        Ok(_) => "ok",
        Err(err) => err.tracing_status(),
    }
}

// -- HTTP handler --

/// Handler for `POST /internal/fetch`. Auth is applied by the existing
/// `require_bearer_token` middleware layer registered in `routes.rs`.
///
/// Returns:
///   - 200 with [`FetchResponse`] on Ok
///   - 404 when no [`FetchEgressClient`] is configured (substrate not
///     enabled)
///   - 400 for [`FetchError::InvalidUrl`]
///   - 403 for [`FetchError::HostNotAllowed`] (security taxonomy —
///     distinct from upstream error 502 so allowlist bypass attempts
///     are pageable)
///   - 502 for upstream / transport / response-too-large errors
///   - 504 for [`FetchError::Timeout`]
pub(crate) async fn handle_internal_fetch(
    State(state): State<AppState>,
    Json(payload): Json<FetchRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let client = match state.fetch_egress_client.as_ref() {
        Some(c) => c,
        None => {
            // No upstream configured — the substrate is compiled in
            // but not wired for this deploy. Q4-safe response: no
            // config detail leaks.
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": "fetch_upstream_not_configured",
                })),
            );
        }
    };

    match client.fetch(payload).await {
        Ok(response) => (
            StatusCode::OK,
            Json(serde_json::to_value(response).expect("FetchResponse serializes")),
        ),
        Err(err) => {
            let status = err.http_status();
            // Response body carries only the taxonomy label — no URL,
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

/// Type-alias used by `AppState` and `main.rs` so the ownership story
/// is explicit: the client is Arc-shared across every request handler.
pub(crate) type SharedFetchEgressClient = Arc<FetchEgressClient>;

// -- Tests --

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_name_stable() {
        let up = FetchUpstream::GouvFr(GouvFrConfig {});
        assert_eq!(up.provider_name(), "gouv_fr");
    }

    #[test]
    fn allowed_hosts_are_lowercase() {
        for host in ALLOWED_HOSTS {
            assert_eq!(
                *host,
                host.to_lowercase(),
                "ALLOWED_HOSTS entry {host:?} must be lowercase — matching in \
                 `classify_host` and `host_matches_allowlist_entry` is \
                 case-sensitive after lowercasing the input host"
            );
        }
    }

    #[test]
    fn allowed_hosts_contain_expected_four() {
        assert_eq!(ALLOWED_HOSTS.len(), 4);
        assert!(ALLOWED_HOSTS.contains(&"service-public.fr"));
        assert!(ALLOWED_HOSTS.contains(&"ants.gouv.fr"));
        assert!(ALLOWED_HOSTS.contains(&"impots.gouv.fr"));
        assert!(ALLOWED_HOSTS.contains(&"data.gouv.fr"));
    }

    #[test]
    fn fetch_error_status_mapping_is_stable() {
        // Operator dashboard keys off these HTTP status codes. Rename
        // requires updating the dashboard config.
        assert_eq!(
            FetchError::HostNotAllowed.http_status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            FetchError::InvalidUrl.http_status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            FetchError::ResponseTooLarge.http_status(),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(
            FetchError::UpstreamStatus(500).http_status(),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(FetchError::Transport.http_status(), StatusCode::BAD_GATEWAY);
        assert_eq!(
            FetchError::Timeout.http_status(),
            StatusCode::GATEWAY_TIMEOUT
        );
    }

    #[test]
    fn fetch_error_tracing_status_is_stable() {
        assert_eq!(
            FetchError::HostNotAllowed.tracing_status(),
            "host_not_allowed"
        );
        assert_eq!(FetchError::InvalidUrl.tracing_status(), "invalid_url");
        assert_eq!(
            FetchError::ResponseTooLarge.tracing_status(),
            "response_too_large"
        );
        assert_eq!(
            FetchError::UpstreamStatus(500).tracing_status(),
            "upstream_error"
        );
        assert_eq!(FetchError::Transport.tracing_status(), "transport_error");
        assert_eq!(FetchError::Timeout.tracing_status(), "timeout");
    }

    #[test]
    fn classify_host_maps_each_allowlist_entry_to_label() {
        assert_eq!(
            classify_host("https://service-public.fr/"),
            "service_public"
        );
        assert_eq!(
            classify_host("https://www.service-public.fr/page"),
            "service_public"
        );
        assert_eq!(classify_host("https://ants.gouv.fr/foo"), "ants");
        assert_eq!(
            classify_host("https://immatriculation.ants.gouv.fr/x"),
            "ants"
        );
        assert_eq!(classify_host("https://impots.gouv.fr/portal"), "impots");
        assert_eq!(
            classify_host("https://www.impots.gouv.fr/portal/y"),
            "impots"
        );
        assert_eq!(classify_host("https://data.gouv.fr/dataset"), "data_gouv");
        assert_eq!(classify_host("https://www.data.gouv.fr/z"), "data_gouv");
    }

    #[test]
    fn classify_host_returns_unknown_on_non_allowlisted() {
        assert_eq!(classify_host("https://example.com/"), "unknown");
        assert_eq!(classify_host("https://evilservice-public.fr/"), "unknown");
        assert_eq!(classify_host("https://google.com/"), "unknown");
    }

    #[test]
    fn classify_host_returns_unknown_on_unparseable_url() {
        assert_eq!(classify_host("not a url"), "unknown");
        assert_eq!(classify_host(""), "unknown");
    }

    #[test]
    fn host_matches_allowlist_entry_exact() {
        assert!(host_matches_allowlist_entry(
            "service-public.fr",
            "service-public.fr"
        ));
        assert!(host_matches_allowlist_entry("ants.gouv.fr", "ants.gouv.fr"));
    }

    #[test]
    fn host_matches_allowlist_entry_subdomain() {
        assert!(host_matches_allowlist_entry(
            "www.service-public.fr",
            "service-public.fr"
        ));
        assert!(host_matches_allowlist_entry(
            "immatriculation.ants.gouv.fr",
            "ants.gouv.fr"
        ));
    }

    #[test]
    fn host_matches_allowlist_entry_rejects_prefix_lookalike() {
        // The load-bearing guard: naive `.ends_with()` would accept
        // "evilservice-public.fr" as matching "service-public.fr".
        // The boundary check requires the char immediately before the
        // suffix to be '.'.
        assert!(!host_matches_allowlist_entry(
            "evilservice-public.fr",
            "service-public.fr"
        ));
        assert!(!host_matches_allowlist_entry(
            "foo-ants.gouv.fr",
            "ants.gouv.fr"
        ));
    }

    #[test]
    fn host_matches_allowlist_entry_rejects_unrelated() {
        assert!(!host_matches_allowlist_entry(
            "example.com",
            "service-public.fr"
        ));
        assert!(!host_matches_allowlist_entry(
            "service-public.frx",
            "service-public.fr"
        ));
    }

    #[test]
    fn fetch_request_deserializes() {
        let raw = r#"{"url": "https://service-public.fr/"}"#;
        let parsed: FetchRequest = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.url, "https://service-public.fr/");
    }

    #[test]
    fn fetch_response_serializes_stable_shape() {
        let resp = FetchResponse {
            body: "hello".to_string(),
            content_type: "text/html".to_string(),
            bytes_read: 5,
        };
        let json = serde_json::to_value(&resp).unwrap();
        // Public wire contract — mika-agent side depends on these
        // field names.
        assert_eq!(json["body"], "hello");
        assert_eq!(json["content_type"], "text/html");
        assert_eq!(json["bytes_read"], 5);
    }
}

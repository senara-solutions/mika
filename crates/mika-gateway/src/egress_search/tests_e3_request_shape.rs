//! E3 (mika#1809) — build-time invariant: dé-liage identité↔requête.
//!
//! Two wiremock-backed structural tests that assert the substrate's outgoing
//! request carries NO tenant / agent / user identifier — no header, no query
//! param, no body field could re-tie a request to a mika tenant.
//!
//! Companion to `mod.rs::tests::log_assertion_no_tenant_no_query_no_forbidden_fields`
//! (LOG side). Together the three tests form the full E3 REQUEST + LOG
//! invariant coverage.
//!
//! # Why here (not in `crates/mika-gateway/tests/`)
//!
//! The substrate keeps every export at `pub(crate)` (Q2 discipline). Moving
//! these tests to a separate integration crate would force weakening the
//! visibility invariant. Inline `#[cfg(test)]` keeps the discipline intact,
//! matching the pattern used by `brave::wiremock_integration`.
//!
//! # Threat model
//!
//! See [`../../docs/egress-search-threat-model.md`](../../docs/egress-search-threat-model.md).
//!
//! The tests below are the load-bearing enforcement of the LIAGE half of
//! Prime's 2026-07-19 invariant ("zéro liage requête↔identité + zéro
//! rétention"). If either test fails, the substrate has grown a per-tenant
//! attribution surface — do NOT weaken the assertion, fix the emit-side.

use secrecy::SecretString;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{BraveConfig, SearchEgressClient, SearchRequest, SearchUpstream};

/// Header names the E3 invariant permits on an outgoing Brave request.
///
/// Matching is lowercase (HTTP headers are case-insensitive; wiremock
/// normalizes to lowercase). Anything OUTSIDE this set is a discipline
/// violation.
///
/// - `x-subscription-token` — the SHARED Brave API key (see § API key in
///   threat model; loaded once from `MIKA_BRAVE_API_KEY`, never per-tenant).
/// - `accept` — content negotiation, constant.
/// - `host` — set by reqwest from the URL, constant per deploy.
/// - `user-agent` — reqwest's default (`reqwest/<version>`), constant
///   across the fleet for a given gateway build. See § User-Agent.
/// - `accept-encoding` — reqwest's transport-layer negotiation, constant.
const ALLOWED_HEADER_NAMES: &[&str] = &[
    "x-subscription-token",
    "accept",
    "host",
    "user-agent",
    "accept-encoding",
];

/// Query param names the E3 invariant permits on an outgoing Brave request.
///
/// - `q` — the caller-supplied query string (content-layer concern, out
///   of E3 scope; see threat model § Query content leak).
/// - `count` — bounded integer `[1, 20]`, not identifying.
const ALLOWED_QUERY_PARAM_NAMES: &[&str] = &["q", "count"];

/// Forbidden header substring matches — any header whose lowercase name
/// contains one of these substrings triggers a hard fail regardless of
/// whether it appeared in the allowlist above. Belt-and-suspenders: the
/// allowlist is exact-match; this list catches variants like
/// `x-mika-tenant-id` or `x-request-user`.
const FORBIDDEN_HEADER_SUBSTRINGS: &[&str] = &[
    "tenant",
    "user",
    "agent",
    "customer",
    "session",
    "trace",
    "request-id",
    "cookie",
    "authorization", // shared auth travels in x-subscription-token
    "correlation",
    "client-id",
];

/// Forbidden query-param substring matches.
const FORBIDDEN_QUERY_SUBSTRINGS: &[&str] = &[
    "tenant",
    "user",
    "agent",
    "customer",
    "session",
    "trace",
    "request_id",
    "requestid",
    "correlation",
    "client_id",
    "clientid",
];

fn brave_client_pointing_at(server: &MockServer) -> SearchEgressClient {
    SearchEgressClient::new(SearchUpstream::Brave(BraveConfig {
        api_key: SecretString::from("e3-test-shared-token"),
        endpoint: format!("{}/res/v1/web/search", server.uri()),
    }))
}

fn simple_request(q: &str, max_results: usize) -> SearchRequest {
    SearchRequest {
        query: q.to_string(),
        max_results,
    }
}

fn empty_brave_ok_body() -> serde_json::Value {
    serde_json::json!({ "web": { "results": [] } })
}

/// **E3 LOAD-BEARING (headers).** Fire a real request through the substrate
/// against a mock Brave upstream, capture the recorded HTTP request via
/// `MockServer::received_requests()`, and assert the HEADER SET is exactly:
/// `{x-subscription-token, accept, host, user-agent, accept-encoding}`.
///
/// Any additional header — `x-user-*`, `x-tenant-*`, `x-agent-*`,
/// `x-customer-*`, `authorization`, `cookie`, `x-request-id`, `x-trace-id`,
/// or any name whose lowercase form contains an identity-shaped substring —
/// fails the assertion.
///
/// Do NOT weaken this test. If the substrate ever needs to send an additional
/// header, extend `ALLOWED_HEADER_NAMES` explicitly AND justify the addition
/// in the threat model doc (`crates/mika-gateway/docs/egress-search-threat-model.md`)
/// AND get sami sign-off (e3-lens review). The default answer is "no".
#[tokio::test]
async fn outgoing_headers_are_shared_only() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/res/v1/web/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(empty_brave_ok_body()))
        .expect(1)
        .mount(&server)
        .await;

    let client = brave_client_pointing_at(&server);
    let _ = client
        .search(simple_request("e3 header shape test", 5))
        .await
        .expect("mock returns 200");

    let received = server
        .received_requests()
        .await
        .expect("wiremock records requests");
    assert_eq!(
        received.len(),
        1,
        "expected exactly one request to Brave (E3 header assertion — retries would mask a header leak)"
    );
    let request = &received[0];

    // Collect lowercase header names for comparison.
    let observed: Vec<String> = request
        .headers
        .iter()
        .map(|(name, _values)| name.as_str().to_ascii_lowercase())
        .collect();

    // (a) Forbidden-substring check — catches variants like `x-mika-tenant-id`.
    for name in &observed {
        for forbidden in FORBIDDEN_HEADER_SUBSTRINGS {
            assert!(
                !name.contains(forbidden),
                "E3 VIOLATION (headers): outgoing header '{name}' contains forbidden substring '{forbidden}'. \
                 The substrate must not carry any per-tenant / per-user / per-agent attribution. \
                 See crates/mika-gateway/docs/egress-search-threat-model.md § Headers."
            );
        }
    }

    // (b) Strict allowlist check — every observed header MUST be in the
    // allowlist. If a future reqwest release adds a new default header,
    // this assertion catches it and forces a threat-model review before the
    // new header ships to Brave.
    for name in &observed {
        assert!(
            ALLOWED_HEADER_NAMES.contains(&name.as_str()),
            "E3 VIOLATION (headers): outgoing header '{name}' is not on the E3 allowlist \
             {ALLOWED_HEADER_NAMES:?}. If this header is legitimate (e.g. reqwest added a new \
             transport-layer default), extend ALLOWED_HEADER_NAMES AND document the addition in \
             crates/mika-gateway/docs/egress-search-threat-model.md § Headers AND get sami sign-off. \
             Observed headers: {observed:?}"
        );
    }

    // (c) The auth header MUST be exactly the shared token. If any code path
    // ever mutates the token per-request, this catches it. The value here is
    // the SAME across every tenant — the shared MIKA_BRAVE_API_KEY.
    let auth_values: Vec<&str> = request
        .headers
        .get_all("x-subscription-token")
        .iter()
        .map(|v| v.to_str().unwrap_or("<non-utf8>"))
        .collect();
    assert_eq!(
        auth_values,
        vec!["e3-test-shared-token"],
        "E3 VIOLATION: x-subscription-token value must be the shared MIKA_BRAVE_API_KEY, \
         verbatim. Observed: {auth_values:?}"
    );

    // (d) User-Agent, when present, MUST be a fixed shape (reqwest's default
    // starts with the literal `reqwest/`). This catches a hypothetical future
    // PR that sets a per-tenant User-Agent — the test would fail because the
    // observed value would not start with `reqwest/`.
    let ua_values: Vec<String> = request
        .headers
        .get_all("user-agent")
        .iter()
        .map(|v| v.to_str().unwrap_or("<non-utf8>").to_string())
        .collect();
    for ua in &ua_values {
        assert!(
            ua.starts_with("reqwest/"),
            "E3 VIOLATION (User-Agent): outgoing User-Agent '{ua}' is not the reqwest default. \
             If a custom User-Agent is intentional, it MUST be a fixed string (e.g. \
             `mika-gateway/<version>`) — NEVER per-tenant / per-agent / per-user. Update \
             this assertion AND document in the threat model."
        );
    }
}

/// **E3 LOAD-BEARING (query params).** Fire several requests through the
/// substrate with a variety of inputs — including malicious-looking ones that
/// try to smuggle an identifier via the caller-controlled fields — and assert
/// the outgoing URL query string contains ONLY the whitelisted param names
/// `{q, count}` on every call.
///
/// This is the LIAGE guarantee at the query-string layer: no `user`,
/// `tenant`, `agent_id`, `session_id`, `trace_id`, `client_id`, etc. can
/// ever appear as a distinct param. If a future PR adds one — even for a
/// legitimate observability reason — this test fails.
#[tokio::test]
async fn query_params_carry_no_identifier() {
    let server = MockServer::start().await;

    // Accept every GET the client sends; we assert on the *received* URLs,
    // not on the mock's own matcher discipline.
    Mock::given(method("GET"))
        .and(path("/res/v1/web/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(empty_brave_ok_body()))
        .mount(&server)
        .await;

    // Variety of inputs — including a caller trying to smuggle a
    // per-tenant identifier via the query text (which is content-layer,
    // not protocol-layer — see threat model § Query content leak). The
    // substrate must not treat these as additional params.
    let inputs = vec![
        simple_request("plain query", 5),
        simple_request("q=nested&user=vincent&tenant=42", 1), // caller tries URL-injection
        simple_request("clamp-test", 1_000),                  // exercise the clamp path
        simple_request("", 20),                               // edge: empty query, max count
        simple_request("unicode-\u{1F600}-emoji", 7),         // exercise URL encoding
    ];

    let client = brave_client_pointing_at(&server);
    for req in inputs {
        let expected_max = req.max_results;
        client
            .search(req)
            .await
            .expect("mock returns 200 for all queries");

        let received = server
            .received_requests()
            .await
            .expect("wiremock records requests");
        let request = received.last().expect("at least one recorded request");

        // Parse the URL and enumerate its query params. `url::Url` is a
        // dev-dep transitively via reqwest; we go through it because
        // wiremock's captured URL is the actual reqwest-serialized form
        // (already percent-encoded), which we need to inspect verbatim.
        let url = &request.url;
        let observed_pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let observed_names: Vec<String> = observed_pairs
            .iter()
            .map(|(k, _)| k.to_ascii_lowercase())
            .collect();

        // (a) Forbidden-substring check.
        for name in &observed_names {
            for forbidden in FORBIDDEN_QUERY_SUBSTRINGS {
                assert!(
                    !name.contains(forbidden),
                    "E3 VIOLATION (query params): param name '{name}' contains forbidden \
                     substring '{forbidden}'. See crates/mika-gateway/docs/egress-search-threat-model.md \
                     § Query params. Full observed pairs: {observed_pairs:?}"
                );
            }
        }

        // (b) Strict allowlist check — every observed param MUST be `q` or
        // `count`. Nothing else.
        for name in &observed_names {
            assert!(
                ALLOWED_QUERY_PARAM_NAMES.contains(&name.as_str()),
                "E3 VIOLATION (query params): param '{name}' is not on the E3 allowlist \
                 {ALLOWED_QUERY_PARAM_NAMES:?}. If it is legitimate, extend \
                 ALLOWED_QUERY_PARAM_NAMES AND document in the threat model AND get sami \
                 sign-off. Observed pairs: {observed_pairs:?}"
            );
        }

        // (c) `count` must be present exactly once and its value must be a
        // clamped integer in `[1, 20]`. The clamp is a defense — a caller
        // asking for 1000 must not be able to send a raw 1000 to Brave.
        let count_values: Vec<&str> = observed_pairs
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("count"))
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(
            count_values.len(),
            1,
            "E3: exactly one `count` param expected, got {count_values:?}"
        );
        let count_value: u32 = count_values[0]
            .parse()
            .expect("count param must be an integer");
        assert!(
            (1..=20).contains(&count_value),
            "E3: count value {count_value} out of clamped range [1, 20]. Expected clamp of \
             requested {expected_max}."
        );

        // (d) `q` must be present exactly once. The value is the raw caller
        // query (content-layer — E3 does not sanitize content). We don't
        // assert on the value here; that's a §Query-content-leak concern
        // covered doctrinally, not by the substrate.
        let q_values: Vec<&str> = observed_pairs
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("q"))
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(
            q_values.len(),
            1,
            "E3: exactly one `q` param expected, got {q_values:?}"
        );
    }
}

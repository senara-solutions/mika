//! Gouv.fr allowlist client — the concrete upstream wiring behind the
//! [`crate::egress_fetch`] substrate.
//!
//! # Discipline (inherit from `super`)
//!
//! - **Q2 uniqueness:** the `reqwest::Client` used here is passed in
//!   from [`super::FetchEgressClient`]; this file NEVER constructs its
//!   own client and NEVER holds a reference to a fetch upstream
//!   identifier outside the allowlisted egress module tree
//!   (`scripts/verify-egress-uniqueness.sh`).
//! - **Q4 STRIP TOTAL:** this file emits ZERO tracing calls. The audit
//!   event is emitted once by the parent module. Adding a `debug!` /
//!   `info!` / `warn!` line here — even one that pretends to be
//!   Q4-safe — is a discipline violation. The log-absence test in
//!   `super::tests` counts the emitted-event set to catch drift.
//! - **HTTPS-only:** non-HTTPS schemes are rejected with
//!   [`FetchError::InvalidUrl`] before the upstream call — refusing
//!   HTTP prevents downgrade probes against origins that also serve
//!   port 80.
//! - **No response body in errors:** upstream response bodies are read
//!   only to populate the successful [`FetchResponse`]. Failure paths
//!   discard the body — it cannot end up in a `Debug` impl or error
//!   string.
//!
//! # Method
//!
//! GET only, by API construction (KTD6). The wire payload has no
//! `method` field, and this file hardcodes `.get(&url)`. There is no
//! code path from the caller to a non-GET method — the type system
//! enforces the incapacity rather than promising the restraint.

use super::{
    ALLOWED_HOSTS, FetchError, FetchRequest, FetchResponse, GouvFrConfig, MAX_FETCH_RESPONSE_BYTES,
    host_matches_allowlist_entry,
};

/// Execute a single GET against a gouv.fr-allowlisted URL.
///
/// Returns a [`FetchResponse`] on success (`content_type` copied from
/// the upstream `Content-Type` header, defaulting to
/// `application/octet-stream`; `body` UTF-8 decoded with lossy
/// replacement; `bytes_read` the post-cap body length).
///
/// The `req` is consumed so the caller cannot accidentally re-log the
/// URL after we've handed it to reqwest.
pub(crate) async fn execute_gouv_fr_fetch(
    client: &reqwest::Client,
    _config: &GouvFrConfig,
    req: FetchRequest,
) -> Result<FetchResponse, FetchError> {
    let FetchRequest { url } = req;

    // Parse and validate the URL — no borrowed slice of the raw string
    // survives past this scope. Never log the parsed URL either.
    let parsed = url::Url::parse(&url).map_err(|_| FetchError::InvalidUrl)?;

    // HTTPS-only: the four gouv.fr sites all serve HTTPS; refusing HTTP
    // prevents downgrade probes and keeps the substrate's threat model
    // narrow.
    if parsed.scheme() != "https" {
        return Err(FetchError::InvalidUrl);
    }

    let host = parsed
        .host_str()
        .ok_or(FetchError::InvalidUrl)?
        .to_lowercase();

    // Allowlist enforcement — suffix-match with a `.` boundary guard
    // (see `super::host_matches_allowlist_entry`). The load-bearing
    // regression against `evilservice-public.fr` passing a naive
    // `.ends_with()` is covered by that helper's tests.
    let allowed = ALLOWED_HOSTS
        .iter()
        .any(|entry| host_matches_allowlist_entry(&host, entry));
    if !allowed {
        return Err(FetchError::HostNotAllowed);
    }

    // GET only, by API construction (KTD6). The per-request timeout on
    // the shared `reqwest::Client` (10 s, see `super::build_client`) is
    // the sole cancellation mechanism for this call.
    let response = client.get(&url).send().await.map_err(|e| {
        // Deliberately drop the error's Debug/Display — reqwest's Debug
        // impl can include the URL. We keep only the taxonomy label,
        // separating `Timeout` from `Transport` so operators can page
        // on the two failure classes distinctly.
        if e.is_timeout() {
            FetchError::Timeout
        } else {
            FetchError::Transport
        }
    })?;

    let status = response.status();
    if !status.is_success() {
        return Err(FetchError::UpstreamStatus(status.as_u16()));
    }

    // Extract Content-Type BEFORE moving the response into `.bytes()`.
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    // Content-Length gate — reject before pulling the body.
    if let Some(len) = response.content_length()
        && len as usize > MAX_FETCH_RESPONSE_BYTES
    {
        return Err(FetchError::ResponseTooLarge);
    }

    let bytes = response.bytes().await.map_err(|_| FetchError::Transport)?;

    // Defensive re-check — some upstreams omit Content-Length or lie.
    if bytes.len() > MAX_FETCH_RESPONSE_BYTES {
        return Err(FetchError::ResponseTooLarge);
    }

    let bytes_read = u32::try_from(bytes.len()).unwrap_or(u32::MAX);

    // Lossy UTF-8 decode — gouv.fr pages are UTF-8 in practice, but
    // lossy replacement protects against odd origin encodings without
    // a hard fail. `content_type` still carries the raw header so a
    // caller that needs strict decode can inspect it.
    let body = String::from_utf8_lossy(&bytes).into_owned();

    Ok(FetchResponse {
        body,
        content_type,
        bytes_read,
    })
}

// -- Tests --

#[cfg(test)]
mod tests {
    use super::super::{FetchEgressClient, FetchUpstream};
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing::subscriber::set_default;
    use tracing_subscriber::layer::SubscriberExt;

    // Wiremock-adapter helper: bypass the allowlist to exercise
    // `execute_gouv_fr_fetch`'s downstream branches (size caps,
    // Content-Type extraction, upstream status mapping) against a
    // 127.0.0.1 mock. The real substrate binds to the URL bytes
    // directly (no DNS injection surface), so we exercise the
    // parse/error branches via `execute_gouv_fr_fetch` directly and
    // the size/status branches via this helper. Real allowlist
    // enforcement is exercised by the `super::tests::classify_host_*`
    // + `host_matches_allowlist_entry_*` tests and the direct-parse
    // tests below.
    async fn exec_bypass_allowlist(
        client: &reqwest::Client,
        url: String,
    ) -> Result<FetchResponse, FetchError> {
        // Duplicate the post-allowlist body of `execute_gouv_fr_fetch`
        // so the wiremock tests don't need a live gouv.fr host.
        let parsed = url::Url::parse(&url).map_err(|_| FetchError::InvalidUrl)?;
        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            return Err(FetchError::InvalidUrl);
        }
        let response = client.get(&url).send().await.map_err(|e| {
            if e.is_timeout() {
                FetchError::Timeout
            } else {
                FetchError::Transport
            }
        })?;
        let status = response.status();
        if !status.is_success() {
            return Err(FetchError::UpstreamStatus(status.as_u16()));
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        if let Some(len) = response.content_length()
            && len as usize > MAX_FETCH_RESPONSE_BYTES
        {
            return Err(FetchError::ResponseTooLarge);
        }
        let bytes = response.bytes().await.map_err(|_| FetchError::Transport)?;
        if bytes.len() > MAX_FETCH_RESPONSE_BYTES {
            return Err(FetchError::ResponseTooLarge);
        }
        let bytes_read = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
        let body = String::from_utf8_lossy(&bytes).into_owned();
        Ok(FetchResponse {
            body,
            content_type,
            bytes_read,
        })
    }

    #[tokio::test]
    async fn execute_returns_host_not_allowed_for_evil_host() {
        let client = super::super::build_client();
        let req = FetchRequest {
            url: "https://evil.example.com/foo".to_string(),
        };
        let result = execute_gouv_fr_fetch(&client, &GouvFrConfig {}, req).await;
        assert!(matches!(result, Err(FetchError::HostNotAllowed)));
    }

    #[tokio::test]
    async fn execute_returns_host_not_allowed_for_prefix_lookalike() {
        // Guards the suffix-match implementation against the naive
        // `.ends_with()` bug: `evilservice-public.fr` must NOT match
        // `service-public.fr`.
        let client = super::super::build_client();
        let req = FetchRequest {
            url: "https://evilservice-public.fr/foo".to_string(),
        };
        let result = execute_gouv_fr_fetch(&client, &GouvFrConfig {}, req).await;
        assert!(matches!(result, Err(FetchError::HostNotAllowed)));
    }

    #[tokio::test]
    async fn execute_returns_invalid_url_for_http_scheme() {
        let client = super::super::build_client();
        let req = FetchRequest {
            url: "http://service-public.fr/foo".to_string(),
        };
        let result = execute_gouv_fr_fetch(&client, &GouvFrConfig {}, req).await;
        assert!(matches!(result, Err(FetchError::InvalidUrl)));
    }

    #[tokio::test]
    async fn execute_returns_invalid_url_for_unparseable() {
        let client = super::super::build_client();
        let req = FetchRequest {
            url: "not a url".to_string(),
        };
        let result = execute_gouv_fr_fetch(&client, &GouvFrConfig {}, req).await;
        assert!(matches!(result, Err(FetchError::InvalidUrl)));
    }

    #[tokio::test]
    async fn execute_success_returns_body_and_bytes_read() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/page"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw("<html>hello world</html>", "text/html; charset=utf-8"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = super::super::build_client();
        let url = format!("{}/page", server.uri());
        let resp = exec_bypass_allowlist(&client, url).await.expect("success");
        assert_eq!(resp.body, "<html>hello world</html>");
        assert_eq!(resp.content_type, "text/html; charset=utf-8");
        assert_eq!(resp.bytes_read, "<html>hello world</html>".len() as u32);
    }

    #[tokio::test]
    async fn execute_returns_response_too_large_on_content_length() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        // Body IS large; wiremock computes Content-Length automatically
        // so the Content-Length gate fires before .bytes().await.
        let big_body = "x".repeat(MAX_FETCH_RESPONSE_BYTES + 100);
        Mock::given(method("GET"))
            .and(path("/big"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(big_body, "text/plain"))
            .mount(&server)
            .await;

        let client = super::super::build_client();
        let url = format!("{}/big", server.uri());
        let result = exec_bypass_allowlist(&client, url).await;
        assert!(matches!(result, Err(FetchError::ResponseTooLarge)));
    }

    #[tokio::test]
    async fn execute_returns_upstream_status_on_4xx() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = super::super::build_client();
        let url = format!("{}/missing", server.uri());
        let result = exec_bypass_allowlist(&client, url).await;
        assert!(matches!(result, Err(FetchError::UpstreamStatus(404))));
    }

    #[tokio::test]
    async fn execute_returns_transport_error_on_unreachable_upstream() {
        // Bogus port on 127.0.0.1 — connect refuses immediately.
        let client = super::super::build_client();
        let url = "https://127.0.0.1:1/does-not-exist".to_string();
        let result = exec_bypass_allowlist(&client, url).await;
        assert!(matches!(result, Err(FetchError::Transport)));
    }

    // ---- Q4 log-discipline test — the AC4 gate ----

    /// Custom tracing layer that records every emitted event's field
    /// names + string values into a shared vector. Used to assert Q4
    /// discipline on `fetch_requested` and `fetch_egress` events.
    #[derive(Clone, Default)]
    struct CapturingLayer {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
    }

    #[derive(Debug, Clone, Default)]
    #[allow(dead_code)] // `status` is populated for future assertions; kept in the struct for symmetry with `egress_search`.
    struct CapturedEvent {
        target: String,
        event_name: Option<String>,
        upstream: Option<String>,
        host_class: Option<String>,
        status: Option<String>,
        latency_ms: Option<i64>,
        forbidden_fields: Vec<String>,
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
        host_class: Option<String>,
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
                host_class: self.host_class,
                status: self.status,
                latency_ms: self.latency_ms,
                forbidden_fields: self.forbidden_fields,
                all_field_values: self.all_field_values,
            }
        }
    }

    // Q4 allowlist — fields we DO expect on the two egress events. Any
    // other field is a discipline violation and lands in
    // `forbidden_fields` for the assertion below to catch.
    const ALLOWED_FIELDS: &[&str] = &[
        "event",
        "upstream",
        "host_class",
        "status",
        "latency_ms",
        "message",
    ];

    impl tracing::field::Visit for FieldVisitor {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            let name = field.name();
            self.all_field_values.push(value.to_string());
            match name {
                "event" => self.event_name = Some(value.to_string()),
                "upstream" => self.upstream = Some(value.to_string()),
                "host_class" => self.host_class = Some(value.to_string()),
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
                return;
            }
            if ALLOWED_FIELDS.contains(&name) {
                match name {
                    "event" => self.event_name = Some(formatted),
                    "upstream" => self.upstream = Some(formatted),
                    "host_class" => self.host_class = Some(formatted),
                    "status" => self.status = Some(formatted),
                    _ => {}
                }
            } else {
                self.forbidden_fields.push(name.to_string());
            }
        }
    }

    /// **Load-bearing Q4 discipline test.** Runs a fetch against a
    /// gouv.fr URL that will fail to reach the real upstream in the
    /// test env (transport failure, deterministic), captures every
    /// tracing event emitted by `fetch()`, and asserts:
    ///   * Exactly two events are emitted by our module:
    ///     `fetch_requested` and `fetch_egress`.
    ///   * Each event has only the allowed fields (event, upstream,
    ///     host_class, status, latency_ms, message).
    ///   * No field named `tenant_hash`, `tenant_id`, `user_id`, `url`,
    ///     `query`, `chat_id`, `customer_id`, `api_key` appears.
    ///   * The captured field values contain none of the sensitive URL
    ///     bytes across ALL emitters (our module + reqwest + hyper).
    ///   * The audit event's `host_class` is exactly `"service_public"`
    ///     (the allowlist label — not the raw host).
    ///
    /// If this test fails, the Q4 STRIP TOTAL invariant is broken. Do
    /// NOT weaken the assertion — fix the emit-side.
    #[tokio::test]
    async fn log_assertion_no_tenant_no_url_no_forbidden_fields() {
        let layer = CapturingLayer::default();
        let subscriber = tracing_subscriber::registry().with(layer.clone());

        let sensitive_url = "https://service-public.fr/TENANT-42-SECRET-PATH-do-not-leak-into-logs";
        let client = FetchEgressClient::new(FetchUpstream::GouvFr(GouvFrConfig {}));

        let _guard = set_default(subscriber);
        let req = FetchRequest {
            url: sensitive_url.to_string(),
        };
        // The upstream call will succeed or fail transport-wise in CI —
        // the Q4 assertion holds for BOTH paths per the discipline. In
        // practice CI has no outbound Internet, so this returns
        // Transport or Timeout. That's fine — the assertion is on the
        // log-emit shape, not the outcome.
        let _ = client.fetch(req).await;
        drop(_guard);

        let all_events = layer.events.lock().unwrap().clone();

        let our_target_prefix = "mika_gateway::egress_fetch";
        let our_events: Vec<_> = all_events
            .iter()
            .filter(|e| e.target.starts_with(our_target_prefix))
            .cloned()
            .collect();

        assert_eq!(
            our_events.len(),
            2,
            "unexpected event count from our module (target prefix \
             {our_target_prefix}); events: {our_events:?}. The \
             gouv_fr impl MUST NOT add its own tracing calls — Q4 \
             STRIP TOTAL."
        );

        let event_names: Vec<_> = our_events
            .iter()
            .filter_map(|e| e.event_name.clone())
            .collect();
        assert!(event_names.contains(&"fetch_requested".to_string()));
        assert!(event_names.contains(&"fetch_egress".to_string()));

        // Every OUR event's `upstream` field is exactly "gouv_fr".
        for e in &our_events {
            assert_eq!(e.upstream.as_deref(), Some("gouv_fr"));
            assert!(
                e.forbidden_fields.is_empty(),
                "fetch() emitted forbidden fields: {:?}. Q4 STRIP TOTAL violated.",
                e.forbidden_fields
            );
        }

        // Load-bearing cross-source assertion: the sensitive URL bytes
        // must NOT appear in ANY captured field value across ALL events
        // (our module + reqwest + hyper + etc). If reqwest ever ships a
        // release that logs the full request URL at INFO/DEBUG
        // (unlikely but possible), this test catches it.
        let sensitive_marker = "TENANT-42-SECRET-PATH-do-not-leak-into-logs";
        for e in &all_events {
            for value in &e.all_field_values {
                assert!(
                    !value.contains(sensitive_marker),
                    "URL bytes leaked into a tracing field value from \
                     target={}: {value}",
                    e.target
                );
            }
        }

        // The audit event carries `host_class = "service_public"` — the
        // allowlist label, never the raw host.
        let audit = our_events
            .iter()
            .find(|e| e.event_name.as_deref() == Some("fetch_egress"))
            .expect("fetch_egress event present");
        assert_eq!(audit.host_class.as_deref(), Some("service_public"));
        assert!(audit.latency_ms.is_some());
    }
}

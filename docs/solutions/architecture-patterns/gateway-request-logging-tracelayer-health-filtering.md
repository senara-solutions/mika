---
title: "Gateway request logging with TraceLayer and health probe filtering"
category: architecture-patterns
date: 2026-03-31
tags: [gateway, middleware, tower-http, TraceLayer, observability, logging, health-probes]
severity: low
components: [mika-gateway, mika-agent]
---

# Gateway Request Logging with TraceLayer and Health Probe Filtering

## Problem

The gateway (`mika-gateway`) had no request/response logging middleware, while `mika-spirit` already used `tower_http::trace::TraceLayer`. This created an observability gap — gateway requests were invisible in logs, making it difficult to debug routing issues, latency problems, or failed requests.

## Root Cause

The gateway router in `crates/mika-gateway/src/routes.rs` was assembled with `RequestBodyLimitLayer` and `SetResponseHeaderLayer` but never included a `TraceLayer`. This was simply an omission from the initial gateway implementation.

## Solution

Added `TraceLayer::new_for_http()` with custom `make_span_with`, `on_response`, and `on_failure` closures to both the gateway and agent server routers. The key design choice was differentiating health probes from operational routes:

```rust
use tower_http::trace::TraceLayer;

// In build_router():
.layer(
    TraceLayer::new_for_http()
        .make_span_with(|request: &http::Request<_>| {
            let path = request.uri().path();
            let method = request.method();
            if is_health_probe(path) {
                tracing::debug_span!("http_request", %method, path)
            } else {
                tracing::info_span!("http_request", %method, path)
            }
        })
        .on_response(
            |response: &http::Response<_>, latency: Duration, span: &tracing::Span| {
                let status = response.status().as_u16();
                let is_debug = span.metadata().is_some_and(|m| {
                    *m.level() == tracing::Level::DEBUG
                });
                if status >= 500 {
                    tracing::warn!(status, ?latency, "response");
                } else if is_debug {
                    tracing::debug!(status, ?latency, "response");
                } else {
                    tracing::info!(status, ?latency, "response");
                }
            },
        )
        .on_failure(
            |error: tower_http::classify::ServerErrorsFailureClass,
             latency: Duration,
             _span: &tracing::Span| {
                tracing::error!(
                    classification = %error,
                    ?latency,
                    "response failed"
                );
            },
        ),
)

fn is_health_probe(path: &str) -> bool {
    matches!(path, "/health" | "/readyz" | "/livez")
}
```

### Key decisions

1. **Custom `make_span_with` instead of defaults:** Both the gateway and mika-spirit use custom span factories to log health probes at DEBUG level (reducing noise from Kubernetes liveness/readiness checks that fire every few seconds). The gateway checks three paths (`/health`, `/readyz`, `/livez`); the server checks only `/health` (its sole health endpoint).

2. **Span-level filtering:** Health probe spans use `debug_span!` while operational routes use `info_span!`. The `on_response` callback checks the span's metadata level to match: health probe responses use `debug!`, operational responses use `info!`, and 5xx responses always use `warn!`. This ensures health probe traffic is fully suppressed at INFO log level — both the span and the response event.

3. **No OTel target tagging:** Per the documented learning in `docs/solutions/integration-issues/langfuse-non-llm-span-filtering.md`, infrastructure spans must not use `target: "mika::otel"`. Tower-http's default span target is automatically excluded from the OTLP exporter's `filter::Targets` configuration.

4. **No Cargo.toml changes:** The `tower-http` crate with the `"trace"` feature was already a workspace dependency.

5. **Custom `on_failure` for connection-level errors:** Both components have explicit `on_failure` callbacks that log at ERROR with classification and latency. Without this, tower-http's default `on_failure` would log timeouts and stream errors without the span's method/path context being visible in flat log searches.

## Prevention

- When adding new HTTP services (Axum routers), include `TraceLayer` from the start with custom `make_span_with`, `on_response`, and `on_failure`.
- Use custom `make_span_with` when health probe noise reduction is needed (any service behind Kubernetes probes).
- Always check `docs/solutions/integration-issues/langfuse-non-llm-span-filtering.md` before adding tracing spans to avoid unintended OTel export.

## References

- Gateway router: `crates/mika-gateway/src/routes.rs`
- Agent server router: `crates/mika-agent/src/server/mod.rs`
- Langfuse span filtering: `docs/solutions/integration-issues/langfuse-non-llm-span-filtering.md`
- GitHub issue: #355

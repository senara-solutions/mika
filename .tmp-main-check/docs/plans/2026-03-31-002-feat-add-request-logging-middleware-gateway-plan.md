---
title: "feat: Add request logging middleware to mika-gateway"
type: feat
status: completed
date: 2026-03-31
---

# Add Request Logging Middleware to mika-gateway

## Overview

The gateway (`mika-gateway`) has no request/response logging middleware, while `mika-spirit` already uses `tower_http::trace::TraceLayer`. This creates an observability gap — gateway requests are invisible in logs. Adding `TraceLayer` with health-endpoint filtering brings the gateway to parity with mika-spirit.

## Problem Statement

Without request logging, operators cannot see incoming webhook traffic, outbound relay calls, or A2A proxy requests in gateway logs. This makes debugging routing issues, latency problems, and failed requests significantly harder. Health probes (`/health`, `/readyz`, `/livez`) fire frequently from Kubernetes liveness/readiness checks and would create noise if logged at INFO level.

## Proposed Solution

Add `TraceLayer` to the gateway router in `routes.rs` with a custom `make_span_with` closure that logs health probe paths at DEBUG level and all other paths at INFO level.

### Key Design Decisions

1. **Custom `make_span_with` (not plain defaults):** mika-spirit uses `TraceLayer::new_for_http()` with defaults, but the issue explicitly requires health probes at DEBUG. The gateway needs a custom span factory to differentiate health paths from operational paths.

2. **Span fields:** method, path — consistent with what `TraceLayer` defaults provide, but explicitly set in our custom span for control.

3. **No OTel target tagging:** Per documented learning (`docs/solutions/integration-issues/langfuse-non-llm-span-filtering.md`), infrastructure spans must NOT use `target: "mika::otel"` to avoid being exported to Langfuse. The default `tower_http` target is correct.

4. **`on_response` logging:** Add a custom `on_response` callback to log status code and latency at the same level as the span (DEBUG for health, INFO for others). This ensures all four required fields (method, path, status, latency) appear in a single log line.

## Acceptance Criteria

- [x] `tower_http::trace::TraceLayer` added to gateway router in `crates/mika-gateway/src/routes.rs`
- [x] Every request logs: method, path, status code, latency
- [x] Health probe requests (`/health`, `/readyz`, `/livez`) logged at DEBUG level
- [x] All other requests (`/webhook/telegram`, `/send`, `/a2a/*`) logged at INFO level
- [x] Structured logging output consistent with existing gateway JSON/pretty format
- [x] No span target set to `"mika::otel"` (avoids Langfuse export)
- [x] `cargo clippy` and `cargo test -p mika-gateway` pass

## MVP

### `crates/mika-gateway/src/routes.rs`

Add import and layer to `build_router()`:

```rust
use tower_http::trace::TraceLayer;

// In build_router(), between SetResponseHeaderLayer and .with_state(state):
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
)

// Helper function:
fn is_health_probe(path: &str) -> bool {
    matches!(path, "/health" | "/readyz" | "/livez")
}
```

**Layer ordering:** `TraceLayer` goes AFTER `SetResponseHeaderLayer` (outermost layer = first to see request, last to see response). This matches mika-spirit's pattern where `TraceLayer` is the outermost `.layer()` call before `.with_state()`.

## Technical Considerations

- **No Cargo.toml changes needed:** `tower-http` with the `"trace"` feature is already a workspace dependency, and `mika-gateway` already depends on `tower-http = { workspace = true }`.
- **No new dependencies:** `http::Request` is available via axum re-export, `Duration` via `std::time::Duration` (already imported).
- **Structured logging:** The gateway already initializes `tracing-subscriber` via `mika_common::logging::init()` with JSON (prod) or pretty (dev) format. `TraceLayer` spans and events automatically use the active subscriber — no additional configuration needed.
- **`on_response` level strategy:** The `on_response` callback checks the span's metadata level via `span.metadata()` to match the response log level to the request span level. Health probe responses use `debug!`, operational responses use `info!`, and 5xx responses always use `warn!`. This ensures health probe traffic is fully silent at INFO log level.

## Sources

- mika-spirit implementation: `crates/mika-agent/src/server/mod.rs:204`
- Gateway router: `crates/mika-gateway/src/routes.rs:64-100`
- Langfuse span filtering learning: `docs/solutions/integration-issues/langfuse-non-llm-span-filtering.md`
- GitHub issue: #355

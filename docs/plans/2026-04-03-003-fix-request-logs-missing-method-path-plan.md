---
title: "fix: request logs missing HTTP method and path"
type: fix
status: completed
date: 2026-04-03
---

# fix: request logs missing HTTP method and path

Gateway and server error/warn logs show status code and latency but not the HTTP method or request path, making it impossible to correlate errors to specific endpoints without timestamp matching.

## Acceptance Criteria

- [x] Gateway `on_response` warn/error logs include `method` and `path` as explicit fields
- [x] Gateway has custom `on_failure` callback with `method`, `path`, classification, and latency
- [x] Agent server (`mika-server`) has custom `TraceLayer` matching the gateway pattern
- [x] Agent server health probe (`/health`) logged at DEBUG level to reduce noise
- [x] No `target: "mika::otel"` on infrastructure spans (OTel export constraint)
- [x] Existing gateway tests pass; new test for agent server health probe classification

## Context

**Current state:**
- **Gateway** (`crates/mika-gateway/src/routes.rs:117-141`): Has custom `make_span_with` (method + path in span), custom `on_response` (WARN for 5xx, DEBUG for health probes, INFO otherwise). Span fields are inherited in JSON output but are nested under `span` key — not flat-searchable. No `on_failure` callback (uses tower-http default which logs classification + latency only).
- **Agent server** (`crates/mika-agent/src/server/mod.rs:202`): Uses bare `TraceLayer::new_for_http()` — all defaults, DEBUG-level spans invisible in production.

**Constraints:**
- Infrastructure spans must NOT use `target: "mika::otel"` — only semantic spans (LLM calls, agent turns) are exported to Langfuse (see `docs/solutions/integration-issues/langfuse-non-llm-span-filtering.md`)
- `on_response` receives `(&Response, Duration, &Span)` — no direct access to request; method/path must come from the span
- `on_failure` receives `(ServerErrorsFailureClass, Duration, &Span)` — same constraint

**Key insight:** The `on_response` and `on_failure` closures run inside the span created by `make_span_with`. To get method/path as explicit (flat) log fields, record them on the span and then retrieve them via `span.record()` pattern — or simply accept that span fields are the canonical location and ensure the closures also log them explicitly by extracting from the span's extensions or by recording them as event fields.

Actually, the simplest approach: since `on_response` and `on_failure` both receive the `&Span` parameter, and the span was created with `%method` and `path` fields — these fields are already recorded on the span. In `tracing_subscriber` JSON format, span fields appear on every event emitted within that span. The real fix is:
1. Ensure both components have proper `make_span_with` (gateway already does, server doesn't)
2. Add custom `on_failure` to both components (neither has one)
3. The `on_response` warn logs already work correctly with span inheritance in JSON format

## MVP

### `crates/mika-gateway/src/routes.rs` — Add `on_failure` callback

```rust
// After .on_response(...), add:
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
)
```

The span's `method` and `path` fields are inherited automatically by the JSON subscriber. The `on_failure` just needs to log the failure-specific fields (classification, latency) — span context provides the request identity.

### `crates/mika-agent/src/server/mod.rs` — Replace bare TraceLayer

```rust
use std::time::Duration;

// Replace: .layer(TraceLayer::new_for_http())
// With:
.layer(
    TraceLayer::new_for_http()
        .make_span_with(|request: &http::Request<_>| {
            let path = request.uri().path();
            let method = request.method();
            if path == "/health" {
                tracing::debug_span!("http_request", %method, path)
            } else {
                tracing::info_span!("http_request", %method, path)
            }
        })
        .on_response(
            |response: &http::Response<_>, latency: Duration, span: &tracing::Span| {
                let status = response.status().as_u16();
                let is_debug = span
                    .metadata()
                    .is_some_and(|m| *m.level() == tracing::Level::DEBUG);
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
```

### Expected log output (JSON format, after fix)

```json
{"timestamp":"...","level":"WARN","fields":{"message":"response","status":502,"latency":"24.040739ms"},"target":"mika_gateway::routes","span":{"method":"POST","path":"/send","name":"http_request"},"spans":[...]}
```

The `method` and `path` are in the `span` object — structured log tools (Datadog, Loki, CloudWatch Insights) can query `span.method` and `span.path`.

## Sources

- Related issue: #412
- Gateway pattern: `crates/mika-gateway/src/routes.rs:117-141`
- Agent server: `crates/mika-agent/src/server/mod.rs:202`
- OTel constraint: `docs/solutions/integration-issues/langfuse-non-llm-span-filtering.md`
- Gateway logging pattern: `docs/solutions/architecture-patterns/gateway-request-logging-tracelayer-health-filtering.md`

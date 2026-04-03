---
title: "TraceLayer top-level JSON fields via response extensions"
category: architecture-patterns
date: 2026-04-03
severity: medium
tags: [logging, tracing, axum, tower-http, json, observability]
related_issues: [412, 417, 419, 420]
---

# TraceLayer top-level JSON fields via response extensions

## Problem

`tower_http::trace::TraceLayer`'s `make_span_with` records `method` and `path` as span fields, but `on_response` can only emit event fields. In `tracing-subscriber`'s JSON format, span fields appear nested inside a `spans` array — not as top-level keys. Log aggregation tools (Datadog, Loki, CloudWatch) query top-level fields, making method and path effectively invisible for filtering and alerting.

PR #417 added the TraceLayer with `make_span_with` but did not solve the JSON nesting issue.

## Root Cause

`tracing-subscriber`'s JSON layer with `flatten_event(true)` flattens *event* metadata to top-level keys, but span fields remain in the `spans` array. There is no built-in option to flatten span fields into events. The `on_response` callback receives `(&Response, Duration, &Span)` — it has the span but cannot programmatically read recorded field values back from it, and it does not have access to the original request.

## Solution

Use an Axum `from_fn` middleware to bridge request data to the response phase via response extensions:

```rust
#[derive(Clone, Debug)]
struct RequestMeta {
    method: String,
    path: String,
}

async fn inject_request_meta(request: Request, next: middleware::Next) -> Response {
    let meta = RequestMeta {
        method: request.method().to_string(),
        path: request.uri().path().to_owned(),
    };
    let mut response = next.run(request).await;
    response.extensions_mut().insert(meta);
    response
}
```

**Critical: Layer ordering.** The `from_fn` middleware must be **inner** to `TraceLayer` so that on the response path, it inserts `RequestMeta` into extensions *before* `on_response` reads them:

```rust
.layer(middleware::from_fn(inject_request_meta))  // inner — response processed first
.layer(TraceLayer::new_for_http()                 // outer — on_response sees extensions
    .on_response(|response: &http::Response<_>, latency: Duration, span: &tracing::Span| {
        let (method, path) = response
            .extensions()
            .get::<RequestMeta>()
            .map(|m| (m.method.as_str(), m.path.as_str()))
            .unwrap_or(("unknown", "unknown"));
        tracing::info!(status, method, path, ?latency, "response");
    })
)
```

**`on_failure` limitation:** Connection-level failures (`on_failure`) never produce a response object, so `RequestMeta` is unavailable. Method and path remain accessible only via the parent span's fields (nested in JSON `spans` array). This is acceptable — connection failures are rare.

**Span fields kept:** `make_span_with` continues to set method and path on the span for trace-level correlation and pretty-format readability. The intentional duplication serves different consumers.

## Prevention

- When adding structured fields to `tracing` events in JSON-formatted services, always verify they appear as top-level JSON keys (not nested in `spans`). Test with `MIKA_LOG_FORMAT=json` and pipe through `jq`.
- Do NOT add `target: "mika::otel"` to infrastructure/middleware spans — this would leak them to the OTLP exporter (Langfuse). See `docs/solutions/integration-issues/langfuse-non-llm-span-filtering.md`.
- When bridging request-phase data to response-phase callbacks in tower middleware, response extensions via `from_fn` is the idiomatic Axum pattern. The alternative (adding axum as a dependency to a shared library for a trivial struct) is not worth the coupling.

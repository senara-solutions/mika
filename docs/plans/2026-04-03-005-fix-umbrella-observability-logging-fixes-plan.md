---
title: "fix: umbrella observability and logging fixes"
type: fix
status: completed
date: 2026-04-03
issues: [420, 419, 412, 354]
---

# fix: umbrella observability and logging fixes

## Overview

Group of three logging and observability fixes for mika-spirit and mika-gateway, shipped as one branch and one PR. Closes #420 (umbrella), #419, #412, #354.

## Problem Statement

1. **#419/#412 — method and path missing from JSON log events.** PR #417 added `TraceLayer` with `make_span_with` that puts `method` and `path` on the tracing span, but `on_response` only emits `status` and `latency` as event fields. In JSON format (`MIKA_LOG_FORMAT=json`, the production default), span fields are nested inside a `spans` array — not top-level keys. Log aggregation tools (Datadog, Loki, CloudWatch) query top-level fields, so method and path are effectively invisible for filtering and alerting.

2. **#354 — no /version endpoint on mika-gateway.** Operators cannot identify the running build version or git commit without SSH access. Useful for verifying deployments and debugging routing issues.

## Proposed Solution

### Task 1: Top-level method/path in JSON logs (server + gateway)

**Approach:** Axum `from_fn` middleware + response extensions.

1. Create a `RequestMeta` struct carrying `method: String` and `path: String`.
2. Add an Axum `from_fn` middleware (`inject_request_meta`) that:
   - Captures `request.method().to_string()` and `request.uri().path().to_owned()` before forwarding
   - After `next.run(request).await`, inserts `RequestMeta` into `response.extensions_mut()`
3. In `on_response`, extract `RequestMeta` from `response.extensions()` and include `method` and `path` as explicit event fields alongside `status` and `latency`.
4. Apply to both mika-spirit and mika-gateway.

**Layer ordering** (critical): The `from_fn` middleware must be **inner** to `TraceLayer` so that on the response path, `from_fn` inserts extensions *before* `TraceLayer::on_response` reads them:

```
.layer(from_fn(inject_request_meta))  // inner — processes response first
.layer(TraceLayer::new_for_http()...) // outer — on_response sees extensions
```

**`on_failure` limitation:** Connection-level failures (`on_failure`) never produce a response object, so response extensions are unavailable. `on_failure` will continue to rely on span-level fields (method/path nested in JSON `spans` array). This is acceptable — connection failures are rare, and the span context provides sufficient debugging info. Add a code comment documenting this tradeoff.

**Span fields kept:** `make_span_with` continues to set `method` and `path` on the span for trace-level correlation and pretty-format readability. The intentional duplication (top-level event fields + span fields) serves different consumers: log aggregation uses top-level, trace viewers use span context.

### Task 2: /version endpoint on mika-gateway

1. Add `GET /version` route returning `Json<VersionInfo>` with `version` and `git_hash` fields.
2. Version: `env!("CARGO_PKG_VERSION")` (compile-time from Cargo.toml).
3. Git hash: `option_env!("GIT_HASH").unwrap_or("unknown")`, set via `build.rs`:
   ```rust
   let git_hash = std::process::Command::new("git")
       .args(["rev-parse", "--short", "HEAD"])
       .output()
       .ok()
       .and_then(|o| if o.status.success() { Some(o) } else { None })
       .and_then(|o| String::from_utf8(o.stdout).ok())
       .map(|s| s.trim().to_string())
       .unwrap_or_else(|| "unknown".to_string());
   println!("cargo:rustc-env=GIT_HASH={git_hash}");
   ```
4. Add `cargo::rerun-if-changed=.git/HEAD` and `cargo::rerun-if-changed=.git/refs` to `build.rs` so new commits trigger rebuilds.
5. No authentication required (same as health probes).
6. Add `/version` to `is_health_probe()` so it logs at DEBUG level (avoids noise if polled by monitoring).

**Example response:**
```json
{"version": "0.4.0", "git_hash": "abc1234"}
```

## Technical Considerations

- **Path-only logging** (no query string): `request.uri().path()` matches current `make_span_with` behavior. Query strings can contain sensitive data.
- **String allocation per request:** `RequestMeta` allocates `String` for method and path on every request. Negligible overhead for typical web traffic.
- **Duplicate fields in JSON:** After the fix, method/path appear at top-level AND in `spans[0]`. This is intentional — different consumers use different fields.
- **Subscriber config unchanged:** The `flatten_event(true)` JSON config in `logging.rs` is correct and does not need modification.
- **`on_failure` keeps span-only fields:** Documented limitation. Connection-level failures are rare.
- **Git hash fallback:** `"unknown"` when `.git` is absent (Docker builds, tarballs). Short hash (7 chars) for readability.

## Acceptance Criteria

- [x] JSON log output for any request includes `method` and `path` as top-level keys (not only inside `spans`)
- [x] WARN-level logs for 5xx responses include top-level `method` and `path`
- [x] DEBUG-level logs for health probe endpoints include top-level `method` and `path`
- [x] Both mika-spirit and mika-gateway `on_response` logs include top-level `method` and `path`
- [x] Pretty format output is not degraded (method/path still visible)
- [x] `GET /version` on gateway returns `200` with `{"version":"<semver>","git_hash":"<7-char-hash>"}`
- [x] `GET /version` requires no authentication
- [x] `/version` logged at DEBUG level (not INFO)
- [x] `on_failure` handler documents the span-only limitation in a code comment
- [x] Existing tests pass (`cargo test`)
- [x] `cargo clippy` clean

## Implementation Steps

### Step 1: Shared `RequestMeta` type + middleware

**File: `crates/mika-common/src/middleware.rs` (new)**

```rust
use axum::{extract::Request, middleware::Next, response::Response};

/// Carries HTTP method and path from request to response extensions,
/// making them available to TraceLayer's `on_response` callback.
#[derive(Clone, Debug)]
pub struct RequestMeta {
    pub method: String,
    pub path: String,
}

/// Middleware that captures method and path from the request and injects
/// them into response extensions for downstream logging.
pub async fn inject_request_meta(request: Request, next: Next) -> Response {
    let meta = RequestMeta {
        method: request.method().to_string(),
        path: request.uri().path().to_owned(),
    };
    let mut response = next.run(request).await;
    response.extensions_mut().insert(meta);
    response
}
```

Export from `crates/mika-common/src/lib.rs` as `pub mod middleware`.

### Step 2: Update mika-spirit TraceLayer

**File: `crates/mika-agent/src/server/mod.rs` (lines 203-240)**

Add `use mika_common::middleware::{RequestMeta, inject_request_meta};` and `use axum::middleware;`.

Update the router layers:

```rust
.layer(middleware::from_fn(inject_request_meta))  // inner
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
                let (method, path) = response
                    .extensions()
                    .get::<RequestMeta>()
                    .map(|m| (m.method.as_str(), m.path.as_str()))
                    .unwrap_or(("unknown", "unknown"));
                if status >= 500 {
                    tracing::warn!(status, method, path, ?latency, "response");
                } else if is_debug {
                    tracing::debug!(status, method, path, ?latency, "response");
                } else {
                    tracing::info!(status, method, path, ?latency, "response");
                }
            },
        )
        .on_failure(
            // NOTE: on_failure fires on connection-level failures where no response
            // is produced. RequestMeta is carried via response extensions, so it is
            // unavailable here. Method and path remain accessible via the parent
            // span's fields (nested in JSON output under `spans`).
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
)  // outer
```

### Step 3: Update mika-gateway TraceLayer

**File: `crates/mika-gateway/src/routes.rs` (lines 116-154)**

Same pattern as Step 2. Add `use mika_common::middleware::{RequestMeta, inject_request_meta};`. Update `is_health_probe()` to include `/version`. Insert `from_fn` layer before `TraceLayer`. Update `on_response` to extract and log method/path from response extensions.

### Step 4: Add /version endpoint to gateway

**File: `crates/mika-gateway/src/routes.rs`**

Add route and handler:

```rust
.route("/version", get(handle_version))

// ...

#[derive(serde::Serialize)]
struct VersionInfo {
    version: &'static str,
    git_hash: &'static str,
}

async fn handle_version() -> axum::Json<VersionInfo> {
    axum::Json(VersionInfo {
        version: env!("CARGO_PKG_VERSION"),
        git_hash: option_env!("GIT_HASH").unwrap_or("unknown"),
    })
}
```

### Step 5: Update gateway build.rs for git hash

**File: `crates/mika-gateway/build.rs`**

```rust
fn main() {
    println!("cargo::rerun-if-changed=migrations");

    // Capture short git hash for /version endpoint.
    // Falls back to "unknown" when .git is absent (Docker builds, tarballs).
    let git_hash = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| if o.status.success() { Some(o) } else { None })
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_HASH={git_hash}");

    println!("cargo::rerun-if-changed=.git/HEAD");
    println!("cargo::rerun-if-changed=.git/refs");
}
```

### Step 6: Update is_health_probe to include /version

**File: `crates/mika-gateway/src/routes.rs`**

```rust
fn is_health_probe(path: &str) -> bool {
    matches!(path, "/health" | "/readyz" | "/livez" | "/version")
}
```

Update existing unit tests and add test for `/version`.

### Step 7: Tests

- Existing `cargo test` must pass
- Add unit test for `/version` handler in gateway
- Update `is_health_probe` unit tests to include `/version`
- `cargo clippy` clean

## Sources

- Issue #420: [umbrella: observability and logging fixes](https://github.com/senara-solutions/mika/issues/420)
- Issue #419: [TraceLayer on_response doesn't include method/path in log events](https://github.com/senara-solutions/mika/issues/419)
- Issue #412: [Request logs missing HTTP method and path](https://github.com/senara-solutions/mika/issues/412)
- Issue #354: [Add /version endpoint to mika-gateway](https://github.com/senara-solutions/mika/issues/354)
- PR #417: [fix: add method and path to request error/warn logs](https://github.com/senara-solutions/mika/pull/417) (incomplete fix)
- Learning: `docs/solutions/architecture-patterns/gateway-request-logging-tracelayer-health-filtering.md`
- Learning: `docs/solutions/integration-issues/langfuse-non-llm-span-filtering.md` (do NOT use `target: "mika::otel"` on request logging spans)
- Learning: `docs/solutions/security-issues/debug-log-secret-leakage-and-file-permissions.md` (no raw payload logging)

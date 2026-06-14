# Plan: Add Interactive OpenAPI Docs (Swagger UI) — mika#175

## Summary

Serve interactive Swagger UI for both mika-server and mika-gateway OpenAPI specs directly from their HTTP servers. Users can browse endpoints, inspect request/response schemas, and optionally try requests from the browser.

## Current State

- Both crates already use `utoipa` to generate OpenAPI specs (`AgentApiDoc` in `mika-agent`, `GatewayApiDoc` in `mika-gateway`).
- Committed YAML specs live at `docs/openapi/mika-server.yaml` and `docs/openapi/gateway.yaml`.
- CI validates specs stay in sync via `test_committed_spec_is_current` tests.
- No interactive docs are served — only raw YAML files exist.

## Approach

Use `utoipa-swagger-ui` (the official utoipa companion crate) with its Axum integration. This is the natural choice:
- Same maintainer as `utoipa` (already a workspace dependency).
- First-class Axum support via the `axum` feature flag.
- Embeds Swagger UI static assets at compile time — no external CDN or npm build needed.
- Single function call to create an Axum router that serves the UI + spec JSON.

## Implementation Steps

### Step 1: Add `utoipa-swagger-ui` workspace dependency

**File:** `Cargo.toml` (workspace root)

Add to `[workspace.dependencies]`:
```toml
utoipa-swagger-ui = { version = "9", features = ["axum"] }
```

Version 9 is the current major that pairs with utoipa 5.x.

### Step 2: Add dependency to `mika-agent`

**File:** `crates/mika-agent/Cargo.toml`

Add:
```toml
utoipa-swagger-ui.workspace = true
```

### Step 3: Mount Swagger UI in mika-server router

**File:** `crates/mika-agent/src/server/mod.rs`

In `build_router()`, add the Swagger UI route **outside** the auth layers (the spec is already committed as a public YAML file — serving it interactively adds no new exposure):

```rust
use utoipa_swagger_ui::SwaggerUi;

// In build_router(), before the final chain:
let swagger = SwaggerUi::new("/swagger-ui/{_:.*}")
    .url("/api-docs/openapi.json", openapi::AgentApiDoc::openapi());
```

Then `.merge(swagger)` into the router alongside the other unauthenticated routes (near `/health` and `/dashboard`).

The Swagger UI will be accessible at `/swagger-ui/` on the mika-server.

### Step 4: Add dependency to `mika-gateway`

**File:** `crates/mika-gateway/Cargo.toml`

Add:
```toml
utoipa-swagger-ui.workspace = true
```

### Step 5: Mount Swagger UI in gateway router

**File:** `crates/mika-gateway/src/routes.rs`

In `build_router()`, merge a Swagger UI router:

```rust
use utoipa_swagger_ui::SwaggerUi;

let swagger = SwaggerUi::new("/swagger-ui/{_:.*}")
    .url("/api-docs/openapi.json", crate::openapi::GatewayApiDoc::openapi());
```

Merge before the middleware layers (security headers, trace layer) so the static assets get the same treatment. Place alongside the unauthenticated health/version routes.

### Step 6: Add tests

**File:** `crates/mika-agent/src/server/openapi.rs`

Add a test that verifies the Swagger UI route is accessible (status 200 on `/swagger-ui/`). This can use the existing `test_app` test helper if available, or a simple axum `TestClient`.

**File:** `crates/mika-gateway/src/openapi.rs`

Same pattern — verify `/swagger-ui/` returns 200.

### Step 7: Update documentation

**File:** `docs/openapi/README.md` (new, minimal)

Document the interactive API docs URLs:
- mika-server: `http://localhost:8080/swagger-ui/`
- gateway: `http://localhost:3001/swagger-ui/`

**File:** `crates/mika-agent/CLAUDE.md`

Add a line to the HTTP Server section noting the Swagger UI endpoint.

**File:** `crates/mika-gateway/CLAUDE.md`

Add `/swagger-ui/` to the endpoints table.

## Design Decisions

1. **Swagger UI over Redoc/Scalar:** Swagger UI is the most widely recognized OpenAPI renderer, has first-class utoipa integration, and supports "Try it out" functionality. Redoc is read-only. Scalar is newer but has less utoipa ecosystem support.

2. **No auth on Swagger UI:** The OpenAPI specs are already committed as public YAML files in the repo. The Swagger UI serves the same information interactively. "Try it out" requests still require Bearer tokens per the security scheme definitions — the UI just provides a convenient input field for them.

3. **Compile-time asset embedding:** `utoipa-swagger-ui` embeds all Swagger UI static assets into the binary. No runtime file serving, no CDN dependency, no npm build step. Increases binary size by ~3-4 MB (acceptable for a server binary).

4. **Consistent path (`/swagger-ui/`):** Both servers use the same path for discoverability. The path is conventional and won't conflict with existing routes.

5. **No Starlight/docs-site integration:** The ticket mentions a future Starlight docs site. This plan covers the runtime HTTP server integration only — the specs served by Swagger UI stay in sync automatically because they're generated from the same `utoipa` derive macros. Starlight integration is a separate concern (static site build vs runtime server).

## Scope Boundaries

- **In scope:** Swagger UI on both HTTP servers, workspace dependency setup, basic tests, doc updates.
- **Out of scope:** Starlight docs site integration (separate ticket), Redoc/Scalar alternatives, custom Swagger UI theming, OpenAPI spec expansion (adding more endpoints to the spec).

## Risk Assessment

- **Low risk.** Additive change — no existing routes or behavior modified. The `utoipa-swagger-ui` crate is mature and widely used. Binary size increase is the only trade-off.
- **Build impact:** First build after adding the dependency will be slower (downloading + compiling Swagger UI assets). Subsequent incremental builds unaffected.

## Acceptance Criteria Mapping

| Criteria | How addressed |
|----------|---------------|
| Both OpenAPI specs rendered interactively | Swagger UI mounted on both mika-server (`:8080/swagger-ui/`) and gateway (`:3001/swagger-ui/`) |
| Navigation sidebar includes API reference | Swagger UI provides built-in endpoint navigation from the spec |
| Specs stay in sync automatically | Specs are generated at compile time from the same `utoipa` derive macros — no manual sync needed |

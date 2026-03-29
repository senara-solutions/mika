# Fix: Restore Dashboard Endpoints in OpenAPI Spec via utoipa Annotations

**Issue:** #321
**Date:** 2026-03-29
**Type:** Bug fix

## Problem

PR #319 (commit c602d98) regenerated `docs/openapi/mika-server.yaml`, removing 3 dashboard toggle endpoints that were previously manually added. The handlers in `embedded_dashboard.rs` lack `#[utoipa::path]` annotations, so code generation doesn't include them.

### Removed Endpoints

- `POST /api/v1/dashboard/enable` — enable embedded dashboard at runtime
- `POST /api/v1/dashboard/disable` — disable embedded dashboard at runtime
- `GET /api/v1/dashboard/status` — return dashboard state (enabled, has_assets, has_token)

### Impact

The `self-knowledge` skill serves the OpenAPI spec via `get_documentation("api-spec")`. Agents asked about the dashboard API get incomplete documentation.

## Implementation Plan

### Step 1: Add utoipa annotations to `embedded_dashboard.rs`

**File:** `crates/mika-agent/src/server/embedded_dashboard.rs`

Add `#[utoipa::path]` annotations to the 3 handler functions:

- `handle_enable` — POST, `/api/v1/dashboard/enable`, 200 + 401 responses, bearer security
- `handle_disable` — POST, `/api/v1/dashboard/disable`, 200 + 401 responses, bearer security
- `handle_status` — GET, `/api/v1/dashboard/status`, 200 + 401 responses, bearer security

Use `body = inline(serde_json::Value)` with `example` values matching actual handler output. Full path prefix required — utoipa doesn't know about Axum's `nest()` routing.

### Step 2: Register paths in `openapi.rs`

**File:** `crates/mika-agent/src/server/openapi.rs`

- Add `use super::embedded_dashboard;` import
- Add 3 paths to `AgentApiDoc`'s `#[openapi(paths(...))]`
- Add test assertion for dashboard endpoints

### Step 3: Regenerate the OpenAPI spec

```bash
cargo test -p mika-agent server::openapi::tests::write_agent_openapi_yaml -- --ignored
```

### Step 4: Sync crate-local copy

```bash
./scripts/sync-agent-docs.sh
```

## Verification

```bash
cargo test -p mika-agent -- openapi         # OpenAPI tests pass (including spec currency check)
cargo build                                 # workspace compiles
cargo clippy                                # no warnings
```

## Acceptance Criteria

- All 3 dashboard endpoints appear in `docs/openapi/mika-server.yaml`
- Each endpoint has correct HTTP method, path, response schema, and security
- `test_committed_spec_is_current` passes
- No new security schemes added (reuses `bearer`)
- No new dependencies added

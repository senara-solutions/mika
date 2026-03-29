---
title: "fix: Restore dashboard endpoints in OpenAPI spec via utoipa annotations"
type: fix
status: active
date: 2026-03-29
---

# fix: Restore Dashboard Endpoints in OpenAPI Spec via utoipa Annotations

## Overview

PR #319 (commit c602d98) regenerated `docs/openapi/mika-server.yaml`, removing 3 dashboard toggle endpoints that were previously manually added. The handlers in `embedded_dashboard.rs` lack `#[utoipa::path]` annotations, so code generation doesn't include them.

## Problem Statement / Motivation

The `self-knowledge` skill serves the OpenAPI spec via `get_documentation("api-spec")`. With the dashboard endpoints missing, agents asked about the dashboard API get incomplete documentation. The root cause is that the endpoints were originally hand-added to the YAML but never backed by utoipa annotations — any spec regeneration removes them.

### Removed Endpoints

- `POST /api/v1/dashboard/enable` — enable embedded dashboard at runtime
- `POST /api/v1/dashboard/disable` — disable embedded dashboard at runtime
- `GET /api/v1/dashboard/status` — return dashboard state (enabled, has_assets, has_token)

## Proposed Solution

Add `#[utoipa::path]` annotations to the 3 handler functions in `embedded_dashboard.rs` and register them in `AgentApiDoc` in `openapi.rs`. This makes them first-class generated endpoints that survive future spec regeneration.

## Technical Considerations

- **Annotation pattern:** Follow the existing convention from `handlers.rs` — `#[utoipa::path]` with HTTP method, full path, responses, and bearer security
- **Response schema:** Use `body = inline(serde_json::Value)` with `example = json!(...)` since these handlers return ad-hoc JSON (no dedicated response struct). The `json!` macro is resolved by utoipa's proc macro internally — no explicit `use serde_json::json` import needed
- **Full path required:** utoipa doesn't know about Axum's `nest()` routing, so paths must include the full prefix (e.g., `/api/v1/dashboard/enable` not just `/dashboard/enable`)
- **Two-copy sync:** Both `docs/openapi/mika-server.yaml` (canonical) and `crates/mika-agent/docs/openapi/mika-server.yaml` (crate-local for crates.io) must be updated. `scripts/sync-agent-docs.sh` handles the copy

## Implementation Plan

### Step 1: Add utoipa annotations to `embedded_dashboard.rs`

**File:** `crates/mika-agent/src/server/embedded_dashboard.rs`

Add `#[utoipa::path]` annotations to the 3 handler functions:

- `handle_enable` — POST, `/api/v1/dashboard/enable`, 200 + 401 responses, bearer security
- `handle_disable` — POST, `/api/v1/dashboard/disable`, 200 + 401 responses, bearer security
- `handle_status` — GET, `/api/v1/dashboard/status`, 200 + 401 responses, bearer security

### Step 2: Register paths in `openapi.rs`

**File:** `crates/mika-agent/src/server/openapi.rs`

- Add `use super::embedded_dashboard;` import
- Add 3 paths to `AgentApiDoc`'s `#[openapi(paths(...))]`
- Add test assertions for dashboard endpoints in `test_agent_openapi_yaml_contains_endpoints`

### Step 3: Regenerate the OpenAPI spec

```bash
cargo test -p mika-agent --lib -- write_agent_openapi_yaml --ignored
```

### Step 4: Sync crate-local copy

```bash
./scripts/sync-agent-docs.sh
```

## Acceptance Criteria

- [x] All 3 dashboard endpoints appear in `docs/openapi/mika-server.yaml`
- [x] Each endpoint has correct HTTP method, path, response schema, and security
- [x] `test_committed_spec_is_current` passes
- [x] `test_agent_openapi_yaml_contains_endpoints` covers all 3 new paths
- [x] No new security schemes added (reuses `bearer`)
- [x] No new dependencies added
- [x] `cargo clippy` clean

## Verification

```bash
cargo test -p mika-agent --lib -- openapi     # OpenAPI tests pass (including spec currency check)
cargo build -p mika-agent                     # crate compiles
cargo clippy -p mika-agent                    # no warnings
```

## Sources & References

- Similar implementation: `crates/mika-agent/src/server/handlers.rs` — existing utoipa annotations on `handle_health`, `handle_message`, `handle_task_complete`
- Learning: `docs/solutions/integration-issues/gateway-monorepo-migration.md` — documents the `test_committed_spec_is_current` pattern
- Learning: `docs/solutions/architecture-patterns/embed-dashboard-spa-rust-embed.md` — documents dashboard route architecture
- Related issue: #321
- Related PR: #319 (caused the regression)

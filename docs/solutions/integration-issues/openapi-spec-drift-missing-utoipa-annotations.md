---
title: "OpenAPI spec drift from missing utoipa annotations or registrations"
category: integration-issues
date: 2026-04-11
tags: [openapi, utoipa, spec-generation, dashboard, regression]
issue: ["#321", "#328"]
pr: ["#326"]
modules: [mika-agent/server]
---

# OpenAPI Spec Drift from Missing utoipa Annotations or Registrations

## Problem

Two variants of the same bug class:

1. **#321 — Missing annotations:** After PR #319 regenerated `docs/openapi/mika-server.yaml`, 3 dashboard toggle endpoints (`/api/v1/dashboard/enable`, `/disable`, `/status`) disappeared from the spec. The endpoints were originally hand-added to the YAML but never backed by `#[utoipa::path]` annotations — any spec regeneration silently removed them.

2. **#328 — Missing registration:** `handle_task_complete` in `handlers.rs` had a `#[utoipa::path]` annotation but was never added to the `paths(...)` list in `AgentApiDoc` in `openapi.rs`. The annotation existed but the endpoint was invisible to the generator because it wasn't registered.

**Symptom (both variants):** The `self-knowledge` skill serves the OpenAPI spec via `get_documentation("api-spec")`. Agents and external consumers couldn't discover the affected endpoints.

## Root Cause

The `AgentApiDoc` struct in `openapi.rs` only generates spec entries for functions that meet **both** requirements: (1) the handler has a `#[utoipa::path]` proc macro annotation, and (2) the handler is listed in the `paths(...)` attribute of `#[openapi(...)]`. Missing either one makes the endpoint invisible. Request/response types also need registration in `components(schemas(...))`.

## Solution

1. **Add `#[utoipa::path]` annotations** to each handler in `embedded_dashboard.rs`:

```rust
#[utoipa::path(
    post,
    path = "/api/v1/dashboard/enable",
    responses(
        (status = 200, description = "Dashboard enabled", body = inline(serde_json::Value),
            example = json!({"enabled": true})),
        (status = 401, description = "Missing or invalid Bearer token"),
    ),
    security(("bearer" = []))
)]
pub async fn handle_enable(State(state): State<AppState>) -> Json<serde_json::Value> { ... }
```

2. **Register in `AgentApiDoc`** in `openapi.rs`:

```rust
paths(
    handlers::handle_health,
    handlers::handle_message,
    embedded_dashboard::handle_enable,
    embedded_dashboard::handle_disable,
    embedded_dashboard::handle_status,
),
```

3. **Add regression test assertions** in `test_agent_openapi_yaml_contains_endpoints`.

4. **Regenerate spec:** `cargo test -p mika-agent --lib -- write_agent_openapi_yaml --ignored`

5. **Sync crate-local copy:** `./scripts/sync-agent-docs.sh`

### Key Details

- **Full paths required:** utoipa doesn't know about Axum's `nest()` routing — use `/api/v1/dashboard/enable`, not `/dashboard/enable`.
- **`json!` macro in attributes:** utoipa's proc macro resolves `json!(...)` internally — no `use serde_json::json` import needed in the handler file.
- **`inline(serde_json::Value)`:** Use for ad-hoc JSON responses without a dedicated struct. Generates `schema: {}` in YAML but the `example` compensates.
- **Two-copy sync:** Both `docs/openapi/mika-server.yaml` (canonical) and `crates/mika-agent/docs/openapi/mika-server.yaml` (crate-local for crates.io) must match. `scripts/sync-agent-docs.sh` handles the copy. The `test_committed_spec_is_current` test only validates the canonical copy.

## Prevention

- **Always add `#[utoipa::path]` when creating new Axum handlers.** If a handler should appear in the OpenAPI spec, annotate it at creation time — not after the fact.
- **Always register in `AgentApiDoc` paths().** Adding the annotation is not enough — the handler must also be listed in the `paths(...)` attribute in `openapi.rs`. Register request/response types in `components(schemas(...))` too.
- **The `test_committed_spec_is_current` test** catches drift between generated and committed specs, but only for endpoints that have annotations AND are registered. It cannot detect missing annotations or missing registrations.
- **The `test_agent_openapi_yaml_contains_endpoints` test** serves as an explicit endpoint inventory — add a new assertion whenever a new endpoint is annotated. This is the best defense against variant #328 (annotated but not registered).

## Related

- [Gateway monorepo migration](gateway-monorepo-migration.md) — documents the same `test_committed_spec_is_current` pattern for mika-gateway
- [Embed dashboard SPA](../architecture-patterns/embed-dashboard-spa-rust-embed.md) — documents the dashboard route architecture

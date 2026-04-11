---
title: "fix: register handle_task_complete in OpenAPI spec paths"
type: fix
status: completed
date: 2026-04-11
---

# fix: register handle_task_complete in OpenAPI spec paths

`handle_task_complete` in `crates/mika-agent/src/server/handlers.rs` has a `#[utoipa::path]` annotation but is **not registered** in the `AgentApiDoc` `paths()` macro in `crates/mika-agent/src/server/openapi.rs`. This means `POST /tasks/{id}/complete` does not appear in `docs/openapi/mika-server.yaml` despite being fully annotated.

The `self-knowledge` skill serves the OpenAPI spec via `get_documentation("api-spec")`. Agents and external consumers can't discover the task completion endpoint.

## Acceptance Criteria

- [x] `handlers::handle_task_complete` added to `paths()` in `openapi.rs`
- [x] `TaskCompleteRequest` and `TaskCompleteResponse` added to `components(schemas(...))` in `openapi.rs`
- [x] `test_agent_openapi_yaml_contains_endpoints` asserts `/tasks/{id}/complete` is present
- [x] Spec regenerated: `cargo test -p mika-agent --lib -- write_agent_openapi_yaml --ignored`
- [x] Crate-local copy synced: `./scripts/sync-agent-docs.sh`
- [x] `test_committed_spec_is_current` passes
- [x] `cargo clippy` clean

## MVP

### 1. `crates/mika-agent/src/server/openapi.rs` — register path + schemas

```rust
paths(
    handlers::handle_health,
    handlers::handle_message,
    handlers::handle_task_complete,  // <-- add
    embedded_dashboard::handle_enable,
    embedded_dashboard::handle_disable,
    embedded_dashboard::handle_status,
),
components(schemas(
    types::MessageRequest,
    types::AcceptedResponse,
    types::HealthResponse,
    types::TaskCompleteRequest,   // <-- add
    types::TaskCompleteResponse,  // <-- add
)),
```

### 2. `crates/mika-agent/src/server/openapi.rs` — add test assertion

```rust
assert!(
    yaml.contains("/tasks/{id}/complete"),
    "missing /tasks/{id}/complete endpoint"
);
```

### 3. Regenerate spec & sync

```bash
cargo test -p mika-agent --lib -- write_agent_openapi_yaml --ignored
./scripts/sync-agent-docs.sh
```

## Sources

- Related issue: #328
- Same class of bug as: #321

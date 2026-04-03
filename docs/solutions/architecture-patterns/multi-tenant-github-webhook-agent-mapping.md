---
title: "Multi-tenant GitHub webhook routing with per-repo agent name overrides"
category: architecture-patterns
date: 2026-04-04
tags: [gateway, github, multi-tenant, routing, agent-mapping, jsonb, postgres]
module: mika-gateway
issue: 411
---

# Multi-Tenant GitHub Webhook Agent Mapping

## Problem

The gateway's GitHub webhook handler routes events to hardcoded agent names (`mika-dev`, `mika-qa`) via `route_event()`. In multi-tenant deployments, each customer has differently-named agent containers (e.g., `acme-dev`, `acme-qa`). Without per-repo overrides, all customers must use the same agent names, which conflicts with the per-customer container isolation model.

## Root Cause

Migration 004 established the `github_repos` table for multi-tenant routing (repo -> customer), but only mapped repos to customer containers. The agent name within the container was still hardcoded — the routing resolved WHERE to send, but not WHO to address within the container.

## Solution

### 1. Migration 005: `agent_mapping` JSONB column

```sql
ALTER TABLE github_repos ADD COLUMN agent_mapping JSONB NOT NULL DEFAULT '{}';
```

Schema: keys are default agent names from `route_event()`, values are replacement names.

```json
{"mika-dev": "acme-dev", "mika-qa": "acme-qa"}
```

Empty `{}` preserves defaults. `DEFAULT '{}'` makes the column zero-config for existing rows.

### 2. `ResolvedRoute` struct

Refactored `resolve_github_container_url()` from `Option<String>` to `Option<ResolvedRoute>`:

```rust
struct ResolvedRoute {
    container_url: String,
    agent_mapping: serde_json::Value,
}
```

The query changed from `SELECT customer_id` to `SELECT customer_id, agent_mapping`.

### 3. `apply_agent_mapping()` with validation

```rust
fn apply_agent_mapping(agent_mapping: &serde_json::Value, default_agent: &str) -> String {
    agent_mapping
        .get(default_agent)
        .and_then(|v| v.as_str())
        .filter(|s| is_valid_agent_name(s))
        .unwrap_or(default_agent)
        .to_string()
}
```

Key design decisions:
- **`serde_json::Value` over typed struct** — JSONB content varies; flexible deserialization prevents parsing failures on unexpected keys (lesson from #403 `app_id` incident).
- **`is_valid_agent_name()` validation** — defense-in-depth: rejects names with spaces, special chars, uppercase, consecutive hyphens, or >63 chars. Falls back to default on invalid values rather than forwarding garbage to the agent container.
- **Applied after `route_event()`** — the mapping only remaps agent names, it does not expand which events are routable. `route_event()` remains the single gate for routability.

### 4. Log level strategy for unregistered repos

- `debug!` when `agent_base_url` is set (single-tenant fallback working as designed)
- `warn!` when no fallback configured (event will be dropped — operational concern)

This prevents log noise in single-tenant deployments where every webhook hits the `Ok(None)` path.

## Key Files

- `crates/mika-gateway/migrations/005_github_repos_agent_mapping.sql`
- `crates/mika-gateway/src/github.rs` — `ResolvedRoute`, `apply_agent_mapping()`, `is_valid_agent_name()`

## Prevention / Best Practices

1. **Use `DEFAULT '{}'` for JSONB columns** — ensures backward compatibility without data migration.
2. **Validate JSONB-sourced values at the application layer** — even operator-managed tables can contain typos. Gateway-level validation catches config errors early rather than producing silent 404s at the agent container.
3. **Use `serde_json::Value` for flexible JSONB** — strongly-typed deserialization fails on unexpected keys. For JSONB that may evolve, use `Value` and validate programmatically.
4. **Log level should match operational impact** — `debug` for expected fallback paths, `warn` for events that will be silently dropped.

## Related

- `docs/solutions/architecture-patterns/github-webhook-endpoint-gateway.md` — base webhook handler architecture
- `docs/solutions/architecture/github-app-identity-and-agent-infrastructure.md` — migration 004 and multi-tenant routing design
- `docs/solutions/runtime-errors/github-webhook-parse-fails-missing-app-id.md` — lesson on flexible deserialization
- `docs/solutions/build-errors/sqlx-migration-stale-incremental-cache.md` — `build.rs` requirement for new migrations

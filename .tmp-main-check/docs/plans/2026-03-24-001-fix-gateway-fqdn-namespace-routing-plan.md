---
title: "fix: Gateway should use FQDN with MIKA_AGENTS_NAMESPACE for cross-namespace routing"
type: fix
status: completed
date: 2026-03-24
---

# fix: Gateway should use FQDN with MIKA_AGENTS_NAMESPACE for cross-namespace routing

## Overview

The gateway constructs agent service URLs using short DNS (`http://mika-{customer_id}:8080`) which fails when gateway and agent pods are in different environment-scoped namespaces (e.g. `mika-system-prd` vs `mika-agents-prd`). mika-cloud PR #48 already injected `MIKA_AGENTS_NAMESPACE` as an env var into the gateway configmap. This fix makes the gateway Rust code consume it for FQDN construction.

## Problem Statement

Two functions in `crates/mika-gateway/src/routes.rs` construct agent URLs with short DNS:

```rust
// routes.rs:254
None => format!("http://mika-{customer_id}:8080"),

// routes.rs:263
None => format!("http://mika-{customer_id}:8080"),
```

Short DNS (`mika-{id}`) resolves via the pod's DNS search domain, which only includes the pod's own namespace. When the gateway runs in `mika-system-prd` and agents run in `mika-agents-prd`, short DNS fails — FQDN is required: `http://mika-{customer_id}.mika-agents-prd.svc.cluster.local:8080`.

## Proposed Solution

Follow the existing `agent_base_url` pattern — add `agents_namespace` to settings, thread it through `AppState`, and use it in the URL construction functions.

### Changes

#### 1. `crates/mika-gateway/src/settings.rs`

Add `agents_namespace` field to `GatewaySettings`:

```rust
/// Namespace where agent pods run (for FQDN construction).
/// Maps to MIKA_AGENTS_NAMESPACE env var.
#[serde(default = "default_agents_namespace")]
pub agents_namespace: String,
```

Add default function:

```rust
fn default_agents_namespace() -> String {
    "mika-agents".to_string()
}
```

Include in `Debug` impl.

#### 2. `crates/mika-gateway/src/routes.rs`

Add `agents_namespace` field to `AppState`:

```rust
/// Namespace where agent pods run (for FQDN DNS resolution).
pub agents_namespace: String,
```

Update `container_url()` and `container_url_str()` to use FQDN:

```rust
fn container_url(customer_id: &Uuid, agent_base_url: &Option<String>, agents_namespace: &str) -> String {
    match agent_base_url {
        Some(base) => base.clone(),
        None => format!("http://mika-{customer_id}.{agents_namespace}.svc.cluster.local:8080"),
    }
}

pub(crate) fn container_url_str(customer_id: &str, agent_base_url: Option<&str>, agents_namespace: &str) -> String {
    match agent_base_url {
        Some(base) => base.to_string(),
        None => format!("http://mika-{customer_id}.{agents_namespace}.svc.cluster.local:8080"),
    }
}
```

Update all call sites (3 in `routes.rs`, 2 in `a2a_routes.rs`) to pass `&state.agents_namespace`.

#### 3. `crates/mika-gateway/src/main.rs`

Pass through settings to AppState:

```rust
agents_namespace: settings.agents_namespace.clone(),
```

#### 4. `crates/mika-gateway/src/a2a_routes.rs`

Update 2 call sites to pass the namespace parameter.

#### 5. Tests

Update existing tests and add a test for FQDN construction.

## Acceptance Criteria

- [x] `GatewaySettings` reads `MIKA_AGENTS_NAMESPACE` with default `mika-agents`
- [x] `container_url()` produces `http://mika-{id}.{ns}.svc.cluster.local:8080`
- [x] `container_url_str()` produces the same FQDN format
- [x] `agent_base_url` override still takes precedence (local dev path unchanged)
- [x] All call sites in `routes.rs` and `a2a_routes.rs` pass the namespace
- [x] `cargo test` passes
- [x] `cargo clippy` clean

## Dependencies & Risks

- **Backward compatible**: Default `mika-agents` produces `mika-{id}.mika-agents.svc.cluster.local:8080` — FQDN always works, even in same-namespace setups.
- **Cross-repo dependency**: mika-cloud PR #48 (merged) already injects the env var into the gateway configmap. No further cloud changes needed.

## Sources

- Issue: senara-solutions/mika#251
- Companion PR: senara-solutions/mika-cloud#48
- Gateway route code: `crates/mika-gateway/src/routes.rs:251-265`
- Settings pattern: `crates/mika-gateway/src/settings.rs:38-39` (`agent_base_url`)
- AppState pattern: `crates/mika-gateway/src/routes.rs:42` (`agent_base_url`)

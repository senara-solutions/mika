---
title: Gateway FQDN for cross-namespace agent routing
category: integration-issues
date: 2026-03-24
tags: [gateway, kubernetes, dns, namespace, fqdn, routing]
issue: "#251"
---

# Gateway FQDN for Cross-Namespace Agent Routing

## Problem

The gateway constructed agent service URLs using short DNS: `http://mika-{customer_id}:8080`. This works only when the gateway and agent pods share a namespace (or the DNS search domain includes the agents namespace). With environment-scoped namespaces (`mika-system-prd` / `mika-agents-prd`), cross-namespace DNS resolution requires FQDN.

## Root Cause

The gateway's `container_url()` and `container_url_str()` functions in `crates/mika-gateway/src/routes.rs` used short DNS without a namespace qualifier. Kubernetes short DNS resolves relative to the pod's own namespace, so a gateway pod in `mika-system-prd` cannot reach `mika-{id}` in `mika-agents-prd` without the full `mika-{id}.mika-agents-prd.svc.cluster.local` address.

## Solution

Added `agents_namespace` field to `GatewaySettings` (reads `MIKA_AGENTS_NAMESPACE`, defaults to `mika-agents`) and `AppState`. Updated both URL construction functions to produce FQDN:

```rust
format!("http://mika-{customer_id}.{agents_namespace}.svc.cluster.local:8080")
```

The `agent_base_url` override (used for local dev) still takes precedence — FQDN is only used in the production K8s path.

**Files changed:** `settings.rs` (new field + default), `routes.rs` (AppState field + FQDN in both URL functions + updated tests), `a2a_routes.rs` (pass namespace to URL functions), `main.rs` (thread settings through to AppState).

## Prevention

- When gateway routing depends on K8s namespace topology, always use FQDN — short DNS is fragile across namespace boundaries.
- The companion mika-cloud PR #48 injected the env var into the Helm configmap. Cross-repo changes like this should be tracked together (companion PRs referencing each other).

## Cross-Repo Context

This is the `mika/` side of a two-part fix. mika-cloud PR #48 parameterized Helm chart namespaces and injected `MIKA_AGENTS_NAMESPACE` into the gateway configmap. This PR makes the gateway Rust code consume it.

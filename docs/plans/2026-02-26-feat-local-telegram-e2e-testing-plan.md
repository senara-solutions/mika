---
title: "feat: Add configurable agent URL for local Telegram E2E testing"
type: feat
status: active
date: 2026-02-26
---

# feat: Add configurable agent URL for local Telegram E2E testing

## Overview

Enable local end-to-end testing of the full Telegram pipeline (Telegram -> ngrok -> mika-gateway -> mika-server -> Claude API -> response). The gateway currently hardcodes Kubernetes DNS for agent container routing, which won't resolve locally. A single env var override unblocks local dev.

## Problem Statement

`container_url()` in `crates/mika-gateway/src/routes.rs:163` hardcodes `http://mika-{customer_id}.mika-agents.svc.cluster.local:8080`. This K8s DNS pattern only resolves inside the cluster. For local development, all messages need to route to `http://localhost:8080` (or wherever mika-server runs).

## Proposed Solution

Add `MIKA_AGENT_BASE_URL` optional env var. When set, all messages route to that URL (single-agent local dev). When unset, production K8s DNS pattern is preserved.

## Code Changes

### 1. `crates/mika-gateway/src/settings.rs`

**Add field to `GatewaySettings` (after line 29):**

```rust
/// Override agent container URL for local dev (e.g., "http://localhost:8080")
/// When set, all messages route here instead of using K8s DNS.
pub agent_base_url: Option<String>,
```

**Add to manual `Debug` impl (lines 75-87):**

```rust
.field("agent_base_url", &self.agent_base_url)
```

This is not a secret — it's a URL like `telegram_webhook_url`, safe to log.

### 2. `crates/mika-gateway/src/routes.rs`

**Add field to `AppState` struct (line 30-38):**

```rust
pub agent_base_url: Option<String>,
```

**Add to `AppState` Debug impl (lines 40-47):**

```rust
.field("agent_base_url", &self.agent_base_url)
```

**Update `container_url()` (line 163-165) to accept override:**

```rust
fn container_url(customer_id: &Uuid, agent_base_url: &Option<String>) -> String {
    match agent_base_url {
        Some(url) => url.clone(),
        None => format!("http://mika-{customer_id}.mika-agents.svc.cluster.local:8080"),
    }
}
```

**Update call site in `handle_text_message()` (line 220):**

```rust
let url = container_url(&row.id, &state.agent_base_url);
```

**Update call site in `handle_pairing()` (line 301):**

```rust
let url = container_url(&row.id, &state.agent_base_url);
```

**Update `test_container_url` test (line 525-533):**

```rust
#[test]
fn test_container_url() {
    let id = Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap();

    // Production: K8s DNS
    let url = container_url(&id, &None);
    assert_eq!(
        url,
        "http://mika-12345678-1234-1234-1234-123456789abc.mika-agents.svc.cluster.local:8080"
    );

    // Local dev: override URL
    let url = container_url(&id, &Some("http://localhost:8080".to_string()));
    assert_eq!(url, "http://localhost:8080");
}
```

### 3. `crates/mika-gateway/src/main.rs`

**Thread `agent_base_url` into AppState construction (lines 62-72):**

```rust
let state = AppState {
    pool,
    telegram,
    http_client,
    internal_token: settings.internal_token.clone(),
    webhook_secret: settings.telegram_webhook_secret.clone(),
    ready: ready.clone(),
    webhook_semaphore: Arc::new(tokio::sync::Semaphore::new(30)),
    agent_base_url: settings.agent_base_url.clone(),
};
```

## Acceptance Criteria

- [ ] `MIKA_AGENT_BASE_URL` env var is optional — gateway starts without it
- [ ] When set, all agent requests route to the override URL
- [ ] When unset, K8s DNS pattern is used (no behavior change in production)
- [ ] `cargo test -p mika-gateway` passes (including updated `test_container_url`)
- [ ] `cargo clippy -p mika-gateway` has no warnings
- [ ] Debug impls include the new field (non-redacted, it's a URL)

## Verification

```bash
cargo test -p mika-gateway
cargo clippy -p mika-gateway
```

## Local E2E Testing Guide (post-implementation)

After the code change, the full pipeline can be tested locally:

1. **Postgres:** `createdb mika_gateway` (gateway auto-runs migrations)
2. **Tokens:** `export MIKA_INTERNAL_TOKEN=$(openssl rand -hex 32)` and `export MIKA_TELEGRAM_WEBHOOK_SECRET=$(openssl rand -hex 32)`
3. **ngrok:** `ngrok http 9090` — note the https URL
4. **Gateway (terminal 1):** Start with `MIKA_AGENT_BASE_URL="http://localhost:8080"` plus other env vars
5. **Agent server (terminal 2):** Start mika-server with matching `MIKA_INTERNAL_TOKEN`
6. **Customer:** Insert test customer with pairing token into Postgres
7. **Pair:** Open `https://t.me/<bot>?start=<token>` in Telegram
8. **Test:** Send message in Telegram, verify full round-trip response

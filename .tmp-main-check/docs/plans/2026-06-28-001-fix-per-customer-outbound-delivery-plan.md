# Plan: Fix per-customer outbound delivery (mika#1607)

**Issue:** mika#1607

## Problem

Per-customer agents cannot deliver outbound Telegram replies in multi-bot gateway mode. The agent's `GatewayMessageSender` builds the `/send` payload without `customer_id`, so the gateway cannot resolve the per-customer bot token and returns 400.

The gateway already supports `customer_id` in the `SendPayload` (`routes.rs:1173`). The agent already has the value available via `MIKA_CUSTOMER_ID` → `Settings.customer_id: Option<String>`. The code just never threads it through to the outbound payload.

## Fix shape

Agent-side only. Three construction sites for `GatewayMessageSender` need `customer_id` threaded through. Gateway code is untouched.

## Steps

### 1. Add `customer_id` field to `GatewayMessageSender`

**File:** `crates/mika-agent/src/messaging.rs`

- Add `customer_id: Option<String>` field to the `GatewayMessageSender` struct (line 50-61).
- Add `customer_id: Option<String>` parameter to `GatewayMessageSender::new()` (line 64-82).
- In `MessageSender::send()` impl (line 167), conditionally include `"customer_id"` in the JSON payload when `self.customer_id` is `Some`. Use `serde_json::json!` conditional inclusion or build the payload with `if let Some(cid) = &self.customer_id { payload["customer_id"] = json!(cid); }`.

### 2. Thread `customer_id` from `Settings` at all construction sites

There are four construction sites for `GatewayMessageSender::new()`:

#### 2a. Engine sender (`server/mod.rs:429`)

Pass `settings.customer_id.clone()` as the new parameter. `settings` (the agent-scoped `Settings`) is already in scope at line 429.

#### 2b. Request handler sender (`server/handlers.rs:751`)

Pass `state.settings.customer_id.clone()` (or `a.settings.customer_id.clone()` — the `AgentState.settings` is the per-agent settings loaded at init). The `AgentState` `a` is resolved at line 726.

#### 2c. Failed-sends flush sender (`server/handlers.rs:1001`)

Same pattern: `state.settings.customer_id.clone()` or `agent_state.settings.customer_id.clone()`.

#### 2d. Delegate task sender (`tools/delegate_task.rs:220`)

Pass `self.settings.customer_id.clone()`. The delegate runs in the same container as the orchestrator, so it shares the same `MIKA_CUSTOMER_ID`. `self.settings` is available (the `DelegateTask` struct holds a `Settings` reference).

### 3. Add test

**File:** `crates/mika-agent/src/messaging.rs` (in `#[cfg(test)] mod tests`)

Add a test `test_send_payload_includes_customer_id` that:
1. Creates a `GatewayMessageSender` with `customer_id: Some("test-customer-uuid".to_string())`.
2. Verifies the struct holds the `customer_id` field.
3. Optionally: use a local HTTP mock (or just verify the payload construction) to assert `customer_id` appears in the outbound JSON.

A simpler approach: extract the payload-building logic into a helper method and test that directly, or add a unit test that checks the `serde_json::json!` payload includes the field.

Also add a test `test_send_payload_omits_customer_id_when_none` confirming backward compatibility — when `customer_id` is `None`, the payload does not include the key.

### 4. Update existing tests

All existing tests in `messaging.rs` construct `GatewayMessageSender::new()` with 7 arguments. Add the 8th argument `None` (no `customer_id`) to each call site to maintain backward compatibility. There are 10 test call sites.

## Backward compatibility

When `customer_id` is `None` (CLI/dev mode, no `MIKA_CUSTOMER_ID` set), the payload omits the field entirely. The gateway's `SendPayload.customer_id` is `Option<Uuid>` with `#[serde(default)]`, so an absent field deserializes to `None` and the gateway falls back to its global single-bot path. No behavioral change for existing single-bot deployments.

## Files changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/messaging.rs` | Add `customer_id` field, thread through constructor and payload |
| `crates/mika-agent/src/server/mod.rs` | Pass `customer_id` at engine sender construction |
| `crates/mika-agent/src/server/handlers.rs` | Pass `customer_id` at request handler and flush sender construction |
| `crates/mika-agent/src/tools/delegate_task.rs` | Pass `customer_id` at delegate sender construction |

## Acceptance criteria

- [ ] Per-customer agent (provisioned with `MIKA_CUSTOMER_ID`) includes `customer_id` in the `/send` payload so the gateway resolves the per-customer bot token in multi-bot mode — no 400, no `failed_sends` entry.
- [ ] Single-bot mode (no `MIKA_CUSTOMER_ID`) continues to work unchanged — `customer_id` key omitted.
- [ ] `cargo test -p mika-agent` passes including the new payload tests.

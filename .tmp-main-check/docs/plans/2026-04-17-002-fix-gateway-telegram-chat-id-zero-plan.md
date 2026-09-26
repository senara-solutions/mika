---
title: "fix: Prevent GitHub webhook from poisoning Telegram chat_id"
type: fix
status: active
date: 2026-04-17
issue: 580
---

# fix: Prevent GitHub webhook from poisoning Telegram chat_id

## Overview

GitHub webhook forwarding in the gateway hardcodes `chat_id: 0` in the payload sent to agent containers. The agent's `handle_message` unconditionally stores this value in `customer_config`, overwriting the real Telegram chat_id. All subsequent outbound Telegram notifications fail with `"Bad Request: chat not found"` until the next inbound Telegram message restores the correct value.

## Problem Frame

The gateway must forward GitHub events to agent containers for processing, but GitHub events have no Telegram chat context. The current implementation uses `0` as a sentinel value for `chat_id`, but the agent stores it without validation — poisoning the chat_id used for all outbound Telegram delivery. The fix must make `chat_id` optional in the gateway-to-agent wire format and guard storage on the agent side.

## Requirements Trace

- R1. GitHub webhook forwarding must not corrupt stored Telegram chat_id
- R2. Owner user's Telegram chat_id must resolve to the correct non-zero value after the fix
- R3. `send_message` with no explicit recipient must deliver to the owner via Telegram
- R4. Gateway `/send` must reject `chat_id <= 0` with a clear 400 error before calling Telegram API
- R5. Integration tests must cover the lookup path (mock Telegram API, assert correct chat_id)

## Scope Boundaries

- This fix addresses the chat_id poisoning bug only
- Tool-side error surfacing (making agent `send_message` failures visible to the user) is a separate issue per the acceptance criteria
- No automated cleanup of existing poisoned `customer_config` entries — the next inbound Telegram message restores the correct value
- Agent-side retry logic (retrying permanent 400 errors) is not changed; wasteful but harmless

### Deferred to Separate Tasks

- Tool-side error surfacing for send failures: companion ticket (referenced in issue #580 acceptance)
- DLQ + replay for permanently failed outbound sends: tracked in #590

## Context & Research

### Relevant Code and Patterns

- `crates/mika-gateway/src/github.rs:669-675` — hardcoded `"chat_id": 0` in `forward_to_resolved_route`
- `crates/mika-agent/src/server/handlers.rs:234-238` — unconditional `set_customer_config("chat_id", ...)`
- `crates/mika-agent/src/server/types.rs:13-24` — `MessageRequest` struct with `chat_id: i64`
- `crates/mika-gateway/src/routes.rs:811-901` — `handle_send()` with no chat_id validation
- `crates/mika-gateway/src/routes.rs:903-913` — `SendPayload` struct with `chat_id: i64`
- `crates/mika-agent/src/messaging.rs:65-76` — `resolve_chat_id()` two-tier resolution
- `crates/mika-agent/src/messaging.rs:78-93` — `try_send()` retry logic

### Institutional Learnings

- `docs/solutions/integration-issues/multi-agent-telegram-delivery-and-reply-routing.md` — documents the same class of bug: agent-scoped `customer_config` chat_id can be overwritten by wrong context. Prevention: pass chat_id explicitly rather than relying on DB lookup under wrong keys.
- `docs/solutions/integration-issues/gateway-inbound-webhook-retry-on-429-5xx.md` — gateway delivery is fire-and-forget after 200 OK; if chat_id is invalid, the notification is permanently lost.

## Key Technical Decisions

- **Make `MessageRequest.chat_id` `Option<i64>` with `#[serde(default)]`:** Idiomatic Rust, makes "no chat context" explicit at the type level. `serde(default)` ensures omitted fields deserialize as `None` (not a 400).
- **Omit `chat_id` from GitHub payload (not send `null` or `0`):** Cleaner semantics — GitHub events genuinely have no Telegram chat context. Omitting the field with `#[serde(skip_serializing_if)]` in the JSON construction is straightforward since it uses `serde_json::json!`.
- **Guard storage universally (not just for GitHub channel):** A Telegram update with chat_id=0 would also be malformed. Guarding `if chat_id.is_some_and(|id| id != 0)` is safer than channel-specific logic.
- **Return 400 from gateway `/send` for invalid chat_id:** Clear signal that the error is permanent, not transient. The agent retries once (wasteful but harmless) then saves to `failed_sends`.

## Open Questions

### Resolved During Planning

- **Should `MessageRequest.chat_id` be `Option<i64>` or remain `i64` with sentinel?** `Option<i64>` — idiomatic, prevents sentinel ambiguity.
- **Should GitHub payload omit `chat_id` or send `null`?** Omit — cleaner semantics. `serde_json::json!` macro naturally supports conditional field inclusion.
- **Should existing poisoned entries be cleaned up?** No — the next Telegram message restores the correct value. Manual SQL available if needed: `DELETE FROM customer_config WHERE key = 'chat_id' AND value = '0'`.

### Deferred to Implementation

- Exact error message wording for 400 response on invalid chat_id
- Whether `resolve_chat_id` should also filter `0` values read from DB (defense-in-depth against pre-existing poisoned entries)

## Implementation Units

- [x] **Unit 1: Make `MessageRequest.chat_id` optional**

**Goal:** Change the wire format between gateway and agent to support absent chat_id

**Requirements:** R1

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/server/types.rs`
- Modify: `crates/mika-agent/src/server/handlers.rs`
- Test: `crates/mika-agent/src/server/handlers.rs` (inline tests)

**Approach:**
- Change `MessageRequest.chat_id` from `i64` to `Option<i64>` with `#[serde(default)]`
- In `handle_message`, only call `set_customer_config("chat_id", ...)` when `chat_id.is_some_and(|id| id != 0)`
- Update all references to `req.chat_id` in `handle_message` to handle the `Option`

**Patterns to follow:**
- `MessageRequest.images` already uses `Option<Vec<ImagePayload>>` with `#[serde(default)]` — same pattern
- `MessageRequest.agent` uses `#[serde(default)]` for optional string — same attribute

**Test scenarios:**
- Happy path: message with `chat_id: Some(12345)` stores "12345" in customer_config
- Edge case: message with `chat_id: None` (omitted from JSON) does NOT call set_customer_config for chat_id
- Edge case: message with `chat_id: Some(0)` does NOT store "0" in customer_config
- Edge case: message with `chat_id: Some(-1)` does NOT store "-1" in customer_config (negative group IDs are valid in Telegram but should still be stored)

**Verification:**
- All existing handler tests pass with the updated type
- New tests confirm storage guard behavior

- [x] **Unit 2: Remove hardcoded `chat_id: 0` from GitHub webhook forwarding**

**Goal:** GitHub webhook payloads no longer include a chat_id field

**Requirements:** R1, R2

**Dependencies:** Unit 1 (agent must accept missing chat_id)

**Files:**
- Modify: `crates/mika-gateway/src/github.rs`
- Test: `crates/mika-gateway/src/github.rs` (inline tests)

**Approach:**
- Remove `"chat_id": 0` from the `serde_json::json!` payload in `forward_to_resolved_route`
- The agent's `MessageRequest` with `Option<i64>` + `#[serde(default)]` will deserialize the missing field as `None`

**Patterns to follow:**
- The existing payload construction at `github.rs:669` uses `serde_json::json!` — simply remove the `chat_id` key

**Test scenarios:**
- Happy path: verify the constructed JSON payload for a GitHub webhook forward does NOT contain a `chat_id` field
- Integration: GitHub webhook forward followed by outbound send — stored chat_id from a prior Telegram message is preserved (not overwritten)

**Verification:**
- Gateway compiles and passes existing GitHub webhook tests
- No `chat_id` field in forwarded GitHub payloads

- [x] **Unit 3: Add chat_id validation to gateway `/send` endpoint**

**Goal:** Reject outbound sends with invalid chat_id before calling Telegram API

**Requirements:** R4

**Dependencies:** None (independent of Units 1-2, but logically follows)

**Files:**
- Modify: `crates/mika-gateway/src/routes.rs`
- Test: `crates/mika-gateway/src/routes.rs` (inline tests)

**Approach:**
- Add validation in `handle_send` after payload deserialization: if `chat_id <= 0`, return 400 with a clear error message
- Place this check before the Telegram API call, after existing text and agent_name validation

**Patterns to follow:**
- Existing validation in `handle_send` (text length check at line 816, agent_name format check at line 827) — same early-return pattern with `StatusCode::BAD_REQUEST` and JSON error body

**Test scenarios:**
- Happy path: `/send` with `chat_id: 12345` proceeds to Telegram API call
- Error path: `/send` with `chat_id: 0` returns 400 with error message
- Error path: `/send` with `chat_id: -1` returns 400 with error message (negative IDs are group chats — valid for Telegram, but the gateway routes to individual users only; revisit if group support is added)

**Verification:**
- Invalid chat_id requests are rejected before reaching the Telegram API
- Error response includes actionable message

- [x] **Unit 4: Add integration test for the full lookup path**

**Goal:** End-to-end test proving correct chat_id flows through the system

**Requirements:** R3, R5

**Dependencies:** Units 1-3

**Files:**
- Test: `crates/mika-agent/src/messaging.rs` (inline tests)
- Test: `crates/mika-agent/src/server/handlers.rs` (inline tests)

**Approach:**
- In agent messaging tests: verify `resolve_chat_id()` returns the correct value when DB has a real chat_id (not "0")
- In agent handler tests: sequence test — Telegram message sets chat_id, GitHub message does NOT overwrite it, subsequent resolve_chat_id returns original value
- Leverage existing `MockLlmProvider` and `TestHarness` patterns for agent-side tests

**Patterns to follow:**
- Existing `resolve_chat_id` tests in `messaging.rs:180-199` — three tests covering explicit override, DB fallback, and missing-both-fails
- Existing handler test patterns in the gateway crate

**Test scenarios:**
- Integration: Telegram message with `chat_id: 99999` -> GitHub message with no chat_id -> `resolve_chat_id()` returns 99999 (not 0, not error)
- Integration: GitHub-only agent (never received Telegram) -> `resolve_chat_id()` returns error "chat_id not configured"
- Happy path: `resolve_chat_id()` with DB value "0" (pre-existing poisoned entry) — confirm behavior (returns 0 today; consider filtering)
- Edge case: `resolve_chat_id()` with explicit override of `Some(0)` — returns 0 (explicit override is trusted)

**Verification:**
- All test scenarios pass
- No regressions in existing messaging and handler tests

## System-Wide Impact

- **Interaction graph:** GitHub webhook handler -> agent `handle_message` -> `customer_config` store -> `GatewayMessageSender::resolve_chat_id` -> gateway `/send` -> Telegram API. The fix touches the first three nodes.
- **Error propagation:** Invalid chat_id now produces a 400 at the gateway `/send` boundary. The agent's `try_send` treats this as a failure, retries once, then persists to `failed_sends`. No change to retry behavior.
- **State lifecycle risks:** Pre-existing poisoned `customer_config` entries with `chat_id = "0"` will persist until the next inbound Telegram message. The `resolve_chat_id()` function will read "0" and the gateway will reject it with 400 — correct behavior (fail-closed, not silent corruption).
- **API surface parity:** `MessageRequest` is consumed only by the agent's `handle_message` endpoint. `SendPayload` is consumed only by the gateway's `handle_send`. Both are internal APIs (gateway <-> agent), not public.
- **Unchanged invariants:** The Telegram inbound flow (`/webhook/telegram`) is unchanged — it always sends a real chat_id. The `outbound_messages` table only stores successful sends, so no invalid entries will accumulate.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Rolling deploy: new gateway (omits chat_id) hits old agent (requires chat_id as i64) | `#[serde(default)]` on `Option<i64>` means missing field = `None`, but old agent expects `i64` which would fail deserialization. Deploy agent first, then gateway. |
| Pre-existing poisoned entries cause 400 on next send | Correct behavior — fail-closed. Next Telegram message restores valid chat_id. Operators can run manual SQL cleanup if needed. |

## Documentation / Operational Notes

- Deploy order: agent containers first (accept optional chat_id), then gateway (omit chat_id from GitHub payloads)
- Operators with poisoned entries can run: `DELETE FROM customer_config WHERE key = 'chat_id' AND value = '0'`

## Sources & References

- Related issue: #580
- Related learning: `docs/solutions/integration-issues/multi-agent-telegram-delivery-and-reply-routing.md`
- Related code: `crates/mika-gateway/src/github.rs:669-675`, `crates/mika-agent/src/server/handlers.rs:234-238`

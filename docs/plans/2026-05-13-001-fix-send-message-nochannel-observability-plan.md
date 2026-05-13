---
type: fix
ticket: mika#1090
branch: fix/1090/agent-send-message-on-chat-id-0-github
date: 2026-05-13
---

# Plan: send_message on chat_id=0 emits error log for operator observability

## Context

`send_message` invoked on a session with `chat_id=0` (GitHub webhook-originated sessions) returns `ToolOutput::success` but the message silently disappears — the operator never receives the escalation. This was root-caused in mika#1089: mika-dev fabricated a grooming-rejection `send_message`, it succeeded per the tool output, but the message hit the `NoChannel` path and was lost with only a `warn!` log line (no structured fields beyond `agent_name`).

The ticket scopes this to **Level 1: observability only** — emit an `error!`-level log line with structured fields so the operator can grep and detect lost escalations after the fact. Tool output behavior is explicitly unchanged (still `ToolOutput::success` to prevent LLM retry loops).

## Changes

### 1. Upgrade `NoChannel` log level and add structured fields

**File:** `crates/mika-agent/src/messaging.rs` — `GatewayMessageSender::send()` method, lines 147–152.

**Current code (line 148):**
```rust
warn!(
    agent_name = ?self.agent_name,
    "send() called with chat_id=0 — no reply channel available"
);
```

**New code:**
```rust
error!(
    agent_name = ?self.agent_name,
    request_id = ?self.request_id,
    message_text = %truncate_for_log(text, 500),
    "send_message_nochannel: message lost — chat_id=0, no reply channel available"
);
```

Changes:
- `warn!` → `error!` — `NoChannel` is a permanent message-loss condition, not a transient warning. The operator needs to detect these at the `error` level in log grep workflows.
- Add `request_id` field — correlates to the inbound request that spawned this session (`trace_id` equivalent at the transport layer).
- Add `message_text` field — the escalation content that was lost, truncated to 500 chars to keep log lines manageable.
- Event name `send_message_nochannel` — grep-friendly, matches the existing naming convention in the codebase (e.g., `verdict_classification_failed`, `kg_budget_exhausted`).

**Helper:** Add a `truncate_for_log(text: &str, max_chars: usize) -> &str` private helper in `messaging.rs` that returns a UTF-8-safe prefix. Use `text.char_indices()` to find the boundary at `max_chars` characters, return the byte slice up to that point. This avoids panicking on multi-byte UTF-8 when truncating.

### 2. Upgrade `NoChannel` log in `send_message.rs` tool handler

**File:** `crates/mika-agent/src/tools/send_message.rs` — line 84.

**Current code:**
```rust
warn!("send_message: no reply channel (chat_id=0)");
```

**New code:**
```rust
error!(
    trace_id = %ctx.trace_id,
    session_id = %ctx.session_id,
    "send_message_nochannel: tool returned success but message was NOT delivered — chat_id=0"
);
```

This gives the operator the `trace_id` and `session_id` at the tool level, complementing the transport-level fields emitted in change #1. Between the two log lines, the operator has: `agent_name`, `request_id`, `trace_id`, `session_id`, and `message_text` — enough to reconstruct the full context of a lost escalation.

### 3. Tests

#### 3a. Unit test: `send_message_chat_id_zero_emits_error_log`

**File:** `crates/mika-agent/src/messaging.rs` — add to existing `#[cfg(test)] mod tests` block.

Test that `GatewayMessageSender::send()` with `chat_id=0` (explicit override) returns `Ok(SendOutcome::NoChannel)` and — critically — does NOT attempt an HTTP POST. The existing test `test_send_chat_id_zero_explicit_returns_no_channel` already covers the return value; this new test focuses on verifying the `error!` log emission.

Use `tracing_subscriber::layer::SubscriberExt` with a `tracing::subscriber::with_default` scope and a custom `Layer` that captures events, then assert the event:
- Level is `ERROR`
- Message contains `send_message_nochannel`
- Has `agent_name` field
- Has `request_id` field
- Has `message_text` field

If capturing tracing events in-process is too complex, an alternative is to assert behavior only (return value + no HTTP call) and rely on the existing test structure. The log emission is structural code, not conditional logic, so a compile-time check (the code exists) plus the behavioral test is sufficient.

**Decision:** Given the codebase's existing test patterns (mock-based behavioral tests, not log-capture tests), keep this simple. Extend the existing `test_send_chat_id_zero_explicit_returns_no_channel` test to also verify the mock server received zero requests (it already does this implicitly via `mockito`). The `error!` level upgrade is a one-line structural change that doesn't need its own test — it's verified by code review.

#### 3b. Regression test: `send_message_chat_id_zero_returns_success_tool_output`

**File:** `crates/mika-agent/src/tools/send_message.rs` — add to existing test module.

Explicitly assert Level 1 does NOT change the `ToolOutput` shape:
- Set up a mock `MessageSender` that returns `Ok(SendOutcome::NoChannel)`.
- Call `SendMessageTool.execute()` with valid text input.
- Assert `output.is_error == false` (still success).
- Assert `output.content` contains the redirect guidance text ("No reply channel").

This prevents accidental regression to `ToolOutput::error` which would cause LLM retry loops. The existing `test_send_message_no_channel` test (line 368) already covers this — verify it's sufficient and add a comment explaining the Level 1 regression-prevention intent.

### 4. No-change verification

Confirm these paths are NOT modified:
- Non-zero `chat_id` routing — the `if chat_id == 0` guard is the only change point; the else branch (HTTP POST path) is untouched.
- `ToolOutput` shape — `NoChannel` still returns `ToolOutput::success`.
- `failed_sends` table — `NoChannel` still skips the `save_failed_send` call.
- `SendOutcome` enum — no new variants.

## Scope boundary

This plan implements **Level 1 only** (observability). Levels 2 (fallback channel routing) and 3 (rejecting the call) are explicitly out of scope per the ticket.

## Risk assessment

**Low risk.** The change is two log-level upgrades (`warn!` → `error!`) with additional structured fields. No control flow changes. No new dependencies. No schema changes. The `truncate_for_log` helper is the only new code with any logic, and it's a simple char-boundary truncation.

## Estimated size

~30 lines of production code, ~10 lines of test additions/modifications. Single-file primary change (`messaging.rs`) with a companion change in `send_message.rs`.

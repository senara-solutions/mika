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

## Pinned Source (Phase 0 Pin — architect F1)

### Site A: `messaging.rs` — `GatewayMessageSender::send()`

**Function signature** (line 141):
```rust
async fn send(&self, text: &str) -> Result<SendOutcome>
```

**Fields in scope at the `chat_id == 0` branch (line 147):**
- `self.agent_name: Option<String>` — ✅ available (struct field)
- `self.request_id: Option<String>` — ✅ available (struct field)
- `text: &str` — ✅ available (function parameter — the message content)
- `self.db: AsyncDatabase` — available but not needed for logging
- `self.chat_id: Option<i64>` — available but value is known (resolved to 0)

**No signature change required.** All fields needed for the `error!` log are already in scope as struct fields or function parameters.

**Current `warn!` call (lines 148–151):**
```rust
warn!(
    agent_name = ?self.agent_name,
    "send() called with chat_id=0 — no reply channel available"
);
```

### Site B: `send_message.rs` — tool handler `NoChannel` match arm

**Handler signature** (line 41):
```rust
async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput>
```

**Fields in scope at the `NoChannel` match arm (line 83):**
- `ctx.trace_id: &str` — ✅ available (ToolContext field, per CLAUDE.md)
- `ctx.session_id: &str` — ✅ available (ToolContext field)
- `ctx.db: &AsyncDatabase` — available but not needed
- `cleaned: String` — the processed message text (in scope from line 54)

**No signature change required.** `trace_id` and `session_id` are standard ToolContext fields available to all tool handlers.

**Current `warn!` call (line 84):**
```rust
warn!("send_message: no reply channel (chat_id=0)");
```

### Field distribution across sites

| Field | Site A (messaging.rs) | Site B (send_message.rs) | Reason for placement |
|-------|----------------------|-------------------------|---------------------|
| `agent_name` | ✅ emitted | ❌ not emitted | Available on `self`; not on `ToolContext` |
| `request_id` | ✅ emitted | ❌ not emitted | Available on `self`; not on `ToolContext` |
| `message_text` | ✅ emitted (truncated) | ❌ not emitted | `text` param available; avoids duplication |
| `trace_id` | ❌ not emitted | ✅ emitted | Not on `GatewayMessageSender`; available on `ToolContext` |
| `session_id` | ❌ not emitted | ✅ emitted | Not on `GatewayMessageSender`; available on `ToolContext` |

Fields are placed where they're naturally in scope. No field requires threading through a new parameter.

## Dual-Emission Design (architect F2)

### Co-trigger analysis

**Tool-handler path (most common):** `SendMessageTool::execute()` → `sender.send()` → both sites fire. This is the only path where Site B fires.

**Non-tool-handler callers of `GatewayMessageSender::send()`:**
- `failed_sends` flush path (`handlers.rs` line ~933) — retries previously-failed deliveries. These already resolved to a non-zero `chat_id` on original send (failed due to HTTP error, not `NoChannel`). `NoChannel` messages are never saved to `failed_sends` (the sender returns early before the retry/save logic). **Site A will NOT fire on this path.**
- Task engine notification paths — use `send_message` tool via the agent loop, not direct `GatewayMessageSender::send()` calls. **Both sites fire.**

**Conclusion:** In practice, both sites always co-fire for `NoChannel` events. There is no code path where Site A fires without Site B. Site A is the *comprehensive* site (closest to the transport decision); Site B is the *enrichment* site (adds agent-turn context).

### Correlation strategy

The two log lines share `agent_name` (Site A) and are emitted within microseconds of each other on the same tokio task. Operators can correlate by:
1. **Timestamp + agent_name** — sufficient for low-volume environments (current production: single agent per container).
2. **Future improvement (out of scope):** Thread `trace_id` onto `GatewayMessageSender` to enable single-field correlation. This would be a constructor change affecting all instantiation sites — deferred to Level 2 if needed.

### Alert acknowledgment

Each lost `send_message` produces **two** `error!` log lines. This is accepted as defense-in-depth:
- Site A carries the message content (what was lost).
- Site B carries the trace/session context (where it was lost).
- Alert deduplication (if configured) can key on the shared event name `send_message_nochannel` + `agent_name` + timestamp window.

## Changes

### 1. Upgrade `NoChannel` log level and add structured fields (Site A)

**File:** `crates/mika-agent/src/messaging.rs` — `GatewayMessageSender::send()` method, lines 147–152.

**New code replacing the `warn!` call:**
```rust
// Operator-visible: message content that was lost. Truncated to 500 chars
// because the operator needs to assess severity of the lost escalation,
// and most escalation messages fit within this budget.
error!(
    agent_name = ?self.agent_name,
    request_id = ?self.request_id,
    message_text = %truncate_for_log(text, 500),
    "send_message_nochannel: message lost — chat_id=0, no reply channel available"
);
```

Changes:
- `warn!` → `error!` — `NoChannel` is a permanent message-loss condition, not a transient warning. The operator needs to detect these at the `error` level in log grep workflows. Consistent with mika#1088 (billing errors promoted to `error!` for the same consequence-driven rationale).
- Add `request_id` field — correlates to the inbound request that spawned this session.
- Add `message_text` field — the escalation content that was lost, truncated to 500 chars.
- Event name `send_message_nochannel` — grep-friendly, matches the existing naming convention (e.g., `verdict_classification_failed`, `kg_budget_exhausted`).

**Helper:** Add a `truncate_for_log(text: &str, max_chars: usize) -> &str` private helper in `messaging.rs` that returns a UTF-8-safe prefix. Use `text.char_indices()` to find the boundary at `max_chars` characters, return the byte slice up to that point. This avoids panicking on multi-byte UTF-8 when truncating.

### 2. Upgrade `NoChannel` log in `send_message.rs` tool handler (Site B)

**File:** `crates/mika-agent/src/tools/send_message.rs` — line 84.

**New code replacing the `warn!` call:**
```rust
error!(
    trace_id = %ctx.trace_id,
    session_id = %ctx.session_id,
    "send_message_nochannel: tool returned success but message was NOT delivered — chat_id=0"
);
```

This gives the operator the `trace_id` and `session_id` at the tool level, complementing the transport-level fields emitted at Site A. Between the two log lines, the operator has: `agent_name`, `request_id`, `trace_id`, `session_id`, and `message_text` — enough to reconstruct the full context of a lost escalation.

### 3. Tests

#### 3a. Regression test: `ToolOutput::success` preserved (architect F6 + ticket AC)

**File:** `crates/mika-agent/src/tools/send_message.rs` — existing test `test_send_message_no_channel` (line 368).

The existing test already asserts:
- Mock `MessageSender` returns `Ok(SendOutcome::NoChannel)`
- `output.is_error == false` (success)
- `output.content` contains the redirect guidance text

**Addition:** Add a comment to the existing test explaining the Level 1 regression-prevention intent:
```rust
// Level 1 regression guard (mika#1090): NoChannel MUST return ToolOutput::success,
// not ToolOutput::error. Returning error causes LLM retry loops because chat_id=0
// is a permanent session condition. Level 3 (rejecting the call) would change this
// intentionally — that's a separate ticket with retry-semantic coupling analysis.
```

#### 3b. Log emission shape test: `send_message_nochannel_emits_structured_error`

**File:** `crates/mika-agent/src/messaging.rs` — add to existing `#[cfg(test)] mod tests` block.

The codebase does not currently have a pattern for capturing tracing events in tests. Rather than introducing a new `tracing-test` dependency or custom subscriber for a single test, use a pragmatic approach:

**Test the `truncate_for_log` helper directly** (the only new logic):
- Empty string → returns empty
- String shorter than limit → returns full string
- String exactly at limit → returns full string
- String longer than limit → returns truncated prefix at char boundary
- String with multi-byte UTF-8 (e.g., emoji, CJK) longer than limit → truncates at char boundary without panic

**For the `error!` emission itself:** The log call is structural code (not conditional logic) — it's the same `error!` macro invocation that fires unconditionally on the `NoChannel` path. The behavioral assertion (existing test: `SendOutcome::NoChannel` returned, no HTTP POST made) plus the `truncate_for_log` unit tests provide sufficient coverage. The `error!` level is verified by code review, not runtime assertion.

#### 3c. No-regression on non-zero chat_id paths

**File:** `crates/mika-agent/src/messaging.rs` — existing tests `test_send_success` and `test_send_gateway_failure`.

Verify these existing tests still pass unchanged — they exercise the non-zero `chat_id` path and must not be affected by the `NoChannel` log changes. No modifications needed.

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

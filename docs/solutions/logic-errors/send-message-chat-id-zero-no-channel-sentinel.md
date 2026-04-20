---
title: send_message propagates gateway 400 instead of structured NoChannel for chat_id == 0
date: 2026-04-20
category: logic-errors
module: crates/mika-agent/src/messaging.rs
problem_type: logic_error
component: tooling
symptoms:
  - "gateway /send returned 400 Bad Request: chat_id must be non-zero"
  - "send_message tool returns ToolOutput::error with opaque gateway error on GitHub webhook sessions"
  - "LLM sees infrastructure failure instead of actionable redirect to channel-appropriate tools"
  - "2-second retry delay wasted on permanent condition, failed_sends table polluted with undeliverable entries"
root_cause: logic_error
resolution_type: code_fix
severity: high
tags:
  - send-message
  - chat-id
  - no-channel
  - gateway-400
  - github-webhook
  - sentinel-value
  - send-outcome
---

# send_message propagates gateway 400 instead of structured NoChannel for chat_id == 0

## Problem

`send_message` invoked with `chat_id == 0` (the documented sentinel for "no reply channel" on GitHub webhook sessions) triggered a gateway 400 error that propagated back to the LLM as a generic delivery failure. 10 occurrences in 24 hours across callbacks, heartbeats, and CLI sessions — notifications silently lost.

## Symptoms

- `send_message` tool returns `ToolOutput::error("Message delivery failed: gateway /send returned 400 Bad Request: {\"error\":\"chat_id must be non-zero\"}")`
- Agent retries uselessly (2-second sleep), saves to `failed_sends` (futile for permanent condition)
- LLM sees opaque infrastructure error, cannot pivot to `run_gh` or other channel-appropriate tools
- `failed_sends` flush re-attempts delivery on every subsequent message, accumulating futile retries

## What Didn't Work

- The gateway's 400 response for `chat_id == 0` was correct validation — the bug was agent-side, not gateway-side. The gateway should continue rejecting `chat_id == 0` as defense-in-depth.
- Silent drop was explicitly rejected: documented anti-pattern in `docs/solutions/channels/multi-agent-telegram-delivery-and-reply-routing.md`. Drops hide defects and kill observability.
- Re-routing at the gateway (sending to an alternate channel) couples gateway delivery logic to channel semantics — violates the "gateway is a dumb delivery layer" principle.

## Solution

Added `SendOutcome::NoChannel` variant to the existing `SendOutcome` enum, detected in `GatewayMessageSender::send()` before the HTTP POST:

```rust
// In SendOutcome enum (messaging.rs)
pub enum SendOutcome {
    Delivered,
    Failed { reason: String },
    NoChannel,  // chat_id == 0 sentinel — permanent, no retry
}

// In GatewayMessageSender::send() — after resolve_chat_id(), before HTTP POST
if chat_id == 0 {
    warn!(agent_name = ?self.agent_name, "send() called with chat_id=0");
    return Ok(SendOutcome::NoChannel);
}
```

**Key design decisions:**

1. **Check in `send()`, not the tool handler** — single choke point catches all 7 callers (tool, task engine, verdict/CI handlers, failed_sends flush).

2. **`ToolOutput::success` (not error)** — follows the existing "no sender" precedent. `chat_id == 0` is permanent; `ToolOutput::error` would cause LLM retry loops. Tool output: "No reply channel for this session (chat_id is zero). Use channel-appropriate tools (e.g., run_gh for GitHub) to deliver your response."

3. **No retry, no `failed_sends`** — early return before HTTP POST, retry, and DB write. The `failed_sends` flush path deletes NoChannel entries (cleaning up pre-fix entries).

All 7 `SendOutcome` match sites updated. Rust exhaustive matching enforces coverage at compile time.

## Why This Works

`chat_id == 0` is the documented sentinel from the GitHub webhook design (#580) meaning "this session has no Telegram reply channel." The gateway correctly rejects it — but the agent-side `GatewayMessageSender::send()` didn't recognize it as an expected, permanent condition. It treated the 400 as a transient failure: retried, saved to `failed_sends`, and returned `SendOutcome::Failed`.

The fix intercepts at the correct architectural layer — after `resolve_chat_id()` returns the value but before any network call — and returns a typed outcome that each caller handles appropriately:
- Tool handler: success with redirect text (LLM can pivot to `run_gh`)
- Task engine dispatcher: log and continue (fire-and-forget)
- Verdict/CI handlers: log and continue (merge proceeds, notification skipped)
- Failed_sends flush: delete entry (permanent condition, not retryable)

## Prevention

- **New `SendOutcome` variants require exhaustive handling.** Rust's match enforcement guarantees all callers are updated at compile time. Grep for `SendOutcome` match sites when extending the enum.
- **Sentinel values (0, -1) should be caught at the sender layer, not the gateway.** The gateway's 400 is defense-in-depth; the agent-side check is the primary guard.
- **Permanent conditions return `ToolOutput::success` with explanation, not `ToolOutput::error`.** The "no sender" pattern at `send_message.rs:90` established this convention — errors trigger LLM retries on permanent conditions.
- **7 callsites for `SendOutcome`** across 5 files as of #650: `send_message.rs`, `handlers.rs` (3 sites), `verdict_handler.rs`, `ci_success_handler.rs`, `dispatcher.rs`. Check all when modifying the enum.

## Related Issues

- #650 — This fix
- #580 — GitHub webhook chat_id poisoning (gateway-side defense-in-depth)
- #581 — `SendOutcome` enum introduction (false success on gateway error)
- `docs/solutions/logic-errors/send-message-tool-false-success-on-gateway-error.md` — Companion: introduced the `SendOutcome` enum
- `docs/solutions/integration-issues/github-webhook-poisons-telegram-chat-id.md` — Companion: gateway-side chat_id == 0 validation
- `docs/solutions/integration-issues/multi-agent-telegram-delivery-and-reply-routing.md` — Anti-pattern rules ("Treat None as deliberate", "failed_sends silently dropped")

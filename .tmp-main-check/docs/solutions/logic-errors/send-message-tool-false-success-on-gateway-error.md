---
title: send_message tool reports success on gateway non-2xx responses
module: crates/mika-agent/src/tools/send_message.rs
date: 2026-04-17
problem_type: logic_error
component: tooling
severity: high
symptoms:
  - "send_message tool returns success=1 and 'Message sent.' when gateway responds with 502"
  - "Agent proceeds confidently after failed delivery, no retry or user notification"
  - "Gateway logs show non-2xx responses but tool_calls table shows success=true"
root_cause: logic_error
resolution_type: code_fix
tags:
  - send-message
  - gateway
  - tool-output
  - error-handling
  - message-sender
  - false-success
related_components:
  - crates/mika-agent/src/messaging.rs
  - crates/mika-agent/src/server/handlers.rs
  - crates/mika-agent/src/task_engine/dispatcher.rs
---

# send_message tool reports success on gateway non-2xx responses

## Problem

The `send_message` tool returned `ToolOutput::success("Message sent.")` regardless of whether the gateway actually delivered the message. When the gateway returned non-2xx (e.g., 502 Bad Gateway), `GatewayMessageSender::send()` saved the message to `failed_sends` and returned `Ok(())`, hiding the delivery failure from the agent. The agent then claimed delivery succeeded, violating the grounding rule.

## Symptoms

- `send_message` tool_call records show `success=1` when gateway returns 502
- Agent tells user "message sent" when the message was never delivered
- Gateway logs contain `WARN response status=502` but tool telemetry shows no failure
- No retry or error notification path activates because the tool reports success

## What Didn't Work

The original design intentionally returned `Ok(())` from `GatewayMessageSender::send()` on both retry failures, with a comment: "Return Ok — message queued, don't confuse Claude." This was based on the assumption that returning an error would cause the LLM to retry in a loop. However, the LLM can handle `ToolOutput::error` gracefully — it informs the user of the failure rather than blindly retrying.

## Solution

### 1. Rich return type instead of `Result<()>`

Changed `MessageSender::send()` from `Result<()>` to `Result<SendOutcome>` where:

```rust
pub enum SendOutcome {
    Delivered,           // Gateway returned 2xx
    Failed { reason: String },  // Non-2xx after retries, saved to failed_sends
}
```

`Err` is reserved for infrastructure failures (chat_id resolution, DB errors) — not delivery failures.

### 2. Tool surfaces delivery failures

```rust
// Before: sender.send(&cleaned).await?;
// After:
match sender.send(&cleaned).await {
    Ok(SendOutcome::Delivered) => Ok(ToolOutput::success("Message sent.")),
    Ok(SendOutcome::Failed { reason }) => Ok(ToolOutput::error(
        format!("Message delivery failed: {reason}")
    )),
    Err(e) => Ok(ToolOutput::error(format!("Message delivery error: {e}"))),
}
```

### 3. Error classification in `try_send()`

Gateway errors include HTTP status + truncated body snippet (first 200 chars). Network errors are classified:

- Connection error: `"gateway unreachable (connection error): {e}"`
- Timeout: `"gateway request timed out: {e}"`
- Other: `"gateway request failed: {e}"`

### 4. `save_failed_send` error does not mask gateway error

The DB write for `failed_sends` uses `if let Err` instead of `?`, so a DB failure logs a warning but still returns the original gateway error reason to the caller:

```rust
let reason = e.to_string();
if let Err(db_err) = self.db.save_failed_send(text, None).await {
    warn!(error = %db_err, "failed to save to failed_sends table");
}
Ok(SendOutcome::Failed { reason })
```

### 5. All callsites updated

| Callsite | Policy |
|----------|--------|
| `send_message` tool | Surfaces `Failed` as `ToolOutput::error` |
| Task-engine dispatcher | Fire-and-forget — logs warning, returns `Ok(())` |
| Server EndTurn handler | Logs warning on `Failed` |
| Verdict/CI notification handlers | Log warning on `Failed` |
| `failed_sends` flush | Increments retry count on `Failed` |

## Why This Works

The root cause was a `Result<()>` return type that conflated "delivery succeeded" with "delivery failed but we saved it for later." The `SendOutcome` enum makes the distinction explicit at the type level. Each callsite can then apply the appropriate policy: the tool surfaces errors to the LLM, while fire-and-forget paths absorb them.

The `failed_sends` table continues to be populated for server-side retry flush, providing belt-and-suspenders recovery. The agent also knows delivery failed and can inform the user.

## Prevention

1. **HTTP status codes are well-defined errors.** Never swallow non-2xx responses as success. This is an explicit codebase convention — see `docs/solutions/logic-errors/exec-handler-stdout-discarded-on-nonzero-exit.md` for the carve-out that exec handler exit codes are ambiguous but HTTP status codes are not.
2. **Use rich return types instead of `Result<()>` for operations with meaningful non-error outcomes.** A `Result<()>` that returns `Ok(())` for both "succeeded" and "failed but queued" is a lie at the type level. Use an enum.
3. **When changing a trait return type, grep for all `impl` blocks and all callsites.** The `MessageSender` trait had implementations in 4 files and callsites in 7 files. Missing one (handlers.rs EndTurn path) was caught during review.
4. **Never let a persistence error mask the original failure.** If `save_failed_send` fails, the caller should still see the gateway error, not a DB error. Use `if let Err` for best-effort persistence, not `?`.

## Related

- [tool-call-success-contradicts-non-zero-exit](tool-call-success-contradicts-non-zero-exit.md) — same class of bug: derived `success` field didn't consider actual outcome
- [exec-handler-stdout-discarded-on-nonzero-exit](exec-handler-stdout-discarded-on-nonzero-exit.md) — establishes the "HTTP status codes are well-defined errors" convention
- [grounding-rule-downstream-state-hallucination](../prompt-engineering/grounding-rule-downstream-state-hallucination.md) — grounding rule is secondary defense; primary defense must be tool-level accuracy
- GitHub issue: [#581](https://github.com/senara-solutions/mika/issues/581)

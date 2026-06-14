---
title: send_message on chat_id=0 silently succeeds — operator never receives escalation
date: 2026-05-13
category: logic-errors
module: crates/mika-agent/src/messaging.rs
problem_type: logic_error
component: tooling
symptoms:
  - "send_message on chat_id=0 returns ToolOutput::success but message is silently lost"
  - "Operator never receives escalation notifications on GitHub webhook-originated sessions"
  - "Only a warn! log line (no structured fields beyond agent_name) — insufficient for operator grep workflows"
  - "Autonomous loop stalls for hours when escalation (e.g., grooming rejection) hits NoChannel"
root_cause: logic_error
resolution_type: code_fix
severity: high
tags:
  - send-message
  - nochannel
  - chat-id-zero
  - observability
  - escalation
  - error-log
  - github-webhook
  - operator-visibility
---

# send_message on chat_id=0 silently succeeds — operator never receives escalation

## Problem

`send_message` invoked on a session with `chat_id=0` (GitHub webhook-originated sessions) returned `ToolOutput::success` but the message never reached any user-visible channel. The only log evidence was a `warn!`-level line with no structured fields beyond `agent_name` — insufficient for operator detection. Root-caused via mika#1089: mika-dev's grooming-rejection `send_message` hit the `NoChannel` path and was lost for 2h45min before manual intervention.

## Symptoms

- `send_message` returns `ToolOutput::success` on `chat_id=0` but no message is delivered
- Operator has no visibility into lost escalations without manual investigation
- `warn!` log line lacks `trace_id`, `session_id`, `request_id`, and message content
- Autonomous loop escalation paths (grooming rejection, pipeline failure, asserted unavailability) silently fail on webhook sessions
- `tool_calls` table records `success=1` with no failure signal

## What Didn't Work

- **Level 2 (fallback channel routing)** was considered but deferred — it couples to multi-tenant routing decisions (whose Telegram does a tenant agent escalate to?) and is a separate UX decision.
- **Level 3 (rejecting the call with ToolOutput::error)** was considered but deferred — it couples to LLM retry semantics since `chat_id=0` is a permanent session condition. Returning error causes retry loops.
- The existing `warn!` log was inadequate because operators filter by `error` level in production log grep workflows, and the line lacked the structured fields needed to reconstruct the context of a lost escalation.

## Solution

Upgraded the two `warn!` calls on the `NoChannel` path to `error!` with structured fields (Level 1: observability only). `ToolOutput::success` behavior is explicitly preserved.

**Site A — `messaging.rs` (transport layer):**

```rust
error!(
    agent_name = ?self.agent_name,
    request_id = ?self.request_id,
    message_text = %truncate_for_log(text, 500),
    "send_message_nochannel: message lost — chat_id=0, no reply channel available"
);
```

**Site B — `send_message.rs` (tool handler):**

```rust
error!(
    trace_id = %ctx.trace_id,
    session_id = %ctx.session_id,
    "send_message_nochannel: tool returned success but message was NOT delivered — chat_id=0"
);
```

**UTF-8-safe truncation helper:**

```rust
fn truncate_for_log(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => &text[..byte_idx],
        None => text,
    }
}
```

Between the two log lines, the operator has: `agent_name`, `request_id`, `trace_id`, `session_id`, and `message_text` — enough to reconstruct the full context of any lost escalation.

## Why This Works

The `NoChannel` path (#650) was architecturally correct — it prevented gateway 400 errors and LLM retry loops. But it treated message loss as a transient warning when it's actually a permanent delivery failure with operator-impact consequences. Every autonomous-loop escalation that relies on `send_message` (grooming rejection, pipeline failure, asserted unavailability, required-tools-gate rejection) is unreliable on webhook sessions without operator visibility.

Promoting to `error!` with structured fields makes these events:
1. **Detectable** via `grep send_message_nochannel server.log` or `jq 'select(.fields.message == "send_message_nochannel: message lost")' server.log`
2. **Diagnosable** — the two correlated log lines carry all 5 fields needed to reconstruct the lost escalation
3. **Alertable** — `error!` level integrates with existing log-monitoring infrastructure

The dual-site emission is intentional defense-in-depth: Site A carries what was lost (message content), Site B carries where it was lost (trace/session context).

## Prevention

- **Log level reflects consequence, not transience.** `NoChannel` is permanent message loss — `error!` is the correct level. `warn!` should be reserved for conditions the system can recover from automatically.
- **Grep for `send_message_nochannel` after deploys** to validate the fix is working: `jq 'select(.fields.message | test("send_message_nochannel"))' < $MIKA_SPIRIT_LOG_FILE`
- **Future Level 2/3 work is filed separately.** Level 2 (fallback channel routing) and Level 3 (rejecting the call) are out of scope for this fix — each has its own coupling analysis.

## Related Issues

- #1090 — This fix
- #1089 — Ready-label dispatch fabrication that exposed the silent escalation loss
- #650 — Original `NoChannel` sentinel introduction (companion doc: `docs/solutions/logic-errors/send-message-chat-id-zero-no-channel-sentinel.md`)
- #907 — Guard OR-shape that depends on `send_message` reaching the operator
- `docs/solutions/workflow-issues/ready-label-dispatch-requires-grooming-marker-2026-04-30.md` — Documents the NoChannel gap in § "Why This Matters"

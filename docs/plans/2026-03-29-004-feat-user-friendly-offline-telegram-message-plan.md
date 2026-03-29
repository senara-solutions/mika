---
title: "feat: Return user-friendly Telegram message when agent is offline"
type: feat
status: completed
date: 2026-03-29
issue: "#309"
---

# feat: Return user-friendly Telegram message when agent is offline

## Overview

When an agent container is unreachable (scaled to 0, deprovisioned, or DNS not resolving), the gateway currently sends a generic "I'm having trouble right now. Please try again in a moment." message via Telegram. This is the same message used for DB errors, Telegram API errors, and other transient failures — it gives no signal that the agent is offline.

This change adds a pure classifier function that distinguishes connect errors (connection refused, DNS failure) from other transient errors (timeout, broken pipe) and returns an appropriate user-facing message for each case.

## Problem Statement

Users receiving the generic transient error message when their agent is offline have no way to:
- Know their agent is down vs experiencing a momentary glitch
- Take appropriate action (contact admin, check subscription)
- Avoid repeatedly retrying when the agent won't recover on its own

## Proposed Solution

Add a `forward_error_message(is_connect: bool) -> &'static str` pure function and use it in the `handle_forward_result` `Err(e)` branch. The function takes a `bool` (not `&reqwest::Error`) to enable pure sync unit testing, consistent with existing gateway test patterns.

### Files to Modify

- `crates/mika-gateway/src/routes.rs` — add function, modify error branch, add tests

### Implementation Steps

#### 1. Add `forward_error_message` function

Add after `reply_transient_error` (around line 896):

```rust
/// Classify a forwarding error into a user-facing reply message.
/// Connect errors (connection refused, DNS failure) indicate the agent is offline.
/// Other errors (timeout, broken pipe) are transient.
fn forward_error_message(is_connect: bool) -> &'static str {
    if is_connect {
        "Your Mika assistant is currently offline. \
         Please contact your administrator or check your subscription status \
         at console.getmika.ai."
    } else {
        "I'm having trouble right now. Please try again in a moment."
    }
}
```

#### 2. Modify `handle_forward_result` `Err(e)` branch

Replace the `Err(e)` branch (lines 359-364) to use `e.is_connect()` for classification:

```rust
Err(e) => {
    reset_dedup(state, customer_id, update_id).await;
    let is_connect = e.is_connect();
    warn!(error = %e, %customer_id, is_connect, "container unreachable for {msg_kind}, dedup reset");
    let msg = forward_error_message(is_connect);
    let _ = state.telegram.send_message(chat_id, msg).await;
}
```

Key changes:
- Extract `is_connect` from the reqwest error
- Add `is_connect` to the structured log for observability
- Use `forward_error_message` instead of `reply_transient_error`
- Do NOT modify the `Ok(resp)` error branch — container returned HTTP error means it's running

#### 3. Add unit tests

Two sync tests in the existing `mod tests` block:

```rust
#[test]
fn test_forward_error_message_connect() {
    let msg = forward_error_message(true);
    assert!(msg.contains("offline"), "connect errors should mention offline");
    assert!(msg.contains("console.getmika.ai"), "should include console URL");
}

#[test]
fn test_forward_error_message_other() {
    let msg = forward_error_message(false);
    assert!(msg.contains("try again"), "non-connect errors should suggest retry");
    assert!(!msg.contains("offline"), "non-connect errors should not mention offline");
}
```

### What NOT to change

- `reply_transient_error` — used by `resolve_customer` (DB errors), `handle_pairing` (pairing errors), and the `Ok(resp)` error branch where the container is running but unhealthy
- No new dependencies
- No changes to `telegram.rs` or other gateway files

## Technical Notes

- `reqwest::Error::is_connect()` returns `true` for both connection refused AND DNS resolution failure — no need to distinguish them
- The `bool` parameter design follows existing gateway test patterns (pure sync tests, no mocking)
- The `Ok(resp)` error branch (4xx/5xx from container) correctly keeps using `reply_transient_error` — an HTTP response means the container is running

## Acceptance Criteria

- [x] `forward_error_message(true)` returns the offline message with console URL
- [x] `forward_error_message(false)` returns the generic transient retry message
- [x] `handle_forward_result` `Err(e)` branch uses `forward_error_message(e.is_connect())`
- [x] `is_connect` is included in the structured warning log
- [x] `reply_transient_error` remains unchanged
- [x] No new dependencies added
- [x] `cargo test -p mika-gateway` passes
- [x] `cargo build -p mika-gateway` compiles
- [x] `cargo clippy -p mika-gateway` has no warnings

## Verification

```bash
cargo test -p mika-gateway
cargo build -p mika-gateway
cargo clippy -p mika-gateway
```

## Sources

- Related issue: #309
- Gateway error handling: `crates/mika-gateway/src/routes.rs:342-365`
- Institutional learning: `docs/solutions/runtime-errors/agent-max-steps-no-followup.md` — user-facing error messages should be specific and actionable
- Institutional learning: `docs/solutions/logic-errors/failed-callback-tasks-silently-dropped.md` — enumerate all failure modes for visible notifications

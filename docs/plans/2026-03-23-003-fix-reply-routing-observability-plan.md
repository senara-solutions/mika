# Fix: Reply Routing Observability (#231)

## Problem

Telegram reply routing to specific agents appears broken — replies fall back to the default agent. Code review confirmed the routing logic is correct across the entire stack, but silent error handling (`let _ =`) masks failures at critical points in the pipeline.

## Approach

Add structured logging at every failure point in the reply routing pipeline to make the root cause visible in production logs.

## Changes

### `crates/mika-gateway/src/routes.rs`
- Replace `let _ =` on outbound_messages INSERT with `warn!` on error
- Add fallback warning in `handle_text_message` and `handle_photo_message` when `reply_to_message_id` is present but no agent is found
- Add `debug!` log on successful agent resolution in `resolve_reply_agent`
- Replace `let _ =` on cleanup query with `debug!` on error

### `crates/mika-gateway/src/telegram.rs`
- Replace `unwrap_or(0)` on sendMessage response with explicit match + `warn!` when result is missing

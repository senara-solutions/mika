# Fix: Reply Routing — Parse Agent Name from Reply Text (#231)

## Problem

Telegram reply routing to specific agents was broken. The `ReplyToMessage` struct only captured `message_id`, discarding the `text` field from Telegram's reply context. The `[agent_name]` prefix (e.g., `[mika-test] hello`) was available in every outbound message but never parsed on inbound replies.

## Approach

Parse `[agent_name]` from `reply_to_message.text` as the primary routing mechanism. Keep the `outbound_messages` DB lookup as a fallback.

## Changes

- `telegram.rs`: Added `text: Option<String>` to `ReplyToMessage`, `reply_to_text` to `ParsedMessage` variants, `parse_agent_prefix()` function, 10 unit tests
- `routes.rs`: Threaded `reply_to_text` through handlers, updated `resolve_reply_agent()` to use text prefix first
- `gateway.yaml`: Regenerated OpenAPI spec for updated `ReplyToMessage` schema

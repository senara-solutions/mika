---
status: complete
priority: p2
issue_id: 282
tags: [code-review, security, validation]
dependencies: []
---

# Validate chat_id as integer in /config set

## Problem Statement

The `/config set chat_id <value>` command validates timezone values via `chrono_tz::Tz` parsing but performs no validation on `chat_id`. In the gateway, `chat_id` is typed as `i64` (Telegram chat IDs are integers). Setting a non-numeric value causes silent failures in outbound message delivery via `GatewayMessageSender`.

## Findings

- **Security Sentinel:** `chat_id` config value not validated (Low severity)
- **Architecture Strategist:** No `chat_id` integer validation — user sets invalid value, causing silent delivery failures
- **Pattern Recognition:** Domain-specific validation applied per-key for timezone but not for chat_id

## Proposed Solutions

### Solution A: Add i64 parse check (Recommended)
Add `value.parse::<i64>()` validation alongside the existing timezone check in `handle_config_set`.

**File:** `crates/mika-cli/src/tui/commands/handlers.rs:305`

```rust
if key == "chat_id" && value.parse::<i64>().is_err() {
    return format!("Invalid chat_id: {value}\nchat_id must be a numeric Telegram chat ID");
}
```

- Effort: Small
- Risk: None

## Acceptance Criteria

- [ ] `/config set chat_id abc` returns error message
- [ ] `/config set chat_id 12345` succeeds
- [ ] `/config set chat_id -100123456789` succeeds (group chat IDs are negative)

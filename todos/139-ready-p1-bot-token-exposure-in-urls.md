---
status: ready
priority: p1
issue_id: "139"
tags: [plan-review, security]
dependencies: []
---

# Bot token exposure in URLs and log output

## Problem Statement
The Telegram Bot API embeds the bot token directly in API URLs: `https://api.telegram.org/bot<TOKEN>/sendMessage`. If these URLs are logged (request errors, debug logging, HTTP client tracing), the bot token is exposed in plaintext in logs. The bot token grants full control over the Telegram bot — reading messages, sending messages as the bot, managing webhooks.

**Why it matters:** Bot token compromise means an attacker can impersonate the bot, read all incoming messages, and send messages to any paired user.

## Findings
- Source: Security Sentinel (C-4)
- Location: Plan Phase 3.5 (telegram.rs) — `send_telegram_message()` function
- Bot token embedded in URL path: `format!("https://api.telegram.org/bot{}/sendMessage", token)`
- reqwest and tracing may log full URLs on errors
- The existing Settings Debug impl redacts API keys, but URL-embedded tokens need separate handling

## Proposed Solutions

### Option 1: Redact bot token in all error/log paths (Recommended)
- Never log the full Telegram API URL
- Use a wrapper that redacts the token portion in Display/Debug impls
- Consider using `secrecy::SecretString` for the bot token config value
- Set reqwest logging to not include URLs, or filter at tracing subscriber level
- **Pros**: Prevents accidental exposure, follows existing redaction pattern
- **Cons**: Requires discipline in all error paths
- **Effort**: Small
- **Risk**: Low

### Option 2: Telegram Bot API via header auth
Some Telegram Bot API wrappers support token-in-header instead of URL. Check if the API supports this.
- **Pros**: Token never in URL
- **Cons**: May not be supported by Telegram API
- **Effort**: Small (if supported)
- **Risk**: Medium (API compatibility)

## Technical Details
- **Affected files**: Plan Phase 3.5 (telegram.rs), config.rs
- **Related Components**: Logging configuration, error handling

## Acceptance Criteria
- [ ] Bot token never appears in log output
- [ ] Error messages from Telegram API calls redact the token
- [ ] Bot token stored as SecretString or equivalent
- [ ] Manual verification: grep logs for bot token pattern after test run

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent plan review)
**Actions:** Security Sentinel flagged bot token embedded in API URLs leaking to logs

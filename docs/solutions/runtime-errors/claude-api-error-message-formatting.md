---
title: "Show user-friendly error messages for Claude API failures"
date: 2026-02-25
module: mika-common/claude, mika-agent/agent, mika-cli/commands, mika-cli/tui
severity: medium
tags:
  - error-handling
  - user-experience
  - api-errors
  - anyhow
  - cli
related_issues:
  - "PR #15"
  - "PR #14 (foundation: API key whitespace fix)"
  - "todos #266, #267, #268"
---

# Show User-Friendly Error Messages for Claude API Failures

## Symptom

When Claude API authentication failed (or any API error occurred), the TUI displayed an unreadable single-line error:

```
Error: Claude API call failed: Authentication failed. Check that MIKA_ANTHROPIC_API_KEY is set to a valid Anthropic API key.: Claude API HTTP error (401): invalid x-api-key
```

200+ characters on one line, wrapping mid-word in 80-column terminals. The actionable hint was buried between a generic prefix and raw API internals.

## Root Cause

Three compounding issues:

1. **`{e:#}` format**: `chat.rs` used anyhow's alternate display (`format!("{e:#}")`) which concatenates the full error chain with `: ` separators on a single line.

2. **Incomplete `.context()` coverage**: Only 401 errors had user-friendly context wrappers. Other error classes (429, 500+, transport, parse) returned raw `ClaudeApiError` display strings.

3. **Redundant context layers**: The agent loop added `.context("Claude API call failed")` on top of the API client's context, creating a 3-layer chain with no useful information in the outermost layer.

Additionally:
- `ChatRole::System` rendered as a single `Span`, unlike `Command` which splits on `\n`
- `mika ask` used `process::exit(1)` bypassing Drop-based cleanup

## Investigation

The error chain was traced through three layers:

| Layer | Source | Text |
|-------|--------|------|
| Outermost | `agent.rs` `.context()` | `"Claude API call failed"` |
| Middle | `claude.rs` `.context()` | `"Authentication failed. Check that..."` |
| Innermost | `ClaudeApiError::HttpError` | `"Claude API HTTP error (401): invalid x-api-key"` |

With `{e:#}`, all three concatenate into one line. With `{e}`, only the outermost displays.

The fix required moving user-friendly context to the innermost wrapping point (the API client) and removing redundant outer layers, so `{e}` shows the right message.

## Solution

### 1. Add `.context()` wrappers for all error classes (`claude.rs`)

```rust
return Err(match &e {
    ClaudeApiError::HttpError { status: 401, .. } => {
        anyhow::Error::from(e).context(
            "Authentication failed. Check that MIKA_ANTHROPIC_API_KEY is set to a valid Anthropic API key.",
        )
    }
    ClaudeApiError::HttpError { status: 429, .. } => {
        anyhow::Error::from(e).context(
            "Claude API is busy. Please wait a moment and try again.",
        )
    }
    ClaudeApiError::HttpError { status, .. } if *status >= 500 => {
        anyhow::Error::from(e).context(
            "Claude API is temporarily unavailable. Please try again shortly.",
        )
    }
    ClaudeApiError::Transport(_) => {
        anyhow::Error::from(e).context(
            "Could not connect to Claude API. Check your internet connection.",
        )
    }
    ClaudeApiError::ParseError(_) => {
        anyhow::Error::from(e).context(
            "Received an unexpected response from Claude API.",
        )
    }
    ClaudeApiError::HttpError { .. } => {
        anyhow::Error::from(e).context(
            "Claude API returned an unexpected error. Please try again.",
        )
    }
});
```

Retry-exhaustion path also matches error type for accurate messaging (500+ vs 429).

### 2. Change `{e:#}` to `{e}` in CLI (`chat.rs`)

```rust
// Before: concatenates full chain
content: format!("{e:#}"),

// After: shows only outermost context
content: format!("{e}"),
```

This single-character change is the highest-impact fix. anyhow's `Display` (`{e}`) returns only the outermost `.context()` message.

### 3. Remove redundant context from agent loop (`agent.rs`)

```rust
// Before: adds noise
let response = claude.send_message(&request).await.context("Claude API call failed")?;

// After: let API client's context speak for itself
let response = claude.send_message(&request).await?;
```

### 4. Multi-line System messages in TUI (`ui.rs`)

```rust
ChatRole::System => {
    lines.push(Line::default());
    for line in msg.content.lines() {
        lines.push(Line::from(vec![Span::styled(
            line.to_string(),
            Style::default().fg(Color::Red),
        )]));
    }
}
```

Matches the existing `ChatRole::Command` pattern.

### 5. Clean error handling in `mika ask` (`ask.rs` + `main.rs`)

Moved `process::exit(1)` from `ask.rs` to `main.rs` so `AppContext` drops cleanly before exit:

```rust
// main.rs
Some(Commands::Ask { message }) => {
    match commands::ask::run(&message, &agent_name).await {
        Ok(()) => Ok(()),
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}
```

### Result

| Error | Before | After |
|-------|--------|-------|
| 401 | `Claude API call failed: Authentication failed...: Claude API HTTP error (401): invalid x-api-key` | `Authentication failed. Check that MIKA_ANTHROPIC_API_KEY is set to a valid Anthropic API key.` |
| 429 | `Claude API call failed: Claude API HTTP error (429): rate_limit_exceeded` | `Claude API is busy. Please wait a moment and try again.` |
| 500 | `Claude API call failed: Claude API HTTP error (500): internal_error` | `Claude API is temporarily unavailable. Please try again shortly.` |
| Network | `Claude API call failed: Claude API request failed: connection refused` | `Could not connect to Claude API. Check your internet connection.` |

Raw API details remain in tracing logs for debugging.

## Prevention

### Conventions established

1. **Use `{e}` for users, `{e:#}` for logs** — one format per audience
2. **Wrap at boundaries, not call sites** — one `.context()` per error class, at the API client level
3. **Context text is actionable** — no raw API details, no technical jargon
4. **Return `Result<()>` from CLI commands** — never `process::exit()` in command functions; centralize in `main.rs`
5. **Split multi-line output** — use `.lines()` when rendering user messages in TUI

### When adding new API clients

- Classify errors (auth, rate-limit, server, transport, parse)
- Add `.context()` wrappers for each class at the API boundary
- Do NOT add `.context()` at call sites — the API client handles it

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-common/src/claude.rs` | `.context()` wrappers for all error classes + retry-exhaustion |
| `crates/mika-agent/src/agent.rs` | Removed `.context("Claude API call failed")` from 3 call sites |
| `crates/mika-cli/src/commands/chat.rs` | Changed `{e:#}` to `{e}` (3 occurrences) |
| `crates/mika-cli/src/commands/ask.rs` | Reverted to `?` propagation |
| `crates/mika-cli/src/main.rs` | Added error handling for Ask command |
| `crates/mika-cli/src/tui/ui.rs` | Multi-line System message rendering |

## Related

- [API key whitespace fix](../security-issues/api-key-whitespace-opaque-401-error.md) — PR #14, established the 401 context pattern this PR extends
- [Fresh install sentinel mismatch](../logic-errors/fresh-install-agent-not-found-sentinel-mismatch.md) — related "check vs transform" pattern
- `TelegramApiError::Unauthorized` in `mika-gateway/src/telegram.rs` — alternative pattern with hints baked into `#[error]` attribute

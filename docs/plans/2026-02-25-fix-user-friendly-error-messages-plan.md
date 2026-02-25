---
title: "fix: Improve user-facing error messages for Claude API failures"
type: fix
status: completed
date: 2026-02-25
---

# fix: Improve user-facing error messages for Claude API failures

## Overview

When Claude API authentication fails (401), the TUI displays an unreadable single-line error that concatenates three anyhow context layers with `: ` separators. The actionable hint is buried in the middle, raw API internals are exposed, and the message wraps awkwardly in the terminal. This fix introduces user-friendly error formatting across all CLI error paths.

## Problem Statement

**Current TUI output for a 401:**
```
Error: Claude API call failed: Authentication failed. Check that MIKA_ANTHROPIC_API_KEY is set to a valid Anthropic API key.: Claude API HTTP error (401): invalid x-api-key
```

Issues:
1. **Unreadable**: 200+ chars on one line, wraps mid-word in 80-col terminals
2. **Buried actionable info**: The useful hint is sandwiched between generic prefix and raw API detail
3. **Exposes internals**: `"invalid x-api-key"` is Anthropic's internal error text
4. **No multi-line support**: `ChatRole::System` renders as a single `Span`, unlike `Command` which splits on `\n`
5. **Inconsistent**: TUI uses `{e:#}` (single line), `mika ask` uses anyhow Debug (multi-line)

## Proposed Solution

Three focused changes:

1. **Add user-friendly `.context()` wrappers** for all error classes in `claude.rs` `send_message()` (not just 401)
2. **Extract only the user-friendly context** when formatting errors for display (stop showing the full chain)
3. **Support multi-line System messages** in the TUI renderer

## Technical Approach

### 1. Add user-friendly context for all error classes

**File:** `crates/mika-common/src/claude.rs` — `send_message()` (lines 196-208)

Currently only 401 gets a `.context()` wrapper. Add wrappers for other error classes after retry exhaustion:

```rust
Err(e) => {
    if attempt < MAX_RETRIES && is_retryable(&e) {
        warn!(attempt, error = %e, "transient Claude API error");
        last_error = Some(e);
        continue;
    }
    return Err(match &e {
        ClaudeApiError::HttpError { status: 401, .. } => {
            anyhow::Error::from(e).context(
                "Authentication failed. Check that MIKA_ANTHROPIC_API_KEY is set to a valid Anthropic API key."
            )
        }
        ClaudeApiError::HttpError { status: 429, .. } => {
            anyhow::Error::from(e).context(
                "Claude API is busy. Please wait a moment and try again."
            )
        }
        ClaudeApiError::HttpError { status, .. } if *status >= 500 => {
            anyhow::Error::from(e).context(
                "Claude API is temporarily unavailable. Please try again shortly."
            )
        }
        ClaudeApiError::Transport(_) => {
            anyhow::Error::from(e).context(
                "Could not connect to Claude API. Check your internet connection."
            )
        }
        ClaudeApiError::ParseError(_) => {
            anyhow::Error::from(e).context(
                "Received an unexpected response from Claude API."
            )
        }
        _ => e.into(),
    });
}
```

Also wrap the retry-exhaustion path at the end of `send_message()`:

```rust
Err(last_error
    .map(|e| anyhow::Error::from(e).context(
        "Claude API is busy. Please wait a moment and try again."
    ))
    .unwrap_or_else(|| anyhow::anyhow!("max retries exceeded")))
```

### 2. Create a user-friendly error formatter

**New function in:** `crates/mika-cli/src/tui/app.rs` (or a small `error_format` module in mika-cli)

Extract only the outermost `.context()` message from an anyhow error chain, which is always the user-friendly one we added:

```rust
/// Extract the user-friendly message from an anyhow error chain.
/// Returns the outermost context (first in the chain), which is the
/// user-facing message added via `.context()`. Falls back to `{e}`
/// (Display) if the chain has only one layer.
fn format_user_error(e: &anyhow::Error) -> String {
    // The outermost message is the `.context()` we added — that's the user-friendly one.
    // anyhow's Display (`{e}`) already returns just the outermost context.
    format!("{e}")
}
```

Key insight: anyhow's `Display` format (`{e}`) already returns **only** the outermost context message. The problem is that `chat.rs` uses `{e:#}` (alternate format) which concatenates the full chain. The fix is simply changing `{e:#}` to `{e}`.

### 3. Apply the formatter in both CLI paths

**File:** `crates/mika-cli/src/commands/chat.rs` (line 102)

```rust
// Before:
Err(e) => AgentResponse {
    content: format!("{e:#}"),
    is_error: true,
},

// After:
Err(e) => AgentResponse {
    content: format!("{e}"),
    is_error: true,
},
```

**File:** `crates/mika-cli/src/commands/ask.rs` (line 47)

Wrap the error for clean output instead of anyhow's Debug format:

```rust
// Before:
let response = agent::run_agent(&AgentParams { ... }).await?;

// After:
let response = match agent::run_agent(&AgentParams { ... }).await {
    Ok(content) => content,
    Err(e) => {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
};
```

### 4. Remove redundant "Claude API call failed" context

**File:** `crates/mika-agent/src/agent.rs` (line 189)

```rust
// Before:
let response = claude
    .send_message(&request)
    .await
    .context("Claude API call failed")?;

// After:
let response = claude.send_message(&request).await?;
```

This generic wrapper adds no value now that each error class has its own user-friendly context. Apply the same change at lines ~438 and ~570 (other `send_message` call sites in agent.rs).

### 5. Support multi-line System messages in TUI

**File:** `crates/mika-cli/src/tui/ui.rs` (lines 89-95)

```rust
// Before:
ChatRole::System => {
    lines.push(Line::default());
    lines.push(Line::from(vec![Span::styled(
        msg.content.clone(),
        Style::default().fg(Color::Red),
    )]));
}

// After:
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

This matches the existing `ChatRole::Command` pattern (lines 96-104).

## Acceptance Criteria

- [x] 401 error in TUI shows: `Error: Authentication failed. Check that MIKA_ANTHROPIC_API_KEY is set to a valid Anthropic API key.`
- [x] 429 error (after retries) shows a user-friendly "busy" message, not raw API details
- [x] 500/529 errors show "temporarily unavailable" message
- [x] Network errors show "check your internet connection" message
- [x] `mika ask` shows the same clean error to stderr (not anyhow Debug chain)
- [x] TUI System messages render multi-line content (split on `\n`)
- [x] Raw API error details (`"invalid x-api-key"`) are logged via tracing but not shown to users
- [x] All existing tests pass (`cargo test`)

## Files to Modify

| File | Change |
|------|--------|
| `crates/mika-common/src/claude.rs` | Add `.context()` wrappers for 429, 500+, transport, parse errors; wrap retry-exhaustion path |
| `crates/mika-agent/src/agent.rs` | Remove `.context("Claude API call failed")` from all `send_message` call sites |
| `crates/mika-cli/src/commands/chat.rs` | Change `{e:#}` to `{e}` on line 102 |
| `crates/mika-cli/src/commands/ask.rs` | Catch error explicitly, print with `{e}` to stderr, exit(1) |
| `crates/mika-cli/src/tui/ui.rs` | Split System messages on `\n` like Command messages |

## Out of Scope

- Server mode error differentiation (transient vs permanent) — separate concern
- `EmbeddingClient` parity — embedding errors are already swallowed gracefully
- Adding a `ChatRole::Error` variant — the existing `System` role with `"Error: "` prefix is sufficient
- Debug/verbose mode for full error chains — users can check logs

## References

- PR #14: API key whitespace fix (established the 401 context pattern)
- `docs/solutions/security-issues/api-key-whitespace-opaque-401-error.md`: Institutional learnings
- `crates/mika-gateway/src/telegram.rs`: `TelegramApiError::Unauthorized` — inline actionable hints pattern

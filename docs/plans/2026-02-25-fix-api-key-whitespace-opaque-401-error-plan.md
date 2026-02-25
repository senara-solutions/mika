---
title: "Fix: API Key Whitespace & Opaque 401 Error"
type: fix
status: completed
date: 2026-02-25
---

# Fix: API Key Whitespace & Opaque 401 Error

## Overview

When a user sets `MIKA_ANTHROPIC_API_KEY` with leading/trailing whitespace (common from copy-paste), Mika sends the whitespace to the Anthropic API, which rejects it with a 401 "invalid x-api-key". The error message shown to the user is opaque (`Claude API HTTP error (401)`) with no hint about the cause.

## Problem Statement

Two related bugs in `crates/mika-common/src/claude.rs`:

**Bug 1: API key not trimmed (root cause)**

```rust
// claude.rs:148-150
let api_key = api_key
    .filter(|k| !k.trim().is_empty())  // trim used for CHECK only
    .ok_or_else(|| ...)?;              // original untrimmed value stored
```

The `.filter()` uses `.trim()` to test emptiness but the original untrimmed `String` passes through. If the key is `" sk-ant-... "` (with spaces), it's stored with spaces and sent as-is in the `x-api-key` header.

**Bug 2: Opaque error message for 401**

```rust
// claude.rs:106-107
#[error("Claude API HTTP error ({status})")]
HttpError { status: u16 },
```

The error type only carries the HTTP status code. The API's response message ("invalid x-api-key") is logged at `warn!` level (line 237) but discarded from the error chain. The user sees `Claude API HTTP error (401)` with no actionable guidance.

**Reproduction flow:**
1. Set `MIKA_ANTHROPIC_API_KEY=" sk-ant-api03-abc... "` (with spaces)
2. Run `mika`
3. Type a message
4. Get: `Error: Claude API call failed: Claude API HTTP error (401)`

## Proposed Solution

### 1. Trim the API key in `ClaudeClient::new()`

```rust
// Before
let api_key = api_key
    .filter(|k| !k.trim().is_empty())
    .ok_or_else(|| anyhow::anyhow!("MIKA_ANTHROPIC_API_KEY is required but not set"))?;

// After
let api_key = api_key
    .map(|k| k.trim().to_string())
    .filter(|k| !k.is_empty())
    .ok_or_else(|| anyhow::anyhow!("MIKA_ANTHROPIC_API_KEY is required but not set"))?;
```

### 2. Include API error message in `ClaudeApiError::HttpError`

```rust
// Before
#[error("Claude API HTTP error ({status})")]
HttpError { status: u16 },

// After
#[error("Claude API HTTP error ({status}): {message}")]
HttpError { status: u16, message: String },
```

Update `send_once()` to pass the parsed message into the error variant. This surfaces the API's actual error message ("invalid x-api-key") to the user.

### 3. Add actionable context for 401 in `send_message()`

In the non-retryable error path, if status is 401, add context suggesting the user check their API key:

```rust
Err(ClaudeApiError::HttpError { status: 401, .. }) => {
    return Err(e).context(
        "Authentication failed. Check that MIKA_ANTHROPIC_API_KEY is set to a valid Anthropic API key."
    );
}
```

### Files to change

| File | Change |
|------|--------|
| `crates/mika-common/src/claude.rs` | Trim API key, add message to HttpError, add 401 context |

## Acceptance Criteria

- [x] API key with leading/trailing whitespace is trimmed before use
- [x] 401 error message includes the API's response ("invalid x-api-key")
- [x] 401 error includes actionable hint about checking the API key
- [x] Existing tests pass
- [x] New tests for whitespace trimming
- [x] `is_retryable()` updated for new HttpError shape

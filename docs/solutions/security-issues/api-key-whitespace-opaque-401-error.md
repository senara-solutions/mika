---
title: "API Key Whitespace Causes Opaque 401 Error"
date: 2026-02-25
category: security-issues
severity: high
component: mika-common/claude, mika-common/embedding
tags:
  - api-key
  - authentication
  - whitespace-handling
  - error-messaging
  - configuration
related_issues:
  - PR #14
---

# API Key Whitespace Causes Opaque 401 Error

## Problem

When a user sets `MIKA_ANTHROPIC_API_KEY` with leading or trailing whitespace (common from copy-paste), the application sends the untrimmed key to the Anthropic API, which rejects it with a 401 authentication error. The error message shown to the user is opaque and provides no actionable guidance:

```
Error: Claude API call failed: Claude API HTTP error (401)
```

The same bug existed in `EmbeddingClient` for the OpenAI API key.

## Root Cause

Two interconnected bugs in `crates/mika-common/src/claude.rs`:

**Bug 1: API key not trimmed**

```rust
// claude.rs:148-150
let api_key = api_key
    .filter(|k| !k.trim().is_empty())  // trim used for CHECK only
    .ok_or_else(|| ...)?;              // original untrimmed value stored
```

The `.filter()` uses `.trim()` to test emptiness but the original **untrimmed** `String` passes through. If the key is `" sk-ant-... "` (with spaces), it's stored with spaces and sent as-is in the `x-api-key` header.

**Bug 2: Opaque error type**

```rust
#[error("Claude API HTTP error ({status})")]
HttpError { status: u16 },
```

The error type only carries the HTTP status code. The API's response message ("invalid x-api-key") is logged at `warn!` level but discarded from the error chain.

**Failure chain:**
1. User sets `MIKA_ANTHROPIC_API_KEY=" sk-ant-api03-abc... "` (with spaces from copy-paste)
2. `ClaudeClient::new()` stores the untrimmed key (passes the `.filter()` emptiness check)
3. `send_once()` sends `x-api-key: " sk-ant-api03-abc... "` (with spaces)
4. Anthropic rejects with 401 "invalid x-api-key"
5. `ClaudeApiError::HttpError { status: 401 }` carries only the status code
6. User sees `"Claude API HTTP error (401)"` with no hint about the cause

## Solution

Three complementary fixes:

### 1. Trim the API key before storage

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

### 2. Include error message in HttpError variant

```rust
// Before
#[error("Claude API HTTP error ({status})")]
HttpError { status: u16 },

// After
#[error("Claude API HTTP error ({status}): {message}")]
HttpError { status: u16, message: String },
```

With truncated fallback for non-JSON responses:

```rust
let message = serde_json::from_str::<ApiErrorResponse>(&body)
    .map(|e| e.error.message)
    .unwrap_or_else(|_| {
        let truncated: String = body.chars().take(200).collect();
        format!("unexpected error response (HTTP {status_code}): {truncated}")
    });
```

### 3. Add actionable context for 401

```rust
if matches!(&e, ClaudeApiError::HttpError { status: 401, .. }) {
    return Err(anyhow::Error::from(e).context(
        "Authentication failed. Check that MIKA_ANTHROPIC_API_KEY is set to a valid Anthropic API key.",
    ));
}
```

### Files Changed

| File | Change |
|------|--------|
| `crates/mika-common/src/claude.rs` | Trim API key, add message to HttpError, add 401 context, truncate raw body fallback |
| `crates/mika-common/src/embedding.rs` | Apply same whitespace trim to OpenAI API key |

## Prevention

1. **Validate and transform together.** When using `.trim()` on user input, apply the transformation (`.map(|s| s.trim().to_string())`) before validation (`.filter(|s| !s.is_empty())`). Don't use `.trim()` inside a closure that only inspects the original string.

2. **Provide diagnostic-grade error context.** HTTP status codes alone do not enable users to self-diagnose configuration errors. Include the API's error message and an actionable hint.

3. **Audit for pattern replication.** When a bug is found in one credential-handling path, search for the same pattern in related code. The same bug existed in both `ClaudeClient` and `EmbeddingClient`.

4. **Sanitize external API errors.** Truncate raw API response bodies before including in error messages to prevent leaking proxy/CDN internals.

5. **Test with intentional whitespace in credentials.** Add tests that verify API key validation with leading/trailing spaces.

## Key Insight

"Check" and "transform" are different operations. When code inspects a derived value (like `k.trim()`) but stores the original, it creates a gap where the stored value doesn't match the validated property. Always transform first, then validate the transformed value. This is the string equivalent of the "sentinel mismatch" pattern seen in the fresh install bug — the existence check inspected one thing but the code operated on another.

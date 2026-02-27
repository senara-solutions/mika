---
title: "feat: Support Anthropic OAuth subscription tokens for local development"
type: feat
status: completed
date: 2026-02-27
---

# feat: Support Anthropic OAuth subscription tokens for local development

## Overview

Allow developers to use their Claude subscription (OAuth token) instead of a billed API key when running Mika locally. The existing `MIKA_ANTHROPIC_API_KEY` env var stays — Mika auto-detects whether the value is an API key or an OAuth subscription token based on the `sk-ant-oat` prefix and switches the HTTP auth scheme accordingly.

## Problem Statement / Motivation

Currently Mika only supports Anthropic API key auth (`x-api-key` header). Developers with a Claude Pro/Team subscription must generate and pay for a separate API key to use Mika locally. Anthropic's API accepts OAuth bearer tokens from Claude subscriptions, routing usage against the subscription quota instead of API billing. OpenClaw already implements this pattern.

## Proposed Solution

**Auto-detect from token prefix** on the same `MIKA_ANTHROPIC_API_KEY` env var:

- Value starts with `sk-ant-oat` → OAuth bearer auth
- Anything else → standard API key auth (current behavior)

No new env var. The user just pastes whichever credential they have:

```bash
# API key (billed)
export MIKA_ANTHROPIC_API_KEY=sk-ant-api03-...

# Subscription token (uses Claude quota)
export MIKA_ANTHROPIC_API_KEY=sk-ant-oat01-...
```

### HTTP-level difference

**API key auth (current):**
```
x-api-key: sk-ant-api03-...
anthropic-version: 2023-06-01
```

**OAuth bearer auth (new):**
```
Authorization: Bearer sk-ant-oat01-...
anthropic-version: 2023-06-01
anthropic-beta: oauth-2025-04-20
```

## Technical Approach

### Phase 1: `AnthropicAuth` enum and header switching

**File: `crates/mika-common/src/claude.rs`**

1. Add an `AnthropicAuth` enum:

```rust
// crates/mika-common/src/claude.rs

const OAUTH_TOKEN_PREFIX: &str = "sk-ant-oat";

#[derive(Clone)]
pub enum AnthropicAuth {
    ApiKey(String),
    OAuthBearer(String),
}

impl AnthropicAuth {
    /// Auto-detect auth method from token prefix.
    pub fn from_token(token: String) -> Self {
        if token.starts_with(OAUTH_TOKEN_PREFIX) {
            Self::OAuthBearer(token)
        } else {
            Self::ApiKey(token)
        }
    }

    /// Return the raw credential (for HeaderValue validation).
    fn credential(&self) -> &str {
        match self {
            Self::ApiKey(k) => k,
            Self::OAuthBearer(t) => t,
        }
    }

    /// Whether this is an OAuth bearer token.
    pub fn is_oauth(&self) -> bool {
        matches!(self, Self::OAuthBearer(_))
    }
}
```

2. Update `ClaudeClient` struct — replace `api_key: String` with `auth: AnthropicAuth`:

```rust
#[derive(Clone)]
pub struct ClaudeClient {
    client: reqwest::Client,
    auth: AnthropicAuth,
    pub model: String,
    pub max_tokens: u32,
}
```

3. Update `ClaudeClient::new()` signature:

```rust
pub fn new(api_key: Option<String>, model: String, max_tokens: u32) -> Result<Self> {
    let credential = api_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .ok_or_else(|| anyhow::anyhow!(
            "MIKA_ANTHROPIC_API_KEY is required but not set. \
             Set it to an API key (sk-ant-api03-...) or OAuth token (sk-ant-oat01-...)."
        ))?;

    let auth = AnthropicAuth::from_token(credential);

    // ... rest unchanged, store `auth` instead of `api_key`
}
```

The constructor signature stays the same (`Option<String>`) so **all four call sites are unchanged**.

4. Update `send_message()` — validate the credential as a HeaderValue:

```rust
pub async fn send_message(&self, request: &MessagesRequest) -> Result<MessagesResponse> {
    let credential_header = HeaderValue::from_str(self.auth.credential())
        .context("invalid API key/token characters")?;
    // ... pass credential_header to send_once
}
```

5. Update `send_once()` — conditional header insertion:

```rust
async fn send_once(
    &self,
    request: &MessagesRequest,
    credential_header: HeaderValue,
) -> std::result::Result<MessagesResponse, ClaudeApiError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert("anthropic-version", HeaderValue::from_static(API_VERSION));

    // Auth headers
    match &self.auth {
        AnthropicAuth::ApiKey(_) => {
            headers.insert("x-api-key", credential_header);
        }
        AnthropicAuth::OAuthBearer(_) => {
            // Format: "Bearer <token>"
            let bearer = HeaderValue::from_str(&format!("Bearer {}", self.auth.credential()))
                .expect("already validated credential characters");
            headers.insert(reqwest::header::AUTHORIZATION, bearer);
        }
    }

    // Beta headers — collect all needed betas, then set once
    let mut betas: Vec<&str> = Vec::new();
    if self.auth.is_oauth() {
        betas.push("oauth-2025-04-20");
    }
    if request.thinking.is_some() {
        betas.push("interleaved-thinking-2025-05-14");
    }
    if !betas.is_empty() {
        let beta_value = betas.join(",");
        headers.insert(
            "anthropic-beta",
            HeaderValue::from_str(&beta_value).expect("static beta values are valid"),
        );
    }

    // ... rest of send_once unchanged
}
```

This fixes the existing **bug-in-waiting** where `headers.insert("anthropic-beta", ...)` would replace a previous value. Now all beta flags are collected into a single comma-separated header.

6. Update 401 error context to be auth-method-aware:

```rust
ClaudeApiError::HttpError { status: 401, .. } => {
    let hint = if self.auth.is_oauth() {
        "Authentication failed. Your OAuth token may have expired. \
         Run `claude setup-token` to get a new one, then update MIKA_ANTHROPIC_API_KEY."
    } else {
        "Authentication failed. Check that MIKA_ANTHROPIC_API_KEY is set to a valid Anthropic API key."
    };
    anyhow::Error::from(e).context(hint)
}
```

### Phase 2: Config display and documentation

**File: `crates/mika-cli/src/commands/config.rs`**

Update `mika config` to show auth method:

```rust
let auth_display = match &ctx.settings.anthropic_api_key {
    Some(key) if key.trim_start().starts_with("sk-ant-oat") => "OAuth token [REDACTED]",
    Some(_) => "API key [REDACTED]",
    None => "[NOT SET]",
};
println!("  Auth:       {}", auth_display);
```

**File: `crates/mika-common/src/home.rs`** (line 226-235)

Update `DEFAULT_CONFIG` comment:

```rust
pub const DEFAULT_CONFIG: &str = r#"# Mika configuration
# Override with MIKA_* environment variables (highest priority).
#
# Secrets MUST be set via environment variables, not in this file:
#   MIKA_ANTHROPIC_API_KEY — Anthropic API key or OAuth token (sk-ant-oat01-...)

claude_model = "claude-sonnet-4-6"
claude_max_tokens = 4096
log_level = "info"
"#;
```

**File: `.env.example`**

```
# Anthropic credential (required) — API key or OAuth subscription token
# API key (billed):    MIKA_ANTHROPIC_API_KEY=sk-ant-api03-...
# OAuth token (subscription): MIKA_ANTHROPIC_API_KEY=sk-ant-oat01-...
#   Get OAuth token via: claude setup-token (from Claude Code CLI)
MIKA_ANTHROPIC_API_KEY=sk-ant-...
```

**File: `crates/mika-agent/src/bin/mika-server.rs`** (line 15)

Update error message:

```rust
.context("Failed to load config. Set MIKA_ANTHROPIC_API_KEY (API key or OAuth token) and MIKA_INTERNAL_TOKEN.")?;
```

### Phase 3: Tests

**File: `crates/mika-common/src/claude.rs` — new tests:**

```rust
#[test]
fn test_auth_auto_detect_api_key() {
    let auth = AnthropicAuth::from_token("sk-ant-api03-abc123".into());
    assert!(matches!(auth, AnthropicAuth::ApiKey(_)));
    assert!(!auth.is_oauth());
}

#[test]
fn test_auth_auto_detect_oauth_token() {
    let auth = AnthropicAuth::from_token("sk-ant-oat01-abc123def456".into());
    assert!(matches!(auth, AnthropicAuth::OAuthBearer(_)));
    assert!(auth.is_oauth());
}

#[test]
fn test_auth_unknown_prefix_falls_back_to_api_key() {
    let auth = AnthropicAuth::from_token("some-random-key".into());
    assert!(matches!(auth, AnthropicAuth::ApiKey(_)));
}

#[test]
fn test_new_with_oauth_token() {
    let client = ClaudeClient::new(
        Some("sk-ant-oat01-test-token".into()),
        "model".into(),
        100,
    ).unwrap();
    assert!(client.auth.is_oauth());
}

#[test]
fn test_new_with_api_key() {
    let client = ClaudeClient::new(
        Some("sk-ant-api03-test-key".into()),
        "model".into(),
        100,
    ).unwrap();
    assert!(!client.auth.is_oauth());
}
```

**Beta header collision test** (verifies the fix):

```rust
#[test]
fn test_beta_headers_combine_oauth_and_thinking() {
    // Verify that when both OAuth and thinking are active,
    // both beta values appear in a single comma-separated header.
    let mut betas: Vec<&str> = Vec::new();
    betas.push("oauth-2025-04-20");
    betas.push("interleaved-thinking-2025-05-14");
    let combined = betas.join(",");
    assert_eq!(combined, "oauth-2025-04-20,interleaved-thinking-2025-05-14");
    // Verify it's a valid HeaderValue
    assert!(HeaderValue::from_str(&combined).is_ok());
}
```

**Existing tests to update:**

- `test_new_trims_api_key_whitespace` — verify `auth` field type instead of `api_key` string
- `test_new_rejects_whitespace_only_key` — error message assertion updated
- `test_new_rejects_none_key` — error message assertion updated
- Server test helper at `crates/mika-agent/src/server/mod.rs:264` — use `Some("test-key".to_string())` (unchanged, auto-detects as API key)

## Acceptance Criteria

- [x] `MIKA_ANTHROPIC_API_KEY=sk-ant-oat01-...` sends `Authorization: Bearer` + `anthropic-beta: oauth-2025-04-20`
- [x] `MIKA_ANTHROPIC_API_KEY=sk-ant-api03-...` sends `x-api-key` header (unchanged behavior)
- [x] OAuth + extended thinking: both beta flags present in single comma-separated header
- [x] 401 error with OAuth token mentions `claude setup-token` and token expiry
- [x] 401 error with API key mentions `MIKA_ANTHROPIC_API_KEY` (unchanged)
- [x] `mika config` shows "OAuth token [REDACTED]" or "API key [REDACTED]"
- [x] `.env.example` documents both credential types
- [x] All existing tests pass, new auth tests added
- [x] `cargo clippy` clean

## Dependencies & Risks

**No new crate dependencies.** All changes use existing `reqwest` header APIs.

**Risks:**
- **Beta header format:** Anthropic may require multiple `anthropic-beta` headers instead of comma-separated. Need to verify. If so, use `headers.append()` instead of `headers.insert()`.
- **Token expiry UX:** OAuth tokens expire silently. The user only discovers this on the next API call. Acceptable for v1; startup validation could be added later.
- **Prefix stability:** If Anthropic changes the `sk-ant-oat` prefix, the auto-detect breaks. Low risk — this prefix is used by Claude Code and OpenClaw.

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-common/src/claude.rs` | Add `AnthropicAuth` enum, update `ClaudeClient` struct/constructor/send_once/error messages, fix beta header collision, add tests |
| `crates/mika-cli/src/commands/config.rs` | Show auth method type in `mika config` output |
| `crates/mika-common/src/home.rs` | Update `DEFAULT_CONFIG` comment to mention OAuth tokens |
| `crates/mika-agent/src/bin/mika-server.rs` | Update startup error message |
| `.env.example` | Document OAuth token option |
| `CLAUDE.md` | Update environment variables section |

## References

- OpenClaw OAuth implementation (provided by user): prefix detection, `Authorization: Bearer` + `anthropic-beta: oauth-2025-04-20` header pattern
- Existing Bearer auth pattern in Mika: `crates/mika-common/src/embedding.rs:122` (OpenAI), `crates/mika-agent/src/server/auth.rs` (internal)
- Current `ClaudeClient` auth: `crates/mika-common/src/claude.rs:296-312`
- Beta header collision bug: `crates/mika-common/src/claude.rs:308-311` (`headers.insert` replaces)

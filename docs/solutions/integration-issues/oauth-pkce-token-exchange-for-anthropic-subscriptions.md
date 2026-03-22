---
title: OAuth PKCE Token Exchange for Anthropic Subscription Tokens
category: integration-issues
date: 2026-03-22
module: mika-common, mika-cli
severity: high
tags: [oauth, pkce, authentication, anthropic, token-refresh, credential-management]
issue: 232
---

# OAuth PKCE Token Exchange for Anthropic Subscription Tokens

## Problem

Anthropic's `claude setup-token` CLI produces subscription tokens (`sk-ant-oat*`) that don't work directly with the messages API. Sending them as Bearer tokens returns `400 invalid_request_error` with an opaque `"Error"` message. These are not access tokens — they are subscription/refresh tokens that must go through an OAuth PKCE flow to obtain short-lived access tokens.

## Root Cause

The `sk-ant-oat*` token is a subscription credential, not an API access token. Anthropic's OAuth system requires a full Authorization Code + PKCE flow to exchange it for a short-lived access token. The access token is what the messages API accepts. This is the same flow that OpenClaw's `pi-ai` SDK implements automatically.

## Solution

### Architecture

Three components:

1. **`oauth.rs`** (`crates/mika-common/src/oauth.rs`) — Core PKCE module:
   - `generate_pkce_params()` — 32-byte random verifier + SHA-256 challenge + hex state
   - `exchange_code()` — POST to `console.anthropic.com/v1/oauth/token` with form-urlencoded body
   - `OAuthTokenManager` — Thread-safe (`tokio::sync::RwLock`) token lifecycle manager with in-memory caching and `~/.mika/oauth.json` persistence
   - Proactive refresh 60 seconds before expiry + fallback force-refresh on 401
   - Subscription token hash tracking for change detection

2. **`AnthropicAuth::OAuthManaged`** (`crates/mika-common/src/claude.rs`) — New enum variant holding `Arc<OAuthTokenManager>`. `send_message_inner()` calls `manager.get_valid_token()` for async token resolution. On 401, attempts force-refresh before surfacing the error.

3. **`mika setup --mode oauth`** (`crates/mika-cli/src/commands/setup.rs`) — Interactive PKCE flow: generate params → show authorize URL → user pastes code → exchange → persist.

### Key Design Decisions

- **Subscription token stays in `MIKA_LLM_API_KEY`**; derived access/refresh tokens cached in `~/.mika/oauth.json` (0600 permissions, atomic write pattern)
- **Standard API keys (`sk-ant-api*`) completely unaffected** — only the `OAuthManaged` path triggers for `sk-ant-oat*` prefix
- **No new external crates** — all dependencies (`sha2`, `base64`, `rand`, `url`, `hex`) were existing workspace deps
- **Thread-safe with double-check locking** — read lock fast path for cached valid tokens, write lock for refresh with re-check to prevent thundering herd
- **CSRF state parameter generated but not validated** — intentional for manual copy-paste flow (no redirect endpoint to intercept)

### OAuth Endpoints

| Parameter | Value |
|-----------|-------|
| Client ID | `9d1c250a-e61b-44d9-88ed-5944d1962f5e` |
| Authorize URL | `https://claude.ai/oauth/authorize` |
| Token Exchange URL | `https://console.anthropic.com/v1/oauth/token` |
| Redirect URI | `https://console.anthropic.com/oauth/code/callback` |
| Scopes | `org:create_api_key user:profile user:inference` |

### Token Resolution Chain

```
MIKA_LLM_API_KEY=sk-ant-oat01-... detected in ClaudeClient::new()
  → Creates OAuthTokenManager(subscription_token, ~/.mika/)
  → AnthropicAuth::OAuthManaged(Arc<OAuthTokenManager>)
  → send_message_inner() calls manager.get_valid_token()
    → Cache hit (valid) → return access_token
    → Cache miss/expired → refresh via token endpoint → persist → return
    → No tokens → error directing to `mika setup --mode oauth`
```

## Prevention / Best Practices

1. **When adding new auth flows**: Keep the existing auth path untouched. The `AnthropicAuth` enum extension (adding `OAuthManaged`) did not modify `ApiKey` behavior at all.

2. **Token storage pattern**: Use `~/.mika/oauth.json` (structured JSON) for complex credentials, not `.env` (which can't handle multi-field time-sensitive data). Follow the atomic write pattern from `dotenv::set_env_var()`.

3. **Secret redaction checklist**: Any new struct holding tokens needs a manual `Debug` impl that prints `[REDACTED]`. The `TokenResponse` struct (deserialized from the API) intentionally does not derive `Debug`.

4. **Validate `expires_in`**: Always range-check server-provided expiry values (reject <=0 or >30 days) to prevent permanently-expired or permanently-valid tokens from a buggy server.

5. **Multi-instance safety**: When multiple provider instances may refresh the same credential, use file-based coordination (the disk re-read fallback after failed refresh handles the case where another process already refreshed).

## Related

- Issue: [#232](https://github.com/senara-solutions/mika/issues/232)
- Investigation doc: `docs/solutions/integration-issues/2026-03-21-anthropic-oauth-pkce-required-for-subscription-tokens.md`
- OpenClaw PKCE reference: `../openclaw/src/plugin-sdk/oauth-utils.ts`
- Security pattern: `docs/solutions/security-issues/debug-log-secret-leakage-and-file-permissions.md`
- Atomic write pattern: `docs/solutions/architecture-patterns/simplified-config-4-source-model.md`

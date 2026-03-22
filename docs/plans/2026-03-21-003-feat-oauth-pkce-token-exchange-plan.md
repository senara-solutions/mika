---
title: "feat: Add Anthropic OAuth PKCE token exchange"
type: feat
status: completed
date: 2026-03-21
issue: 232
---

# Add Anthropic OAuth PKCE Token Exchange

## Overview

Implement OAuth PKCE flow so Mika can authenticate with `sk-ant-oat*` subscription tokens from `claude setup-token`. Currently, raw `sk-ant-oat` tokens are passed directly as Bearer tokens (which fails with 400). They must be exchanged for short-lived access tokens via PKCE, then auto-refreshed on expiry.

## Problem Statement

OAuth tokens (`sk-ant-oat*`) from Anthropic's `claude setup-token` don't work directly with the messages API. The API returns `400 invalid_request_error`. The root cause: the raw `sk-ant-oat` token is a subscription/refresh token — not an access token. It must go through a PKCE OAuth flow to obtain a short-lived access token that the API accepts.

The existing codebase already has scaffolding:
- `AnthropicAuth::OAuthBearer(String)` variant in `claude.rs` (line 24)
- Auto-detection via `is_oauth_token()` (prefix `sk-ant-oat`, line 16)
- Bearer auth header construction (line 357)
- `oauth-2025-04-20` beta header (line 454)
- 401 error hint referencing OAuth (line 393)

But the scaffolding passes the raw subscription token directly — it never exchanges it for an access token.

## Proposed Solution

### Architecture

Three new components:

1. **`OAuthTokenManager`** — Thread-safe token lifecycle manager (`Arc<RwLock<CachedTokens>>`). Loads/saves `~/.mika/oauth.json`. Provides `get_valid_token()` that transparently refreshes expired access tokens.

2. **`AnthropicAuth::OAuthManaged` variant** — New enum variant in `claude.rs` that holds `Arc<OAuthTokenManager>` instead of a static token string. `send_message_inner()` calls the manager on each request.

3. **`mika setup --mode oauth` CLI command** — Interactive PKCE flow: generate params → show authorize URL → user pastes code → exchange for tokens → persist.

### Token Resolution Chain

```
MIKA_LLM_API_KEY=sk-ant-oat01-...
        │
        ▼
ClaudeClient::new() detects sk-ant-oat prefix
        │
        ▼
Creates OAuthTokenManager(subscription_token, home_dir)
        │
        ▼
AnthropicAuth::OAuthManaged { manager: Arc<OAuthTokenManager> }
        │
        ▼
send_message_inner() → manager.get_valid_token().await?
        │
        ├── oauth.json exists + access_token not expired → use it
        ├── oauth.json exists + access_token expired → refresh_token() → update file → use new token
        └── oauth.json missing/corrupt/stale → Err("Run `mika setup --mode oauth`")
```

### OAuth Endpoints (from pi-ai SDK reference)

| Parameter | Value |
|-----------|-------|
| Client ID | `9d1c250a-e61b-44d9-88ed-5944d1962f5e` |
| Authorize URL | `https://claude.ai/oauth/authorize` |
| Token Exchange URL | `https://console.anthropic.com/v1/oauth/token` |
| Redirect URI | `https://console.anthropic.com/oauth/code/callback` |
| Scopes | `org:create_api_key user:profile user:inference` |
| Challenge Method | S256 |

### PKCE Parameters

- **Code verifier**: 32 random bytes → base64url-encoded (43 chars, per RFC 7636)
- **Code challenge**: SHA-256(verifier) → base64url-encoded
- **State**: 16 random bytes → hex-encoded (CSRF protection)

## Technical Considerations

### Thread Safety

`OAuthTokenManager` uses `tokio::sync::RwLock<CachedTokens>` internally:
- Multiple readers can call `get_valid_token()` concurrently when the token is valid
- A single writer acquires the lock for refresh, others wait and then use the refreshed token
- File writes use atomic temp+rename pattern (consistent with `dotenv::set_env_var()`)

### Proactive Refresh with Buffer

Use local clock comparison with a 60-second buffer (refresh before expiry). On 401 after a supposedly-valid token, attempt one forced refresh (handles clock skew). If that also 401s, surface the re-auth error.

### Refresh Token Rotation

Assume Anthropic rotates refresh tokens. The implementation:
1. Acquires write lock
2. Calls token endpoint with refresh_token
3. Persists new tokens to disk (access_token + refresh_token + expires_at)
4. Updates in-memory cache
5. Releases lock

Critical: persist before using the new access token. If the process crashes between refresh and persist, the old refresh token may be invalidated.

### Subscription Token Change Detection

Store a SHA-256 hash of the subscription token in `oauth.json`. On load, compare against the current `MIKA_LLM_API_KEY`. If mismatch → invalidate cached tokens, require re-authorization. This prevents stale tokens when the user gets a new subscription token.

### Multi-Instance Safety

Multiple `OAuthTokenManager` instances (from `delegate_task`, `TeamEngine`) sharing the same `~/.mika/oauth.json`:
- In-process: The manager is `Arc`-wrapped. `create_provider()` can accept an optional `Arc<OAuthTokenManager>` to share across providers created from the same `Settings`.
- Cross-process: File-based. Each read loads the latest tokens. Concurrent refresh may cause one instance to fail (rotated refresh token); it re-reads from disk and retries once.

### Dependencies

All needed crates are already in the workspace `Cargo.toml`:

| Crate | Used for | Currently in mika-common? |
|-------|----------|--------------------------|
| `sha2` | PKCE challenge + subscription token hash | No → add |
| `base64` | Base64url encoding for PKCE | Optional (telemetry) → make non-optional |
| `rand` | Code verifier + state generation | No → add |
| `url` | Building authorize URL | No → add |
| `chrono` | Token expiry timestamps | Yes |
| `reqwest` | HTTP token exchange | Yes |
| `serde`/`serde_json` | oauth.json serialization | Yes |
| `tokio` | RwLock, async runtime | Yes |

No new external crates needed — all are existing workspace dependencies.

## System-Wide Impact

### Interaction Graph

- `Settings::make_llm_provider()` → `create_provider()` → `AnthropicProvider::new()` → `ClaudeClient::new()` — the chain that creates the OAuth manager when `sk-ant-oat` is detected
- `ClaudeClient::send_message_inner()` → `OAuthTokenManager::get_valid_token()` → `refresh_token()` (if expired) — called on every Anthropic API request
- `delegate_task.rs` creates fresh providers via `settings.make_llm_provider()` — must share the token manager
- `TeamEngine` creates providers via `settings.make_llm_provider()` — same sharing requirement
- `AppState.llm` in server mode — single provider instance, naturally shared
- `check_health()` makes API calls — benefits from auto-refresh without changes

### Error Propagation

- `OAuthTokenManager::get_valid_token()` returns `Result<String>` — errors propagate as `anyhow::Error` through `ClaudeClient::send_message_inner()` → `ClaudeApiError` context chain
- Refresh HTTP failures (429, 500, network) — wrapped with context explaining what failed
- Missing `oauth.json` — clear error directing user to run `mika setup --mode oauth`
- Expired refresh token — clear error directing user to re-authorize

### State Lifecycle Risks

- **Partial refresh persistence**: Process crash between token refresh and file write. Mitigation: atomic file write (temp + fsync + rename).
- **Stale in-memory cache**: Multiple providers reading different versions. Mitigation: each `get_valid_token()` call checks expiry; file read on cache miss.
- **Cross-process race**: Two mika processes refresh simultaneously, one gets invalidated refresh token. Mitigation: on refresh failure, re-read file and retry once.

### API Surface Parity

- `ClaudeClient::new()` signature unchanged (`api_key: Option<String>, model, max_tokens`)
- `AnthropicProvider::new()` signature unchanged
- `Settings::make_llm_provider()` signature unchanged
- `is_oauth_token()` public function unchanged
- New public API: `oauth` module functions (see implementation plan)

## Acceptance Criteria

### Functional Requirements

- [x] `mika setup --mode oauth` completes the PKCE flow interactively and stores tokens to `~/.mika/oauth.json`
- [x] `mika ask hello` works with `MIKA_LLM_API_KEY=sk-ant-oat01-...` when `oauth.json` has valid tokens
- [x] Token auto-refreshes transparently when access token expires (no user intervention)
- [x] Standard API keys (`sk-ant-api*`) continue to work unchanged (zero regression)
- [x] When refresh token is expired/invalid, user gets a clear error message to re-authorize
- [x] When `MIKA_LLM_API_KEY` changes to a different subscription token, cached tokens are invalidated
- [x] `oauth.json` has 0600 file permissions on Unix
- [x] All token fields are redacted in `Debug` impls and log output
- [x] Headless/SSH environments: authorization URL printed to stdout for manual browser open

### Non-Functional Requirements

- [x] No new external crates added
- [x] `cargo test` passes
- [x] `cargo clippy` passes
- [x] Thread-safe token refresh (server mode with concurrent handlers)

## Implementation Phases

### Phase 1: OAuth Module (`crates/mika-common/src/oauth.rs`)

**New file.** Core OAuth PKCE implementation.

```rust
// crates/mika-common/src/oauth.rs

// -- Constants --
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";
const REDIRECT_URI: &str = "https://console.anthropic.com/oauth/code/callback";
const SCOPES: &str = "org:create_api_key user:profile user:inference";
const REFRESH_BUFFER_SECS: i64 = 60;
const OAUTH_FILE: &str = "oauth.json";

// -- Types --

/// OAuth PKCE flow parameters.
pub struct PkceParams {
    pub authorize_url: String,
    pub code_verifier: String,
    pub state: String,
}

/// Persisted OAuth token credentials.
#[derive(Serialize, Deserialize, Clone)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: String,  // ISO 8601
    pub subscription_token_hash: String,
}

// Manual Debug — redact all token fields
impl fmt::Debug for OAuthTokens { ... }

/// Thread-safe token lifecycle manager.
pub struct OAuthTokenManager {
    subscription_token: String,
    subscription_hash: String,
    home_dir: PathBuf,
    cache: tokio::sync::RwLock<Option<OAuthTokens>>,
    http_client: reqwest::Client,
}

// -- Public API --

/// Generate PKCE parameters and authorization URL.
pub fn generate_pkce_params() -> PkceParams { ... }

/// Exchange authorization code for tokens.
pub async fn exchange_code(
    code: &str,
    code_verifier: &str,
) -> Result<OAuthTokens> { ... }

/// Load tokens from ~/.mika/oauth.json.
pub fn load_oauth_tokens(home_dir: &Path) -> Result<Option<OAuthTokens>> { ... }

/// Save tokens to ~/.mika/oauth.json with 0600 permissions.
pub fn save_oauth_tokens(home_dir: &Path, tokens: &OAuthTokens) -> Result<()> { ... }

impl OAuthTokenManager {
    /// Create a new manager for the given subscription token.
    pub fn new(subscription_token: String, home_dir: PathBuf) -> Self { ... }

    /// Get a valid access token, refreshing if expired.
    /// Returns Err if no tokens cached and setup is needed.
    pub async fn get_valid_token(&self) -> Result<String> { ... }

    /// Force a token refresh (used after 401 response).
    pub async fn force_refresh(&self) -> Result<String> { ... }
}
```

**Key implementation details:**

- `generate_pkce_params()`: 32 random bytes → base64url (no padding) for verifier; SHA-256 → base64url for challenge; 16 random bytes → hex for state. Build URL with `url::Url::parse_with_params()`.
- `exchange_code()`: POST to TOKEN_URL with `application/x-www-form-urlencoded` body: `grant_type=authorization_code`, `code`, `redirect_uri`, `client_id`, `code_verifier`. Parse JSON response for `access_token`, `refresh_token`, `expires_in`. Compute `expires_at` from `chrono::Utc::now() + expires_in`.
- `save_oauth_tokens()`: Atomic write: `serde_json::to_string_pretty()` → temp file → `chmod 0600` → rename. Pattern from `dotenv::set_env_var()`.
- `get_valid_token()`: Read lock → check cache → if valid (not within 60s of expiry), return. If expired or missing → upgrade to write lock → double-check (another thread may have refreshed) → call `refresh()` → persist → update cache → return.
- `subscription_hash`: `hex::encode(sha2::Sha256::digest(token))`. Compared on load to detect changed subscription tokens.

### Phase 2: Claude Client Integration (`crates/mika-common/src/claude.rs`)

**Modify existing file.** Add OAuth-managed auth variant and token resolution.

Changes:

1. **New `AnthropicAuth` variant:**

```rust
enum AnthropicAuth {
    ApiKey(String),
    OAuthBearer(String),  // Keep for backward compat (raw static token)
    OAuthManaged(Arc<OAuthTokenManager>),  // New: managed token lifecycle
}
```

2. **Modify `ClaudeClient::new()`** — When `is_oauth_token()` returns true, create `OAuthTokenManager` and use `OAuthManaged` variant:

```rust
let auth = if is_oauth_token(&credential) {
    let home_dir = crate::home::resolve_home_dir();
    let manager = OAuthTokenManager::new(credential, home_dir);
    AnthropicAuth::OAuthManaged(Arc::new(manager))
} else {
    AnthropicAuth::from_token(credential)
};
```

3. **Modify `send_message_inner()`** — Token resolution becomes async for the managed variant:

```rust
let auth_header = match &self.auth {
    AnthropicAuth::ApiKey(k) => {
        HeaderValue::from_str(k).context("invalid API key characters")?
    }
    AnthropicAuth::OAuthBearer(t) => {
        HeaderValue::from_str(&format!("Bearer {t}"))
            .context("invalid OAuth token characters")?
    }
    AnthropicAuth::OAuthManaged(manager) => {
        let token = manager.get_valid_token().await
            .context("OAuth token refresh failed. Run `mika setup --mode oauth` to re-authorize.")?;
        HeaderValue::from_str(&format!("Bearer {token}"))
            .context("invalid OAuth access token characters")?
    }
};
```

4. **Modify 401 error handling** — After a 401 with `OAuthManaged`, attempt one forced refresh before surfacing the error:

```rust
ClaudeApiError::HttpError { status: 401, .. } if self.auth.is_oauth_managed() => {
    // Try force-refresh before failing
    if let AnthropicAuth::OAuthManaged(manager) = &self.auth {
        if let Ok(new_token) = manager.force_refresh().await {
            // Retry with the fresh token
            let new_header = HeaderValue::from_str(&format!("Bearer {new_token}"))?;
            match self.send_once(request, new_header).await {
                Ok(response) => return Ok(response),
                Err(_) => {} // Fall through to the error path
            }
        }
    }
    let hint = "Authentication failed after token refresh. \
                Run `mika setup --mode oauth` to re-authorize.";
    anyhow::Error::from(e).context(hint)
}
```

5. **Update `is_oauth()` helper** — Return true for both `OAuthBearer` and `OAuthManaged`.

6. **Update `Debug` impl** — Redact the managed variant.

### Phase 3: CLI Setup Command (`crates/mika-cli/src/commands/setup.rs`)

**Modify existing file.** Add `OAuth` setup mode.

1. **Add `SetupMode::OAuth` variant:**

```rust
#[derive(Debug, Clone, ValueEnum)]
pub enum SetupMode {
    Cli,
    Server,
    Compose,
    OAuth,  // New
}
```

2. **Add OAuth handler in `run()`:**

```rust
SetupMode::OAuth => {
    run_oauth_setup(&home_dir).await?;
}
```

3. **`run_oauth_setup()` implementation:**

```rust
async fn run_oauth_setup(home_dir: &Path) -> Result<()> {
    println!("\n🔐 Anthropic OAuth Setup (PKCE)\n");
    println!("This will authorize Mika to use your Claude Pro/Max subscription.\n");

    // 1. Generate PKCE parameters
    let params = mika_common::oauth::generate_pkce_params();

    // 2. Try to open browser
    println!("Opening your browser to authorize Mika...\n");
    println!("If the browser doesn't open, visit this URL manually:\n");
    println!("  {}\n", params.authorize_url);

    let _ = open_browser(&params.authorize_url);  // Best-effort

    // 3. Prompt for authorization code
    println!("After authorizing, you'll see a code on the page.");
    let code: String = dialoguer::Input::new()
        .with_prompt("Paste the authorization code here")
        .interact_text()?;
    let code = code.trim().to_string();

    // 4. Exchange code for tokens
    println!("\nExchanging code for tokens...");
    let tokens = mika_common::oauth::exchange_code(&code, &params.code_verifier).await
        .context("Failed to exchange authorization code. The code may have expired — try again.")?;

    // 5. Persist tokens
    mika_common::oauth::save_oauth_tokens(home_dir, &tokens)?;

    println!("✅ OAuth tokens saved to ~/.mika/oauth.json");
    println!("   Access token expires at: {}", tokens.expires_at);
    println!("\nMika will automatically refresh the token when it expires.");

    Ok(())
}
```

4. **Reuse existing `open_browser()` pattern** from `dashboard.rs` (cross-platform: `xdg-open`/`open`/`cmd start`).

### Phase 4: Module Wiring

1. **`crates/mika-common/src/lib.rs`** — Add `pub mod oauth;`

2. **`crates/mika-common/Cargo.toml`** — Add dependencies:
   ```toml
   sha2 = { workspace = true }
   base64 = { workspace = true }  # remove feature-gate
   rand = { workspace = true }
   url = { workspace = true }
   ```

3. **Update `.env.example`** — Add comment about OAuth tokens:
   ```
   # OAuth tokens (sk-ant-oat*) require initial setup: mika setup --mode oauth
   ```

4. **Update existing 401 error messages** in `claude.rs` to reference `mika setup --mode oauth` instead of `claude setup-token`.

## Edge Cases Addressed

| Edge Case | Handling |
|-----------|----------|
| No `oauth.json` + OAuth token | Error: "Run `mika setup --mode oauth` to authorize" |
| Corrupt `oauth.json` | Treat as missing — same error message |
| Access token expired, refresh valid | Transparent refresh, no user intervention |
| Refresh token expired/revoked | Error: "Run `mika setup --mode oauth` to re-authorize" |
| Subscription token changed | Hash mismatch → treat as missing, require re-setup |
| Clock skew | 60-second buffer + fallback force-refresh on 401 |
| Headless/SSH environment | URL printed to stdout, browser open is best-effort |
| Concurrent refresh (server mode) | `RwLock` serializes refreshes; other callers wait |
| Standard API key (`sk-ant-api*`) | Zero change — `AnthropicAuth::ApiKey` path untouched |
| Non-Anthropic provider with `sk-ant-oat` key | Only Anthropic provider uses `ClaudeClient`; OpenAI-compatible providers use their own HTTP client. If model prefix is `openai/` but key is `sk-ant-oat`, the key is passed as-is (user error, not our problem) |
| Rate limit on token endpoint | Standard reqwest timeout (30s). No retry on refresh — surfaces error immediately |
| Network down on startup | Token resolution is lazy (first API call). Startup succeeds; first request fails with network error |

## Files Changed

| File | Action | Description |
|------|--------|-------------|
| `crates/mika-common/src/oauth.rs` | **Create** | OAuth PKCE flow, token manager, persistence |
| `crates/mika-common/src/lib.rs` | Modify | Add `pub mod oauth;` |
| `crates/mika-common/src/claude.rs` | Modify | Add `OAuthManaged` variant, async token resolution, force-refresh on 401 |
| `crates/mika-common/Cargo.toml` | Modify | Add `sha2`, `base64`, `rand`, `url` deps |
| `crates/mika-cli/src/commands/setup.rs` | Modify | Add `SetupMode::OAuth`, `run_oauth_setup()` |
| `.env.example` | Modify | Document OAuth token setup |

## Sources & References

- **Investigation doc:** `../docs/solutions/integration-issues/2026-03-21-anthropic-oauth-pkce-required-for-subscription-tokens.md`
- **OpenClaw PKCE reference:** `../openclaw/src/plugin-sdk/oauth-utils.ts` (verifier/challenge generation)
- **Existing auth scaffolding:** `crates/mika-common/src/claude.rs:11-50` (AnthropicAuth, is_oauth_token)
- **Atomic file write pattern:** `crates/mika-common/src/dotenv.rs:25-87` (set_env_var)
- **Browser launch pattern:** `crates/mika-cli/src/commands/dashboard.rs:73-99`
- **Secret redaction pattern:** `crates/mika-common/src/claude.rs:27-34` (AnthropicAuth Debug impl)
- **Security learnings:** `docs/solutions/security-issues/debug-log-secret-leakage-and-file-permissions.md`
- Related issue: #232

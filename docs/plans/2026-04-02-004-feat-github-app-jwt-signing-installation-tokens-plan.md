---
title: "feat: github_app module — JWT signing, installation token generation and caching"
type: feat
status: active
date: 2026-04-02
issue: "#381"
parent: "senara-solutions/mika-platform#3"
---

# feat: github_app module — JWT signing, installation token generation and caching

## Overview

Add a `github_app` module to `mika-common` for GitHub App authentication: RS256 JWT signing, installation token exchange via the GitHub API, and async token caching with automatic refresh. This replaces or supplements the existing `MIKA_GITHUB_TOKEN` PAT for agent GitHub operations.

Phase 1 of the GitHub App for mika-dev-bot initiative (senara-solutions/mika-platform#3). Depends on Phase 0 (#380).

## Problem Statement / Motivation

The current GitHub integration uses a Personal Access Token (`MIKA_GITHUB_TOKEN`) for all agent operations (`run_gh`, PR diff context injection, work item enrichment). PATs are tied to a human user account, have broad scopes, and require manual rotation. GitHub App installation tokens are:

- **Scoped to the organization** — not tied to a personal account
- **Short-lived** (1 hour) — reduced blast radius if leaked
- **Rate-limited separately** (5,000 req/hour per installation vs. 5,000 per user)
- **Auditable** — operations attributed to the app, not a human

## Proposed Solution

A new `github_app.rs` module in `mika-common` following the `OAuthTokenManager` pattern (`oauth.rs`):

1. **`GitHubApp` struct** — holds `app_id`, pre-parsed `EncodingKey` (from base64-encoded PEM), `installation_id`, and a `tokio::sync::RwLock<Option<CachedToken>>` cache
2. **`from_settings()`** constructor — returns `None` if any of the 3 config fields are missing; eagerly decodes base64 and parses PEM at construction time (fail-fast)
3. **`installation_token()`** method — double-checked locking: read lock fast path → write lock slow path with JWT generation + HTTP exchange → cache
4. **Token resolution helper** on `Settings` — `resolve_github_token(&self, github_app: Option<&GitHubApp>) -> Option<String>` eagerly resolves the best available token (installation token preferred, PAT fallback)
5. **Threading** — `Arc<GitHubApp>` on `AppState`/`AgentState`, borrowed reference through `AgentParams` → resolved token string into `ToolContext.github_token`

## Technical Considerations

### Architecture: Eager Token Resolution

The token is resolved **once at the start of each agent turn** (before `ToolContext` construction), not lazily inside each tool. This means:

- No structural changes to `ToolContext` (stays `github_token: Option<&'a str>`)
- No changes to tool implementations (`run_gh`, `check_work_item`, `fetch_pr_diff`)
- The resolved token string is stored in a local variable in the agent loop, borrowed into `ToolContext`
- Trade-off: one extra token resolution per turn even if no GitHub tool is called (negligible — cache hit is a `RwLock::read()`)

### JWT Claims

Per [GitHub docs](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-json-web-token-jwt-for-a-github-app):

- `iat`: current time **minus 60 seconds** (clock skew protection per GitHub recommendation)
- `exp`: `iat + 600` (10 minutes, GitHub maximum)
- `iss`: `app_id` as string
- Algorithm: RS256
- Key format: PKCS#1 RSA private key PEM (GitHub's default export format)

### Caching Strategy

Following `OAuthTokenManager` pattern in `oauth.rs`:

```
┌─────────────┐   read lock    ┌──────────────────┐
│ Tool call    │ ──────────────>│ RwLock<Option<    │
│ needs token  │                │   CachedToken>>   │
└─────────────┘                └──────────────────┘
                                      │
                          ┌───────────┴───────────┐
                     cache valid?            cache expired/empty?
                     (>5min left)            (<5min to expiry)
                          │                       │
                     return token            write lock
                                                  │
                                           double-check
                                           (still expired?)
                                                  │
                                          generate JWT
                                          POST /installations/{id}/access_tokens
                                          cache new token
                                          return token
```

- **Expiry buffer:** 5 minutes before the 1-hour GitHub expiry (conservative)
- **No disk persistence:** Installation tokens are short-lived; regeneration is cheap
- **Thundering herd prevention:** Double-check after acquiring write lock

### Fallback Behavior

| GitHub App configured | Token exchange succeeds | PAT configured | Result |
|---|---|---|---|
| Yes | Yes | Any | Use installation token |
| Yes | No (error) | Yes | `warn!`, fall back to PAT |
| Yes | No (error) | No | Return error |
| No | N/A | Yes | Use PAT (current behavior) |
| No | N/A | No | No auth (current behavior) |

Key: when App is configured but broken, **warn and fall back to PAT** if available. Never silently mask a configuration problem — the `warn!` log ensures visibility.

### Security

- Private key stored as `SecretString` in `Settings` with `[REDACTED]` in `Debug` impl
- `MIKA_GITHUB_APP_*` env vars auto-scrubbed by existing `scrub_mika_env_vars()` (all `MIKA_*` prefix)
- `EncodingKey` held in `GitHubApp` struct for process lifetime (acceptable — same as `OAuthTokenManager` holding refresh tokens)
- Installation tokens never persisted to disk (memory-only cache)
- Custom `Debug` impl on `GitHubApp` redacts private key and cached token

### Performance

- JWT generation: pure crypto, ~1ms
- Token exchange: single HTTP POST to GitHub API, ~200ms
- Normal operation: ~1 token exchange per 55 minutes (1-hour expiry minus 5-minute buffer)
- Agent turn overhead: one `RwLock::read()` per turn (nanoseconds on cache hit)

## System-Wide Impact

### Interaction Graph

`Settings::load_for_agent()` → `GitHubApp::from_settings()` → `Arc<GitHubApp>` stored in `AgentState`. Each agent turn: `github_app.installation_token().await` → resolved string stored locally → borrowed into `ToolContext.github_token`. From there: `run_gh` → `GH_TOKEN` env, `fetch_pr_diff` → `Authorization` header, `check_work_item` → `github_get()` calls.

### Error Propagation

`installation_token()` returns `Result<String>`. Errors: base64 decode (caught at construction), JWT signing failure (unlikely with valid key), HTTP errors (timeout/4xx/5xx). All errors logged at `warn!` level. Fallback to PAT happens at the resolution helper level, not inside `GitHubApp`.

### State Lifecycle Risks

Token cache is memory-only — no orphaned state on crash. `RwLock` is not poisoned by `tokio::sync::RwLock` (unlike `std::sync::RwLock`). No DB state changes.

### API Surface Parity

Three consumers of `github_token` today — all continue to work via the same `ToolContext.github_token` field. No API surface change for tools.

### Integration Test Scenarios

1. **GitHub App configured + PAT configured:** Agent calls `run_gh` → installation token used, PAT ignored
2. **GitHub App configured + token exchange fails + PAT configured:** Agent calls `run_gh` → warn logged, PAT used
3. **Partial config (2 of 3 vars):** `from_settings()` returns `None` → PAT used, doctor warns
4. **Invalid base64 PEM:** `from_settings()` returns `None` with `warn!` → PAT used, doctor warns
5. **Token expires mid-session:** Next tool call triggers refresh → new token cached

## Acceptance Criteria

- [ ] `jsonwebtoken` crate added to `mika-common/Cargo.toml`
- [ ] `github_app.rs` module in `mika-common/src/` with `GitHubApp` struct, `CachedToken`, `from_settings()`, `installation_token()`, `generate_jwt()`
- [ ] Three new `Settings` fields: `github_app_id: Option<u64>`, `github_app_private_key: Option<SecretString>`, `github_app_installation_id: Option<u64>`
- [ ] `ConfigKeyInfo` entries for all 3 new fields (`secret: true` for private key, `secret: false` for IDs)
- [ ] Manual `Debug` impl on `Settings` redacts `github_app_private_key`
- [ ] Custom `Debug` impl on `GitHubApp` redacts private key and cached token
- [ ] `get_effective_value()` match arms for all 3 new keys
- [ ] `GitHubApp` threaded through `AppState`/`AgentState` as `Option<Arc<GitHubApp>>`
- [ ] Token resolution at agent turn start (eager) — resolved string borrowed into `ToolContext.github_token`
- [ ] `run_gh` uses installation token when available (no code change needed — flows via `ToolContext.github_token`)
- [ ] Context injection (`fetch_pr_diff`) uses installation token (no code change needed — flows via resolved token)
- [ ] `mika doctor` checks: validates all 3 vars, decodes base64 and parses PEM
- [ ] `.env.example` updated with new section
- [ ] `docs/configuration.md` updated
- [ ] `CLAUDE.md` environment variables section updated
- [ ] Unit tests: JWT claim structure, base64 decode errors, PEM parse errors, cache expiry logic, partial config detection, `from_settings()` with complete/incomplete config

## Implementation Plan

### Phase 1: Core Module (`mika-common`)

#### 1a. Add dependency

**`crates/mika-common/Cargo.toml`:**
```toml
jsonwebtoken = "9"
```

#### 1b. Create `github_app.rs`

**`crates/mika-common/src/github_app.rs`:**

```rust
use anyhow::{Context, Result};
use base64::Engine;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Cached installation token with expiry.
struct CachedToken {
    token: String,
    expires_at: SystemTime,
}

/// GitHub App authentication manager.
///
/// Generates JWT tokens for GitHub App auth and exchanges them for
/// short-lived installation access tokens. Caches tokens with
/// automatic refresh using double-checked locking.
pub struct GitHubApp {
    app_id: u64,
    signing_key: EncodingKey,
    installation_id: u64,
    cache: RwLock<Option<CachedToken>>,
    http_client: reqwest::Client,
}

#[derive(Serialize)]
struct JwtClaims {
    iat: u64,
    exp: u64,
    iss: String,
}

/// Buffer before expiry to trigger proactive refresh.
const EXPIRY_BUFFER: Duration = Duration::from_secs(5 * 60);

/// Clock skew backdating for iat claim (GitHub recommendation).
const IAT_BACKDATE: Duration = Duration::from_secs(60);

/// JWT lifetime (GitHub maximum: 10 minutes).
const JWT_LIFETIME: Duration = Duration::from_secs(600);

impl GitHubApp {
    /// Create from Settings. Returns None if config is incomplete.
    /// Eagerly decodes base64 PEM and parses the RSA key.
    pub fn from_settings(settings: &crate::config::Settings) -> Option<Arc<Self>> {
        let app_id = settings.github_app_id?;
        let private_key_b64 = settings.github_app_private_key.as_ref()?;
        let installation_id = settings.github_app_installation_id?;

        let pem_bytes = match base64::engine::general_purpose::STANDARD
            .decode(private_key_b64.expose_secret().as_bytes())
        {
            Ok(bytes) => bytes,
            Err(e) => {
                warn!(
                    "MIKA_GITHUB_APP_PRIVATE_KEY: base64 decode failed: {e}. \
                     Encode with: base64 -w0 < your-app.pem"
                );
                return None;
            }
        };

        let signing_key = match EncodingKey::from_rsa_pem(&pem_bytes) {
            Ok(key) => key,
            Err(e) => {
                warn!("MIKA_GITHUB_APP_PRIVATE_KEY: RSA PEM parse failed: {e}");
                return None;
            }
        };

        info!(app_id, installation_id, "GitHub App configured");
        Some(Arc::new(Self {
            app_id,
            signing_key,
            installation_id,
            cache: RwLock::new(None),
            http_client: reqwest::Client::new(),
        }))
    }

    /// Get a valid installation token, refreshing if needed.
    pub async fn installation_token(&self) -> Result<String> {
        // Fast path: read lock
        {
            let cache = self.cache.read().await;
            if let Some(ref token) = *cache {
                if token.expires_at > SystemTime::now() + EXPIRY_BUFFER {
                    return Ok(token.token.clone());
                }
            }
        }

        // Slow path: write lock + double-check
        let mut cache = self.cache.write().await;
        if let Some(ref token) = *cache {
            if token.expires_at > SystemTime::now() + EXPIRY_BUFFER {
                return Ok(token.token.clone());
            }
        }

        let jwt = self.generate_jwt()?;
        let new_token = self.exchange_jwt_for_token(&jwt).await?;
        let result = new_token.token.clone();
        *cache = Some(new_token);
        Ok(result)
    }

    /// Generate a JWT for GitHub App authentication.
    fn generate_jwt(&self) -> Result<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock before UNIX epoch")?;

        let iat = now.as_secs() - IAT_BACKDATE.as_secs();
        let exp = iat + JWT_LIFETIME.as_secs();

        let claims = JwtClaims {
            iat,
            exp,
            iss: self.app_id.to_string(),
        };

        let header = Header::new(Algorithm::RS256);
        encode(&header, &claims, &self.signing_key)
            .context("JWT signing failed")
    }

    /// Exchange JWT for an installation access token.
    async fn exchange_jwt_for_token(&self, jwt: &str) -> Result<CachedToken> {
        let url = format!(
            "https://api.github.com/app/installations/{}/access_tokens",
            self.installation_id
        );

        let resp = self.http_client
            .post(&url)
            .header("Authorization", format!("Bearer {jwt}"))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "mika")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .context("GitHub API request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "GitHub installation token exchange failed (HTTP {status}): {body}"
            );
        }

        #[derive(serde::Deserialize)]
        struct TokenResponse {
            token: String,
            expires_at: String,
        }

        let body: TokenResponse = resp.json().await
            .context("failed to parse token response")?;

        let expires_at = chrono::DateTime::parse_from_rfc3339(&body.expires_at)
            .context("failed to parse expires_at")?;

        let expires_at = UNIX_EPOCH + Duration::from_secs(expires_at.timestamp() as u64);

        info!("GitHub App installation token refreshed");

        Ok(CachedToken {
            token: body.token,
            expires_at,
        })
    }
}

impl std::fmt::Debug for GitHubApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubApp")
            .field("app_id", &self.app_id)
            .field("installation_id", &self.installation_id)
            .field("signing_key", &"[REDACTED]")
            .field("cache", &"[REDACTED]")
            .finish()
    }
}
```

**`crates/mika-common/src/lib.rs`:**
```rust
pub mod github_app;
```

### Phase 2: Config Integration (`mika-common/src/config.rs`)

#### 2a. Settings struct — add 3 new fields

```rust
// In Settings struct, after github_token field:
pub github_app_id: Option<u64>,
pub github_app_private_key: Option<SecretString>,
pub github_app_installation_id: Option<u64>,
```

#### 2b. ConfigKeyInfo entries

```rust
ConfigKeyInfo {
    key: "github_app_id",
    backend: ConfigBackend::Env,
    env_var: Some("MIKA_GITHUB_APP_ID"),
    secret: false,
    description: "GitHub App ID for mika-dev-bot",
},
ConfigKeyInfo {
    key: "github_app_private_key",
    backend: ConfigBackend::Env,
    env_var: Some("MIKA_GITHUB_APP_PRIVATE_KEY"),
    secret: true,
    description: "GitHub App private key (base64-encoded PEM)",
},
ConfigKeyInfo {
    key: "github_app_installation_id",
    backend: ConfigBackend::Env,
    env_var: Some("MIKA_GITHUB_APP_INSTALLATION_ID"),
    secret: false,
    description: "GitHub App installation ID for the org",
},
```

#### 2c. `get_effective_value()` match arms

```rust
"github_app_id" => self.github_app_id.map(|v| v.to_string()),
"github_app_private_key" => self.github_app_private_key.as_ref().map(|_| "[SET]".to_string()),
"github_app_installation_id" => self.github_app_installation_id.map(|v| v.to_string()),
```

#### 2d. `Debug` impl redaction

```rust
.field("github_app_private_key", &self.github_app_private_key.as_ref().map(|_| "[REDACTED]"))
```

#### 2e. Token resolution helper

```rust
// On Settings impl:
/// Resolve the best available GitHub token.
/// Prefers GitHub App installation token, falls back to MIKA_GITHUB_TOKEN PAT.
pub async fn resolve_github_token(
    &self,
    github_app: Option<&GitHubApp>,
) -> Option<String> {
    if let Some(app) = github_app {
        match app.installation_token().await {
            Ok(token) => return Some(token),
            Err(e) => {
                warn!("GitHub App token exchange failed: {e}. Falling back to PAT.");
            }
        }
    }
    self.github_token.clone()
}
```

### Phase 3: Threading Through the Stack

#### 3a. `AppState` / `AgentState` (`crates/mika-agent/src/server/state.rs`)

Add `github_app: Option<Arc<GitHubApp>>` to `AgentState`. Initialize from `GitHubApp::from_settings(&settings)` during server startup.

#### 3b. `AgentParams` (`crates/mika-agent/src/agent.rs`)

Add `github_app: Option<&'a GitHubApp>` to `AgentParams`. The token resolution happens in the agent loop before constructing `ToolContext`:

```rust
// In run_agent(), before each turn:
let resolved_github_token = if let Some(settings) = params.settings {
    settings.resolve_github_token(params.github_app).await
} else {
    params.github_token.map(String::from)
};
// Then borrow into ToolContext:
let ctx = ToolContext {
    github_token: resolved_github_token.as_deref(),
    // ... other fields unchanged
};
```

#### 3c. CLI path (`mika-cli`)

CLI constructs `GitHubApp::from_settings()` and threads it through `AgentParams`.

### Phase 4: Doctor Check (`crates/mika-cli/src/commands/doctor.rs`)

Add a new check group:

```rust
// GitHub App (optional)
check_optional_key("GitHub App ID", "MIKA_GITHUB_APP_ID", &global_home, "GitHub App auth");
check_optional_key("GitHub App installation ID", "MIKA_GITHUB_APP_INSTALLATION_ID", &global_home, "GitHub App auth");
// Special: decode base64 and parse PEM
check_github_app_key("GitHub App private key", "MIKA_GITHUB_APP_PRIVATE_KEY", &global_home);
```

The `check_github_app_key` function:
- If unset: `Warn` with "GitHub App auth not configured"
- If set: decode base64 → parse PEM → `Ok` if valid, `Fail` if invalid with actionable error

Partial config detection: if 1-2 of 3 vars are set, `Warn` with "Incomplete GitHub App config — need all 3 vars".

### Phase 5: Documentation

#### 5a. `.env.example`

```bash
# -- GitHub App (optional, preferred over PAT) --
# Register at: https://github.com/settings/apps
# MIKA_GITHUB_APP_ID=123456
# MIKA_GITHUB_APP_PRIVATE_KEY=<base64 -w0 < your-app.pem>
# MIKA_GITHUB_APP_INSTALLATION_ID=78901234
```

#### 5b. `docs/configuration.md`

Add "GitHub App Authentication" section explaining the 3 env vars, encoding instructions, and fallback behavior.

#### 5c. `CLAUDE.md`

Add the 3 env vars to the Environment Variables section under a new "Optional (GitHub App — preferred over PAT)" subsection.

### Phase 6: Tests

#### 6a. Unit tests in `github_app.rs`

```rust
#[cfg(test)]
mod tests {
    // test_generate_jwt_claims — verify iat backdating, exp, iss
    // test_from_settings_complete — all 3 fields → Some
    // test_from_settings_partial — 1-2 fields → None
    // test_from_settings_invalid_base64 — bad b64 → None + warn
    // test_from_settings_invalid_pem — valid b64, bad PEM → None + warn
    // test_cache_expiry_logic — token within buffer → refresh
    // test_cache_valid — token not expired → return cached
}
```

Note: Tests for `generate_jwt()` require a test RSA key. Generate one in the test setup or use a const test key.

## Dependencies & Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| `jsonwebtoken` crate API changes | Low | Pin to major version `9` |
| Clock skew causes JWT rejection | Medium | 60-second `iat` backdating per GitHub recommendation |
| GitHub App permissions insufficient | Low | Phase 0 (#380) configures permissions |
| Installation token not accepted by `gh` CLI | Low | GitHub docs confirm support; test during implementation |
| Base64 encoding user errors | High | Actionable error message with exact command to run |

## Sources & References

- GitHub issue: #381
- Parent initiative: senara-solutions/mika-platform#3
- Phase 0 (prerequisite): #380
- GitHub App JWT docs: https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-json-web-token-jwt-for-a-github-app
- GitHub installation token docs: https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/authenticating-as-a-github-app-installation
- Existing pattern: `crates/mika-common/src/oauth.rs` (OAuthTokenManager)
- 9-layer checklist: `docs/solutions/architecture-patterns/config-key-rename-across-layers.md`
- GitHub token architecture: `docs/solutions/architecture-patterns/dedicated-github-token-agent-operations.md`
- GH_TOKEN scrubbing: `docs/solutions/security-issues/gh-token-identity-collision-dotenv-leak.md`

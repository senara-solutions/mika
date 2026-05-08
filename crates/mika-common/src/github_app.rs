//! GitHub App authentication: JWT signing, installation token exchange, and caching.
//!
//! Follows the [`OAuthTokenManager`](crate::oauth::OAuthTokenManager) pattern —
//! `tokio::sync::RwLock`-based cache with double-checked locking for thundering-herd
//! prevention.

use anyhow::{Context, Result};
use base64::Engine;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Buffer before expiry to trigger proactive refresh (5 minutes).
const EXPIRY_BUFFER: Duration = Duration::from_secs(5 * 60);

/// Clock skew backdating for `iat` claim (GitHub recommendation: 60 seconds).
const IAT_BACKDATE: Duration = Duration::from_secs(60);

/// JWT lifetime — 9 minutes (60s under GitHub's 10-minute hard ceiling).
/// The headroom tolerates positive host-clock skew up to ~120s before
/// GitHub's `exp ≤ iat + 600s` validator rejects the token. A single-use
/// JWT has no caching value, so shortening the lifetime has no
/// operational cost. See mika#1042.
const JWT_LIFETIME: Duration = Duration::from_secs(540);

/// Cached installation token with expiry (in-memory).
struct CachedToken {
    token: String,
    expires_at: SystemTime,
}

/// File-based token cache format (JSON, persisted at `~/.mika/github_app_token.json`).
#[derive(Serialize, Deserialize)]
struct FileCachedToken {
    token: String,
    expires_at: String, // RFC 3339
}

/// GitHub App authentication manager.
///
/// Generates RS256 JWT tokens for GitHub App authentication and exchanges them
/// for short-lived installation access tokens. Tokens are cached in memory with
/// automatic refresh using double-checked locking.
///
/// # Usage
///
/// ```ignore
/// let app = GitHubApp::from_settings(&settings)?;
/// let token = app.installation_token().await?;
/// ```
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

impl GitHubApp {
    /// Create from [`Settings`](crate::config::Settings). Returns `None` if config is incomplete.
    ///
    /// Eagerly decodes the base64-encoded PEM and parses the RSA key at construction
    /// time (fail-fast). If any field is missing or the key is invalid, a `warn!` is
    /// logged and `None` is returned so the system falls back to PAT.
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

    /// Construct directly for testing (bypasses Settings).
    #[cfg(any(test, feature = "test-utils"))]
    pub fn new(app_id: u64, signing_key: EncodingKey, installation_id: u64) -> Arc<Self> {
        Arc::new(Self {
            app_id,
            signing_key,
            installation_id,
            cache: RwLock::new(None),
            http_client: reqwest::Client::new(),
        })
    }

    /// Get a valid installation token, refreshing if needed.
    ///
    /// Uses double-checked locking:
    /// 1. Fast path: read lock — return cached token if valid
    /// 2. Slow path: write lock — re-check, then generate JWT + exchange for token
    pub async fn installation_token(&self) -> Result<String> {
        // Fast path: read lock
        {
            let cache = self.cache.read().await;
            if let Some(ref cached) = *cache
                && Self::is_valid(cached)
            {
                return Ok(cached.token.clone());
            }
        }

        // Slow path: write lock + double-check
        let mut cache = self.cache.write().await;
        if let Some(ref cached) = *cache
            && Self::is_valid(cached)
        {
            return Ok(cached.token.clone());
        }

        let jwt = self.generate_jwt()?;
        let new_token = self.exchange_jwt_for_token(&jwt).await?;
        let result = new_token.token.clone();
        *cache = Some(new_token);
        Ok(result)
    }

    /// Get a valid installation token, with file-based caching for CLI processes.
    ///
    /// Short-lived CLI processes (e.g., `mika credential-helper`) lose the in-memory
    /// cache between invocations. This method adds a file-based cache layer:
    ///
    /// 1. Read file cache → if valid, return immediately
    /// 2. Fall through to in-memory cache / JWT exchange
    /// 3. Write result to file cache (0o600 permissions)
    ///
    /// File cache failures are non-fatal — falls through to the normal token exchange.
    pub async fn installation_token_with_file_cache(&self, cache_path: &Path) -> Result<String> {
        // Try file cache first
        if let Some(token) = Self::read_file_cache(cache_path) {
            return Ok(token);
        }

        // Fall through to in-memory cache / JWT exchange
        let token = self.installation_token().await?;

        // Read the actual expiry from the in-memory cache (populated by installation_token)
        let expires_at = {
            let cache = self.cache.read().await;
            cache.as_ref().map(|c| c.expires_at)
        };

        // Best-effort write to file cache with actual expiry
        if let Some(expires_at) = expires_at {
            Self::write_file_cache(cache_path, &token, expires_at);
        }

        Ok(token)
    }

    /// Read a token from the file cache if it exists and is still valid.
    fn read_file_cache(cache_path: &Path) -> Option<String> {
        let content = std::fs::read_to_string(cache_path).ok()?;
        let cached: FileCachedToken = serde_json::from_str(&content).ok()?;

        // Parse expiry and check validity (same 5-minute buffer as in-memory cache)
        let expires_at = chrono::DateTime::parse_from_rfc3339(&cached.expires_at).ok()?;
        let expires_secs: u64 = expires_at.timestamp().try_into().ok()?;
        let expires_at_st = UNIX_EPOCH + Duration::from_secs(expires_secs);

        let in_memory = CachedToken {
            token: cached.token,
            expires_at: expires_at_st,
        };

        if Self::is_valid(&in_memory) {
            Some(in_memory.token)
        } else {
            None
        }
    }

    /// Write a token to the file cache with restrictive permissions.
    ///
    /// Uses atomic write (temp file + rename) with 0o600 permissions set at
    /// file creation time (via `OpenOptionsExt::mode`) to eliminate the TOCTOU
    /// race between write and chmod.
    fn write_file_cache(cache_path: &Path, token: &str, expires_at: SystemTime) {
        let expires_at_rfc3339 = {
            let secs = expires_at
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            chrono::DateTime::from_timestamp(secs as i64, 0)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default()
        };

        if expires_at_rfc3339.is_empty() {
            return;
        }

        let cached = FileCachedToken {
            token: token.to_string(),
            expires_at: expires_at_rfc3339,
        };

        let Ok(json) = serde_json::to_string_pretty(&cached) else {
            return;
        };

        // Write atomically: create temp file with 0o600, write, then rename.
        let tmp_path = cache_path.with_extension("tmp");
        let write_ok = {
            #[cfg(unix)]
            {
                use std::io::Write;
                use std::os::unix::fs::OpenOptionsExt;
                std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(&tmp_path)
                    .and_then(|mut f| f.write_all(json.as_bytes()))
                    .is_ok()
            }
            #[cfg(not(unix))]
            {
                std::fs::write(&tmp_path, &json).is_ok()
            }
        };

        if write_ok && std::fs::rename(&tmp_path, cache_path).is_err() {
            // Clean up temp file on failed rename
            let _ = std::fs::remove_file(&tmp_path);
        }
    }

    /// Check whether a cached token is still valid (has more than `EXPIRY_BUFFER` remaining).
    fn is_valid(cached: &CachedToken) -> bool {
        SystemTime::now()
            .checked_add(EXPIRY_BUFFER)
            .is_some_and(|threshold| cached.expires_at > threshold)
    }

    /// Generate a JWT for GitHub App authentication.
    ///
    /// Claims:
    /// - `iat`: current time minus 60 seconds (clock skew protection)
    /// - `exp`: `iat` + 600 seconds (10-minute GitHub maximum)
    /// - `iss`: app ID as string
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
        encode(&header, &claims, &self.signing_key).context("JWT signing failed")
    }

    /// Exchange a JWT for an installation access token via the GitHub API.
    async fn exchange_jwt_for_token(&self, jwt: &str) -> Result<CachedToken> {
        let url = format!(
            "https://api.github.com/app/installations/{}/access_tokens",
            self.installation_id
        );

        let resp = self
            .http_client
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
            let body_preview: String = body.chars().take(200).collect();
            anyhow::bail!(
                "GitHub installation token exchange failed (HTTP {status}): {body_preview}"
            );
        }

        #[derive(serde::Deserialize)]
        struct TokenResponse {
            token: String,
            expires_at: String,
        }

        let body: TokenResponse = resp
            .json()
            .await
            .context("failed to parse token response")?;

        let expires_at = chrono::DateTime::parse_from_rfc3339(&body.expires_at)
            .context("failed to parse expires_at")?;

        let expires_secs: u64 = expires_at
            .timestamp()
            .try_into()
            .context("expires_at before UNIX epoch")?;
        let expires_at = UNIX_EPOCH + Duration::from_secs(expires_secs);

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

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::SecretString;

    /// RSA 2048-bit test key (PKCS#1 PEM format, matching GitHub App export).
    /// Generated with: openssl genrsa -traditional 2048
    const TEST_RSA_PEM: &str = "-----BEGIN RSA PRIVATE KEY-----\n\
MIIEpAIBAAKCAQEAqmNXtQx4L3Eko0G+ky5u03BpRRwLfQ1+zuRzUxtDIAb2LFcf\n\
2PCCusvna5qAuXfCttcsTTFt0+x3vqI3wkO7pZ7MQatBcuQSFL3eSDhqNNLZ8zh6\n\
evsuCwgdhn+etApM8PtEpwcps/pjlLsIb9iyB7jcYBQsr0lGRrXVdsGPQXF0yGkr\n\
Vd3zH11tqLOWdDGXlpZTZkwwow7ojVID5POWHkp1WkY6xYCq6qYA1Gt9VfQNCzE8\n\
WKjsd0phBgN1W4le0Q30UFiaFDErbW1uqrsQShz0Wv9bHbUpVOyTotGdXOBWg0CP\n\
rJ5IBmt8KF73HLaK0zOwIe9qwlCLMszH4d+TaQIDAQABAoIBAA6rd/EwDibzhFaE\n\
Ag709/i/XGjlVb3iDBFvDNjSZ5CZ2NcPdz/70R2ZEacribquK3cHhppsz4pn+RVS\n\
LR/OKhlD100uG/fy1/WuNTWdmdNLdhVhPvZYqumrPLOISFcy7dXvpEUHMll7DNjQ\n\
05ShoQ5WJa8l/YTn96N940+Ssa1OHesGZJa4ATP+fxiXqow5Mq/DbLTWBQ0Kj2Qc\n\
WZFa6wc1ws61zK81U69gtW7+nnX2hzcboQhq8RVEmtJKINmfieuHSl0QOZsEuh09\n\
fFjLLwUhwIrmZKNv3hpqJpyKL6dvgr1f+5xyfgYUoQIFB2G8V7+Xto1urGYHNjRO\n\
DVWCbXMCgYEA04A9zJnYxwNPqnC86rxWy9fN0AsB8S4sWoO9M/ZWexfiWsMK6Mze\n\
uOfj1cVNjBm6aLJL6F2ts/ig4wA6alR72P5ZRqneAMFgIes5SP7j70U3gFodcCe/\n\
RoVhWNyjX4Oz9Dwu57QK5DB+3NRM/4On0wsO4GjQgl1RQnDZfYccRUcCgYEAzjyz\n\
CzQKzT21jyzb0/0xBovlUwxnctXV5lHScHETXh8TJdgD4gU+tBJNcJoa/swSNRgL\n\
6KfXj1LH4tbl0vBZps3RpuWVobqEZrkBjkJO9aGRsTkqtlEQJ0Lc3yQPbTWENlG+\n\
VbfrOkAyTn69LNmndOMBKq7syBrKJTtwgVcTec8CgYEAghGr79ftaPawV7FdfT62\n\
YkYlXHxohVpQDJpYEUy9gpX9rrOkUecsUarKgv0D49UuvpRn+k8iNDwDNZc+VYX/\n\
ZEOHw91TmkNSS4nNgQbARrXanCTPVdob19LPO0b1chgc42bfsb8Xs53fZw9pCvp8\n\
i12RmJDdKk8ZWjLsjjY5PKECgYB9EqC+nZwjZlYyc1EJ2hYeUz8LQ42FPhuPp3WJ\n\
DXpibVQOclfAfc/OIv9l13+hoJ82JdQrD4cR+3EPp6YPbAXivBV2Muuw/k2HgpFn\n\
9dyu6IJTyUiW8shqFwmeJd9ZKsh4rNBSacy1MfOQWRpfFcyRfY3aleUxYdXQCKEt\n\
P2KnTwKBgQCMt/E5AyZ1x7xsD68M/+dQc4kZG+3wyjfgkQ5tivveW5JxRNJ7Doy/\n\
Zk4PUTq3pSCC2sQY5Ay2b2iPez8d660jFuWT02+0sQdFmGwnFC9IxdEUPZXxeRr6\n\
omInFBLWVyWK89xoc49UvUcyRcbL3iWqa+zAv7eOC5TZyy1SVJtPVw==\n\
-----END RSA PRIVATE KEY-----";

    fn test_key_base64() -> String {
        base64::engine::general_purpose::STANDARD.encode(TEST_RSA_PEM.as_bytes())
    }

    fn make_test_settings(
        app_id: Option<u64>,
        private_key: Option<SecretString>,
        installation_id: Option<u64>,
    ) -> crate::config::Settings {
        let tmp = tempfile::tempdir().unwrap();
        let mut settings = crate::config::Settings::load(tmp.path()).unwrap();
        settings.github_app_id = app_id;
        settings.github_app_private_key = private_key;
        settings.github_app_installation_id = installation_id;
        settings
    }

    #[test]
    fn test_from_settings_complete() {
        let settings = make_test_settings(
            Some(12345),
            Some(SecretString::from(test_key_base64())),
            Some(67890),
        );
        let app = GitHubApp::from_settings(&settings);
        assert!(app.is_some());
        let app = app.unwrap();
        assert_eq!(app.app_id, 12345);
        assert_eq!(app.installation_id, 67890);
    }

    #[test]
    fn test_from_settings_missing_app_id() {
        let settings = make_test_settings(
            None,
            Some(SecretString::from(test_key_base64())),
            Some(67890),
        );
        assert!(GitHubApp::from_settings(&settings).is_none());
    }

    #[test]
    fn test_from_settings_missing_private_key() {
        let settings = make_test_settings(Some(12345), None, Some(67890));
        assert!(GitHubApp::from_settings(&settings).is_none());
    }

    #[test]
    fn test_from_settings_missing_installation_id() {
        let settings = make_test_settings(
            Some(12345),
            Some(SecretString::from(test_key_base64())),
            None,
        );
        assert!(GitHubApp::from_settings(&settings).is_none());
    }

    #[test]
    fn test_from_settings_invalid_base64() {
        let settings = make_test_settings(
            Some(12345),
            Some(SecretString::from("not-valid-base64!!!")),
            Some(67890),
        );
        assert!(GitHubApp::from_settings(&settings).is_none());
    }

    #[test]
    fn test_from_settings_invalid_pem() {
        // Valid base64 but not a PEM key
        let bad_pem_b64 =
            base64::engine::general_purpose::STANDARD.encode(b"this is not a PEM key");
        let settings = make_test_settings(
            Some(12345),
            Some(SecretString::from(bad_pem_b64)),
            Some(67890),
        );
        assert!(GitHubApp::from_settings(&settings).is_none());
    }

    #[test]
    fn test_generate_jwt_claims() {
        let pem_bytes = TEST_RSA_PEM.as_bytes();
        let signing_key = EncodingKey::from_rsa_pem(pem_bytes).unwrap();
        let app = GitHubApp::new(12345, signing_key, 67890);

        let jwt = app.generate_jwt().unwrap();

        // Decode the JWT to verify claims (without signature verification — we just
        // want to check the payload structure).
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3, "JWT should have 3 parts");

        let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).unwrap();

        // iss should be the app_id as a string
        assert_eq!(payload["iss"], "12345");

        // exp - iat should equal JWT_LIFETIME (540 seconds).
        // Pinned at 540s to tolerate ~120s of positive host-clock skew
        // below GitHub's hard 600s ceiling. See mika#1042.
        let iat = payload["iat"].as_u64().unwrap();
        let exp = payload["exp"].as_u64().unwrap();
        assert_eq!(exp - iat, JWT_LIFETIME.as_secs());
        assert_eq!(JWT_LIFETIME.as_secs(), 540);

        // iat should be backdated (roughly current_time - 60s)
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let diff = now.abs_diff(iat + IAT_BACKDATE.as_secs());
        assert!(
            diff < 5,
            "iat should be ~60s before current time, diff={diff}"
        );
    }

    #[test]
    fn test_cache_validity() {
        // Token expiring in 10 minutes — should be valid (> 5min buffer)
        let valid_token = CachedToken {
            token: "valid".into(),
            expires_at: SystemTime::now() + Duration::from_secs(600),
        };
        assert!(GitHubApp::is_valid(&valid_token));

        // Token expiring in 2 minutes — should be invalid (< 5min buffer)
        let expiring_token = CachedToken {
            token: "expiring".into(),
            expires_at: SystemTime::now() + Duration::from_secs(120),
        };
        assert!(!GitHubApp::is_valid(&expiring_token));

        // Already expired token
        let expired_token = CachedToken {
            token: "expired".into(),
            expires_at: SystemTime::now() - Duration::from_secs(60),
        };
        assert!(!GitHubApp::is_valid(&expired_token));
    }

    #[test]
    fn test_file_cache_write_and_read() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_path = tmp.path().join("github_app_token.json");

        // Write a token with 30-minute expiry (well within the 5-minute buffer)
        let expires_at = SystemTime::now() + Duration::from_secs(1800);
        GitHubApp::write_file_cache(&cache_path, "ghs_test_token_123", expires_at);

        // Read it back — should succeed (token is fresh)
        let result = GitHubApp::read_file_cache(&cache_path);
        assert_eq!(result.as_deref(), Some("ghs_test_token_123"));

        // Verify file permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(&cache_path).unwrap().permissions();
            assert_eq!(perms.mode() & 0o777, 0o600, "File should be owner-only");
        }
    }

    #[test]
    fn test_file_cache_expired_token() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_path = tmp.path().join("github_app_token.json");

        // Write a token that expired 10 minutes ago
        let expired = chrono::Utc::now() - chrono::Duration::minutes(10);
        let cached = FileCachedToken {
            token: "ghs_expired".to_string(),
            expires_at: expired.to_rfc3339(),
        };
        std::fs::write(&cache_path, serde_json::to_string(&cached).unwrap()).unwrap();

        // Should return None — token is expired
        assert!(GitHubApp::read_file_cache(&cache_path).is_none());
    }

    #[test]
    fn test_file_cache_within_expiry_buffer() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_path = tmp.path().join("github_app_token.json");

        // Write a token that expires in 3 minutes (within 5-minute buffer)
        let near_expiry = chrono::Utc::now() + chrono::Duration::minutes(3);
        let cached = FileCachedToken {
            token: "ghs_near_expiry".to_string(),
            expires_at: near_expiry.to_rfc3339(),
        };
        std::fs::write(&cache_path, serde_json::to_string(&cached).unwrap()).unwrap();

        // Should return None — within expiry buffer
        assert!(GitHubApp::read_file_cache(&cache_path).is_none());
    }

    #[test]
    fn test_file_cache_nonexistent_file() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_path = tmp.path().join("nonexistent.json");

        assert!(GitHubApp::read_file_cache(&cache_path).is_none());
    }

    #[test]
    fn test_file_cache_corrupt_json() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_path = tmp.path().join("github_app_token.json");

        std::fs::write(&cache_path, "not valid json").unwrap();

        assert!(GitHubApp::read_file_cache(&cache_path).is_none());
    }

    #[test]
    fn test_debug_redacts_secrets() {
        let pem_bytes = TEST_RSA_PEM.as_bytes();
        let signing_key = EncodingKey::from_rsa_pem(pem_bytes).unwrap();
        let app = GitHubApp::new(12345, signing_key, 67890);

        let debug = format!("{:?}", app);
        assert!(debug.contains("app_id: 12345"));
        assert!(debug.contains("installation_id: 67890"));
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("RSA"));
        assert!(!debug.contains("PRIVATE"));
    }
}

//! GitHub App authentication: JWT signing, installation token exchange, and caching.
//!
//! Follows the [`OAuthTokenManager`](crate::oauth::OAuthTokenManager) pattern —
//! `tokio::sync::RwLock`-based cache with double-checked locking for thundering-herd
//! prevention.

use anyhow::{Context, Result};
use base64::Engine;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use secrecy::ExposeSecret;
use serde::Serialize;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Buffer before expiry to trigger proactive refresh (5 minutes).
const EXPIRY_BUFFER: Duration = Duration::from_secs(5 * 60);

/// Clock skew backdating for `iat` claim (GitHub recommendation: 60 seconds).
const IAT_BACKDATE: Duration = Duration::from_secs(60);

/// JWT lifetime (GitHub maximum: 10 minutes).
const JWT_LIFETIME: Duration = Duration::from_secs(600);

/// Cached installation token with expiry.
struct CachedToken {
    token: String,
    expires_at: SystemTime,
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
    pub fn new(
        app_id: u64,
        signing_key: EncodingKey,
        installation_id: u64,
    ) -> Arc<Self> {
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
            anyhow::bail!("GitHub installation token exchange failed (HTTP {status}): {body}");
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

        // exp - iat should equal JWT_LIFETIME (600 seconds)
        let iat = payload["iat"].as_u64().unwrap();
        let exp = payload["exp"].as_u64().unwrap();
        assert_eq!(exp - iat, JWT_LIFETIME.as_secs());

        // iat should be backdated (roughly current_time - 60s)
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let diff = now.abs_diff(iat + IAT_BACKDATE.as_secs());
        assert!(diff < 5, "iat should be ~60s before current time, diff={diff}");
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

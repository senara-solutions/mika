//! OAuth PKCE token exchange for Anthropic subscription tokens (`sk-ant-oat*`).
//!
//! Implements the Authorization Code + PKCE flow required to obtain short-lived
//! access tokens from Anthropic's OAuth system. Raw subscription tokens cannot
//! be used directly with the messages API — they must be exchanged first.
//!
//! # Flow
//!
//! 1. `generate_pkce_params()` — builds the authorization URL with PKCE challenge
//! 2. User opens URL, authorizes, and copies the authorization code
//! 3. `exchange_code()` — exchanges the code + verifier for tokens
//! 4. `save_oauth_tokens()` — persists tokens to `~/.mika/oauth.json`
//! 5. `OAuthTokenManager::get_valid_token()` — transparently refreshes on expiry

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use base64::Engine;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use tracing::{info, warn};

// -- OAuth constants (from Anthropic's pi-ai SDK) --

const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";
const REDIRECT_URI: &str = "https://console.anthropic.com/oauth/code/callback";
const SCOPES: &str = "org:create_api_key user:profile user:inference";
const OAUTH_FILE: &str = "oauth.json";

/// Seconds before actual expiry to proactively refresh the access token.
/// Handles clock skew and avoids one failed request per expiry cycle.
const REFRESH_BUFFER_SECS: i64 = 60;

// -- Types --

/// PKCE flow parameters: the authorization URL the user must visit,
/// the code verifier to use during token exchange, and the CSRF state.
pub struct PkceParams {
    /// Full authorization URL including PKCE challenge parameters.
    pub authorize_url: String,
    /// Code verifier — must be sent during token exchange (kept secret, never shared).
    pub code_verifier: String,
    /// Random state parameter for CSRF protection.
    pub state: String,
}

/// Persisted OAuth token credentials stored in `~/.mika/oauth.json`.
#[derive(Serialize, Deserialize, Clone)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    /// ISO 8601 timestamp when the access token expires.
    pub expires_at: String,
    /// SHA-256 hash of the subscription token used to obtain these tokens.
    /// Used to detect when the user switches to a different subscription token.
    pub subscription_token_hash: String,
}

impl fmt::Debug for OAuthTokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthTokens")
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .field("subscription_token_hash", &self.subscription_token_hash)
            .finish()
    }
}

/// Token endpoint response from Anthropic's OAuth server.
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    /// Token lifetime in seconds.
    expires_in: i64,
}

/// Thread-safe token lifecycle manager.
///
/// Holds the subscription token hash, loads/persists `oauth.json`,
/// and transparently refreshes expired access tokens. Safe to share
/// via `Arc` across multiple concurrent callers (server mode).
pub struct OAuthTokenManager {
    subscription_hash: String,
    home_dir: PathBuf,
    cache: tokio::sync::RwLock<Option<OAuthTokens>>,
    http_client: reqwest::Client,
}

impl fmt::Debug for OAuthTokenManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthTokenManager")
            .field("home_dir", &self.home_dir)
            .field("subscription_hash", &self.subscription_hash)
            .finish()
    }
}

// -- PKCE helpers --

/// Base64url-encode without padding (RFC 7636 / RFC 4648 Section 5).
fn base64url_encode(data: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

/// Compute SHA-256 hash and return hex-encoded string.
fn sha256_hex(data: &str) -> String {
    hex::encode(sha2::Sha256::digest(data.as_bytes()))
}

/// Generate PKCE code verifier (32 random bytes, base64url-encoded).
fn generate_code_verifier() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    base64url_encode(&bytes)
}

/// Compute PKCE code challenge from a verifier (SHA-256, base64url-encoded).
fn generate_code_challenge(verifier: &str) -> String {
    let hash = sha2::Sha256::digest(verifier.as_bytes());
    base64url_encode(&hash)
}

/// Generate a random hex state parameter for CSRF protection.
fn generate_state() -> String {
    let mut bytes = [0u8; 16];
    rand::fill(&mut bytes);
    hex::encode(bytes)
}

/// Maximum acceptable `expires_in` value from the token endpoint (30 days).
const MAX_EXPIRES_IN_SECS: i64 = 86400 * 30;

/// Validate the `expires_in` field from the token endpoint response.
fn validate_expires_in(expires_in: i64) -> Result<()> {
    if expires_in <= 0 {
        anyhow::bail!("token endpoint returned non-positive expires_in: {expires_in}");
    }
    if expires_in > MAX_EXPIRES_IN_SECS {
        anyhow::bail!(
            "token endpoint returned unreasonably large expires_in: {expires_in}s (max {MAX_EXPIRES_IN_SECS}s)"
        );
    }
    Ok(())
}

// -- Public API --

/// Generate PKCE parameters and build the full authorization URL.
///
/// The caller should present `authorize_url` to the user (open browser or print),
/// keep `code_verifier` in memory, and use it when calling `exchange_code()`.
pub fn generate_pkce_params() -> PkceParams {
    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);
    let state = generate_state();

    let authorize_url = url::Url::parse_with_params(
        AUTHORIZE_URL,
        &[
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", REDIRECT_URI),
            ("scope", SCOPES),
            ("code_challenge", &code_challenge),
            ("code_challenge_method", "S256"),
            ("state", &state),
        ],
    )
    .expect("static URL with valid params should always parse")
    .to_string();

    PkceParams {
        authorize_url,
        code_verifier,
        state,
    }
}

/// Exchange an authorization code for OAuth tokens.
///
/// `code` is the authorization code from the callback page.
/// `code_verifier` is the PKCE verifier generated by `generate_pkce_params()`.
/// `subscription_token` is the original `sk-ant-oat*` token (used for hash tracking).
pub async fn exchange_code(
    code: &str,
    code_verifier: &str,
    subscription_token: &str,
) -> Result<OAuthTokens> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("failed to build HTTP client for token exchange")?;

    let response = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT_URI),
            ("client_id", CLIENT_ID),
            ("code_verifier", code_verifier),
        ])
        .send()
        .await
        .context("failed to connect to Anthropic token endpoint")?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let truncated: String = body.chars().take(200).collect();
        anyhow::bail!("token exchange failed (HTTP {status}): {truncated}");
    }

    let token_response: TokenResponse = response
        .json()
        .await
        .context("failed to parse token exchange response")?;

    validate_expires_in(token_response.expires_in)?;
    let expires_at = Utc::now() + chrono::Duration::seconds(token_response.expires_in);

    Ok(OAuthTokens {
        access_token: token_response.access_token,
        refresh_token: token_response.refresh_token,
        expires_at: expires_at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        subscription_token_hash: sha256_hex(subscription_token),
    })
}

/// Load OAuth tokens from `{home_dir}/oauth.json`.
///
/// Returns `Ok(None)` if the file does not exist.
/// Returns `Err` if the file exists but cannot be read or parsed.
pub fn load_oauth_tokens(home_dir: &Path) -> Result<Option<OAuthTokens>> {
    let path = home_dir.join(OAUTH_FILE);
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let tokens: OAuthTokens = serde_json::from_str(&content)
                .with_context(|| format!("failed to parse {}", path.display()))?;
            Ok(Some(tokens))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("failed to read {}", path.display())),
    }
}

/// Save OAuth tokens to `{home_dir}/oauth.json` with 0600 permissions.
///
/// Uses atomic write (temp file + chmod + rename) consistent with `dotenv::set_env_var()`.
pub fn save_oauth_tokens(home_dir: &Path, tokens: &OAuthTokens) -> Result<()> {
    let path = home_dir.join(OAUTH_FILE);
    let content =
        serde_json::to_string_pretty(tokens).context("failed to serialize OAuth tokens")?;

    // Atomic write: temp file → chmod → rename
    let tmp_path = path.with_file_name(".oauth.json.tmp");
    std::fs::write(&tmp_path, &content)
        .with_context(|| format!("failed to write {}", tmp_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))
        {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e).context("failed to set permissions on oauth.json");
        }
    }

    std::fs::rename(&tmp_path, &path).with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp_path.display(),
            path.display()
        )
    })?;

    Ok(())
}

// -- OAuthTokenManager --

impl OAuthTokenManager {
    /// Create a new token manager for the given subscription token.
    ///
    /// Does NOT eagerly load or validate tokens — resolution is lazy (first API call).
    pub fn new(subscription_token: &str, home_dir: PathBuf) -> Self {
        let subscription_hash = sha256_hex(subscription_token);
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client for OAuth token manager");

        Self {
            subscription_hash,
            home_dir,
            cache: tokio::sync::RwLock::new(None),
            http_client,
        }
    }

    /// Get a valid access token, refreshing if expired.
    ///
    /// - If cached token is valid (not within `REFRESH_BUFFER_SECS` of expiry), return it.
    /// - If cached token is expired, refresh using the stored refresh token.
    /// - If no cached token exists, load from disk.
    /// - If no token on disk or subscription token changed, return an error
    ///   directing the user to run `mika setup --mode oauth`.
    pub async fn get_valid_token(&self) -> Result<String> {
        // Fast path: read lock, check cache
        {
            let cache = self.cache.read().await;
            if let Some(ref tokens) = *cache
                && self.is_token_valid(tokens)
            {
                return Ok(tokens.access_token.clone());
            }
        }

        // Slow path: write lock, refresh or load
        let mut cache = self.cache.write().await;

        // Double-check: another task may have refreshed while we waited for the lock
        if let Some(ref tokens) = *cache
            && self.is_token_valid(tokens)
        {
            return Ok(tokens.access_token.clone());
        }

        // Try to load from disk if cache is empty
        if cache.is_none()
            && let Some(tokens) = load_oauth_tokens(&self.home_dir)?
        {
            if tokens.subscription_token_hash == self.subscription_hash {
                if self.is_token_valid(&tokens) {
                    let access_token = tokens.access_token.clone();
                    *cache = Some(tokens);
                    return Ok(access_token);
                }
                // Token loaded but expired — continue to refresh
                *cache = Some(tokens);
            } else {
                warn!("subscription token changed — cached OAuth tokens are invalid");
                // Hash mismatch: subscription token changed, need re-auth
            }
        }

        // If we have tokens (possibly expired), try to refresh
        if let Some(ref tokens) = *cache {
            match self.refresh_tokens(&tokens.refresh_token).await {
                Ok(new_tokens) => {
                    save_oauth_tokens(&self.home_dir, &new_tokens)?;
                    let access_token = new_tokens.access_token.clone();
                    info!("OAuth access token refreshed successfully");
                    *cache = Some(new_tokens);
                    return Ok(access_token);
                }
                Err(e) => {
                    warn!(error = %e, "OAuth token refresh failed");
                    // Try re-reading from disk in case another process refreshed
                    if let Ok(Some(disk_tokens)) = load_oauth_tokens(&self.home_dir)
                        && disk_tokens.subscription_token_hash == self.subscription_hash
                        && self.is_token_valid(&disk_tokens)
                    {
                        let access_token = disk_tokens.access_token.clone();
                        *cache = Some(disk_tokens);
                        return Ok(access_token);
                    }
                    anyhow::bail!(
                        "OAuth token refresh failed: {e:#}. \
                         Run `mika setup --mode oauth` to re-authorize."
                    );
                }
            }
        }

        anyhow::bail!(
            "No OAuth tokens found. Run `mika setup --mode oauth` to authorize \
             Mika with your Claude Pro/Max subscription."
        )
    }

    /// Force a token refresh, regardless of expiry.
    ///
    /// Used after a 401 response when the token was believed to be valid
    /// (handles clock skew between client and server).
    pub(crate) async fn force_refresh(&self) -> Result<String> {
        let mut cache = self.cache.write().await;

        // Load from disk if cache is empty
        if cache.is_none()
            && let Some(tokens) = load_oauth_tokens(&self.home_dir)?
            && tokens.subscription_token_hash == self.subscription_hash
        {
            *cache = Some(tokens);
        }

        let refresh_token = cache
            .as_ref()
            .map(|t| t.refresh_token.clone())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No OAuth tokens available for force-refresh. \
                     Run `mika setup --mode oauth` to authorize."
                )
            })?;

        let new_tokens = self.refresh_tokens(&refresh_token).await?;
        save_oauth_tokens(&self.home_dir, &new_tokens)?;
        let access_token = new_tokens.access_token.clone();
        info!("OAuth access token force-refreshed successfully");
        *cache = Some(new_tokens);
        Ok(access_token)
    }

    /// Check if the access token is still valid (not within the refresh buffer).
    fn is_token_valid(&self, tokens: &OAuthTokens) -> bool {
        // Our format (%Y-%m-%dT%H:%M:%SZ) is valid RFC 3339 — Z is accepted.
        let Ok(expires_at) = chrono::DateTime::parse_from_rfc3339(&tokens.expires_at) else {
            warn!(
                expires_at = %tokens.expires_at,
                "failed to parse OAuth token expiry timestamp"
            );
            return false;
        };

        let now = Utc::now();
        let buffer = chrono::Duration::seconds(REFRESH_BUFFER_SECS);
        now < (expires_at - buffer)
    }

    /// Refresh tokens using the stored refresh token.
    async fn refresh_tokens(&self, refresh_token: &str) -> Result<OAuthTokens> {
        let response = self
            .http_client
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", CLIENT_ID),
            ])
            .send()
            .await
            .context("failed to connect to Anthropic token endpoint for refresh")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let truncated: String = body.chars().take(200).collect();
            anyhow::bail!("token refresh failed (HTTP {status}): {truncated}");
        }

        let token_response: TokenResponse = response
            .json()
            .await
            .context("failed to parse token refresh response")?;

        validate_expires_in(token_response.expires_in)?;
        let expires_at = Utc::now() + chrono::Duration::seconds(token_response.expires_in);

        Ok(OAuthTokens {
            access_token: token_response.access_token,
            refresh_token: token_response.refresh_token,
            expires_at: expires_at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            subscription_token_hash: self.subscription_hash.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_code_verifier_length() {
        let verifier = generate_code_verifier();
        // 32 bytes base64url-encoded without padding = 43 chars
        assert_eq!(verifier.len(), 43);
    }

    #[test]
    fn test_generate_code_verifier_is_base64url() {
        let verifier = generate_code_verifier();
        assert!(
            verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "verifier should be base64url (no padding): {verifier}"
        );
    }

    #[test]
    fn test_generate_code_challenge_is_base64url() {
        let verifier = generate_code_verifier();
        let challenge = generate_code_challenge(&verifier);
        // SHA-256 = 32 bytes, base64url-encoded without padding = 43 chars
        assert_eq!(challenge.len(), 43);
        assert!(
            challenge
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "challenge should be base64url (no padding): {challenge}"
        );
    }

    #[test]
    fn test_generate_code_challenge_deterministic() {
        // Same verifier should produce same challenge
        let challenge1 = generate_code_challenge("test-verifier");
        let challenge2 = generate_code_challenge("test-verifier");
        assert_eq!(challenge1, challenge2);
    }

    #[test]
    fn test_generate_code_challenge_known_value() {
        // Known SHA-256 of "test" is 9f86d08... base64url of the hash
        let challenge = generate_code_challenge("test");
        // SHA-256("test") = 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08
        // base64url of those 32 bytes
        let expected_hash = sha2::Sha256::digest(b"test");
        let expected = base64url_encode(&expected_hash);
        assert_eq!(challenge, expected);
    }

    #[test]
    fn test_generate_state_length() {
        let state = generate_state();
        // 16 bytes hex-encoded = 32 chars
        assert_eq!(state.len(), 32);
    }

    #[test]
    fn test_generate_state_is_hex() {
        let state = generate_state();
        assert!(
            state.chars().all(|c| c.is_ascii_hexdigit()),
            "state should be hex: {state}"
        );
    }

    #[test]
    fn test_generate_state_uniqueness() {
        let s1 = generate_state();
        let s2 = generate_state();
        assert_ne!(s1, s2);
    }

    #[test]
    fn test_generate_pkce_params_url_structure() {
        let params = generate_pkce_params();

        let url = url::Url::parse(&params.authorize_url).unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("claude.ai"));
        assert_eq!(url.path(), "/oauth/authorize");

        let query_pairs: std::collections::HashMap<_, _> = url
            .query_pairs()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        assert_eq!(query_pairs.get("response_type").unwrap(), "code");
        assert_eq!(query_pairs.get("client_id").unwrap(), CLIENT_ID);
        assert_eq!(query_pairs.get("redirect_uri").unwrap(), REDIRECT_URI);
        assert_eq!(query_pairs.get("scope").unwrap(), SCOPES);
        assert_eq!(query_pairs.get("code_challenge_method").unwrap(), "S256");
        assert!(query_pairs.contains_key("code_challenge"));
        assert!(query_pairs.contains_key("state"));

        // Verify the challenge matches the verifier
        let expected_challenge = generate_code_challenge(&params.code_verifier);
        assert_eq!(
            query_pairs.get("code_challenge").unwrap(),
            &expected_challenge
        );

        // State in URL matches returned state
        assert_eq!(query_pairs.get("state").unwrap(), &params.state);
    }

    #[test]
    fn test_sha256_hex() {
        // Known value: SHA-256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        let hash = sha256_hex("hello");
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn test_oauth_tokens_debug_redacts_secrets() {
        let tokens = OAuthTokens {
            access_token: "super-secret-access-token".into(),
            refresh_token: "super-secret-refresh-token".into(),
            expires_at: "2026-01-01T00:00:00Z".into(),
            subscription_token_hash: "abc123".into(),
        };
        let debug = format!("{:?}", tokens);
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("super-secret"));
    }

    #[test]
    fn test_oauth_tokens_serialization_roundtrip() {
        let tokens = OAuthTokens {
            access_token: "access-123".into(),
            refresh_token: "refresh-456".into(),
            expires_at: "2026-03-21T12:00:00Z".into(),
            subscription_token_hash: "hash-789".into(),
        };
        let json = serde_json::to_string(&tokens).unwrap();
        let deserialized: OAuthTokens = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.access_token, "access-123");
        assert_eq!(deserialized.refresh_token, "refresh-456");
        assert_eq!(deserialized.expires_at, "2026-03-21T12:00:00Z");
        assert_eq!(deserialized.subscription_token_hash, "hash-789");
    }

    #[test]
    fn test_load_oauth_tokens_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let result = load_oauth_tokens(tmp.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_save_and_load_oauth_tokens() {
        let tmp = tempfile::tempdir().unwrap();
        let tokens = OAuthTokens {
            access_token: "access-abc".into(),
            refresh_token: "refresh-def".into(),
            expires_at: "2026-12-31T23:59:59Z".into(),
            subscription_token_hash: "hash-ghi".into(),
        };
        save_oauth_tokens(tmp.path(), &tokens).unwrap();

        let loaded = load_oauth_tokens(tmp.path()).unwrap().unwrap();
        assert_eq!(loaded.access_token, "access-abc");
        assert_eq!(loaded.refresh_token, "refresh-def");
        assert_eq!(loaded.expires_at, "2026-12-31T23:59:59Z");
        assert_eq!(loaded.subscription_token_hash, "hash-ghi");
    }

    #[cfg(unix)]
    #[test]
    fn test_save_oauth_tokens_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let tokens = OAuthTokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: "2026-01-01T00:00:00Z".into(),
            subscription_token_hash: "h".into(),
        };
        save_oauth_tokens(tmp.path(), &tokens).unwrap();

        let perms = std::fs::metadata(tmp.path().join(OAUTH_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(perms, 0o600);
    }

    #[test]
    fn test_load_oauth_tokens_corrupt_json() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(OAUTH_FILE), "not valid json").unwrap();
        let result = load_oauth_tokens(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_token_manager_new() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = OAuthTokenManager::new("sk-ant-oat01-test", tmp.path().to_path_buf());
        assert_eq!(manager.subscription_hash, sha256_hex("sk-ant-oat01-test"));
    }

    #[test]
    fn test_is_token_valid_future_expiry() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = OAuthTokenManager::new("test", tmp.path().to_path_buf());

        let future = Utc::now() + chrono::Duration::hours(1);
        let tokens = OAuthTokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: future.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            subscription_token_hash: sha256_hex("test"),
        };
        assert!(manager.is_token_valid(&tokens));
    }

    #[test]
    fn test_is_token_valid_past_expiry() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = OAuthTokenManager::new("test", tmp.path().to_path_buf());

        let past = Utc::now() - chrono::Duration::hours(1);
        let tokens = OAuthTokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: past.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            subscription_token_hash: sha256_hex("test"),
        };
        assert!(!manager.is_token_valid(&tokens));
    }

    #[test]
    fn test_is_token_valid_within_buffer() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = OAuthTokenManager::new("test", tmp.path().to_path_buf());

        // Expires in 30 seconds — within the 60-second buffer
        let soon = Utc::now() + chrono::Duration::seconds(30);
        let tokens = OAuthTokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: soon.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            subscription_token_hash: sha256_hex("test"),
        };
        assert!(!manager.is_token_valid(&tokens));
    }

    #[test]
    fn test_is_token_valid_invalid_timestamp() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = OAuthTokenManager::new("test", tmp.path().to_path_buf());

        let tokens = OAuthTokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: "not-a-timestamp".into(),
            subscription_token_hash: sha256_hex("test"),
        };
        assert!(!manager.is_token_valid(&tokens));
    }

    #[tokio::test]
    async fn test_get_valid_token_no_tokens() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = OAuthTokenManager::new("test-token", tmp.path().to_path_buf());

        let result = manager.get_valid_token().await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("No OAuth tokens found"),
            "error should mention missing tokens: {err}"
        );
    }

    #[tokio::test]
    async fn test_get_valid_token_loads_from_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let subscription_token = "sk-ant-oat01-test-subscription";
        let manager = OAuthTokenManager::new(subscription_token, tmp.path().to_path_buf());

        let future = Utc::now() + chrono::Duration::hours(1);
        let tokens = OAuthTokens {
            access_token: "disk-access-token".into(),
            refresh_token: "disk-refresh-token".into(),
            expires_at: future.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            subscription_token_hash: sha256_hex(subscription_token),
        };
        save_oauth_tokens(tmp.path(), &tokens).unwrap();

        let result = manager.get_valid_token().await.unwrap();
        assert_eq!(result, "disk-access-token");
    }

    #[tokio::test]
    async fn test_get_valid_token_hash_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = OAuthTokenManager::new("new-subscription-token", tmp.path().to_path_buf());

        let future = Utc::now() + chrono::Duration::hours(1);
        let tokens = OAuthTokens {
            access_token: "old-access-token".into(),
            refresh_token: "old-refresh-token".into(),
            expires_at: future.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            subscription_token_hash: sha256_hex("old-subscription-token"),
        };
        save_oauth_tokens(tmp.path(), &tokens).unwrap();

        let result = manager.get_valid_token().await;
        assert!(
            result.is_err(),
            "should reject tokens with mismatched subscription hash"
        );
    }

    #[tokio::test]
    async fn test_get_valid_token_uses_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let subscription_token = "sk-ant-oat01-cached";
        let manager = OAuthTokenManager::new(subscription_token, tmp.path().to_path_buf());

        let future = Utc::now() + chrono::Duration::hours(1);
        let tokens = OAuthTokens {
            access_token: "cached-access".into(),
            refresh_token: "cached-refresh".into(),
            expires_at: future.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            subscription_token_hash: sha256_hex(subscription_token),
        };
        save_oauth_tokens(tmp.path(), &tokens).unwrap();

        // First call loads from disk
        let result1 = manager.get_valid_token().await.unwrap();
        assert_eq!(result1, "cached-access");

        // Delete the file — second call should use cache
        std::fs::remove_file(tmp.path().join(OAUTH_FILE)).unwrap();

        let result2 = manager.get_valid_token().await.unwrap();
        assert_eq!(result2, "cached-access");
    }

    #[test]
    fn test_oauth_token_manager_debug_redacts() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = OAuthTokenManager::new("secret-token", tmp.path().to_path_buf());
        let debug = format!("{:?}", manager);
        assert!(!debug.contains("secret-token"));
        assert!(debug.contains("OAuthTokenManager"));
    }
}

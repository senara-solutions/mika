//! Shared validation helpers for CLI diagnostics and config management.

use std::path::{Path, PathBuf};

/// Result of API key format validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiKeyFormat {
    /// Standard Anthropic API key (sk-ant-*)
    ApiKey,
    /// OAuth subscription token (sk-ant-oat*)
    OAuthToken,
    /// Non-Anthropic provider key (OpenAI, Groq, etc.)
    ThirdParty,
}

/// Validate the format of an LLM API key.
///
/// Returns the detected format, or an error if the key is empty.
/// Recognizes Anthropic API keys, OAuth tokens, and third-party provider keys.
pub fn validate_api_key_format(key: &str) -> Result<ApiKeyFormat, String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("API key is empty".to_string());
    }
    if crate::claude::is_oauth_token(key) {
        Ok(ApiKeyFormat::OAuthToken)
    } else if key.starts_with("sk-ant-") {
        Ok(ApiKeyFormat::ApiKey)
    } else if key.len() < 20 {
        Err("API key appears too short — verify you copied the full key".to_string())
    } else {
        Ok(ApiKeyFormat::ThirdParty)
    }
}

/// Check permissions on a path. Returns `(exists, mode_octal)`.
/// On non-Unix, always returns mode 0.
#[cfg(unix)]
pub fn check_path_permissions(path: &Path) -> Result<(bool, u32), String> {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(meta) => {
            let mode = meta.permissions().mode() & 0o777;
            Ok((true, mode))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((false, 0)),
        Err(e) => Err(format!("Cannot read {}: {e}", path.display())),
    }
}

#[cfg(not(unix))]
pub fn check_path_permissions(path: &Path) -> Result<(bool, u32), String> {
    match path.exists() {
        true => Ok((true, 0)),
        false => Ok((false, 0)),
    }
}

/// Search for a binary in PATH directories. Returns the first match.
pub fn check_binary_in_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let full = dir.join(name);
            if full.is_file() { Some(full) } else { None }
        })
    })
}

/// Validate a value for a config.toml (File backend) key.
pub fn validate_file_key(key: &str, value: &str) -> Result<(), String> {
    match key {
        "llm_model" | "embedding_model" => {
            if value.trim().is_empty() {
                return Err(format!("{key} cannot be empty"));
            }
        }
        "llm_max_tokens" => {
            let n: u32 = value
                .parse()
                .map_err(|_| format!("Invalid {key}: must be a positive integer"))?;
            if n == 0 || n > 131_072 {
                return Err(format!("Invalid {key}: must be between 1 and 131072"));
            }
        }
        "log_level" => {
            if !matches!(value, "trace" | "debug" | "info" | "warn" | "error" | "off") {
                return Err(format!(
                    "Invalid {key}: must be one of trace, debug, info, warn, error, off"
                ));
            }
        }
        "server_port" => {
            let n: u16 = value
                .parse()
                .map_err(|_| format!("Invalid {key}: must be an integer 1-65535"))?;
            if n == 0 {
                return Err(format!("Invalid {key}: must be between 1 and 65535"));
            }
        }
        "embedding_dimensions" => {
            let n: u32 = value
                .parse()
                .map_err(|_| format!("Invalid {key}: must be a positive integer"))?;
            if n == 0 || n > 4096 {
                return Err(format!("Invalid {key}: must be between 1 and 4096"));
            }
        }
        "llm_base_url" => {
            let value = value.trim();
            if value.is_empty() {
                return Err(format!("{key} cannot be empty"));
            }
            let parsed = reqwest::Url::parse(value).map_err(|e| format!("Invalid {key}: {e}"))?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err(format!(
                    "Invalid {key}: must use http or https scheme, got '{}'",
                    parsed.scheme()
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_api_key_format_standard() {
        assert_eq!(
            validate_api_key_format("sk-ant-api03-something"),
            Ok(ApiKeyFormat::ApiKey)
        );
    }

    #[test]
    fn test_validate_api_key_format_oauth() {
        assert_eq!(
            validate_api_key_format("sk-ant-oat01-something"),
            Ok(ApiKeyFormat::OAuthToken)
        );
    }

    #[test]
    fn test_validate_api_key_format_empty() {
        assert!(validate_api_key_format("").is_err());
        assert!(validate_api_key_format("   ").is_err());
    }

    #[test]
    fn test_validate_api_key_format_third_party() {
        // OpenAI key (>= 20 chars)
        assert_eq!(
            validate_api_key_format("sk-proj-something-long-enough"),
            Ok(ApiKeyFormat::ThirdParty)
        );
        // Groq key (>= 20 chars)
        assert_eq!(
            validate_api_key_format("gsk_something_long_enough"),
            Ok(ApiKeyFormat::ThirdParty)
        );
        // Exactly 20 characters should pass
        assert_eq!(
            validate_api_key_format("12345678901234567890"),
            Ok(ApiKeyFormat::ThirdParty)
        );
    }

    #[test]
    fn test_validate_api_key_format_third_party_too_short() {
        // Keys shorter than 20 characters should be rejected
        assert!(validate_api_key_format("some-key").is_err());
        assert!(validate_api_key_format("short").is_err());
        assert_eq!(
            validate_api_key_format("tooshort"),
            Err("API key appears too short — verify you copied the full key".to_string())
        );
        // 19 characters — still too short
        assert!(validate_api_key_format("1234567890123456789").is_err());
    }

    #[test]
    fn test_check_binary_in_path_existing() {
        // `sh` should exist on any Unix system
        if cfg!(unix) {
            assert!(check_binary_in_path("sh").is_some());
        }
    }

    #[test]
    fn test_check_binary_in_path_nonexistent() {
        assert!(check_binary_in_path("nonexistent_binary_xyz_12345").is_none());
    }

    #[test]
    fn test_validate_file_key_llm_model() {
        assert!(validate_file_key("llm_model", "claude-sonnet-4-6").is_ok());
        assert!(validate_file_key("llm_model", "").is_err());
        assert!(validate_file_key("llm_model", "  ").is_err());
    }

    #[test]
    fn test_validate_file_key_max_tokens() {
        assert!(validate_file_key("llm_max_tokens", "4096").is_ok());
        assert!(validate_file_key("llm_max_tokens", "1").is_ok());
        assert!(validate_file_key("llm_max_tokens", "131072").is_ok());
        assert!(validate_file_key("llm_max_tokens", "0").is_err());
        assert!(validate_file_key("llm_max_tokens", "131073").is_err());
        assert!(validate_file_key("llm_max_tokens", "abc").is_err());
    }

    #[test]
    fn test_validate_file_key_log_level() {
        for level in &["trace", "debug", "info", "warn", "error", "off"] {
            assert!(validate_file_key("log_level", level).is_ok());
        }
        assert!(validate_file_key("log_level", "verbose").is_err());
    }

    #[test]
    fn test_validate_file_key_server_port() {
        assert!(validate_file_key("server_port", "8080").is_ok());
        assert!(validate_file_key("server_port", "1").is_ok());
        assert!(validate_file_key("server_port", "65535").is_ok());
        assert!(validate_file_key("server_port", "0").is_err());
        assert!(validate_file_key("server_port", "99999").is_err());
    }

    #[test]
    fn test_validate_file_key_embedding_dimensions() {
        assert!(validate_file_key("embedding_dimensions", "512").is_ok());
        assert!(validate_file_key("embedding_dimensions", "0").is_err());
        assert!(validate_file_key("embedding_dimensions", "4097").is_err());
    }

    #[test]
    fn test_validate_file_key_llm_base_url() {
        assert!(validate_file_key("llm_base_url", "http://localhost:11434/v1").is_ok());
        assert!(validate_file_key("llm_base_url", "https://api.openai.com/v1").is_ok());
        assert!(validate_file_key("llm_base_url", "").is_err());
        assert!(validate_file_key("llm_base_url", "  ").is_err());
        assert!(validate_file_key("llm_base_url", "file:///etc/passwd").is_err());
        assert!(validate_file_key("llm_base_url", "gopher://evil.com").is_err());
        assert!(validate_file_key("llm_base_url", "not-a-url").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn test_check_path_permissions_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("test");
        std::fs::write(&file, "test").unwrap();
        let (exists, mode) = check_path_permissions(&file).unwrap();
        assert!(exists);
        assert!(mode > 0);
    }

    #[test]
    fn test_check_path_permissions_missing() {
        let (exists, _) = check_path_permissions(Path::new("/nonexistent/path")).unwrap();
        assert!(!exists);
    }
}

//! Shared validation helpers for CLI diagnostics and config management.

use std::path::{Path, PathBuf};

/// Result of API key format validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiKeyFormat {
    /// Standard Anthropic API key (sk-ant-*)
    ApiKey,
    /// OAuth subscription token (sk-ant-oat*)
    OAuthToken,
}

/// Validate the format of an Anthropic API key.
///
/// Returns the detected format, or an error if the key is empty or has
/// an unrecognized prefix.
pub fn validate_api_key_format(key: &str) -> Result<ApiKeyFormat, String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("API key is empty".to_string());
    }
    if crate::claude::is_oauth_token(key) {
        Ok(ApiKeyFormat::OAuthToken)
    } else if key.starts_with("sk-ant-") {
        Ok(ApiKeyFormat::ApiKey)
    } else {
        Err(format!(
            "Unrecognized API key format (expected sk-ant-* prefix, got {}...)",
            &key[..key.len().min(10)]
        ))
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
        "claude_model" | "embedding_model" => {
            if value.trim().is_empty() {
                return Err(format!("{key} cannot be empty"));
            }
        }
        "claude_max_tokens" => {
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
    fn test_validate_api_key_format_bad_prefix() {
        let err = validate_api_key_format("bad-key-here").unwrap_err();
        assert!(err.contains("Unrecognized"));
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
    fn test_validate_file_key_claude_model() {
        assert!(validate_file_key("claude_model", "claude-sonnet-4-6").is_ok());
        assert!(validate_file_key("claude_model", "").is_err());
        assert!(validate_file_key("claude_model", "  ").is_err());
    }

    #[test]
    fn test_validate_file_key_max_tokens() {
        assert!(validate_file_key("claude_max_tokens", "4096").is_ok());
        assert!(validate_file_key("claude_max_tokens", "1").is_ok());
        assert!(validate_file_key("claude_max_tokens", "131072").is_ok());
        assert!(validate_file_key("claude_max_tokens", "0").is_err());
        assert!(validate_file_key("claude_max_tokens", "131073").is_err());
        assert!(validate_file_key("claude_max_tokens", "abc").is_err());
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

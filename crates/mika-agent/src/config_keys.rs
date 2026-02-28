//! Customer configuration key allowlist and per-key validation.
//!
//! Shared between the CLI `/config set` handler and the agent's
//! `set_config` tool so both enforce the same rules.

/// Config keys that may be set via agent tools or `/config set`.
pub const SETTABLE_CONFIG_KEYS: &[&str] = &["chat_id", "timezone", "thinking_level"];

/// Validate a config value for the given key.
///
/// Returns `Ok(())` if the value is valid, or `Err(message)` describing what
/// is wrong with the value.
pub fn validate_config_value(key: &str, value: &str) -> Result<(), String> {
    if value.len() > 1000 {
        return Err("Config value too long (max 1000 characters)".to_string());
    }

    match key {
        "chat_id" => {
            if value.parse::<i64>().is_err() {
                return Err(format!(
                    "Invalid chat_id: {value}\nchat_id must be a numeric Telegram chat ID"
                ));
            }
        }
        "timezone" => {
            if value.parse::<chrono_tz::Tz>().is_err() {
                return Err(format!(
                    "Invalid timezone: {value}\nExample: Asia/Singapore, America/New_York, Europe/London"
                ));
            }
        }
        "thinking_level" => {
            if !matches!(value, "low" | "medium" | "med" | "high" | "off") {
                return Err(format!(
                    "Invalid thinking_level: {value}\nAllowed values: low, medium, med, high, off"
                ));
            }
        }
        _ => {}
    }

    Ok(())
}

/// Check whether a key is in the settable allowlist.
pub fn is_settable_key(key: &str) -> bool {
    SETTABLE_CONFIG_KEYS.contains(&key)
}

/// Format the allowlist as a comma-separated string (for error messages).
pub fn settable_keys_display() -> String {
    SETTABLE_CONFIG_KEYS.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allowlist_contains_expected_keys() {
        assert!(is_settable_key("chat_id"));
        assert!(is_settable_key("timezone"));
        assert!(is_settable_key("thinking_level"));
        assert!(!is_settable_key("api_key"));
        assert!(!is_settable_key("db_path"));
    }

    #[test]
    fn test_validate_chat_id_valid() {
        assert!(validate_config_value("chat_id", "123456789").is_ok());
        assert!(validate_config_value("chat_id", "-987654321").is_ok());
    }

    #[test]
    fn test_validate_chat_id_invalid() {
        let err = validate_config_value("chat_id", "not-a-number").unwrap_err();
        assert!(err.contains("Invalid chat_id"));
    }

    #[test]
    fn test_validate_timezone_valid() {
        assert!(validate_config_value("timezone", "Asia/Singapore").is_ok());
        assert!(validate_config_value("timezone", "America/New_York").is_ok());
        assert!(validate_config_value("timezone", "Europe/London").is_ok());
        assert!(validate_config_value("timezone", "UTC").is_ok());
    }

    #[test]
    fn test_validate_timezone_invalid() {
        let err = validate_config_value("timezone", "Not/A/Timezone").unwrap_err();
        assert!(err.contains("Invalid timezone"));
    }

    #[test]
    fn test_validate_value_too_long() {
        let long = "x".repeat(1001);
        let err = validate_config_value("chat_id", &long).unwrap_err();
        assert!(err.contains("too long"));
    }

    #[test]
    fn test_validate_thinking_level_valid() {
        assert!(validate_config_value("thinking_level", "low").is_ok());
        assert!(validate_config_value("thinking_level", "medium").is_ok());
        assert!(validate_config_value("thinking_level", "high").is_ok());
        assert!(validate_config_value("thinking_level", "off").is_ok());
    }

    #[test]
    fn test_validate_thinking_level_invalid() {
        let err = validate_config_value("thinking_level", "max").unwrap_err();
        assert!(err.contains("Invalid thinking_level"));
    }

    #[test]
    fn test_settable_keys_display() {
        let display = settable_keys_display();
        assert!(display.contains("chat_id"));
        assert!(display.contains("timezone"));
        assert!(display.contains("thinking_level"));
    }
}

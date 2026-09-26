use std::path::Path;

use regex::Regex;

use crate::error::MikaOsError;

/// Regex pattern matching secret-shaped env var names.
const SECRET_KEY_PATTERN: &str =
    r"(?i)^[A-Z_]*(?:API_KEY|TOKEN|SECRET|PRIVATE_KEY|PASSWORD|CREDENTIAL)s?$";

/// Placeholder value for redacted secrets.
const REDACTED_VALUE: &str = "REDACTED_BY_MIKA_FORK";

/// Redact secret-shaped values in a `.env` file.
///
/// Matches lines like `KEY=value` where KEY matches the secret pattern.
/// Preserves key names and structure; replaces only values.
pub fn redact_env_file(path: &Path) -> Result<(), MikaOsError> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        MikaOsError::RedactionFailed(format!("failed to read {}: {e}", path.display()))
    })?;

    let key_re = Regex::new(SECRET_KEY_PATTERN).expect("valid regex");
    let mut output = String::with_capacity(content.len());

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            output.push_str(line);
            output.push('\n');
            continue;
        }

        if let Some((key, _value)) = trimmed.split_once('=') {
            let key_name = key.trim();
            if key_re.is_match(key_name) {
                output.push_str(&format!("{key_name}={REDACTED_VALUE}\n"));
            } else {
                output.push_str(line);
                output.push('\n');
            }
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }

    std::fs::write(path, output).map_err(|e| {
        MikaOsError::RedactionFailed(format!("failed to write {}: {e}", path.display()))
    })?;

    Ok(())
}

/// Redact oauth.json by overwriting with an empty object.
pub fn redact_oauth_json(path: &Path) -> Result<(), MikaOsError> {
    if !path.exists() {
        return Ok(());
    }
    std::fs::write(path, "{}").map_err(|e| {
        MikaOsError::RedactionFailed(format!("failed to redact {}: {e}", path.display()))
    })?;
    Ok(())
}

/// Redact secret-shaped values in config.toml.
///
/// Parses with the `toml` crate, walks all string values, and redacts
/// those whose key matches the secret pattern.
pub fn redact_config_toml(path: &Path) -> Result<(), MikaOsError> {
    if !path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(path).map_err(|e| {
        MikaOsError::RedactionFailed(format!("failed to read {}: {e}", path.display()))
    })?;

    let mut doc: toml::Value = toml::from_str(&content).map_err(|e| {
        MikaOsError::RedactionFailed(format!("failed to parse {}: {e}", path.display()))
    })?;

    let key_re = Regex::new(SECRET_KEY_PATTERN).expect("valid regex");
    redact_toml_value(&mut doc, &key_re);

    let output = toml::to_string_pretty(&doc).map_err(|e| {
        MikaOsError::RedactionFailed(format!("failed to serialize {}: {e}", path.display()))
    })?;

    std::fs::write(path, output).map_err(|e| {
        MikaOsError::RedactionFailed(format!("failed to write {}: {e}", path.display()))
    })?;

    Ok(())
}

fn redact_toml_value(value: &mut toml::Value, key_re: &Regex) {
    match value {
        toml::Value::Table(table) => {
            for (key, val) in table.iter_mut() {
                if key_re.is_match(key) {
                    if val.is_str() {
                        *val = toml::Value::String(REDACTED_VALUE.to_string());
                    }
                } else {
                    redact_toml_value(val, key_re);
                }
            }
        }
        toml::Value::Array(arr) => {
            for val in arr.iter_mut() {
                redact_toml_value(val, key_re);
            }
        }
        _ => {}
    }
}

/// Redact secret-shaped values in the SQLite database.
///
/// Currently targets the `settings` table if it exists,
/// redacting values whose key matches the secret pattern.
pub fn redact_db_secrets(db_path: &Path, _agent_id: &str) -> Result<(), MikaOsError> {
    if !db_path.exists() {
        return Ok(());
    }

    let conn = rusqlite::Connection::open(db_path)?;

    // Check if settings table exists
    let has_settings: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='settings'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);

    if !has_settings {
        return Ok(());
    }

    let key_re = Regex::new(SECRET_KEY_PATTERN).expect("valid regex");

    let mut stmt = conn.prepare("SELECT key FROM settings")?;
    let keys: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .filter(|k| key_re.is_match(k))
        .collect();

    for key in &keys {
        conn.execute(
            "UPDATE settings SET value = ?1 WHERE key = ?2",
            rusqlite::params![REDACTED_VALUE, key],
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_pattern_matches_expected_keys() {
        let re = Regex::new(SECRET_KEY_PATTERN).unwrap();
        assert!(re.is_match("MIKA_ANTHROPIC_API_KEY"));
        assert!(re.is_match("MIKA_GITHUB_TOKEN"));
        assert!(re.is_match("MIKA_INTERNAL_TOKEN"));
        assert!(re.is_match("GH_TOKEN"));
        assert!(re.is_match("MIKA_GITHUB_APP_PRIVATE_KEY"));
        assert!(re.is_match("SOME_SECRET"));
        assert!(re.is_match("MY_PASSWORD"));
        assert!(re.is_match("AWS_CREDENTIALS"));

        // Should NOT match
        assert!(!re.is_match("MIKA_HOME"));
        assert!(!re.is_match("MIKA_LOG_FORMAT"));
        assert!(!re.is_match("PATH"));
        assert!(!re.is_match("MIKA_DEV_MODE"));
    }

    #[test]
    fn redact_env_file_preserves_structure() {
        let dir = tempfile::tempdir().unwrap();
        let env_file = dir.path().join(".env");
        std::fs::write(
            &env_file,
            "# Comment\nMIKA_HOME=/home/mika\nMIKA_ANTHROPIC_API_KEY=sk-ant-123\nMIKA_LOG_FORMAT=json\nMIKA_GITHUB_TOKEN=ghp_abc\n",
        )
        .unwrap();

        redact_env_file(&env_file).unwrap();
        let result = std::fs::read_to_string(&env_file).unwrap();

        assert!(result.contains("# Comment"));
        assert!(result.contains("MIKA_HOME=/home/mika"));
        assert!(result.contains(&format!("MIKA_ANTHROPIC_API_KEY={REDACTED_VALUE}")));
        assert!(result.contains("MIKA_LOG_FORMAT=json"));
        assert!(result.contains(&format!("MIKA_GITHUB_TOKEN={REDACTED_VALUE}")));
        assert!(!result.contains("sk-ant-123"));
        assert!(!result.contains("ghp_abc"));
    }

    #[test]
    fn redact_oauth_json_writes_empty() {
        let dir = tempfile::tempdir().unwrap();
        let oauth_file = dir.path().join("oauth.json");
        std::fs::write(&oauth_file, r#"{"token": "secret"}"#).unwrap();

        redact_oauth_json(&oauth_file).unwrap();
        let result = std::fs::read_to_string(&oauth_file).unwrap();
        assert_eq!(result, "{}");
    }

    #[test]
    fn redact_oauth_json_noop_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let oauth_file = dir.path().join("oauth.json");
        // Should not error
        redact_oauth_json(&oauth_file).unwrap();
    }

    #[test]
    fn redact_config_toml_replaces_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let config_file = dir.path().join("config.toml");
        std::fs::write(
            &config_file,
            "log_format = \"json\"\napi_key = \"sk-secret\"\n\n[provider]\ntoken = \"ghp_abc\"\nmodel = \"claude-sonnet\"\n",
        )
        .unwrap();

        redact_config_toml(&config_file).unwrap();
        let result = std::fs::read_to_string(&config_file).unwrap();

        assert!(result.contains("log_format"));
        assert!(result.contains(&format!("api_key = \"{REDACTED_VALUE}\"")));
        assert!(result.contains(&format!("token = \"{REDACTED_VALUE}\"")));
        assert!(result.contains("model = \"claude-sonnet\""));
        assert!(!result.contains("sk-secret"));
        assert!(!result.contains("ghp_abc"));
    }
}

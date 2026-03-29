use anyhow::{Context, Result};
use std::path::Path;
use tracing::{debug, warn};

/// Load `.env` from `{home_dir}/.env` into process environment variables.
///
/// Uses `dotenvy::from_path()` which does NOT override existing env vars —
/// shell-set `MIKA_*` variables always win. Silently skips if the file is missing.
pub fn load_dotenv(home_dir: &Path) {
    let env_path = home_dir.join(".env");
    match dotenvy::from_path(&env_path) {
        Ok(()) => debug!(path = %env_path.display(), "loaded .env"),
        Err(dotenvy::Error::Io(ref e)) if e.kind() == std::io::ErrorKind::NotFound => {
            // Expected — most users won't have a .env file initially
        }
        Err(e) => {
            // Log but don't fail — env vars or config files may still provide values
            warn!(path = %env_path.display(), error = %e, "failed to load .env");
        }
    }
}

/// Check for deprecated environment variables and warn the user.
///
/// Call this after `load_dotenv()` so `.env` values are already in the process environment.
/// Currently checks for `MIKA_LLM_API_KEY` which was superseded by per-provider keys
/// (e.g., `MIKA_ANTHROPIC_API_KEY`).
pub fn check_deprecated_env_vars() {
    if std::env::var("MIKA_LLM_API_KEY").is_ok() {
        warn!(
            "MIKA_LLM_API_KEY is deprecated and ignored by the config system. \
             Rename it to MIKA_ANTHROPIC_API_KEY in your ~/.mika/.env file."
        );
    }
}

/// Write or update a key in `{home_dir}/.env`. Creates the file if it doesn't exist.
/// Sets file permissions to 0600 on Unix (secrets file).
pub fn set_env_var(home_dir: &Path, key: &str, value: &str) -> Result<()> {
    // Validate key: non-empty, ASCII alphanumeric or underscore
    if key.is_empty() || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        anyhow::bail!("invalid .env key: must be non-empty ASCII alphanumeric/underscore");
    }
    // Reject newlines in both key and value to prevent injection
    if key.contains('\n') || key.contains('\r') || value.contains('\n') || value.contains('\r') {
        anyhow::bail!(".env key and value must not contain newline characters");
    }

    let env_path = home_dir.join(".env");
    let mut lines: Vec<String> = Vec::new();
    let mut found = false;

    if let Ok(content) = std::fs::read_to_string(&env_path) {
        for line in content.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty()
                && !trimmed.starts_with('#')
                && let Some((k, _)) = trimmed.split_once('=')
                && k.trim() == key
            {
                let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
                lines.push(format!("{key}=\"{escaped}\""));
                found = true;
                continue;
            }
            lines.push(line.to_string());
        }
    }

    if !found {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        lines.push(format!("{key}=\"{escaped}\""));
    }

    // Ensure trailing newline
    let content = lines.join("\n") + "\n";

    // Atomic write: write to temp file, then rename
    let tmp_path = env_path.with_file_name(".env.tmp");
    std::fs::write(&tmp_path, &content)
        .with_context(|| format!("failed to write {}", tmp_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))
        {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e.into());
        }
    }

    std::fs::rename(&tmp_path, &env_path).with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp_path.display(),
            env_path.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn test_load_dotenv_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        // Should not panic or error
        load_dotenv(tmp.path());
    }

    #[test]
    #[serial]
    fn test_load_dotenv_loads_vars() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".env"),
            "MIKA_TEST_DOTENV_LOAD=hello_world\n",
        )
        .unwrap();

        // Ensure the var doesn't exist before
        unsafe { std::env::remove_var("MIKA_TEST_DOTENV_LOAD") };

        load_dotenv(tmp.path());

        assert_eq!(
            std::env::var("MIKA_TEST_DOTENV_LOAD").ok(),
            Some("hello_world".to_string())
        );

        // Cleanup
        unsafe { std::env::remove_var("MIKA_TEST_DOTENV_LOAD") };
    }

    #[test]
    #[serial]
    fn test_load_dotenv_does_not_override() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".env"),
            "MIKA_TEST_DOTENV_NO_OVERRIDE=from_file\n",
        )
        .unwrap();

        // Set env var before loading
        unsafe { std::env::set_var("MIKA_TEST_DOTENV_NO_OVERRIDE", "from_shell") };

        load_dotenv(tmp.path());

        // Shell value should win
        assert_eq!(
            std::env::var("MIKA_TEST_DOTENV_NO_OVERRIDE").ok(),
            Some("from_shell".to_string())
        );

        // Cleanup
        unsafe { std::env::remove_var("MIKA_TEST_DOTENV_NO_OVERRIDE") };
    }

    #[test]
    fn test_set_env_var_creates_file() {
        let tmp = tempfile::tempdir().unwrap();
        set_env_var(tmp.path(), "MY_KEY", "my_value").unwrap();

        let content = std::fs::read_to_string(tmp.path().join(".env")).unwrap();
        assert!(content.contains("MY_KEY=\"my_value\""));
    }

    #[test]
    fn test_set_env_var_updates_existing() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".env"), "# Comment\nFOO=bar\nBAZ=qux\n").unwrap();

        set_env_var(tmp.path(), "FOO", "updated").unwrap();

        let content = std::fs::read_to_string(tmp.path().join(".env")).unwrap();
        assert!(content.contains("FOO=\"updated\""));
        assert!(content.contains("BAZ=qux"));
        assert!(content.contains("# Comment"));
        // Should not have duplicate FOO
        assert_eq!(content.matches("FOO=").count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn test_set_env_var_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        set_env_var(tmp.path(), "SECRET", "s3cret").unwrap();

        let perms = std::fs::metadata(tmp.path().join(".env"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(perms, 0o600);
    }

    #[test]
    #[serial]
    fn test_set_env_var_roundtrip_special_chars() {
        let tmp = tempfile::tempdir().unwrap();
        let key = "MIKA_TEST_ROUNDTRIP_SPECIAL";
        // Value with hash, equals, quotes, spaces
        let value = "val#ue=with \"quotes\" and spaces";
        set_env_var(tmp.path(), key, value).unwrap();

        unsafe { std::env::remove_var(key) };
        load_dotenv(tmp.path());
        assert_eq!(std::env::var(key).ok().as_deref(), Some(value));

        // Cleanup
        unsafe { std::env::remove_var(key) };
    }

    #[test]
    #[serial]
    fn test_set_env_var_roundtrip_backslash() {
        let tmp = tempfile::tempdir().unwrap();
        let key = "MIKA_TEST_ROUNDTRIP_BSLASH";
        let value = r"C:\Users\test";
        set_env_var(tmp.path(), key, value).unwrap();

        unsafe { std::env::remove_var(key) };
        load_dotenv(tmp.path());
        assert_eq!(std::env::var(key).ok().as_deref(), Some(value));

        // Cleanup
        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn test_set_env_var_rejects_newlines() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(set_env_var(tmp.path(), "KEY", "value\nINJECTED=bad").is_err());
        assert!(set_env_var(tmp.path(), "KEY\nBAD", "value").is_err());
        assert!(set_env_var(tmp.path(), "KEY", "value\rINJECTED=bad").is_err());
    }

    #[test]
    fn test_set_env_var_rejects_invalid_key() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(set_env_var(tmp.path(), "", "value").is_err());
        assert!(set_env_var(tmp.path(), "has space", "value").is_err());
        assert!(set_env_var(tmp.path(), "has=equals", "value").is_err());
        assert!(set_env_var(tmp.path(), "has-dash", "value").is_err());
    }
}

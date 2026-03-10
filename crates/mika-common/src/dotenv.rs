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

/// Read a single key from `{home_dir}/.env` without loading into process env.
/// Uses dotenvy's parser for consistent behavior with `load_dotenv`.
pub fn get_env_var(home_dir: &Path, key: &str) -> Option<String> {
    let env_path = home_dir.join(".env");
    dotenvy::from_path_iter(&env_path).ok()?.find_map(|r| {
        let (k, v) = r.ok()?;
        (k == key).then_some(v)
    })
}

/// Write or update a key in `{home_dir}/.env`. Creates the file if it doesn't exist.
/// Sets file permissions to 0600 on Unix (secrets file).
pub fn set_env_var(home_dir: &Path, key: &str, value: &str) -> Result<()> {
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
                lines.push(format!("{key}={value}"));
                found = true;
                continue;
            }
            lines.push(line.to_string());
        }
    }

    if !found {
        lines.push(format!("{key}={value}"));
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
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))?;
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
        assert!(content.contains("MY_KEY=my_value"));
    }

    #[test]
    fn test_set_env_var_updates_existing() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".env"), "# Comment\nFOO=bar\nBAZ=qux\n").unwrap();

        set_env_var(tmp.path(), "FOO", "updated").unwrap();

        let content = std::fs::read_to_string(tmp.path().join(".env")).unwrap();
        assert!(content.contains("FOO=updated"));
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
    fn test_get_env_var_reads_key() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".env"),
            "# Comment\nKEY_A=value_a\nKEY_B=\"quoted_value\"\nKEY_C='single_quoted'\n",
        )
        .unwrap();

        assert_eq!(
            get_env_var(tmp.path(), "KEY_A"),
            Some("value_a".to_string())
        );
        assert_eq!(
            get_env_var(tmp.path(), "KEY_B"),
            Some("quoted_value".to_string())
        );
        assert_eq!(
            get_env_var(tmp.path(), "KEY_C"),
            Some("single_quoted".to_string())
        );
        assert_eq!(get_env_var(tmp.path(), "MISSING"), None);
    }

    #[test]
    fn test_get_env_var_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(get_env_var(tmp.path(), "ANY_KEY"), None);
    }
}

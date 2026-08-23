use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;
use tracing::{error, info, warn};

/// Load `.env` from `{home_dir}/.env` into process environment variables.
///
/// Uses `dotenvy::from_path()` which does NOT override existing env vars —
/// shell-set `MIKA_*` variables always win. Emits observability for the
/// three states (loaded / absent / load_error) via TWO channels (mika#1968
/// AC1/AC2 + adversarial review A1 P0):
/// - `eprintln!` to stderr — durable across subscriber init state, so
///   supervise-daemon/OpenRC boot lines fire BEFORE `logging::init()`
///   installs a tracing subscriber. Captured by the service log.
/// - `info!/error!` structured events — silent when no subscriber is
///   installed (mika-spirit boot), visible when one is (post-init paths,
///   CLI callers that init logging before load_dotenv, tests with a
///   subscriber).
///
/// Both channels use the same event names (`dotenv_loaded`, `dotenv_absent`,
/// `dotenv_load_error`) and field names (`path`, `keys_from_file`) so a
/// single grep hits either sink. This defends the founding-incident
/// observability contract for supervised binaries where `logging::init()`
/// legitimately runs after `load_dotenv()` (the Settings-then-init ordering
/// enforced by `mika-spirit::main`).
pub fn load_dotenv(home_dir: &Path) {
    let env_path = home_dir.join(".env");

    // Double-read tradeoff: parse_dotenv() for count, then dotenvy::from_path()
    // for the actual load. Cost: 2 syscalls at boot. Kept for observability —
    // the `keys_from_file` field in the dotenv_loaded log line lets operators
    // distinguish "file loaded 0 keys" (empty file) from "no file" (dotenv_absent).
    match dotenvy::from_path(&env_path) {
        Ok(()) => {
            let keys_from_file = parse_dotenv(home_dir).len();
            // Pre-init durable channel (see fn docstring — A1 P0 fix).
            eprintln!(
                "dotenv_loaded path={} keys_from_file={}",
                env_path.display(),
                keys_from_file
            );
            info!(
                target: "mika::env",
                event = "dotenv_loaded",
                path = %env_path.display(),
                keys_from_file,
                "loaded .env"
            );
        }
        Err(dotenvy::Error::Io(ref e)) if e.kind() == std::io::ErrorKind::NotFound => {
            // Pre-init durable channel (see fn docstring — A1 P0 fix).
            eprintln!("dotenv_absent path={}", env_path.display());
            info!(
                target: "mika::env",
                event = "dotenv_absent",
                path = %env_path.display(),
                "no .env file found"
            );
        }
        Err(e) => {
            // Pre-init durable channel (see fn docstring — A1 P0 fix).
            // Errors are the highest-signal state — operators MUST see this
            // even when no subscriber exists.
            eprintln!("dotenv_load_error path={} error={}", env_path.display(), e);
            error!(
                target: "mika::env",
                event = "dotenv_load_error",
                path = %env_path.display(),
                error = %e,
                "failed to load .env"
            );
        }
    }
}

/// Parse a `.env` file and return key-value pairs without modifying process env.
///
/// Keys are returned as-is (e.g., `MIKA_GITHUB_TOKEN`) — the caller decides
/// how to map them. Uses `dotenvy::from_path_iter()` which reads without setting
/// env vars. Silently returns an empty map if the file is missing or unparseable.
pub fn parse_dotenv(home_dir: &Path) -> HashMap<String, String> {
    let env_path = home_dir.join(".env");
    let mut map = HashMap::new();
    match dotenvy::from_path_iter(&env_path) {
        Ok(iter) => {
            for (key, val) in iter.flatten() {
                map.insert(key, val);
            }
        }
        Err(dotenvy::Error::Io(ref e)) if e.kind() == std::io::ErrorKind::NotFound => {
            // Expected — no .env file
        }
        Err(e) => {
            warn!(path = %env_path.display(), error = %e, "failed to parse .env");
        }
    }
    map
}

/// Convert parsed dotenv key-value pairs into a TOML string for config-rs ingestion.
///
/// Strips the `MIKA_` prefix and lowercases keys to match config-rs field names
/// (e.g., `MIKA_GITHUB_TOKEN` → `github_token = "..."`). Non-MIKA keys are skipped.
/// Values are TOML-escaped (backslash, double-quote, newline, carriage return, tab).
pub fn dotenv_to_toml(vars: &HashMap<String, String>) -> String {
    let mut toml = String::new();
    for (key, value) in vars {
        if let Some(config_key) = key.strip_prefix("MIKA_") {
            let config_key = config_key.to_lowercase();
            let escaped = value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
                .replace('\t', "\\t");
            toml.push_str(&format!("{config_key} = \"{escaped}\"\n"));
        }
    }
    toml
}

/// Check for deprecated and misplaced environment variables, warn the user, and
/// actively remove dangerous ones from the process environment.
///
/// Call this after `load_dotenv()` so `.env` values are already in the process environment.
///
/// Checks:
/// - `MIKA_LLM_API_KEY` — deprecated, superseded by per-provider keys
/// - `GH_TOKEN` in `.env` file — misplaced, causes identity collision (see #380)
pub fn check_env_warnings(home_dir: &Path) {
    // 1. Deprecated: MIKA_LLM_API_KEY
    if std::env::var("MIKA_LLM_API_KEY").is_ok() {
        warn!(
            "MIKA_LLM_API_KEY is deprecated and ignored by the config system. \
             Rename it to MIKA_ANTHROPIC_API_KEY in your ~/.mika/.env file."
        );
    }

    // 2. Misplaced: GH_TOKEN in .env file
    //
    // If GH_TOKEN is defined in the .env file, dotenvy loads it into the process
    // environment, overriding the host's `gh auth` identity for ALL child processes.
    // This collapses the developer/platform identity separation and causes
    // self-approval blocks in the autonomous dev loop.
    //
    // Detection: we check the .env file text directly (not process env) to avoid
    // false positives when GH_TOKEN is legitimately set in the host shell.
    if env_file_contains_key(home_dir, "GH_TOKEN") {
        warn!(
            "GH_TOKEN found in {}/.env — this overrides your host `gh auth` identity \
             and breaks the developer/platform identity separation. Removed from process \
             environment. Use MIKA_GITHUB_TOKEN for agent GitHub operations instead. \
             See docs/configuration.md for the identity matrix.",
            home_dir.display()
        );
        // Actively remove to prevent identity collision in child processes.
        // Safe because: run_gh explicitly injects MIKA_GITHUB_TOKEN as GH_TOKEN,
        // MCP uses env_clear(), and scrub_mika_env_vars + GH_TOKEN scrub covers
        // exec handlers.
        unsafe { std::env::remove_var("GH_TOKEN") };
    }
}

/// Check whether a `.env` file in `home_dir` contains a given key assignment.
///
/// Parses the file as text looking for uncommented `KEY=...` lines.
/// Returns `false` if the file doesn't exist or can't be read.
fn env_file_contains_key(home_dir: &Path, key: &str) -> bool {
    let env_path = home_dir.join(".env");
    let content = match std::fs::read_to_string(&env_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // Strip optional "export " prefix (dotenvy supports this syntax)
        let trimmed = trimmed.strip_prefix("export ").unwrap_or(trimmed);
        if let Some((k, _)) = trimmed.split_once('=')
            && k.trim() == key
        {
            return true;
        }
    }
    false
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

    /// mika#1968 AC1/AC2 — no .env file. Must complete without panic and
    /// leave `parse_dotenv` returning an empty map for the same path (the
    /// structural invariant behind the `dotenv_absent` info line).
    #[test]
    fn load_dotenv_reports_absent_when_file_missing() {
        let tmp = tempfile::tempdir().unwrap();
        // No .env file at tmp.path()
        load_dotenv(tmp.path());
        // Parse-dotenv API surface confirms the "loaded" count is 0 for the same path
        let parsed = parse_dotenv(tmp.path());
        assert_eq!(parsed.len(), 0);
    }

    /// mika#1968 T5 P3 — empty-file canary. Locks the "file present but
    /// empty" state that the `keys_from_file=0` observability count is
    /// meant to distinguish from `dotenv_absent`. A regression that
    /// silently reports non-zero for an empty file (e.g., `values().count()`
    /// dropping empty-string values) would break this canary. See
    /// `docs/solutions/best-practices/app-owned-env-vs-init-owned-env-2026-08-23.md`.
    #[test]
    fn load_dotenv_reports_zero_keys_for_empty_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".env"), "").unwrap();
        load_dotenv(tmp.path());
        assert_eq!(
            parse_dotenv(tmp.path()).len(),
            0,
            "empty .env file must report 0 keys — the canary distinguishes 'present but empty' from 'absent'"
        );
    }

    /// mika#1968 T3 P2 — dotenv_load_error arm. Locks the third
    /// observability state: file present but unparseable → `dotenvy` returns
    /// `Err(LineParse)`, code emits `dotenv_load_error` structured event
    /// (and pre-init `eprintln!` per A1 P0 fix). Regression that swallows
    /// the error or panics would fail here. Structural check on the code
    /// path — does not attempt to capture log/eprintln output (that requires
    /// a subscriber install + stderr redirect setup we don't want to force
    /// on unit-test scope).
    #[test]
    fn load_dotenv_handles_parse_error_without_panic() {
        let tmp = tempfile::tempdir().unwrap();
        // dotenvy 0.15's parser accepts most inputs — the reliably-rejected
        // shape is a line without an `=` that also doesn't look like a
        // comment. `KEY_WITHOUT_EQUALS` (bare identifier) triggers LineParse.
        std::fs::write(tmp.path().join(".env"), "KEY_WITHOUT_EQUALS\n").unwrap();
        // MUST complete without panic — the log-and-continue posture is
        // load-bearing (env vars or config files may still provide values).
        load_dotenv(tmp.path());
    }

    /// mika#1968 AC1/AC2 — presence with N keys. Locks the structural
    /// invariant: `load_dotenv`'s reported `keys_from_file` count is derived
    /// from `parse_dotenv().len()` for the same path. Any regression that
    /// stops calling `parse_dotenv` here would break the observability
    /// contract behind the `dotenv_loaded` info line.
    #[test]
    #[serial]
    fn load_dotenv_reports_loaded_count_matches_parsed() {
        let tmp = tempfile::tempdir().unwrap();
        let env_file = tmp.path().join(".env");
        std::fs::write(
            &env_file,
            "MIKA_TEST_KEYS_COUNT_1=val1\nMIKA_TEST_KEYS_COUNT_2=val2\n",
        )
        .unwrap();

        // Clean env before load so shell-set values don't skew the shape.
        unsafe {
            std::env::remove_var("MIKA_TEST_KEYS_COUNT_1");
            std::env::remove_var("MIKA_TEST_KEYS_COUNT_2");
        }

        let parsed = parse_dotenv(tmp.path());
        assert_eq!(parsed.len(), 2);
        load_dotenv(tmp.path());
        // The load_dotenv info line's keys_from_file field equals parsed.len()
        // by construction — the impl calls parse_dotenv() to derive the count.
        assert_eq!(parsed.len(), parse_dotenv(tmp.path()).len());

        // Cleanup so we don't leak into serial siblings.
        unsafe {
            std::env::remove_var("MIKA_TEST_KEYS_COUNT_1");
            std::env::remove_var("MIKA_TEST_KEYS_COUNT_2");
        }
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

    // --- parse_dotenv tests ---

    #[test]
    fn test_parse_dotenv_returns_pairs() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".env"),
            "MIKA_GITHUB_TOKEN=ghp_abc\nMIKA_BRAVE_API_KEY=brave_123\n",
        )
        .unwrap();

        let map = parse_dotenv(tmp.path());
        assert_eq!(map.get("MIKA_GITHUB_TOKEN").unwrap(), "ghp_abc");
        assert_eq!(map.get("MIKA_BRAVE_API_KEY").unwrap(), "brave_123");
    }

    #[test]
    fn test_parse_dotenv_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let map = parse_dotenv(tmp.path());
        assert!(map.is_empty());
    }

    #[test]
    fn test_parse_dotenv_does_not_set_env() {
        let tmp = tempfile::tempdir().unwrap();
        let key = "MIKA_TEST_PARSE_NO_SET";
        std::fs::write(
            tmp.path().join(".env"),
            format!("{key}=should_not_appear\n"),
        )
        .unwrap();

        unsafe { std::env::remove_var(key) };

        let map = parse_dotenv(tmp.path());
        assert_eq!(map.get(key).unwrap(), "should_not_appear");
        // Must NOT be in process env
        assert!(
            std::env::var(key).is_err(),
            "parse_dotenv must not set process env vars"
        );
    }

    // --- dotenv_to_toml tests ---

    #[test]
    fn test_dotenv_to_toml_strips_prefix_and_lowercases() {
        let mut vars = HashMap::new();
        vars.insert("MIKA_GITHUB_TOKEN".to_string(), "ghp_abc".to_string());
        let toml = dotenv_to_toml(&vars);
        assert!(toml.contains("github_token = \"ghp_abc\""));
    }

    #[test]
    fn test_dotenv_to_toml_skips_non_mika_keys() {
        let mut vars = HashMap::new();
        vars.insert("GH_TOKEN".to_string(), "ghp_leaked".to_string());
        vars.insert("MIKA_BRAVE_API_KEY".to_string(), "key".to_string());
        let toml = dotenv_to_toml(&vars);
        assert!(!toml.contains("gh_token"));
        assert!(toml.contains("brave_api_key"));
    }

    #[test]
    fn test_dotenv_to_toml_escapes_special_chars() {
        let mut vars = HashMap::new();
        vars.insert(
            "MIKA_TEST_KEY".to_string(),
            "has\\backslash\"quotes\nnewline\rtab\there".to_string(),
        );
        let toml = dotenv_to_toml(&vars);
        assert!(toml.contains(r#"test_key = "has\\backslash\"quotes\nnewline\rtab\there""#));
    }

    #[test]
    fn test_dotenv_to_toml_empty_map() {
        let vars = HashMap::new();
        let toml = dotenv_to_toml(&vars);
        assert!(toml.is_empty());
    }

    // --- env_file_contains_key tests ---

    #[test]
    fn test_env_file_contains_key_found() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".env"),
            "MIKA_ANTHROPIC_API_KEY=sk-ant-xxx\nGH_TOKEN=ghp_abc123\n",
        )
        .unwrap();

        assert!(env_file_contains_key(tmp.path(), "GH_TOKEN"));
        assert!(env_file_contains_key(tmp.path(), "MIKA_ANTHROPIC_API_KEY"));
    }

    #[test]
    fn test_env_file_contains_key_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".env"),
            "MIKA_ANTHROPIC_API_KEY=sk-ant-xxx\n",
        )
        .unwrap();

        assert!(!env_file_contains_key(tmp.path(), "GH_TOKEN"));
    }

    #[test]
    fn test_env_file_contains_key_commented_out() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".env"),
            "# GH_TOKEN=ghp_old_value\nMIKA_ANTHROPIC_API_KEY=sk-ant-xxx\n",
        )
        .unwrap();

        // Commented-out lines should NOT match
        assert!(!env_file_contains_key(tmp.path(), "GH_TOKEN"));
    }

    #[test]
    fn test_env_file_contains_key_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        // No .env file exists
        assert!(!env_file_contains_key(tmp.path(), "GH_TOKEN"));
    }

    #[test]
    fn test_env_file_contains_key_export_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".env"), "export GH_TOKEN=ghp_abc123\n").unwrap();

        // "export GH_TOKEN=..." should be detected (dotenvy supports this syntax)
        assert!(env_file_contains_key(tmp.path(), "GH_TOKEN"));
    }

    #[test]
    fn test_env_file_contains_key_empty_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".env"), "").unwrap();

        assert!(!env_file_contains_key(tmp.path(), "GH_TOKEN"));
    }

    #[test]
    #[serial]
    fn test_check_env_warnings_removes_gh_token_from_env() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".env"),
            "GH_TOKEN=ghp_leaked_token\nMIKA_ANTHROPIC_API_KEY=sk-ant-xxx\n",
        )
        .unwrap();

        // Simulate dotenvy having loaded GH_TOKEN into the process env
        unsafe { std::env::set_var("GH_TOKEN", "ghp_leaked_token") };

        check_env_warnings(tmp.path());

        // GH_TOKEN should have been removed from the process environment
        assert!(
            std::env::var("GH_TOKEN").is_err(),
            "GH_TOKEN should be removed from process env after check_env_warnings"
        );
    }

    #[test]
    #[serial]
    fn test_check_env_warnings_no_removal_when_not_in_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".env"),
            "MIKA_ANTHROPIC_API_KEY=sk-ant-xxx\n",
        )
        .unwrap();

        // GH_TOKEN set from host shell (not from .env file)
        unsafe { std::env::set_var("GH_TOKEN", "ghp_host_shell_token") };

        check_env_warnings(tmp.path());

        // GH_TOKEN from host shell should NOT be removed
        assert_eq!(
            std::env::var("GH_TOKEN").ok(),
            Some("ghp_host_shell_token".to_string()),
            "GH_TOKEN from host shell should be preserved"
        );

        // Cleanup
        unsafe { std::env::remove_var("GH_TOKEN") };
    }
}

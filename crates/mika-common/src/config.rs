use config::{Config, Environment, File};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Clone)]
pub struct Settings {
    /// Anthropic API key
    pub anthropic_api_key: String,

    /// Claude model ID (default: claude-sonnet-4-6)
    #[serde(default = "default_claude_model")]
    pub claude_model: String,

    /// Max tokens for Claude responses
    #[serde(default = "default_max_tokens")]
    pub claude_max_tokens: u32,

    /// SQLite database path
    #[serde(default = "default_db_path")]
    pub db_path: PathBuf,

    /// Log level
    #[serde(default = "default_log_level")]
    pub log_level: String,

    /// Routing layer URL (for outbound messages from agent container)
    #[serde(default)]
    pub routing_url: Option<String>,

    /// Customer ID (set per container)
    #[serde(default)]
    pub customer_id: Option<String>,

    /// HTTP server port (default: 8080, only used in server mode)
    #[serde(default = "default_server_port")]
    pub server_port: u16,

    /// Internal bearer token for gateway ↔ container auth
    #[serde(default)]
    pub internal_token: Option<String>,

    /// Resolved home directory path (populated after load, not from config file)
    #[serde(skip)]
    pub home_dir: PathBuf,
}

fn default_claude_model() -> String {
    "claude-sonnet-4-6".to_string()
}

fn default_max_tokens() -> u32 {
    4096
}

fn default_db_path() -> PathBuf {
    PathBuf::from("mika.db")
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_server_port() -> u16 {
    8080
}

impl Settings {
    /// Load settings from config files + environment variables.
    ///
    /// Config cascade (lowest to highest priority):
    ///   1. config/default.toml  (bundled defaults)
    ///   2. config/local.toml    (gitignored local overrides)
    ///   3. ~/.mika/config.toml  (user home directory config)
    ///   4. MIKA_* env vars      (highest priority)
    ///
    /// The `home_dir` argument is the resolved Mika home directory.
    /// If `db_path` is not explicitly set, it defaults to `{home_dir}/data/mika.db`.
    pub fn load(home_dir: &Path) -> anyhow::Result<Self> {
        let home_config = home_dir.join("config.toml");

        let mut settings: Settings = Config::builder()
            .add_source(File::with_name("config/default").required(false))
            .add_source(File::with_name("config/local").required(false))
            .add_source(File::from(home_config).required(false))
            .add_source(
                Environment::with_prefix("MIKA")
                    .prefix_separator("_")
                    .separator("__"),
            )
            .build()?
            .try_deserialize()?;

        settings.home_dir = home_dir.to_path_buf();

        // If db_path is still the default "mika.db", resolve it to ~/.mika/data/mika.db
        if settings.db_path == Path::new("mika.db") {
            settings.db_path = home_dir.join("data").join("mika.db");
        }

        Ok(settings)
    }
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("anthropic_api_key", &"[REDACTED]")
            .field("claude_model", &self.claude_model)
            .field("claude_max_tokens", &self.claude_max_tokens)
            .field("db_path", &self.db_path)
            .field("log_level", &self.log_level)
            .field("routing_url", &self.routing_url)
            .field("customer_id", &self.customer_id)
            .field("server_port", &self.server_port)
            .field(
                "internal_token",
                &self.internal_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("home_dir", &self.home_dir)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// Set required env vars and clear optional ones to ensure clean state.
    fn clean_env() {
        // Safety: tests set env vars; no production thread reads these.
        unsafe {
            std::env::set_var("MIKA_ANTHROPIC_API_KEY", "test-key");
            std::env::remove_var("MIKA_CLAUDE_MODEL");
            std::env::remove_var("MIKA_DB_PATH");
        }
    }

    #[test]
    #[serial]
    fn test_defaults() {
        clean_env();

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();
        assert_eq!(settings.claude_model, "claude-sonnet-4-6");
        assert_eq!(settings.claude_max_tokens, 4096);
        assert_eq!(settings.log_level, "info");
        // db_path should resolve to home_dir/data/mika.db
        let expected_db = tmp.path().join("data").join("mika.db");
        assert_eq!(settings.db_path, expected_db);
        assert_eq!(settings.home_dir, tmp.path());
    }

    #[test]
    #[serial]
    fn test_home_config_loaded() {
        clean_env();

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            "claude_model = \"claude-opus-4-6\"\nlog_level = \"debug\"\n",
        )
        .unwrap();

        let settings = Settings::load(tmp.path()).unwrap();
        assert_eq!(settings.claude_model, "claude-opus-4-6");
        assert_eq!(settings.log_level, "debug");
    }

    #[test]
    #[serial]
    fn test_env_overrides_home_config() {
        clean_env();
        // Safety: test-only env var.
        unsafe { std::env::set_var("MIKA_CLAUDE_MODEL", "claude-haiku-4-5") };

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            "claude_model = \"claude-opus-4-6\"\n",
        )
        .unwrap();

        let settings = Settings::load(tmp.path()).unwrap();
        // Env var should win over home config
        assert_eq!(settings.claude_model, "claude-haiku-4-5");

        unsafe { std::env::remove_var("MIKA_CLAUDE_MODEL") };
    }

    #[test]
    #[serial]
    fn test_explicit_db_path_not_overridden() {
        clean_env();
        // Safety: test-only env var.
        unsafe { std::env::set_var("MIKA_DB_PATH", "/custom/path.db") };

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();
        assert_eq!(settings.db_path, PathBuf::from("/custom/path.db"));

        unsafe { std::env::remove_var("MIKA_DB_PATH") };
    }
}

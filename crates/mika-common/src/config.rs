use config::{Config, Environment, File};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Clone)]
pub struct Settings {
    /// Anthropic API key (optional; only required for commands that call the Claude API)
    #[serde(default)]
    pub anthropic_api_key: Option<String>,

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
    pub internal_token: Option<SecretString>,

    /// OpenAI API key for embeddings (optional; enables Layer 3 vector search)
    #[serde(default)]
    pub openai_api_key: Option<String>,

    /// Embedding model ID (default: text-embedding-3-small)
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,

    /// Embedding dimensions (default: 512)
    #[serde(default = "default_embedding_dimensions")]
    pub embedding_dimensions: u32,

    /// Brave Search API key (optional; enables web_search builtin skill)
    #[serde(default)]
    pub brave_api_key: Option<String>,

    /// Optional log file path for mika-server (maps to MIKA_SERVER_LOG_FILE)
    #[serde(default)]
    pub server_log_file: Option<PathBuf>,

    /// Disable bundled skill re-sync on startup (default: false)
    #[serde(default)]
    pub disable_bundled_skills: bool,

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

fn default_embedding_model() -> String {
    "text-embedding-3-small".to_string()
}

fn default_embedding_dimensions() -> u32 {
    512
}

impl Settings {
    /// Create an EmbeddingClient if OpenAI API key is configured.
    pub fn make_embedding_client(&self) -> Option<crate::embedding::EmbeddingClient> {
        self.openai_api_key
            .as_ref()
            .filter(|k| !k.trim().is_empty())
            .and_then(|key| {
                crate::embedding::EmbeddingClient::new(
                    key.clone(),
                    self.embedding_model.clone(),
                    self.embedding_dimensions,
                )
                .ok()
            })
    }

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
    /// Load settings from config files + environment variables.
    ///
    /// Backward-compatible wrapper: uses `home_dir` as both global and agent home.
    pub fn load(home_dir: &Path) -> anyhow::Result<Self> {
        Self::load_for_agent(home_dir, home_dir)
    }

    /// Load settings with multi-agent config cascade.
    ///
    /// Config cascade (lowest to highest priority):
    ///   1. config/default.toml  (bundled defaults)
    ///   2. config/local.toml    (gitignored local overrides)
    ///   3. `{global_home}/config.toml`  (shared settings)
    ///   4. `{agent_home}/config.toml`   (per-agent overrides)
    ///   5. MIKA_* env vars              (highest priority)
    ///
    /// `agent_home` is the resolved directory for the specific agent.
    /// `db_path` defaults to `{agent_home}/data/mika.db` if not explicitly set.
    pub fn load_for_agent(global_home: &Path, agent_home: &Path) -> anyhow::Result<Self> {
        let global_config = global_home.join("config.toml");
        let agent_config = agent_home.join("config.toml");

        let mut builder = Config::builder()
            .add_source(File::with_name("config/default").required(false))
            .add_source(File::with_name("config/local").required(false))
            .add_source(File::from(global_config).required(false));

        // Only add agent config if it's different from global config
        if global_home != agent_home {
            builder = builder.add_source(File::from(agent_config).required(false));
        }

        let mut settings: Settings = builder
            .add_source(
                Environment::with_prefix("MIKA")
                    .prefix_separator("_")
                    .separator("__"),
            )
            .build()?
            .try_deserialize()?;

        settings.home_dir = agent_home.to_path_buf();

        // If db_path is still the default "mika.db", resolve it to {agent_home}/data/mika.db
        if settings.db_path == Path::new("mika.db") {
            settings.db_path = agent_home.join("data").join("mika.db");
        }

        // Validate internal_token format if present (fixed-length eliminates timing leak)
        if let Some(ref token) = settings.internal_token {
            let val = token.expose_secret();
            if val.len() != 64 || !val.bytes().all(|b| b.is_ascii_hexdigit()) {
                anyhow::bail!(
                    "MIKA_INTERNAL_TOKEN must be exactly 64 hex characters (32 bytes hex-encoded)"
                );
            }
        }

        Ok(settings)
    }
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field(
                "anthropic_api_key",
                &self.anthropic_api_key.as_ref().map(|_| "[REDACTED]"),
            )
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
            .field(
                "openai_api_key",
                &self.openai_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("embedding_model", &self.embedding_model)
            .field("embedding_dimensions", &self.embedding_dimensions)
            .field(
                "brave_api_key",
                &self.brave_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("server_log_file", &self.server_log_file)
            .field("disable_bundled_skills", &self.disable_bundled_skills)
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
            std::env::remove_var("MIKA_DISABLE_BUNDLED_SKILLS");
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
        assert!(!settings.disable_bundled_skills);
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

    #[test]
    #[serial]
    fn test_load_for_agent_cascade() {
        clean_env();

        let tmp = tempfile::tempdir().unwrap();
        let global_home = tmp.path().join("global");
        let agent_home = tmp.path().join("agent");
        std::fs::create_dir_all(&global_home).unwrap();
        std::fs::create_dir_all(&agent_home).unwrap();

        // Global config sets model
        std::fs::write(
            global_home.join("config.toml"),
            "claude_model = \"claude-opus-4-6\"\nlog_level = \"debug\"\n",
        )
        .unwrap();

        // Agent config overrides model but not log_level
        std::fs::write(
            agent_home.join("config.toml"),
            "claude_model = \"claude-haiku-4-5\"\n",
        )
        .unwrap();

        let settings = Settings::load_for_agent(&global_home, &agent_home).unwrap();
        // Agent config should override global config
        assert_eq!(settings.claude_model, "claude-haiku-4-5");
        // log_level should come from global config
        assert_eq!(settings.log_level, "debug");
        // home_dir should be agent_home
        assert_eq!(settings.home_dir, agent_home);
        // db_path should resolve to agent_home/data/mika.db
        assert_eq!(settings.db_path, agent_home.join("data").join("mika.db"));
    }

    #[test]
    #[serial]
    fn test_load_for_agent_same_as_load_when_same_path() {
        clean_env();

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            "claude_model = \"claude-opus-4-6\"\n",
        )
        .unwrap();

        let via_load = Settings::load(tmp.path()).unwrap();
        let via_load_for_agent = Settings::load_for_agent(tmp.path(), tmp.path()).unwrap();
        assert_eq!(via_load.claude_model, via_load_for_agent.claude_model);
        assert_eq!(via_load.home_dir, via_load_for_agent.home_dir);
    }

    #[test]
    #[serial]
    fn test_disable_bundled_skills_from_env() {
        clean_env();
        // Safety: test-only env var.
        unsafe { std::env::set_var("MIKA_DISABLE_BUNDLED_SKILLS", "true") };

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();
        assert!(settings.disable_bundled_skills);

        unsafe { std::env::remove_var("MIKA_DISABLE_BUNDLED_SKILLS") };
    }
}

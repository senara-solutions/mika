use std::sync::Arc;

use config::{Config, Environment, File};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use std::path::{Path, PathBuf};

// -- Config Key Registry --

/// Storage backend for a configuration key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigBackend {
    /// config.toml (per-agent or global)
    File,
    /// ~/.mika/.env (secrets)
    Env,
    /// customer_config DB table
    Database,
    /// Computed at runtime, not writable
    ReadOnly,
}

/// Metadata for a single configuration key.
#[derive(Debug, Clone)]
pub struct ConfigKeyInfo {
    pub key: &'static str,
    pub backend: ConfigBackend,
    /// The MIKA_* env var that can override this key (if any).
    pub env_var: Option<&'static str>,
    /// Whether this key contains a secret and should be redacted.
    pub secret: bool,
    pub description: &'static str,
}

/// All known configuration keys across all backends.
pub static CONFIG_KEYS: &[ConfigKeyInfo] = &[
    // File backend (config.toml)
    ConfigKeyInfo {
        key: "llm_model",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_LLM_MODEL"),
        secret: false,
        description: "LLM model ID (supports provider/model prefix)",
    },
    ConfigKeyInfo {
        key: "llm_max_tokens",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_LLM_MAX_TOKENS"),
        secret: false,
        description: "Max response tokens",
    },
    ConfigKeyInfo {
        key: "log_level",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_LOG_LEVEL"),
        secret: false,
        description: "Log level (trace/debug/info/warn/error/off)",
    },
    ConfigKeyInfo {
        key: "log_format",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_LOG_FORMAT"),
        secret: false,
        description: "Stdout log format for mika-server and mika-gateway (json or pretty). CLI always uses pretty.",
    },
    ConfigKeyInfo {
        key: "server_port",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_SERVER_PORT"),
        secret: false,
        description: "HTTP server port",
    },
    ConfigKeyInfo {
        key: "embedding_model",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_EMBEDDING_MODEL"),
        secret: false,
        description: "OpenAI embedding model",
    },
    ConfigKeyInfo {
        key: "embedding_dimensions",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_EMBEDDING_DIMENSIONS"),
        secret: false,
        description: "Embedding vector dimensions",
    },
    ConfigKeyInfo {
        key: "llm_base_url",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_LLM_BASE_URL"),
        secret: false,
        description: "LLM provider base URL override",
    },
    ConfigKeyInfo {
        key: "dashboard_enabled",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_DASHBOARD_ENABLED"),
        secret: false,
        description: "Enable embedded dashboard SPA at /dashboard/ (default: false)",
    },
    // Env backend (.env secrets)
    ConfigKeyInfo {
        key: "openai_api_key",
        backend: ConfigBackend::Env,
        env_var: Some("MIKA_OPENAI_API_KEY"),
        secret: true,
        description: "OpenAI API key (for embeddings)",
    },
    ConfigKeyInfo {
        key: "llm_api_key",
        backend: ConfigBackend::Env,
        env_var: Some("MIKA_LLM_API_KEY"),
        secret: true,
        description: "LLM API key",
    },
    ConfigKeyInfo {
        key: "brave_api_key",
        backend: ConfigBackend::Env,
        env_var: Some("MIKA_BRAVE_API_KEY"),
        secret: true,
        description: "Brave Search API key",
    },
    ConfigKeyInfo {
        key: "internal_token",
        backend: ConfigBackend::Env,
        env_var: Some("MIKA_INTERNAL_TOKEN"),
        secret: true,
        description: "Server internal auth token",
    },
    ConfigKeyInfo {
        key: "investigate_github_token",
        backend: ConfigBackend::Env,
        env_var: Some("MIKA_INVESTIGATE_GITHUB_TOKEN"),
        secret: true,
        description: "GitHub token for investigation panel issue creation",
    },
    ConfigKeyInfo {
        key: "github_repo",
        backend: ConfigBackend::Env,
        env_var: Some("MIKA_GITHUB_REPO"),
        secret: false,
        description: "GitHub repo (owner/repo) for issue creation",
    },
    // Database backend (customer_config table)
    ConfigKeyInfo {
        key: "timezone",
        backend: ConfigBackend::Database,
        env_var: None,
        secret: false,
        description: "User timezone",
    },
    ConfigKeyInfo {
        key: "thinking_level",
        backend: ConfigBackend::Database,
        env_var: None,
        secret: false,
        description: "Claude thinking level (low/medium/high/off)",
    },
    // ReadOnly (runtime-computed)
    ConfigKeyInfo {
        key: "home_dir",
        backend: ConfigBackend::ReadOnly,
        env_var: Some("MIKA_HOME"),
        secret: false,
        description: "Mika home directory",
    },
    ConfigKeyInfo {
        key: "db_path",
        backend: ConfigBackend::ReadOnly,
        env_var: None,
        secret: false,
        description: "Database file path",
    },
];

/// Look up a config key by name.
pub fn lookup_config_key(key: &str) -> Option<&'static ConfigKeyInfo> {
    CONFIG_KEYS.iter().find(|k| k.key == key)
}

/// Get the effective value of a config key from a loaded Settings struct.
/// For DB keys, returns None (caller must query the database).
pub fn get_effective_value(key: &str, settings: &Settings) -> Option<String> {
    match key {
        "llm_model" => Some(settings.llm_model.clone()),
        "llm_max_tokens" => Some(settings.llm_max_tokens.to_string()),
        "log_level" => Some(settings.log_level.clone()),
        "log_format" => Some(settings.log_format.clone()),
        "server_port" => Some(settings.server_port.to_string()),
        "embedding_model" => Some(settings.embedding_model.clone()),
        "embedding_dimensions" => Some(settings.embedding_dimensions.to_string()),
        "llm_base_url" => settings.llm_base_url.clone(),
        "llm_api_key" => settings.llm_api_key.clone(),
        "openai_api_key" => settings.openai_api_key.clone(),
        "brave_api_key" => settings.brave_api_key.clone(),
        "dashboard_enabled" => Some(settings.dashboard_enabled.to_string()),

        "investigate_github_token" => settings.investigate_github_token.clone(),
        "github_repo" => settings.github_repo.clone(),
        "internal_token" => settings
            .internal_token
            .as_ref()
            .map(|s| s.expose_secret().to_string()),
        "home_dir" => Some(settings.home_dir.display().to_string()),
        "db_path" => Some(settings.db_path.display().to_string()),
        // DB keys (timezone, thinking_level) not available from Settings
        _ => None,
    }
}

#[derive(Deserialize, Clone)]
pub struct Settings {
    /// LLM model ID (default: claude-sonnet-4-6). Supports provider prefix:
    /// `openai/gpt-4o`, `ollama/llama3`, `groq/llama-3.1-70b`.
    /// No prefix defaults to Anthropic.
    #[serde(default = "default_llm_model")]
    pub llm_model: String,

    /// Max tokens for LLM responses
    #[serde(default = "default_max_tokens")]
    pub llm_max_tokens: u32,

    /// SQLite database path
    #[serde(default = "default_db_path")]
    pub db_path: PathBuf,

    /// Log level
    #[serde(default = "default_log_level")]
    pub log_level: String,

    /// Stdout log format for server/gateway: "json" (default) or "pretty"
    #[serde(default = "default_log_format")]
    pub log_format: String,

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

    /// Separate bearer token for read-only dashboard API routes (env: MIKA_DASHBOARD_TOKEN).
    /// If unset, dashboard routes accept `internal_token` for backwards compatibility.
    /// This token only grants access to `/api/v1/*` routes — mutation endpoints still
    /// require `internal_token`.
    #[serde(default)]
    pub dashboard_token: Option<SecretString>,

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

    /// GitHub Personal Access Token for investigation panel issue creation (optional)
    #[serde(default)]
    pub investigate_github_token: Option<String>,

    /// GitHub repository in owner/repo format for issue creation (optional)
    #[serde(default)]
    pub github_repo: Option<String>,

    /// LLM provider base URL override (for OpenAI-compatible providers)
    #[serde(default)]
    pub llm_base_url: Option<String>,

    /// LLM API key (required for any command that calls an LLM).
    /// Supports Anthropic API keys, OAuth tokens, and third-party provider keys.
    #[serde(default)]
    pub llm_api_key: Option<String>,

    /// Enable embedded dashboard SPA at /dashboard/ (default: false)
    #[serde(default)]
    pub dashboard_enabled: bool,

    /// Disable bundled skill re-sync on startup (default: false)
    #[serde(default)]
    pub disable_bundled_skills: bool,

    /// Enable OpenTelemetry trace export (default: false, requires "telemetry" feature)
    #[serde(default)]
    pub telemetry_enabled: bool,

    /// OTLP endpoint URL (e.g. "https://cloud.langfuse.com/api/public/otel")
    #[serde(default)]
    pub otlp_endpoint: Option<String>,

    /// OTLP authorization header value (e.g. Base64-encoded "public:secret" for Langfuse)
    #[serde(default)]
    pub otlp_auth_header: Option<SecretString>,

    /// Resolved home directory path (populated after load, not from config file)
    #[serde(skip)]
    pub home_dir: PathBuf,
}

fn default_llm_model() -> String {
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

fn default_log_format() -> String {
    "json".to_string()
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
    /// Create an LLM provider from the current settings.
    ///
    /// Parses `llm_model` as a model spec (e.g. `anthropic/claude-sonnet-4-6`,
    /// `openai/gpt-4o`, or just `claude-sonnet-4-6` which defaults to Anthropic).
    /// Uses `llm_api_key` for all providers.
    pub fn make_llm_provider(&self) -> anyhow::Result<Arc<dyn crate::llm::LlmProvider>> {
        let spec = crate::llm::ModelSpec::parse(&self.llm_model)?;

        let spec = spec
            .with_base_url(self.llm_base_url.clone())
            .with_api_key(self.llm_api_key.clone());

        crate::llm::create_provider(&spec, self.llm_max_tokens)
    }

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
    ///   1. Rust `Default` / serde defaults  (compiled-in)
    ///   2. `~/.mika/config.toml`            (user config)
    ///   3. `~/.mika/.env`                   (secrets, loaded by caller before this)
    ///   4. `MIKA_*` env vars                (highest priority)
    ///
    /// Backward-compatible wrapper: uses `home_dir` as both global and agent home.
    pub fn load(home_dir: &Path) -> anyhow::Result<Self> {
        Self::load_for_agent(home_dir, home_dir)
    }

    /// Load settings with multi-agent config cascade.
    ///
    /// Config cascade (lowest to highest priority):
    ///   1. Rust `Default` / serde defaults        (compiled-in)
    ///   2. `{global_home}/config.toml`             (shared settings)
    ///   3. `{agent_home}/config.toml`              (per-agent overrides)
    ///   4. `~/.mika/.env`                          (secrets, loaded by caller)
    ///   5. MIKA_* env vars                         (highest priority)
    ///
    /// `agent_home` is the resolved directory for the specific agent.
    /// `db_path` defaults to `{global_home}/data/mika.db` (single container DB).
    pub fn load_for_agent(global_home: &Path, agent_home: &Path) -> anyhow::Result<Self> {
        let global_config = global_home.join("config.toml");
        let agent_config = agent_home.join("config.toml");

        let mut builder = Config::builder().add_source(File::from(global_config).required(false));

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

        // If db_path is still the default "mika.db", resolve it to {global_home}/data/mika.db
        // (single unified database per container — see brainstorm)
        if settings.db_path == Path::new("mika.db") {
            settings.db_path = global_home.join("data").join("mika.db");
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
            .field("llm_model", &self.llm_model)
            .field("llm_max_tokens", &self.llm_max_tokens)
            .field("db_path", &self.db_path)
            .field("log_level", &self.log_level)
            .field("log_format", &self.log_format)
            .field("routing_url", &self.routing_url)
            .field("customer_id", &self.customer_id)
            .field("server_port", &self.server_port)
            .field(
                "internal_token",
                &self.internal_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "dashboard_token",
                &self.dashboard_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "openai_api_key",
                &self.openai_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("embedding_model", &self.embedding_model)
            .field("embedding_dimensions", &self.embedding_dimensions)
            .field("llm_base_url", &self.llm_base_url)
            .field(
                "llm_api_key",
                &self.llm_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "brave_api_key",
                &self.brave_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "investigate_github_token",
                &self.investigate_github_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("github_repo", &self.github_repo)
            .field("server_log_file", &self.server_log_file)
            .field("dashboard_enabled", &self.dashboard_enabled)
            .field("disable_bundled_skills", &self.disable_bundled_skills)
            .field("telemetry_enabled", &self.telemetry_enabled)
            .field("otlp_endpoint", &self.otlp_endpoint)
            .field(
                "otlp_auth_header",
                &self.otlp_auth_header.as_ref().map(|_| "[REDACTED]"),
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
            std::env::remove_var("MIKA_LLM_MODEL");
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
        assert_eq!(settings.llm_model, "claude-sonnet-4-6");
        assert_eq!(settings.llm_max_tokens, 4096);
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
            "llm_model = \"claude-opus-4-6\"\nlog_level = \"debug\"\n",
        )
        .unwrap();

        let settings = Settings::load(tmp.path()).unwrap();
        assert_eq!(settings.llm_model, "claude-opus-4-6");
        assert_eq!(settings.log_level, "debug");
    }

    #[test]
    #[serial]
    fn test_env_overrides_home_config() {
        clean_env();
        // Safety: test-only env var.
        unsafe { std::env::set_var("MIKA_LLM_MODEL", "claude-haiku-4-5") };

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            "llm_model = \"claude-opus-4-6\"\n",
        )
        .unwrap();

        let settings = Settings::load(tmp.path()).unwrap();
        // Env var should win over home config
        assert_eq!(settings.llm_model, "claude-haiku-4-5");

        unsafe { std::env::remove_var("MIKA_LLM_MODEL") };
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
            "llm_model = \"claude-opus-4-6\"\nlog_level = \"debug\"\n",
        )
        .unwrap();

        // Agent config overrides model but not log_level
        std::fs::write(
            agent_home.join("config.toml"),
            "llm_model = \"claude-haiku-4-5\"\n",
        )
        .unwrap();

        let settings = Settings::load_for_agent(&global_home, &agent_home).unwrap();
        // Agent config should override global config
        assert_eq!(settings.llm_model, "claude-haiku-4-5");
        // log_level should come from global config
        assert_eq!(settings.log_level, "debug");
        // home_dir should be agent_home
        assert_eq!(settings.home_dir, agent_home);
        // db_path should resolve to global_home/data/mika.db (single container DB)
        assert_eq!(settings.db_path, global_home.join("data").join("mika.db"));
    }

    #[test]
    #[serial]
    fn test_load_for_agent_same_as_load_when_same_path() {
        clean_env();

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            "llm_model = \"claude-opus-4-6\"\n",
        )
        .unwrap();

        let via_load = Settings::load(tmp.path()).unwrap();
        let via_load_for_agent = Settings::load_for_agent(tmp.path(), tmp.path()).unwrap();
        assert_eq!(via_load.llm_model, via_load_for_agent.llm_model);
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

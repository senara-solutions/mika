use std::sync::Arc;

use config::{Config, Environment, File, FileFormat};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::llm::ProviderKind;

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
    // -- Active provider selection --
    ConfigKeyInfo {
        key: "llm_provider",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_LLM_PROVIDER"),
        secret: false,
        description: "Active LLM provider (anthropic, openai, openrouter, groq, ollama, mistral, google, deepseek)",
    },
    // -- Per-provider: Anthropic --
    ConfigKeyInfo {
        key: "anthropic_model",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_ANTHROPIC_MODEL"),
        secret: false,
        description: "Anthropic model ID",
    },
    ConfigKeyInfo {
        key: "anthropic_api_key",
        backend: ConfigBackend::Env,
        env_var: Some("MIKA_ANTHROPIC_API_KEY"),
        secret: true,
        description: "Anthropic API key",
    },
    ConfigKeyInfo {
        key: "anthropic_base_url",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_ANTHROPIC_BASE_URL"),
        secret: false,
        description: "Anthropic base URL override",
    },
    // -- Per-provider: OpenAI --
    ConfigKeyInfo {
        key: "openai_model",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_OPENAI_MODEL"),
        secret: false,
        description: "OpenAI model ID",
    },
    ConfigKeyInfo {
        key: "openai_base_url",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_OPENAI_BASE_URL"),
        secret: false,
        description: "OpenAI base URL override",
    },
    // Note: openai_api_key is listed separately below (legacy, also used for embeddings)
    // -- Per-provider: OpenRouter --
    ConfigKeyInfo {
        key: "openrouter_model",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_OPENROUTER_MODEL"),
        secret: false,
        description: "OpenRouter model ID",
    },
    ConfigKeyInfo {
        key: "openrouter_api_key",
        backend: ConfigBackend::Env,
        env_var: Some("MIKA_OPENROUTER_API_KEY"),
        secret: true,
        description: "OpenRouter API key",
    },
    ConfigKeyInfo {
        key: "openrouter_base_url",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_OPENROUTER_BASE_URL"),
        secret: false,
        description: "OpenRouter base URL override",
    },
    // -- Per-provider: Groq --
    ConfigKeyInfo {
        key: "groq_model",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_GROQ_MODEL"),
        secret: false,
        description: "Groq model ID",
    },
    ConfigKeyInfo {
        key: "groq_api_key",
        backend: ConfigBackend::Env,
        env_var: Some("MIKA_GROQ_API_KEY"),
        secret: true,
        description: "Groq API key",
    },
    ConfigKeyInfo {
        key: "groq_base_url",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_GROQ_BASE_URL"),
        secret: false,
        description: "Groq base URL override",
    },
    // -- Per-provider: Ollama --
    ConfigKeyInfo {
        key: "ollama_model",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_OLLAMA_MODEL"),
        secret: false,
        description: "Ollama model ID",
    },
    ConfigKeyInfo {
        key: "ollama_api_key",
        backend: ConfigBackend::Env,
        env_var: Some("MIKA_OLLAMA_API_KEY"),
        secret: true,
        description: "Ollama API key (usually not needed)",
    },
    ConfigKeyInfo {
        key: "ollama_base_url",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_OLLAMA_BASE_URL"),
        secret: false,
        description: "Ollama base URL override",
    },
    // -- Per-provider: Mistral --
    ConfigKeyInfo {
        key: "mistral_model",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_MISTRAL_MODEL"),
        secret: false,
        description: "Mistral model ID",
    },
    ConfigKeyInfo {
        key: "mistral_api_key",
        backend: ConfigBackend::Env,
        env_var: Some("MIKA_MISTRAL_API_KEY"),
        secret: true,
        description: "Mistral API key",
    },
    ConfigKeyInfo {
        key: "mistral_base_url",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_MISTRAL_BASE_URL"),
        secret: false,
        description: "Mistral base URL override",
    },
    // -- Per-provider: Google --
    ConfigKeyInfo {
        key: "google_model",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_GOOGLE_MODEL"),
        secret: false,
        description: "Google AI model ID",
    },
    ConfigKeyInfo {
        key: "google_api_key",
        backend: ConfigBackend::Env,
        env_var: Some("MIKA_GOOGLE_API_KEY"),
        secret: true,
        description: "Google AI API key",
    },
    ConfigKeyInfo {
        key: "google_base_url",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_GOOGLE_BASE_URL"),
        secret: false,
        description: "Google AI base URL override",
    },
    // -- Per-provider: DeepSeek --
    ConfigKeyInfo {
        key: "deepseek_model",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_DEEPSEEK_MODEL"),
        secret: false,
        description: "DeepSeek model ID",
    },
    ConfigKeyInfo {
        key: "deepseek_api_key",
        backend: ConfigBackend::Env,
        env_var: Some("MIKA_DEEPSEEK_API_KEY"),
        secret: true,
        description: "DeepSeek API key",
    },
    ConfigKeyInfo {
        key: "deepseek_base_url",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_DEEPSEEK_BASE_URL"),
        secret: false,
        description: "DeepSeek base URL",
    },
    // MiniMax
    ConfigKeyInfo {
        key: "minimax_model",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_MINIMAX_MODEL"),
        secret: false,
        description: "MiniMax model name",
    },
    ConfigKeyInfo {
        key: "minimax_api_key",
        backend: ConfigBackend::Env,
        env_var: Some("MIKA_MINIMAX_API_KEY"),
        secret: true,
        description: "MiniMax API key",
    },
    ConfigKeyInfo {
        key: "minimax_base_url",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_MINIMAX_BASE_URL"),
        secret: false,
        description: "DeepSeek base URL override",
    },
    // -- Non-provider settings (File backend) --
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
        key: "dashboard_enabled",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_DASHBOARD_ENABLED"),
        secret: false,
        description: "Enable embedded dashboard SPA at /dashboard/ (default: false)",
    },
    // -- Observability --
    ConfigKeyInfo {
        key: "store_llm_calls",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_STORE_LLM_CALLS"),
        secret: false,
        description: "Store LLM call metadata in SQLite (default: true)",
    },
    ConfigKeyInfo {
        key: "store_tool_calls",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_STORE_TOOL_CALLS"),
        secret: false,
        description: "Store full tool call I/O in SQLite (default: true)",
    },
    ConfigKeyInfo {
        key: "log_llm_bodies",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_LOG_LLM_BODIES"),
        secret: false,
        description: "Log full LLM request/response bodies at debug level (default: false)",
    },
    // -- Env backend (.env secrets) --
    ConfigKeyInfo {
        key: "openai_api_key",
        backend: ConfigBackend::Env,
        env_var: Some("MIKA_OPENAI_API_KEY"),
        secret: true,
        description: "OpenAI API key (for embeddings + OpenAI LLM provider)",
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
        key: "github_token",
        backend: ConfigBackend::Env,
        env_var: Some("MIKA_GITHUB_TOKEN"),
        secret: true,
        description: "GitHub token for agent operations (context injection, task enrichment, PR merge)",
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
    // -- GitHub App --
    ConfigKeyInfo {
        key: "github_app_id",
        backend: ConfigBackend::Env,
        env_var: Some("MIKA_GITHUB_APP_ID"),
        secret: false,
        description: "GitHub App ID for mika-dev-bot",
    },
    ConfigKeyInfo {
        key: "github_app_private_key",
        backend: ConfigBackend::Env,
        env_var: Some("MIKA_GITHUB_APP_PRIVATE_KEY"),
        secret: true,
        description: "GitHub App private key (base64-encoded PEM)",
    },
    ConfigKeyInfo {
        key: "github_app_installation_id",
        backend: ConfigBackend::Env,
        env_var: Some("MIKA_GITHUB_APP_INSTALLATION_ID"),
        secret: false,
        description: "GitHub App installation ID for the org",
    },
    ConfigKeyInfo {
        key: "github_app_login",
        backend: ConfigBackend::Env,
        env_var: Some("MIKA_GITHUB_APP_LOGIN"),
        secret: false,
        description: "GitHub App bot login (e.g., mika-dev[bot])",
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
        "llm_provider" => Some(settings.llm_provider.to_string()),
        "llm_max_tokens" => Some(settings.llm_max_tokens.to_string()),
        "log_level" => Some(settings.log_level.clone()),
        "log_format" => Some(settings.log_format.clone()),
        "server_port" => Some(settings.server_port.to_string()),
        "embedding_model" => Some(settings.embedding_model.clone()),
        "embedding_dimensions" => Some(settings.embedding_dimensions.to_string()),
        "dashboard_enabled" => Some(settings.dashboard_enabled.to_string()),

        // Per-provider: Anthropic
        "anthropic_model" => settings.anthropic_model.clone(),
        "anthropic_api_key" => settings.anthropic_api_key.clone(),
        "anthropic_base_url" => settings.anthropic_base_url.clone(),
        // Per-provider: OpenAI (api_key shares with openai_api_key)
        "openai_model" => settings.openai_model.clone(),
        "openai_api_key" => settings.openai_api_key.clone(),
        "openai_base_url" => settings.openai_base_url.clone(),
        // Per-provider: OpenRouter
        "openrouter_model" => settings.openrouter_model.clone(),
        "openrouter_api_key" => settings.openrouter_api_key.clone(),
        "openrouter_base_url" => settings.openrouter_base_url.clone(),
        // Per-provider: Groq
        "groq_model" => settings.groq_model.clone(),
        "groq_api_key" => settings.groq_api_key.clone(),
        "groq_base_url" => settings.groq_base_url.clone(),
        // Per-provider: Ollama
        "ollama_model" => settings.ollama_model.clone(),
        "ollama_api_key" => settings.ollama_api_key.clone(),
        "ollama_base_url" => settings.ollama_base_url.clone(),
        // Per-provider: Mistral
        "mistral_model" => settings.mistral_model.clone(),
        "mistral_api_key" => settings.mistral_api_key.clone(),
        "mistral_base_url" => settings.mistral_base_url.clone(),
        // Per-provider: Google
        "google_model" => settings.google_model.clone(),
        "google_api_key" => settings.google_api_key.clone(),
        "google_base_url" => settings.google_base_url.clone(),
        // Per-provider: DeepSeek
        "deepseek_model" => settings.deepseek_model.clone(),
        "deepseek_api_key" => settings.deepseek_api_key.clone(),
        "minimax_api_key" => settings.minimax_api_key.clone(),
        "kimi_api_key" => settings.kimi_api_key.clone(),
        "qwen_api_key" => settings.qwen_api_key.clone(),
        "deepseek_base_url" => settings.deepseek_base_url.clone(),

        // Non-provider secrets/settings
        "brave_api_key" => settings.brave_api_key.clone(),
        "github_token" => settings.github_token.clone(),
        "investigate_github_token" => settings.investigate_github_token.clone(),
        "github_repo" => settings.github_repo.clone(),
        "github_app_id" => settings.github_app_id.map(|v| v.to_string()),
        "github_app_private_key" => settings
            .github_app_private_key
            .as_ref()
            .map(|_| "[SET]".to_string()),
        "github_app_installation_id" => settings.github_app_installation_id.map(|v| v.to_string()),
        "github_app_login" => settings.github_app_login.clone(),
        "internal_token" => settings
            .internal_token
            .as_ref()
            .map(|s| s.expose_secret().to_string()),
        "home_dir" => Some(settings.home_dir.display().to_string()),
        "db_path" => Some(settings.db_path.display().to_string()),
        // Observability
        "store_llm_calls" => Some(settings.store_llm_calls.to_string()),
        "store_tool_calls" => Some(settings.store_tool_calls.to_string()),
        "log_llm_bodies" => Some(settings.log_llm_bodies.to_string()),
        // DB keys (timezone, thinking_level) not available from Settings
        _ => None,
    }
}

#[derive(Deserialize, Clone)]
pub struct Settings {
    /// Active LLM provider. Each provider has its own model, api_key, and base_url fields.
    #[serde(default = "default_llm_provider")]
    pub llm_provider: ProviderKind,

    /// Max tokens for LLM responses
    #[serde(default = "default_max_tokens")]
    pub llm_max_tokens: u32,

    // -- Per-provider fields: Anthropic --
    #[serde(default)]
    pub anthropic_model: Option<String>,
    #[serde(default)]
    pub anthropic_api_key: Option<String>,
    #[serde(default)]
    pub anthropic_base_url: Option<String>,

    // -- Per-provider fields: OpenAI --
    #[serde(default)]
    pub openai_model: Option<String>,
    // openai_api_key is below (shared with legacy embedding key)
    #[serde(default)]
    pub openai_base_url: Option<String>,

    // -- Per-provider fields: OpenRouter --
    #[serde(default)]
    pub openrouter_model: Option<String>,
    #[serde(default)]
    pub openrouter_api_key: Option<String>,
    #[serde(default)]
    pub openrouter_base_url: Option<String>,

    // -- Per-provider fields: Groq --
    #[serde(default)]
    pub groq_model: Option<String>,
    #[serde(default)]
    pub groq_api_key: Option<String>,
    #[serde(default)]
    pub groq_base_url: Option<String>,

    // -- Per-provider fields: Ollama --
    #[serde(default)]
    pub ollama_model: Option<String>,
    #[serde(default)]
    pub ollama_api_key: Option<String>,
    #[serde(default)]
    pub ollama_base_url: Option<String>,

    // -- Per-provider fields: Mistral --
    #[serde(default)]
    pub mistral_model: Option<String>,
    #[serde(default)]
    pub mistral_api_key: Option<String>,
    #[serde(default)]
    pub mistral_base_url: Option<String>,

    // -- Per-provider fields: Google --
    #[serde(default)]
    pub google_model: Option<String>,
    #[serde(default)]
    pub google_api_key: Option<String>,
    #[serde(default)]
    pub google_base_url: Option<String>,

    // -- Per-provider fields: DeepSeek --
    #[serde(default)]
    pub deepseek_model: Option<String>,
    pub minimax_model: Option<String>,
    pub minimax_base_url: Option<String>,
    pub kimi_api_key: Option<String>,
    pub kimi_model: Option<String>,
    pub kimi_base_url: Option<String>,
    pub qwen_api_key: Option<String>,
    pub qwen_model: Option<String>,
    pub qwen_base_url: Option<String>,
    #[serde(default)]
    pub deepseek_api_key: Option<String>,
    pub minimax_api_key: Option<String>,
    #[serde(default)]
    pub deepseek_base_url: Option<String>,

    // -- Non-provider settings --
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
    #[serde(default)]
    pub dashboard_token: Option<SecretString>,

    /// OpenAI API key — used for embeddings AND as the OpenAI LLM provider's API key.
    /// Legacy field: MIKA_OPENAI_API_KEY env var.
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

    /// GitHub Personal Access Token for agent operations (context injection, task enrichment, PR merge).
    /// No fallback — `investigate_github_token` is used only by the investigation panel.
    #[serde(default)]
    pub github_token: Option<String>,

    /// GitHub Personal Access Token for investigation panel issue creation (optional)
    #[serde(default)]
    pub investigate_github_token: Option<String>,

    /// GitHub repository in owner/repo format for issue creation (optional)
    #[serde(default)]
    pub github_repo: Option<String>,

    /// GitHub App ID (optional; enables GitHub App authentication).
    #[serde(default)]
    pub github_app_id: Option<u64>,

    /// GitHub App private key, base64-encoded PEM (optional).
    /// Encode with: `base64 -w0 < your-app.pem`
    #[serde(default)]
    pub github_app_private_key: Option<SecretString>,

    /// GitHub App installation ID for the org (optional).
    #[serde(default)]
    pub github_app_installation_id: Option<u64>,

    /// GitHub App bot login (e.g., "mika-dev[bot]"). Used for assignee filtering
    /// in autonomous issue pickup. Derived from the App slug or set explicitly.
    #[serde(default)]
    pub github_app_login: Option<String>,

    /// Enable embedded dashboard SPA at /dashboard/ (default: false)
    #[serde(default)]
    pub dashboard_enabled: bool,

    /// Disable bundled skill re-sync on startup (default: false)
    #[serde(default)]
    pub disable_bundled_skills: bool,

    /// Enable dev mode — auto-provisions well-known development agents
    /// (mika-dev, mika-qa) with role-specific identity and skill assignments
    /// on startup (default: false)
    #[serde(default)]
    pub dev_mode: bool,

    /// Disable agent provisioning on startup (default: false).
    /// When true, prevents auto-creation and file overwrites for well-known agents,
    /// allowing manual edits to persist across restarts/deploys.
    #[serde(default)]
    pub disable_agent_provisioning: bool,

    /// Enable OpenTelemetry trace export (default: false, requires "telemetry" feature)
    #[serde(default)]
    pub telemetry_enabled: bool,

    /// OTLP endpoint URL (e.g. "https://cloud.langfuse.com/api/public/otel")
    #[serde(default)]
    pub otlp_endpoint: Option<String>,

    /// OTLP authorization header value (e.g. Base64-encoded "public:secret" for Langfuse)
    #[serde(default)]
    pub otlp_auth_header: Option<SecretString>,

    /// Store LLM call metadata (model, tokens, latency) in SQLite (default: true)
    #[serde(default = "default_true")]
    pub store_llm_calls: bool,

    /// Store full tool call input/output in SQLite (default: true)
    #[serde(default = "default_true")]
    pub store_tool_calls: bool,

    /// Log full LLM request/response bodies at debug level (default: false)
    #[serde(default)]
    pub log_llm_bodies: bool,

    /// Resolved home directory path (populated after load, not from config file)
    #[serde(skip)]
    pub home_dir: PathBuf,
}

fn default_true() -> bool {
    true
}

fn default_llm_provider() -> ProviderKind {
    ProviderKind::Anthropic
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

/// Per-provider config fields resolved for the active provider.
pub struct ActiveLlmConfig {
    pub provider: ProviderKind,
    pub model: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
}

impl Settings {
    /// Resolve the GitHub token for agent operations.
    /// Returns `MIKA_GITHUB_TOKEN` only — no fallback to `MIKA_INVESTIGATE_GITHUB_TOKEN`.
    pub fn agent_github_token(&self) -> Option<&str> {
        self.github_token.as_deref()
    }

    /// Resolve the GitHub token to use for agent action operations.
    ///
    /// Returns the `MIKA_GITHUB_TOKEN` PAT if configured — this is the agent's
    /// machine user identity (per ADR-008), required for operations where the
    /// GitHub author/reviewer identity matters: PR reviews, merges, comments,
    /// issue creation. Without distinct PATs per agent, GitHub rejects
    /// cross-agent operations like `mika-qa` approving `mika-dev`'s PRs with
    /// `Review Can not approve your own pull request`, because both agents
    /// would otherwise share the single `mika-platform-bot[bot]` App identity.
    ///
    /// Falls back to a GitHub App installation token only when no PAT is
    /// configured (single-identity deployments, bootstrap, or agents that do
    /// not need a distinct machine user). Returns `None` if neither is
    /// available.
    pub async fn resolve_github_token(
        &self,
        github_app: Option<&crate::github_app::GitHubApp>,
    ) -> Option<String> {
        // PAT first — the agent's machine user identity.
        if let Some(pat) = self.github_token.as_deref() {
            return Some(pat.to_string());
        }
        // Fall back to an App installation token (short-lived, org-scoped).
        if let Some(app) = github_app {
            match app.installation_token().await {
                Ok(token) => return Some(token),
                Err(e) => {
                    tracing::warn!(
                        "GitHub App token exchange failed: {e}. No PAT configured for fallback."
                    );
                }
            }
        }
        None
    }

    /// Return `(model_field, api_key_field, base_url_field)` references for a given provider.
    pub fn provider_fields(
        &self,
        provider: ProviderKind,
    ) -> (Option<&str>, Option<&str>, Option<&str>) {
        match provider {
            ProviderKind::Anthropic => (
                self.anthropic_model.as_deref(),
                self.anthropic_api_key.as_deref(),
                self.anthropic_base_url.as_deref(),
            ),
            ProviderKind::OpenAi => (
                self.openai_model.as_deref(),
                self.openai_api_key.as_deref(), // shared with embedding key
                self.openai_base_url.as_deref(),
            ),
            ProviderKind::OpenRouter => (
                self.openrouter_model.as_deref(),
                self.openrouter_api_key.as_deref(),
                self.openrouter_base_url.as_deref(),
            ),
            ProviderKind::Groq => (
                self.groq_model.as_deref(),
                self.groq_api_key.as_deref(),
                self.groq_base_url.as_deref(),
            ),
            ProviderKind::Ollama => (
                self.ollama_model.as_deref(),
                self.ollama_api_key.as_deref(),
                self.ollama_base_url.as_deref(),
            ),
            ProviderKind::Mistral => (
                self.mistral_model.as_deref(),
                self.mistral_api_key.as_deref(),
                self.mistral_base_url.as_deref(),
            ),
            ProviderKind::Google => (
                self.google_model.as_deref(),
                self.google_api_key.as_deref(),
                self.google_base_url.as_deref(),
            ),
            ProviderKind::DeepSeek => (
                self.deepseek_model.as_deref(),
                self.deepseek_api_key.as_deref(),
                self.deepseek_base_url.as_deref(),
            ),
            ProviderKind::MiniMax => (
                self.minimax_model.as_deref(),
                self.minimax_api_key.as_deref(),
                self.minimax_base_url.as_deref(),
            ),
            ProviderKind::Kimi => (
                self.kimi_model.as_deref(),
                self.kimi_api_key.as_deref(),
                self.kimi_base_url.as_deref(),
            ),
            ProviderKind::Qwen => (
                self.qwen_model.as_deref(),
                self.qwen_api_key.as_deref(),
                self.qwen_base_url.as_deref(),
            ),
        }
    }

    /// Resolve the active LLM configuration from per-provider fields.
    /// Falls back to the provider's default model if none is set.
    pub fn active_llm_config(&self) -> ActiveLlmConfig {
        let (model, api_key, base_url) = self.provider_fields(self.llm_provider);
        ActiveLlmConfig {
            provider: self.llm_provider,
            model: model
                .unwrap_or(self.llm_provider.default_model())
                .to_string(),
            api_key: api_key.map(String::from),
            base_url: base_url.map(String::from),
        }
    }

    /// Set the model for a given provider (used by `--model` overrides and `/model` command).
    pub fn set_provider_model(&mut self, provider: ProviderKind, model: Option<String>) {
        match provider {
            ProviderKind::Anthropic => self.anthropic_model = model,
            ProviderKind::OpenAi => self.openai_model = model,
            ProviderKind::OpenRouter => self.openrouter_model = model,
            ProviderKind::Groq => self.groq_model = model,
            ProviderKind::Ollama => self.ollama_model = model,
            ProviderKind::Mistral => self.mistral_model = model,
            ProviderKind::Google => self.google_model = model,
            ProviderKind::DeepSeek => self.deepseek_model = model,
            ProviderKind::MiniMax => self.minimax_model = model,
            ProviderKind::Kimi => self.kimi_model = model,
            ProviderKind::Qwen => self.qwen_model = model,
        }
    }

    /// The active model string for display (provider/model or just model for Anthropic).
    pub fn active_model_display(&self) -> String {
        let config = self.active_llm_config();
        if config.provider == ProviderKind::Anthropic {
            config.model
        } else {
            format!("{}/{}", config.provider, config.model)
        }
    }

    /// Create an LLM provider from the current settings.
    ///
    /// Resolves per-provider model, api_key, and base_url from the active provider's fields.
    pub fn make_llm_provider(&self) -> anyhow::Result<Arc<dyn crate::llm::LlmProvider>> {
        let config = self.active_llm_config();
        let spec = crate::llm::ModelSpec {
            provider: config.provider,
            model: config.model,
            base_url: config.base_url,
            api_key: config.api_key,
        };
        crate::llm::create_provider(&spec, self.llm_max_tokens)
    }

    /// Create an LLM provider for a specific provider kind with an optional model override.
    ///
    /// Used by per-skill LLM overrides: a skill's `[llm]` section can specify a different
    /// provider and/or model. This method resolves the provider's credentials from the
    /// agent's per-provider config and constructs a fresh provider instance.
    ///
    /// Falls back to the provider's default model if no model override is given and
    /// no per-provider model is configured.
    pub fn make_provider_for(
        &self,
        provider: crate::llm::ProviderKind,
        model_override: Option<&str>,
    ) -> anyhow::Result<Arc<dyn crate::llm::LlmProvider>> {
        let (model_field, api_key, base_url) = self.provider_fields(provider);
        let model = model_override
            .map(String::from)
            .or_else(|| model_field.map(String::from))
            .unwrap_or_else(|| provider.default_model().to_string());
        let spec = crate::llm::ModelSpec {
            provider,
            model,
            base_url: base_url.map(String::from),
            api_key: api_key.map(String::from),
        };
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
    ///   4. `~/.mika/agents/<name>/.env`             (per-agent secrets, parsed inline)
    ///   5. `~/.mika/.env`                          (global secrets, loaded by caller into process env)
    ///   6. MIKA_* env vars                         (highest priority — shell always wins)
    ///
    /// In CLI mode (single agent), the caller also loads per-agent `.env` into the
    /// process environment before this method, so layers 4 and 5 are redundant but
    /// harmless. In server mode (multiple agents), per-agent `.env` is NOT in the
    /// process environment — layer 4 is the only path for per-agent secrets.
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

        // Per-agent .env: parse without mutating process env, inject as config source.
        // Priority: config files < per-agent .env < process env vars (shell always wins).
        // In server mode, process env only has global .env values — this is the only
        // path for per-agent secrets like MIKA_GITHUB_APP_*.
        // Converted to inline TOML and added as a File source so that the process-env
        // Environment source (added next) retains highest priority.
        if global_home != agent_home {
            let dotenv_vars = crate::dotenv::parse_dotenv(agent_home);
            if !dotenv_vars.is_empty() {
                let toml = crate::dotenv::dotenv_to_toml(&dotenv_vars);
                builder = builder.add_source(File::from_str(&toml, FileFormat::Toml));
            }
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
            .field("llm_provider", &self.llm_provider)
            .field("llm_max_tokens", &self.llm_max_tokens)
            // Per-provider (redact api_keys)
            .field("anthropic_model", &self.anthropic_model)
            .field(
                "anthropic_api_key",
                &self.anthropic_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("anthropic_base_url", &self.anthropic_base_url)
            .field("openai_model", &self.openai_model)
            .field(
                "openai_api_key",
                &self.openai_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("openai_base_url", &self.openai_base_url)
            .field("openrouter_model", &self.openrouter_model)
            .field(
                "openrouter_api_key",
                &self.openrouter_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("openrouter_base_url", &self.openrouter_base_url)
            .field("groq_model", &self.groq_model)
            .field(
                "groq_api_key",
                &self.groq_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("groq_base_url", &self.groq_base_url)
            .field("ollama_model", &self.ollama_model)
            .field(
                "ollama_api_key",
                &self.ollama_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("ollama_base_url", &self.ollama_base_url)
            .field("mistral_model", &self.mistral_model)
            .field(
                "mistral_api_key",
                &self.mistral_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("mistral_base_url", &self.mistral_base_url)
            .field("google_model", &self.google_model)
            .field(
                "google_api_key",
                &self.google_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("google_base_url", &self.google_base_url)
            .field("deepseek_model", &self.deepseek_model)
            .field(
                "deepseek_api_key",
                &self.deepseek_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "minimax_api_key",
                &self.minimax_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("deepseek_base_url", &self.deepseek_base_url)
            // Non-provider
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
            .field("embedding_model", &self.embedding_model)
            .field("embedding_dimensions", &self.embedding_dimensions)
            .field(
                "brave_api_key",
                &self.brave_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "github_token",
                &self.github_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "investigate_github_token",
                &self.investigate_github_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("github_repo", &self.github_repo)
            .field("github_app_id", &self.github_app_id)
            .field(
                "github_app_private_key",
                &self.github_app_private_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "github_app_installation_id",
                &self.github_app_installation_id,
            )
            .field("github_app_login", &self.github_app_login)
            .field("server_log_file", &self.server_log_file)
            .field("dashboard_enabled", &self.dashboard_enabled)
            .field("disable_bundled_skills", &self.disable_bundled_skills)
            .field("dev_mode", &self.dev_mode)
            .field(
                "disable_agent_provisioning",
                &self.disable_agent_provisioning,
            )
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
            std::env::remove_var("MIKA_LLM_PROVIDER");
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
        assert_eq!(settings.llm_provider, ProviderKind::Anthropic);
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
            "llm_provider = \"openai\"\nopenai_model = \"gpt-4o\"\nlog_level = \"debug\"\n",
        )
        .unwrap();

        let settings = Settings::load(tmp.path()).unwrap();
        assert_eq!(settings.llm_provider, ProviderKind::OpenAi);
        assert_eq!(settings.openai_model, Some("gpt-4o".to_string()));
        assert_eq!(settings.log_level, "debug");
    }

    #[test]
    #[serial]
    fn test_env_overrides_home_config() {
        clean_env();
        // Safety: test-only env var.
        unsafe { std::env::set_var("MIKA_LLM_PROVIDER", "groq") };

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            "llm_provider = \"openai\"\n",
        )
        .unwrap();

        let settings = Settings::load(tmp.path()).unwrap();
        // Env var should win over home config
        assert_eq!(settings.llm_provider, ProviderKind::Groq);

        unsafe { std::env::remove_var("MIKA_LLM_PROVIDER") };
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

        // Global config sets provider and log_level
        std::fs::write(
            global_home.join("config.toml"),
            "llm_provider = \"openai\"\nlog_level = \"debug\"\n",
        )
        .unwrap();

        // Agent config overrides provider but not log_level
        std::fs::write(
            agent_home.join("config.toml"),
            "llm_provider = \"anthropic\"\n",
        )
        .unwrap();

        let settings = Settings::load_for_agent(&global_home, &agent_home).unwrap();
        // Agent config should override global config
        assert_eq!(settings.llm_provider, ProviderKind::Anthropic);
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
            "llm_provider = \"anthropic\"\n",
        )
        .unwrap();

        let via_load = Settings::load(tmp.path()).unwrap();
        let via_load_for_agent = Settings::load_for_agent(tmp.path(), tmp.path()).unwrap();
        assert_eq!(via_load.llm_provider, via_load_for_agent.llm_provider);
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

    #[test]
    #[serial]
    fn test_dev_mode_from_env() {
        clean_env();
        // Safety: test-only env var.
        unsafe { std::env::set_var("MIKA_DEV_MODE", "true") };

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();
        assert!(settings.dev_mode);

        unsafe { std::env::remove_var("MIKA_DEV_MODE") };
    }

    #[test]
    #[serial]
    fn test_disable_agent_provisioning_from_env() {
        clean_env();
        // Safety: test-only env var.
        unsafe { std::env::set_var("MIKA_DISABLE_AGENT_PROVISIONING", "true") };

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();
        assert!(settings.disable_agent_provisioning);

        unsafe { std::env::remove_var("MIKA_DISABLE_AGENT_PROVISIONING") };
    }

    #[test]
    #[serial]
    fn test_dev_mode_defaults_false() {
        clean_env();

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();
        assert!(!settings.dev_mode);
        assert!(!settings.disable_agent_provisioning);
    }

    #[test]
    #[serial]
    fn test_active_llm_config_uses_provider_defaults() {
        clean_env();

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();
        let config = settings.active_llm_config();
        assert_eq!(config.provider, ProviderKind::Anthropic);
        assert_eq!(config.model, "claude-sonnet-4-6");
        assert_eq!(config.api_key, None);
    }

    #[test]
    #[serial]
    fn test_active_llm_config_with_explicit_model() {
        clean_env();

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            "llm_provider = \"openai\"\nopenai_model = \"gpt-4-turbo\"\n",
        )
        .unwrap();
        let settings = Settings::load(tmp.path()).unwrap();
        let config = settings.active_llm_config();
        assert_eq!(config.provider, ProviderKind::OpenAi);
        assert_eq!(config.model, "gpt-4-turbo");
    }

    #[test]
    #[serial]
    fn test_active_model_display() {
        clean_env();

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();
        // Anthropic shows just the model
        assert_eq!(settings.active_model_display(), "claude-sonnet-4-6");
    }

    #[tokio::test]
    #[serial]
    async fn test_resolve_github_token_returns_pat_when_set() {
        clean_env();

        let tmp = tempfile::tempdir().unwrap();
        let mut settings = Settings::load(tmp.path()).unwrap();
        // Simulate per-agent `.env` setting MIKA_GITHUB_TOKEN to the machine
        // user PAT (e.g., `github_pat_...`).
        settings.github_token = Some("github_pat_test_value".to_string());

        // With no GitHub App available, `resolve_github_token` must still
        // return the PAT — the PAT is the agent's machine user identity and
        // is the primary token source per ADR-008.
        let resolved = settings.resolve_github_token(None).await;
        assert_eq!(resolved.as_deref(), Some("github_pat_test_value"));
    }

    #[tokio::test]
    #[serial]
    async fn test_resolve_github_token_returns_none_when_nothing_set() {
        clean_env();

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();
        // Clean-slate settings: no PAT, and the test passes `None` for the
        // App parameter, so `resolve_github_token` has nothing to return.
        assert!(settings.github_token.is_none());

        let resolved = settings.resolve_github_token(None).await;
        assert!(resolved.is_none());
    }
}

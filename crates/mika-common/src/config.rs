use std::sync::Arc;

use config::{Config, Environment, File, FileFormat};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde::de;
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
    // -- Per-provider: MikaModel (internal endpoint) --
    ConfigKeyInfo {
        key: "mikamodel_model",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_MIKAMODEL_MODEL"),
        secret: false,
        description: "MikaModel model ID",
    },
    ConfigKeyInfo {
        key: "mikamodel_api_key",
        backend: ConfigBackend::Env,
        env_var: Some("MIKA_MIKAMODEL_API_KEY"),
        secret: true,
        description: "MikaModel API key (optional; reserved for hosted-endpoint swap)",
    },
    ConfigKeyInfo {
        key: "mikamodel_base_url",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_MIKAMODEL_BASE_URL"),
        secret: false,
        description: "MikaModel base URL (defaults to local Ollama transport)",
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
        description: "Stdout log format for mika-spirit and mika-gateway (json or pretty). CLI always uses pretty.",
    },
    ConfigKeyInfo {
        key: "spirit_port",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_SPIRIT_PORT"),
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
    // -- Operational partner --
    ConfigKeyInfo {
        key: "operational_partner",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_OPERATIONAL_PARTNER"),
        secret: false,
        description: "Enable operational partner read APIs (writes always-on, default: false)",
    },
    // -- Task engine --
    ConfigKeyInfo {
        key: "max_agent_tasks_per_session",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_MAX_AGENT_TASKS_PER_SESSION"),
        secret: false,
        description: "Maximum agent-created tasks per session (default: 25)",
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
    // -- Knowledge Graph --
    ConfigKeyInfo {
        key: "kg_docs_root",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_KG_DOCS_ROOT"),
        secret: false,
        description: "Absolute path to docs root for KG lexical ingestion. Defaults to <CWD>/docs/solutions when unset. Needed on hosts where the service CWD != repo root (e.g., OpenRC).",
    },
    ConfigKeyInfo {
        key: "kg_docs_roots",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_KG_DOCS_ROOTS"),
        secret: false,
        description: "Colon-separated list of absolute paths to docs root directories for multi-corpus agents. Global fallback; per-agent identity.toml [kg].docs_roots takes precedence. Linux/macOS only.",
    },
    // Permission-decision authority (mika#1733 AC3, AC8)
    ConfigKeyInfo {
        key: "decision_authority",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_DECISION_AUTHORITY"),
        secret: false,
        description: "Global permission-decision authority: 'strict' (default) means the classifier verdict wins; 'override' allows the operator to flip a classifier deny. Per-tenant/per-agent scopes resolved via MIKA_DECISION_AUTHORITY__TENANT__<id> / MIKA_DECISION_AUTHORITY__AGENT__<id> env vars.",
    },
    ConfigKeyInfo {
        key: "permission_hold_timeout_secs",
        backend: ConfigBackend::File,
        env_var: Some("MIKA_PERMISSION_HOLD_TIMEOUT_SECS"),
        secret: false,
        description: "Server-side held-request timeout in seconds. Timeout materializes an internal deny (fail-closed). Default: 300.",
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
        "spirit_port" => Some(settings.spirit_port.to_string()),
        "embedding_model" => Some(settings.embedding_model.clone()),
        "embedding_dimensions" => Some(settings.embedding_dimensions.to_string()),
        "dashboard_enabled" => Some(settings.dashboard_enabled.to_string()),

        // Per-provider: Anthropic
        "anthropic_model" => settings.anthropic_model.clone(),
        "anthropic_api_key" => settings
            .anthropic_api_key
            .as_ref()
            .map(|_| "[SET]".to_string()),
        "anthropic_base_url" => settings.anthropic_base_url.clone(),
        // Per-provider: OpenAI (api_key shares with openai_api_key)
        "openai_model" => settings.openai_model.clone(),
        "openai_api_key" => settings
            .openai_api_key
            .as_ref()
            .map(|_| "[SET]".to_string()),
        "openai_base_url" => settings.openai_base_url.clone(),
        // Per-provider: OpenRouter
        "openrouter_model" => settings.openrouter_model.clone(),
        "openrouter_api_key" => settings
            .openrouter_api_key
            .as_ref()
            .map(|_| "[SET]".to_string()),
        "openrouter_base_url" => settings.openrouter_base_url.clone(),
        // Per-provider: Groq
        "groq_model" => settings.groq_model.clone(),
        "groq_api_key" => settings.groq_api_key.as_ref().map(|_| "[SET]".to_string()),
        "groq_base_url" => settings.groq_base_url.clone(),
        // Per-provider: Ollama
        "ollama_model" => settings.ollama_model.clone(),
        "ollama_api_key" => settings
            .ollama_api_key
            .as_ref()
            .map(|_| "[SET]".to_string()),
        "ollama_base_url" => settings.ollama_base_url.clone(),
        // Per-provider: Mistral
        "mistral_model" => settings.mistral_model.clone(),
        "mistral_api_key" => settings
            .mistral_api_key
            .as_ref()
            .map(|_| "[SET]".to_string()),
        "mistral_base_url" => settings.mistral_base_url.clone(),
        // Per-provider: Google
        "google_model" => settings.google_model.clone(),
        "google_api_key" => settings
            .google_api_key
            .as_ref()
            .map(|_| "[SET]".to_string()),
        "google_base_url" => settings.google_base_url.clone(),
        // Per-provider: DeepSeek
        "deepseek_model" => settings.deepseek_model.clone(),
        "deepseek_api_key" => settings
            .deepseek_api_key
            .as_ref()
            .map(|_| "[SET]".to_string()),
        "minimax_api_key" => settings
            .minimax_api_key
            .as_ref()
            .map(|_| "[SET]".to_string()),
        "kimi_api_key" => settings.kimi_api_key.as_ref().map(|_| "[SET]".to_string()),
        "qwen_api_key" => settings.qwen_api_key.as_ref().map(|_| "[SET]".to_string()),
        "zai_api_key" => settings.zai_api_key.as_ref().map(|_| "[SET]".to_string()),
        "deepseek_base_url" => settings.deepseek_base_url.clone(),
        // Per-provider: MikaModel
        "mikamodel_model" => settings.mikamodel_model.clone(),
        "mikamodel_api_key" => settings
            .mikamodel_api_key
            .as_ref()
            .map(|_| "[SET]".to_string()),
        "mikamodel_base_url" => settings.mikamodel_base_url.clone(),

        // Non-provider secrets/settings
        "brave_api_key" => settings.brave_api_key.as_ref().map(|_| "[SET]".to_string()),
        "github_token" => settings.github_token.as_ref().map(|_| "[SET]".to_string()),
        "investigate_github_token" => settings
            .investigate_github_token
            .as_ref()
            .map(|_| "[SET]".to_string()),
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
            .map(|_| "[SET]".to_string()),
        "home_dir" => Some(settings.home_dir.display().to_string()),
        "db_path" => Some(settings.db_path.display().to_string()),
        // Task engine
        "max_agent_tasks_per_session" => Some(settings.max_agent_tasks_per_session.to_string()),
        // Observability
        "store_llm_calls" => Some(settings.store_llm_calls.to_string()),
        "store_tool_calls" => Some(settings.store_tool_calls.to_string()),
        "log_llm_bodies" => Some(settings.log_llm_bodies.to_string()),
        // KG
        "kg_docs_root" => settings
            .kg_docs_root
            .as_ref()
            .map(|p| p.display().to_string()),
        "kg_docs_roots" => settings.kg_docs_roots.as_ref().map(|paths| {
            paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(":")
        }),
        // Permission-decision authority (mika#1733)
        "decision_authority" => Some(match settings.decision_authority {
            DecisionAuthority::Strict => "strict".to_string(),
            DecisionAuthority::Override => "override".to_string(),
        }),
        "permission_hold_timeout_secs" => Some(settings.permission_hold_timeout_secs.to_string()),
        // DB keys (timezone, thinking_level) not available from Settings
        _ => None,
    }
}

/// Server-side permission-decision authority (mika#1733 AC3, AC8).
///
/// Controls whether an operator's decision can flip a classifier verdict.
/// Compile-time default is [`DecisionAuthority::Strict`] per AC8 — the operator's
/// decision is advisory only and the classifier verdict always wins. `Override`
/// mode is opt-in via configuration (env or config file) and NEVER via wire
/// input; see [`crate::config::CONFIG_KEYS`] for the `MIKA_DECISION_AUTHORITY`
/// key registration.
///
/// Wire-schema note: `PermissionDecideRequest` in `mika-agent` rejects any
/// `decision_authority` field on the POST body via
/// `#[serde(deny_unknown_fields)]` — server-side config is NEVER wire-carried.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAuthority {
    /// Operator decisions are advisory only; classifier verdict wins. This is
    /// the shipped default per AC8.
    #[default]
    Strict,
    /// Operator decisions can flip a classifier deny to approve. Enabled only
    /// via explicit config.
    Override,
}

/// Default server-side hold-timeout for a held permission request (seconds).
/// Overridable via `MIKA_PERMISSION_HOLD_TIMEOUT_SECS`. Timeout materializes
/// an internal `deny` per the fail-closed discipline.
pub const DEFAULT_PERMISSION_HOLD_TIMEOUT_SECS: u64 = 300;

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
    pub anthropic_api_key: Option<SecretString>,
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
    pub openrouter_api_key: Option<SecretString>,
    #[serde(default)]
    pub openrouter_base_url: Option<String>,

    // -- Per-provider fields: Groq --
    #[serde(default)]
    pub groq_model: Option<String>,
    #[serde(default)]
    pub groq_api_key: Option<SecretString>,
    #[serde(default)]
    pub groq_base_url: Option<String>,

    // -- Per-provider fields: Ollama --
    #[serde(default)]
    pub ollama_model: Option<String>,
    #[serde(default)]
    pub ollama_api_key: Option<SecretString>,
    #[serde(default)]
    pub ollama_base_url: Option<String>,

    // -- Per-provider fields: Mistral --
    #[serde(default)]
    pub mistral_model: Option<String>,
    #[serde(default)]
    pub mistral_api_key: Option<SecretString>,
    #[serde(default)]
    pub mistral_base_url: Option<String>,

    // -- Per-provider fields: Google --
    #[serde(default)]
    pub google_model: Option<String>,
    #[serde(default)]
    pub google_api_key: Option<SecretString>,
    #[serde(default)]
    pub google_base_url: Option<String>,

    // -- Per-provider fields: DeepSeek --
    #[serde(default)]
    pub deepseek_model: Option<String>,
    pub minimax_model: Option<String>,
    pub minimax_base_url: Option<String>,
    pub kimi_api_key: Option<SecretString>,
    pub kimi_model: Option<String>,
    pub kimi_base_url: Option<String>,
    pub qwen_api_key: Option<SecretString>,
    pub qwen_model: Option<String>,
    pub qwen_base_url: Option<String>,
    // -- Per-provider fields: Z.AI (direct GLM API, OpenAI-compatible) --
    #[serde(default)]
    pub zai_api_key: Option<SecretString>,
    #[serde(default)]
    pub zai_model: Option<String>,
    #[serde(default)]
    pub zai_base_url: Option<String>,
    // -- Per-provider fields: MikaModel (internal endpoint, served via Ollama transport) --
    pub mikamodel_model: Option<String>,
    pub mikamodel_api_key: Option<SecretString>,
    pub mikamodel_base_url: Option<String>,
    #[serde(default)]
    pub deepseek_api_key: Option<SecretString>,
    pub minimax_api_key: Option<SecretString>,
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
    #[serde(default = "default_spirit_port")]
    pub spirit_port: u16,

    /// Internal bearer token for gateway ↔ container auth
    #[serde(default)]
    pub internal_token: Option<SecretString>,

    /// Separate bearer token for read-only dashboard API routes (env: MIKA_DASHBOARD_TOKEN).
    #[serde(default)]
    pub dashboard_token: Option<SecretString>,

    /// OpenAI API key — used for embeddings AND as the OpenAI LLM provider's API key.
    /// Legacy field: MIKA_OPENAI_API_KEY env var.
    #[serde(default)]
    pub openai_api_key: Option<SecretString>,

    /// Embedding model ID (default: text-embedding-3-small)
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,

    /// Embedding dimensions (default: 512)
    #[serde(default = "default_embedding_dimensions")]
    pub embedding_dimensions: u32,

    /// Brave Search API key (optional; enables web_search builtin skill)
    #[serde(default)]
    pub brave_api_key: Option<SecretString>,

    /// Optional log file path for mika-spirit (maps to MIKA_SPIRIT_LOG_FILE)
    #[serde(default)]
    pub spirit_log_file: Option<PathBuf>,

    /// GitHub Personal Access Token for agent operations (context injection, task enrichment, PR merge).
    /// No fallback — `investigate_github_token` is used only by the investigation panel.
    #[serde(default)]
    pub github_token: Option<SecretString>,

    /// GitHub Personal Access Token for investigation panel issue creation (optional)
    #[serde(default)]
    pub investigate_github_token: Option<SecretString>,

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

    /// Maximum agent-created tasks per session (default: 25).
    /// Guards against runaway task creation while allowing legitimate bulk operations.
    #[serde(default = "default_max_agent_tasks_per_session")]
    pub max_agent_tasks_per_session: i64,

    /// Store LLM call metadata (model, tokens, latency) in SQLite (default: true)
    #[serde(default = "default_true")]
    pub store_llm_calls: bool,

    /// Store full tool call input/output in SQLite (default: true)
    #[serde(default = "default_true")]
    pub store_tool_calls: bool,

    /// Log full LLM request/response bodies at debug level (default: false)
    #[serde(default)]
    pub log_llm_bodies: bool,

    // -- KG (Knowledge Graph) model settings --
    /// KG ingestion model — shared fallback for extraction and resolution models.
    /// Format: `provider/model` (e.g., `anthropic/claude-haiku-4-5-20251001`).
    /// If unset, KG features requiring LLM calls are disabled.
    #[serde(default)]
    pub kg_ingestion_model: Option<String>,

    /// KG extraction model — used for NER + fact-triple extraction (#690).
    /// Falls back to `kg_ingestion_model` if unset.
    #[serde(default)]
    pub kg_extraction_model: Option<String>,

    /// KG resolution model — used for entity disambiguation (#691).
    /// Falls back to `kg_ingestion_model` if unset. Mid-tier model recommended
    /// for better judgment on ambiguous entity matches.
    #[serde(default)]
    pub kg_resolution_model: Option<String>,

    /// KG per-batch LLM call budget — structural cap on startup extraction and
    /// resolution batches (#757). Unset = default of [`DEFAULT_KG_BATCH_BUDGET`].
    /// `0` disables the phase entirely (no LLM calls).
    ///
    /// Worst-case per-startup cost is `2 × N_agents × budget` — extraction batch
    /// plus resolution batch, one of each per agent. See `kg_schema.rs` for the
    /// fan-out cost model.
    #[serde(default)]
    pub kg_batch_budget: Option<u32>,

    /// Callback watchdog grace period in seconds (#959).
    ///
    /// After detecting that a callback task's subprocess has exited (PID dead),
    /// the watchdog waits this many seconds before marking the task `failed`.
    /// This grace period allows for in-flight callback delivery that may be in
    /// transit when the subprocess exits cleanly.
    ///
    /// Default: [`DEFAULT_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS`] (120s).
    #[serde(default)]
    pub callback_watchdog_grace_period_secs: Option<u64>,

    /// KG docs root — absolute path to the docs directory the lexical ingestor
    /// reads (#738). Resolution chain: `MIKA_KG_DOCS_ROOT` env > `kg_docs_root`
    /// config field > `<CWD>/docs/solutions` (container-native default).
    /// Needed on hosts where the service CWD ≠ repo root (e.g., OpenRC
    /// `supervise-daemon` launches with CWD=`/`).
    #[serde(default)]
    pub kg_docs_root: Option<PathBuf>,

    /// KG docs roots — colon-separated list of absolute paths to docs root
    /// directories for multi-corpus agents (#798). Global fallback; per-agent
    /// `identity.toml [kg].docs_roots` takes precedence. Linux/macOS only
    /// (colon separator conflicts with Windows drive letters).
    #[serde(default, deserialize_with = "deserialize_colon_paths")]
    pub kg_docs_roots: Option<Vec<PathBuf>>,

    /// Enable operational partner mode — gates read APIs for operational items
    /// (HTTP endpoint, CLI surface). Writes are always-on once the migration
    /// lands (mika#1262). Default: false.
    #[serde(default)]
    pub operational_partner: bool,

    /// Global permission-decision authority (mika#1733 AC3, AC8). Compile-time
    /// default is [`DecisionAuthority::Strict`]. Per-tenant / per-agent scopes
    /// are resolved by [`crate::permission_authority::resolve_authority`] (env
    /// vars only — `Settings` is the global-tier fallback).
    #[serde(default)]
    pub decision_authority: DecisionAuthority,

    /// Server-side held-request timeout in seconds (mika#1733 AC3). Default:
    /// [`DEFAULT_PERMISSION_HOLD_TIMEOUT_SECS`] (300).
    #[serde(default = "default_permission_hold_timeout_secs")]
    pub permission_hold_timeout_secs: u64,

    /// Resolved home directory path (populated after load, not from config file)
    #[serde(skip)]
    pub home_dir: PathBuf,
}

fn default_permission_hold_timeout_secs() -> u64 {
    DEFAULT_PERMISSION_HOLD_TIMEOUT_SECS
}

/// Default per-batch LLM call budget for KG extraction and resolution (#757).
pub const DEFAULT_KG_BATCH_BUDGET: u32 = 500;

/// Default grace period (seconds) for the callback watchdog (#959).
///
/// After detecting subprocess death, wait this long before marking the callback
/// task `failed` — allows for in-flight callback delivery that may be in transit.
pub const DEFAULT_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS: u64 = 120;

fn default_true() -> bool {
    true
}

/// Deserialize `kg_docs_roots` from either a colon-separated string (env var /
/// dotenv path) or a native TOML/JSON array.  Aligns with the manual
/// `split(':').filter(|p| !p.is_empty())` in `kg/config.rs` Tier 3.
fn deserialize_colon_paths<'de, D>(deserializer: D) -> Result<Option<Vec<PathBuf>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct ColonPathsVisitor;

    impl<'de> de::Visitor<'de> for ColonPathsVisitor {
        type Value = Option<Vec<PathBuf>>;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a colon-separated string or array of paths")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            let paths: Vec<PathBuf> = v
                .split(':')
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .collect();
            if paths.is_empty() {
                Ok(None)
            } else {
                Ok(Some(paths))
            }
        }

        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut paths = Vec::new();
            while let Some(s) = seq.next_element::<String>()? {
                if !s.is_empty() {
                    paths.push(PathBuf::from(s));
                }
            }
            if paths.is_empty() {
                Ok(None)
            } else {
                Ok(Some(paths))
            }
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
    }

    deserializer.deserialize_any(ColonPathsVisitor)
}

fn default_llm_provider() -> ProviderKind {
    ProviderKind::Anthropic
}

fn default_max_tokens() -> u32 {
    4096
}

fn default_max_agent_tasks_per_session() -> i64 {
    25
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

fn default_spirit_port() -> u16 {
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
        self.github_token.as_ref().map(|s| s.expose_secret())
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
        if let Some(pat) = self.github_token.as_ref() {
            return Some(pat.expose_secret().to_string());
        }
        // Fall back to an App installation token (short-lived, org-scoped).
        if let Some(app) = github_app {
            match app.installation_token().await {
                Ok(token) => return Some(token),
                Err(e) => {
                    tracing::warn!(
                        target: "mika::github_auth",
                        event = "gh_app_token_exchange_failed",
                        error = %e,
                        has_pat_fallback = false,
                        "GitHub App token exchange failed; no PAT configured for fallback"
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
                self.anthropic_api_key.as_ref().map(|s| s.expose_secret()),
                self.anthropic_base_url.as_deref(),
            ),
            ProviderKind::OpenAi => (
                self.openai_model.as_deref(),
                self.openai_api_key.as_ref().map(|s| s.expose_secret()),
                self.openai_base_url.as_deref(),
            ),
            ProviderKind::OpenRouter => (
                self.openrouter_model.as_deref(),
                self.openrouter_api_key.as_ref().map(|s| s.expose_secret()),
                self.openrouter_base_url.as_deref(),
            ),
            ProviderKind::Groq => (
                self.groq_model.as_deref(),
                self.groq_api_key.as_ref().map(|s| s.expose_secret()),
                self.groq_base_url.as_deref(),
            ),
            ProviderKind::Ollama => (
                self.ollama_model.as_deref(),
                self.ollama_api_key.as_ref().map(|s| s.expose_secret()),
                self.ollama_base_url.as_deref(),
            ),
            ProviderKind::Mistral => (
                self.mistral_model.as_deref(),
                self.mistral_api_key.as_ref().map(|s| s.expose_secret()),
                self.mistral_base_url.as_deref(),
            ),
            ProviderKind::Google => (
                self.google_model.as_deref(),
                self.google_api_key.as_ref().map(|s| s.expose_secret()),
                self.google_base_url.as_deref(),
            ),
            ProviderKind::DeepSeek => (
                self.deepseek_model.as_deref(),
                self.deepseek_api_key.as_ref().map(|s| s.expose_secret()),
                self.deepseek_base_url.as_deref(),
            ),
            ProviderKind::MiniMax => (
                self.minimax_model.as_deref(),
                self.minimax_api_key.as_ref().map(|s| s.expose_secret()),
                self.minimax_base_url.as_deref(),
            ),
            ProviderKind::Kimi => (
                self.kimi_model.as_deref(),
                self.kimi_api_key.as_ref().map(|s| s.expose_secret()),
                self.kimi_base_url.as_deref(),
            ),
            ProviderKind::Qwen => (
                self.qwen_model.as_deref(),
                self.qwen_api_key.as_ref().map(|s| s.expose_secret()),
                self.qwen_base_url.as_deref(),
            ),
            ProviderKind::MikaModel => (
                self.mikamodel_model.as_deref(),
                self.mikamodel_api_key.as_ref().map(|s| s.expose_secret()),
                self.mikamodel_base_url.as_deref(),
            ),
            ProviderKind::ZAi => (
                self.zai_model.as_deref(),
                self.zai_api_key.as_ref().map(|s| s.expose_secret()),
                self.zai_base_url.as_deref(),
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
            ProviderKind::MikaModel => self.mikamodel_model = model,
            ProviderKind::ZAi => self.zai_model = model,
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
        crate::llm::create_provider(&spec, self.llm_max_tokens, self.log_llm_bodies)
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
        crate::llm::create_provider(&spec, self.llm_max_tokens, self.log_llm_bodies)
    }

    /// Create an LLM provider for KG extraction (NER + fact triples).
    ///
    /// Resolution order: `MIKA_KG_EXTRACTION_MODEL` → `MIKA_KG_INGESTION_MODEL` → `None`.
    /// Format: `provider/model` (e.g., `anthropic/claude-haiku-4-5-20251001`).
    /// Returns `None` when no KG model is configured (extraction disabled).
    pub fn make_kg_extraction_provider(
        &self,
    ) -> Option<anyhow::Result<Arc<dyn crate::llm::LlmProvider>>> {
        let model_str = self
            .kg_extraction_model
            .as_deref()
            .or(self.kg_ingestion_model.as_deref())?;

        Some(self.make_provider_from_model_string(model_str))
    }

    /// Create an LLM provider for KG entity resolution (disambiguation).
    ///
    /// Resolution order: `MIKA_KG_RESOLUTION_MODEL` → `MIKA_KG_INGESTION_MODEL` → `None`.
    /// Format: `provider/model` (e.g., `anthropic/claude-haiku-4-5-20251001`).
    /// Returns `None` when no KG model is configured (resolution LLM disabled).
    pub fn make_kg_resolution_provider(
        &self,
    ) -> Option<anyhow::Result<Arc<dyn crate::llm::LlmProvider>>> {
        let model_str = self
            .kg_resolution_model
            .as_deref()
            .or(self.kg_ingestion_model.as_deref())?;

        Some(self.make_provider_from_model_string(model_str))
    }

    /// Effective KG per-batch LLM call budget (#757).
    ///
    /// Returns the configured value or [`DEFAULT_KG_BATCH_BUDGET`] when unset.
    /// A return value of `0` means "disabled — the extraction or resolution
    /// phase makes zero LLM calls per batch and returns immediately."
    pub fn effective_kg_batch_budget(&self) -> u32 {
        self.kg_batch_budget.unwrap_or(DEFAULT_KG_BATCH_BUDGET)
    }

    /// Effective callback watchdog grace period in seconds (#959).
    ///
    /// Returns the configured value or [`DEFAULT_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS`] (120s).
    pub fn effective_callback_watchdog_grace_period_secs(&self) -> u64 {
        self.callback_watchdog_grace_period_secs
            .unwrap_or(DEFAULT_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS)
    }

    /// Parse a `provider/model` string and create an LLM provider.
    ///
    /// Reusable by `make_kg_extraction_provider` and `make_kg_resolution_provider`.
    fn make_provider_from_model_string(
        &self,
        model_str: &str,
    ) -> anyhow::Result<Arc<dyn crate::llm::LlmProvider>> {
        let (provider_str, model) = model_str
            .split_once('/')
            .ok_or_else(|| anyhow::anyhow!(
                "KG model string must be in 'provider/model' format (e.g., 'anthropic/claude-haiku-4-5-20251001'), got: {model_str}"
            ))?;

        let provider: crate::llm::ProviderKind = provider_str
            .parse()
            .map_err(|e: String| anyhow::anyhow!(e))?;

        self.make_provider_for(provider, Some(model))
    }

    /// Create an EmbeddingClient if OpenAI API key is configured.
    pub fn make_embedding_client(&self) -> Option<crate::embedding::EmbeddingClient> {
        self.openai_api_key
            .as_ref()
            .map(|s| s.expose_secret())
            .filter(|k| !k.trim().is_empty())
            .and_then(|key| {
                crate::embedding::EmbeddingClient::new(
                    key.to_string(),
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

    /// Minimal `Settings` for deterministic tests — no API keys, no network, no secrets.
    ///
    /// Lives on `Settings` itself so that any new field addition produces a compile
    /// error here rather than in scattered test helpers.
    #[cfg(any(test, feature = "test-utils"))]
    pub fn test_defaults() -> Self {
        Self {
            llm_provider: ProviderKind::Anthropic,
            llm_max_tokens: 4096,
            // Per-provider fields (all None = use defaults)
            anthropic_model: None,
            anthropic_api_key: None,
            anthropic_base_url: None,
            openai_model: None,
            openai_base_url: None,
            openrouter_model: None,
            openrouter_api_key: None,
            openrouter_base_url: None,
            groq_model: None,
            groq_api_key: None,
            groq_base_url: None,
            ollama_model: None,
            ollama_api_key: None,
            ollama_base_url: None,
            mistral_model: None,
            mistral_api_key: None,
            mistral_base_url: None,
            google_model: None,
            google_api_key: None,
            google_base_url: None,
            deepseek_model: None,
            deepseek_api_key: None,
            deepseek_base_url: None,
            minimax_model: None,
            minimax_api_key: None,
            minimax_base_url: None,
            kimi_model: None,
            kimi_api_key: None,
            kimi_base_url: None,
            qwen_model: None,
            qwen_api_key: None,
            qwen_base_url: None,
            zai_model: None,
            zai_api_key: None,
            zai_base_url: None,
            mikamodel_model: None,
            mikamodel_api_key: None,
            mikamodel_base_url: None,
            // Non-provider settings
            db_path: PathBuf::from("test.db"),
            log_level: "info".to_string(),
            log_format: "json".to_string(),
            routing_url: None,
            customer_id: None,
            spirit_port: 8080,
            internal_token: None,
            dashboard_token: None,
            openai_api_key: None,
            embedding_model: "text-embedding-3-small".to_string(),
            embedding_dimensions: 512,
            brave_api_key: None,
            github_token: None,
            investigate_github_token: None,
            github_repo: None,
            github_app_id: None,
            github_app_private_key: None,
            github_app_installation_id: None,
            github_app_login: None,
            home_dir: PathBuf::from("/tmp"),
            spirit_log_file: None,
            dashboard_enabled: false,
            disable_bundled_skills: false,
            dev_mode: false,
            disable_agent_provisioning: false,
            telemetry_enabled: false,
            otlp_endpoint: None,
            otlp_auth_header: None,
            max_agent_tasks_per_session: 25,
            store_llm_calls: true,
            store_tool_calls: true,
            log_llm_bodies: false,
            kg_ingestion_model: None,
            kg_extraction_model: None,
            kg_resolution_model: None,
            kg_batch_budget: None,
            callback_watchdog_grace_period_secs: None,
            kg_docs_root: None,
            kg_docs_roots: None,
            operational_partner: false,
            decision_authority: DecisionAuthority::Strict,
            permission_hold_timeout_secs: DEFAULT_PERMISSION_HOLD_TIMEOUT_SECS,
        }
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
            .field(
                "kimi_api_key",
                &self.kimi_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "qwen_api_key",
                &self.qwen_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "zai_api_key",
                &self.zai_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("deepseek_base_url", &self.deepseek_base_url)
            // Non-provider
            .field("db_path", &self.db_path)
            .field("log_level", &self.log_level)
            .field("log_format", &self.log_format)
            .field("routing_url", &self.routing_url)
            .field("customer_id", &self.customer_id)
            .field("spirit_port", &self.spirit_port)
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
            .field("spirit_log_file", &self.spirit_log_file)
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
            .field("kg_docs_root", &self.kg_docs_root)
            .field("kg_docs_roots", &self.kg_docs_roots)
            .field("operational_partner", &self.operational_partner)
            .field("decision_authority", &self.decision_authority)
            .field(
                "permission_hold_timeout_secs",
                &self.permission_hold_timeout_secs,
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
            std::env::remove_var("MIKA_KG_DOCS_ROOT");
            std::env::remove_var("MIKA_KG_DOCS_ROOTS");
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
        settings.github_token = Some("github_pat_test_value".to_string().into());

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

    #[test]
    #[serial]
    fn test_api_keys_deserialize_as_secret_string() {
        clean_env();

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            r#"anthropic_api_key = "sk-ant-test-key-123""#,
        )
        .unwrap();

        let settings = Settings::load(tmp.path()).unwrap();
        let key = settings
            .anthropic_api_key
            .as_ref()
            .expect("key should be set");
        assert_eq!(key.expose_secret(), "sk-ant-test-key-123");
    }

    #[test]
    #[serial]
    fn test_get_effective_value_returns_set_for_secrets() {
        clean_env();

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            concat!(
                "anthropic_api_key = \"sk-ant-secret\"\n",
                "openai_api_key = \"sk-openai-secret\"\n",
                "brave_api_key = \"BSA-secret\"\n",
            ),
        )
        .unwrap();

        let settings = Settings::load(tmp.path()).unwrap();

        // All secret fields should return "[SET]", not the raw value
        assert_eq!(
            get_effective_value("anthropic_api_key", &settings),
            Some("[SET]".to_string())
        );
        assert_eq!(
            get_effective_value("openai_api_key", &settings),
            Some("[SET]".to_string())
        );
        assert_eq!(
            get_effective_value("brave_api_key", &settings),
            Some("[SET]".to_string())
        );

        // Unset secret fields should return None
        assert_eq!(get_effective_value("groq_api_key", &settings), None);
        assert_eq!(get_effective_value("github_token", &settings), None);
    }

    #[test]
    #[serial]
    fn test_debug_does_not_leak_secrets() {
        clean_env();

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            concat!(
                "anthropic_api_key = \"sk-ant-LEAKED\"\n",
                "brave_api_key = \"BSA-LEAKED\"\n",
                "github_token = \"ghp_LEAKED\"\n",
            ),
        )
        .unwrap();

        let settings = Settings::load(tmp.path()).unwrap();
        let debug_output = format!("{:?}", settings);

        assert!(!debug_output.contains("sk-ant-LEAKED"));
        assert!(!debug_output.contains("BSA-LEAKED"));
        assert!(!debug_output.contains("ghp_LEAKED"));
        assert!(debug_output.contains("[REDACTED]"));
    }

    #[test]
    #[serial]
    fn test_provider_fields_exposes_secret_correctly() {
        clean_env();

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            concat!(
                "anthropic_api_key = \"sk-ant-provider-test\"\n",
                "anthropic_model = \"claude-sonnet-4-20250514\"\n",
            ),
        )
        .unwrap();

        let settings = Settings::load(tmp.path()).unwrap();
        let (model, api_key, _base_url) = settings.provider_fields(ProviderKind::Anthropic);

        assert_eq!(model, Some("claude-sonnet-4-20250514"));
        assert_eq!(api_key, Some("sk-ant-provider-test"));
    }

    // -- KG batch budget (#757) --

    #[test]
    #[serial]
    fn kg_batch_budget_defaults_to_500() {
        clean_env();
        unsafe { std::env::remove_var("MIKA_KG_BATCH_BUDGET") };

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();

        assert_eq!(settings.kg_batch_budget, None);
        assert_eq!(
            settings.effective_kg_batch_budget(),
            DEFAULT_KG_BATCH_BUDGET
        );
        assert_eq!(settings.effective_kg_batch_budget(), 500);
    }

    #[test]
    #[serial]
    fn kg_batch_budget_env_override() {
        clean_env();
        // Safety: test-only env var.
        unsafe { std::env::set_var("MIKA_KG_BATCH_BUDGET", "100") };

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();

        assert_eq!(settings.kg_batch_budget, Some(100));
        assert_eq!(settings.effective_kg_batch_budget(), 100);

        unsafe { std::env::remove_var("MIKA_KG_BATCH_BUDGET") };
    }

    #[test]
    #[serial]
    fn kg_batch_budget_zero_disables_phase() {
        clean_env();
        // Safety: test-only env var.
        unsafe { std::env::set_var("MIKA_KG_BATCH_BUDGET", "0") };

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();

        // 0 is a valid configured value meaning "disable" — extractor/resolver
        // honor this as an immediate-return signal (verified in their own tests).
        assert_eq!(settings.kg_batch_budget, Some(0));
        assert_eq!(settings.effective_kg_batch_budget(), 0);

        unsafe { std::env::remove_var("MIKA_KG_BATCH_BUDGET") };
    }

    #[test]
    #[serial]
    fn kg_docs_root_defaults_to_none() {
        clean_env();

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();

        assert_eq!(settings.kg_docs_root, None);
    }

    #[test]
    #[serial]
    fn kg_docs_root_env_override() {
        clean_env();
        // Safety: test-only env var.
        unsafe { std::env::set_var("MIKA_KG_DOCS_ROOT", "/srv/mika/docs/solutions") };

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();

        assert_eq!(
            settings.kg_docs_root,
            Some(PathBuf::from("/srv/mika/docs/solutions"))
        );

        unsafe { std::env::remove_var("MIKA_KG_DOCS_ROOT") };
    }

    #[test]
    #[serial]
    fn kg_docs_root_config_file() {
        clean_env();

        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        std::fs::write(
            &config_path,
            "kg_docs_root = \"/opt/mika/docs/solutions\"\n",
        )
        .unwrap();

        let settings = Settings::load(tmp.path()).unwrap();

        assert_eq!(
            settings.kg_docs_root,
            Some(PathBuf::from("/opt/mika/docs/solutions"))
        );
    }

    #[test]
    #[serial]
    fn kg_docs_root_env_wins_over_config() {
        clean_env();
        // Safety: test-only env var.
        unsafe { std::env::set_var("MIKA_KG_DOCS_ROOT", "/env/path") };

        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        std::fs::write(&config_path, "kg_docs_root = \"/config/path\"\n").unwrap();

        let settings = Settings::load(tmp.path()).unwrap();

        // Env var takes precedence over config file via config-rs cascade.
        assert_eq!(settings.kg_docs_root, Some(PathBuf::from("/env/path")));

        unsafe { std::env::remove_var("MIKA_KG_DOCS_ROOT") };
    }

    #[test]
    #[serial]
    fn kg_docs_root_get_effective_value() {
        clean_env();
        // Safety: test-only env var.
        unsafe { std::env::set_var("MIKA_KG_DOCS_ROOT", "/effective/path") };

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();

        assert_eq!(
            get_effective_value("kg_docs_root", &settings),
            Some("/effective/path".to_string())
        );

        unsafe { std::env::remove_var("MIKA_KG_DOCS_ROOT") };
    }

    // -- kg_docs_roots colon-separated env var parsing (#814) --

    #[test]
    #[serial]
    fn kg_docs_roots_env_var_colon_separated() {
        clean_env();
        // Safety: test-only env var.
        unsafe { std::env::set_var("MIKA_KG_DOCS_ROOTS", "/a:/b:/c") };

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();

        let roots = settings
            .kg_docs_roots
            .expect("should parse colon-separated env var");
        assert_eq!(
            roots,
            vec![
                PathBuf::from("/a"),
                PathBuf::from("/b"),
                PathBuf::from("/c")
            ]
        );

        unsafe { std::env::remove_var("MIKA_KG_DOCS_ROOTS") };
    }

    #[test]
    #[serial]
    fn kg_docs_roots_env_var_single_path() {
        clean_env();
        // Safety: test-only env var.
        unsafe { std::env::set_var("MIKA_KG_DOCS_ROOTS", "/single") };

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();

        let roots = settings.kg_docs_roots.expect("should parse single path");
        assert_eq!(roots, vec![PathBuf::from("/single")]);

        unsafe { std::env::remove_var("MIKA_KG_DOCS_ROOTS") };
    }

    #[test]
    #[serial]
    fn kg_docs_roots_env_var_empty_string_is_none() {
        clean_env();
        // Safety: test-only env var.
        unsafe { std::env::set_var("MIKA_KG_DOCS_ROOTS", "") };

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();
        assert!(
            settings.kg_docs_roots.is_none(),
            "empty string should yield None"
        );

        unsafe { std::env::remove_var("MIKA_KG_DOCS_ROOTS") };
    }

    #[test]
    #[serial]
    fn kg_docs_roots_env_var_consecutive_colons_filtered() {
        clean_env();
        // Safety: test-only env var.
        unsafe { std::env::set_var("MIKA_KG_DOCS_ROOTS", "/a::/b") };

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();

        let roots = settings
            .kg_docs_roots
            .expect("should filter empty segments");
        assert_eq!(roots, vec![PathBuf::from("/a"), PathBuf::from("/b")]);

        unsafe { std::env::remove_var("MIKA_KG_DOCS_ROOTS") };
    }

    #[test]
    #[serial]
    fn kg_docs_roots_env_var_trailing_colon_filtered() {
        clean_env();
        // Safety: test-only env var.
        unsafe { std::env::set_var("MIKA_KG_DOCS_ROOTS", "/a:/b:") };

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();

        let roots = settings
            .kg_docs_roots
            .expect("should filter trailing colon");
        assert_eq!(roots, vec![PathBuf::from("/a"), PathBuf::from("/b")]);

        unsafe { std::env::remove_var("MIKA_KG_DOCS_ROOTS") };
    }

    #[test]
    #[serial]
    fn kg_docs_roots_toml_array() {
        clean_env();

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            "kg_docs_roots = [\"/x\", \"/y\"]\n",
        )
        .unwrap();

        let settings = Settings::load(tmp.path()).unwrap();

        let roots = settings.kg_docs_roots.expect("TOML array should parse");
        assert_eq!(roots, vec![PathBuf::from("/x"), PathBuf::from("/y")]);
    }

    #[test]
    #[serial]
    fn kg_docs_roots_env_overrides_toml() {
        clean_env();
        // Safety: test-only env var.
        unsafe { std::env::set_var("MIKA_KG_DOCS_ROOTS", "/env/a:/env/b") };

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            "kg_docs_roots = [\"/toml/x\", \"/toml/y\"]\n",
        )
        .unwrap();

        let settings = Settings::load(tmp.path()).unwrap();

        let roots = settings.kg_docs_roots.expect("env should override TOML");
        assert_eq!(
            roots,
            vec![PathBuf::from("/env/a"), PathBuf::from("/env/b")]
        );

        unsafe { std::env::remove_var("MIKA_KG_DOCS_ROOTS") };
    }

    #[test]
    #[serial]
    fn kg_docs_roots_unset_is_none() {
        clean_env();

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();
        assert!(settings.kg_docs_roots.is_none(), "unset should yield None");
    }

    #[test]
    #[serial]
    fn kg_docs_roots_get_effective_value() {
        clean_env();
        // Safety: test-only env var.
        unsafe { std::env::set_var("MIKA_KG_DOCS_ROOTS", "/x:/y:/z") };

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();

        assert_eq!(
            get_effective_value("kg_docs_roots", &settings),
            Some("/x:/y:/z".to_string())
        );

        unsafe { std::env::remove_var("MIKA_KG_DOCS_ROOTS") };
    }

    #[test]
    #[serial]
    fn kg_docs_roots_toml_empty_array_is_none() {
        clean_env();

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("config.toml"), "kg_docs_roots = []\n").unwrap();

        let settings = Settings::load(tmp.path()).unwrap();
        assert!(
            settings.kg_docs_roots.is_none(),
            "empty TOML array should yield None"
        );
    }

    #[test]
    #[serial]
    fn kg_docs_roots_four_element_env_var() {
        clean_env();
        // Safety: test-only env var.
        unsafe { std::env::set_var("MIKA_KG_DOCS_ROOTS", "/a:/b:/c:/d") };

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();

        let roots = settings
            .kg_docs_roots
            .expect("4-element env var should parse");
        assert_eq!(roots.len(), 4);
        assert_eq!(roots[0], PathBuf::from("/a"));
        assert_eq!(roots[3], PathBuf::from("/d"));

        unsafe { std::env::remove_var("MIKA_KG_DOCS_ROOTS") };
    }

    // -- Callback watchdog grace period (#959) --

    #[test]
    #[serial]
    fn callback_watchdog_grace_period_defaults_to_120() {
        clean_env();
        unsafe { std::env::remove_var("MIKA_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS") };

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();

        assert_eq!(settings.callback_watchdog_grace_period_secs, None);
        assert_eq!(
            settings.effective_callback_watchdog_grace_period_secs(),
            DEFAULT_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS
        );
        assert_eq!(
            settings.effective_callback_watchdog_grace_period_secs(),
            120
        );
    }

    #[test]
    #[serial]
    fn callback_watchdog_grace_period_env_override() {
        clean_env();
        // Safety: test-only env var.
        unsafe { std::env::set_var("MIKA_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS", "60") };

        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();

        assert_eq!(settings.callback_watchdog_grace_period_secs, Some(60));
        assert_eq!(settings.effective_callback_watchdog_grace_period_secs(), 60);

        unsafe { std::env::remove_var("MIKA_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS") };
    }
}

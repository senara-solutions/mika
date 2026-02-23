use config::{Config, Environment, File};
use serde::Deserialize;

#[derive(Deserialize, Clone)]
pub struct Settings {
    /// Anthropic API key
    pub anthropic_api_key: String,

    /// Claude model ID (default: claude-sonnet-4-5-20250514)
    #[serde(default = "default_claude_model")]
    pub claude_model: String,

    /// Max tokens for Claude responses
    #[serde(default = "default_max_tokens")]
    pub claude_max_tokens: u32,

    /// AES-256-GCM encryption key (hex-encoded, 32 bytes = 64 hex chars)
    pub encryption_key: String,

    /// SQLite database path
    #[serde(default = "default_db_path")]
    pub db_path: String,

    /// Log level
    #[serde(default = "default_log_level")]
    pub log_level: String,

    /// Routing layer URL (for outbound messages from agent container)
    #[serde(default)]
    pub routing_url: Option<String>,

    /// Customer ID (set per container)
    #[serde(default)]
    pub customer_id: Option<String>,
}

fn default_claude_model() -> String {
    "claude-sonnet-4-5-20250514".to_string()
}

fn default_max_tokens() -> u32 {
    4096
}

fn default_db_path() -> String {
    "mika.db".to_string()
}

fn default_log_level() -> String {
    "info".to_string()
}

impl Settings {
    /// Load settings from config file + environment variables.
    /// Environment variables are prefixed with MIKA_ (e.g., MIKA_ANTHROPIC_API_KEY).
    /// Note: field underscores map directly (no nested separator).
    pub fn load() -> anyhow::Result<Self> {
        let settings = Config::builder()
            .add_source(File::with_name("config/default").required(false))
            .add_source(File::with_name("config/local").required(false))
            .add_source(Environment::with_prefix("MIKA"))
            .build()?
            .try_deserialize()?;
        Ok(settings)
    }
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("anthropic_api_key", &"[REDACTED]")
            .field("claude_model", &self.claude_model)
            .field("claude_max_tokens", &self.claude_max_tokens)
            .field("encryption_key", &"[REDACTED]")
            .field("db_path", &self.db_path)
            .field("log_level", &self.log_level)
            .field("routing_url", &self.routing_url)
            .field("customer_id", &self.customer_id)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        // Safety: test is single-threaded; no other thread reads these env vars.
        unsafe {
            std::env::set_var("MIKA_ANTHROPIC_API_KEY", "test-key");
            std::env::set_var("MIKA_ENCRYPTION_KEY", "0".repeat(64));
        }

        let settings = Settings::load().unwrap();
        assert_eq!(settings.claude_model, "claude-sonnet-4-5-20250514");
        assert_eq!(settings.claude_max_tokens, 4096);
        assert_eq!(settings.db_path, "mika.db");
        assert_eq!(settings.log_level, "info");
    }
}

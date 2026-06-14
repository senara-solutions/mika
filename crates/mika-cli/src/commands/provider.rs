use std::path::Path;

use anyhow::{Result, bail};
use serde::Serialize;

use mika_common::config::Settings;
use mika_common::llm::ProviderKind;
use mika_common::llm::models::ModelInfo;

use crate::cli::OutputFormat;

// ── Structured return types ──────────────────────────────────────────

#[derive(Serialize)]
pub struct ProviderInfo {
    pub name: String,
    pub model: String,
    pub current: bool,
}

#[derive(Serialize)]
pub struct ProviderSwitchResult {
    pub provider: String,
    /// The bare model name (no provider prefix).
    pub model: String,
    pub warnings: Vec<String>,
}

#[derive(Serialize)]
pub struct ProviderSetResult {
    pub field: String,
    pub value: String,
}

// ── Shared logic ─────────────────────────────────────────────────────

/// List all providers with the user's configured model (or default).
pub fn list_providers(
    global_home: &Path,
    home_dir: &Path,
    current: ProviderKind,
) -> Vec<ProviderInfo> {
    let settings = Settings::load_for_agent(global_home, home_dir).ok();
    ProviderKind::ALL
        .iter()
        .map(|&p| {
            let model = settings
                .as_ref()
                .and_then(|s| s.provider_fields(p).0.map(String::from))
                .unwrap_or_else(|| p.default_model().to_string());
            ProviderInfo {
                name: p.to_string(),
                model,
                current: p == current,
            }
        })
        .collect()
}

/// Pre-validate that switching to a specific provider will work (API key present, etc.).
/// Returns the loaded settings (with `llm_provider` set to the target) on success,
/// so the caller can read `active_llm_config()` without a second load.
pub fn validate_provider_switch(
    global_home: &Path,
    home_dir: &Path,
    provider: ProviderKind,
) -> Result<Settings, String> {
    let mut settings = Settings::load_for_agent(global_home, home_dir)
        .map_err(|e| format!("failed to load settings: {e}"))?;
    settings.llm_provider = provider;
    settings.make_llm_provider().map_err(|e| e.to_string())?;
    Ok(settings)
}

/// Switch the active provider. Validates, persists to config.toml, and returns
/// structured result with the resolved model and any warnings.
///
/// When `prefetch_models` is true, fetches the model list synchronously (for CLI).
/// When false, skips the fetch (TUI does its own fire-and-forget spawn).
pub async fn switch_provider(
    global_home: &Path,
    home_dir: &Path,
    target: ProviderKind,
    prefetch_models: bool,
) -> Result<ProviderSwitchResult> {
    // Load current settings to check current provider
    let current_settings = Settings::load_for_agent(global_home, home_dir)?;
    let old_provider = current_settings.llm_provider;

    if target == old_provider {
        bail!("Already using {target}.");
    }

    // Pre-validate
    let settings = validate_provider_switch(global_home, home_dir, target)
        .map_err(|e| anyhow::anyhow!("Cannot switch to {target}: {e}"))?;

    let config = settings.active_llm_config();
    let model = config.model.clone();
    let had_configured_model = settings.provider_fields(target).0.is_some();

    let config_path = home_dir.join("config.toml");
    let mut warnings = Vec::new();

    // Persist provider switch
    super::config::write_config_toml(&config_path, "llm_provider", &target.to_string())?;

    // Persist default model when no {provider}_model was previously configured
    if !had_configured_model {
        let model_key = format!("{}_model", target.config_prefix());
        if let Err(e) = super::config::write_config_toml(&config_path, &model_key, &model) {
            tracing::warn!(error = %e, "failed to persist default model to config.toml");
        }
    }

    // Warn if llm_max_tokens exceeds the new provider's limit
    let max = target.max_output_tokens();
    if settings.llm_max_tokens > max {
        warnings.push(format!(
            "llm_max_tokens ({}) exceeds {target}'s limit ({max})",
            settings.llm_max_tokens
        ));
    }

    // Pre-fetch model list synchronously when requested (CLI mode — process exits after)
    if prefetch_models {
        eprintln!("Fetching models for {target}...");
        let _ = mika_common::llm::models::get_models(
            home_dir,
            target,
            config.base_url.as_deref(),
            config.api_key.as_deref(),
        )
        .await;
    }

    Ok(ProviderSwitchResult {
        provider: target.to_string(),
        model,
        warnings,
    })
}

/// Set a provider config field (model, api_key, base_url).
///
/// For `api_key`, the `value` parameter is written to the global `.env`.
/// For `model` and `base_url`, the value is written to `config.toml`.
pub fn set_provider_field(
    global_home: &Path,
    home_dir: &Path,
    field: &str,
    value: &str,
) -> Result<ProviderSetResult> {
    let settings = Settings::load_for_agent(global_home, home_dir)?;
    let provider = settings.llm_provider;
    let prefix = provider.config_prefix();
    let config_path = home_dir.join("config.toml");

    match field {
        "model" => {
            let key = format!("{prefix}_model");
            validate_provider_switch(global_home, home_dir, provider)
                .map_err(|e| anyhow::anyhow!("Cannot use model {value}: {e}"))?;
            super::config::write_config_toml(&config_path, &key, value)?;
            Ok(ProviderSetResult {
                field: key,
                value: value.to_string(),
            })
        }
        "base_url" | "base-url" => {
            let key = format!("{prefix}_base_url");
            super::config::write_config_toml(&config_path, &key, value)?;
            Ok(ProviderSetResult {
                field: key,
                value: value.to_string(),
            })
        }
        "api_key" | "api-key" => {
            let env_key = format!("MIKA_{}_API_KEY", prefix.to_uppercase());
            mika_common::dotenv::set_env_var(global_home, &env_key, value)?;
            Ok(ProviderSetResult {
                field: env_key,
                value: "set".to_string(),
            })
        }
        _ => bail!("Unknown field: {field}. Use: model, api-key, base-url"),
    }
}

// ── Model list helpers ───────────────────────────────────────────────

/// Fetch model list for a provider (from cache or API).
pub async fn fetch_models(
    global_home: &Path,
    home_dir: &Path,
    provider: ProviderKind,
) -> Vec<ModelInfo> {
    let settings = Settings::load_for_agent(global_home, home_dir).ok();
    let (api_key, base_url) = settings
        .as_ref()
        .map(|s| {
            let (_, key, url) = s.provider_fields(provider);
            (key.map(String::from), url.map(String::from))
        })
        .unwrap_or((None, None));

    mika_common::llm::models::get_models(
        home_dir,
        provider,
        base_url.as_deref(),
        api_key.as_deref(),
    )
    .await
}

// ── Display helpers (shared between TUI and CLI) ─────────────────────

/// Format model string for the agent worker — always includes provider prefix
/// so the worker can switch both provider and model atomically. See #451.
pub fn format_worker_model(provider: ProviderKind, model: &str) -> String {
    format!("{provider}/{model}")
}

/// Format model string for display — no prefix for Anthropic (matches
/// `Settings::active_model_display()` convention).
pub fn format_display_model(provider: ProviderKind, model: &str) -> String {
    if provider == ProviderKind::Anthropic {
        model.to_string()
    } else {
        format!("{provider}/{model}")
    }
}

// ── CLI run function ─────────────────────────────────────────────────

pub async fn run(args: crate::cli::ProviderArgs, agent_name: &str) -> Result<()> {
    let global_home = mika_common::home::resolve_home_dir()?;
    let home_dir = mika_common::agent::agent_dir(&global_home, agent_name);
    let settings = Settings::load_for_agent(&global_home, &home_dir)?;
    let current_provider = settings.llm_provider;

    // Handle subcommand first (provider set ...)
    if let Some(crate::cli::ProviderSubcommand::Set { field, value }) = args.command {
        use crate::cli::ProviderField;

        match field {
            ProviderField::ApiKey => {
                // Security: never accept API key as CLI arg — always prompt
                if value.is_some() {
                    bail!(
                        "API key must not be passed as a CLI argument (shell history risk). \
                         Omit the value to use the interactive prompt."
                    );
                }
                use std::io::IsTerminal;
                if !std::io::stdin().is_terminal() {
                    bail!(
                        "Cannot set API key non-interactively. \
                         Use MIKA_{}_API_KEY environment variable instead.",
                        current_provider.config_prefix().to_uppercase()
                    );
                }
                let password = dialoguer::Password::new()
                    .with_prompt(format!("Enter API key for {}", current_provider))
                    .interact()?;
                let result = set_provider_field(&global_home, &home_dir, "api_key", &password)?;
                match args.format {
                    OutputFormat::Json => {
                        println!("{}", serde_json::to_string_pretty(&result)?);
                    }
                    OutputFormat::Yaml => {
                        print!("{}", serde_yaml::to_string(&result)?);
                    }
                    OutputFormat::Text => {
                        println!(
                            "Set {} in .env (restart required for changes to take effect)",
                            result.field
                        );
                    }
                }
            }
            ProviderField::Model => {
                let value = value
                    .ok_or_else(|| anyhow::anyhow!("Usage: mika provider set model <value>"))?;
                let result = set_provider_field(&global_home, &home_dir, "model", &value)?;
                match args.format {
                    OutputFormat::Json => {
                        println!("{}", serde_json::to_string_pretty(&result)?);
                    }
                    OutputFormat::Yaml => {
                        print!("{}", serde_yaml::to_string(&result)?);
                    }
                    OutputFormat::Text => {
                        println!("Set {} = \"{}\"", result.field, result.value);
                    }
                }
            }
            ProviderField::BaseUrl => {
                let value = value
                    .ok_or_else(|| anyhow::anyhow!("Usage: mika provider set base-url <value>"))?;
                let result = set_provider_field(&global_home, &home_dir, "base_url", &value)?;
                match args.format {
                    OutputFormat::Json => {
                        println!("{}", serde_json::to_string_pretty(&result)?);
                    }
                    OutputFormat::Yaml => {
                        print!("{}", serde_yaml::to_string(&result)?);
                    }
                    OutputFormat::Text => {
                        println!("Set {} = \"{}\"", result.field, result.value);
                    }
                }
            }
        }
        return Ok(());
    }

    // No subcommand — either list or switch
    match args.name {
        None => {
            let providers = list_providers(&global_home, &home_dir, current_provider);
            match args.format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&providers)?);
                }
                OutputFormat::Yaml => {
                    print!("{}", serde_yaml::to_string(&providers)?);
                }
                OutputFormat::Text => {
                    println!("Current provider: {current_provider}\n\nAvailable providers:");
                    for p in &providers {
                        let marker = if p.current { " (current)" } else { "" };
                        println!("  {} — {}{}", p.name, p.model, marker);
                    }
                    println!("\nUsage: mika provider <name>");
                }
            }
        }
        Some(name) => {
            let target: ProviderKind = name.parse().map_err(|e: String| anyhow::anyhow!("{e}"))?;
            let result = switch_provider(&global_home, &home_dir, target, true).await?;
            match args.format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Yaml => {
                    print!("{}", serde_yaml::to_string(&result)?);
                }
                OutputFormat::Text => {
                    println!("Switched to {} (model: {}).", result.provider, result.model);
                    for w in &result.warnings {
                        println!("  {w}");
                    }
                }
            }
        }
    }

    Ok(())
}

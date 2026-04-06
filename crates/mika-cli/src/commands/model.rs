use std::path::Path;

use anyhow::{Result, bail};
use serde::Serialize;

use mika_common::config::Settings;
use mika_common::llm::ProviderKind;

use crate::cli::{MODEL_ALIASES, OutputFormat};

// ── Structured return types ──────────────────────────────────────────

#[derive(Serialize)]
pub struct ModelListEntry {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub current: bool,
}

#[derive(Serialize)]
pub struct ModelListResult {
    pub provider: String,
    pub current_model: String,
    pub models: Vec<ModelListEntry>,
    pub aliases: Vec<AliasEntry>,
}

#[derive(Serialize)]
pub struct AliasEntry {
    pub alias: String,
    pub model_id: String,
    pub display: String,
}

#[derive(Serialize)]
pub struct ModelSwitchResult {
    pub provider: String,
    /// The bare model name (no provider prefix).
    pub model: String,
    pub previous_model: String,
    pub provider_changed: bool,
    pub warnings: Vec<String>,
}

// ── Shared logic ─────────────────────────────────────────────────────

/// List models for the current provider with aliases.
pub async fn list_models(global_home: &Path, home_dir: &Path) -> Result<ModelListResult> {
    let settings = Settings::load_for_agent(global_home, home_dir)?;
    let provider = settings.llm_provider;
    let config = settings.active_llm_config();
    let current_model = config.model.clone();

    let models = super::provider::fetch_models(global_home, home_dir, provider).await;

    // Determine current model name (strip provider prefix if present)
    let current_bare = current_model
        .split('/')
        .next_back()
        .unwrap_or(&current_model);

    let model_entries: Vec<ModelListEntry> = models
        .iter()
        .map(|m| ModelListEntry {
            id: m.id.clone(),
            name: m.name.clone(),
            current: m.id == current_bare,
        })
        .collect();

    let aliases: Vec<AliasEntry> = MODEL_ALIASES
        .iter()
        .map(|&(alias, full_id, display)| AliasEntry {
            alias: alias.to_string(),
            model_id: full_id.to_string(),
            display: display.to_string(),
        })
        .collect();

    Ok(ModelListResult {
        provider: provider.to_string(),
        current_model: super::provider::format_display_model(provider, &current_model),
        models: model_entries,
        aliases,
    })
}

/// Switch to a model by name (supports aliases, provider/model format, and direct names).
///
/// Returns the bare model name in `result.model` — callers use `format_display_model()`
/// and `format_worker_model()` for their respective needs.
pub fn switch_model(global_home: &Path, home_dir: &Path, input: &str) -> Result<ModelSwitchResult> {
    let settings = Settings::load_for_agent(global_home, home_dir)?;
    let current_provider = settings.llm_provider;
    let current_config = settings.active_llm_config();
    let previous_model = current_config.model.clone();

    // Try resolving as an alias
    let lower = input.to_lowercase();
    for &(alias, full_id, display) in MODEL_ALIASES {
        if lower == alias || lower == full_id {
            let (target_provider, model_name) = parse_provider_model(full_id, current_provider);

            if target_provider == current_provider && model_name == previous_model {
                bail!("Already using {display}.");
            }

            super::provider::validate_provider_switch(global_home, home_dir, target_provider)
                .map_err(|e| anyhow::anyhow!("Cannot switch to {display}: {e}"))?;

            return apply_switch(
                home_dir,
                current_provider,
                target_provider,
                model_name,
                &previous_model,
            );
        }
    }

    // Not an alias — parse provider/model format or treat as direct name.
    // If current provider uses slash-separated model names (e.g., OpenRouter uses
    // "qwen/qwen-plus"), don't interpret the slash as a cross-provider switch.
    let (target_provider, model_name) = parse_provider_model(input, current_provider);

    let display_id = super::provider::format_display_model(target_provider, model_name);
    let current_display = super::provider::format_display_model(current_provider, &previous_model);
    if display_id == current_display {
        bail!("Already using {input}.");
    }

    super::provider::validate_provider_switch(global_home, home_dir, target_provider)
        .map_err(|e| anyhow::anyhow!("Cannot use model {input}: {e}"))?;

    apply_switch(
        home_dir,
        current_provider,
        target_provider,
        model_name,
        &previous_model,
    )
}

/// Parse a model input string into (provider, model_name).
/// Respects `model_names_contain_slash()` — e.g., OpenRouter models like
/// "qwen/qwen-plus" are NOT interpreted as cross-provider switches.
fn parse_provider_model(input: &str, default_provider: ProviderKind) -> (ProviderKind, &str) {
    // If current provider uses slashes in model names (e.g., OpenRouter),
    // or input has no slash, treat the whole input as the model name.
    if default_provider.model_names_contain_slash() || !input.contains('/') {
        return (default_provider, input);
    }
    let slash_pos = input.find('/').unwrap();
    let prefix = &input[..slash_pos];
    match prefix.parse::<ProviderKind>() {
        Ok(p) => (p, &input[slash_pos + 1..]),
        Err(_) => (default_provider, input),
    }
}

/// Apply a model switch: persist provider (if changed) and model to config.toml.
fn apply_switch(
    home_dir: &Path,
    old_provider: ProviderKind,
    target_provider: ProviderKind,
    model_name: &str,
    previous_model: &str,
) -> Result<ModelSwitchResult> {
    let config_path = home_dir.join("config.toml");
    let provider_changed = target_provider != old_provider;
    let mut warnings = Vec::new();

    // If targeting a different provider, switch the active provider too
    if provider_changed
        && let Err(e) = super::config::write_config_toml(
            &config_path,
            "llm_provider",
            &target_provider.to_string(),
        )
    {
        tracing::warn!(error = %e, "failed to persist provider switch to config.toml");
        warnings.push("failed to save provider — change won't survive restart".to_string());
    }

    // Persist model to config.toml
    let model_key = format!("{}_model", target_provider.config_prefix());
    if let Err(e) = super::config::write_config_toml(&config_path, &model_key, model_name) {
        tracing::warn!(error = %e, "failed to persist model to config.toml");
        warnings.push("failed to save model — change won't survive restart".to_string());
    }

    Ok(ModelSwitchResult {
        provider: target_provider.to_string(),
        model: model_name.to_string(),
        previous_model: previous_model.to_string(),
        provider_changed,
        warnings,
    })
}

// ── CLI run function ─────────────────────────────────────────────────

pub async fn run(args: crate::cli::ModelArgs, agent_name: &str) -> Result<()> {
    let global_home = mika_common::home::resolve_home_dir()?;
    let home_dir = mika_common::agent::agent_dir(&global_home, agent_name);

    match args.name {
        None => {
            let result = list_models(&global_home, &home_dir).await?;
            match args.format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Text => {
                    println!(
                        "Current model: {}\n\nAvailable models for {}:",
                        result.current_model, result.provider
                    );
                    if result.models.is_empty() {
                        println!("  (no models available — type a model name directly)");
                    } else {
                        for m in &result.models {
                            let marker = if m.current { " (current)" } else { "" };
                            println!("  {}{}", m.id, marker);
                        }
                    }
                    println!("\nAliases:");
                    for a in &result.aliases {
                        println!("  {} — {}", a.alias, a.display);
                    }
                    println!("\nUsage: mika model <name> or mika model <alias>");
                }
            }
        }
        Some(name) => {
            let result = switch_model(&global_home, &home_dir, &name)?;
            let display = super::provider::format_display_model(
                result.provider.parse().unwrap_or(ProviderKind::Anthropic),
                &result.model,
            );
            match args.format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Text => {
                    println!("Switched to {}.", display);
                    if result.provider_changed {
                        println!("  (switched provider to {})", result.provider);
                    }
                    for w in &result.warnings {
                        println!("  {w}");
                    }
                }
            }
        }
    }

    Ok(())
}

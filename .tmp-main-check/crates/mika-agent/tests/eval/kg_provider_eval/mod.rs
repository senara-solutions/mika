//! KG provider evaluation matrix (#762).
//!
//! Compares LLM providers on two KG tasks:
//! 1. Entity extraction (NER + fact triples from solution docs)
//! 2. Entity resolution (disambiguation against domain graph candidates)
//!
//! Run with:
//! ```bash
//! MIKA_EVAL_KG_PROVIDERS=default cargo test -p mika-agent --test eval kg_provider_eval -- --ignored --nocapture
//! ```
//!
//! Or specify providers explicitly:
//! ```bash
//! MIKA_EVAL_KG_PROVIDERS=anthropic/claude-haiku-4-5-20251001,openrouter/deepseek/deepseek-v3 \
//!   cargo test -p mika-agent --test eval kg_provider_eval -- --ignored --nocapture
//! ```

pub mod cost;
pub mod extraction_eval;
pub mod prompts;
pub mod report;
pub mod resolution_eval;
pub mod truncation_eval;

use std::sync::Arc;

use mika_common::llm::{LlmProvider, ModelSpec, ProviderKind, create_provider};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

/// Specification for a KG evaluation provider.
#[derive(Debug, Clone)]
pub struct KgProviderSpec {
    pub kind: ProviderKind,
    pub model: String,
    pub display_name: String,
}

/// Per-document extraction outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KgExtractionOutcome {
    pub provider: String,
    pub doc_path: String,
    pub category: String,
    pub entity_count: usize,
    pub relationship_count: usize,
    pub json_parse_ok: bool,
    /// Fraction of entities with valid names (lowercase + underscore + no colons).
    pub name_quality: f64,
    /// Fraction of entities with approved entity types.
    pub type_quality: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub latency_ms: u64,
    pub error: Option<String>,
}

/// Per-case resolution outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KgResolutionOutcome {
    pub provider: String,
    pub entity_key: String,
    pub case_category: String,
    pub correct: bool,
    pub expected_match: String,
    pub actual_match: Option<String>,
    pub confidence: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub latency_ms: u64,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Default provider set
// ---------------------------------------------------------------------------

/// Default providers for the KG eval matrix.
const DEFAULT_PROVIDERS: &[(&str, &str, &str)] = &[
    (
        "anthropic",
        "claude-haiku-4-5-20251001",
        "anthropic/claude-haiku-4-5-20251001",
    ),
    (
        "anthropic",
        "claude-sonnet-4-6",
        "anthropic/claude-sonnet-4-6",
    ),
    (
        "openrouter",
        "deepseek/deepseek-v3",
        "openrouter/deepseek/deepseek-v3",
    ),
    (
        "openrouter",
        "moonshotai/kimi-k2.5",
        "openrouter/moonshotai/kimi-k2.5",
    ),
];

// ---------------------------------------------------------------------------
// Provider parsing
// ---------------------------------------------------------------------------

/// Parse the `MIKA_EVAL_KG_PROVIDERS` environment variable into provider specs.
///
/// Accepts:
/// - `default` — the four-provider default set
/// - Comma-separated `provider/model` strings (e.g. `anthropic/claude-haiku-4-5-20251001,openrouter/deepseek/deepseek-v3`)
///
/// Returns an empty vec if the env var is unset or empty.
pub fn parse_kg_providers() -> Vec<KgProviderSpec> {
    let val = match std::env::var("MIKA_EVAL_KG_PROVIDERS") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => return Vec::new(),
    };

    let trimmed = val.trim();
    if trimmed.eq_ignore_ascii_case("default") {
        return DEFAULT_PROVIDERS
            .iter()
            .map(|(kind_str, model, display)| {
                let kind = parse_provider_kind(kind_str);
                KgProviderSpec {
                    kind,
                    model: model.to_string(),
                    display_name: display.to_string(),
                }
            })
            .collect();
    }

    trimmed
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| {
            let s = s.trim();
            // Format: provider/model (e.g. anthropic/claude-haiku-4-5-20251001)
            // For OpenRouter, model itself contains a slash: openrouter/deepseek/deepseek-v3
            let (kind_str, model) = s.split_once('/').unwrap_or_else(|| {
                panic!(
                    "MIKA_EVAL_KG_PROVIDERS: invalid format '{}' — expected 'provider/model'",
                    s
                )
            });
            let kind = parse_provider_kind(kind_str);
            KgProviderSpec {
                kind,
                model: model.to_string(),
                display_name: s.to_string(),
            }
        })
        .collect()
}

fn parse_provider_kind(s: &str) -> ProviderKind {
    match s.to_lowercase().as_str() {
        "anthropic" => ProviderKind::Anthropic,
        "openai" => ProviderKind::OpenAi,
        "openrouter" => ProviderKind::OpenRouter,
        "groq" => ProviderKind::Groq,
        "ollama" => ProviderKind::Ollama,
        "mistral" => ProviderKind::Mistral,
        "google" => ProviderKind::Google,
        "deepseek" => ProviderKind::DeepSeek,
        "minimax" => ProviderKind::MiniMax,
        "kimi" => ProviderKind::Kimi,
        "qwen" => ProviderKind::Qwen,
        other => panic!(
            "MIKA_EVAL_KG_PROVIDERS: unknown provider '{}'. Valid: anthropic, openai, openrouter, groq, ollama, mistral, google, deepseek, minimax, kimi, qwen",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Provider construction
// ---------------------------------------------------------------------------

/// Create an LLM provider from a KG provider spec.
///
/// Reads `MIKA_{PREFIX}_API_KEY` from the environment. Returns `None` if the
/// API key is not set (allowing tests to skip that provider gracefully).
pub fn create_kg_provider(spec: &KgProviderSpec) -> Option<Arc<dyn LlmProvider>> {
    let prefix = spec.kind.config_prefix().to_uppercase();
    let env_key = format!("MIKA_{}_API_KEY", prefix);
    let api_key = std::env::var(&env_key).ok();

    // For Ollama, API key is usually not needed
    if api_key.is_none() && spec.kind != ProviderKind::Ollama {
        eprintln!(
            "[kg_provider_eval] skipping {} — {} not set",
            spec.display_name, env_key
        );
        return None;
    }

    let model_spec = ModelSpec {
        provider: spec.kind,
        model: spec.model.clone(),
        base_url: spec.kind.default_base_url().map(String::from),
        api_key,
    };

    // 4096 max tokens is plenty for JSON extraction/resolution output.
    match create_provider(&model_spec, 4096, false) {
        Ok(provider) => Some(provider),
        Err(e) => {
            eprintln!(
                "[kg_provider_eval] failed to create provider {}: {}",
                spec.display_name, e
            );
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn kg_provider_eval_full() {
    let providers = parse_kg_providers();
    if providers.is_empty() {
        eprintln!(
            "[kg_provider_eval] MIKA_EVAL_KG_PROVIDERS not set — skipping. \
             Set to 'default' or 'anthropic/claude-haiku-4-5-20251001,openrouter/deepseek/deepseek-v3'"
        );
        return;
    }

    let mut active_providers = Vec::new();
    for spec in &providers {
        if let Some(provider) = create_kg_provider(spec) {
            active_providers.push((spec.clone(), provider));
        }
    }

    if active_providers.is_empty() {
        eprintln!("[kg_provider_eval] no providers have API keys configured — skipping");
        return;
    }

    println!(
        "\n=== KG Provider Eval: {} providers ===\n",
        active_providers.len()
    );

    // Run extraction eval
    let extraction_outcomes = extraction_eval::run_extraction_eval(&active_providers).await;

    // Run resolution eval
    let resolution_outcomes = resolution_eval::run_resolution_eval(&active_providers).await;

    // Generate report
    report::generate_report(
        &extraction_outcomes,
        &resolution_outcomes,
        &active_providers,
    );
}

#[tokio::test]
#[ignore]
async fn kg_provider_eval_extraction_only() {
    let providers = parse_kg_providers();
    if providers.is_empty() {
        eprintln!("[kg_provider_eval] MIKA_EVAL_KG_PROVIDERS not set — skipping");
        return;
    }

    let mut active_providers = Vec::new();
    for spec in &providers {
        if let Some(provider) = create_kg_provider(spec) {
            active_providers.push((spec.clone(), provider));
        }
    }

    if active_providers.is_empty() {
        eprintln!("[kg_provider_eval] no providers configured — skipping");
        return;
    }

    println!(
        "\n=== KG Extraction Eval: {} providers ===\n",
        active_providers.len()
    );

    let outcomes = extraction_eval::run_extraction_eval(&active_providers).await;
    report::print_extraction_summary(&outcomes);
}

#[tokio::test]
#[ignore]
async fn kg_provider_eval_resolution_only() {
    let providers = parse_kg_providers();
    if providers.is_empty() {
        eprintln!("[kg_provider_eval] MIKA_EVAL_KG_PROVIDERS not set — skipping");
        return;
    }

    let mut active_providers = Vec::new();
    for spec in &providers {
        if let Some(provider) = create_kg_provider(spec) {
            active_providers.push((spec.clone(), provider));
        }
    }

    if active_providers.is_empty() {
        eprintln!("[kg_provider_eval] no providers configured — skipping");
        return;
    }

    println!(
        "\n=== KG Resolution Eval: {} providers ===\n",
        active_providers.len()
    );

    let outcomes = resolution_eval::run_resolution_eval(&active_providers).await;
    report::print_resolution_summary(&outcomes);
}

#[tokio::test]
#[ignore]
async fn truncation_eval() {
    let providers = parse_kg_providers();
    if providers.is_empty() {
        eprintln!(
            "[truncation_eval] MIKA_EVAL_KG_PROVIDERS not set — skipping. \
             Set to 'default' or 'anthropic/claude-haiku-4-5-20251001,openrouter/deepseek/deepseek-v3'"
        );
        return;
    }

    let mut active_providers = Vec::new();
    for spec in &providers {
        if let Some(provider) = create_kg_provider(spec) {
            active_providers.push((spec.clone(), provider));
        }
    }

    if active_providers.is_empty() {
        eprintln!("[truncation_eval] no providers have API keys configured — skipping");
        return;
    }

    println!(
        "\n=== Truncation Comparison Eval (two-run protocol, mika#766 §1): {} providers ===\n",
        active_providers.len()
    );

    println!("--- Run 1 ---");
    let outcomes_run1 = truncation_eval::run_truncation_eval(&active_providers).await;
    truncation_eval::print_truncation_summary(&outcomes_run1);

    println!("\n--- Run 2 (confirmatory) ---");
    let outcomes_run2 = truncation_eval::run_truncation_eval(&active_providers).await;
    truncation_eval::print_truncation_summary(&outcomes_run2);

    truncation_eval::print_two_run_comparison(&outcomes_run1, &outcomes_run2);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_provider_kind() {
        assert_eq!(parse_provider_kind("anthropic"), ProviderKind::Anthropic);
        assert_eq!(parse_provider_kind("openrouter"), ProviderKind::OpenRouter);
        assert_eq!(parse_provider_kind("ANTHROPIC"), ProviderKind::Anthropic);
    }

    #[test]
    #[should_panic(expected = "unknown provider")]
    fn test_parse_provider_kind_unknown() {
        parse_provider_kind("unknown_provider");
    }

    #[test]
    fn test_default_providers_count() {
        assert_eq!(DEFAULT_PROVIDERS.len(), 4);
    }
}

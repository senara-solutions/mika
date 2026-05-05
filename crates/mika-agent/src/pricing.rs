//! LLM API cost estimation from token counts and model identity.
//!
//! Provides approximate pricing data for major LLM providers and a cost
//! estimation function. Prices are per million tokens and may drift from
//! actual provider pricing — treat as estimates for dashboard display.

/// Per-model pricing rates (USD per million tokens).
#[derive(Debug, Clone)]
pub struct ModelPricing {
    /// Cost per million input tokens.
    pub input_per_mtok: f64,
    /// Cost per million output tokens.
    pub output_per_mtok: f64,
    /// Multiplier applied to input rate for cache-read tokens (e.g. 0.1 for Anthropic).
    pub cache_read_multiplier: f64,
    /// Multiplier applied to input rate for cache-write tokens (e.g. 1.25 for Anthropic).
    pub cache_write_multiplier: f64,
}

/// Fallback pricing used when the model is not recognized.
const FALLBACK_PRICING: ModelPricing = ModelPricing {
    input_per_mtok: 1.0,
    output_per_mtok: 5.0,
    cache_read_multiplier: 1.0,
    cache_write_multiplier: 1.0,
};

/// Zero-cost pricing for local models (Ollama).
const ZERO_PRICING: ModelPricing = ModelPricing {
    input_per_mtok: 0.0,
    output_per_mtok: 0.0,
    cache_read_multiplier: 1.0,
    cache_write_multiplier: 1.0,
};

/// Anthropic cache multipliers.
const ANTHROPIC_CACHE_READ: f64 = 0.1;
const ANTHROPIC_CACHE_WRITE: f64 = 1.25;

struct PricingEntry {
    /// Model name prefixes to match (lowercase).
    models: &'static [&'static str],
    input_per_mtok: f64,
    output_per_mtok: f64,
}

const ANTHROPIC_MODELS: &[PricingEntry] = &[
    PricingEntry {
        models: &["claude-opus-4", "claude-opus-4-0520"],
        input_per_mtok: 15.0,
        output_per_mtok: 75.0,
    },
    PricingEntry {
        models: &[
            "claude-sonnet-4",
            "claude-sonnet-4-0514",
            "claude-sonnet-4-6",
            "claude-sonnet-4-6-20250514",
        ],
        input_per_mtok: 3.0,
        output_per_mtok: 15.0,
    },
    PricingEntry {
        models: &["claude-haiku-4-5", "claude-haiku-4-5-20251022"],
        input_per_mtok: 0.80,
        output_per_mtok: 4.0,
    },
];

const OPENAI_MODELS: &[PricingEntry] = &[
    PricingEntry {
        models: &["gpt-4o", "gpt-4o-2024-08-06"],
        input_per_mtok: 2.50,
        output_per_mtok: 10.0,
    },
    PricingEntry {
        models: &["gpt-4o-mini"],
        input_per_mtok: 0.15,
        output_per_mtok: 0.60,
    },
    PricingEntry {
        models: &["o3-mini"],
        input_per_mtok: 1.10,
        output_per_mtok: 4.40,
    },
];

const DEEPSEEK_MODELS: &[PricingEntry] = &[
    PricingEntry {
        models: &["deepseek-chat", "deepseek-v3"],
        input_per_mtok: 0.27,
        output_per_mtok: 1.10,
    },
    PricingEntry {
        models: &["deepseek-reasoner", "deepseek-r1"],
        input_per_mtok: 0.55,
        output_per_mtok: 2.19,
    },
];

const GOOGLE_MODELS: &[PricingEntry] = &[
    PricingEntry {
        models: &["gemini-2.5-flash"],
        input_per_mtok: 0.15,
        output_per_mtok: 0.60,
    },
    PricingEntry {
        models: &["gemini-2.5-pro"],
        input_per_mtok: 1.25,
        output_per_mtok: 10.0,
    },
];

const GROQ_MODELS: &[PricingEntry] = &[
    PricingEntry {
        models: &["llama-3.3-70b-versatile"],
        input_per_mtok: 0.59,
        output_per_mtok: 0.79,
    },
    PricingEntry {
        models: &["llama-3.1-8b-instant"],
        input_per_mtok: 0.05,
        output_per_mtok: 0.08,
    },
];

const MISTRAL_MODELS: &[PricingEntry] = &[
    PricingEntry {
        models: &["mistral-large-latest"],
        input_per_mtok: 2.0,
        output_per_mtok: 6.0,
    },
    PricingEntry {
        models: &["mistral-small-latest"],
        input_per_mtok: 0.10,
        output_per_mtok: 0.30,
    },
];

/// Provider name to pricing table mapping.
struct ProviderPricing {
    name: &'static str,
    entries: &'static [PricingEntry],
    cache_read_multiplier: f64,
    cache_write_multiplier: f64,
}

const PROVIDERS: &[ProviderPricing] = &[
    ProviderPricing {
        name: "anthropic",
        entries: ANTHROPIC_MODELS,
        cache_read_multiplier: ANTHROPIC_CACHE_READ,
        cache_write_multiplier: ANTHROPIC_CACHE_WRITE,
    },
    ProviderPricing {
        name: "openai",
        entries: OPENAI_MODELS,
        cache_read_multiplier: 1.0,
        cache_write_multiplier: 1.0,
    },
    ProviderPricing {
        name: "deepseek",
        entries: DEEPSEEK_MODELS,
        cache_read_multiplier: 1.0,
        cache_write_multiplier: 1.0,
    },
    ProviderPricing {
        name: "google",
        entries: GOOGLE_MODELS,
        cache_read_multiplier: 1.0,
        cache_write_multiplier: 1.0,
    },
    ProviderPricing {
        name: "groq",
        entries: GROQ_MODELS,
        cache_read_multiplier: 1.0,
        cache_write_multiplier: 1.0,
    },
    ProviderPricing {
        name: "mistral",
        entries: MISTRAL_MODELS,
        cache_read_multiplier: 1.0,
        cache_write_multiplier: 1.0,
    },
];

/// Look up pricing for a (provider, model) pair.
///
/// Normalizes provider prefixes (e.g. `openrouter/anthropic/` routes to
/// Anthropic pricing) and performs case-insensitive matching. Returns
/// [`FALLBACK_PRICING`] for unknown models.
pub fn get_pricing(provider: &str, model: &str) -> ModelPricing {
    let provider_lower = provider.to_lowercase();
    let model_lower = model.to_lowercase();

    // Ollama / local models are always zero cost.
    if provider_lower == "ollama" || provider_lower.starts_with("ollama/") {
        return ZERO_PRICING;
    }

    // Resolve effective provider. For compound providers like "openrouter",
    // try to extract the real provider from the model string (e.g.
    // "anthropic/claude-sonnet-4.6" → provider "anthropic", model "claude-sonnet-4.6").
    let (effective_provider, effective_model) =
        resolve_provider_and_model(&provider_lower, &model_lower);

    // Find the provider's pricing table.
    if let Some(pp) = PROVIDERS.iter().find(|pp| pp.name == effective_provider)
        && let Some(entry) = find_model_in_entries(pp.entries, effective_model)
    {
        return ModelPricing {
            input_per_mtok: entry.input_per_mtok,
            output_per_mtok: entry.output_per_mtok,
            cache_read_multiplier: pp.cache_read_multiplier,
            cache_write_multiplier: pp.cache_write_multiplier,
        };
    }

    FALLBACK_PRICING
}

/// Estimate the cost of a single LLM call in USD.
///
/// Formula:
/// ```text
/// effective_input = input_tokens - cache_read
/// cost = effective_input * input_rate
///      + cache_read * (input_rate * cache_read_multiplier)
///      + cache_write * (input_rate * cache_write_multiplier)
///      + output_tokens * output_rate
/// ```
///
/// All rates are per-million-token, so final result is divided by 1,000,000.
pub fn estimate_call_cost(
    pricing: &ModelPricing,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: Option<u64>,
    cache_write_tokens: Option<u64>,
) -> f64 {
    let cache_read = cache_read_tokens.unwrap_or(0) as f64;
    let cache_write = cache_write_tokens.unwrap_or(0) as f64;
    let input = input_tokens as f64;
    let output = output_tokens as f64;

    let effective_input = (input - cache_read).max(0.0);

    let cost = effective_input * pricing.input_per_mtok
        + cache_read * (pricing.input_per_mtok * pricing.cache_read_multiplier)
        + cache_write * (pricing.input_per_mtok * pricing.cache_write_multiplier)
        + output * pricing.output_per_mtok;

    cost / 1_000_000.0
}

/// Returns `true` if the pricing matches the fallback (unknown model).
pub fn is_fallback_pricing(pricing: &ModelPricing) -> bool {
    pricing.input_per_mtok == FALLBACK_PRICING.input_per_mtok
        && pricing.output_per_mtok == FALLBACK_PRICING.output_per_mtok
        && pricing.cache_read_multiplier == FALLBACK_PRICING.cache_read_multiplier
        && pricing.cache_write_multiplier == FALLBACK_PRICING.cache_write_multiplier
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Resolve a compound provider string into (effective_provider, effective_model).
///
/// Examples:
/// - ("openrouter", "anthropic/claude-sonnet-4.6") → ("anthropic", "claude-sonnet-4.6")
/// - ("openrouter", "deepseek/deepseek-chat") → ("deepseek", "deepseek-chat")
/// - ("anthropic", "claude-sonnet-4.6") → ("anthropic", "claude-sonnet-4.6")
fn resolve_provider_and_model<'a>(provider: &'a str, model: &'a str) -> (&'a str, &'a str) {
    // If the provider is a known direct provider, use it as-is.
    if PROVIDERS.iter().any(|pp| pp.name == provider) {
        return (provider, model);
    }

    // For compound providers (e.g. "openrouter"), try to extract the real
    // provider from the model string: "anthropic/claude-sonnet-4.6".
    if let Some(slash_pos) = model.find('/') {
        let candidate_provider = &model[..slash_pos];
        let remainder = &model[slash_pos + 1..];
        if PROVIDERS.iter().any(|pp| pp.name == candidate_provider) {
            return (candidate_provider, remainder);
        }
    }

    // Try the last segment of the provider itself (e.g. "openrouter/anthropic").
    if let Some(slash_pos) = provider.rfind('/') {
        let last_segment = &provider[slash_pos + 1..];
        if PROVIDERS.iter().any(|pp| pp.name == last_segment) {
            return (last_segment, model);
        }
    }

    (provider, model)
}

/// Find a model in a pricing table. Tries exact match first, then prefix match.
fn find_model_in_entries<'a>(entries: &'a [PricingEntry], model: &str) -> Option<&'a PricingEntry> {
    // Exact match.
    for entry in entries {
        for &name in entry.models {
            if name == model {
                return Some(entry);
            }
        }
    }

    // Prefix match: the model string starts with a known name.
    // This handles versioned suffixes like "claude-sonnet-4-6-20250514" matching
    // "claude-sonnet-4-6" and also dotted variants like "claude-sonnet-4.6".
    //
    // Sort candidates by length descending so longer (more specific) prefixes win.
    let mut candidates: Vec<(&PricingEntry, &str)> = Vec::new();
    for entry in entries {
        for &name in entry.models {
            if model.starts_with(name) {
                candidates.push((entry, name));
            }
        }
    }
    candidates.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    if let Some((entry, _)) = candidates.first() {
        return Some(entry);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_sonnet_46_basic_cost() {
        let pricing = get_pricing("anthropic", "claude-sonnet-4-6");
        assert_eq!(pricing.input_per_mtok, 3.0);
        assert_eq!(pricing.output_per_mtok, 15.0);

        let cost = estimate_call_cost(&pricing, 1000, 500, None, None);
        // (1000 * 3.0 + 500 * 15.0) / 1_000_000 = (3000 + 7500) / 1_000_000 = 0.0000105
        let expected = (1000.0 * 3.0 + 500.0 * 15.0) / 1_000_000.0;
        assert!(
            (cost - expected).abs() < 1e-12,
            "cost={cost}, expected={expected}"
        );
    }

    #[test]
    fn full_four_component_cost_with_cache() {
        let pricing = get_pricing("anthropic", "claude-sonnet-4-6");
        let cost = estimate_call_cost(&pricing, 1000, 500, Some(500), Some(200));
        // effective_input = 1000 - 500 = 500
        // cost = 500 * 3.0                       (regular input)
        //      + 500 * (3.0 * 0.1)               (cache read)
        //      + 200 * (3.0 * 1.25)              (cache write)
        //      + 500 * 15.0                       (output)
        //      = 1500 + 150 + 750 + 7500 = 9900
        // 9900 / 1_000_000 = 0.0000099
        let expected = (500.0 * 3.0 + 500.0 * 0.3 + 200.0 * 3.75 + 500.0 * 15.0) / 1_000_000.0;
        assert!(
            (cost - expected).abs() < 1e-12,
            "cost={cost}, expected={expected}"
        );
    }

    #[test]
    fn zero_tokens_zero_cost() {
        let pricing = get_pricing("anthropic", "claude-sonnet-4-6");
        let cost = estimate_call_cost(&pricing, 0, 0, None, None);
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn unknown_model_returns_fallback() {
        let pricing = get_pricing("unknown-provider", "some-custom-model");
        assert!(is_fallback_pricing(&pricing));
        assert_eq!(pricing.input_per_mtok, 1.0);
        assert_eq!(pricing.output_per_mtok, 5.0);
    }

    #[test]
    fn openrouter_prefix_resolves_to_anthropic() {
        let pricing = get_pricing("openrouter", "anthropic/claude-sonnet-4.6");
        assert_eq!(pricing.input_per_mtok, 3.0);
        assert_eq!(pricing.output_per_mtok, 15.0);
        assert_eq!(pricing.cache_read_multiplier, ANTHROPIC_CACHE_READ);
        assert!(!is_fallback_pricing(&pricing));
    }

    #[test]
    fn cache_read_none_treated_as_zero() {
        let pricing = get_pricing("anthropic", "claude-sonnet-4-6");
        let cost_none = estimate_call_cost(&pricing, 1000, 500, None, None);
        let cost_zero = estimate_call_cost(&pricing, 1000, 500, Some(0), Some(0));
        assert!((cost_none - cost_zero).abs() < 1e-12);
    }

    #[test]
    fn ollama_provider_zero_cost() {
        let pricing = get_pricing("ollama", "llama3:latest");
        assert_eq!(pricing.input_per_mtok, 0.0);
        assert_eq!(pricing.output_per_mtok, 0.0);

        let cost = estimate_call_cost(&pricing, 10000, 5000, None, None);
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn known_anthropic_model_not_fallback() {
        let pricing = get_pricing("anthropic", "claude-opus-4");
        assert!(!is_fallback_pricing(&pricing));
        assert_eq!(pricing.input_per_mtok, 15.0);
        assert_eq!(pricing.output_per_mtok, 75.0);
    }

    #[test]
    fn openrouter_deepseek_resolves() {
        let pricing = get_pricing("openrouter", "deepseek/deepseek-v3");
        assert_eq!(pricing.input_per_mtok, 0.27);
        assert_eq!(pricing.output_per_mtok, 1.10);
        assert!(!is_fallback_pricing(&pricing));
    }

    #[test]
    fn case_insensitive_matching() {
        let pricing = get_pricing("Anthropic", "Claude-Sonnet-4-6");
        assert_eq!(pricing.input_per_mtok, 3.0);
        assert!(!is_fallback_pricing(&pricing));
    }

    #[test]
    fn versioned_model_prefix_match() {
        // A model string with a date suffix should still match.
        let pricing = get_pricing("anthropic", "claude-haiku-4-5-20251022");
        assert_eq!(pricing.input_per_mtok, 0.80);
        assert_eq!(pricing.output_per_mtok, 4.0);
    }

    #[test]
    fn google_gemini_flash() {
        let pricing = get_pricing("google", "gemini-2.5-flash");
        assert_eq!(pricing.input_per_mtok, 0.15);
        assert_eq!(pricing.output_per_mtok, 0.60);
        assert!(!is_fallback_pricing(&pricing));
    }

    #[test]
    fn groq_llama_model() {
        let pricing = get_pricing("groq", "llama-3.3-70b-versatile");
        assert_eq!(pricing.input_per_mtok, 0.59);
        assert_eq!(pricing.output_per_mtok, 0.79);
    }

    #[test]
    fn mistral_models() {
        let large = get_pricing("mistral", "mistral-large-latest");
        assert_eq!(large.input_per_mtok, 2.0);
        assert_eq!(large.output_per_mtok, 6.0);

        let small = get_pricing("mistral", "mistral-small-latest");
        assert_eq!(small.input_per_mtok, 0.10);
        assert_eq!(small.output_per_mtok, 0.30);
    }

    #[test]
    fn non_anthropic_cache_multipliers_are_one() {
        let pricing = get_pricing("openai", "gpt-4o");
        assert_eq!(pricing.cache_read_multiplier, 1.0);
        assert_eq!(pricing.cache_write_multiplier, 1.0);
    }
}

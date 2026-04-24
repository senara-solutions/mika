//! Per-provider pricing constants and cost projection helpers (#762).
//!
//! Pricing as of 2026-04-24 from provider pricing pages.

// ---------------------------------------------------------------------------
// Pricing table: (input_cost_per_million_tokens, output_cost_per_million_tokens)
// ---------------------------------------------------------------------------

/// Look up per-million-token pricing for a model.
///
/// Returns `(input_cost_per_million, output_cost_per_million)`.
/// Falls back to a conservative estimate for unknown models.
fn model_pricing(model: &str) -> (f64, f64) {
    // Normalize: strip leading provider prefix if present (e.g. "anthropic/claude-..." -> "claude-...")
    let normalized = model
        .strip_prefix("anthropic/")
        .or_else(|| model.strip_prefix("openrouter/"))
        .unwrap_or(model);

    match normalized {
        // Anthropic
        "claude-haiku-4-5-20251001" => (1.00, 5.00),
        "claude-sonnet-4-6" => (3.00, 15.00),

        // OpenRouter: DeepSeek
        "deepseek/deepseek-v3" => (0.30, 0.88),

        // OpenRouter: Kimi / Moonshot
        "moonshotai/kimi-k2.5" => (0.20, 0.60),

        // Fallback: use a middle-of-the-road estimate
        _ => (1.00, 5.00),
    }
}

/// Compute the cost of a single LLM call.
pub fn per_call_cost(model: &str, input_tokens: u64, output_tokens: u64) -> f64 {
    let (input_rate, output_rate) = model_pricing(model);
    (input_tokens as f64 * input_rate + output_tokens as f64 * output_rate) / 1_000_000.0
}

/// Project burst cost for a batch of identical calls.
///
/// `call_count` calls, each with `per_call_input` input tokens and
/// `per_call_output` output tokens.
pub fn project_burst_cost(
    model: &str,
    per_call_input: u64,
    per_call_output: u64,
    call_count: u64,
) -> f64 {
    per_call_cost(
        model,
        per_call_input * call_count,
        per_call_output * call_count,
    )
}

/// Project steady-state cost for a full KG extraction+resolution cycle.
///
/// Assumes: 11 agents x 500 calls/cycle x 5 cycles = 27,500 calls.
pub fn project_steady_state_cost(model: &str, per_call_input: u64, per_call_output: u64) -> f64 {
    let total_calls: u64 = 11 * 500 * 5; // 27,500
    project_burst_cost(model, per_call_input, per_call_output, total_calls)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_haiku_pricing() {
        // 1000 input tokens + 500 output tokens at Haiku rates
        let cost = per_call_cost("claude-haiku-4-5-20251001", 1000, 500);
        // Expected: (1000 * 1.00 + 500 * 5.00) / 1_000_000 = 0.0035
        let expected = (1000.0 * 1.00 + 500.0 * 5.00) / 1_000_000.0;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn test_deepseek_pricing() {
        let cost = per_call_cost("deepseek/deepseek-v3", 1000, 500);
        let expected = (1000.0 * 0.30 + 500.0 * 0.88) / 1_000_000.0;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn test_burst_cost() {
        let cost = project_burst_cost("claude-haiku-4-5-20251001", 1000, 500, 100);
        let single = per_call_cost("claude-haiku-4-5-20251001", 1000, 500);
        assert!((cost - single * 100.0).abs() < 1e-10);
    }

    #[test]
    fn test_steady_state_cost_nonzero() {
        let cost = project_steady_state_cost("claude-haiku-4-5-20251001", 1000, 500);
        assert!(cost > 0.0);
        // 27,500 calls * $0.0035/call = ~$96.25
        let expected = 27_500.0 * per_call_cost("claude-haiku-4-5-20251001", 1000, 500);
        assert!((cost - expected).abs() < 1e-6);
    }

    #[test]
    fn test_unknown_model_fallback() {
        // Unknown model should not panic, should use fallback pricing
        let cost = per_call_cost("some-unknown-model", 1000, 500);
        assert!(cost > 0.0);
    }
}

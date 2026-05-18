//! Real-provider env gate and construction helpers for the eval matrix.
//!
//! Re-exports promoted helpers from `src/calibration/providers.rs` and provides
//! the env-var reader (`parse_real_providers`) which is test-runner glue.

use mika_common::llm::ProviderKind;

// Re-export promoted helpers
pub use mika_agent::calibration::providers::{create_real_provider, parse_provider_list};

/// Parse the `MIKA_EVAL_REAL_PROVIDERS` environment variable.
///
/// Returns a list of `ProviderKind` values. Special value `all` returns all 11 providers.
/// Empty or unset returns an empty vec. Unknown names panic with a listing of valid providers.
pub fn parse_real_providers() -> Vec<ProviderKind> {
    let val = match std::env::var("MIKA_EVAL_REAL_PROVIDERS") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => return Vec::new(),
    };
    parse_provider_list(&val)
}

/// Check if any real providers are configured for testing.
pub fn has_real_providers() -> bool {
    !parse_real_providers().is_empty()
}

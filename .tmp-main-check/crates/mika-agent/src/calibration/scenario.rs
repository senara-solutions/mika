//! Scenario outcome and registry types for the calibration framework.
//!
//! These types are shared between the eval test harness and the `calibrate` binary.

use std::sync::Arc;

use mika_common::llm::LlmProvider;
use serde::{Deserialize, Serialize};

/// Outcome of running a single scenario against a single provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioOutcome {
    /// Name of the scenario.
    pub scenario: String,
    /// Provider name.
    pub provider: String,
    /// Model used.
    pub model: String,
    /// Whether the scenario succeeded.
    pub success: bool,
    /// Error message if the scenario failed.
    pub error: Option<String>,
    /// The response text from the LLM.
    pub response_text: Option<String>,
    /// Input token count (if reported by the provider).
    pub input_tokens: Option<u64>,
    /// Output token count (if reported by the provider).
    pub output_tokens: Option<u64>,
    /// Wall-clock latency in milliseconds.
    pub latency_ms: u64,
}

/// A boxed async future returning a `ScenarioOutcome`.
pub type ScenarioFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = ScenarioOutcome> + Send>>;

/// A provider-level scenario (tests basic LLM functionality, not role-specific behavior).
pub struct Scenario {
    pub name: &'static str,
    pub description: &'static str,
    pub run: fn(Arc<dyn LlmProvider>) -> ScenarioFuture,
}

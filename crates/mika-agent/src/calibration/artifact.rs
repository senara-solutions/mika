//! Calibration artifact schema and diff tool.
//!
//! Provides the JSON schema for calibration reports and drift detection
//! between two calibration runs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::scenario::ScenarioOutcome;

/// Top-level calibration artifact, written as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationArtifact {
    /// Schema version for forward compatibility.
    pub version: u32,
    /// ISO 8601 timestamp of when the calibration was run.
    pub timestamp: String,
    /// Per-provider results.
    pub providers: BTreeMap<String, ProviderCalibration>,
}

/// Per-provider calibration data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCalibration {
    /// Model used for this provider's run.
    pub model: String,
    /// Per-scenario results.
    pub scenarios: BTreeMap<String, ScenarioCalibration>,
}

/// Per-scenario calibration data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioCalibration {
    /// Whether the scenario succeeded.
    pub outcome: String, // "pass", "fail", "error"
    /// Error classification if applicable.
    pub error_class: Option<String>,
    /// Input token count.
    pub input_tokens: Option<u64>,
    /// Output token count.
    pub output_tokens: Option<u64>,
    /// Wall-clock latency in milliseconds.
    pub latency_ms: u64,
    /// Diagnostic emission signal (mika#1699): whether the model emitted the
    /// denied command shape. `None` for non-diagnostic scenarios. Consumed by
    /// `disambiguator::render_comparison_report` to compute the verdict.
    #[serde(default)]
    pub emitted_shape: Option<bool>,
}

/// A single change detected between two calibration artifacts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationDiff {
    pub provider: String,
    pub scenario: String,
    pub old_outcome: String,
    pub new_outcome: String,
    pub change_type: String, // "tolerance_change", "new_provider", "new_scenario", "removed"
}

/// Diff result including token count summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffResult {
    pub changes: Vec<CalibrationDiff>,
    /// Per-scenario total token counts across providers.
    pub token_summary: BTreeMap<String, TokenSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenSummary {
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub provider_count: u32,
}

impl CalibrationArtifact {
    /// Build a calibration artifact from scenario outcomes.
    pub fn from_outcomes(outcomes: &[ScenarioOutcome]) -> Self {
        let mut providers: BTreeMap<String, ProviderCalibration> = BTreeMap::new();

        for outcome in outcomes {
            let provider_entry = providers
                .entry(outcome.provider.clone())
                .or_insert_with(|| ProviderCalibration {
                    model: outcome.model.clone(),
                    scenarios: BTreeMap::new(),
                });

            provider_entry.scenarios.insert(
                outcome.scenario.clone(),
                ScenarioCalibration {
                    outcome: if outcome.success {
                        "pass".to_string()
                    } else {
                        "fail".to_string()
                    },
                    error_class: outcome.error.as_ref().map(|e| classify_error(e)),
                    input_tokens: outcome.input_tokens,
                    output_tokens: outcome.output_tokens,
                    latency_ms: outcome.latency_ms,
                    emitted_shape: outcome.emitted_shape,
                },
            );
        }

        Self {
            version: 1,
            timestamp: chrono::Utc::now().to_rfc3339(),
            providers,
        }
    }

    /// Write the artifact to `target/eval-calibration/{timestamp}.json`.
    pub fn write_to_target(&self) -> anyhow::Result<PathBuf> {
        let dir = Path::new("target/eval-calibration");
        std::fs::create_dir_all(dir)?;

        let filename = format!("{}.json", self.timestamp.replace(':', "-").replace('+', ""));
        let path = dir.join(filename);
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)?;
        Ok(path)
    }

    /// Write the artifact to a specific path.
    pub fn write_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load a calibration artifact from a JSON file.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&content)?)
    }
}

/// Diff two calibration artifacts and report changes.
pub fn diff_calibrations(old: &CalibrationArtifact, new: &CalibrationArtifact) -> DiffResult {
    let mut changes = Vec::new();
    let mut token_summary: BTreeMap<String, TokenSummary> = BTreeMap::new();

    // Check all scenarios in the new artifact
    for (provider, new_cal) in &new.providers {
        for (scenario, new_scenario) in &new_cal.scenarios {
            // Update token summary
            let summary = token_summary
                .entry(scenario.clone())
                .or_insert(TokenSummary {
                    total_input_tokens: 0,
                    total_output_tokens: 0,
                    provider_count: 0,
                });
            summary.total_input_tokens += new_scenario.input_tokens.unwrap_or(0);
            summary.total_output_tokens += new_scenario.output_tokens.unwrap_or(0);
            summary.provider_count += 1;

            // Compare with old
            let old_outcome = old
                .providers
                .get(provider)
                .and_then(|p| p.scenarios.get(scenario))
                .map(|s| s.outcome.as_str());

            match old_outcome {
                Some(old_out) if old_out != new_scenario.outcome => {
                    changes.push(CalibrationDiff {
                        provider: provider.clone(),
                        scenario: scenario.clone(),
                        old_outcome: old_out.to_string(),
                        new_outcome: new_scenario.outcome.clone(),
                        change_type: "tolerance_change".to_string(),
                    });
                }
                None => {
                    // Check if it's a new provider or new scenario
                    let change_type = if old.providers.contains_key(provider) {
                        "new_scenario"
                    } else {
                        "new_provider"
                    };
                    changes.push(CalibrationDiff {
                        provider: provider.clone(),
                        scenario: scenario.clone(),
                        old_outcome: String::new(),
                        new_outcome: new_scenario.outcome.clone(),
                        change_type: change_type.to_string(),
                    });
                }
                _ => {} // Same outcome — no change
            }
        }
    }

    // Check for removed providers/scenarios
    for (provider, old_cal) in &old.providers {
        for (scenario, old_scenario) in &old_cal.scenarios {
            let still_present = new
                .providers
                .get(provider)
                .and_then(|p| p.scenarios.get(scenario))
                .is_some();
            if !still_present {
                changes.push(CalibrationDiff {
                    provider: provider.clone(),
                    scenario: scenario.clone(),
                    old_outcome: old_scenario.outcome.clone(),
                    new_outcome: String::new(),
                    change_type: "removed".to_string(),
                });
            }
        }
    }

    DiffResult {
        changes,
        token_summary,
    }
}

/// Simple error classification for calibration artifacts.
fn classify_error(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("schema") || lower.contains("validation") {
        "schema_validation".to_string()
    } else if lower.contains("rate") || lower.contains("429") {
        "rate_limit".to_string()
    } else if lower.contains("timeout") {
        "timeout".to_string()
    } else if lower.contains("auth") || lower.contains("401") || lower.contains("403") {
        "auth_error".to_string()
    } else {
        "unknown".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_outcome(scenario: &str, provider: &str, success: bool) -> ScenarioOutcome {
        ScenarioOutcome {
            scenario: scenario.to_string(),
            provider: provider.to_string(),
            model: "test-model".to_string(),
            success,
            error: if success {
                None
            } else {
                Some("test error".to_string())
            },
            response_text: Some("test".to_string()),
            input_tokens: Some(100),
            output_tokens: Some(50),
            latency_ms: 200,
            emitted_shape: None,
        }
    }

    #[test]
    fn test_calibration_round_trip() {
        let outcomes = vec![
            sample_outcome("basic", "anthropic", true),
            sample_outcome("basic", "openai", true),
            sample_outcome("multi_turn", "anthropic", false),
        ];

        let artifact = CalibrationArtifact::from_outcomes(&outcomes);
        let json = serde_json::to_string_pretty(&artifact).unwrap();
        let parsed: CalibrationArtifact = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.providers.len(), 2);
        assert!(parsed.providers.contains_key("anthropic"));
        assert!(parsed.providers.contains_key("openai"));

        let anthropic = &parsed.providers["anthropic"];
        assert_eq!(anthropic.scenarios.len(), 2);
        assert_eq!(anthropic.scenarios["basic"].outcome, "pass");
        assert_eq!(anthropic.scenarios["multi_turn"].outcome, "fail");
    }

    #[test]
    fn test_diff_identical_artifacts() {
        let outcomes = vec![sample_outcome("basic", "anthropic", true)];
        let artifact = CalibrationArtifact::from_outcomes(&outcomes);
        let result = diff_calibrations(&artifact, &artifact);
        assert!(
            result.changes.is_empty(),
            "Identical artifacts should produce no changes"
        );
    }

    #[test]
    fn test_diff_tolerance_change() {
        let old = CalibrationArtifact::from_outcomes(&[sample_outcome("basic", "anthropic", true)]);
        let new =
            CalibrationArtifact::from_outcomes(&[sample_outcome("basic", "anthropic", false)]);

        let result = diff_calibrations(&old, &new);
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].change_type, "tolerance_change");
        assert_eq!(result.changes[0].old_outcome, "pass");
        assert_eq!(result.changes[0].new_outcome, "fail");
    }

    #[test]
    fn test_diff_new_provider() {
        let old = CalibrationArtifact::from_outcomes(&[sample_outcome("basic", "anthropic", true)]);
        let new = CalibrationArtifact::from_outcomes(&[
            sample_outcome("basic", "anthropic", true),
            sample_outcome("basic", "openai", true),
        ]);

        let result = diff_calibrations(&old, &new);
        let new_provider: Vec<_> = result
            .changes
            .iter()
            .filter(|c| c.change_type == "new_provider")
            .collect();
        assert_eq!(new_provider.len(), 1);
        assert_eq!(new_provider[0].provider, "openai");
    }

    #[test]
    fn test_diff_token_summary() {
        let outcomes = vec![
            sample_outcome("basic", "anthropic", true),
            sample_outcome("basic", "openai", true),
        ];
        let artifact = CalibrationArtifact::from_outcomes(&outcomes);
        let result = diff_calibrations(&artifact, &artifact);

        assert!(result.token_summary.contains_key("basic"));
        let summary = &result.token_summary["basic"];
        assert_eq!(summary.total_input_tokens, 200); // 100 × 2 providers
        assert_eq!(summary.total_output_tokens, 100); // 50 × 2
        assert_eq!(summary.provider_count, 2);
    }
}

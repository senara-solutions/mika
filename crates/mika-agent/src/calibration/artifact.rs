//! Calibration artifact schema and diff tool.
//!
//! Provides the JSON schema for calibration reports and drift detection
//! between two calibration runs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::scenario::ScenarioOutcome;

/// Character cap for the per-scenario `response_text` field in the artifact JSON
/// (mika#1716, AC3). Bounds artifact size on verbose models — an 8000-char cap
/// keeps a full role suite well under a megabyte even when every scenario is
/// captured.
pub const RESPONSE_TEXT_CAP: usize = 8000;

/// Char-safe truncation to `cap` characters (mika#1716, DR-2).
///
/// Returns the (possibly truncated) string and whether truncation occurred.
/// Cuts on a UTF-8 char boundary via `char_indices` — never a raw byte slice —
/// so multi-byte characters at the boundary do not panic (mika#764, enforced by
/// `scripts/check-byte-slices.sh`).
pub(crate) fn truncate_to_chars(text: &str, cap: usize) -> (String, bool) {
    match text.char_indices().nth(cap) {
        Some((byte_idx, _)) => (text[..byte_idx].to_string(), true),
        None => (text.to_string(), false),
    }
}

/// Cap a captured response text for the artifact JSON, appending a marker when
/// truncated (mika#1716, AC3 + DR-2). `None` passes through unchanged.
fn cap_response_text(text: Option<&str>) -> Option<String> {
    text.map(|t| {
        let (mut capped, truncated) = truncate_to_chars(t, RESPONSE_TEXT_CAP);
        if truncated {
            capped.push_str(&format!("… [truncated to {RESPONSE_TEXT_CAP} chars]"));
        }
        capped
    })
}

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
    ///
    /// `Option` (not bare `u64`) so hand-authored baselines that omit latency
    /// (e.g. `mika-dev-1633`, which records `null`) still deserialize. A bare
    /// `u64` here made those baselines unloadable — which, under the #1701 gate,
    /// now means exit 2 instead of a silent pass. Tolerating `null` keeps a
    /// genuine committed baseline usable as a real gate.
    #[serde(default)]
    pub latency_ms: Option<u64>,
    /// Full LLM response text, capped at [`RESPONSE_TEXT_CAP`] chars (mika#1716,
    /// AC2/AC3). Present for both PASS (scenario-tuning) and FAIL (verify-not-guess
    /// diagnostic) scenarios. `#[serde(default)]` keeps v1 baselines loadable (AC6).
    #[serde(default)]
    pub response_text: Option<String>,
    /// Raw human-readable failure reason (mika#1716, AC1) — e.g. "Did not name
    /// `make deploy` as the deploy path". Distinct from the classified `error_class`:
    /// this is the exact reason string, `error_class` is its bucket. `#[serde(default)]`
    /// keeps v1 baselines loadable (AC6).
    #[serde(default)]
    pub failure_reason: Option<String>,
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
                    latency_ms: Some(outcome.latency_ms),
                    response_text: cap_response_text(outcome.response_text.as_deref()),
                    failure_reason: outcome.error.clone(),
                },
            );
        }

        Self {
            version: 2,
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

    /// Unweighted pass rate across all providers/scenarios: `pass_count / total`.
    ///
    /// The swap-gate compares a run's unweighted pass rate against the baseline's
    /// (#1701 AC8). Artifacts do not carry per-scenario weights (DR-7), so this is
    /// the only rate an artifact can report — and the run side must use the same
    /// unweighted accessor for the comparison to be meaningful. Returns `0.0` for
    /// an empty artifact.
    pub fn unweighted_pass_rate(&self) -> f64 {
        let total = self
            .providers
            .values()
            .flat_map(|p| p.scenarios.values())
            .count();
        if total == 0 {
            return 0.0;
        }
        let passed = self
            .providers
            .values()
            .flat_map(|p| p.scenarios.values())
            .filter(|s| s.outcome == "pass")
            .count();
        passed as f64 / total as f64
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
        }
    }

    #[test]
    fn test_unweighted_pass_rate() {
        // 2 pass / 3 total = 0.666…
        let artifact = CalibrationArtifact::from_outcomes(&[
            sample_outcome("a", "role", true),
            sample_outcome("b", "role", true),
            sample_outcome("c", "role", false),
        ]);
        assert!((artifact.unweighted_pass_rate() - 2.0 / 3.0).abs() < 1e-9);

        // All pass = 1.0
        let all_pass = CalibrationArtifact::from_outcomes(&[sample_outcome("a", "role", true)]);
        assert_eq!(all_pass.unweighted_pass_rate(), 1.0);
    }

    #[test]
    fn test_load_tolerates_null_latency() {
        // Regression for #1701: the mika-dev-1633 committed baseline records
        // `latency_ms: null`. A bare-`u64` field made it unloadable, which the
        // gate now treats as exit 2. Assert `null` deserializes to `None`.
        let json = r#"{
            "version": 1,
            "timestamp": "2026-06-29T00:00:00+00:00",
            "providers": {
                "mika-dev": {
                    "model": "openrouter/z-ai/glm-5.2",
                    "scenarios": {
                        "s1": {
                            "outcome": "pass",
                            "error_class": null,
                            "input_tokens": null,
                            "output_tokens": null,
                            "latency_ms": null
                        }
                    }
                }
            }
        }"#;
        let artifact: CalibrationArtifact = serde_json::from_str(json).unwrap();
        assert_eq!(artifact.unweighted_pass_rate(), 1.0);
        assert_eq!(
            artifact.providers["mika-dev"].scenarios["s1"].latency_ms,
            None
        );
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

        assert_eq!(parsed.version, 2);
        assert_eq!(parsed.providers.len(), 2);
        assert!(parsed.providers.contains_key("anthropic"));
        assert!(parsed.providers.contains_key("openai"));

        let anthropic = &parsed.providers["anthropic"];
        assert_eq!(anthropic.scenarios.len(), 2);
        assert_eq!(anthropic.scenarios["basic"].outcome, "pass");
        assert_eq!(anthropic.scenarios["multi_turn"].outcome, "fail");
    }

    #[test]
    fn test_from_outcomes_populates_response_text_and_failure_reason() {
        // PASS: response_text present (scenario-tuning), failure_reason None.
        // FAIL: both present (verify-not-guess diagnostic). mika#1716 AC1/AC2.
        let artifact = CalibrationArtifact::from_outcomes(&[
            sample_outcome("pass_scn", "role", true),
            sample_outcome("fail_scn", "role", false),
        ]);
        let scenarios = &artifact.providers["role"].scenarios;

        let pass = &scenarios["pass_scn"];
        assert_eq!(pass.outcome, "pass");
        assert_eq!(pass.response_text.as_deref(), Some("test"));
        assert_eq!(pass.failure_reason, None);

        let fail = &scenarios["fail_scn"];
        assert_eq!(fail.outcome, "fail");
        assert_eq!(fail.response_text.as_deref(), Some("test"));
        // Raw reason preserved (distinct from the classified `error_class`).
        assert_eq!(fail.failure_reason.as_deref(), Some("test error"));
        assert_eq!(fail.error_class.as_deref(), Some("unknown"));
    }

    #[test]
    fn test_cap_response_text_truncates_on_char_boundary() {
        // Short strings pass through unchanged.
        assert_eq!(cap_response_text(Some("short")).as_deref(), Some("short"));
        assert_eq!(cap_response_text(None), None);

        // A >8000-char string is capped to 8000 chars + marker, cut on a char
        // boundary. Place a multi-byte char (é, 2 bytes) exactly at the boundary
        // to prove no panic on a byte-slice.
        let mut s = "é".repeat(RESPONSE_TEXT_CAP);
        s.push_str("TAIL");
        let capped = cap_response_text(Some(&s)).unwrap();
        assert!(capped.starts_with(&"é".repeat(RESPONSE_TEXT_CAP)));
        assert!(!capped.contains("TAIL"));
        assert!(capped.contains(&format!("[truncated to {RESPONSE_TEXT_CAP} chars]")));

        // Exactly-at-cap: not truncated (no marker).
        let at_cap = "a".repeat(RESPONSE_TEXT_CAP);
        let capped_at = cap_response_text(Some(&at_cap)).unwrap();
        assert_eq!(capped_at, at_cap);
        assert!(!capped_at.contains("truncated"));
    }

    #[test]
    fn test_v1_baseline_without_new_fields_still_loads() {
        // Backwards-compat (mika#1716 AC6): a v1-shaped blob lacking `response_text`
        // and `failure_reason` must still deserialize, both defaulting to None.
        let json = r#"{
            "version": 1,
            "timestamp": "2026-06-29T00:00:00+00:00",
            "providers": {
                "mika-qa": {
                    "model": "openrouter/z-ai/glm-5.2",
                    "scenarios": {
                        "s1": {
                            "outcome": "pass",
                            "error_class": null,
                            "input_tokens": 100,
                            "output_tokens": 50,
                            "latency_ms": 200
                        }
                    }
                }
            }
        }"#;
        let artifact: CalibrationArtifact = serde_json::from_str(json).unwrap();
        let scn = &artifact.providers["mika-qa"].scenarios["s1"];
        assert_eq!(scn.response_text, None);
        assert_eq!(scn.failure_reason, None);
        assert_eq!(artifact.unweighted_pass_rate(), 1.0);
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

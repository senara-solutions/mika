//! Role-scoped scenario abstraction for model calibration.
//!
//! A `Role` represents an agent role (mika-dev, mika-arch, etc.) and its
//! associated calibration scenario suite.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::failure::FailureClass;
use super::scenario::ScenarioOutcome;

/// Display cap for the per-FAIL response snippet in the markdown report (mika#1716).
/// Smaller than the artifact JSON cap (`artifact::RESPONSE_TEXT_CAP`) — the report is
/// meant for at-a-glance scanning, the JSON preserves the fuller (8000-char) text.
const RESPONSE_TEXT_DISPLAY_CAP: usize = 2000;

/// A role-scoped scenario for model calibration.
///
/// Unlike provider-level `Scenario` (which tests basic LLM functionality),
/// a `RoleScenario` tests agent-role-specific behavior through the full
/// agent loop with synthetic skills.
pub struct RoleScenario {
    /// Unique identifier for this scenario.
    pub id: &'static str,
    /// Human-readable description.
    pub description: &'static str,
    /// Tags for filtering/grouping.
    pub tags: &'static [&'static str],
    /// Whether this scenario is known-flaky (weighted at 0.5 in pass-rate).
    pub flaky: bool,
    /// Weight for pass-rate calculation (default 1.0).
    pub weight: f64,
    /// Failure classes that MUST be absent for a pass.
    pub expected_failure_classes_absent: &'static [&'static str],
}

/// Result of running a single role-scoped scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleScenarioResult {
    /// Scenario identifier.
    pub id: String,
    /// Whether the scenario passed.
    pub passed: bool,
    /// Failure classification if the scenario failed.
    pub failure_class: Option<FailureClass>,
    /// Input token count.
    pub input_tokens: Option<u64>,
    /// Output token count.
    pub output_tokens: Option<u64>,
    /// Wall-clock latency in milliseconds.
    pub latency_ms: u64,
    /// Error details if any.
    pub error: Option<String>,
    /// LLM response text captured during the scenario run (mika#1716).
    ///
    /// `None` on error-path results (transport error, empty/refusal turn) where
    /// there is no visible response text. Populated via [`Self::with_response_text`]
    /// in the `Ok(response)` branch of each role runner, then forwarded into the
    /// artifact JSON by [`Self::to_scenario_outcome`].
    #[serde(default)]
    pub response_text: Option<String>,
}

impl RoleScenarioResult {
    /// Create a passing result.
    pub fn pass(id: &str, input_tokens: u64, output_tokens: u64, latency_ms: u64) -> Self {
        Self {
            id: id.to_string(),
            passed: true,
            failure_class: None,
            input_tokens: Some(input_tokens),
            output_tokens: Some(output_tokens),
            latency_ms,
            error: None,
            response_text: None,
        }
    }

    /// Create a failing result.
    pub fn fail(
        id: &str,
        failure_class: FailureClass,
        error: String,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        latency_ms: u64,
    ) -> Self {
        Self {
            id: id.to_string(),
            passed: false,
            failure_class: Some(failure_class),
            input_tokens,
            output_tokens,
            latency_ms,
            error: Some(error),
            response_text: None,
        }
    }

    /// Attach the LLM response text captured during the scenario run (mika#1716).
    ///
    /// Builder form so existing `pass()`/`fail()` callsites compile unchanged; the
    /// role runners chain this in the `Ok(response)` branch to preserve the model's
    /// actual words for the verify-not-guess diagnostic.
    pub fn with_response_text(mut self, text: impl Into<String>) -> Self {
        self.response_text = Some(text.into());
        self
    }

    /// Convert to a provider-level ScenarioOutcome for artifact generation.
    pub fn to_scenario_outcome(&self, provider: &str, model: &str) -> ScenarioOutcome {
        ScenarioOutcome {
            scenario: self.id.clone(),
            provider: provider.to_string(),
            model: model.to_string(),
            success: self.passed,
            error: self.error.clone(),
            response_text: self.response_text.clone(),
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            latency_ms: self.latency_ms,
        }
    }
}

/// Aggregated score report for a role's calibration run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleScoreReport {
    /// Role name.
    pub role: String,
    /// Provider/model tested.
    pub model: String,
    /// Total scenarios run.
    pub total_scenarios: usize,
    /// Scenarios that passed.
    pub passed: usize,
    /// Scenarios that failed.
    pub failed: usize,
    /// Weighted pass rate (0.0–1.0).
    pub pass_rate: f64,
    /// Total input tokens consumed.
    pub total_input_tokens: u64,
    /// Total output tokens consumed.
    pub total_output_tokens: u64,
    /// Total wall-clock latency in milliseconds.
    pub total_latency_ms: u64,
    /// Failure class breakdown.
    pub failure_breakdown: BTreeMap<String, u32>,
    /// Per-scenario results.
    pub results: Vec<RoleScenarioResult>,
}

impl RoleScoreReport {
    /// Build a score report from scenario results.
    pub fn from_results(
        role: &str,
        model: &str,
        results: Vec<RoleScenarioResult>,
        scenarios: &[RoleScenario],
    ) -> Self {
        let total = results.len();
        let passed = results.iter().filter(|r| r.passed).count();
        let failed = total - passed;

        // Weighted pass rate
        let mut weighted_pass = 0.0;
        let mut total_weight = 0.0;
        for (i, result) in results.iter().enumerate() {
            let weight = scenarios
                .get(i)
                .map(|s| if s.flaky { s.weight * 0.5 } else { s.weight })
                .unwrap_or(1.0);
            total_weight += weight;
            if result.passed {
                weighted_pass += weight;
            }
        }
        let pass_rate = if total_weight > 0.0 {
            weighted_pass / total_weight
        } else {
            0.0
        };

        // Token totals
        let total_input_tokens: u64 = results.iter().filter_map(|r| r.input_tokens).sum();
        let total_output_tokens: u64 = results.iter().filter_map(|r| r.output_tokens).sum();
        let total_latency_ms: u64 = results.iter().map(|r| r.latency_ms).sum();

        // Failure breakdown
        let mut failure_breakdown = BTreeMap::new();
        for result in &results {
            if let Some(fc) = &result.failure_class {
                *failure_breakdown.entry(fc.to_string()).or_insert(0) += 1;
            }
        }

        Self {
            role: role.to_string(),
            model: model.to_string(),
            total_scenarios: total,
            passed,
            failed,
            pass_rate,
            total_input_tokens,
            total_output_tokens,
            total_latency_ms,
            failure_breakdown,
            results,
        }
    }

    /// Generate a markdown report string.
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();
        md.push_str(&format!("# Calibration Report: {}\n\n", self.role));
        md.push_str(&format!("**Model:** {}\n", self.model));
        md.push_str(&format!(
            "**Date:** {}\n\n",
            chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
        ));

        md.push_str("## Summary\n\n");
        md.push_str("| Metric | Value |\n");
        md.push_str("|--------|-------|\n");
        md.push_str(&format!(
            "| Pass rate | {:.1}% ({}/{}) |\n",
            self.pass_rate * 100.0,
            self.passed,
            self.total_scenarios
        ));
        md.push_str(&format!("| Input tokens | {} |\n", self.total_input_tokens));
        md.push_str(&format!(
            "| Output tokens | {} |\n",
            self.total_output_tokens
        ));
        md.push_str(&format!(
            "| Total latency | {}ms |\n",
            self.total_latency_ms
        ));
        md.push_str("| Confidence | single-shot |\n\n");

        if !self.failure_breakdown.is_empty() {
            md.push_str("## Failure Breakdown\n\n");
            md.push_str("| Class | Count |\n");
            md.push_str("|-------|-------|\n");
            for (class, count) in &self.failure_breakdown {
                md.push_str(&format!("| {} | {} |\n", class, count));
            }
            md.push('\n');
        }

        md.push_str("## Per-Scenario Results\n\n");
        md.push_str("| Scenario | Result | Latency | Tokens (in/out) | Failure Class |\n");
        md.push_str("|----------|--------|---------|-----------------|---------------|\n");
        for result in &self.results {
            let status = if result.passed { "PASS" } else { "FAIL" };
            let tokens = format!(
                "{}/{}",
                result.input_tokens.unwrap_or(0),
                result.output_tokens.unwrap_or(0)
            );
            let fc = result
                .failure_class
                .as_ref()
                .map(|f| f.to_string())
                .unwrap_or_else(|| "-".to_string());
            md.push_str(&format!(
                "| {} | {} | {}ms | {} | {} |\n",
                result.id, status, result.latency_ms, tokens, fc
            ));
        }

        // Failure Details (mika#1716, AC4): per-FAIL reason + response snippet so a
        // human reading the report can spot fixture-strictness at a glance without
        // re-running with debug logging. Guarded behind "any failures exist" — a
        // fully-passing report is unchanged.
        if self.results.iter().any(|r| !r.passed) {
            md.push_str("\n## Failure Details\n\n");
            for result in self.results.iter().filter(|r| !r.passed) {
                md.push_str(&format!("### {}\n\n", result.id));
                let reason = result.error.as_deref().unwrap_or("(no reason recorded)");
                md.push_str(&format!("**Failure reason:** {reason}\n\n"));
                match result.response_text.as_deref() {
                    Some(text) if !text.trim().is_empty() => {
                        let (snippet, truncated) =
                            super::artifact::truncate_to_chars(text, RESPONSE_TEXT_DISPLAY_CAP);
                        let heading = if truncated {
                            format!(
                                "**Response text** (truncated to {RESPONSE_TEXT_DISPLAY_CAP} chars):"
                            )
                        } else {
                            "**Response text:**".to_string()
                        };
                        md.push_str(&format!("{heading}\n\n```\n{snippet}\n```\n\n"));
                    }
                    _ => {
                        md.push_str("**Response text:** (none captured)\n\n");
                    }
                }
            }
        }

        md
    }

    /// Check if this report passes against a baseline pass rate.
    pub fn passes_baseline(&self, baseline_pass_rate: f64) -> bool {
        self.pass_rate >= baseline_pass_rate
    }

    /// Unweighted pass rate: `passed / total_scenarios`.
    ///
    /// The swap-gate (#1701) uses this for BOTH the 100% floor check and the
    /// baseline comparison, so the two can never diverge (AC8). Distinct from
    /// `pass_rate`, which is *weighted* (flaky scenarios count 0.5). Committed
    /// baselines carry no weights (DR-7), so the gate compares unweighted rates
    /// on both sides. Returns `0.0` when no scenarios ran.
    pub fn unweighted_pass_rate(&self) -> f64 {
        if self.total_scenarios == 0 {
            0.0
        } else {
            self.passed as f64 / self.total_scenarios as f64
        }
    }

    /// IDs of scenarios that failed, in report order. Used by the gate diagnostic
    /// to name exactly which scenarios blocked a swap (#1701 AC1).
    pub fn failing_scenario_ids(&self) -> Vec<&str> {
        self.results
            .iter()
            .filter(|r| !r.passed)
            .map(|r| r.id.as_str())
            .collect()
    }
}

/// YAML manifest schema for a role's scenario suite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleManifest {
    pub version: u32,
    pub role: String,
    pub scenarios: Vec<ManifestScenario>,
}

/// A scenario entry in the YAML manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestScenario {
    pub id: String,
    pub fixture: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub flaky: bool,
    #[serde(default = "default_weight")]
    pub weight: f64,
    #[serde(default)]
    pub expected_failure_classes_absent: Vec<String>,
}

fn default_weight() -> f64 {
    1.0
}

impl RoleManifest {
    /// Load a manifest from a YAML file path.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Ok(serde_yaml::from_str(&content)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_report_shows_failure_details_when_a_scenario_fails() {
        // mika#1716 AC4: the .md companion surfaces the raw reason + response
        // snippet per FAIL so a human can spot fixture-strictness at a glance.
        let pass = RoleScenarioResult::pass("scn_pass", 10, 5, 100);
        let fail = RoleScenarioResult::fail(
            "scn_fail",
            FailureClass::ContractViolation,
            "Did not name `make deploy` as the deploy path".to_string(),
            Some(10),
            Some(5),
            100,
        )
        .with_response_text("I would run the deploy target to ship the merged code.");

        let report =
            RoleScoreReport::from_results("mika-orchestrator", "test-model", vec![pass, fail], &[]);
        let md = report.to_markdown();

        assert!(md.contains("## Failure Details"), "missing section: {md}");
        assert!(md.contains("scn_fail"));
        assert!(md.contains("Did not name `make deploy` as the deploy path"));
        assert!(md.contains("I would run the deploy target"));
    }

    #[test]
    fn markdown_report_omits_failure_details_when_all_pass() {
        let pass = RoleScenarioResult::pass("scn_pass", 10, 5, 100);
        let report =
            RoleScoreReport::from_results("mika-orchestrator", "test-model", vec![pass], &[]);
        let md = report.to_markdown();
        assert!(
            !md.contains("## Failure Details"),
            "all-pass report should omit the Failure Details section: {md}"
        );
    }

    #[test]
    fn to_scenario_outcome_forwards_response_text() {
        // mika#1716: `to_scenario_outcome` must forward the captured text, not the
        // old hardcoded `None`.
        let result =
            RoleScenarioResult::pass("scn", 10, 5, 100).with_response_text("captured model words");
        let outcome = result.to_scenario_outcome("mika-orchestrator", "test-model");
        assert_eq!(
            outcome.response_text.as_deref(),
            Some("captured model words")
        );
    }
}

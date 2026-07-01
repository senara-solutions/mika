//! Role-scoped scenario abstraction for model calibration.
//!
//! A `Role` represents an agent role (mika-dev, mika-arch, etc.) and its
//! associated calibration scenario suite.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::failure::FailureClass;
use super::scenario::ScenarioOutcome;

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
    /// Whether this is a **diagnostic** scenario (mika#1699).
    ///
    /// Diagnostic scenarios measure a signal (e.g. "did the model emit this
    /// command shape?") rather than pass/fail against a contract. They are
    /// excluded from the calibration pass-rate aggregate so they never move the
    /// swap-readiness gate — their output travels via `emitted_shape` instead.
    pub diagnostic: bool,
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
    /// Diagnostic emission signal (mika#1699).
    ///
    /// `Some(true)` when a diagnostic scenario's model output matched the
    /// denied-shape regex, `Some(false)` when it did not, `None` for
    /// non-diagnostic scenarios. Orthogonal to `passed`.
    #[serde(default)]
    pub emitted_shape: Option<bool>,
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
            emitted_shape: None,
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
            emitted_shape: None,
        }
    }

    /// Attach a diagnostic emission signal (mika#1699). Builder-style so
    /// existing `pass`/`fail` call sites are unaffected.
    pub fn with_emitted_shape(mut self, emitted: bool) -> Self {
        self.emitted_shape = Some(emitted);
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
            response_text: None,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            latency_ms: self.latency_ms,
            emitted_shape: self.emitted_shape,
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
        // Diagnostic scenarios (mika#1699) are excluded from the pass/fail
        // aggregate — they measure emission signal, not contract compliance, so
        // they must never move the swap-readiness gate. Their results still ride
        // along in `results` (and the JSON artifact) for the diagnostic report.
        let is_diagnostic = |i: usize| scenarios.get(i).map(|s| s.diagnostic).unwrap_or(false);

        let total = results
            .iter()
            .enumerate()
            .filter(|(i, _)| !is_diagnostic(*i))
            .count();
        let passed = results
            .iter()
            .enumerate()
            .filter(|(i, r)| !is_diagnostic(*i) && r.passed)
            .count();
        let failed = total - passed;

        // Weighted pass rate (diagnostic scenarios contribute zero weight).
        let mut weighted_pass = 0.0;
        let mut total_weight = 0.0;
        for (i, result) in results.iter().enumerate() {
            if is_diagnostic(i) {
                continue;
            }
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

        // Permission-policy diagnostic section (mika#1699). Present only when the
        // suite carried diagnostic scenarios (those with an emission signal). This
        // is the single-model view; the cross-model reproduction table + verdict
        // live in `disambiguator::render_comparison_report`.
        let diagnostics: Vec<&RoleScenarioResult> = self
            .results
            .iter()
            .filter(|r| r.emitted_shape.is_some())
            .collect();
        if !diagnostics.is_empty() {
            let emitted_count = diagnostics
                .iter()
                .filter(|r| r.emitted_shape == Some(true))
                .count();
            md.push_str("\n## Permission-Policy Diagnostic (mika#1699)\n\n");
            md.push_str(&format!(
                "Denied-shape emission for `{}` — {} of {} shapes emitted.\n\n",
                self.model,
                emitted_count,
                diagnostics.len()
            ));
            md.push_str("| Shape | Emitted | Latency |\n");
            md.push_str("|-------|---------|---------|\n");
            for result in &diagnostics {
                let emitted = if result.emitted_shape == Some(true) {
                    "yes"
                } else {
                    "no"
                };
                md.push_str(&format!(
                    "| {} | {} | {}ms |\n",
                    result.id, emitted, result.latency_ms
                ));
            }
        }

        md
    }

    /// Check if this report passes against a baseline pass rate.
    pub fn passes_baseline(&self, baseline_pass_rate: f64) -> bool {
        self.pass_rate >= baseline_pass_rate
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
    /// Whether this is a diagnostic scenario (mika#1699). Excluded from the
    /// pass-rate aggregate; measures `emitted_shape` instead.
    #[serde(default)]
    pub diagnostic: bool,
    /// Regex that detects emission of the denied command shape in model output
    /// (mika#1699, per-fixture pattern — architect F2). Travels with the fixture.
    #[serde(default)]
    pub pattern: Option<String>,
    /// Provenance of the verbatim denied shape (mika#1699 AC1 corpus integrity):
    /// originating task ID, timestamp, and the captured command string.
    #[serde(default)]
    pub provenance: Option<ShapeProvenance>,
}

/// Provenance record for a diagnostic permission-policy shape (mika#1699 AC1).
///
/// Every shape entry references the real task-callback denial it was captured
/// from — task ID, timestamp, and the verbatim command string. Paraphrases are
/// forbidden: the disambiguation is only load-bearing if the corpus is the real
/// denials.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShapeProvenance {
    pub task_id: String,
    pub timestamp: String,
    pub command: String,
    /// Set when the captured string was truncated by the classifier's Halt
    /// event (still the real denial, not a paraphrase).
    #[serde(default)]
    pub truncated: bool,
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

    fn scen(id: &'static str, diagnostic: bool) -> RoleScenario {
        RoleScenario {
            id,
            description: "",
            tags: &[],
            flaky: false,
            weight: 1.0,
            expected_failure_classes_absent: &[],
            diagnostic,
        }
    }

    #[test]
    fn diagnostic_scenarios_excluded_from_pass_rate() {
        // A diagnostic scenario that "fails" (empty response) must not move the
        // swap-readiness aggregate — only the contract scenario counts (mika#1699).
        let scenarios = [scen("contract", false), scen("diag", true)];
        let results = vec![
            RoleScenarioResult::pass("contract", 10, 5, 100),
            RoleScenarioResult::fail(
                "diag",
                FailureClass::EmptyResponse,
                "x".into(),
                None,
                None,
                50,
            )
            .with_emitted_shape(false),
        ];
        let report = RoleScoreReport::from_results("mika-dev", "m", results, &scenarios);
        assert_eq!(report.total_scenarios, 1);
        assert_eq!(report.passed, 1);
        assert_eq!(report.failed, 0);
        assert!((report.pass_rate - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn diagnostic_markdown_section_present() {
        let scenarios = [scen("diag", true)];
        let results = vec![RoleScenarioResult::pass("diag", 10, 5, 100).with_emitted_shape(true)];
        let report = RoleScoreReport::from_results("mika-dev", "glm", results, &scenarios);
        let md = report.to_markdown();
        assert!(md.contains("Permission-Policy Diagnostic"));
        assert!(md.contains("| diag | yes | 100ms |"));
    }
}

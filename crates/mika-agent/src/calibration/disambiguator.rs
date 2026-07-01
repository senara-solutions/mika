//! Permission-policy disambiguator verdict (mika#1699).
//!
//! Cross-model comparison plus the **pre-registered, immutable** decision rule
//! filed in the mika#1699 issue body. Given two calibration artifacts — one for
//! glm-5.2, one for sonnet-4-6 — each carrying `emitted_shape` diagnostic
//! signals, this computes the reproduction count and the verdict.
//!
//! Decision rule (from the issue body, copied to the branch plan so it lives in
//! code, not just the tracker):
//!
//! - **≥ 5/8 shapes reproduced by sonnet-4-6 → substrate stress dominant.**
//!   Tactical-rule cadence is making progress; mika#1686 continues tactical.
//! - **≤ 3/8 reproduced → glm-side generation dominant.** mika#1686 escalates
//!   to class-redesign; "where the gate sits" becomes primary.
//! - **4/8 (middle band) → mixed signal.** mika#1686 design proceeds with the
//!   split data in hand; no re-run trigger.
//!
//! The reproduction metric is sonnet-4-6's emission count. glm-5.2's own count
//! is the corpus-integrity check (it should emit most/all shapes, since the
//! corpus was drawn from its actual denials).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::artifact::CalibrationArtifact;

/// The pre-registered decision-rule verdict (mika#1699 — immutable once filed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    /// ≥ 5/8 shapes reproduced by sonnet-4-6 → substrate stress dominant.
    SubstrateStress,
    /// ≤ 3/8 shapes reproduced → glm-side generation dominant.
    GlmGenerates,
    /// 4/8 (middle band) → mixed signal.
    Mixed,
}

impl Verdict {
    /// Stable machine-readable slug.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SubstrateStress => "substrate-stress",
            Self::GlmGenerates => "glm-generates",
            Self::Mixed => "mixed",
        }
    }

    /// The registered downstream action for this verdict.
    pub fn downstream_action(&self) -> &'static str {
        match self {
            Self::SubstrateStress => {
                "mika#1686 continues tactical-rule cadence — the substrate producing more \
                 denied shapes under a busy Tier-2 sweep will quiet as the sweep drains."
            }
            Self::GlmGenerates => {
                "mika#1686 design session escalates to class-redesign; the \"where the gate \
                 sits\" axis (hot-path prefilter vs invocation-time gate) becomes primary."
            }
            Self::Mixed => {
                "mika#1686 design session proceeds with the split data in hand, treating BOTH \
                 hypotheses as partially true. No further disambiguator run is triggered."
            }
        }
    }
}

/// Compute the verdict from sonnet-4-6's reproduction count (mika#1699).
///
/// The bands (`≥5` / `4` / `≤3`) are defined against the canonical n=8 corpus.
/// `total` is carried for reporting but does not alter the raw-count bands — a
/// reduced-N corpus is flagged in the rendered report, not silently rescaled.
pub fn compute_verdict(sonnet_reproduced: usize) -> Verdict {
    if sonnet_reproduced >= 5 {
        Verdict::SubstrateStress
    } else if sonnet_reproduced <= 3 {
        Verdict::GlmGenerates
    } else {
        // exactly 4
        Verdict::Mixed
    }
}

/// Per-shape cross-model comparison row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShapeComparison {
    pub shape: String,
    pub glm_emitted: Option<bool>,
    pub glm_latency_ms: Option<u64>,
    pub sonnet_emitted: Option<bool>,
    pub sonnet_latency_ms: Option<u64>,
}

/// The full disambiguator report — machine-readable + markdown-renderable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisambiguatorReport {
    pub total_shapes: usize,
    pub glm_reproduced: usize,
    pub sonnet_reproduced: usize,
    pub verdict: Verdict,
    pub shapes: Vec<ShapeComparison>,
    pub glm_median_latency_ms: Option<u64>,
    pub sonnet_median_latency_ms: Option<u64>,
    pub glm_model: Option<String>,
    pub sonnet_model: Option<String>,
}

/// Collect diagnostic (`emitted_shape.is_some()`) scenarios from an artifact,
/// keyed by scenario id, as `(emitted, latency_ms)`.
fn collect_diagnostics(artifact: &CalibrationArtifact) -> BTreeMap<String, (Option<bool>, u64)> {
    let mut out = BTreeMap::new();
    for provider in artifact.providers.values() {
        for (id, scenario) in &provider.scenarios {
            if scenario.emitted_shape.is_some() {
                out.insert(id.clone(), (scenario.emitted_shape, scenario.latency_ms));
            }
        }
    }
    out
}

/// The model string of the single provider in an artifact, if present.
fn artifact_model(artifact: &CalibrationArtifact) -> Option<String> {
    artifact.providers.values().next().map(|p| p.model.clone())
}

/// Median of a slice of latencies (`None` if empty).
fn median(values: &[u64]) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        Some((sorted[mid - 1] + sorted[mid]) / 2)
    } else {
        Some(sorted[mid])
    }
}

impl DisambiguatorReport {
    /// Build the report from a glm-5.2 artifact and a sonnet-4-6 artifact.
    pub fn from_artifacts(glm: &CalibrationArtifact, sonnet: &CalibrationArtifact) -> Self {
        let glm_diag = collect_diagnostics(glm);
        let sonnet_diag = collect_diagnostics(sonnet);

        // Union of shape ids across both runs, deterministically ordered.
        let mut ids: Vec<String> = glm_diag.keys().cloned().collect();
        for id in sonnet_diag.keys() {
            if !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        ids.sort();

        let mut shapes = Vec::new();
        let mut glm_latencies = Vec::new();
        let mut sonnet_latencies = Vec::new();
        let mut glm_reproduced = 0;
        let mut sonnet_reproduced = 0;

        for id in &ids {
            let glm_entry = glm_diag.get(id);
            let sonnet_entry = sonnet_diag.get(id);

            let glm_emitted = glm_entry.and_then(|(e, _)| *e);
            let sonnet_emitted = sonnet_entry.and_then(|(e, _)| *e);
            let glm_latency = glm_entry.map(|(_, l)| *l);
            let sonnet_latency = sonnet_entry.map(|(_, l)| *l);

            if glm_emitted == Some(true) {
                glm_reproduced += 1;
            }
            if sonnet_emitted == Some(true) {
                sonnet_reproduced += 1;
            }
            if let Some(l) = glm_latency {
                glm_latencies.push(l);
            }
            if let Some(l) = sonnet_latency {
                sonnet_latencies.push(l);
            }

            shapes.push(ShapeComparison {
                shape: id.clone(),
                glm_emitted,
                glm_latency_ms: glm_latency,
                sonnet_emitted,
                sonnet_latency_ms: sonnet_latency,
            });
        }

        Self {
            total_shapes: ids.len(),
            glm_reproduced,
            sonnet_reproduced,
            verdict: compute_verdict(sonnet_reproduced),
            shapes,
            glm_median_latency_ms: median(&glm_latencies),
            sonnet_median_latency_ms: median(&sonnet_latencies),
            glm_model: artifact_model(glm),
            sonnet_model: artifact_model(sonnet),
        }
    }

    /// Render the cross-model comparison markdown report (AC4).
    ///
    /// Contains the reproduction table (shape × model → emitted-yes/no), the
    /// latency table (shape × model → ms), and the decision-rule verdict block.
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();
        md.push_str("# Permission-Policy Disambiguator — mika#1699\n\n");
        md.push_str(&format!(
            "**glm-5.2 route:** `{}`  \n",
            self.glm_model.as_deref().unwrap_or("(unknown)")
        ));
        md.push_str(&format!(
            "**sonnet-4-6 route:** `{}`\n\n",
            self.sonnet_model.as_deref().unwrap_or("(unknown)")
        ));

        if self.total_shapes != 8 {
            md.push_str(&format!(
                "> ⚠️ Reduced corpus: {} of the canonical 8 shapes present. The decision-rule \
                 bands (≥5 / 4 / ≤3) are defined for n=8; interpret with care.\n\n",
                self.total_shapes
            ));
        }

        // Reproduction table
        md.push_str("## Reproduction (shape × model → emitted)\n\n");
        md.push_str("| Shape | glm-5.2 | sonnet-4-6 |\n");
        md.push_str("|-------|---------|------------|\n");
        for s in &self.shapes {
            md.push_str(&format!(
                "| {} | {} | {} |\n",
                s.shape,
                fmt_emitted(s.glm_emitted),
                fmt_emitted(s.sonnet_emitted),
            ));
        }
        md.push_str(&format!(
            "| **reproduced** | **{}/{}** | **{}/{}** |\n\n",
            self.glm_reproduced, self.total_shapes, self.sonnet_reproduced, self.total_shapes
        ));

        // Latency table
        md.push_str("## Latency (shape × model → ms)\n\n");
        md.push_str("| Shape | glm-5.2 | sonnet-4-6 |\n");
        md.push_str("|-------|---------|------------|\n");
        for s in &self.shapes {
            md.push_str(&format!(
                "| {} | {} | {} |\n",
                s.shape,
                fmt_latency(s.glm_latency_ms),
                fmt_latency(s.sonnet_latency_ms),
            ));
        }
        md.push_str(&format!(
            "| **median** | {} | {} |\n\n",
            fmt_latency(self.glm_median_latency_ms),
            fmt_latency(self.sonnet_median_latency_ms),
        ));

        // Verdict block
        md.push_str("## Verdict\n\n");
        md.push_str(&format!(
            "**{}** — sonnet-4-6 reproduced **{}/{}** shapes.\n\n",
            self.verdict.as_str(),
            self.sonnet_reproduced,
            self.total_shapes
        ));
        md.push_str(&format!("{}\n", self.verdict.downstream_action()));

        md
    }
}

fn fmt_emitted(e: Option<bool>) -> &'static str {
    match e {
        Some(true) => "yes",
        Some(false) => "no",
        None => "—",
    }
}

fn fmt_latency(l: Option<u64>) -> String {
    match l {
        Some(ms) => format!("{}ms", ms),
        None => "—".to_string(),
    }
}

/// Convenience: render the comparison report markdown directly from two artifacts.
pub fn render_comparison_report(glm: &CalibrationArtifact, sonnet: &CalibrationArtifact) -> String {
    DisambiguatorReport::from_artifacts(glm, sonnet).to_markdown()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calibration::scenario::ScenarioOutcome;

    fn diag_outcome(scenario: &str, model: &str, emitted: bool, latency: u64) -> ScenarioOutcome {
        ScenarioOutcome {
            scenario: scenario.to_string(),
            provider: "mika-dev".to_string(),
            model: model.to_string(),
            success: true,
            error: None,
            response_text: None,
            input_tokens: Some(100),
            output_tokens: Some(50),
            latency_ms: latency,
            emitted_shape: Some(emitted),
        }
    }

    fn artifact_with(model: &str, emissions: &[(&str, bool, u64)]) -> CalibrationArtifact {
        let outcomes: Vec<ScenarioOutcome> = emissions
            .iter()
            .map(|(id, emitted, latency)| diag_outcome(id, model, *emitted, *latency))
            .collect();
        CalibrationArtifact::from_outcomes(&outcomes)
    }

    #[test]
    fn verdict_substrate_stress_at_5() {
        assert_eq!(compute_verdict(5), Verdict::SubstrateStress);
        assert_eq!(compute_verdict(6), Verdict::SubstrateStress);
        assert_eq!(compute_verdict(8), Verdict::SubstrateStress);
    }

    #[test]
    fn verdict_glm_generates_at_3_or_below() {
        assert_eq!(compute_verdict(0), Verdict::GlmGenerates);
        assert_eq!(compute_verdict(3), Verdict::GlmGenerates);
    }

    #[test]
    fn verdict_mixed_at_exactly_4() {
        assert_eq!(compute_verdict(4), Verdict::Mixed);
    }

    #[test]
    fn report_counts_and_verdict() {
        // sonnet reproduces 5/8 → substrate stress.
        let shapes = ["s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8"];
        let glm: Vec<(&str, bool, u64)> = shapes.iter().map(|s| (*s, true, 1000)).collect(); // glm emits all 8
        let sonnet: Vec<(&str, bool, u64)> = shapes
            .iter()
            .enumerate()
            .map(|(i, s)| (*s, i < 5, 1500)) // sonnet emits first 5
            .collect();

        let report = DisambiguatorReport::from_artifacts(
            &artifact_with("openrouter/z-ai/glm-5.2", &glm),
            &artifact_with("openrouter/anthropic/claude-sonnet-4-6", &sonnet),
        );

        assert_eq!(report.total_shapes, 8);
        assert_eq!(report.glm_reproduced, 8);
        assert_eq!(report.sonnet_reproduced, 5);
        assert_eq!(report.verdict, Verdict::SubstrateStress);
        assert_eq!(report.glm_median_latency_ms, Some(1000));
        assert_eq!(report.sonnet_median_latency_ms, Some(1500));

        let md = report.to_markdown();
        assert!(md.contains("## Reproduction"));
        assert!(md.contains("## Latency"));
        assert!(md.contains("substrate-stress"));
        assert!(md.contains("**5/8**"));
    }

    #[test]
    fn report_glm_generates_verdict() {
        // sonnet reproduces only 2/8 → glm generates.
        let shapes = ["s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8"];
        let glm: Vec<(&str, bool, u64)> = shapes.iter().map(|s| (*s, true, 900)).collect();
        let sonnet: Vec<(&str, bool, u64)> = shapes
            .iter()
            .enumerate()
            .map(|(i, s)| (*s, i < 2, 1200))
            .collect();

        let report = DisambiguatorReport::from_artifacts(
            &artifact_with("glm", &glm),
            &artifact_with("sonnet", &sonnet),
        );
        assert_eq!(report.sonnet_reproduced, 2);
        assert_eq!(report.verdict, Verdict::GlmGenerates);
    }

    #[test]
    fn reduced_corpus_flagged() {
        let shapes = ["s1", "s2", "s3", "s4", "s5"]; // only 5 present
        let glm: Vec<(&str, bool, u64)> = shapes.iter().map(|s| (*s, true, 900)).collect();
        let sonnet: Vec<(&str, bool, u64)> = shapes.iter().map(|s| (*s, true, 900)).collect();
        let report = DisambiguatorReport::from_artifacts(
            &artifact_with("glm", &glm),
            &artifact_with("sonnet", &sonnet),
        );
        assert_eq!(report.total_shapes, 5);
        assert!(report.to_markdown().contains("Reduced corpus"));
    }
}

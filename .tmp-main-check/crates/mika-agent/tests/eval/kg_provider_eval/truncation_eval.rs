//! Truncation quality comparison eval (#766).
//!
//! Compares byte-truncation (`safe_truncate`) vs sentence-boundary truncation
//! (`truncate_at_semantic_boundary`) on entity resolution disambiguation quality.
//!
//! The eval loads extended-context resolution cases, runs disambiguation twice
//! per case (one with each truncation strategy), and reports per-case deltas.
//!
//! Run with:
//! ```bash
//! MIKA_EVAL_KG_PROVIDERS=default cargo test -p mika-agent --test eval truncation_eval -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use mika_common::llm::LlmProvider;
use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};
use mika_common::text::safe_truncate;
use serde::{Deserialize, Serialize};

use super::KgProviderSpec;
use super::prompts;

// ---------------------------------------------------------------------------
// Fixture types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TruncationFixture {
    cases: Vec<TruncationCase>,
}

#[derive(Debug, Deserialize)]
struct TruncationCase {
    entity_key: String,
    entity_confidence: f64,
    chunk_context: String,
    expected_match: String,
    case_category: String,
    #[allow(dead_code)]
    disputable: bool,
    #[allow(dead_code)]
    rationale: String,
    candidates: Vec<Candidate>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    entity_key: String,
    description: String,
}

#[derive(Debug, Deserialize)]
struct DisambiguationResponse {
    #[serde(rename = "match")]
    matched: Option<String>,
    confidence: f64,
}

// ---------------------------------------------------------------------------
// Per-case outcome
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct TruncationComparisonOutcome {
    pub provider: String,
    pub entity_key: String,
    pub case_category: String,
    pub expected_match: String,
    /// Byte-truncation variant (A)
    pub byte_correct: bool,
    pub byte_match: Option<String>,
    pub byte_confidence: f64,
    pub byte_context_len: usize,
    pub byte_error: Option<String>,
    /// Semantic-truncation variant (B)
    pub semantic_correct: bool,
    pub semantic_match: Option<String>,
    pub semantic_confidence: f64,
    pub semantic_context_len: usize,
    pub semantic_error: Option<String>,
    /// Delta
    pub flipped: Option<String>,
    pub confidence_delta: f64,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Lowered budget to ensure truncation triggers on most cases.
const TRUNCATION_BUDGET: usize = 500;

/// Truncate prose at the last sentence-ending punctuation (`. ! ?`) within
/// the budget, falling back to `safe_truncate` if no boundary exists or
/// the boundary would use less than 50% of the budget.
///
/// Eval-local helper for the truncation comparison harness. Lives here
/// (not in `mika_common::text`) because the empirical decision in
/// mika#766 (2026-06-26) reverted the production usage of semantic
/// truncation — the function survives only as eval infrastructure for
/// any future re-run of the question (e.g., bigger fixture, different
/// model). Mirrors the implementation that was tested.
fn truncate_at_semantic_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let limit = s.floor_char_boundary(s.len().min(max_bytes));
    let min_utilization = max_bytes / 2;
    if let Some(end) = s[..limit].rfind(['.', '!', '?']) {
        let boundary = end + s[end..].chars().next().map_or(0, |c| c.len_utf8());
        if boundary >= min_utilization {
            return &s[..boundary];
        }
    }
    safe_truncate(s, max_bytes)
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

/// Run the truncation comparison eval across all providers.
pub async fn run_truncation_eval(
    providers: &[(KgProviderSpec, Arc<dyn LlmProvider>)],
) -> Vec<TruncationComparisonOutcome> {
    let fixture = load_truncation_fixture();
    let mut outcomes = Vec::new();

    for case in &fixture.cases {
        let candidates: Vec<(String, String)> = case
            .candidates
            .iter()
            .map(|c| (c.entity_key.clone(), c.description.clone()))
            .collect();

        // Variant A: byte truncation
        let byte_context = safe_truncate(&case.chunk_context, TRUNCATION_BUDGET);
        // Variant B: semantic truncation
        let semantic_context =
            truncate_at_semantic_boundary(&case.chunk_context, TRUNCATION_BUDGET);

        let (system_a, user_a) = prompts::build_disambiguation_prompt(
            &case.entity_key,
            case.entity_confidence,
            byte_context,
            &candidates,
        );

        let (system_b, user_b) = prompts::build_disambiguation_prompt(
            &case.entity_key,
            case.entity_confidence,
            semantic_context,
            &candidates,
        );

        for (spec, provider) in providers {
            println!(
                "  [truncation] {} x {} (byte: {} bytes, semantic: {} bytes) ...",
                spec.display_name,
                case.entity_key,
                byte_context.len(),
                semantic_context.len(),
            );

            let byte_result =
                run_single_disambiguation(provider.as_ref(), &system_a, &user_a).await;
            let semantic_result =
                run_single_disambiguation(provider.as_ref(), &system_b, &user_b).await;

            let byte_correct = is_correct_match(&byte_result.matched, &case.expected_match);
            let semantic_correct = is_correct_match(&semantic_result.matched, &case.expected_match);

            let flipped = match (byte_correct, semantic_correct) {
                (false, true) => Some("byte_wrong_semantic_right".to_string()),
                (true, false) => Some("byte_right_semantic_wrong".to_string()),
                _ => None,
            };

            let confidence_delta = semantic_result.confidence - byte_result.confidence;

            outcomes.push(TruncationComparisonOutcome {
                provider: spec.display_name.clone(),
                entity_key: case.entity_key.clone(),
                case_category: case.case_category.clone(),
                expected_match: case.expected_match.clone(),
                byte_correct,
                byte_match: byte_result.matched,
                byte_confidence: byte_result.confidence,
                byte_context_len: byte_context.len(),
                byte_error: byte_result.error,
                semantic_correct,
                semantic_match: semantic_result.matched,
                semantic_confidence: semantic_result.confidence,
                semantic_context_len: semantic_context.len(),
                semantic_error: semantic_result.error,
                flipped,
                confidence_delta,
            });
        }
    }

    outcomes
}

/// Print a summary of the truncation comparison results.
pub fn print_truncation_summary(outcomes: &[TruncationComparisonOutcome]) {
    if outcomes.is_empty() {
        println!("\n[truncation_eval] No outcomes to report.");
        return;
    }

    println!("\n{:=^80}", " Truncation Comparison Summary ");
    println!(
        "\n{:<35} {:<12} {:<12} {:<8} {:<10}",
        "Case", "Byte", "Semantic", "Flipped", "Conf Δ"
    );
    println!("{:-<80}", "");

    let mut total_byte_correct = 0;
    let mut total_semantic_correct = 0;
    let mut total_flipped_semantic_wins = 0;
    let mut total_flipped_byte_wins = 0;
    let mut total_confidence_delta = 0.0;

    for o in outcomes {
        let byte_mark = if o.byte_correct { "✓" } else { "✗" };
        let semantic_mark = if o.semantic_correct { "✓" } else { "✗" };
        let flip_mark = match o.flipped.as_deref() {
            Some("byte_wrong_semantic_right") => "→ sem",
            Some("byte_right_semantic_wrong") => "→ byte",
            _ => "—",
        };

        println!(
            "{:<35} {:<12} {:<12} {:<8} {:+.3}",
            format!(
                "{} ({})",
                safe_truncate(&o.entity_key, 20),
                safe_truncate(&o.provider, 10)
            ),
            format!("{} ({:.2})", byte_mark, o.byte_confidence),
            format!("{} ({:.2})", semantic_mark, o.semantic_confidence),
            flip_mark,
            o.confidence_delta,
        );

        if o.byte_correct {
            total_byte_correct += 1;
        }
        if o.semantic_correct {
            total_semantic_correct += 1;
        }
        if o.flipped.as_deref() == Some("byte_wrong_semantic_right") {
            total_flipped_semantic_wins += 1;
        }
        if o.flipped.as_deref() == Some("byte_right_semantic_wrong") {
            total_flipped_byte_wins += 1;
        }
        total_confidence_delta += o.confidence_delta;
    }

    let n = outcomes.len();
    println!("\n{:-<80}", "");
    println!("Total cases: {}", n);
    println!(
        "Byte accuracy:     {}/{} ({:.1}%)",
        total_byte_correct,
        n,
        total_byte_correct as f64 / n as f64 * 100.0
    );
    println!(
        "Semantic accuracy: {}/{} ({:.1}%)",
        total_semantic_correct,
        n,
        total_semantic_correct as f64 / n as f64 * 100.0
    );
    println!(
        "Flipped outcomes:  {} semantic wins, {} byte wins",
        total_flipped_semantic_wins, total_flipped_byte_wins
    );
    println!(
        "Mean confidence Δ: {:+.4}",
        total_confidence_delta / n as f64
    );

    // Single-run summary — d-statistic is per-run; §1's mechanical disposition
    // requires the two-run comparison printed by `print_two_run_comparison`.
    let d_value = total_semantic_correct - total_byte_correct;
    println!(
        "d (this run): {} (= total_semantic_correct - total_byte_correct)",
        d_value
    );
}

// ---------------------------------------------------------------------------
// Two-run comparison (§1 — mika#766)
// ---------------------------------------------------------------------------

/// Per-case agreement between two runs (mika#766 §1 two-run protocol).
#[derive(Debug, Clone)]
pub struct CaseAgreement {
    pub provider: String,
    pub entity_key: String,
    /// (byte_correct, semantic_correct) tuple from run 1.
    pub run1_outcome: (bool, bool),
    /// (byte_correct, semantic_correct) tuple from run 2.
    pub run2_outcome: (bool, bool),
    pub agreed: bool,
}

/// Compute per-case agreement between two runs.
///
/// Aligns outcomes by `(provider, entity_key)`. An outcome present in run 1
/// but missing from run 2 is dropped (treated as missing data, not disagreement)
/// — this is conservative and surfaces in the count.
pub fn compute_per_case_agreement(
    run1: &[TruncationComparisonOutcome],
    run2: &[TruncationComparisonOutcome],
) -> Vec<CaseAgreement> {
    let mut agreements = Vec::new();
    for o1 in run1 {
        let o2 = match run2
            .iter()
            .find(|o| o.provider == o1.provider && o.entity_key == o1.entity_key)
        {
            Some(o) => o,
            None => continue,
        };
        let run1_outcome = (o1.byte_correct, o1.semantic_correct);
        let run2_outcome = (o2.byte_correct, o2.semantic_correct);
        agreements.push(CaseAgreement {
            provider: o1.provider.clone(),
            entity_key: o1.entity_key.clone(),
            run1_outcome,
            run2_outcome,
            agreed: run1_outcome == run2_outcome,
        });
    }
    agreements
}

/// Compute d-statistic for a single run: `total_semantic_correct - total_byte_correct`.
fn compute_d(outcomes: &[TruncationComparisonOutcome]) -> i32 {
    outcomes
        .iter()
        .map(|o| (o.semantic_correct as i32) - (o.byte_correct as i32))
        .sum()
}

/// Print the two-run comparison + §1 mechanical disposition (mika#766).
///
/// §1's disposition rule (all gates shown, regardless of which fires):
///   - agreement≥8 ✗ → Revert (inability-to-measure)
///   - d≥3 on both runs ✓ → Stay
///   - d≥3 ✗ on either run → Revert
pub fn print_two_run_comparison(
    run1: &[TruncationComparisonOutcome],
    run2: &[TruncationComparisonOutcome],
) {
    println!("\n{:=^80}", " Two-Run Per-Case Agreement (mika#766 §1) ");

    let agreements = compute_per_case_agreement(run1, run2);
    let total = agreements.len();
    let agreed_count = agreements.iter().filter(|a| a.agreed).count();

    // Per-case table
    println!(
        "\n{:<35} {:<14} {:<14} {:<8}",
        "Case", "Run 1 (B,S)", "Run 2 (B,S)", "Agreed"
    );
    println!("{:-<80}", "");
    for a in &agreements {
        let r1 = format!(
            "({},{})",
            if a.run1_outcome.0 { "T" } else { "F" },
            if a.run1_outcome.1 { "T" } else { "F" }
        );
        let r2 = format!(
            "({},{})",
            if a.run2_outcome.0 { "T" } else { "F" },
            if a.run2_outcome.1 { "T" } else { "F" }
        );
        let mark = if a.agreed { "yes" } else { "no" };
        println!(
            "{:<35} {:<14} {:<14} {:<8}",
            format!(
                "{} ({})",
                safe_truncate(&a.entity_key, 20),
                safe_truncate(&a.provider, 10)
            ),
            r1,
            r2,
            mark,
        );
    }

    let d_run1 = compute_d(run1);
    let d_run2 = compute_d(run2);

    println!("\n{:-<80}", "");
    println!("Per-case agreement: {}/{}", agreed_count, total);
    println!("d (run 1): {}", d_run1);
    println!("d (run 2): {}", d_run2);

    // §1 mechanical disposition — all gates visible, short-circuit explicit
    println!("\n--- §1 Mechanical Disposition (mika#766) ---");
    let agreement_ok = agreed_count >= 8;
    let d_ok_both = d_run1 >= 3 && d_run2 >= 3;
    let agreement_mark = if agreement_ok { "✓" } else { "✗" };
    let d_mark = if d_ok_both { "✓" } else { "✗" };

    let disposition_line = if !agreement_ok {
        format!(
            "d_run1={}, d_run2={}, agreement={}/{} → agreement≥8 {} → Revert (inability-to-measure)",
            d_run1, d_run2, agreed_count, total, agreement_mark
        )
    } else if d_ok_both {
        format!(
            "d_run1={}, d_run2={}, agreement={}/{} → d≥3 on both runs {} AND agreement≥8 {} → Stay",
            d_run1, d_run2, agreed_count, total, d_mark, agreement_mark
        )
    } else {
        format!(
            "d_run1={}, d_run2={}, agreement={}/{} → agreement≥8 {} AND d≥3 on both runs {} → Revert",
            d_run1, d_run2, agreed_count, total, agreement_mark, d_mark
        )
    };
    println!("{}", disposition_line);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct DisambiguationResult {
    matched: Option<String>,
    confidence: f64,
    error: Option<String>,
}

async fn run_single_disambiguation(
    provider: &dyn LlmProvider,
    system: &str,
    user_text: &str,
) -> DisambiguationResult {
    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some(system.to_string()),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text(user_text.to_string()),
        }],
        tools: None,
        max_tokens: provider.max_tokens(),
        thinking: None,
    };

    let start = Instant::now();
    let result = provider.send_message(&request).await;
    let _latency_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(response) => {
            let text = response.text();
            match parse_disambiguation_response(&text) {
                Ok(parsed) => DisambiguationResult {
                    matched: parsed.matched,
                    confidence: parsed.confidence,
                    error: None,
                },
                Err(e) => DisambiguationResult {
                    matched: None,
                    confidence: 0.0,
                    error: Some(format!("parse error: {}", e)),
                },
            }
        }
        Err(e) => DisambiguationResult {
            matched: None,
            confidence: 0.0,
            error: Some(format!("LLM error: {}", e)),
        },
    }
}

fn parse_disambiguation_response(text: &str) -> Result<DisambiguationResponse, String> {
    let cleaned = text
        .trim()
        .strip_prefix("```json")
        .or_else(|| text.trim().strip_prefix("```"))
        .unwrap_or(text.trim());
    let cleaned = cleaned.strip_suffix("```").unwrap_or(cleaned).trim();
    serde_json::from_str(cleaned).map_err(|e| format!("{}", e))
}

fn is_correct_match(actual: &Option<String>, expected: &str) -> bool {
    if expected.is_empty() || expected == "null" {
        actual.is_none()
    } else {
        actual.as_deref() == Some(expected)
    }
}

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

fn load_truncation_fixture() -> TruncationFixture {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("failed to resolve repo root")
        .to_path_buf();
    let fixture_path =
        repo_root.join("docs/solutions/kg/eval-fixtures-2026-04-24/truncation_eval_contexts.toml");
    let content = std::fs::read_to_string(&fixture_path).unwrap_or_else(|e| {
        panic!(
            "failed to read truncation fixture at {:?}: {}",
            fixture_path, e
        )
    });
    toml::from_str(&content).unwrap_or_else(|e| panic!("failed to parse truncation fixture: {}", e))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixture_loads() {
        let fixture = load_truncation_fixture();
        assert!(
            !fixture.cases.is_empty(),
            "truncation fixture must have cases"
        );
    }

    #[test]
    fn test_all_cases_have_long_contexts() {
        let fixture = load_truncation_fixture();
        for case in &fixture.cases {
            assert!(
                case.chunk_context.len() > TRUNCATION_BUDGET,
                "case '{}' chunk_context ({} bytes) must exceed budget ({} bytes)",
                case.entity_key,
                case.chunk_context.len(),
                TRUNCATION_BUDGET,
            );
        }
    }

    #[test]
    fn test_all_cases_have_candidates() {
        let fixture = load_truncation_fixture();
        for case in &fixture.cases {
            assert!(
                !case.candidates.is_empty(),
                "case '{}' must have at least one candidate",
                case.entity_key
            );
        }
    }

    #[test]
    fn test_truncation_produces_different_results() {
        let fixture = load_truncation_fixture();
        let mut any_different = false;
        for case in &fixture.cases {
            let byte = safe_truncate(&case.chunk_context, TRUNCATION_BUDGET);
            let semantic = truncate_at_semantic_boundary(&case.chunk_context, TRUNCATION_BUDGET);
            if byte != semantic {
                any_different = true;
                // Semantic should end at sentence boundary
                let last_char = semantic.chars().last().unwrap_or(' ');
                assert!(
                    matches!(last_char, '.' | '!' | '?'),
                    "case '{}': semantic truncation should end at sentence boundary, got '{}'",
                    case.entity_key,
                    last_char,
                );
            }
        }
        assert!(
            any_different,
            "at least one case should produce different truncations"
        );
    }
}

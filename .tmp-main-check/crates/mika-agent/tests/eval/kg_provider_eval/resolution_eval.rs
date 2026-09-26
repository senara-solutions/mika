//! Resolution evaluation: run the disambiguation prompt against each provider
//! for each ground-truth case and score the results (#762).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use mika_common::llm::LlmProvider;
use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};
use serde::Deserialize;

use super::prompts;
use super::{KgProviderSpec, KgResolutionOutcome};

// ---------------------------------------------------------------------------
// Fixture types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ResolutionFixture {
    cases: Vec<ResolutionCase>,
}

#[derive(Debug, Deserialize)]
struct ResolutionCase {
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

/// Parsed LLM disambiguation response.
#[derive(Debug, Deserialize)]
struct DisambiguationResponse {
    #[serde(rename = "match")]
    matched: Option<String>,
    confidence: f64,
}

// ---------------------------------------------------------------------------
// Resolution runner
// ---------------------------------------------------------------------------

/// Run resolution eval across all providers and ground-truth cases.
///
/// Returns a flat list of per-provider-per-case outcomes.
pub async fn run_resolution_eval(
    providers: &[(KgProviderSpec, Arc<dyn LlmProvider>)],
) -> Vec<KgResolutionOutcome> {
    let fixture = load_resolution_fixture();
    let mut outcomes = Vec::new();

    for case in &fixture.cases {
        let candidates: Vec<(String, String)> = case
            .candidates
            .iter()
            .map(|c| (c.entity_key.clone(), c.description.clone()))
            .collect();

        let (system, user_text) = prompts::build_disambiguation_prompt(
            &case.entity_key,
            case.entity_confidence,
            &case.chunk_context,
            &candidates,
        );

        for (spec, provider) in providers {
            println!(
                "  [resolution] {} x {} ...",
                spec.display_name, case.entity_key
            );

            let outcome =
                run_single_resolution(spec, provider.as_ref(), &system, &user_text, case).await;
            outcomes.push(outcome);
        }
    }

    outcomes
}

async fn run_single_resolution(
    spec: &KgProviderSpec,
    provider: &dyn LlmProvider,
    system: &str,
    user_text: &str,
    case: &ResolutionCase,
) -> KgResolutionOutcome {
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
    let latency_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(response) => {
            let text = response.text();
            let input_tokens = response.usage.input_tokens;
            let output_tokens = response.usage.output_tokens;

            match parse_disambiguation_response(&text) {
                Ok(parsed) => {
                    let actual_match = parsed.matched.clone();
                    let correct = is_correct_match(&actual_match, &case.expected_match);

                    KgResolutionOutcome {
                        provider: spec.display_name.clone(),
                        entity_key: case.entity_key.clone(),
                        case_category: case.case_category.clone(),
                        correct,
                        expected_match: case.expected_match.clone(),
                        actual_match,
                        confidence: parsed.confidence,
                        input_tokens,
                        output_tokens,
                        latency_ms,
                        error: None,
                    }
                }
                Err(e) => KgResolutionOutcome {
                    provider: spec.display_name.clone(),
                    entity_key: case.entity_key.clone(),
                    case_category: case.case_category.clone(),
                    correct: false,
                    expected_match: case.expected_match.clone(),
                    actual_match: None,
                    confidence: 0.0,
                    input_tokens,
                    output_tokens,
                    latency_ms,
                    error: Some(format!("parse error: {}", e)),
                },
            }
        }
        Err(e) => KgResolutionOutcome {
            provider: spec.display_name.clone(),
            entity_key: case.entity_key.clone(),
            case_category: case.case_category.clone(),
            correct: false,
            expected_match: case.expected_match.clone(),
            actual_match: None,
            confidence: 0.0,
            input_tokens: 0,
            output_tokens: 0,
            latency_ms,
            error: Some(format!("LLM error: {}", e)),
        },
    }
}

// ---------------------------------------------------------------------------
// Scoring helpers
// ---------------------------------------------------------------------------

/// Parse the LLM disambiguation response text.
///
/// Strips markdown code fences if present.
fn parse_disambiguation_response(text: &str) -> Result<DisambiguationResponse, String> {
    let cleaned = text
        .trim()
        .strip_prefix("```json")
        .or_else(|| text.trim().strip_prefix("```"))
        .unwrap_or(text.trim());
    let cleaned = cleaned.strip_suffix("```").unwrap_or(cleaned).trim();

    serde_json::from_str(cleaned).map_err(|e| format!("{}", e))
}

/// Check if the actual match is correct given the expected match.
///
/// `expected_match` of `"null"` or empty means "no match expected" — correct if actual is None.
fn is_correct_match(actual: &Option<String>, expected: &str) -> bool {
    if expected.is_empty() || expected == "null" {
        // Expected no match — correct if actual is None
        actual.is_none()
    } else {
        // Expected a specific match
        actual.as_deref() == Some(expected)
    }
}

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

fn load_resolution_fixture() -> ResolutionFixture {
    // Fixtures live in docs/solutions/kg/eval-fixtures-2026-04-24/ (repo root).
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("failed to resolve repo root")
        .to_path_buf();
    let fixture_path =
        repo_root.join("docs/solutions/kg/eval-fixtures-2026-04-24/resolution_ground_truth.toml");
    let content = std::fs::read_to_string(&fixture_path).unwrap_or_else(|e| {
        panic!(
            "failed to read resolution fixture at {:?}: {}",
            fixture_path, e
        )
    });
    toml::from_str(&content).unwrap_or_else(|e| panic!("failed to parse resolution fixture: {}", e))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixture_loads() {
        let fixture = load_resolution_fixture();
        assert!(
            !fixture.cases.is_empty(),
            "resolution fixture must have cases"
        );
    }

    #[test]
    fn test_is_correct_match_exact() {
        assert!(is_correct_match(
            &Some("skill:self-dev".to_string()),
            "skill:self-dev"
        ));
    }

    #[test]
    fn test_is_correct_match_no_match_expected() {
        assert!(is_correct_match(&None, ""));
    }

    #[test]
    fn test_is_correct_match_wrong() {
        assert!(!is_correct_match(
            &Some("skill:qa-review".to_string()),
            "skill:self-dev"
        ));
    }

    #[test]
    fn test_is_correct_match_unexpected_match() {
        assert!(!is_correct_match(&Some("skill:self-dev".to_string()), ""));
    }

    #[test]
    fn test_parse_disambiguation_match() {
        let json = r#"{"match": "skill:self-dev", "confidence": 0.95}"#;
        let result = parse_disambiguation_response(json).unwrap();
        assert_eq!(result.matched, Some("skill:self-dev".to_string()));
        assert!((result.confidence - 0.95).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_disambiguation_null_match() {
        let json = r#"{"match": null, "confidence": 0.0}"#;
        let result = parse_disambiguation_response(json).unwrap();
        assert_eq!(result.matched, None);
    }

    #[test]
    fn test_parse_disambiguation_with_fences() {
        let json = "```json\n{\"match\": null, \"confidence\": 0.0}\n```";
        let result = parse_disambiguation_response(json);
        assert!(result.is_ok());
    }

    #[test]
    fn test_all_cases_have_candidates() {
        let fixture = load_resolution_fixture();
        for case in &fixture.cases {
            assert!(
                !case.candidates.is_empty(),
                "case '{}' must have at least one candidate",
                case.entity_key
            );
        }
    }
}

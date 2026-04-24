//! Extraction evaluation: run the NER extraction prompt against each provider
//! for each sample document and score the results (#762).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use mika_agent::kg::subject_extractor::{APPROVED_ENTITY_TYPES, ExtractionOutput};
use mika_common::llm::LlmProvider;
use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};
use serde::Deserialize;

use super::prompts;
use super::{KgExtractionOutcome, KgProviderSpec};

// ---------------------------------------------------------------------------
// Fixture types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ExtractionFixture {
    samples: Vec<SampleDoc>,
}

#[derive(Debug, Deserialize)]
struct SampleDoc {
    path: String,
    category: String,
    min_entities: u32,
    max_entities: u32,
    #[allow(dead_code)]
    notes: String,
}

// ---------------------------------------------------------------------------
// Extraction runner
// ---------------------------------------------------------------------------

/// Run extraction eval across all providers and sample documents.
///
/// Returns a flat list of per-provider-per-doc outcomes.
pub async fn run_extraction_eval(
    providers: &[(KgProviderSpec, Arc<dyn LlmProvider>)],
) -> Vec<KgExtractionOutcome> {
    let fixture = load_extraction_fixture();
    let repo_root = repo_root();
    let mut outcomes = Vec::new();

    for sample in &fixture.samples {
        let doc_path = repo_root.join(&sample.path);
        let doc_content = match std::fs::read_to_string(&doc_path) {
            Ok(content) => content,
            Err(e) => {
                eprintln!(
                    "[extraction_eval] failed to read {}: {} — skipping",
                    sample.path, e
                );
                // Record error outcome for all providers
                for (spec, _) in providers {
                    outcomes.push(KgExtractionOutcome {
                        provider: spec.display_name.clone(),
                        doc_path: sample.path.clone(),
                        category: sample.category.clone(),
                        entity_count: 0,
                        relationship_count: 0,
                        json_parse_ok: false,
                        name_quality: 0.0,
                        type_quality: 0.0,
                        input_tokens: 0,
                        output_tokens: 0,
                        latency_ms: 0,
                        error: Some(format!("file read error: {}", e)),
                    });
                }
                continue;
            }
        };

        let annotated = prompts::build_annotated_text(&doc_content);
        let (system, user_text) = prompts::build_extraction_prompt(&annotated);

        for (spec, provider) in providers {
            println!("  [extraction] {} x {} ...", spec.display_name, sample.path);

            let outcome =
                run_single_extraction(spec, provider.as_ref(), &system, &user_text, sample).await;
            outcomes.push(outcome);
        }
    }

    outcomes
}

async fn run_single_extraction(
    spec: &KgProviderSpec,
    provider: &dyn LlmProvider,
    system: &str,
    user_text: &str,
    sample: &SampleDoc,
) -> KgExtractionOutcome {
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

            match parse_extraction_response(&text) {
                Ok(output) => {
                    let entity_count = output.entities.len();
                    let relationship_count = output.relationships.len();
                    let name_quality = score_name_quality(&output);
                    let type_quality = score_type_quality(&output);

                    KgExtractionOutcome {
                        provider: spec.display_name.clone(),
                        doc_path: sample.path.clone(),
                        category: sample.category.clone(),
                        entity_count,
                        relationship_count,
                        json_parse_ok: true,
                        name_quality,
                        type_quality,
                        input_tokens,
                        output_tokens,
                        latency_ms,
                        error: None,
                    }
                }
                Err(e) => KgExtractionOutcome {
                    provider: spec.display_name.clone(),
                    doc_path: sample.path.clone(),
                    category: sample.category.clone(),
                    entity_count: 0,
                    relationship_count: 0,
                    json_parse_ok: false,
                    name_quality: 0.0,
                    type_quality: 0.0,
                    input_tokens,
                    output_tokens,
                    latency_ms,
                    error: Some(format!("parse error: {}", e)),
                },
            }
        }
        Err(e) => KgExtractionOutcome {
            provider: spec.display_name.clone(),
            doc_path: sample.path.clone(),
            category: sample.category.clone(),
            entity_count: 0,
            relationship_count: 0,
            json_parse_ok: false,
            name_quality: 0.0,
            type_quality: 0.0,
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

/// Parse the LLM extraction response text as ExtractionOutput.
///
/// Strips markdown code fences if present.
fn parse_extraction_response(text: &str) -> Result<ExtractionOutput, String> {
    let cleaned = text
        .trim()
        .strip_prefix("```json")
        .or_else(|| text.trim().strip_prefix("```"))
        .unwrap_or(text.trim());
    let cleaned = cleaned.strip_suffix("```").unwrap_or(cleaned).trim();

    serde_json::from_str(cleaned).map_err(|e| format!("{}", e))
}

/// Score entity name quality: fraction of entities with valid names
/// (lowercase, underscores only, no colons, no spaces).
fn score_name_quality(output: &ExtractionOutput) -> f64 {
    if output.entities.is_empty() {
        return 1.0; // Vacuously true
    }

    let valid = output
        .entities
        .iter()
        .filter(|e| is_valid_entity_name(&e.name))
        .count();

    valid as f64 / output.entities.len() as f64
}

/// Check if an entity name follows the naming convention.
fn is_valid_entity_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(':')
        && !name.contains(' ')
        && name == name.to_lowercase()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Score type quality: fraction of entities with approved entity types.
fn score_type_quality(output: &ExtractionOutput) -> f64 {
    if output.entities.is_empty() {
        return 1.0;
    }

    let valid = output
        .entities
        .iter()
        .filter(|e| APPROVED_ENTITY_TYPES.contains(&e.entity_type.as_str()))
        .count();

    valid as f64 / output.entities.len() as f64
}

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

fn load_extraction_fixture() -> ExtractionFixture {
    // Fixtures live in docs/solutions/kg/eval-fixtures-2026-04-24/ (repo root).
    let fixture_path =
        repo_root().join("docs/solutions/kg/eval-fixtures-2026-04-24/extraction_sample_docs.toml");
    let content = std::fs::read_to_string(&fixture_path).unwrap_or_else(|e| {
        panic!(
            "failed to read extraction fixture at {:?}: {}",
            fixture_path, e
        )
    });
    toml::from_str(&content).unwrap_or_else(|e| panic!("failed to parse extraction fixture: {}", e))
}

/// Get the repo root (two levels up from CARGO_MANIFEST_DIR which is crates/mika-agent/).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent() // crates/
        .and_then(|p| p.parent()) // repo root
        .expect("failed to resolve repo root")
        .to_path_buf()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixture_loads() {
        let fixture = load_extraction_fixture();
        assert!(
            !fixture.samples.is_empty(),
            "extraction fixture must have samples"
        );
    }

    #[test]
    fn test_sample_doc_paths_exist() {
        let fixture = load_extraction_fixture();
        let root = repo_root();
        for sample in &fixture.samples {
            let path = root.join(&sample.path);
            assert!(
                path.exists(),
                "sample doc not found: {} (resolved to {:?})",
                sample.path,
                path
            );
        }
    }

    #[test]
    fn test_is_valid_entity_name() {
        assert!(is_valid_entity_name("self_dev"));
        assert!(is_valid_entity_name("ci_failure"));
        assert!(is_valid_entity_name("a"));
        assert!(!is_valid_entity_name(""));
        assert!(!is_valid_entity_name("has:colon"));
        assert!(!is_valid_entity_name("has space"));
        assert!(!is_valid_entity_name("HasUpper"));
    }

    #[test]
    fn test_parse_extraction_response_valid() {
        let json = r#"{"entities": [{"type": "skill", "name": "self_dev", "description": "test", "chunk_indices": [0], "confidence": 0.9}], "relationships": []}"#;
        let result = parse_extraction_response(json);
        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(output.entities.len(), 1);
        assert_eq!(output.entities[0].name, "self_dev");
    }

    #[test]
    fn test_parse_extraction_response_with_fences() {
        let json = "```json\n{\"entities\": [], \"relationships\": []}\n```";
        let result = parse_extraction_response(json);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_extraction_response_invalid() {
        let result = parse_extraction_response("not json at all");
        assert!(result.is_err());
    }

    #[test]
    fn test_score_name_quality_perfect() {
        let output = ExtractionOutput {
            entities: vec![mika_agent::kg::subject_extractor::ExtractedEntity {
                entity_type: "skill".to_string(),
                name: "self_dev".to_string(),
                description: None,
                chunk_indices: vec![0],
                confidence: 0.9,
            }],
            relationships: vec![],
        };
        assert!((score_name_quality(&output) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_score_type_quality_invalid_type() {
        let output = ExtractionOutput {
            entities: vec![mika_agent::kg::subject_extractor::ExtractedEntity {
                entity_type: "invalid_type".to_string(),
                name: "test".to_string(),
                description: None,
                chunk_indices: vec![0],
                confidence: 0.9,
            }],
            relationships: vec![],
        };
        assert!((score_type_quality(&output) - 0.0).abs() < f64::EPSILON);
    }
}

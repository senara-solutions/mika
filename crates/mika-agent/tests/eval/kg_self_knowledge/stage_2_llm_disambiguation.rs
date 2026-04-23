//! Scenario 6: Stage 2 — LLM disambiguation.
//!
//! Tests the resolver's Stage 2 LLM disambiguation path. Three ambiguity classes
//! in the unit tier (mock LLM), plus a `skipped_no_llm` variant.
//!
//! ## Ambiguity Classes
//! - **Synonyms:** `skill:qa-review` vs `skill:qa-review-build-callback`
//! - **CaseVariants:** `skill:self-dev` vs `skill:Self-Dev` (low confidence, forces Stage 2)
//! - **skipped_no_llm:** No LLM configured, expects skip
//!
//! ## Hard Assertions
//! - Stage 2 code path executed (resolution written to DB)
//! - `outcome = matched_llm` in resolution log
//! - `confidence = min(extraction_confidence, llm_confidence)`
//!
//! ## Soft Tags
//! - `self-knowledge:disambiguation-correct` — LLM picked expected candidate
//! - `self-knowledge:disambiguation-plausible-alternative` — LLM picked defensible alternative
//!
//! ## Tiers
//! - Unit: mock LLM returning scripted JSON
//! - Integration: real `MIKA_KG_RESOLUTION_MODEL` (gated by env var)
//!
//! Reference: mika#740 D2 scenario 6, D3 tier table

use std::sync::Arc;

use super::kg_fixtures::*;
use mika_agent::kg::entity_resolver::SubjectEntityResolver;
use mika_common::llm::LlmProvider;
use mika_common::llm::mock::{MockLlmProvider, text_response};

/// Helper: create a mock LLM that returns a disambiguation JSON response.
fn mock_disambiguation_llm(matched_key: &str, confidence: f64) -> Arc<dyn LlmProvider> {
    let response_text = format!(r#"{{"match": "{matched_key}", "confidence": {confidence}}}"#,);
    Arc::new(
        MockLlmProvider::builder()
            .response(text_response(&response_text))
            .provider_name("mock-resolver")
            .model_name("mock-resolution-model")
            .build(),
    )
}

/// Synonyms: two similar skill names, LLM picks the correct one.
#[tokio::test]
async fn test_stage_2_synonyms_disambiguation() {
    let db = test_db();
    assert_schema_version(&db).await;

    // Seed two domain candidates
    let qa_review_id = seed_domain_entity(
        &db,
        &DomainEntitySpec {
            entity_type: "skill",
            name: "qa-review",
            properties_json: None,
        },
    )
    .await;

    seed_domain_entity(
        &db,
        &DomainEntitySpec {
            entity_type: "skill",
            name: "qa-review-build-callback",
            properties_json: None,
        },
    )
    .await;

    // Seed subject entity mentioning "qa review" without qualifiers
    // Low confidence to force Stage 2
    let subject_id = seed_subject_entity(
        &db,
        &SubjectEntitySpec {
            entity_type: "skill",
            name: "qa-review",
            confidence: 0.85, // Below 0.9 threshold, forces Stage 2
            properties_json: None,
        },
    )
    .await;

    // Mock LLM picks "skill:qa-review"
    let llm = mock_disambiguation_llm("skill:qa-review", 0.9);

    let resolver = SubjectEntityResolver::new(db.clone(), Some(llm), Some("test-synonyms"));
    let stats = resolver.resolve_pending().await.unwrap();

    assert_eq!(stats.total, 1);
    assert_eq!(
        stats.matched_llm, 1,
        "[synonyms] Should match via LLM disambiguation"
    );
    assert_eq!(
        stats.matched_exact, 0,
        "[synonyms] Should NOT use exact match (low confidence)"
    );

    // Check resolution log
    let log = get_resolution_log(&db, subject_id).await;
    assert!(
        log.is_some(),
        "[synonyms] Resolution log entry should exist"
    );
    let (outcome, model) = log.unwrap();
    assert_eq!(
        outcome, "matched_llm",
        "[synonyms] Outcome should be matched_llm"
    );
    assert_eq!(model.as_deref(), Some("mock-resolution-model"));

    // Check resolution confidence = min(extraction=0.85, llm=0.9)
    let resolution = get_resolution(&db, subject_id).await;
    assert!(
        resolution.is_some(),
        "[synonyms] Resolution row should exist"
    );
    let (domain_id, confidence) = resolution.unwrap();
    assert_eq!(
        domain_id, qa_review_id,
        "[synonyms] Should resolve to qa-review"
    );
    assert!(
        (confidence - 0.85).abs() < f64::EPSILON,
        "[synonyms] Confidence should be min(0.85, 0.9) = 0.85, got {}",
        confidence
    );
}

/// Case variants with low confidence: forces LLM disambiguation even though
/// exact match exists, because extraction confidence is below threshold.
#[tokio::test]
async fn test_stage_2_case_variant_low_confidence() {
    let db = test_db();

    // Seed domain entity
    let domain_id = seed_domain_entity(
        &db,
        &DomainEntitySpec {
            entity_type: "skill",
            name: "self-dev",
            properties_json: None,
        },
    )
    .await;

    // Seed subject with case variant, LOW confidence
    let subject_id = seed_subject_entity(
        &db,
        &SubjectEntitySpec {
            entity_type: "skill",
            name: "Self-Dev",
            confidence: 0.80, // Below threshold, Stage 1 finds exact match but escalates
            properties_json: None,
        },
    )
    .await;

    // Mock LLM confirms the match
    let llm = mock_disambiguation_llm("skill:self-dev", 0.95);

    let resolver = SubjectEntityResolver::new(db.clone(), Some(llm), Some("test-case-variants"));
    let stats = resolver.resolve_pending().await.unwrap();

    assert_eq!(stats.total, 1);
    assert_eq!(
        stats.matched_llm, 1,
        "[case-variants] Should go through LLM due to low confidence"
    );

    let resolution = get_resolution(&db, subject_id).await;
    let (resolved_id, confidence) = resolution.unwrap();
    assert_eq!(
        resolved_id, domain_id,
        "[case-variants] Should resolve to domain entity"
    );
    // min(0.80, 0.95) = 0.80
    assert!(
        (confidence - 0.80).abs() < f64::EPSILON,
        "[case-variants] Confidence should be min(0.80, 0.95) = 0.80, got {}",
        confidence
    );
}

/// No LLM configured: exact match fails (low confidence), should yield `skipped_no_llm`.
#[tokio::test]
async fn test_stage_2_skipped_no_llm() {
    let db = test_db();

    // Seed domain entity
    seed_domain_entity(
        &db,
        &DomainEntitySpec {
            entity_type: "skill",
            name: "qa-review",
            properties_json: None,
        },
    )
    .await;

    seed_domain_entity(
        &db,
        &DomainEntitySpec {
            entity_type: "skill",
            name: "qa-review-build-callback",
            properties_json: None,
        },
    )
    .await;

    // Seed subject with low confidence
    let subject_id = seed_subject_entity(
        &db,
        &SubjectEntitySpec {
            entity_type: "skill",
            name: "qa-review",
            confidence: 0.85,
            properties_json: None,
        },
    )
    .await;

    // No LLM configured
    let resolver = SubjectEntityResolver::new(db.clone(), None, Some("test-no-llm"));
    let stats = resolver.resolve_pending().await.unwrap();

    assert_eq!(stats.total, 1);
    assert_eq!(
        stats.skipped_no_llm, 1,
        "Should skip when no LLM configured"
    );
    assert_eq!(stats.matched_exact, 0);
    assert_eq!(stats.matched_llm, 0);

    let log = get_resolution_log(&db, subject_id).await;
    let (outcome, _) = log.unwrap();
    assert_eq!(outcome, "skipped_no_llm");
}

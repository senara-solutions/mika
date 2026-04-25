//! Scenario 5: Stage 1 — Exact match precision (Resolver).
//!
//! Seeds a case-variant subject entity (`skill:Self-Dev`) and a domain entity
//! (`skill:self-dev`). Runs `resolve_pending()` synchronously and asserts:
//! - `outcome = matched_exact` in `kg_resolutions_log`
//! - `confidence = extraction_confidence` (not clamped by LLM)
//!
//! ## Hard Assertions
//! - Resolution log shows `matched_exact`
//! - Resolution confidence matches extraction confidence
//! - Subject→domain resolution row exists
//!
//! Reference: mika#740 D2 scenario 5, D1 resolver source verified

use super::kg_fixtures::*;
use mika_agent::kg::entity_resolver::SubjectEntityResolver;

#[tokio::test]
async fn test_stage_1_exact_match_case_insensitive() {
    let db = test_db();
    assert_schema_version(&db).await;

    // Seed domain entity (lowercase)
    let domain_id = seed_domain_entity(
        &db,
        &DomainEntitySpec {
            entity_type: "skill",
            name: "self-dev",
            properties_json: None,
        },
    )
    .await;

    // Seed subject entity with case variant (PascalCase)
    // Confidence > 0.9 to trigger immediate exact match
    let subject_id = seed_subject_entity(
        &db,
        &SubjectEntitySpec {
            entity_type: "skill",
            name: "Self-Dev",
            confidence: 0.95,
            properties_json: None,
        },
    )
    .await;

    // Run resolver — no LLM needed for exact match
    let resolver = SubjectEntityResolver::new(
        db.clone(),
        None,
        vec!["0000000000000000".to_string()],
        Some("test-stage-1"),
    );
    let stats = resolver.resolve_pending(u32::MAX).await.unwrap();

    // Hard assertions on stats
    assert_eq!(stats.total, 1, "Should process exactly one pending entity");
    assert_eq!(
        stats.matched_exact, 1,
        "Should match via exact (case-insensitive) match"
    );
    assert_eq!(stats.matched_llm, 0, "Should NOT use LLM for exact match");
    assert_eq!(
        stats.skipped_no_llm, 0,
        "Should NOT skip — exact match found"
    );

    // Check resolution log
    let log = get_resolution_log(&db, subject_id).await;
    assert!(log.is_some(), "Resolution log entry should exist");
    let (outcome, model) = log.unwrap();
    assert_eq!(outcome, "matched_exact", "Outcome should be matched_exact");
    assert!(model.is_none(), "No LLM model used for exact match");

    // Check resolution row
    let resolution = get_resolution(&db, subject_id).await;
    assert!(resolution.is_some(), "Resolution row should exist");
    let (resolved_domain_id, confidence) = resolution.unwrap();
    assert_eq!(
        resolved_domain_id, domain_id,
        "Should resolve to the domain entity"
    );
    assert!(
        (confidence - 0.95).abs() < f64::EPSILON,
        "Confidence should equal extraction confidence (0.95), got {}",
        confidence
    );
}

/// Stage 1 with low extraction confidence (below 0.9 threshold) should
/// escalate to Stage 2 — but with no LLM, should yield `skipped_no_llm`.
#[tokio::test]
async fn test_stage_1_low_confidence_escalates_to_stage_2() {
    let db = test_db();

    // Seed domain entity
    seed_domain_entity(
        &db,
        &DomainEntitySpec {
            entity_type: "skill",
            name: "self-dev",
            properties_json: None,
        },
    )
    .await;

    // Seed subject with confidence BELOW threshold (0.9)
    let subject_id = seed_subject_entity(
        &db,
        &SubjectEntitySpec {
            entity_type: "skill",
            name: "Self-Dev",
            confidence: 0.85, // Below 0.9 threshold
            properties_json: None,
        },
    )
    .await;

    // Run resolver with no LLM — exact match found but low confidence,
    // escalates to Stage 2 which skips because no LLM.
    let resolver = SubjectEntityResolver::new(
        db.clone(),
        None,
        vec!["0000000000000000".to_string()],
        Some("test-stage-1-low"),
    );
    let stats = resolver.resolve_pending(u32::MAX).await.unwrap();

    assert_eq!(stats.total, 1);
    assert_eq!(
        stats.matched_exact, 0,
        "Low confidence should NOT resolve via exact match"
    );
    assert_eq!(
        stats.skipped_no_llm, 1,
        "Should skip to skipped_no_llm when no LLM configured"
    );

    // Check log
    let log = get_resolution_log(&db, subject_id).await;
    let (outcome, _) = log.unwrap();
    assert_eq!(outcome, "skipped_no_llm");
}

/// Discovered types (solution_path, failure_mode, pattern) should skip resolution.
/// Uses `resolve_doc_entities()` since `resolve_pending()` filters by domain types in SQL.
#[tokio::test]
async fn test_stage_1_discovered_type_skipped() {
    let db = test_db();

    // Seed subject with a discovered type (no domain counterpart)
    let subject_id = seed_subject_entity(
        &db,
        &SubjectEntitySpec {
            entity_type: "solution_path",
            name: "ci-fix-workflow",
            confidence: 0.9,
            properties_json: None,
        },
    )
    .await;

    let resolver = SubjectEntityResolver::new(
        db.clone(),
        None,
        vec!["0000000000000000".to_string()],
        Some("test-discovered"),
    );
    // Use resolve_doc_entities which takes explicit IDs (no SQL type filter)
    let stats = resolver
        .resolve_doc_entities(&[subject_id], "test-extraction-trace", u32::MAX)
        .await
        .unwrap();

    assert_eq!(stats.total, 1);
    assert_eq!(
        stats.skipped_discovered, 1,
        "Discovered types should be skipped"
    );
    assert_eq!(stats.matched_exact, 0);

    let log = get_resolution_log(&db, subject_id).await;
    let (outcome, _) = log.unwrap();
    assert_eq!(outcome, "skipped_discovered_type");
}

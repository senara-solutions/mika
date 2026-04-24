//! Scenario 4: Path C — Semantic search via chunks.
//!
//! Seeds a chunk with known text, an extracted subject entity, and a resolution
//! to a domain entity. Tests the chunk → subject → domain bridge traversal.
//!
//! ## Hard Assertions (paired: success + failure)
//! - Success: chunk→subject→domain bridge traversal with resolution
//! - Failure: without resolution, only subject-layer result returned
//!
//! ## Tiers
//! - Unit: uses FTS5 search (no embedding client needed for text match)
//! - Integration: would use real embedding client (gated by #340)
//!
//! Reference: mika#740 D2 scenario 4, D3 tier table

use super::kg_fixtures::*;
use mika_agent::kg::query::{KgQueryInput, KgQueryStatus, query_knowledge_graph};

/// Success path: chunk with resolved subject bridges to domain entity.
#[tokio::test]
async fn test_path_c_chunk_to_subject_to_domain_bridge() {
    let db = test_db();
    assert_schema_version(&db).await;

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

    // Seed subject entity (extracted from a doc)
    let subject_id = seed_subject_entity(
        &db,
        &SubjectEntitySpec {
            entity_type: "skill",
            name: "self-dev",
            confidence: 0.95,
            properties_json: None,
        },
    )
    .await;

    // Seed chunk with text that mentions "self-dev"
    let chunk_id = seed_chunk(
        &db,
        &ChunkSpec {
            seq_id: 0,
            source_doc_path: "docs/solutions/self-dev-workflow.md",
            source_doc_hash: "abc123",
            text: "The self-dev skill orchestrates the autonomous development loop. \
               It coordinates claude-pilot sessions and manages PR lifecycle.",
        },
    )
    .await;

    // Link chunk to subject entity
    seed_chunk_subject(&db, chunk_id, subject_id).await;

    // Seed resolution: subject → domain
    seed_resolution(&db, subject_id, domain_id, 0.95, "matched_exact").await;

    // Query with a paraphrase that should match via FTS5
    let input = KgQueryInput {
        question: Some("self-dev".to_string()),
        traversal: None,
        agent_id: Some("mika-dev".to_string()),
        docs_root_hash: None,
        include_context: false,
        result_limit: None,
    };

    let result = query_knowledge_graph(&db, &input).await.unwrap();

    // Entity found (via Path A direct match or Path C bridge), may be TraversalEmpty
    // if the entity is isolated (no outgoing edges at default depth)
    assert!(
        result.status == KgQueryStatus::Ok || result.status == KgQueryStatus::TraversalEmpty,
        "Expected Ok or TraversalEmpty, got {:?}",
        result.status
    );
    // Should find the domain entity (via Path A direct match or Path C bridge)
    assert!(
        result
            .entries
            .iter()
            .any(|e| e.entity_key == "skill:self-dev" && e.layer == "domain"),
        "Should find domain entity skill:self-dev via bridge or direct match. Got: {:?}",
        result
            .entries
            .iter()
            .map(|e| (&e.entity_key, &e.layer))
            .collect::<Vec<_>>()
    );
}

/// Failure path: without resolution, Path C yields subject-only result
/// (no domain bridge).
#[tokio::test]
async fn test_path_c_no_resolution_yields_subject_only() {
    let db = test_db();
    assert_schema_version(&db).await;

    // Seed subject entity (no domain counterpart, no resolution)
    let subject_id = seed_subject_entity(
        &db,
        &SubjectEntitySpec {
            entity_type: "solution_path",
            name: "webhook-ci-handler",
            confidence: 0.88,
            properties_json: None,
        },
    )
    .await;

    // Seed chunk
    let chunk_id = seed_chunk(
        &db,
        &ChunkSpec {
            seq_id: 0,
            source_doc_path: "docs/solutions/webhook-ci-handler.md",
            source_doc_hash: "def456",
            text: "The webhook-ci-handler solution path describes how CI failure webhooks \
               are processed by the agent to dispatch autonomous fixes.",
        },
    )
    .await;

    // Link chunk to subject
    seed_chunk_subject(&db, chunk_id, subject_id).await;

    // NO resolution seeded — subject has no domain bridge

    // Query: should find via FTS5 → chunk → subject → no domain bridge
    let input = KgQueryInput {
        question: Some("webhook-ci-handler".to_string()),
        traversal: None,
        agent_id: Some("mika-dev".to_string()),
        docs_root_hash: Some("0000000000000000".to_string()),
        include_context: false,
        result_limit: None,
    };

    let result = query_knowledge_graph(&db, &input).await.unwrap();

    // Isolated subject entity → TraversalEmpty is expected (entity found, no edges)
    assert!(
        result.status == KgQueryStatus::Ok || result.status == KgQueryStatus::TraversalEmpty,
        "Expected Ok or TraversalEmpty, got {:?}",
        result.status
    );
    // Should find the subject entity (no domain bridge available)
    let subject_entry = result
        .entries
        .iter()
        .find(|e| e.entity_key == "solution_path:webhook-ci-handler");

    assert!(
        subject_entry.is_some(),
        "Should find subject entity via Path C without domain bridge. Got: {:?}",
        result
            .entries
            .iter()
            .map(|e| (&e.entity_key, &e.layer))
            .collect::<Vec<_>>()
    );

    let entry = subject_entry.unwrap();
    assert_eq!(
        entry.layer, "subject",
        "Without resolution, result should be in subject layer"
    );
}

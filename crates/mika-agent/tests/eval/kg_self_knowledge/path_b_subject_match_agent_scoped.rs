//! Scenario 3: Path B — Subject entity match (agent-scoped).
//!
//! Seeds subject entity `pattern:fabrication-guard` for agent `mika-dev` only.
//! Verifies:
//! - Querying as `mika-dev` returns the subject-layer result
//! - Querying as a different agent returns `starting_entity_missing` (isolation)
//!
//! ## Hard Assertions (paired: success + failure)
//! - Success: mika-dev query returns subject entity with correct key
//! - Failure: different-agent query returns `starting_entity_missing`
//!
//! Reference: mika#740 D2 scenario 3, D6 paired assertions

use super::kg_fixtures::*;
use mika_agent::kg::query::{KgQueryInput, KgQueryStatus, query_knowledge_graph};

#[tokio::test]
async fn test_path_b_subject_match_correct_agent() {
    let db = test_db(); // agent_id = "mika-dev"
    assert_schema_version(&db).await;

    // Seed a subject entity for mika-dev only
    seed_subject_entity(&db, &SubjectEntitySpec {
        entity_type: "pattern",
        name: "fabrication-guard",
        confidence: 0.92,
        properties_json: None,
    }).await;

    // Query as mika-dev (should find it)
    let input = KgQueryInput {
        question: Some("fabrication-guard".to_string()),
        traversal: None,
        agent_id: Some("mika-dev".to_string()),
        include_context: false,
        result_limit: None,
    };

    let result = query_knowledge_graph(&db, &input).await.unwrap();

    // Isolated subject entity with no edges → TraversalEmpty (entity found but no neighbors)
    assert!(
        result.status == KgQueryStatus::Ok || result.status == KgQueryStatus::TraversalEmpty,
        "Expected Ok or TraversalEmpty, got {:?}",
        result.status
    );
    assert!(
        result.entries.iter().any(|e| e.entity_key == "pattern:fabrication-guard"),
        "mika-dev should see its own subject entity"
    );

    let entry = result.entries.iter()
        .find(|e| e.entity_key == "pattern:fabrication-guard")
        .unwrap();
    assert_eq!(entry.layer, "subject");
    assert_eq!(entry.entity_type, "pattern");
}

/// Failure path: querying as a different agent should NOT see mika-dev's subjects.
#[tokio::test]
async fn test_path_b_agent_scoped_isolation() {
    let db = test_db(); // seeds as mika-dev
    assert_schema_version(&db).await;

    // Seed a subject entity for mika-dev
    seed_subject_entity(&db, &SubjectEntitySpec {
        entity_type: "pattern",
        name: "fabrication-guard",
        confidence: 0.92,
        properties_json: None,
    }).await;

    // Query as mika-qa (different agent) — should NOT find it
    let input = KgQueryInput {
        question: Some("fabrication-guard".to_string()),
        traversal: None,
        agent_id: Some("mika-qa".to_string()),
        include_context: false,
        result_limit: None,
    };

    let result = query_knowledge_graph(&db, &input).await.unwrap();

    assert_eq!(
        result.status,
        KgQueryStatus::StartingEntityMissing,
        "Different agent should not see mika-dev's subject entities (agent-scoped isolation)"
    );
    assert!(
        result.entries.is_empty(),
        "No entries should be returned for a different agent"
    );
}

/// Edge case: no agent_id provided should skip Path B entirely.
#[tokio::test]
async fn test_path_b_no_agent_id_skips_subject_layer() {
    let db = test_db();

    // Seed subject entity
    seed_subject_entity(&db, &SubjectEntitySpec {
        entity_type: "pattern",
        name: "fabrication-guard",
        confidence: 0.92,
        properties_json: None,
    }).await;

    // Query without agent_id — Path B won't run
    let input = KgQueryInput {
        question: Some("fabrication-guard".to_string()),
        traversal: None,
        agent_id: None,
        include_context: false,
        result_limit: None,
    };

    let result = query_knowledge_graph(&db, &input).await.unwrap();

    // With no domain entity and no agent_id, Path B is skipped
    assert_eq!(
        result.status,
        KgQueryStatus::StartingEntityMissing,
        "Without agent_id, Path B is skipped and only domain search runs"
    );
}

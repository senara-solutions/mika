//! Scenario 2: Path A — Direct domain entity match.
//!
//! Seeds a domain entity `skill:self-dev` in `kg_entities`, then queries
//! via the KG query engine. Asserts one result with `entity_type=Skill`,
//! `hop=0`, and correct entity key.
//!
//! ## Hard Assertions
//! - Query returns `status: ok`
//! - One result entry with `entity_key = "skill:self-dev"`
//! - `entity_type = "skill"`, `hop = 0`
//! - `entry_method` indicates direct match (path A)
//!
//! Reference: mika#740 D2 scenario 2

use super::kg_fixtures::*;
use mika_agent::kg::query::{KgQueryInput, KgQueryStatus, TraversalInput, query_knowledge_graph};

#[tokio::test]
async fn test_path_a_direct_domain_entity_match() {
    let db = test_db();
    assert_schema_version(&db).await;

    // Seed a domain entity
    seed_domain_entity(
        &db,
        &DomainEntitySpec {
            entity_type: "skill",
            name: "self-dev",
            properties_json: None,
        },
    )
    .await;

    // Query by entity_key (direct lookup, not free-text)
    let input = KgQueryInput {
        question: None,
        traversal: Some(TraversalInput {
            start: Some("skill:self-dev".to_string()),
            follow: None,
            max_depth: Some(0),
        }),
        agent_id: None,
        docs_root_hash: None,
        include_context: false,
        result_limit: None,
    };

    let result = query_knowledge_graph(&db, &input).await.unwrap();

    // Hard assertions
    assert_eq!(result.status, KgQueryStatus::Ok);
    assert_eq!(
        result.entries.len(),
        1,
        "Expected exactly one result for direct domain match"
    );

    let entry = &result.entries[0];
    assert_eq!(entry.entity_key, "skill:self-dev");
    assert_eq!(entry.entity_type, "skill");
    assert_eq!(entry.hop, 0);
    assert_eq!(entry.layer, "domain");
    assert!(
        entry.confidence >= 1.0 - f64::EPSILON,
        "Direct domain match should have confidence ~1.0, got {}",
        entry.confidence
    );
    assert_eq!(
        result.entry_method.as_deref(),
        Some("direct_entity_key"),
        "Entry method should indicate direct entity key lookup"
    );
}

/// Path A via free-text name match (LIKE).
/// An isolated entity (no edges) returns `TraversalEmpty` — the entity was found
/// but no neighbors exist to traverse.
#[tokio::test]
async fn test_path_a_name_match_via_question() {
    let db = test_db();

    let self_dev_id = seed_domain_entity(
        &db,
        &DomainEntitySpec {
            entity_type: "skill",
            name: "self-dev",
            properties_json: None,
        },
    )
    .await;

    // Add a relationship so traversal succeeds
    let dep_id = seed_domain_entity(
        &db,
        &DomainEntitySpec {
            entity_type: "skill",
            name: "claude-pilot",
            properties_json: None,
        },
    )
    .await;
    seed_domain_relationship(&db, self_dev_id, dep_id, "DEPENDS_ON").await;

    // Query via free-text question that matches the name
    let input = KgQueryInput {
        question: Some("self-dev".to_string()),
        traversal: None,
        agent_id: None,
        docs_root_hash: None,
        include_context: false,
        result_limit: None,
    };

    let result = query_knowledge_graph(&db, &input).await.unwrap();

    assert_eq!(result.status, KgQueryStatus::Ok);
    assert!(
        result
            .entries
            .iter()
            .any(|e| e.entity_key == "skill:self-dev"),
        "Path A should find skill:self-dev via name match"
    );

    let entry = result
        .entries
        .iter()
        .find(|e| e.entity_key == "skill:self-dev")
        .unwrap();
    assert_eq!(entry.entity_type, "skill");
    assert_eq!(entry.hop, 0);
    assert_eq!(entry.layer, "domain");
}

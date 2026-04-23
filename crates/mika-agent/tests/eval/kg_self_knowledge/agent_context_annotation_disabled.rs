//! Scenario 7: Agent context annotation — disabled skill.
//!
//! Disables `self-dev` for the test agent via `skill_overrides`, then queries
//! the KG. Asserts the skill is RETURNED (annotated, not filtered) with
//! `agent_context.enabled = false`.
//!
//! ## Hard Assertions
//! - Disabled skill appears in results (not filtered)
//! - `agent_context.enabled == false`
//! - Non-skill entities have no `agent_context`
//!
//! Reference: mika#740 D2 scenario 7, KG query D5 (annotate, don't filter)

use super::kg_fixtures::*;
use mika_agent::kg::query::{KgQueryInput, KgQueryStatus, TraversalInput, query_knowledge_graph};

/// Disabled skill is returned with `agent_context.enabled = false`.
#[tokio::test]
async fn test_disabled_skill_annotated_not_filtered() {
    let db = test_db();
    assert_schema_version(&db).await;

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

    // Disable the skill for mika-dev
    disable_skill(&db, "mika-dev", "self-dev").await;

    // Query with agent_id
    let input = KgQueryInput {
        question: None,
        traversal: Some(TraversalInput {
            start: Some("skill:self-dev".to_string()),
            follow: None,
            max_depth: Some(0),
        }),
        agent_id: Some("mika-dev".to_string()),
        include_context: false,
        result_limit: None,
    };

    let result = query_knowledge_graph(&db, &input).await.unwrap();

    // Hard: skill is RETURNED (not filtered)
    assert_eq!(result.status, KgQueryStatus::Ok);
    assert_eq!(result.entries.len(), 1);

    let entry = &result.entries[0];
    assert_eq!(entry.entity_key, "skill:self-dev");

    // Hard: agent_context exists and shows disabled
    assert!(
        entry.agent_context.is_some(),
        "Disabled skill should have agent_context annotation"
    );
    assert!(
        !entry.agent_context.as_ref().unwrap().enabled,
        "agent_context.enabled should be false for disabled skill"
    );
}

/// Enabled skill (no override) should show `agent_context.enabled = true`.
#[tokio::test]
async fn test_enabled_skill_annotated_true() {
    let db = test_db();

    seed_domain_entity(
        &db,
        &DomainEntitySpec {
            entity_type: "skill",
            name: "build-mika",
            properties_json: None,
        },
    )
    .await;

    // No skill_overrides row — default is enabled

    let input = KgQueryInput {
        question: None,
        traversal: Some(TraversalInput {
            start: Some("skill:build-mika".to_string()),
            follow: None,
            max_depth: Some(0),
        }),
        agent_id: Some("mika-dev".to_string()),
        include_context: false,
        result_limit: None,
    };

    let result = query_knowledge_graph(&db, &input).await.unwrap();
    let entry = &result.entries[0];

    assert!(entry.agent_context.is_some());
    assert!(
        entry.agent_context.as_ref().unwrap().enabled,
        "Default (no override) skill should show enabled=true"
    );
}

/// Non-skill entities should NOT have agent_context.
#[tokio::test]
async fn test_non_skill_entity_no_agent_context() {
    let db = test_db();

    seed_domain_entity(
        &db,
        &DomainEntitySpec {
            entity_type: "problem_type",
            name: "ci_failure",
            properties_json: None,
        },
    )
    .await;

    let input = KgQueryInput {
        question: None,
        traversal: Some(TraversalInput {
            start: Some("problem_type:ci_failure".to_string()),
            follow: None,
            max_depth: Some(0),
        }),
        agent_id: Some("mika-dev".to_string()),
        include_context: false,
        result_limit: None,
    };

    let result = query_knowledge_graph(&db, &input).await.unwrap();
    let entry = &result.entries[0];

    assert!(
        entry.agent_context.is_none(),
        "Non-skill entities should not have agent_context"
    );
}

/// Query without agent_id should NOT include agent_context at all.
#[tokio::test]
async fn test_no_agent_id_no_agent_context() {
    let db = test_db();

    seed_domain_entity(
        &db,
        &DomainEntitySpec {
            entity_type: "skill",
            name: "self-dev",
            properties_json: None,
        },
    )
    .await;

    let input = KgQueryInput {
        question: None,
        traversal: Some(TraversalInput {
            start: Some("skill:self-dev".to_string()),
            follow: None,
            max_depth: Some(0),
        }),
        agent_id: None, // No agent_id
        include_context: false,
        result_limit: None,
    };

    let result = query_knowledge_graph(&db, &input).await.unwrap();
    let entry = &result.entries[0];

    assert!(
        entry.agent_context.is_none(),
        "Without agent_id, no agent_context should be present"
    );
}

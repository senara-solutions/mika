//! Scenario 5: KG-result-ignored (agent answers from training data despite
//! `query_knowledge_graph` returning the correct answer)
//!
//! Routed from #740 D4: the failure mode is response-generation, not
//! query-invocation. The agent has the correct answer available from the KG
//! but ignores it.
//!
//! ## Hard Assertions
//! - Response MUST cite `self-dev` (the correct skill from KG)
//! - Response MUST NOT name unrelated skills from training data
//!   (`github-mcp`, `auto-pr`, `merge-bot`, etc.)
//!
//! ## Tags
//! - `grounding:source-cited-correctly` — agent used KG result
//! - `grounding:training-data-hallucination` (failure) — agent ignored KG result
//!
//! ## Frozen Fixture
//! - `fixtures/kg_result_ignored_pre_fix.json` — pre-fix response naming
//!   `github-mcp` despite KG returning `self-dev`.
//!
//! Reference: mika#740 D4, mika#741 D2 scenario 5

use super::*;

/// Skills that should NOT appear in the response — these are training-data
/// fabrications that don't exist in the Mika skill registry.
const FABRICATED_SKILL_NAMES: &[&str] = &[
    "github-mcp",
    "auto-pr",
    "merge-bot",
    "pr-merge",
    "github-merge",
    "auto-merge",
    "pr-automation",
];

/// Primary test: agent uses KG result (`self-dev`) in its response.
///
/// Mock sequence:
/// 1. Agent calls `query_knowledge_graph` with the user's question
/// 2. Tool returns `self-dev` as the skill that handles PR merges
/// 3. Agent correctly cites `self-dev` in its response
#[tokio::test]
async fn test_agent_uses_kg_result() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Agent queries the knowledge graph
            tool_call_response(
                "query_knowledge_graph",
                json!({"question": "which skill handles PR merges?"}),
            ),
            // Agent correctly uses the KG result
            text_response(
                "Based on the knowledge graph, PR merges are handled by the \
                 **self-dev** skill. It provides the `pr_merge_with_gate` tool \
                 which manages the merge workflow with CI check gates.",
            ),
        ])
        .build()
        .await?;

    // Seed KG state: domain entity for self-dev skill
    let domain_id = kg_fixtures::seed_domain_entity(
        &harness.db,
        &kg_fixtures::DomainEntitySpec {
            entity_type: "skill",
            name: "self-dev",
            properties_json: None,
        },
    )
    .await;

    // Seed subject entity for the problem type
    let subject_id = kg_fixtures::seed_subject_entity(
        &harness.db,
        &kg_fixtures::SubjectEntitySpec {
            entity_type: "problem_type",
            name: "pr-merge-skill-routing",
            confidence: 0.9,
            properties_json: None,
        },
    )
    .await;

    // Seed resolution linking subject to domain
    kg_fixtures::seed_resolution(&harness.db, subject_id, domain_id, 0.85, "matched_llm").await;

    let trace = harness.run("Which skill handles PR merges?").await?;

    // Hard: query_knowledge_graph was called
    assert_tools_include(&trace, &["query_knowledge_graph"]);
    // Hard: output exists
    assert_has_output(&trace);
    // Hard: response cites the correct skill
    grounding_assertions::assert_response_contains(&trace, "self-dev");
    // Hard: response does NOT name fabricated skills
    grounding_assertions::assert_response_forbids(&trace, FABRICATED_SKILL_NAMES);

    Ok(())
}

/// Regression-reproduction test: simulates pre-fix behavior where agent names
/// a fabricated skill from training data despite KG returning `self-dev`.
#[tokio::test]
async fn test_regression_kg_result_ignored_detected() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Agent queries KG (correct behavior so far)
            tool_call_response(
                "query_knowledge_graph",
                json!({"question": "which skill handles PR merges?"}),
            ),
            // Pre-fix behavior: agent ignores KG result and names fabricated skill
            text_response(
                "PR merges are handled by the github-mcp integration, which \
                 provides automated merge capabilities through the GitHub API.",
            ),
        ])
        .build()
        .await?;

    // Note: KG seed is intentionally omitted from the regression test — the mock
    // responses are hardcoded and don't use KG state. The seed is only needed in
    // the primary test where the agent loop reads KG context.

    let trace = harness.run("Which skill handles PR merges?").await?;

    // Verify assert_response_forbids catches the fabricated skill name.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        grounding_assertions::assert_response_forbids(&trace, FABRICATED_SKILL_NAMES);
    }));
    assert!(
        result.is_err(),
        "Pre-fix regression: assert_response_forbids should have caught fabricated \
         skill names in the pre-fix response"
    );

    // Verify assert_response_contains catches that `self-dev` is NOT in the response.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        grounding_assertions::assert_response_contains(&trace, "self-dev");
    }));
    assert!(
        result.is_err(),
        "Pre-fix regression: response should NOT contain 'self-dev' (the correct \
         skill from KG)"
    );

    Ok(())
}

//! Scenario 1: Tool selection — agent calls `query_knowledge_graph` before
//! claiming capability state.
//!
//! Verifies the routing decision: structured capability questions should go to
//! `query_knowledge_graph`, not `get_documentation`.
//!
//! ## Hard Assertions
//! - `query_knowledge_graph` called BEFORE any final text response
//! - `get_documentation` NOT called (structured question, not reference)
//!
//! ## Tags
//! - `self-knowledge:query-invoked` — agent used KG for structured question
//!
//! Reference: mika#740 D2 scenario 1

use super::*;

/// User asks "which skill handles PR merges?" → expect `query_knowledge_graph`
/// called before final text response. Mock returns a valid KG result.
#[tokio::test]
async fn test_tool_selection_prefers_kg_for_capability_question() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response(
                "query_knowledge_graph",
                json!({"question": "which skill handles PR merges?"}),
            ),
            text_response(
                "Based on the knowledge graph, PR merges are handled by the qa-review skill, \
                 which uses the pr_merge_with_gate tool.",
            ),
        ])
        .build()
        .await?;

    let trace = harness.run("Which skill handles PR merges?").await?;

    // Hard: query_knowledge_graph was called
    assert_tools_include(&trace, &["query_knowledge_graph"]);
    // Hard: get_documentation was NOT called (structured question)
    assert_tools_exclude(&trace, &["get_documentation"]);
    // Hard: agent produced output
    assert_has_output(&trace);

    Ok(())
}

/// User asks "what does self-dev depend on?" → expect `query_knowledge_graph`
/// with traversal semantics. Verifies the tool is called for dependency questions.
#[tokio::test]
async fn test_tool_selection_kg_for_dependency_question() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response(
                "query_knowledge_graph",
                json!({"question": "what does self-dev depend on?"}),
            ),
            text_response("The self-dev skill depends on claude-pilot and build-mika skills."),
        ])
        .build()
        .await?;

    let trace = harness.run("What does self-dev depend on?").await?;

    assert_tools_include(&trace, &["query_knowledge_graph"]);
    assert_tools_exclude(&trace, &["get_documentation"]);
    assert_has_output(&trace);

    Ok(())
}

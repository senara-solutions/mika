//! Integration tests: self-knowledge + knowledge graph interaction (#692).
//!
//! These tests verify agent loop integration — that scripted tool call
//! sequences execute correctly, traces are recorded, and step counts match.
//! They do NOT test prompt routing correctness (whether a real LLM would
//! choose the right tool given the system prompt guidance). Prompt routing
//! should be validated via LLM-based evals or manual testing.

use anyhow::Result;
use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;

#[tokio::test]
async fn test_kg_query_for_capability_question() -> Result<()> {
    // When asked "what solves CI failures?", the agent should call
    // query_knowledge_graph (a structural question), not get_documentation.
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response(
                "query_knowledge_graph",
                json!({"question": "what solves CI failures?"}),
            ),
            text_response(
                "Based on the knowledge graph, CI failures are solved by the self-dev-webhook-ci \
                 skill, which depends on the build-mika skill.",
            ),
        ])
        .build()
        .await?;

    let trace = harness.run("What solves CI failures?").await?;

    assert_has_output(&trace);
    assert_tools_include(&trace, &["query_knowledge_graph"]);
    assert_tools_exclude(&trace, &["get_documentation"]);
    Ok(())
}

#[tokio::test]
async fn test_doc_query_for_reference_question() -> Result<()> {
    // When asked "explain the memory model", the agent should call
    // get_documentation (a reference question), not query_knowledge_graph.
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("get_documentation", json!({"topic": "architecture"})),
            text_response("The memory model has three layers: core memory, structured facts, and hybrid search."),
        ])
        .build()
        .await?;

    let trace = harness.run("Explain the memory model.").await?;

    assert_has_output(&trace);
    assert_tools_include(&trace, &["get_documentation"]);
    assert_tools_exclude(&trace, &["query_knowledge_graph"]);
    Ok(())
}

#[tokio::test]
async fn test_chained_kg_then_doc_for_spanning_question() -> Result<()> {
    // When asked a question that spans both KG and docs, the agent should
    // chain: query_knowledge_graph for the structured answer, then
    // get_documentation for the reference material.
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response(
                "query_knowledge_graph",
                json!({"question": "what skill handles CI failures?"}),
            ),
            tool_call_response("get_documentation", json!({"topic": "skills"})),
            text_response(
                "The self-dev-webhook-ci skill handles CI failures. Skills use exec handlers \
                 that run shell commands and support long_running mode for async dispatch.",
            ),
        ])
        .build()
        .await?;

    let trace = harness
        .run("What skill handles CI failures and how do skills work?")
        .await?;

    assert_has_output(&trace);
    assert_tools_include(&trace, &["query_knowledge_graph", "get_documentation"]);
    assert_exact_steps(&trace, 3);
    Ok(())
}

#[tokio::test]
async fn test_kg_starting_entity_missing_triggers_fallback() -> Result<()> {
    // When query_knowledge_graph returns starting_entity_missing, the agent
    // should fall back to list_skills (for a "what skills" question).
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response(
                "query_knowledge_graph",
                json!({"question": "what skills do I have?", "agent_id": "test-agent"}),
            ),
            // After seeing starting_entity_missing in the tool result,
            // the agent falls back to list_skills
            tool_call_response("list_skills", json!({})),
            text_response(
                "You're not yet in the knowledge graph. Based on the live registry, \
                 you have the following skills: self-knowledge, git-ops, shell-exec.",
            ),
        ])
        .build()
        .await?;

    let trace = harness.run("What skills do I have?").await?;

    assert_has_output(&trace);
    assert_tools_include(&trace, &["query_knowledge_graph", "list_skills"]);
    // The response should mention the registry source
    assert_output_contains(&trace, "registry");
    Ok(())
}

#[tokio::test]
async fn test_kg_traversal_empty_no_fallback() -> Result<()> {
    // When query_knowledge_graph returns traversal_empty, the agent should
    // NOT fall back — it should trust the KG and say so.
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response(
                "query_knowledge_graph",
                json!({"question": "what solves fabrication?"}),
            ),
            text_response(
                "I found the fabrication problem type in the knowledge graph, but \
                 it has no connected solution paths or skills yet.",
            ),
        ])
        .build()
        .await?;

    let trace = harness.run("What solves fabrication?").await?;

    assert_has_output(&trace);
    assert_tools_include(&trace, &["query_knowledge_graph"]);
    // Should NOT fall back to any registry tool
    assert_tools_exclude(&trace, &["list_skills"]);
    assert_exact_steps(&trace, 2);
    Ok(())
}

#[tokio::test]
async fn test_kg_query_with_agent_context() -> Result<()> {
    // When the agent queries with agent_id, the response should include
    // agent_context metadata and the agent should interpret enabled status.
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response(
                "query_knowledge_graph",
                json!({
                    "traversal": {"start": "skill:self-dev"},
                    "agent_id": "mika-dev"
                }),
            ),
            text_response(
                "The self-dev skill is enabled for mika-dev. It depends on \
                 claude-pilot and build-mika skills.",
            ),
        ])
        .build()
        .await?;

    let trace = harness.run("Tell me about the self-dev skill").await?;

    assert_has_output(&trace);
    assert_tools_include(&trace, &["query_knowledge_graph"]);
    Ok(())
}

#[tokio::test]
async fn test_disabled_skill_mentioned_in_response() -> Result<()> {
    // When KG results include a skill with enabled=false, the agent should
    // explicitly mention it's disabled rather than hiding it.
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response(
                "query_knowledge_graph",
                json!({"question": "what skills handle PR reviews?", "agent_id": "mika-dev"}),
            ),
            text_response(
                "The qa-review skill handles PR reviews, but it is currently disabled \
                 for mika-dev. The skill-review skill is also available and enabled.",
            ),
        ])
        .build()
        .await?;

    let trace = harness.run("What skills handle PR reviews?").await?;

    assert_has_output(&trace);
    assert_tools_include(&trace, &["query_knowledge_graph"]);
    // Response should mention the disabled state
    assert_output_contains(&trace, "disabled");
    Ok(())
}

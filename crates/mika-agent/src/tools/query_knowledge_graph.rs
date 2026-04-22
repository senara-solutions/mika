//! `query_knowledge_graph` — Read-only KG traversal tool.
//!
//! Lets agents discover capabilities, find solution paths, and reason about
//! their environment by querying the three-layer knowledge graph (domain,
//! lexical, subject). See `docs/plans/2026-04-21-008-feat-kg-query-tool-plan.md`.

use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};
use crate::kg::query::{KgQueryInput, KgQueryStatus, TraversalInput, query_knowledge_graph};

pub struct QueryKnowledgeGraphTool;

#[async_trait]
impl Tool for QueryKnowledgeGraphTool {
    fn name(&self) -> &str {
        "query_knowledge_graph"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "query_knowledge_graph".to_string(),
            description: "Traverse the knowledge graph to discover capabilities, find solution \
                paths, and reason about your environment. Searches KG entities and relationships \
                across domain (skills, tools, agents, problem types) and subject (extracted \
                entities and fact triples) layers. Use 'question' for free-text queries or \
                'traversal.start' with a known entity_key for direct graph traversal. \
                For documentation content, use get_documentation instead."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "Free-text question to find relevant entities (e.g., 'what solves CI failures?'). Mutually exclusive with traversal.start."
                    },
                    "traversal": {
                        "type": "object",
                        "description": "Explicit traversal parameters.",
                        "properties": {
                            "start": {
                                "type": "string",
                                "description": "Starting entity_key for direct traversal (e.g., 'problem_type:ci_failure'). Mutually exclusive with question."
                            },
                            "follow": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Relationship types to follow. Default: SOLVED_BY, PROVIDES, DEPENDS_ON, USES, CALLS."
                            },
                            "max_depth": {
                                "type": "integer",
                                "description": "Max traversal depth (0-4). Default: 2."
                            },
                            "cross_layer": {
                                "type": "boolean",
                                "description": "Enable cross-layer hops between domain and subject layers. Default: false."
                            }
                        }
                    },
                    "agent_id": {
                        "type": "string",
                        "description": "Agent ID for scoped queries and skill enablement context."
                    },
                    "include_context": {
                        "type": "boolean",
                        "description": "Include chunk prose text in results. Default: false."
                    },
                    "result_limit": {
                        "type": "integer",
                        "description": "Max entities to return (1-20). Default: 20."
                    }
                }
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        // Parse question if present, validate length
        let question = input["question"].as_str().map(|s| s.to_string());
        if let Some(q) = &question
            && q.len() > MAX_INPUT_LEN
        {
            return Ok(ToolOutput::error(format!(
                "'question' too long: {} characters (max: {MAX_INPUT_LEN})",
                q.len()
            )));
        }

        // Parse traversal if present
        let traversal = if input["traversal"].is_object() {
            let start = input["traversal"]["start"].as_str().map(|s| s.to_string());
            let follow = input["traversal"]["follow"].as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            });
            let max_depth = input["traversal"]["max_depth"].as_u64().map(|d| d as u32);
            let cross_layer = input["traversal"]["cross_layer"].as_bool().unwrap_or(false);

            Some(TraversalInput {
                start,
                follow,
                max_depth,
                cross_layer,
            })
        } else {
            None
        };

        let agent_id = input["agent_id"].as_str().map(|s| s.to_string());
        let include_context = input["include_context"].as_bool().unwrap_or(false);
        let result_limit = input["result_limit"].as_u64().map(|l| l as usize);

        let query_input = KgQueryInput {
            question,
            traversal,
            agent_id,
            include_context,
            result_limit,
        };

        match query_knowledge_graph(ctx.db, &query_input).await {
            Ok(result) => {
                let json = serde_json::to_string_pretty(&result)
                    .unwrap_or_else(|e| format!("{{\"error\": \"serialization failed: {e}\"}}"));

                match result.status {
                    KgQueryStatus::Ok => Ok(ToolOutput::success(json)),
                    KgQueryStatus::StartingEntityMissing => Ok(ToolOutput::success(json)),
                    KgQueryStatus::TraversalEmpty => Ok(ToolOutput::success(json)),
                }
            }
            Err(e) => Ok(ToolOutput::error(format!(
                "Knowledge graph query failed: {e}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    #[tokio::test]
    async fn test_tool_definition_has_correct_name() {
        let tool = QueryKnowledgeGraphTool;
        assert_eq!(tool.name(), "query_knowledge_graph");
        let def = tool.definition();
        assert_eq!(def.name, "query_knowledge_graph");
    }

    #[tokio::test]
    async fn test_empty_kg_returns_starting_entity_missing() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = QueryKnowledgeGraphTool;

        let result = tool
            .execute(
                serde_json::json!({"question": "what solves CI failures?"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("starting_entity_missing"));
    }

    #[tokio::test]
    async fn test_question_too_long() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = QueryKnowledgeGraphTool;

        let long_question = "a".repeat(MAX_INPUT_LEN + 1);
        let result = tool
            .execute(serde_json::json!({"question": long_question}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("too long"));
    }

    #[tokio::test]
    async fn test_both_question_and_start_returns_error() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = QueryKnowledgeGraphTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "question": "test",
                    "traversal": { "start": "problem_type:ci_failure" }
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Exactly one"));
    }

    #[tokio::test]
    async fn test_traversal_with_direct_start() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();

        // Seed a domain entity
        let db = &harness.db;
        db.with_db(|db| {
            db.conn.execute(
                "INSERT INTO kg_entities (entity_key, type, name) VALUES ('agent:test', 'agent', 'test')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let tool = QueryKnowledgeGraphTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "traversal": {
                        "start": "agent:test",
                        "max_depth": 0
                    }
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("agent:test"));
    }

    #[tokio::test]
    async fn test_agent_scoped_query() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();

        // Seed a skill domain entity
        let db = &harness.db;
        db.with_db(|db| {
            db.conn.execute(
                "INSERT INTO kg_entities (entity_key, type, name) VALUES ('skill:test-skill', 'skill', 'test-skill')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let tool = QueryKnowledgeGraphTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "traversal": {
                        "start": "skill:test-skill",
                        "max_depth": 0
                    },
                    "agent_id": "mika"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("agent_context"));
        assert!(result.content.contains("\"enabled\": true"));
    }
}

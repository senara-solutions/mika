//! JSON-schema divergence response-handling tests (D4).
//!
//! Mock-based tests verifying Mika correctly handles out-of-schema LLM responses.
//! Documents per-adapter tolerance for various schema violation classes.
//! No `#[ignore]` — runs in CI on every push.

use anyhow::Result;
use async_trait::async_trait;
use mika_agent::tools::{Tool, ToolContext, ToolOutput, ToolRegistry};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::{Value, json};

use super::assertions::*;
use super::harness::EvalHarness;

/// A synthetic test tool with a strict schema for testing schema divergence behavior.
///
/// Schema: `command` (string, required), `priority` (string, enum: low/medium/high).
struct SchemaTestTool;

#[async_trait]
impl Tool for SchemaTestTool {
    fn name(&self) -> &str {
        "schema_test_tool"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "schema_test_tool".to_string(),
            description: "Test tool for schema divergence scenarios.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The command to execute."
                    },
                    "priority": {
                        "type": "string",
                        "description": "Priority level.",
                        "enum": ["low", "medium", "high"]
                    }
                },
                "required": ["command"]
            }),
        }
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("");

        if command.is_empty() {
            return Ok(ToolOutput::error("missing required field: command"));
        }

        Ok(ToolOutput::success(format!("executed: {command}")))
    }
}

fn schema_test_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(SchemaTestTool));
    registry
}

#[tokio::test]
async fn test_well_formed_tool_call_succeeds() {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response(
                "schema_test_tool",
                json!({"command": "test", "priority": "high"}),
            ),
            text_response("Done."),
        ])
        .tools(schema_test_registry())
        .build()
        .await
        .unwrap();

    let trace = harness.run("Run a test").await.unwrap();

    assert_has_output(&trace);
    assert_tools_include(&trace, &["schema_test_tool"]);
    assert_tool_output_contains(&trace, "schema_test_tool", 0, "executed: test");
}

#[tokio::test]
async fn test_unknown_fields_tolerated() {
    // Mika's tool dispatch uses serde for deserialization, which by default
    // ignores unknown fields. This documents that extra fields from the LLM
    // are tolerated at the Mika dispatch layer (provider-side rejection is
    // a separate concern tested by the real-API matrix).
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response(
                "schema_test_tool",
                json!({"command": "test", "extra_field": "surprise"}),
            ),
            text_response("Done with extras."),
        ])
        .tools(schema_test_registry())
        .build()
        .await
        .unwrap();

    let trace = harness.run("Run with extra fields").await.unwrap();

    assert_has_output(&trace);
    assert_tools_include(&trace, &["schema_test_tool"]);
    // Tool should still execute successfully — extra fields ignored
    assert_tool_output_contains(&trace, "schema_test_tool", 0, "executed: test");
}

#[tokio::test]
async fn test_missing_required_field_produces_error() {
    // LLM omits the required `command` field. The tool itself detects
    // the missing field and returns a structured error.
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("schema_test_tool", json!({"priority": "high"})),
            text_response("I see the command field was missing, let me try again."),
        ])
        .tools(schema_test_registry())
        .build()
        .await
        .unwrap();

    let trace = harness.run("Run without command").await.unwrap();

    assert_has_output(&trace);
    assert_tools_include(&trace, &["schema_test_tool"]);
    // Tool should return an error about the missing field
    assert_tool_output_contains(&trace, "schema_test_tool", 0, "missing required field");
}

#[tokio::test]
async fn test_out_of_enum_value_accepted() {
    // LLM sends an enum value ("urgent") not in the declared schema
    // enum (["low", "medium", "high"]). Mika's dispatch layer does NOT
    // validate enum values — it passes the raw JSON to the tool. This
    // documents that Mika is permissive on enum values; provider-side
    // rejection behavior is tested by the real-API matrix.
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response(
                "schema_test_tool",
                json!({"command": "test", "priority": "urgent"}),
            ),
            text_response("Done with non-standard priority."),
        ])
        .tools(schema_test_registry())
        .build()
        .await
        .unwrap();

    let trace = harness.run("Run with urgent priority").await.unwrap();

    assert_has_output(&trace);
    assert_tools_include(&trace, &["schema_test_tool"]);
    // Tool still executes — Mika doesn't enforce enum values
    assert_tool_output_contains(&trace, "schema_test_tool", 0, "executed: test");
}

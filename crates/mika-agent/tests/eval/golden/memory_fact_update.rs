//! Golden scenario: Search for an existing fact, then update it.
//!
//! Class: Memory | Expected tokens: 2000

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "memory_fact_update",
        GoldenScenarioMeta {
            class: ScenarioClass::Memory,
            expected_tokens: 2000,
            description: "Agent searches for existing fact, then updates it with new information",
        },
    );
}

#[tokio::test]
async fn test_memory_fact_update() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent searches for the existing fact
            tool_call_response("search_memory", json!({"query": "project deadline"})),
            // Step 2: Agent updates the fact with new deadline
            tool_call_response(
                "update_fact",
                json!({"fact_id": "1", "content": "Project deadline moved to May 15th"}),
            ),
            // Step 3: Agent confirms the update
            text_response("I've updated the project deadline from April 30th to May 15th."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("The project deadline has been moved to May 15th, please update it.")
        .await
        .unwrap();

    // Hard assertions: search first, then update
    assert_tools_include(&trace, &["search_memory", "update_fact"]);
    assert_tool_order(&trace, &["search_memory", "update_fact"]);
    assert_has_output(&trace);
}

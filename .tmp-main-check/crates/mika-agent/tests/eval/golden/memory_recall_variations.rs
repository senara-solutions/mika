//! Golden scenario: Query a fact using a natural language variation.
//!
//! Class: Memory | Expected tokens: 2000

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "memory_recall_variations",
        GoldenScenarioMeta {
            class: ScenarioClass::Memory,
            expected_tokens: 2000,
            description: "Store a fact about John's birthday, query with natural language variation",
        },
    );
}

#[tokio::test]
async fn test_memory_recall_variations() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent searches memory with a relevant query about John
            tool_call_response("search_memory", json!({"query": "John birthday"})),
            // Step 2: Agent responds with the birthday info
            text_response("John's birthday is on March 15th."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("When is John's birthday?").await.unwrap();

    // Hard assertions
    assert_tools_include(&trace, &["search_memory"]);
    assert_has_output(&trace);
    assert_output_contains(&trace, "March");
}

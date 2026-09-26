//! Golden scenario: Store a fact, then query and recall it.
//!
//! Class: Memory | Expected tokens: 1500

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "memory_recall_single_fact",
        GoldenScenarioMeta {
            class: ScenarioClass::Memory,
            expected_tokens: 1500,
            description: "Store a fact, then query for it and verify recall",
        },
    );
}

#[tokio::test]
async fn test_memory_recall_single_fact() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent searches memory for the stored fact
            tool_call_response("search_memory", json!({"query": "project deadline"})),
            // Step 2: Agent responds with the recalled fact
            text_response("Based on my records, the project deadline is April 30th."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("What is the project deadline?").await.unwrap();

    // Hard assertions
    assert_tools_include(&trace, &["search_memory"]);
    assert_has_output(&trace);
    assert_output_contains(&trace, "April 30th");
}

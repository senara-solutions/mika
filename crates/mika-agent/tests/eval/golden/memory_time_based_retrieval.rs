//! Golden scenario: Query about recent events triggers time-based memory search.
//!
//! Class: Memory | Expected tokens: 2000

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "memory_time_based_retrieval",
        GoldenScenarioMeta {
            class: ScenarioClass::Memory,
            expected_tokens: 2000,
            description: "Query about recent events uses search_memory with time-related terms",
        },
    );
}

#[tokio::test]
async fn test_memory_time_based_retrieval() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent searches memory with a time-related query
            tool_call_response("search_memory", json!({"query": "last week discussions"})),
            // Step 2: Agent responds with what it found
            text_response(
                "Last week you discussed the Q2 roadmap and the upcoming product launch.",
            ),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("What did I discuss last week?").await.unwrap();

    // Hard assertions
    assert_tools_include(&trace, &["search_memory"]);
    assert_has_output(&trace);
}

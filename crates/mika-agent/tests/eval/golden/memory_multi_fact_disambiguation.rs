//! Golden scenario: Multiple facts stored, ambiguous query requires disambiguation.
//!
//! Class: Memory | Expected tokens: 2500

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "memory_multi_fact_disambiguation",
        GoldenScenarioMeta {
            class: ScenarioClass::Memory,
            expected_tokens: 2500,
            description: "Multiple facts about different people; ambiguous query about meetings",
        },
    );
}

#[tokio::test]
async fn test_memory_multi_fact_disambiguation() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent searches for meeting-related facts
            tool_call_response("search_memory", json!({"query": "meetings"})),
            // Step 2: Agent synthesizes results from multiple facts
            text_response(
                "I found several meeting-related notes: you have a standup with Alice \
                 on Mondays and a 1:1 with Bob on Wednesdays.",
            ),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("What do I know about meetings?").await.unwrap();

    // Hard assertions
    assert_tools_include(&trace, &["search_memory"]);
    assert_has_output(&trace);
}

//! Golden scenario: Search memory for upcoming meetings.
//!
//! Class: ToolSelection | Expected tokens: 1500

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "tool_selection_memory_search",
        GoldenScenarioMeta {
            class: ScenarioClass::ToolSelection,
            expected_tokens: 1500,
            description: "User asks about upcoming meetings; agent searches memory",
        },
    );
}

#[tokio::test]
async fn test_tool_selection_memory_search() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent searches memory for meetings
            tool_call_response("search_memory", json!({"query": "meetings upcoming"})),
            // Step 2: Agent responds with search results
            text_response("Based on your stored information, you have a team sync on Monday and a 1:1 with Alex on Wednesday."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("What meetings do I have coming up?")
        .await
        .unwrap();

    // Hard assertions
    assert_tools_include(&trace, &["search_memory"]);
    assert_has_output(&trace);
}

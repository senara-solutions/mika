//! Golden scenario: Multi-step tool sequence — search then store.
//!
//! Class: ToolSelection | Expected tokens: 3000

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "tool_selection_multi_step_sequence",
        GoldenScenarioMeta {
            class: ScenarioClass::ToolSelection,
            expected_tokens: 3000,
            description: "User asks to find info then store it; agent calls search_memory then store_fact in order",
        },
    );
}

#[tokio::test]
async fn test_tool_selection_multi_step_sequence() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent searches memory for John's email
            tool_call_response("search_memory", json!({"query": "John email address"})),
            // Step 2: Agent stores the found email as a fact
            tool_call_response(
                "store_fact",
                json!({
                    "category": "person",
                    "content": "John's email is john@example.com"
                }),
            ),
            // Step 3: Agent confirms completion
            text_response("I found John's email (john@example.com) and stored it as a fact for future reference."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("Find John's email and then store it as a fact")
        .await
        .unwrap();

    // Hard assertions: both tools called in correct order
    assert_tools_include(&trace, &["search_memory", "store_fact"]);
    assert_tool_order(&trace, &["search_memory", "store_fact"]);
    assert_has_output(&trace);
}

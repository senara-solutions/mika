//! Golden scenario: Disambiguate between store_fact and update_core_memory.
//!
//! Class: ToolSelection | Expected tokens: 2500

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "tool_selection_conflict_two_plausible",
        GoldenScenarioMeta {
            class: ScenarioClass::ToolSelection,
            expected_tokens: 2500,
            description: "User shares a preference about a person; agent should use store_fact (not update_core_memory)",
        },
    );
}

#[tokio::test]
async fn test_tool_selection_conflict_two_plausible() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent correctly stores this as a fact (granular preference)
            tool_call_response(
                "store_fact",
                json!({
                    "category": "preference",
                    "content": "John prefers morning meetings"
                }),
            ),
            // Step 2: Agent confirms storage
            text_response("Noted! I've saved that John prefers morning meetings."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("Remember that John prefers morning meetings")
        .await
        .unwrap();

    // Hard assertions: store_fact is the correct choice for a new individual preference
    assert_tools_include(&trace, &["store_fact"]);
    assert_tools_exclude(&trace, &["update_core_memory"]);
    assert_has_output(&trace);
}

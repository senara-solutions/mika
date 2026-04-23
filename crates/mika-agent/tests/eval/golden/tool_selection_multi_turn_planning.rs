//! Golden scenario: Multi-turn planning — search context then set reminder.
//!
//! Class: ToolSelection | Expected tokens: 3500

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "tool_selection_multi_turn_planning",
        GoldenScenarioMeta {
            class: ScenarioClass::ToolSelection,
            expected_tokens: 3500,
            description: "Turn 1: search memory for context; Turn 2: create a reminder based on the plan",
        },
    );
}

#[tokio::test]
async fn test_tool_selection_multi_turn_planning() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1, Step 1: Agent searches memory for existing meeting context
            tool_call_response("search_memory", json!({"query": "project kickoff meeting"})),
            // Turn 1, Step 2: Agent responds with a plan
            text_response("I checked your records and don't see an existing kickoff meeting scheduled. Here's a suggested plan:\n1. Send agenda to stakeholders\n2. Book a conference room\n3. Prepare the project brief\nWould you like me to help with any of these steps?"),
        ])
        .build()
        .await
        .unwrap();

    // Turn 1: Plan the meeting
    let trace1 = harness
        .run("Help me plan a project kickoff meeting")
        .await
        .unwrap();

    assert_tools_include(&trace1, &["search_memory"]);
    assert_has_output(&trace1);

    // Turn 2: Set a reminder for the meeting
    let trace2 = harness
        .run_turn(
            "Now remind me about it next Monday",
            vec![
                // Turn 2, Step 1: Agent creates a reminder
                tool_call_response(
                    "create_reminder",
                    json!({
                        "time": "next Monday",
                        "message": "Project kickoff meeting"
                    }),
                ),
                // Turn 2, Step 2: Agent confirms
                text_response(
                    "I've set a reminder for next Monday about the project kickoff meeting.",
                ),
            ],
        )
        .await
        .unwrap();

    assert_tools_include(&trace2, &["create_reminder"]);
    assert_has_output(&trace2);
    assert_output_contains(&trace2, "reminder");
}

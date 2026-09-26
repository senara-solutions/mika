//! Golden scenario: Create a reminder for a future event.
//!
//! Class: ToolSelection | Expected tokens: 2000

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "tool_selection_calendar_create",
        GoldenScenarioMeta {
            class: ScenarioClass::ToolSelection,
            expected_tokens: 2000,
            description: "User asks to be reminded about a standup; agent calls create_reminder",
        },
    );
}

#[tokio::test]
async fn test_tool_selection_calendar_create() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls create_reminder with time and message
            tool_call_response(
                "create_reminder",
                json!({
                    "time": "tomorrow 9am",
                    "message": "Team standup meeting"
                }),
            ),
            // Step 2: Agent confirms the reminder was set
            text_response(
                "Done! I've set a reminder for tomorrow at 9am about the team standup meeting.",
            ),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("Remind me about the team standup tomorrow at 9am")
        .await
        .unwrap();

    // Hard assertions
    assert_tools_include(&trace, &["create_reminder"]);
    assert_has_output(&trace);
    assert_output_contains(&trace, "reminder");
}

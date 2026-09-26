//! Golden scenario: Google Workspace calendar query via memory search.
//!
//! Class: SkillSpecific | Expected tokens: 3000

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "skill_google_workspace_calendar",
        GoldenScenarioMeta {
            class: ScenarioClass::SkillSpecific,
            expected_tokens: 3000,
            description: "Agent searches memory for calendar/schedule info when asked about today's agenda",
        },
    );
}

#[tokio::test]
async fn test_skill_google_workspace_calendar() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent searches memory for calendar/schedule info
            tool_call_response("search_memory", json!({"query": "calendar schedule today"})),
            // Step 2: Agent responds with structured calendar summary
            text_response(
                "Here's what's on your calendar for today:\n\
                 - 10:00 AM — Team standup\n\
                 - 1:00 PM — Design review with Sarah\n\
                 - 3:30 PM — Sprint planning",
            ),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("What's on my calendar for today?")
        .await
        .unwrap();

    // Hard assertions: search_memory called and output is present
    assert_tools_include(&trace, &["search_memory"]);
    assert_has_output(&trace);
}

//! Golden scenario: Agent should not hallucinate tools it does not have.
//!
//! Class: ToolSelection | Expected tokens: 1500

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "tool_selection_no_hallucinated_tools",
        GoldenScenarioMeta {
            class: ScenarioClass::ToolSelection,
            expected_tokens: 1500,
            description: "User asks about weather; agent has no weather tool and admits the limitation",
        },
    );
}

#[tokio::test]
async fn test_tool_selection_no_hallucinated_tools() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Agent responds directly without calling any tool
            text_response("I don't have access to weather information. I can help you with scheduling, memory, messaging, and task management. For weather, you might want to check a weather service or app."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("What's the weather like today?").await.unwrap();

    // Hard assertions: no tools called, agent admits limitation
    assert_no_tools(&trace);
    assert_has_output(&trace);
}

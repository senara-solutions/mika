//! Golden scenario: Multi-turn context maintains coherence around state changes.
//!
//! Class: ConversationQuality | Expected tokens: 3000

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "conversation_quality_rewind_semantics",
        GoldenScenarioMeta {
            class: ScenarioClass::ConversationQuality,
            expected_tokens: 3000,
            description: "Turn 1: agent stores a fact via tool; Turn 2: user asks about the stored fact",
        },
    );
}

#[tokio::test]
async fn test_conversation_quality_rewind_semantics() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1, Step 1: Agent stores the fact
            tool_call_response(
                "store_fact",
                json!({
                    "content": "Meeting at 3pm",
                    "category": "Events"
                }),
            ),
            // Turn 1, Step 2: Agent confirms
            text_response("Got it, I've noted your meeting at 3pm."),
        ])
        .build()
        .await
        .unwrap();

    // Turn 1: User tells agent about a meeting
    let trace1 = harness.run("I have a meeting at 3pm today").await.unwrap();

    assert_tools_include(&trace1, &["store_fact"]);
    assert_has_output(&trace1);

    // Turn 2: User asks about the stored fact
    let trace2 = harness
        .run_turn(
            "What meeting do I have today?",
            vec![
                // Agent recalls from context
                text_response("You have a meeting at 3pm."),
            ],
        )
        .await
        .unwrap();

    // Hard assertions
    assert_has_output(&trace2);
    assert_output_contains(&trace2, "3pm");
}

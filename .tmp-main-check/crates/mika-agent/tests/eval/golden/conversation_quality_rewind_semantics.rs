//! Golden scenario: Agent handles correction/undo requests gracefully.
//!
//! Tests that when a user says "actually, undo that" or "that was wrong",
//! the agent acknowledges the correction and takes appropriate action
//! (e.g., updating or searching for the fact to correct it).
//!
//! Class: ConversationQuality | Expected tokens: 3000

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "conversation_quality_rewind_semantics",
        GoldenScenarioMeta {
            class: ScenarioClass::ConversationQuality,
            expected_tokens: 3000,
            description: "Agent handles correction/undo: stores fact, then user corrects it",
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
                    "category": "commitment",
                    "description": "Meeting at 3pm"
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

    // Turn 2: User corrects — "actually that was wrong, it's at 4pm"
    // Agent should acknowledge the correction and search for the original fact
    let trace2 = harness
        .run_turn(
            "Actually, I got that wrong. The meeting is at 4pm, not 3pm.",
            vec![
                // Agent searches for the existing fact to correct it
                tool_call_response("search_memory", json!({"query": "meeting 3pm"})),
                // Agent stores the corrected fact
                tool_call_response(
                    "store_fact",
                    json!({
                        "category": "commitment",
                        "description": "Meeting at 4pm"
                    }),
                ),
                // Agent confirms the correction
                text_response("Updated! Your meeting is now at 4pm, not 3pm."),
            ],
        )
        .await
        .unwrap();

    // Hard assertions: agent should search for the original and store correction
    assert_tools_include(&trace2, &["search_memory"]);
    assert_has_output(&trace2);
    assert_output_contains(&trace2, "4pm");
}

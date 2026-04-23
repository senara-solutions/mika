//! Golden scenario: Important context survives across multiple turns with compaction enabled.
//!
//! Class: ConversationQuality | Expected tokens: 3500

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "conversation_quality_compaction_survival",
        GoldenScenarioMeta {
            class: ScenarioClass::ConversationQuality,
            expected_tokens: 3500,
            description: "Store a deadline in turn 1, recall it in turn 2 with compaction enabled",
        },
    );
}

#[tokio::test]
async fn test_conversation_quality_compaction_survival() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1, Step 1: Agent stores the deadline
            tool_call_response(
                "store_fact",
                json!({
                    "content": "Project deadline December 1st",
                    "category": "Commitments"
                }),
            ),
            // Turn 1, Step 2: Agent confirms
            text_response("Noted!"),
        ])
        .skip_compaction(false)
        .build()
        .await
        .unwrap();

    // Turn 1: User tells agent about a deadline
    let trace1 = harness
        .run("My project deadline is December 1st")
        .await
        .unwrap();

    assert_tools_include(&trace1, &["store_fact"]);
    assert_has_output(&trace1);

    // Turn 2: User queries the stored deadline
    let trace2 = harness
        .run_turn(
            "What's my project deadline?",
            vec![
                // Turn 2, Step 1: Agent searches memory for the deadline
                tool_call_response("search_memory", json!({"query": "project deadline"})),
                // Turn 2, Step 2: Agent responds with the recalled deadline
                text_response("Your project deadline is December 1st."),
            ],
        )
        .await
        .unwrap();

    // Hard assertions
    assert_tools_include(&trace2, &["search_memory"]);
    assert_has_output(&trace2);
    assert_output_contains(&trace2, "December");
}

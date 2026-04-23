//! Golden scenario: Facts persist across conversation turns.
//!
//! Class: Memory | Expected tokens: 2500

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "memory_cross_session_persistence",
        GoldenScenarioMeta {
            class: ScenarioClass::Memory,
            expected_tokens: 2500,
            description: "Turn 1 stores a fact via tool; Turn 2 retrieves it via search_memory",
        },
    );
}

#[tokio::test]
async fn test_memory_cross_session_persistence() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1, Step 1: Agent stores the fact
            tool_call_response(
                "store_fact",
                json!({
                    "category": "person",
                    "name": "Sarah",
                    "relationship": "colleague",
                    "notes": "Joined the team last month, works on backend"
                }),
            ),
            // Turn 1, Step 2: Agent confirms storage
            text_response("Got it, I've noted that Sarah is a colleague who joined last month and works on backend."),
        ])
        .build()
        .await
        .unwrap();

    // Turn 1: Store the fact
    let trace1 = harness
        .run("Sarah is a new colleague who joined last month. She works on backend.")
        .await
        .unwrap();

    assert_tools_include(&trace1, &["store_fact"]);
    assert_has_output(&trace1);

    // Turn 2: Query the stored fact with fresh mock responses
    let trace2 = harness
        .run_turn(
            "What do you know about Sarah?",
            vec![
                // Turn 2, Step 1: Agent searches for Sarah
                tool_call_response("search_memory", json!({"query": "Sarah"})),
                // Turn 2, Step 2: Agent responds with recalled info
                text_response(
                    "Sarah is a colleague who joined the team last month. She works on backend.",
                ),
            ],
        )
        .await
        .unwrap();

    assert_tools_include(&trace2, &["search_memory"]);
    assert_has_output(&trace2);
    assert_output_contains(&trace2, "Sarah");
}

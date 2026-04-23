//! Golden scenario: Follow-up question uses conversation context from prior turns.
//!
//! Class: ConversationQuality | Expected tokens: 2500

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "conversation_quality_followup_context",
        GoldenScenarioMeta {
            class: ScenarioClass::ConversationQuality,
            expected_tokens: 2500,
            description: "Turn 1: user states a fact; Turn 2: follow-up question requires prior context to answer",
        },
    );
}

#[tokio::test]
async fn test_conversation_quality_followup_context() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Agent acknowledges the fact
            text_response("That's a cute name! I'll remember that."),
        ])
        .build()
        .await
        .unwrap();

    // Turn 1: User states a fact
    let _trace1 = harness.run("My cat's name is Whiskers").await.unwrap();

    // Turn 2: Follow-up question that requires turn 1 context
    let trace2 = harness
        .run_turn(
            "What's my cat's name?",
            vec![
                // Agent recalls from conversation history
                text_response("Your cat's name is Whiskers!"),
            ],
        )
        .await
        .unwrap();

    // Hard assertions
    assert_has_output(&trace2);
    assert_output_contains(&trace2, "Whiskers");

    // Verify conversation history carries across turns:
    // The captured_requests accumulate across turns; the last request (turn 2)
    // should include prior conversation messages (user + assistant from turn 1).
    let all_requests = harness.mock().captured_requests();
    let turn2_request = all_requests
        .last()
        .expect("should have captured turn 2 request");
    assert!(
        turn2_request.messages.len() >= 3,
        "Turn 2 request should carry conversation history (user1 + assistant1 + user2), got {} messages",
        turn2_request.messages.len()
    );
}

//! Golden scenario: Agent responds concisely to a simple question.
//!
//! Class: ConversationQuality | Expected tokens: 1500

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "conversation_quality_concise_response",
        GoldenScenarioMeta {
            class: ScenarioClass::ConversationQuality,
            expected_tokens: 1500,
            description: "Simple question gets a concise response without unnecessary verbosity",
        },
    );
}

#[tokio::test]
async fn test_conversation_quality_concise_response() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Agent responds concisely — no clock tool available
            text_response(
                "I don't have access to the current time. You can check your device's clock.",
            ),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("What time is it?").await.unwrap();

    // Hard assertions
    assert_has_output(&trace);

    // Conciseness check: output should be short for a simple question
    let output = trace.output.text.as_deref().unwrap_or("");
    assert!(
        output.len() < 200,
        "Response should be concise (< 200 chars) for a simple question, got {} chars: {:?}",
        output.len(),
        output
    );
}

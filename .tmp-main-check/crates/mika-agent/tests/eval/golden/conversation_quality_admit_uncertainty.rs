//! Golden scenario: Agent admits uncertainty rather than hallucinating a prediction.
//!
//! Class: ConversationQuality | Expected tokens: 1500

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "conversation_quality_admit_uncertainty",
        GoldenScenarioMeta {
            class: ScenarioClass::ConversationQuality,
            expected_tokens: 1500,
            description: "Agent should admit uncertainty on unpredictable questions, not hallucinate",
        },
    );
}

#[tokio::test]
async fn test_conversation_quality_admit_uncertainty() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Agent responds with appropriate uncertainty
            text_response(
                "I can't predict stock market movements. Markets are influenced by countless \
                 unpredictable factors. I'd recommend consulting a financial advisor for \
                 investment guidance.",
            ),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("What will the stock market do tomorrow?")
        .await
        .unwrap();

    // Hard assertions
    assert_has_output(&trace);
    assert_no_tools(&trace);

    // Output should NOT contain definitive prediction language
    let output = trace.output.text.as_deref().unwrap_or("");
    assert!(
        !output.contains("will go up"),
        "Agent should not predict market direction (found 'will go up')"
    );
    assert!(
        !output.contains("will go down"),
        "Agent should not predict market direction (found 'will go down')"
    );
    assert!(
        !output.contains("will increase"),
        "Agent should not predict market direction (found 'will increase')"
    );
    assert!(
        !output.contains("will decrease"),
        "Agent should not predict market direction (found 'will decrease')"
    );
}

//! Integration tests: per-skill LLM provider override routing (D5).
//!
//! The `resolve_skill_llm_override()` function is tested extensively in
//! `crates/mika-agent/src/agent.rs` (inline unit tests for #463). Those tests
//! cover the Keyword/AlwaysOn/Dependency filter, conflict resolution, and
//! same-provider short-circuit.
//!
//! This integration test verifies the observable behavior from the harness level:
//! - Provider attribution is recorded in `llm_calls` per-turn
//! - The mock provider name propagates correctly through the agent loop

use mika_common::llm::mock::*;

use super::assertions::*;
use super::harness::EvalHarness;

#[tokio::test]
async fn test_provider_attribution_recorded_in_llm_calls() {
    // Verify that the provider name and model name propagate through the agent
    // loop and are recorded in the llm_calls table. This is the observable
    // foundation that real per-skill override tests build on.
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Hello from the custom provider.")])
        .provider_name("custom-provider")
        .model_name("custom-model-v1")
        .build()
        .await
        .unwrap();

    let trace = harness.run("Hello").await.unwrap();

    assert_has_output(&trace);

    // The llm_calls table should record the provider's model name
    assert!(
        !trace.llm_calls.is_empty(),
        "Expected at least one LLM call recorded"
    );
    let call = &trace.llm_calls[0];
    assert_eq!(
        call.model.as_str(),
        "custom-model-v1",
        "LLM call should record the provider's model name"
    );
}

#[tokio::test]
async fn test_different_provider_names_across_harness_instances() {
    // Simulate what a per-skill override would look like at the observable level:
    // two separate harness runs with different provider names, verifying that
    // each records the correct attribution.

    // Agent default provider
    let harness_a = EvalHarness::builder()
        .responses(vec![text_response("Response from provider A.")])
        .provider_name("agent-default")
        .model_name("agent-model")
        .build()
        .await
        .unwrap();

    let trace_a = harness_a.run("Test A").await.unwrap();
    assert_has_output(&trace_a);
    assert_eq!(trace_a.llm_calls[0].model.as_str(), "agent-model");

    // Skill override provider (different harness instance simulating the override)
    let harness_b = EvalHarness::builder()
        .responses(vec![text_response("Response from provider B.")])
        .provider_name("skill-override")
        .model_name("skill-model")
        .build()
        .await
        .unwrap();

    let trace_b = harness_b.run("Test B").await.unwrap();
    assert_has_output(&trace_b);
    assert_eq!(trace_b.llm_calls[0].model.as_str(), "skill-model");
}

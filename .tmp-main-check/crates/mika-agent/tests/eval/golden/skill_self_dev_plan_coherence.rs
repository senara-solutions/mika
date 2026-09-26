//! Golden scenario: Self-dev plan coherence — agent gathers context then writes a plan.
//!
//! Class: SkillSpecific | Expected tokens: 4000

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "skill_self_dev_plan_coherence",
        GoldenScenarioMeta {
            class: ScenarioClass::SkillSpecific,
            expected_tokens: 4000,
            description: "Agent searches memory for context then writes a plan document via write_agent_file",
        },
    );
}

#[tokio::test]
async fn test_skill_self_dev_plan_coherence() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent searches memory for existing context about the feature
            tool_call_response(
                "search_memory",
                json!({"query": "health check endpoint"}),
            ),
            // Step 2: Agent writes the plan document
            tool_call_response(
                "write_agent_file",
                json!({
                    "path": "plans/health-check-endpoint.md",
                    "content": "# Health Check Endpoint Plan\n\n## Overview\nImplement a /health endpoint...\n\n## Steps\n1. Add route\n2. Add handler\n3. Add tests"
                }),
            ),
            // Step 3: Agent responds with plan summary
            text_response(
                "I've created a plan for the health check endpoint at plans/health-check-endpoint.md. \
                 The plan covers adding a /health route, implementing the handler, and writing tests.",
            ),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("Create a plan for implementing a health check endpoint")
        .await
        .unwrap();

    // Hard assertions: agent gathers context then writes the plan
    assert_tools_include(&trace, &["search_memory", "write_agent_file"]);
    assert_tool_order(&trace, &["search_memory", "write_agent_file"]);
    assert_has_output(&trace);

    let output = trace.output.text.as_deref().unwrap_or("");
    assert!(
        output.contains("plan") || output.contains("health"),
        "Output should mention 'plan' or 'health' (got: {output})"
    );
}

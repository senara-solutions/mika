//! Integration test: compact provider gate for ProviderKind::MikaModel (mika#1491).
//!
//! Verifies that when the agent runs with `provider_name == "mikamodel"`,
//! the LLM request has a compact system prompt (<=5KB) and a filtered
//! tool array (<=10 core tools). This is the CI-gated regression test
//! for AC1, AC2, and AC4.

use mika_common::llm::mock::*;

use super::harness::EvalHarness;

#[tokio::test]
async fn test_compact_provider_request_shape() {
    let harness = EvalHarness::builder()
        .provider_name("mikamodel")
        .responses(vec![text_response("Hello from MikaModel!")])
        .build()
        .await
        .unwrap();

    let trace = harness.run("Hi there").await.unwrap();

    // Verify the LLM request was captured
    assert!(
        !trace.captured_requests.is_empty(),
        "should have at least one captured request"
    );

    let request = &trace.captured_requests[0];

    // AC1: system prompt <= 5KB
    let system = request.system.as_deref().unwrap_or("");
    assert!(
        system.len() <= 5120,
        "compact provider system prompt should be <=5KB, got {} bytes",
        system.len()
    );

    // No skill prompts should be in the system prompt
    assert!(
        !system.contains("<context type=\"skill\""),
        "compact provider should have zero skill prompts in system"
    );

    // AC2: tools array <= 10 entries
    let tools = request.tools.as_deref().unwrap_or(&[]);
    assert!(
        tools.len() <= 10,
        "compact provider should have <=10 tools, got {}",
        tools.len()
    );

    // Verify only core tools are present
    let core_tools = [
        "send_message",
        "update_core_memory",
        "store_fact",
        "search_memory",
        "update_fact",
        "create_reminder",
        "list_reminders",
        "read_agent_file",
        "write_agent_file",
        "list_agent_files",
    ];
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    for tool in &tool_names {
        assert!(
            core_tools.contains(tool),
            "unexpected tool '{tool}' in compact provider request"
        );
    }
}

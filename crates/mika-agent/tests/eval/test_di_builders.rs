//! Integration tests: optional dependency injection builder methods.
//!
//! Wiring smoke tests for the four DI builder methods on `EvalHarnessBuilder`.
//! Proves the builder chain compiles and threads values into `AgentParams`.
//! Behavioral validation of the injected dependencies lands with downstream
//! callers (#338, #740, #741).

use mika_common::llm::mock::*;

use super::assertions::*;
use super::harness::EvalHarness;

/// Setting `brave_api_key` and `github_token` on the builder compiles and
/// threads through to `run_agent()` without error. These are the two
/// string-typed DI fields; `embedding_client` and `mcp_manager` require
/// real network clients and are validated in downstream tickets.
#[tokio::test]
async fn test_di_string_fields_thread_through_agent_params() {
    let harness = EvalHarness::builder()
        .brave_api_key("test-brave-key")
        .github_token("test-github-token")
        .responses(vec![text_response("OK")])
        .build()
        .await
        .unwrap();

    let trace = harness.run("Hello").await.unwrap();

    // The agent runs successfully with DI fields set — proves they thread
    // through AgentParams without type errors or runtime panics.
    assert_has_output(&trace);
    assert_exact_steps(&trace, 1);
}

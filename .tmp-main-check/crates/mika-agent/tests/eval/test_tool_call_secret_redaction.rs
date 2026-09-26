//! End-to-end test for tool_calls secret redaction (#908).
//!
//! This test fails if redaction happens in the wrong path — i.e. if we redact
//! BEFORE the LLM sees the value (breaks the tool-use loop, agent can't pass
//! the secret onward) or if we forget to redact BEFORE persistence (the bug
//! #908 prevents).
//!
//! DO NOT simplify this into a unit test on `scrub_secrets()` alone — that
//! covers neither path. The DB-layer unit tests in `db.rs` cover persistence
//! redaction in isolation but cannot prove the live `ToolOutput` returned to
//! the LLM stays unscrubbed. This test is the only place where both paths
//! are exercised together against the real persistence boundary.

use anyhow::Result;
use mika_common::llm::mock::*;
use mika_common::llm::{LlmContent, LlmContentBlock, LlmToolResultContent};
use serde_json::json;
use std::fs;

use super::harness::EvalHarness;

/// A realistic GitHub PAT that should be detected by the secret scrubber.
const TEST_PAT: &str = "github_pat_11ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZab";

#[tokio::test]
async fn test_tool_output_unscrubbed_for_llm_but_scrubbed_in_db() -> Result<()> {
    // Stage 1: Build harness and write a fixture .env file with a real-shape PAT.
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("read_agent_file", json!({"path": ".env"})),
            text_response("I found the configuration file."),
        ])
        .build()
        .await?;

    // Write the fixture .env file into the agent's home directory.
    let env_path = harness.home_dir.path().join(".env");
    fs::write(
        &env_path,
        format!("MIKA_GITHUB_TOKEN={TEST_PAT}\nMIKA_LOG_FORMAT=pretty\n"),
    )?;

    // Stage 2: Run the agent turn.
    let trace = harness.run("Read my .env file").await?;

    // Stage 3: Assert R3 — live ToolOutput sent to LLM is NOT scrubbed.
    // The second captured request (after the tool call) should contain a
    // tool_result block with the real PAT value intact.
    let second_request = trace
        .captured_requests
        .get(1)
        .expect("expected a follow-up LLM request after tool execution");

    let tool_result_text = second_request
        .messages
        .iter()
        .flat_map(|msg| match &msg.content {
            LlmContent::Blocks(blocks) => blocks.iter().collect::<Vec<_>>(),
            LlmContent::Text(_) => vec![],
        })
        .filter_map(|block| match block {
            LlmContentBlock::ToolResult { content, .. } => Some(content),
            _ => None,
        })
        .filter_map(|content| match content {
            LlmToolResultContent::Text(text) => Some(text.clone()),
            LlmToolResultContent::Blocks(blocks) => {
                let texts: Vec<String> = blocks
                    .iter()
                    .filter_map(|b| match b {
                        mika_common::llm::LlmToolResultBlock::Text(t) => Some(t.clone()),
                        _ => None,
                    })
                    .collect();
                if texts.is_empty() {
                    None
                } else {
                    Some(texts.join("\n"))
                }
            }
        })
        .collect::<Vec<String>>()
        .join("\n");

    assert!(
        tool_result_text.contains(TEST_PAT),
        "R3 FAILED: The live ToolOutput sent to the LLM should contain the \
         unscrubbed secret for downstream tool composition. \
         Tool result text: {tool_result_text}"
    );

    // Stage 4: Assert R5 — DB persisted tool_calls.output IS scrubbed.
    let read_calls = trace.calls_for_tool("read_agent_file");
    assert_eq!(
        read_calls.len(),
        1,
        "Expected exactly 1 read_agent_file call"
    );

    let db_output = read_calls[0]
        .output
        .as_deref()
        .expect("tool_calls.output should be present");

    // The original PAT must NOT appear in the DB row.
    assert!(
        !db_output.contains(TEST_PAT),
        "R5 FAILED: The persisted tool_calls.output still contains the raw secret. \
         Scrubbing did not happen before INSERT. Output: {db_output}"
    );

    // The redacted form should be present instead.
    assert!(
        db_output.contains("<REDACTED>"),
        "R5 FAILED: Expected '<REDACTED>' placeholder in persisted output. \
         Output: {db_output}"
    );

    Ok(())
}

//! Integration tests: required_tools gate terminal failure bypass (#516).
//!
//! Verifies that the required_tools gate:
//! 1. Allows EndTurn when a required tool failed with a terminal error
//! 2. Retries when required tools are missing and no terminal failure occurred
//! 3. Retries when a required tool fails with a retryable error

use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use mika_agent::skills::SkillRegistry;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{Constraints, SkillInfo, SkillManifest, Triggers};
use mika_agent::tools::{Tool, ToolContext, ToolOutput, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;

// -- Stub tools --

/// A stub tool that always returns a terminal error (GitHub self-approval).
struct TerminalErrorTool;

#[async_trait]
impl Tool for TerminalErrorTool {
    fn name(&self) -> &str {
        "tool_a"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "tool_a".to_string(),
            description: "Stub tool that returns terminal error".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        // Simulates `spawn_and_collect` output for a non-zero exit with terminal error.
        // Note: builtin handlers return ToolOutput::success even for non-zero exits,
        // with the error text in content. The ToolCallSummary builder detects the
        // "Exit code:" prefix and sets non_zero_exit=true, success=false.
        Ok(ToolOutput::success(
            "Exit code: 1\nGraphQL: Can not approve your own pull request".to_string(),
        ))
    }
}

/// A stub tool that always returns a retryable error (rate limit).
struct RetryableErrorTool;

#[async_trait]
impl Tool for RetryableErrorTool {
    fn name(&self) -> &str {
        "tool_a"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "tool_a".to_string(),
            description: "Stub tool that returns retryable error".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            "Exit code: 1\nHTTP 429: rate limit exceeded".to_string(),
        ))
    }
}

/// A stub tool that succeeds.
struct SuccessTool {
    name: String,
}

#[async_trait]
impl Tool for SuccessTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.clone(),
            description: format!("Stub success tool: {}", self.name),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success("ok".to_string()))
    }
}

// -- Helpers --

/// Create a skill entry with required_tools and keyword triggers.
fn make_skill_with_required_tools(
    name: &str,
    keywords: &[&str],
    required_tools: &[&str],
) -> SkillEntry {
    SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: name.to_string(),
                description: format!("{name} test skill"),
                version: "0.1.0".to_string(),
                always_on: false,
                timeout_secs: 30,
                dependencies: vec![],
                max_prompt_size: None,
            },
            triggers: Triggers {
                keywords: keywords.iter().map(|s| s.to_string()).collect(),
            },
            llm: Default::default(),
            constraints: Constraints {
                required_tools: required_tools.iter().map(|s| s.to_string()).collect(),
                required_fetches_for_quoted_resources: false,
            },
            output: Default::default(),
            context: HashMap::new(),
            variants: Default::default(),
        },
        dir: PathBuf::from(format!("/skills/{name}")),
        keywords_lower: keywords.iter().map(|s| s.to_lowercase()).collect(),
        prompt_snippet: String::new(),
        skill_tools: vec![],
        enabled: true,
        has_override: false,
        provider_overrides: HashMap::new(),
        prompt_sources: SkillEntry::empty_prompt_sources(),
        model_overrides: HashMap::new(),
    }
}

/// Build a SkillRegistry with the given entries.
fn skill_registry(entries: Vec<SkillEntry>) -> SkillRegistry {
    // SkillRegistry fields are pub(crate), but we can construct via the test-visible pattern.
    // Using the same approach as the unit tests in skills/mod.rs.
    SkillRegistry::from_test_entries(entries)
}

/// When a required tool fails with a terminal error, the gate should allow EndTurn
/// without retrying — even if other required tools were not called.
///
/// Scenario: skill requires ["tool_a", "tool_b"]. Agent calls tool_a which fails
/// with "Can not approve your own pull request". Agent responds with text explaining
/// the failure without calling tool_b. Gate should allow EndTurn.
#[tokio::test]
async fn terminal_failure_bypasses_required_tools_retry() {
    let skills = skill_registry(vec![make_skill_with_required_tools(
        "test-review",
        &["review"],
        &["tool_a", "tool_b"],
    )]);

    let mut tools = default_tools();
    tools.register(Box::new(TerminalErrorTool));
    tools.register(Box::new(SuccessTool {
        name: "tool_b".to_string(),
    }));

    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls tool_a (which returns terminal error)
            tool_call_response("tool_a", json!({})),
            // Step 2: Agent responds with text explaining the failure (no tool_b call)
            text_response(
                "I cannot approve this PR because you cannot approve your own pull request.",
            ),
        ])
        .tools(tools)
        .skills(skills)
        .build()
        .await
        .unwrap();

    // The user message must contain "review" to trigger keyword matching
    let trace = harness.run("review PR #42").await.unwrap();

    assert_has_output(&trace);
    // Gate should allow EndTurn: 2 LLM calls (tool_a call + text response), no retry
    assert_exact_steps(&trace, 2);
    assert_output_contains(&trace, "cannot approve");
}

/// When required tools are missing and NO terminal failure occurred, the gate should
/// retry once (existing behavior preserved).
///
/// Scenario: skill requires ["tool_a", "tool_b"]. Agent responds with text without
/// calling either tool. Gate retries once. After retry, agent calls both tools and
/// responds with text.
#[tokio::test]
async fn missing_tools_without_failure_triggers_retry() {
    let skills = skill_registry(vec![make_skill_with_required_tools(
        "test-review",
        &["review"],
        &["tool_a", "tool_b"],
    )]);

    let mut tools = default_tools();
    tools.register(Box::new(SuccessTool {
        name: "tool_a".to_string(),
    }));
    tools.register(Box::new(SuccessTool {
        name: "tool_b".to_string(),
    }));

    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent responds with text but doesn't call required tools
            text_response("The PR looks good, I approve it."),
            // Step 2: After re-prompt, agent calls tool_a
            tool_call_response("tool_a", json!({})),
            // Step 3: Agent calls tool_b
            tool_call_response("tool_b", json!({})),
            // Step 4: Agent responds with text (tools now called)
            text_response("After reviewing, the PR is approved."),
        ])
        .tools(tools)
        .skills(skills)
        .build()
        .await
        .unwrap();

    let trace = harness.run("review PR #42").await.unwrap();

    assert_has_output(&trace);
    // Gate retried: 1 (rejected text) + 1 (tool_a) + 1 (tool_b) + 1 (final text) = 4 LLM calls
    assert_exact_steps(&trace, 4);
    assert_output_contains(&trace, "approved");
}

/// When a required tool fails with a retryable error (e.g., 429), the gate should
/// still retry — retryable errors don't bypass enforcement.
#[tokio::test]
async fn retryable_failure_does_not_bypass_retry() {
    let skills = skill_registry(vec![make_skill_with_required_tools(
        "test-review",
        &["review"],
        &["tool_a", "tool_b"],
    )]);

    let mut tools = default_tools();
    tools.register(Box::new(RetryableErrorTool));
    tools.register(Box::new(SuccessTool {
        name: "tool_b".to_string(),
    }));

    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls tool_a (retryable error)
            tool_call_response("tool_a", json!({})),
            // Step 2: Agent responds with text without calling tool_b
            text_response("The API returned a rate limit error, I'll try later."),
            // Step 3: After re-prompt (gate retried), agent calls tool_b
            tool_call_response("tool_b", json!({})),
            // Step 4: Agent responds with text
            text_response("After retrying, the review is complete."),
        ])
        .tools(tools)
        .skills(skills)
        .build()
        .await
        .unwrap();

    let trace = harness.run("review PR #42").await.unwrap();

    assert_has_output(&trace);
    // Gate retried because the error was retryable:
    // 1 (tool_a) + 1 (rejected text) + 1 (tool_b) + 1 (final text) = 4 LLM calls
    assert_exact_steps(&trace, 4);
}

/// When all required tools are called successfully, the gate should pass immediately.
#[tokio::test]
async fn all_required_tools_called_passes_gate() {
    let skills = skill_registry(vec![make_skill_with_required_tools(
        "test-review",
        &["review"],
        &["tool_a", "tool_b"],
    )]);

    let mut tools = default_tools();
    tools.register(Box::new(SuccessTool {
        name: "tool_a".to_string(),
    }));
    tools.register(Box::new(SuccessTool {
        name: "tool_b".to_string(),
    }));

    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls tool_a
            tool_call_response("tool_a", json!({})),
            // Step 2: Agent calls tool_b
            tool_call_response("tool_b", json!({})),
            // Step 3: Agent responds with text
            text_response("Both tools called successfully. PR approved."),
        ])
        .tools(tools)
        .skills(skills)
        .build()
        .await
        .unwrap();

    let trace = harness.run("review PR #42").await.unwrap();

    assert_has_output(&trace);
    // No retry: 1 (tool_a) + 1 (tool_b) + 1 (text) = 3 LLM calls
    assert_exact_steps(&trace, 3);
    assert_output_contains(&trace, "approved");
}

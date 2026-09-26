//! Integration tests: self-dev-callback engine consistency (#806).
//!
//! Verifies that each documented callback-handler branch in
//! `self-dev-callback/system_prompt.md` prescribes tool calls that the engine
//! accepts (or properly defers) — not hard-rejected by guards.
//!
//! Uses conversation mode with `[callback:` prefix (not `.callback_turn(true)`)
//! because guards trigger on the `[callback:` prefix in `user_input_text`.
//! See `test_callback_terminal_action.rs` doc comment for rationale.
//!
//! Test cases:
//! 1. Pipeline failure — retry path (retries < 2): run_claude_pilot deferred, not rejected.
//! 2. Pipeline failure — retry exhausted (retries >= 2): blocked, no dispatch.
//! 3. Success callback — PR discovered, metadata persisted, operator notified.
//! 4. Failure callback — no PR exists, logs read, escalated.
//! 5. Failure callback — PR exists despite failure, routes to success handling.
//! 6. error_max_turns — unpushed recovery path, no re-dispatch.
//! 7. Groom callback — GROOMED verdict, task completed, ready label added.

use async_trait::async_trait;
use mika_agent::tools::{Tool, ToolContext, ToolOutput, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;

// ---------------------------------------------------------------------------
// Stub tools
// ---------------------------------------------------------------------------

/// Stub `update_task_status` that always succeeds.
struct StubUpdateTaskStatusTool;

#[async_trait]
impl Tool for StubUpdateTaskStatusTool {
    fn name(&self) -> &str {
        "update_task_status"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "update_task_status".to_string(),
            description: "Stub update_task_status for callback consistency tests".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success("Task updated."))
    }
}

/// Stub `check_task` that returns task metadata with configurable retry count.
struct StubCheckTaskTool {
    response: String,
}

impl StubCheckTaskTool {
    /// Pipeline failure retry path — metadata with a specific retry count.
    fn with_retry_count(count: u32) -> Self {
        Self {
            response: json!({
                "task_id": "task-001",
                "status": "in_progress",
                "label": "long_running:run_claude_pilot:dev-pilot:mika#100",
                "metadata": {
                    "pipeline_retry_count": count,
                    "branch": "feat/issue-100"
                }
            })
            .to_string(),
        }
    }

    /// Success/failure callback — label identifies the callback type.
    fn with_label(label: &str) -> Self {
        Self {
            response: json!({
                "task_id": "task-001",
                "status": "in_progress",
                "label": label,
                "metadata": {
                    "branch": "feat/issue-100"
                }
            })
            .to_string(),
        }
    }
}

#[async_trait]
impl Tool for StubCheckTaskTool {
    fn name(&self) -> &str {
        "check_task"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "check_task".to_string(),
            description: "Stub check_task for callback consistency tests".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(self.response.clone()))
    }
}

/// Stub `run_gh` that returns a configurable response.
struct StubRunGhTool {
    response: String,
}

impl StubRunGhTool {
    fn with_response(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
        }
    }
}

#[async_trait]
impl Tool for StubRunGhTool {
    fn name(&self) -> &str {
        "run_gh"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_gh".to_string(),
            description: "Stub run_gh for callback consistency tests".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(self.response.clone()))
    }
}

/// Stub `run_claude_pilot` — returns deferred dispatch result.
struct StubRunClaudePilotTool;

#[async_trait]
impl Tool for StubRunClaudePilotTool {
    fn name(&self) -> &str {
        "run_claude_pilot"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_claude_pilot".to_string(),
            description: "Stub run_claude_pilot for callback consistency tests".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            json!({"status": "deferred", "deferred": true}).to_string(),
        ))
    }
}

/// Stub `run_shell` — returns a configurable response (e.g., log output).
struct StubRunShellTool {
    response: String,
}

impl StubRunShellTool {
    fn with_response(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
        }
    }
}

#[async_trait]
impl Tool for StubRunShellTool {
    fn name(&self) -> &str {
        "run_shell"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_shell".to_string(),
            description: "Stub run_shell for callback consistency tests".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(self.response.clone()))
    }
}

// ---------------------------------------------------------------------------
// Helper: build tool registries for each test scenario
// ---------------------------------------------------------------------------

/// Tools for pipeline failure retry path.
fn tools_pipeline_failure_retry() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubUpdateTaskStatusTool));
    tools.register(Box::new(StubCheckTaskTool::with_retry_count(0)));
    tools.register(Box::new(StubRunClaudePilotTool));
    tools.register(Box::new(StubRunGhTool::with_response("")));
    tools.register(Box::new(StubRunShellTool::with_response("")));
    tools
}

/// Tools for pipeline failure exhausted path.
fn tools_pipeline_failure_exhausted() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubUpdateTaskStatusTool));
    tools.register(Box::new(StubCheckTaskTool::with_retry_count(2)));
    tools.register(Box::new(StubRunClaudePilotTool));
    tools.register(Box::new(StubRunGhTool::with_response("")));
    tools
}

/// Tools for success callback path.
fn tools_success_callback() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubUpdateTaskStatusTool));
    tools.register(Box::new(StubCheckTaskTool::with_label(
        "long_running:run_claude_pilot:dev-pilot:mika#100",
    )));
    tools.register(Box::new(StubRunGhTool::with_response(
        "https://github.com/senara-solutions/mika/pull/42",
    )));
    tools
}

/// Tools for failure callback (no PR) path.
fn tools_failure_no_pr() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubUpdateTaskStatusTool));
    tools.register(Box::new(StubCheckTaskTool::with_label(
        "long_running:run_claude_pilot:dev-pilot:mika#100",
    )));
    tools.register(Box::new(StubRunGhTool::with_response("")));
    tools.register(Box::new(StubRunShellTool::with_response(
        "Error: thread 'main' panicked at 'assertion failed'\nnote: run with `RUST_BACKTRACE=1`",
    )));
    tools
}

/// Tools for failure callback (PR exists) path.
fn tools_failure_pr_exists() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubUpdateTaskStatusTool));
    tools.register(Box::new(StubCheckTaskTool::with_label(
        "long_running:run_claude_pilot:dev-pilot:mika#100",
    )));
    tools.register(Box::new(StubRunGhTool::with_response(
        "https://github.com/senara-solutions/mika/pull/42",
    )));
    tools
}

/// Tools for error_max_turns callback path.
fn tools_error_max_turns() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubUpdateTaskStatusTool));
    tools.register(Box::new(StubCheckTaskTool::with_label(
        "long_running:run_claude_pilot:dev-pilot:mika#100",
    )));
    tools.register(Box::new(StubRunGhTool::with_response("")));
    tools.register(Box::new(StubRunShellTool::with_response(
        "abc1234 feat: implement feature X\ndef5678 fix: handle edge case",
    )));
    tools.register(Box::new(StubRunClaudePilotTool));
    tools
}

/// Tools for groom callback path.
fn tools_groom_callback() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubUpdateTaskStatusTool));
    tools.register(Box::new(StubCheckTaskTool::with_label(
        "long_running:run_claude_pilot_groom:dev-groom:mika#200",
    )));
    tools.register(Box::new(StubRunGhTool::with_response(
        "## Plan\n> - **Branch:** feat/issue-200\nsecond-pass (GROOMED)\nsome more body text",
    )));
    tools
}

// ---------------------------------------------------------------------------
// Test 1: Pipeline failure — retry path (retries < 2)
//
// Critical assertion: run_claude_pilot is called and returns deferred dispatch
// (not a hard error from the engine).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn callback_pipeline_failure_retry_dispatches_claude_pilot() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: check_task — read retry count from metadata
            tool_call_response("check_task", json!({"task_id": "task-001"})),
            // Step 2: update_task_status — increment retry count
            tool_call_response(
                "update_task_status",
                json!({"task_id": "task-001", "metadata": {"pipeline_retry_count": 1}}),
            ),
            // Step 3: run_claude_pilot — re-dispatch (CRITICAL: must not be hard-rejected)
            tool_call_response(
                "run_claude_pilot",
                json!({"skill": "dev-pilot", "task_id": "task-001"}),
            ),
            // Step 4: send_message — notify operator
            tool_call_response(
                "send_message",
                json!({"text": "Pipeline failure detected. Retrying (attempt 1/2)."}),
            ),
            // Step 5: final text
            text_response("Callback processed: pipeline failure with retry."),
        ])
        .tools(tools_pipeline_failure_retry())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("[callback: long_running:run_claude_pilot] PIPELINE FAILURE: build failed")
        .await
        .unwrap();

    assert_has_output(&trace);
    // Critical: run_claude_pilot was called and did NOT produce a hard error
    assert_tools_include(
        &trace,
        &[
            "check_task",
            "run_claude_pilot",
            "update_task_status",
            "send_message",
        ],
    );

    // Verify run_claude_pilot returned deferred dispatch, not an error
    let rcp_calls = trace.calls_for_tool("run_claude_pilot");
    assert!(
        !rcp_calls.is_empty(),
        "run_claude_pilot should have been called"
    );
    let rcp_output = rcp_calls[0].output.as_deref().unwrap_or("");
    assert!(
        !rcp_output.contains("cannot run in the current context"),
        "run_claude_pilot should NOT be hard-rejected in callback context, got: {}",
        rcp_output
    );
    assert!(
        rcp_output.contains("deferred") || rcp_output.contains("dispatched"),
        "run_claude_pilot should return deferred or dispatched status, got: {}",
        rcp_output
    );

    // Agent loop completed within step budget
    assert_max_steps(&trace, 10);
}

// ---------------------------------------------------------------------------
// Test 2: Pipeline failure — retry exhausted (retries >= 2)
//
// Agent marks task blocked and escalates. run_claude_pilot is NOT called.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn callback_pipeline_failure_exhausted_blocks_without_dispatch() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: check_task — retry count is 2 (exhausted)
            tool_call_response("check_task", json!({"task_id": "task-001"})),
            // Step 2: update_task_status — mark blocked
            tool_call_response(
                "update_task_status",
                json!({"task_id": "task-001", "status": "blocked"}),
            ),
            // Step 3: send_message — escalate to operator
            tool_call_response(
                "send_message",
                json!({"text": "Pipeline failure after 2 retries. Task blocked, escalating."}),
            ),
            // Step 4: final text
            text_response("Callback processed: pipeline failure exhausted, escalated."),
        ])
        .tools(tools_pipeline_failure_exhausted())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("[callback: long_running:run_claude_pilot] PIPELINE FAILURE: build failed again")
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_tools_include(
        &trace,
        &["check_task", "update_task_status", "send_message"],
    );
    // run_claude_pilot should NOT be called when retries are exhausted
    assert_tools_exclude(&trace, &["run_claude_pilot"]);
    assert_max_steps(&trace, 8);
}

// ---------------------------------------------------------------------------
// Test 3: Success callback — PR discovered, metadata persisted, notified
// ---------------------------------------------------------------------------

#[tokio::test]
async fn callback_success_discovers_pr_and_notifies() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: check_task
            tool_call_response("check_task", json!({"task_id": "task-001"})),
            // Step 2: run_gh — discover PR URL
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "list", "--head", "feat/issue-100"]}),
            ),
            // Step 3: update_task_status with PR metadata
            tool_call_response(
                "update_task_status",
                json!({
                    "task_id": "task-001",
                    "metadata": {"pr_url": "https://github.com/senara-solutions/mika/pull/42"}
                }),
            ),
            // Step 4: send_message — notify operator
            tool_call_response(
                "send_message",
                json!({"text": "PR #42 opened for mika#100."}),
            ),
            // Step 5: final text
            text_response("Callback processed: success with PR."),
        ])
        .tools(tools_success_callback())
        .build()
        .await
        .unwrap();

    // Success callback — result text contains structured metadata, no PIPELINE FAILURE
    let trace = harness
        .run(
            "[callback: long_running:run_claude_pilot] \
             Session: abc-123\nCost: $0.42\nTurns: 15\nDuration: 300s\n\
             PR: https://github.com/senara-solutions/mika/pull/42",
        )
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_tools_include(
        &trace,
        &["check_task", "run_gh", "update_task_status", "send_message"],
    );
    // No retry dispatch on success
    assert_tools_exclude(&trace, &["run_claude_pilot"]);
    assert_max_steps(&trace, 8);
}

// ---------------------------------------------------------------------------
// Test 4: Failure callback — no PR exists, logs read, escalated
// ---------------------------------------------------------------------------

#[tokio::test]
async fn callback_failure_no_pr_reads_logs_and_escalates() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: check_task
            tool_call_response("check_task", json!({"task_id": "task-001"})),
            // Step 2: run_gh — no PR found
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "list", "--head", "feat/issue-100"]}),
            ),
            // Step 3: run_shell — read tail of log file
            tool_call_response(
                "run_shell",
                json!({"command": "tail -50 /path/to/session.log"}),
            ),
            // Step 4: update_task_status — mark blocked
            tool_call_response(
                "update_task_status",
                json!({"task_id": "task-001", "status": "blocked"}),
            ),
            // Step 5: send_message — escalate
            tool_call_response(
                "send_message",
                json!({"text": "Pipeline failed with no PR. Task blocked. Logs attached."}),
            ),
            // Step 6: final text
            text_response("Callback processed: failure with no PR, escalated."),
        ])
        .tools(tools_failure_no_pr())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run(
            "[callback: long_running:run_claude_pilot] \
             FAILED: claude-pilot exited with code 1",
        )
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_tools_include(
        &trace,
        &[
            "check_task",
            "run_gh",
            "run_shell",
            "update_task_status",
            "send_message",
        ],
    );
    // run_shell is accepted in callback context (not long_running)
    let shell_calls = trace.calls_for_tool("run_shell");
    assert!(
        !shell_calls.is_empty(),
        "run_shell should have been called in callback context"
    );
    let shell_output = shell_calls[0].output.as_deref().unwrap_or("");
    assert!(
        !shell_output.contains("cannot run in the current context"),
        "run_shell should NOT be blocked in callback context"
    );

    assert_max_steps(&trace, 10);
}

// ---------------------------------------------------------------------------
// Test 5: Failure callback — PR exists despite failure, routes to success
// ---------------------------------------------------------------------------

#[tokio::test]
async fn callback_failure_with_pr_routes_to_success() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: check_task
            tool_call_response("check_task", json!({"task_id": "task-001"})),
            // Step 2: run_gh — PR found despite failure
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "list", "--head", "feat/issue-100"]}),
            ),
            // Step 3: update_task_status with success metadata
            tool_call_response(
                "update_task_status",
                json!({
                    "task_id": "task-001",
                    "metadata": {"pr_url": "https://github.com/senara-solutions/mika/pull/42"}
                }),
            ),
            // Step 4: send_message
            tool_call_response(
                "send_message",
                json!({"text": "Despite exit code, PR #42 was created."}),
            ),
            // Step 5: final text
            text_response("Callback processed: failure but PR exists."),
        ])
        .tools(tools_failure_pr_exists())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run(
            "[callback: long_running:run_claude_pilot] \
             FAILED: claude-pilot exited with code 1",
        )
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_tools_include(
        &trace,
        &["check_task", "run_gh", "update_task_status", "send_message"],
    );
    assert_max_steps(&trace, 8);
}

// ---------------------------------------------------------------------------
// Test 6: error_max_turns callback — unpushed recovery, NO re-dispatch
//
// The documented handler routes to recover_unpushed_work or failure,
// never to run_claude_pilot.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn callback_error_max_turns_no_redispatch() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: check_task — read metadata with branch
            tool_call_response("check_task", json!({"task_id": "task-001"})),
            // Step 2: run_gh — no PR exists
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "list", "--head", "feat/issue-100"]}),
            ),
            // Step 3: run_shell — git log shows commits exist
            tool_call_response(
                "run_shell",
                json!({"command": "git log --oneline feat/issue-100 ^main"}),
            ),
            // Step 4: update_task_status — mark recovery pending
            tool_call_response(
                "update_task_status",
                json!({
                    "task_id": "task-001",
                    "metadata": {"unpushed_recovery_pending": true}
                }),
            ),
            // Step 5: send_message — recovery notification
            tool_call_response(
                "send_message",
                json!({"text": "error_max_turns: found unpushed commits. Recovery pending."}),
            ),
            // Step 6: final text
            text_response("Callback processed: error_max_turns with recovery."),
        ])
        .tools(tools_error_max_turns())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run(
            "[callback: long_running:run_claude_pilot] \
             error_max_turns: agent exceeded tool step limit",
        )
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_tools_include(
        &trace,
        &[
            "check_task",
            "run_gh",
            "run_shell",
            "update_task_status",
            "send_message",
        ],
    );
    // Critical: run_claude_pilot is NOT called on error_max_turns
    assert_tools_exclude(&trace, &["run_claude_pilot"]);
    assert_max_steps(&trace, 10);
}

// ---------------------------------------------------------------------------
// Test 7: Groom callback — GROOMED verdict path
//
// After groom completes, task is marked completed, ready label added,
// and operator notified. No re-dispatch.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn callback_groom_groomed_completes_and_labels() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: check_task — detect groom callback via label
            tool_call_response("check_task", json!({"task_id": "task-001"})),
            // Step 2: run_gh — issue view shows GROOMED marker in body
            tool_call_response(
                "run_gh",
                json!({"command": ["issue", "view", "200", "--repo", "senara-solutions/mika"]}),
            ),
            // Step 3: update_task_status — mark completed
            tool_call_response(
                "update_task_status",
                json!({"task_id": "task-001", "status": "completed"}),
            ),
            // Step 4: run_gh — add ready label
            tool_call_response(
                "run_gh",
                json!({"command": ["issue", "edit", "200", "--add-label", "ready"]}),
            ),
            // Step 5: send_message — notify
            tool_call_response(
                "send_message",
                json!({"text": "mika#200 groomed and marked ready."}),
            ),
            // Step 6: final text
            text_response("Callback processed: groom GROOMED."),
        ])
        .tools(tools_groom_callback())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run(
            "[callback: long_running:run_claude_pilot_groom] \
             Verdict: GROOMED\nSession: groom-abc\nCost: $0.10",
        )
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_tools_include(
        &trace,
        &["check_task", "run_gh", "update_task_status", "send_message"],
    );
    // No re-dispatch on groom completion
    assert_tools_exclude(&trace, &["run_claude_pilot"]);
    assert_max_steps(&trace, 10);
}

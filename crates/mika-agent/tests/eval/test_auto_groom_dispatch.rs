//! Integration tests: auto-groom dispatch before dev-pilot (#996).
//!
//! Verifies that the Ready-Label Dispatch handler and the Milestone M4 path
//! auto-groom ungroomed tickets via `dev-groom` before dispatching `dev-pilot`,
//! and bypass grooming when the Plan callout is already present.

use async_trait::async_trait;
use mika_agent::tools::{Tool, ToolContext, ToolOutput, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;

/// Ready-label webhook message matching the gateway's format.
const READY_LABEL_MSG: &str = "[GitHub] Issue labeled ready on senara-solutions/mika#500 — feat: ungroomed ticket\n\
     https://github.com/senara-solutions/mika/issues/500";

// -- Stub tools --

/// Stub `send_message` — simulates operator notification.
struct StubSendMessage;

#[async_trait]
impl Tool for StubSendMessage {
    fn name(&self) -> &str {
        "send_message"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "send_message".to_string(),
            description: "Stub send_message for auto-groom tests".to_string(),
            input_schema: json!({"type": "object", "properties": {"message": {"type": "string"}}}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success("Message sent".to_string()))
    }
}

/// Stub `run_claude_pilot` — captures the skill parameter to verify dispatch target.
struct StubRunClaudePilot;

#[async_trait]
impl Tool for StubRunClaudePilot {
    fn name(&self) -> &str {
        "run_claude_pilot"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_claude_pilot".to_string(),
            description: "Stub run_claude_pilot for auto-groom tests".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "skill": {"type": "string", "enum": ["dev-pilot", "dev-groom"]},
                    "prompt": {"type": "string"},
                    "task_id": {"type": "string"}
                }
            }),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success("Dispatched".to_string()))
    }
}

/// Stub `run_gh` — simulates GitHub CLI operations.
struct StubRunGh;

#[async_trait]
impl Tool for StubRunGh {
    fn name(&self) -> &str {
        "run_gh"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_gh".to_string(),
            description: "Stub run_gh for auto-groom tests".to_string(),
            input_schema: json!({"type": "object", "properties": {"args": {"type": "string"}}}),
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

/// Stub `create_task` — simulates task creation.
struct StubCreateTask;

#[async_trait]
impl Tool for StubCreateTask {
    fn name(&self) -> &str {
        "create_task"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "create_task".to_string(),
            description: "Stub create_task for auto-groom tests".to_string(),
            input_schema: json!({"type": "object", "properties": {"label": {"type": "string"}}}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            r#"{"task_id": "00000000-0000-0000-0000-000000000042"}"#.to_string(),
        ))
    }
}

/// Build a tool registry with all auto-groom test stub tools.
fn tools_with_auto_groom_stubs() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubSendMessage));
    tools.register(Box::new(StubRunClaudePilot));
    tools.register(Box::new(StubRunGh));
    tools.register(Box::new(StubCreateTask));
    tools
}

/// #996 — Webhook path: ungroomed ticket dispatches dev-groom (not send_message
/// rejection). The agent detects missing Plan callout and calls run_claude_pilot
/// with skill="dev-groom" to auto-groom before dispatch.
#[tokio::test]
async fn webhook_ungroomed_dispatches_dev_groom() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: run_gh to remove ready label
            tool_call_response(
                "run_gh",
                json!({"args": "issue edit 500 --remove-label ready"}),
            ),
            // Step 2: run_gh to fetch issue body (no Plan callout)
            tool_call_response(
                "run_gh",
                json!({"args": "issue view 500 --json title,body"}),
            ),
            // Step 3: create_task for grooming (with ?phase=groom discriminator)
            tool_call_response(
                "create_task",
                json!({
                    "label": "groom senara-solutions/mika#500",
                    "reference_url": "https://github.com/senara-solutions/mika/issues/500?phase=groom"
                }),
            ),
            // Step 4: run_claude_pilot with skill="dev-groom"
            tool_call_response(
                "run_claude_pilot",
                json!({
                    "skill": "dev-groom",
                    "prompt": "mika issue#500",
                    "task_id": "00000000-0000-0000-0000-000000000042"
                }),
            ),
            // Step 5: final text
            text_response("Auto-grooming mika#500 via dev-groom before dispatch."),
        ])
        .tools(tools_with_auto_groom_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(READY_LABEL_MSG).await.unwrap();

    assert_has_output(&trace);
    // 5 steps: run_gh (label) + run_gh (fetch) + create_task + run_claude_pilot + text
    assert_exact_steps(&trace, 5);
    // The agent calls run_claude_pilot with skill="dev-groom", not send_message
    assert_tools_include(&trace, &["run_claude_pilot"]);
    assert_tools_exclude(&trace, &["send_message"]);
    // Verify the skill parameter is "dev-groom"
    assert_tool_args_contain(&trace, "run_claude_pilot", 0, json!({"skill": "dev-groom"}));
}

/// #996 — Webhook path: already-groomed ticket bypasses dev-groom and dispatches
/// dev-pilot directly. The Plan callout is present, so no grooming needed.
#[tokio::test]
async fn webhook_groomed_bypasses_dev_groom() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: run_gh to remove ready label
            tool_call_response(
                "run_gh",
                json!({"args": "issue edit 500 --remove-label ready"}),
            ),
            // Step 2: run_gh to fetch issue body (HAS Plan callout)
            tool_call_response(
                "run_gh",
                json!({"args": "issue view 500 --json title,body"}),
            ),
            // Step 3: create_task for dispatch
            tool_call_response("create_task", json!({"label": "feat: ungroomed ticket"})),
            // Step 4: run_claude_pilot with skill="dev-pilot"
            tool_call_response(
                "run_claude_pilot",
                json!({
                    "skill": "dev-pilot",
                    "prompt": "senara-solutions/mika#500",
                    "task_id": "00000000-0000-0000-0000-000000000042"
                }),
            ),
            // Step 5: final text
            text_response("Dispatched claude-pilot for mika#500."),
        ])
        .tools(tools_with_auto_groom_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(READY_LABEL_MSG).await.unwrap();

    assert_has_output(&trace);
    // 5 steps — groomed issue goes straight to dev-pilot
    assert_exact_steps(&trace, 5);
    assert_tools_include(&trace, &["run_claude_pilot"]);
    // Verify the skill parameter is "dev-pilot" (not "dev-groom")
    assert_tool_args_contain(&trace, "run_claude_pilot", 0, json!({"skill": "dev-pilot"}));
}

/// #996 — Webhook path: ESCALATE verdict from dev-groom callback should NOT
/// auto-dispatch. The agent should send_message to operator with the escalation
/// reason and stop.
#[tokio::test]
async fn webhook_escalate_verdict_blocks_dispatch() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: run_gh to remove ready label
            tool_call_response(
                "run_gh",
                json!({"args": "issue edit 500 --remove-label ready"}),
            ),
            // Step 2: run_gh to fetch issue body (no Plan callout)
            tool_call_response(
                "run_gh",
                json!({"args": "issue view 500 --json title,body"}),
            ),
            // Step 3: create_task for grooming
            tool_call_response(
                "create_task",
                json!({"label": "groom senara-solutions/mika#500"}),
            ),
            // Step 4: run_claude_pilot with skill="dev-groom"
            tool_call_response(
                "run_claude_pilot",
                json!({"skill": "dev-groom", "prompt": "mika issue#500"}),
            ),
            // Step 5: final text — agent dispatched grooming
            text_response("Auto-grooming initiated for mika#500."),
        ])
        .tools(tools_with_auto_groom_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(READY_LABEL_MSG).await.unwrap();

    assert_has_output(&trace);
    // The initial dispatch fires dev-groom (verified by tool args)
    assert_tool_args_contain(&trace, "run_claude_pilot", 0, json!({"skill": "dev-groom"}));
    // dev-pilot should NOT have been called in this turn (grooming is async)
    let pilot_calls: Vec<_> = trace
        .tool_calls
        .iter()
        .filter(|tc| tc.tool_name == "run_claude_pilot")
        .collect();
    assert_eq!(
        pilot_calls.len(),
        1,
        "Only one run_claude_pilot call expected (dev-groom), not a second dev-pilot"
    );
}

/// Stub `update_task_status` — simulates task status updates.
struct StubUpdateTaskStatus;

#[async_trait]
impl Tool for StubUpdateTaskStatus {
    fn name(&self) -> &str {
        "update_task_status"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "update_task_status".to_string(),
            description: "Stub update_task_status for auto-groom tests".to_string(),
            input_schema: json!({"type": "object", "properties": {"task_id": {"type": "string"}, "status": {"type": "string"}}}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success("Status updated".to_string()))
    }
}

/// Stub `check_task` — simulates task status checks.
struct StubCheckTask;

#[async_trait]
impl Tool for StubCheckTask {
    fn name(&self) -> &str {
        "check_task"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "check_task".to_string(),
            description: "Stub check_task for auto-groom tests".to_string(),
            input_schema: json!({"type": "object", "properties": {"task_id": {"type": "string"}}}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            r#"{"task_id": "00000000-0000-0000-0000-000000000042", "status": "in_progress"}"#
                .to_string(),
        ))
    }
}

/// Stub `list_tasks` — simulates task listing.
struct StubListTasks;

#[async_trait]
impl Tool for StubListTasks {
    fn name(&self) -> &str {
        "list_tasks"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_tasks".to_string(),
            description: "Stub list_tasks for auto-groom tests".to_string(),
            input_schema: json!({"type": "object", "properties": {}}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            "1 item — 1 in_progress\n| WI | Status | Type | Source | Label |\n| abc-123 | in_progress | milestone | manual | milestone#13 |".to_string(),
        ))
    }
}

/// Build a tool registry with auto-groom stubs plus milestone/callback tools.
fn tools_with_milestone_stubs() -> mika_agent::tools::ToolRegistry {
    let mut tools = tools_with_auto_groom_stubs();
    tools.register(Box::new(StubUpdateTaskStatus));
    tools.register(Box::new(StubCheckTask));
    tools.register(Box::new(StubListTasks));
    tools
}

/// #996 — Milestone-cascade / M4 path: M4 Step 1.5 fires dev-groom for an
/// ungroomed milestone child. This is the load-bearing path given the mika#671
/// incident that motivated this PR. When resuming a milestone and the next child
/// issue lacks a Plan callout, the agent must dispatch dev-groom before dev-pilot.
///
/// The user message triggers the `resume_reconcile` intent guard (detects
/// "resume" + "milestone" keywords). The guard requires a successful `check_task`
/// or `list_tasks` call. The sequence models: reconcile → fetch issue body →
/// update child status → dispatch dev-groom → end turn.
#[tokio::test]
async fn milestone_cascade_ungroomed_child_fires_dev_groom() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: check_task to reconcile milestone state (satisfies
            // resume_reconcile guard)
            tool_call_response(
                "check_task",
                json!({"task_id": "00000000-0000-0000-0000-000000000042"}),
            ),
            // Step 2: run_gh to fetch next child's issue body (no Plan callout)
            tool_call_response(
                "run_gh",
                json!({"args": "issue view 671 --json title,body"}),
            ),
            // Step 3: update_task_status to mark child as in_progress for grooming
            tool_call_response(
                "update_task_status",
                json!({"task_id": "00000000-0000-0000-0000-000000000043", "status": "in_progress"}),
            ),
            // Step 4: run_claude_pilot with skill="dev-groom" (M4 Step 1.5)
            tool_call_response(
                "run_claude_pilot",
                json!({
                    "skill": "dev-groom",
                    "prompt": "senara-solutions/mika issue#671",
                    "task_id": "00000000-0000-0000-0000-000000000043"
                }),
            ),
            // Step 5: final text — agent launched grooming for ungroomed child
            text_response(
                "Launched dev-groom for mika#671 (ungroomed milestone child). \
                 Waiting for architect review before dispatching dev-pilot.",
            ),
            // Step 6: continuation (if any guard re-prompts after step 5)
            text_response("Dev-groom dispatched for milestone child mika#671. Awaiting callback."),
        ])
        .tools(tools_with_milestone_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run(
            "resume mika milestone#13 — next child is senara-solutions/mika#671 \
             (task_id: 00000000-0000-0000-0000-000000000043)",
        )
        .await
        .unwrap();

    assert_has_output(&trace);
    // The agent calls run_claude_pilot with skill="dev-groom", NOT dev-pilot
    assert_tools_include(&trace, &["run_claude_pilot"]);
    assert_tool_args_contain(&trace, "run_claude_pilot", 0, json!({"skill": "dev-groom"}));
    // dev-pilot should NOT appear in this turn — grooming must complete first
    let pilot_skills: Vec<_> = trace
        .tool_calls
        .iter()
        .filter(|tc| {
            tc.tool_name == "run_claude_pilot"
                && tc.input.as_deref().unwrap_or("").contains("\"dev-pilot\"")
        })
        .collect();
    assert!(
        pilot_skills.is_empty(),
        "dev-pilot should NOT be dispatched for an ungroomed milestone child — \
         dev-groom must run first. Found {} dev-pilot calls.",
        pilot_skills.len()
    );
}

/// #996 — ESCALATE callback turn: when dev-groom's callback delivers
/// `Verdict: ESCALATE`, the agent must notify the operator via send_message
/// and must NOT dispatch dev-pilot. This tests the callback turn (Phase 2 step f),
/// not just the initial groom dispatch.
#[tokio::test]
async fn webhook_escalate_callback_notifies_operator_and_blocks_dispatch() {
    // Simulate the callback turn: dev-groom has completed and returned ESCALATE.
    // The callback message arrives as a [callback:...] prefixed message.
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: update_task_status to mark groom task as failed/blocked
            tool_call_response(
                "update_task_status",
                json!({
                    "task_id": "00000000-0000-0000-0000-000000000042",
                    "status": "blocked"
                }),
            ),
            // Step 2: send_message to operator with ESCALATE reason
            tool_call_response(
                "send_message",
                json!({
                    "message": "Auto-groom ESCALATE for senara-solutions/mika#500: \
                    Architect review found structural issues that require operator intervention. \
                    The plan violates the grounding rule — claims downstream system state without \
                    tool confirmation. Halting dispatch."
                }),
            ),
            // Step 3: final text — agent confirms escalation, no dispatch
            text_response(
                "Dev-groom returned Verdict: ESCALATE for mika#500. \
                 Notified operator. Dispatch halted.",
            ),
        ])
        .tools(tools_with_milestone_stubs())
        .callback_turn(true)
        .build()
        .await
        .unwrap();

    let trace = harness
        .run(
            "[callback: long_running:run_claude_pilot] \
             Grooming completed for senara-solutions/mika#500.\n\
             Architect review found structural issues that require operator intervention.\n\
             The plan violates the grounding rule — claims downstream system state without \
             tool confirmation.\n\
             Verdict: ESCALATE",
        )
        .await
        .unwrap();

    assert_has_output(&trace);
    // send_message must have been called (operator notification)
    assert_tools_include(&trace, &["send_message"]);
    // dev-pilot must NOT have been dispatched
    let pilot_calls: Vec<_> = trace
        .tool_calls
        .iter()
        .filter(|tc| {
            tc.tool_name == "run_claude_pilot"
                && tc.input.as_deref().unwrap_or("").contains("\"dev-pilot\"")
        })
        .collect();
    assert!(
        pilot_calls.is_empty(),
        "dev-pilot should NOT be dispatched when dev-groom returns ESCALATE. \
         Found {} dev-pilot calls.",
        pilot_calls.len()
    );
    // No run_claude_pilot at all — ESCALATE halts the pipeline
    assert_tools_exclude(&trace, &["run_claude_pilot"]);
}

/// #996 AC#4 — 5-ticket milestone cascade: enqueue 5 ungroomed tickets and
/// assert each gets a Plan-callout check before dev-pilot dispatch fires.
/// This verifies the M4 Step 1.5 grooming pre-flight at scale — the cascade
/// must not skip grooming for any child, even under sequential processing.
#[tokio::test]
async fn milestone_cascade_five_tickets_all_groomed_before_dispatch() {
    // Build a response sequence for 5 children. For each child:
    // 1. run_gh to fetch issue body (body contains Plan callout → already groomed)
    // 2. run_claude_pilot with skill="dev-pilot" (bypass grooming, dispatch directly)
    //
    // This tests that the agent checks EVERY child for the Plan callout before
    // dispatching, not just the first one. If any child were dispatched without
    // the Plan check, the test structure would need dev-groom calls mixed in.
    let mut responses = vec![
        // Initial: check_task to inspect milestone state
        tool_call_response(
            "check_task",
            json!({"task_id": "00000000-0000-0000-0000-000000000010"}),
        ),
        // list_tasks to see children
        tool_call_response("list_tasks", json!({"source": "self_dev"})),
    ];

    // For each of 5 children: fetch body (groomed) + dispatch dev-pilot
    for i in 1..=5 {
        let issue_num = 700 + i;
        let task_id = format!("00000000-0000-0000-0000-0000000000{:02}", 10 + i);

        // Fetch issue body — contains Plan callout (already groomed)
        responses.push(tool_call_response(
            "run_gh",
            json!({"args": format!("issue view {} --json title,body", issue_num)}),
        ));
        // Update child status to in_progress
        responses.push(tool_call_response(
            "update_task_status",
            json!({"task_id": &task_id, "status": "in_progress"}),
        ));
        // Dispatch dev-pilot (Plan callout was present → bypass grooming)
        responses.push(tool_call_response(
            "run_claude_pilot",
            json!({
                "skill": "dev-pilot",
                "prompt": format!("senara-solutions/mika#{}", issue_num),
                "task_id": &task_id
            }),
        ));
    }

    // Final summary text
    responses.push(text_response(
        "Dispatched dev-pilot for all 5 milestone children (701-705). \
         All had Plan callouts — no grooming required.",
    ));

    let harness = EvalHarness::builder()
        .responses(responses)
        .tools(tools_with_milestone_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness
        .run(
            "resume mika milestone#13 — dispatch children #701, #702, #703, #704, #705 \
             (all have Plan callouts in their issue bodies)",
        )
        .await
        .unwrap();

    assert_has_output(&trace);

    // All 5 run_claude_pilot calls must use skill="dev-pilot"
    let pilot_calls: Vec<_> = trace
        .tool_calls
        .iter()
        .filter(|tc| tc.tool_name == "run_claude_pilot")
        .collect();
    assert_eq!(
        pilot_calls.len(),
        5,
        "Expected 5 run_claude_pilot calls (one per child), got {}",
        pilot_calls.len()
    );
    for (i, call) in pilot_calls.iter().enumerate() {
        let input: serde_json::Value =
            serde_json::from_str(call.input.as_deref().unwrap_or("{}")).unwrap();
        assert_eq!(
            input.get("skill").and_then(|v| v.as_str()),
            Some("dev-pilot"),
            "Child {} dispatch should use skill='dev-pilot', got: {:?}",
            i + 1,
            input.get("skill")
        );
    }

    // All 5 children had their issue body fetched via run_gh (Plan callout check)
    let gh_calls: Vec<_> = trace
        .tool_calls
        .iter()
        .filter(|tc| {
            tc.tool_name == "run_gh" && tc.input.as_deref().unwrap_or("").contains("issue view")
        })
        .collect();
    assert!(
        gh_calls.len() >= 5,
        "Expected at least 5 run_gh issue-view calls (one per child for Plan check), got {}",
        gh_calls.len()
    );
}

/// #996 — Engine guard still accepts run_claude_pilot with skill="dev-groom"
/// as a valid dispatch. The webhook_ready_label_dispatch guard is skill-agnostic.
#[tokio::test]
async fn engine_guard_accepts_dev_groom_as_valid_dispatch() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: run_gh to remove label
            tool_call_response(
                "run_gh",
                json!({"args": "issue edit 500 --remove-label ready"}),
            ),
            // Step 2: run_gh to fetch issue body
            tool_call_response(
                "run_gh",
                json!({"args": "issue view 500 --json title,body"}),
            ),
            // Step 3: create_task
            tool_call_response(
                "create_task",
                json!({"label": "groom senara-solutions/mika#500"}),
            ),
            // Step 4: run_claude_pilot with dev-groom
            tool_call_response(
                "run_claude_pilot",
                json!({"skill": "dev-groom", "prompt": "mika issue#500"}),
            ),
            // Step 5: final text — guard should accept this
            text_response("Dispatched dev-groom for auto-grooming."),
        ])
        .tools(tools_with_auto_groom_stubs())
        .build()
        .await
        .unwrap();

    let trace = harness.run(READY_LABEL_MSG).await.unwrap();

    assert_has_output(&trace);
    // 5 steps means no guard rejection occurred — the guard accepted
    // run_claude_pilot (skill="dev-groom") as valid completion
    assert_exact_steps(&trace, 5);
}

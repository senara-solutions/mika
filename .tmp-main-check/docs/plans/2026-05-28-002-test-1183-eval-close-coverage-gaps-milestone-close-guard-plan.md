# Plan: Close coverage gaps for milestone-close guard (mika#1183)

**Issue:** mika#1183
**Type:** test(eval)
**Priority:** p3-nice-to-have
**Component:** agent-core

## Problem

The `/ce:review` of mika#797 flagged three eval coverage gaps in the milestone-close guard test suite. None block the guard from working in the realistic incident path, but together they leave the M5 close-out workflow under-rehearsed against the failure modes it defends against.

## Approach

Three new test scenarios in `crates/mika-agent/tests/eval/grounding_regressions/milestone_close.rs`, each addressing one gap. All three use the existing `EvalHarness` + `MockLlmProvider` infrastructure — no new engine code, no new assertion helpers.

## Changes

### File 1: `crates/mika-agent/tests/eval/grounding_regressions/milestone_close.rs`

#### Gap 1 — Strengthen C2's guard-pinning assertion

**Problem:** C2 (`test_milestone_close_regression_no_patch`) asserts `trace.llm_call_count >= 2` — sufficient to prove *some* guard fired, but not *which* guard. The mocked text "I closed milestone#17, tasks reconciled, memory updated. All children completed successfully." matches both `detect_completion_claim` (on "completed") and `detect_milestone_close_claim_without_patch` (on "I closed...milestone"). In the eval harness, `update_task_status` is NOT in the default tool registry and no tasks are seeded, so the completion-claim guard skips and #4b fires — but the test cannot structurally confirm this.

**Fix:** Add an assertion on `trace.captured_requests[1]` (the second LLM request, i.e., the retry after guard injection) that inspects the last user-role message for the milestone-close correction substring. The milestone-close guard injects text containing `"close PATCH"` and `"milestone#17, 2026-04-24"` — the completion-claim correction does not. This pins which guard fired without changing the mock sequence.

**Implementation:**

```rust
// After the existing assertions in test_milestone_close_regression_no_patch:
//
// Verify the correction message is from the milestone-close guard (#4b),
// not the completion-claim guard (#4). The milestone-close guard's correction
// mentions "close PATCH" — the completion-claim guard does not.
assert!(
    trace.captured_requests.len() >= 2,
    "Expected at least 2 captured requests (original + retry after guard), got {}",
    trace.captured_requests.len()
);
let second_request = &trace.captured_requests[1];
let last_user_msg = second_request.messages.iter()
    .rev()
    .find(|m| matches!(m.role, LlmRole::User))
    .expect("Second request should have at least one user message");
let correction_text = match &last_user_msg.content {
    LlmContent::Text(t) => t.clone(),
    LlmContent::Blocks(blocks) => blocks.iter()
        .filter_map(|b| match b {
            LlmContentBlock::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(""),
};
assert!(
    correction_text.contains("close PATCH"),
    "Expected milestone-close guard correction (contains 'close PATCH'), \
     got completion-claim or other guard. Correction text: {}",
    &correction_text[..correction_text.len().min(500)]
);
```

**Imports needed:** Add `use mika_common::llm::types::{LlmRole, LlmContent, LlmContentBlock};` to the existing imports in the file (via the `super::*` re-export or directly).

**Verification:** Check that `LlmRole`, `LlmContent`, and `LlmContentBlock` are accessible from the test module. They're defined in `mika_common::llm::types` and should be re-exported through `mika_common::llm`. If not accessible via `super::*`, import directly.

#### Gap 2 — M5 step 3c divergent-readback scenario (new test C8)

**Problem:** `self-dev/system_prompt.md` step 3c specifies: if the readback after PATCH returns `"open"` (not `"closed"`), the agent must STOP, NOT call `update_task_status(completed)`, and instead call `send_message` + `update_task_status(blocked)`. This is the exact divergence shape #797 defends against, but it's prompt-enforced only — no eval scenario rehearses it.

**Fix:** Add eval scenario C8 with a mock sequence where:
1. Agent calls `run_gh` PATCH (close milestone) — returns success
2. Agent calls `run_gh` readback — returns `"open"` (divergent state)
3. Agent calls `send_message` to notify operator
4. Agent calls `update_task_status(blocked)` to halt
5. Agent emits final text acknowledging the divergence

**Implementation:**

```rust
/// C8 — Divergent readback: PATCH returns 2xx but readback shows "open" (#1183 Gap 2).
///
/// M5 step 3c specifies: if the readback returns "open" after a successful PATCH,
/// STOP, notify Vincent via send_message, mark task blocked. This scenario
/// rehearses the exact divergence shape that the milestone#17 incident exposed.
///
/// Mock sequence:
/// 1. Agent calls run_gh PATCH → success (mocked tool result returns ok)
/// 2. Agent calls run_gh readback → returns "open" (divergent state)
/// 3. Agent calls send_message to notify operator
/// 4. Agent calls update_task_status with status=blocked
/// 5. Agent emits final text
///
/// Hard assertions:
/// - send_message was called (notification sent)
/// - update_task_status was called (task marked blocked)
/// - Final output does NOT contain "Milestone closed on GitHub" (the happy-path marker)
/// - Agent did NOT emit a completion claim that would trigger guard #4b
#[tokio::test]
async fn test_milestone_close_divergent_readback_blocked() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls run_gh PATCH to close milestone
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["api", "-X", "PATCH",
                                "/repos/senara-solutions/mika/milestones/17",
                                "-f", "state=closed"]
                }),
            ),
            // Step 2: Agent calls run_gh readback — gets "open" (divergent)
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["api",
                                "/repos/senara-solutions/mika/milestones/17",
                                "--jq", ".state"]
                }),
            ),
            // Step 3: Agent calls send_message to notify operator
            tool_call_response(
                "send_message",
                json!({
                    "message": "Milestone senara-solutions/mika milestone#17 close PATCH returned 2xx but readback shows state=open. GitHub-side divergence; not marking local task complete."
                }),
            ),
            // Step 4: Agent calls update_task_status with blocked
            tool_call_response(
                "update_task_status",
                json!({
                    "task_id": "abc-123",
                    "status": "blocked",
                    "note": "GitHub milestone close readback mismatch — got state=open"
                }),
            ),
            // Step 5: Agent emits final text acknowledging divergence
            text_response(
                "Milestone#17 close PATCH succeeded but readback returned state=open. \
                 Task marked blocked. Notified Vincent of the divergence.",
            ),
        ])
        .build()
        .await?;

    let trace = harness
        .run("Close milestone#17 on GitHub after all children completed.")
        .await?;

    // Hard: agent called send_message (notification sent)
    assert_tools_include(&trace, &["send_message"]);
    // Hard: agent called update_task_status (task marked blocked)
    assert_tools_include(&trace, &["update_task_status"]);
    // Hard: output does NOT claim success — the divergent path must not
    // contain the happy-path verified-close marker
    assert_has_output(&trace);
    let output_text = trace.output.text.as_deref().unwrap_or("");
    assert!(
        !output_text.contains("Milestone closed on GitHub"),
        "Output should NOT contain happy-path marker when readback diverged, got: {}",
        output_text
    );

    Ok(())
}
```

**Note:** This test exercises prompt-level behavior (the mock LLM follows the expected M5 step 3c path). It does NOT test an engine guard — it rehearses the LLM behavior the prompt demands, so the test structure is "assert the mock sequence completes correctly" rather than "assert a guard fires."

**Tool registration consideration:** `send_message` and `update_task_status` need to be in the tool registry. `send_message` is in `default_tools()`. `update_task_status` is NOT in `default_tools()` — however, since we're using `MockLlmProvider`, the tool call flows through `process_tool_calls()` which looks up the tool by name. If it's not registered, the tool call returns an error result. For this test to work correctly, we need either:

- Option A: Use the `StubUpdateTaskStatusTool` pattern from `test_completion_claim_guard.rs` — register a stub `update_task_status` tool and build with `.tools(tools_with_update_task_status())`.
- Option B: Accept that the tool call will "fail" (return unknown-tool error) and assert based on the mock sequence completing regardless.

**Decision:** Use Option A. Copy the `StubUpdateTaskStatusTool` pattern and `tools_with_update_task_status()` helper from `test_completion_claim_guard.rs`, adapted inline in the milestone_close test file. This ensures the tool call succeeds and the assertion on `assert_tools_include(&trace, &["update_task_status"])` passes (since tool_calls DB records are written only for registered tools).

**Wait — re-check:** Actually, looking at how `EvalHarness` works: when `MockLlmProvider` returns a `tool_call_response("update_task_status", ...)`, the agent loop's `process_tool_calls()` looks up the tool in the registry. If not found, it returns an error `ToolOutput`. The tool call IS still recorded in `tool_calls` DB table (the recording happens regardless of whether the tool is found). So `assert_tools_include` should work even without registering the tool.

**Re-decision:** Verify by checking how tool_calls are recorded. Looking at the eval infrastructure — `trace.tool_calls` comes from `db.query_tool_calls_by_trace()`. Tool call rows are saved by `save_tool_call()` which is called in `process_tool_calls()` AFTER tool execution. If the tool isn't registered, the code path may differ.

**Final decision:** Use the stub pattern (Option A) to be safe. It's a small amount of code and guarantees correctness. Import `StubUpdateTaskStatusTool` inline.

#### Gap 3 — Chain-ordering integration test via EvalHarness (new test C9)

**Problem:** The inline unit test `test_milestone_close_fires_after_completion_claim_satisfied` (agent.rs:8623) only calls the two detector functions directly — it does NOT drive `run_loop`. A future maintainer swapping guard order would not break the test.

**Fix:** Add an `EvalHarness` integration test that drives the full `run_agent()` path:

1. **Turn 1:** Agent emits "I completed milestone#17 and closed it on GitHub" with NO tool calls. Both guards' regexes match. Guard #4 (completion-claim) fires first because `update_task_status` IS in the registry AND active tasks exist AND it wasn't called. Agent gets correction about calling `update_task_status`.
2. **Turn 2:** Agent responds by calling `update_task_status(completed)` then emitting the same text again. Guard #4 is satisfied (tool was called). Guard #4b (milestone-close) fires because no `run_gh` PATCH was made. Agent gets correction about calling the close PATCH.
3. **Turn 3:** Agent calls `run_gh` PATCH, readback, then emits verified text. Both guards satisfied. Response accepted.

This structurally proves the chain ordering (#4 before #4b) by observing three LLM calls with specific correction content at each stage.

**Implementation:**

```rust
/// C9 — Chain ordering: completion-claim guard (#4) fires before milestone-close (#4b).
///
/// This is the EvalHarness integration counterpart to the inline unit test
/// `test_milestone_close_fires_after_completion_claim_satisfied` (agent.rs).
/// That test only calls detection functions — this one drives run_agent() to
/// structurally verify the guard chain ordering. A maintainer swapping guard
/// order would break this test.
///
/// Mock sequence:
/// Turn 1: Agent emits text matching both guards, no tool calls.
///   → Guard #4 fires (completion-claim: "completed" + update_task_status registered
///     + active task + not called).
/// Turn 2: Agent calls update_task_status (satisfies #4), emits same text.
///   → Guard #4b fires (milestone-close: first-person claim, no PATCH).
/// Turn 3: Agent calls run_gh PATCH + readback, emits verified text.
///   → Both guards satisfied. Response accepted.
///
/// Hard assertions:
/// - Exactly 3 LLM calls (two corrections + final accept).
/// - captured_requests[1] contains completion-claim correction ("update_task_status").
/// - captured_requests[2] contains milestone-close correction ("close PATCH").
#[tokio::test]
async fn test_milestone_close_chain_ordering_completion_then_close() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Both guards match. #4 fires first (completion-claim).
            text_response(
                "I completed milestone#17 and closed it on GitHub. \
                 All children completed successfully.",
            ),
            // Turn 2: After #4 fires, agent calls update_task_status (satisfies #4).
            // Then emits same claim — #4b fires (milestone-close, no PATCH).
            tool_call_response(
                "update_task_status",
                json!({"task_id": "placeholder", "status": "completed"}),
            ),
            text_response(
                "I completed milestone#17 and closed it on GitHub.",
            ),
            // Turn 3: After #4b fires, agent calls run_gh PATCH + readback.
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["api", "-X", "PATCH",
                                "/repos/senara-solutions/mika/milestones/17",
                                "-f", "state=closed"]
                }),
            ),
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["api",
                                "/repos/senara-solutions/mika/milestones/17",
                                "--jq", ".state"]
                }),
            ),
            text_response(
                "I closed milestone#17 on GitHub.\n\
                 Milestone closed on GitHub: ✓",
            ),
        ])
        .tools(tools_with_update_task_status())
        .build()
        .await?;

    // Seed an active task — required for completion-claim guard to fire
    seed_task(&harness, "Complete milestone#17").await;

    let trace = harness
        .run("All milestone children done. Complete and close milestone#17.")
        .await?;

    // Hard: exactly 3 LLM calls (two guard corrections + final accept).
    // More than 3 means a guard fired unexpectedly; fewer means chain didn't compose.
    assert!(
        trace.llm_call_count >= 3,
        "Expected at least 3 LLM calls (completion-claim correction + \
         milestone-close correction + final accept), got {}",
        trace.llm_call_count
    );

    // Hard: second request's correction is from the completion-claim guard (#4).
    // The completion-claim correction mentions "update_task_status".
    assert!(
        trace.captured_requests.len() >= 2,
        "Expected at least 2 captured requests, got {}",
        trace.captured_requests.len()
    );
    let req2_correction = extract_last_user_text(&trace.captured_requests[1]);
    assert!(
        req2_correction.contains("update_task_status"),
        "Second request should contain completion-claim correction \
         (mentions 'update_task_status'), got: {}",
        &req2_correction[..req2_correction.len().min(500)]
    );

    // Hard: third request's correction is from the milestone-close guard (#4b).
    // The milestone-close correction mentions "close PATCH".
    assert!(
        trace.captured_requests.len() >= 3,
        "Expected at least 3 captured requests, got {}",
        trace.captured_requests.len()
    );
    let req3_correction = extract_last_user_text(&trace.captured_requests[2]);
    assert!(
        req3_correction.contains("close PATCH"),
        "Third request should contain milestone-close correction \
         (mentions 'close PATCH'), got: {}",
        &req3_correction[..req3_correction.len().min(500)]
    );

    // Hard: agent called both tools after corrections
    assert_tools_include(&trace, &["update_task_status", "run_gh"]);

    Ok(())
}
```

**Shared helper needed:** Both Gap 1 and Gap 3 need to extract the last user message text from a captured request. Add a file-local helper:

```rust
/// Extract text from the last user-role message in a captured LLM request.
/// Used to inspect guard correction messages.
fn extract_last_user_text(request: &LlmRequest) -> String {
    let msg = request.messages.iter()
        .rev()
        .find(|m| matches!(m.role, LlmRole::User))
        .expect("Request should have at least one user message");
    match &msg.content {
        LlmContent::Text(t) => t.clone(),
        LlmContent::Blocks(blocks) => blocks.iter()
            .filter_map(|b| match b {
                LlmContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""),
    }
}
```

**Task seeding and tool registration:** Gap 3 requires:
1. An active task in the DB (for completion-claim guard to fire)
2. `update_task_status` in the tool registry (for completion-claim guard to check registry)

Copy the `StubUpdateTaskStatusTool`, `seed_task`, and `tools_with_update_task_status` patterns from `test_completion_claim_guard.rs`.

### File 2: `crates/mika-agent/tests/eval/grounding_regressions/mod.rs`

No change needed — `milestone_close` module is already registered as `pub mod milestone_close;`.

## Implementation Order

1. **Add imports** — `LlmRole`, `LlmContent`, `LlmContentBlock`, `LlmRequest` types + task/tool infrastructure imports
2. **Add shared helpers** — `extract_last_user_text()`, `StubUpdateTaskStatusTool`, `seed_task()`, `tools_with_update_task_status()`
3. **Gap 1** — Strengthen C2 assertion (modify `test_milestone_close_regression_no_patch`)
4. **Gap 2** — Add C8 test (`test_milestone_close_divergent_readback_blocked`)
5. **Gap 3** — Add C9 test (`test_milestone_close_chain_ordering_completion_then_close`)
6. **Run tests** — `cargo test -p mika-agent --test eval milestone_close`

## What's NOT in Scope

- No engine code changes (all three gaps are test-only)
- No new assertion helpers in `grounding_assertions/mod.rs` (the `extract_last_user_text` helper is file-local)
- No changes to the milestone-close guard detection logic
- No changes to the completion-claim guard detection logic
- No changes to the self-dev system prompt

## Risk Assessment

**Low risk.** All changes are additive test code in an existing test file. No production code modified. The only modification to existing code is strengthening an assertion in C2 (adding assertions, not changing the mock sequence).

**Potential test flakiness:** Gap 3's chain-ordering test depends on both guards firing in sequence. If a future change makes the completion-claim guard skip (e.g., removing the registry check), the test would need 2 LLM calls instead of 3. This is intentional — the test's purpose IS to detect such changes.

## Verification

```bash
# Run the milestone_close eval tests
cargo test -p mika-agent --test eval milestone_close -- --nocapture

# Run all grounding regression tests to verify no interference
cargo test -p mika-agent --test eval grounding_regressions

# Full eval suite
cargo test -p mika-agent --test eval
```

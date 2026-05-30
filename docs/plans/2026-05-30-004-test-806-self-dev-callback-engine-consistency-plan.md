# Plan: Integration test covering documented callback-handler branches against engine guards

**Ticket:** mika issue#806
**Type:** test
**Date:** 2026-05-30

## Problem

`self-dev-callback/system_prompt.md` documents callback-handler branches that prescribe specific tool calls. Some of these prescribed calls are rejected by engine guards at runtime — the skill text and engine guards are out of sync. This was exposed when a production `error_max_turns` callback hit the long-running tool guard (#798 incident). No automated test validates that documented callback prescriptions are actually executable under the engine's guard chain.

## Goal

A CI-gated integration test suite that, for each documented callback-handler branch in `self-dev-callback/system_prompt.md`, simulates the matching callback context and verifies the prescribed tool calls either execute or are properly deferred (not hard-rejected) by the engine.

## Approach

### Architecture

Create a new test file `crates/mika-agent/tests/eval/test_self_dev_callback_engine_consistency.rs` following the established pattern from `test_callback_terminal_action.rs`:

- **Conversation mode** with `[callback: ...]` prefix in user messages (not `.callback_turn(true)`) — this is how guards trigger on the `[callback:` prefix in `user_input_text`
- **Stub tools** for the tools each handler branch prescribes
- **MockLlmProvider** sequences that drive the agent to invoke the prescribed tools
- **Hard assertions** (fail, not warn) on whether each tool call was accepted or rejected

### What the test validates

For each callback branch, the test constructs a synthetic callback message matching that branch's trigger pattern, mocks the LLM to call the tools the skill document prescribes, and asserts the engine accepts (or properly defers) those calls. This catches the class of bug where a skill prescribes `run_claude_pilot` from callback context but the engine hard-rejects it.

## Implementation Steps

### Step 1 — Stub tools module

Create stub implementations for all tools prescribed across the four callback branches. Following the `test_callback_terminal_action.rs` pattern (struct per tool, `#[async_trait] impl Tool`):

| Stub tool | Returns | Used by branches |
|-----------|---------|-----------------|
| `StubUpdateTaskStatusTool` | `success("Task updated.")` | All branches |
| `StubSendMessageTool` | `success("Message sent.")` | All branches |
| `StubCheckTaskTool` | `success(json!({"task_id": "...", "status": "in_progress", "label": "long_running:run_claude_pilot:...", "metadata": {...}}))` | Pipeline failure, success, failure |
| `StubRunGhTool` | `success("...")` with context-appropriate output | Success (PR discovery), failure (PR check) |
| `StubRunClaudePilotTool` | `success(json!({"status": "deferred", "deferred": true}))` | Pipeline failure (retry) |
| `StubRunShellTool` | `success("...")` | Pipeline failure (log read) |

Helper: `fn tools_with_callback_stubs() -> ToolRegistry` — registers `default_tools()` plus all stubs (stubs override builtins by name).

### Step 2 — Test case: pipeline failure callback

**Trigger pattern:** Callback result text starts with `"PIPELINE FAILURE:"`

**Documented prescription (self-dev-callback §85-91):**
1. `check_task` — read retry count from metadata
2. If retries < 2: `update_task_status` (increment retry count) → `run_claude_pilot` (re-dispatch) → `send_message` (notify)
3. If retries >= 2: `update_task_status` (status: blocked) → `send_message` (escalate)

**Mock LLM sequence:**
- Response 1: tool_call `check_task` → returns metadata with `pipeline_retry_count: 0`
- Response 2: tool_call `update_task_status` with `pipeline_retry_count: 1`
- Response 3: tool_call `run_claude_pilot` with same task_id → **this is the critical assertion**: must return `{"status": "deferred", "deferred": true}` (deferred dispatch), NOT a hard error
- Response 4: tool_call `send_message` (notification)
- Response 5: text response (EndTurn)

**Assertions:**
- `run_claude_pilot` was called and did NOT produce an error containing "cannot run in the current context"
- `update_task_status` and `send_message` were both called (callback_terminal_action guard satisfied)
- Agent loop completed without max-steps exceeded

**Sub-case: retry exhausted (pipeline_retry_count >= 2):**
- `StubCheckTaskTool` returns metadata with `pipeline_retry_count: 2`
- LLM calls `update_task_status(status: blocked)` + `send_message` (escalation)
- `run_claude_pilot` is NOT called
- Assert: loop completes, no dispatch attempted

### Step 3 — Test case: success callback

**Trigger pattern:** Callback result text contains structured metadata lines (Session/Cost/Turns/Duration/PR) without `"PIPELINE FAILURE:"` prefix.

**Documented prescription (self-dev-callback §92-97):**
1. `check_task` — read label to classify callback type
2. `run_gh` with `pr list --head <branch>` — discover PR URL
3. `update_task_status` — persist metadata with pr_url
4. `send_message` — notify operator

**Mock LLM sequence:**
- Response 1: tool_call `check_task` → label: `long_running:run_claude_pilot:dev-pilot:mika#100`
- Response 2: tool_call `run_gh` with pr list args → returns PR URL
- Response 3: tool_call `update_task_status` with metadata
- Response 4: tool_call `send_message`
- Response 5: text response (EndTurn)

**Assertions:**
- All four tools called successfully (none rejected)
- `run_claude_pilot` is NOT called (no retry on success)
- Loop completes cleanly

### Step 4 — Test case: failure callback

**Trigger pattern:** Callback result contains `"FAILED"` or non-zero exit indicator, no `"PIPELINE FAILURE:"` prefix.

**Documented prescription (self-dev-callback §99, §106-138):**
1. `check_task` — read task details
2. `run_gh` with `pr list --head <branch>` — check if PR exists despite failure
3. If no PR: `run_shell` to read logs, classify, possibly `run_claude_pilot` for recoverable failure
4. `update_task_status` — with failure/blocked status
5. `send_message` — notify operator

**Mock LLM sequence (no PR exists path):**
- Response 1: tool_call `check_task`
- Response 2: tool_call `run_gh` (pr list) → returns empty (no PR)
- Response 3: tool_call `run_shell` (tail log file) → returns log snippet
- Response 4: tool_call `update_task_status(status: blocked)`
- Response 5: tool_call `send_message` (escalation)
- Response 6: text response (EndTurn)

**Mock LLM sequence (PR exists path):**
- Response 1: tool_call `check_task`
- Response 2: tool_call `run_gh` (pr list) → returns PR URL
- Response 3: tool_call `update_task_status` with success metadata
- Response 4: tool_call `send_message`
- Response 5: text response (EndTurn)

**Assertions:**
- All prescribed tools called without engine rejection
- On no-PR path: `run_shell` accepted (not blocked in callback context — it's not `long_running`)
- On PR-exists path: routes to success handling

### Step 5 — Test case: error_max_turns callback (post-#804)

**Trigger pattern:** Callback result text contains `error_max_turns` literal.

**Documented prescription (self-dev-callback §52-83):**
1. `check_task` — read metadata, extract branch
2. `run_gh` with `pr list --head <branch>` — check for PR
3. If no PR but commits exist: `update_task_status` with `unpushed_recovery_pending: true` → `send_message` (recovery notification)
4. If no PR and no commits: `update_task_status(status: blocked)` → `send_message` (escalation)

This path should NOT call `run_claude_pilot` — the documented handler routes to `recover_unpushed_work` or failure, not retry.

**Mock LLM sequence (recover_unpushed_work path):**
- Response 1: tool_call `check_task` → returns metadata with branch
- Response 2: tool_call `run_gh` (pr list) → empty
- Response 3: tool_call `run_shell` (`git log`) → returns commit list
- Response 4: tool_call `update_task_status` with `unpushed_recovery_pending: true`
- Response 5: tool_call `send_message` (recovery details)
- Response 6: text response (EndTurn)

**Assertions:**
- `run_claude_pilot` was NOT called (no re-dispatch on `error_max_turns`)
- `update_task_status` and `send_message` both called
- Loop completes cleanly

### Step 6 — Test case: groom callback (GROOMED path)

**Trigger pattern:** Callback result from a dev-groom task (label contains `run_claude_pilot_groom`).

**Documented prescription (self-dev-callback §14-32):**
1. `check_task` — read label, detect groom callback
2. `run_gh` (issue view) — check for `second-pass (GROOMED)` body marker
3. If GROOMED: `update_task_status(completed)` → `run_gh` (add `ready` label) → `send_message`
4. If not GROOMED: fall through to pipeline failure or failure handler

**Mock LLM sequence (GROOMED path):**
- Response 1: tool_call `check_task` → label: `long_running:run_claude_pilot_groom:dev-groom:mika#200`
- Response 2: tool_call `run_gh` (issue view body) → returns body with `second-pass (GROOMED)` marker
- Response 3: tool_call `update_task_status(completed)`
- Response 4: tool_call `run_gh` (issue edit --add-label ready)
- Response 5: tool_call `send_message`
- Response 6: text response (EndTurn)

**Assertions:**
- `run_claude_pilot` was NOT called (groom callback completes without re-dispatch)
- All three prescribed tools called
- `update_task_status` called with `completed` status

### Step 7 — Register the test module

Add `mod test_self_dev_callback_engine_consistency;` to `crates/mika-agent/tests/eval.rs` in the `mod eval` block.

### Step 8 — Negative test: long-running tool hard-rejected without callback context

Verify the engine's hard error path still fires when a long-running tool is called from a non-callback, non-conversation context (e.g., heartbeat). This is the complement — the engine SHOULD reject in contexts where deferred dispatch is unavailable.

**Setup:** Conversation mode (no callback context), user message does NOT have `[callback:` prefix, no `callback_task_id` on ToolContext.

**Mock LLM:** calls a stub long-running tool.

**Assertion:** Tool returns error containing "cannot run in the current context" or similar rejection message.

Note: This test may be better expressed as a unit test on `execute_skill_tool` directly rather than through the full agent loop — assess during implementation.

## File inventory

| File | Action |
|------|--------|
| `crates/mika-agent/tests/eval/test_self_dev_callback_engine_consistency.rs` | Create |
| `crates/mika-agent/tests/eval.rs` | Edit (add `mod` line) |

## Risks and mitigations

1. **Stub fidelity:** Stub tools return canned responses that may not match production tool output shape. Mitigation: keep stub responses minimal — the test validates engine acceptance, not tool output parsing. Use `json!({"status": "ok"})` shapes.

2. **Guard interaction complexity:** Callback turns trigger multiple guards simultaneously (terminal_action #870, milestone_advance #991, etc.). Some test cases may need to satisfy multiple guards. Mitigation: include `update_task_status` + `send_message` in every callback test case's mock sequence to satisfy the #870 terminal action guard.

3. **MockLlmProvider sequencing:** The agent loop may consume mock responses in unexpected order if post-conditions trigger retries. Mitigation: use `assert_exact_steps()` from the assertions module to verify step counts match expectations; add enough mock responses to cover one guard retry.

4. **Deferred dispatch vs direct execution:** The `run_claude_pilot` call in callback context returns `{"status": "deferred"}` instead of actually spawning. The test validates the engine accepts the call (no hard error), not that the deferred dispatch eventually fires. This is the correct scope — deferred dispatch delivery is tested separately in `test_deferred_dispatch_idempotent_ack.rs`.

## Non-goals

- Testing actual skill prompt parsing or keyword matching — this is an engine-level test
- Testing real LLM responses — MockLlmProvider drives deterministic sequences
- Extending to other skills (qa-review, dev-pilot, permission-policy) — the ticket says "start with self-dev, expand later if useful"
- Testing the full silent-mode dispatch path (`run_silent_agent`) — conversation mode with `[callback:` prefix is the established eval pattern per `test_callback_terminal_action.rs`

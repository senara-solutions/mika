---
title: "test: Integration test covering self-dev-callback handler branches against engine guards"
type: test
status: active
date: 2026-05-27
issue: mika#806
---

# test: Integration test covering self-dev-callback handler branches against engine guards

## Overview

Add a new eval test module `skill_engine_consistency/` with an initial test file for `self-dev-callback` handler branches. Each test constructs a synthetic callback context, drives the agent loop with `MockLlmProvider` to invoke the prescribed tool sequence, and verifies the engine permits the calls and post-condition guards are satisfied.

## Problem Frame

`skills/bundled/self-dev-callback/system_prompt.md` documents seven callback-handler branches prescribing specific tool calls. The engine's long-running guard (`executor.rs:297-358`) rejects `run_claude_pilot` in callback context without `LongRunningContext` — routing to deferred dispatch (post-#1058) or hard error. This skill-vs-engine inconsistency was never caught by tests and only surfaced in production (mika#798).

The gap: no automated check verifies that the tool sequences documented in skill prompts are actually executable under the engine's runtime guards for the execution context where they fire.

## Requirements Trace

- R1. For each documented callback-handler branch in `self-dev-callback/system_prompt.md`, a test simulates the matching callback context and verifies prescribed tools execute (or defer) without engine rejection
- R2. Tests must fail loudly (hard assertion failure, not skip or warn) when a documented prescription fails
- R3. Tests use `MockLlmProvider` + stub tools — no real LLM calls, no network
- R4. Module structure is generalizable to other skills (qa-review, dev-pilot, permission-policy)

## Scope Boundaries

- Only self-dev-callback handlers in this ticket; other skills are future work
- Tests verify agent-loop-level behavior (tool dispatch, post-condition guards), not executor internals (those have unit tests in `executor.rs::tests`)
- Does not fix the skill-engine inconsistency itself (that's mika#803/804); this is the regression-prevention layer

## Context & Research

### Callback Handler Location

The callback handler lives at `skills/bundled/self-dev-callback/system_prompt.md` (not `self-dev/system_prompt.md`). The `self-dev-callback` skill is a separate bundled skill with `dependencies = ["self-dev"]`, triggered by keywords like `"long_running:run_claude_pilot"`, `"callback"`, `"PIPELINE FAILURE"`, `"error_max_turns"`.

### Seven Documented Callback Branches

From `self-dev-callback/system_prompt.md`, the following branches are prescribed:

| Branch | Trigger | Prescribed tools | Engine constraint |
|--------|---------|-----------------|-------------------|
| 1. **Auto-skip** | Callback first line JSON with `"status": "auto_skipped"` | `update_task_status` (completed + metadata), proceed to Step 6 | Standard tools — no long_running |
| 2. **Groom callback — GROOMED** | Label `long_running:run_claude_pilot_groom` + body has `second-pass (GROOMED)` | `check_task`, `run_gh` (issue view), `update_task_status` (completed), `run_gh` (issue edit —add-label), `send_message` | Standard tools — no long_running |
| 3. **Groom callback — failure** | Label `long_running:run_claude_pilot_groom` + no GROOMED marker | Falls through to pipeline-failure or failure handlers | May involve `run_claude_pilot` (deferred) |
| 4. **Recover unpushed work** | `error_max_turns` marker + ≥1 commit on branch | `update_task_status` (in_progress + metadata), `send_message` | Standard tools — no long_running |
| 5. **Pipeline failure** | Callback contains `"PIPELINE FAILURE:"` | `check_task`, `update_task_status`, `run_claude_pilot` (retry, receives deferred-ack), `send_message` | `run_claude_pilot` is long_running → deferred dispatch in callback context |
| 6. **Success** | No failure prefix, PR URL in callback or discoverable | `run_gh` (pr list/view), `update_task_status`, `send_message` | Standard tools — no long_running |
| 7. **Failure** | `FAILED`/non-zero exit/non-JSON, no PR found | `run_gh` (pr list), `run_shell` (tail log), classify, `update_task_status`, `send_message` | May attempt `run_claude_pilot` retry (deferred) |

### Relevant Code and Patterns

- `tests/eval/test_callback_terminal_action.rs` — Primary pattern: stub tools, callback-prefix messages in conversation mode, intent-guard verification. Uses conversation mode (not `.callback_turn(true)`) because the guard triggers on `[callback:` prefix in `user_input_text`
- `tests/eval/test_deferred_dispatch_idempotent_ack.rs` — Stub `run_claude_pilot` returning production-shaped deferred-ack response `{"status": "deferred", "already_deferred": true}`
- `tests/eval/harness.rs` — `EvalHarnessBuilder` with `.callback_turn(bool)`, `.tools(registry)`, `.responses(vec![...])`
- `tests/eval/assertions.rs` — `assert_tools_include`, `assert_tool_output_contains`, `assert_no_tool_errors`, `assert_has_output`, `assert_system_prompt_contains`
- `skills/bundled/self-dev-callback/system_prompt.md` — Seven callback branches documented above
- `skills/executor.rs:297-358` — Three-tier long_running guard: conversation (spawn), callback (deferred dispatch via #1058), other (hard error)

### Institutional Learnings

- `docs/solutions/architecture-patterns/agent-eval-testing-harness-mock-provider.md` — Eval harness patterns for mock-based agent loop testing
- `docs/solutions/architecture-patterns/callback-exec-handler-tool-availability.md` — Tool availability rules in callback contexts
- `docs/solutions/logic-errors/callback-deferred-dispatch-gate-rejection-2026-05-10.md` — Deferred dispatch gate for callback turns with cycle detection

### Key Architectural Insight: Stub Tools Bypass Executor

The eval harness registers stub tools as builtins (via `tools.register()`), so they bypass the skill executor's `execute_skill_tool()` — including the long_running guard. This means:

- **What harness tests CAN verify:** Tool availability in callback context, agent-loop post-condition guards (intent guards, terminal action), tool-call sequencing, system-prompt framing
- **What harness tests CANNOT verify:** The executor's actual long_running rejection/deferred-dispatch routing (covered by unit tests in `executor.rs::tests`)

The stubs should return **production-shaped responses** (deferred-ack for `run_claude_pilot`, success for `update_task_status`/`send_message`/`run_gh`) so the test validates the agent-level behavioral contract: "given the tool responds like the real executor, does the agent loop complete correctly?"

## Key Technical Decisions

- **Conversation mode, not `.callback_turn(true)`**: Following `test_callback_terminal_action.rs` — the intent guards trigger on `[callback:` prefix in `user_input_text`, which is only populated in conversation mode. `.callback_turn(true)` skips DB persistence of the user message, leaving `user_input_text` empty
- **Stub tools for production shapes**: Each stub returns the response shape the real executor would produce (deferred-ack for long_running tools, success for standard tools), so the test validates downstream agent behavior
- **Module directory, not flat file**: `skill_engine_consistency/mod.rs` + `self_dev_callback_handlers.rs` — generalizable to other skills per R4
- **Seven tests covering seven documented branches**: Each with distinct callback-result payloads and prescribed tool sequences, plus a non-callback selectivity test

## Open Questions

### Resolved During Planning

- **Q: Should tests use `callback_turn(true)` or conversation mode?** Conversation mode — the intent guards depend on `user_input_text` which is only populated in conversation mode. Production uses `run_silent_agent` which constructs the message differently, but conversation mode is the correct eval harness representation per `test_callback_terminal_action.rs` precedent.
- **Q: How to test `run_claude_pilot` callability without the real executor?** Use a stub returning the deferred-dispatch shape (`{"status": "deferred", "deferred": true}`). This validates the agent-level contract: the tool is callable, the response is handled correctly, and the terminal action guard is satisfied.
- **Q: Which skill prompt file has the callback handlers?** `skills/bundled/self-dev-callback/system_prompt.md` — a separate skill from `self-dev`, with `dependencies = ["self-dev"]`. The callback handler was extracted into its own skill (mika#1106).

### Deferred to Implementation

- **Exact callback-result payloads**: The precise JSON/text structure of each callback branch's result string (pipeline failure marker, PR URL patterns, error messages, auto-skip JSON). Will be extracted from `self-dev-callback/system_prompt.md` and `dispatch-lib.sh` during implementation.
- **`run_shell` stub**: The failure branch prescribes `run_shell("tail -200 ...")` for log reading. The stub needs to return realistic log content for the classify step. Exact shape deferred.

## Output Structure

```
crates/mika-agent/tests/eval/
  skill_engine_consistency/
    mod.rs                           # Module root, re-exports
    self_dev_callback_handlers.rs    # Seven callback-branch tests + non-callback selectivity test
```

## Implementation Units

- [ ] **Unit 1: Create `skill_engine_consistency` module with self-dev-callback handler tests**

  **Goal:** Add seven tests (plus one selectivity test) verifying each documented self-dev-callback handler branch's prescribed tool sequence executes under the engine's callback-context guards.

  **Requirements:** R1, R2, R3, R4

  **Dependencies:** None

  **Files:**
  - Create: `crates/mika-agent/tests/eval/skill_engine_consistency/mod.rs`
  - Create: `crates/mika-agent/tests/eval/skill_engine_consistency/self_dev_callback_handlers.rs`
  - Modify: `crates/mika-agent/tests/eval.rs` (register `pub mod skill_engine_consistency;`)

  **Approach:**
  - Create stub tools: `StubRunClaudePilot` (returns deferred-ack shape), `StubRunClaudePilotGroom` (returns success), `StubUpdateTaskStatus` (returns success), `StubSendMessage` (returns success), `StubRunGh` (returns production-shaped PR list/issue view), `StubCheckTask` (returns task details with label/metadata), `StubRunShell` (returns log tail)
  - For each callback branch, construct a `[callback: long_running:run_claude_pilot]` (or `long_running:run_claude_pilot_groom`) message with the branch-appropriate result payload
  - Mock LLM responses drive the prescribed tool-call sequence for that branch
  - Assert: tools called in expected order, no tool errors (or expected deferred shape for `run_claude_pilot`), agent produces output, `callback_terminal_action` guard satisfied (both `update_task_status` AND `send_message` called)

  **Patterns to follow:**
  - `test_callback_terminal_action.rs` — conversation-mode callback testing with stub tools, intent-guard verification
  - `test_deferred_dispatch_idempotent_ack.rs` — stub `run_claude_pilot` returning production-shaped deferred-ack

  **Test scenarios:**

  1. **Auto-skip**: callback first line is `{"status": "auto_skipped", "reason": "issue_closed"}`. Mock LLM calls `update_task_status` (completed with `auto_skipped` metadata) then `send_message`. Assert both tools called, no errors.

  2. **Groom callback — GROOMED success**: callback label `long_running:run_claude_pilot_groom`. Mock LLM calls `check_task` → `run_gh` (issue view, returns body with `second-pass (GROOMED)`) → `update_task_status` (completed) → `run_gh` (issue edit —add-label ready) → `send_message`. Assert all five tools called, no errors. Verifies groom success path uses no long_running tools.

  3. **Groom callback — failure routing**: callback label `long_running:run_claude_pilot_groom`. Mock LLM calls `check_task` → `run_gh` (issue view, returns body WITHOUT GROOMED marker) → falls through to pipeline-failure path → `update_task_status` → `run_claude_pilot` (deferred-ack) → `send_message`. Assert `run_claude_pilot` returned deferred shape, terminal action guard satisfied.

  4. **Recover unpushed work (error_max_turns + commits)**: callback result contains `error_max_turns`. Mock LLM simulates grounding check (detecting commits on branch via `run_shell` or `run_gh`), then calls `update_task_status` (in_progress with `unpushed_recovery_pending: true` metadata) → `send_message` with verdict payload. Assert both tools called, no `run_claude_pilot` attempted (spec says DO NOT redispatch).

  5. **Pipeline failure (retry path)**: callback result contains `PIPELINE FAILURE:`. Mock LLM calls `check_task` (pipeline_retry_count < 2) → `update_task_status` (increment retry) → `run_claude_pilot` (receives deferred-ack `{"status": "deferred", "deferred": true}`) → `send_message`. Assert `run_claude_pilot` was called and returned deferred shape, terminal action guard satisfied.

  6. **Success**: callback result contains PR URL and structured metadata (session_id, turns, cost_usd). Mock LLM calls `run_gh` (pr list) → `run_gh` (pr view, mergeable) → `update_task_status` → `send_message`. Assert all tools called, no errors.

  7. **Failure (no PR found)**: callback result contains `FAILED`. Mock LLM calls `run_gh` (pr list, returns empty) → `run_shell` (tail log) → classify → `update_task_status` (blocked/failed) → `send_message`. Assert terminal action guard satisfied.

  8. **Non-callback selectivity**: same tools registered but user message lacks `[callback:` prefix. Verify `callback_terminal_action` guard does NOT fire (tool calls not required). Confirms guard selectivity. (Adapted from `test_callback_terminal_action.rs::callback_guard_skips_non_callback_messages`.)

  **Verification:**
  - `cargo test -p mika-agent --test eval skill_engine_consistency` passes
  - Each test fails hard if the prescribed tool sequence is rejected or post-condition guards fire unexpectedly
  - Adding a new callback branch to `self-dev-callback/system_prompt.md` without a corresponding test is detectable by grep/audit

## System-Wide Impact

- **Interaction graph:** Tests exercise the agent loop's post-condition chain (intent guards #6e `callback_terminal_action`) in callback context. No production code changes.
- **Error propagation:** Stub tools return success/deferred shapes; error paths tested via callback-result content, not tool failures.
- **Integration coverage:** The pipeline-failure test covers the `run_claude_pilot → deferred-ack → terminal action guard` chain that was the production failure mode (mika#798). The groom-callback tests cover the #1289 branch that was added after the original issue was filed.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Stub tools diverge from real executor response shapes over time | Name stubs after the production response shape they emulate (e.g., `StubDeferredAckPilotTool`); document the source of truth in stub comments |
| New callback branches added to `self-dev-callback/system_prompt.md` without tests | Module README or inline comment listing which branches are covered; `docs/solutions/` compound entry post-implementation |
| Groom callback branches (#1289) not yet finalized | Plan covers current shape; update tests if groom-callback contract changes before implementation |

## Sources & References

- Related issues: mika#806, mika#798 (production incident), mika#803 (retry primitive), mika#804 (error_max_turns handler), mika#1058 (deferred dispatch), mika#1106 (self-dev-callback extraction), mika#1289 (groom callback handler)
- Existing tests: `tests/eval/test_callback_terminal_action.rs`, `tests/eval/test_deferred_dispatch_idempotent_ack.rs`
- Skill prompt: `skills/bundled/self-dev-callback/system_prompt.md`
- Executor guard: `crates/mika-agent/src/skills/executor.rs:297-358`

---
title: "fix: required_tools gate should recognize terminal tool failures"
type: fix
status: active
date: 2026-04-20
---

# fix: required_tools gate should recognize terminal tool failures

## Overview

The required_tools post-condition gate wastes LLM calls by retrying when a required tool fails with an unrecoverable error. When one required tool in a workflow chain fails terminally (e.g., GitHub "Can not approve your own pull request"), the gate should allow EndTurn instead of forcing the agent to re-run the entire workflow.

## Problem Frame

The required_tools gate (guard #2 in the EndTurn chain) enforces that keyword-matched skills' `[constraints] required_tools` are all called before accepting the agent's response. When missing tools are detected, it rejects the response and re-prompts once.

The gate tracks tool calls via `tools_called: HashSet<String>`, populated from the LLM's ToolCall blocks **before** dispatch. So a tool that was called but failed IS in `tools_called`. The issue arises when one required tool fails terminally and the agent decides not to call the **remaining** required tools (because the workflow is broken). The gate sees missing tools and retries, causing the agent to re-execute the entire workflow and hit the same terminal error.

**Observed in production:** mika-qa trace `b12e6cbd5ce64b008be8369b21dced0b` (2026-04-10). `qa-review` requires `["qa_pr_view", "run_gh"]`. After `run_gh pr review --approve` failed with self-approval error at step 2, the agent tried to EndTurn. The gate rejected because `qa_pr_view` was never called. The agent re-ran the full review flow (steps 4-7), hitting the same error. Total: 9 LLM calls instead of 4.

## Requirements Trace

- R1. When a required tool was called and failed with a terminal error, the gate must allow EndTurn even if other required tools were not called
- R2. When no required tool has failed terminally, the existing retry behavior must be preserved (missing tools trigger one retry)
- R3. Terminal failure detection must work on the data available at gate time: `all_tool_summaries` (name, success, non_zero_exit, output_summary up to 300 chars)
- R4. The detection must be conservative — only classify errors as terminal when there is a positive signal of terminality, defaulting to retry for unknown errors
- R5. The fix must not affect the other four EndTurn guards
- R6. The fix must include integration tests exercising both the terminal-failure bypass and preserved retry behavior

## Scope Boundaries

- Only the required_tools gate in `agent.rs` is modified
- No changes to `ToolOutput`, `ToolCallSummary`, or tool dispatch
- No changes to skill manifests or `Constraints` struct

### Deferred to Separate Tasks

- Generalizing terminal error detection to other guards: separate issue if needed
- Adding terminal error patterns for non-GitHub tools: can be added incrementally to the pattern list

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/agent.rs:770-807` — the required_tools gate
- `crates/mika-agent/src/agent.rs:222-234` — `ToolCallSummary` struct with `success`, `non_zero_exit`, `output_summary`
- `crates/mika-agent/src/agent.rs:236-245` — `has_non_zero_exit_prefix()` for detecting non-zero exits
- `crates/mika-agent/src/agent.rs:1044-1049` — `tools_called` populated before dispatch
- `crates/mika-agent/src/agent.rs:1771-1778` — `ToolCallSummary` construction with `success: !output.is_error && !non_zero_exit`
- `crates/mika-agent/src/agent.rs:2864-2901` — `collect_required_tools()` and `filter_available_required_tools()`
- `crates/mika-agent/src/skills/builtin_handlers.rs:344-415` — `spawn_and_collect()` returns `ToolOutput::success()` even for non-zero exits, with error text in content
- `crates/mika-gateway/src/github.rs:339` — `ForwardResult` retryable/permanent pattern (design reference)

### Institutional Learnings

- `docs/solutions/prompt-engineering/required-tools-enforcement-gate.md` — original gate design; tracks calls by name, no output validation
- `docs/solutions/prompt-engineering/required-tools-availability-filter.md` — prior #516/#517 fix adding `filter_available_required_tools()`
- `docs/solutions/architecture-patterns/completion-claim-guard-work-item-state-enforcement.md` — guard chain ordering and `_retry_done` flag pattern
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — engine-level enforcement for against-gradient behaviors
- `docs/solutions/logic-errors/send-message-tool-false-success-on-gateway-error.md` — precedent for distinguishing tool success from failure

## Key Technical Decisions

- **Check `all_tool_summaries` for terminal failures on required tools, not `tools_called`:** `tools_called` only has names (no outcome). `all_tool_summaries` has `success`, `non_zero_exit`, and `output_summary` — sufficient for terminal error detection.

- **Any required tool with terminal failure waives the entire gate:** If any tool in `effective_required_tools` was called and failed terminally, the gate allows EndTurn regardless of other missing required tools. Rationale: required tools within a skill are part of the same workflow chain. If one fails terminally, the remaining tools are likely pointless. The agent is not fabricating — it tried and hit a real wall.

- **Terminal detection via positive pattern matching on `output_summary`:** Rather than trying to enumerate all retryable errors and treating everything else as terminal, use a curated list of known terminal error patterns. Unknown errors default to retryable (preserving existing behavior). The pattern list can grow incrementally.

- **Function-level encapsulation:** A new `has_terminal_required_tool_failure()` function checks `all_tool_summaries` against `effective_required_tools` for terminal patterns. Keeps the gate logic clean and the pattern list testable in isolation.

## Open Questions

### Resolved During Planning

- **Should the gate check all tool summaries or only required tool summaries?** Only required tools. A non-required tool failing terminally shouldn't waive required tool enforcement.
- **Should the agent's response text be checked for failure acknowledgment?** No — text matching is fragile (LLMs paraphrase). The positive signal is that a required tool was called and failed terminally, which is structural, not textual.

### Deferred to Implementation

- Exact terminal error pattern strings — the implementation will define initial patterns based on observed GitHub CLI errors, with room to extend.

## Implementation Units

- [x] **Unit 1: Terminal failure detection function**

**Goal:** Add a function that checks whether any required tool was called and failed with a terminal error, using `all_tool_summaries`.

**Requirements:** R1, R3, R4

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/agent.rs`

**Approach:**
- Add `fn has_terminal_required_tool_failure(required: &HashSet<String>, summaries: &[ToolCallSummary]) -> bool`
- For each summary where `name` is in `required` AND (`success == false` OR `non_zero_exit == true`): check `output_summary` against terminal error patterns
- Terminal patterns (case-insensitive substring matches on `output_summary`):
  - `"can not approve your own"` / `"can't review your own"` — GitHub self-action errors
  - `"http 404"` / `"not found"` — resource doesn't exist
  - `"http 403"` / `"forbidden"` — permission denied
  - `"http 401"` / `"unauthorized"` — auth failure
  - `"insufficient permissions"` / `"resource not accessible"` — GitHub App scope errors
  - `"permission denied"` — generic permission error
- Add a companion `fn is_terminal_tool_error(output: &str) -> bool` for the pattern matching, making it independently testable
- Explicitly exclude known retryable patterns first: `"http 429"`, `"rate limit"`, `"http 5"`, `"timed out"`, `"timeout"`, `"connection"` — if any retryable pattern matches, return false regardless of terminal patterns

**Patterns to follow:**
- `has_non_zero_exit_prefix()` at line 238 — similar content-inspection function
- `detect_completion_claim()` — similar pattern matching on response text for guard logic

**Test scenarios:**
- Happy path: required tool `run_gh` with `output_summary` containing `"Exit code: 1\nGraphQL: Can not approve your own pull request"` and `success: false` → returns true
- Happy path: required tool with `"HTTP 404: Not Found"` → returns true
- Happy path: required tool with `"HTTP 403: Forbidden"` → returns true
- Edge case: non-required tool fails terminally → returns false (only checks required tools)
- Edge case: required tool succeeds (`success: true`) → returns false
- Edge case: required tool fails with retryable error `"HTTP 429: rate limit exceeded"` → returns false
- Edge case: required tool fails with `"HTTP 500: Internal Server Error"` → returns false (5xx is retryable)
- Edge case: required tool fails with unknown error `"some random error"` → returns false (conservative default)
- Edge case: empty summaries → returns false
- Edge case: `output_summary` is empty string with `success: false` → returns false (no terminal pattern match)
- Edge case: case-insensitive match — `"Can Not Approve Your Own"` → returns true

**Verification:**
- All unit tests pass for the detection function in isolation

- [x] **Unit 2: Integrate terminal failure check into required_tools gate**

**Goal:** Wire the detection function into the gate so terminal failures bypass enforcement.

**Requirements:** R1, R2, R5

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/agent.rs`

**Approach:**
- In the required_tools gate block (lines 770-807), after computing `missing` tools: if `missing` is not empty, call `has_terminal_required_tool_failure(&effective_required_tools, &all_tool_summaries)`
- If true: log a `warn!` with the terminal failure details, set `required_tools_retry_done = true` (to prevent any future retry), and fall through (do NOT `continue` — let the response proceed to the next guard)
- If false: existing retry behavior (push correction message and `continue`)
- The `required_tools_retry_done` flag still prevents infinite loops if the detection somehow fails

**Patterns to follow:**
- Existing guard pattern: check condition → log → push messages → `continue`
- The terminal-failure path is the inverse: check condition → log → fall through

**Test scenarios:**
- Integration: EvalHarness with a skill declaring `required_tools = ["tool_a", "tool_b"]`, mock sequence where `tool_a` is called and returns terminal error, agent responds with text without calling `tool_b` → gate allows EndTurn, only 2 LLM calls (no retry)
- Integration: Same setup but `tool_a` succeeds and `tool_b` is never called → gate retries once (existing behavior preserved), 3 LLM calls
- Integration: `tool_a` called and returns retryable error (429), `tool_b` never called → gate retries (retryable errors don't bypass)

**Verification:**
- `cargo test -p mika-agent` passes
- `cargo clippy` clean
- The gate correctly bypasses for terminal failures and retries for non-terminal missing tools

- [x] **Unit 3: Integration tests via EvalHarness**

**Goal:** Add eval harness tests exercising the terminal failure bypass end-to-end through `run_agent()`.

**Requirements:** R1, R2, R6

**Dependencies:** Unit 1, Unit 2

**Files:**
- Create: `crates/mika-agent/tests/eval/test_required_tools_gate.rs`
- Modify: `crates/mika-agent/tests/eval/mod.rs` (add module)

**Approach:**
- Create a test skill entry with `constraints.required_tools = ["tool_a", "tool_b"]` and `triggers.keywords = ["review"]`
- Use `EvalHarness::builder().skills(registry).responses(mock_sequence).build()`
- For the terminal-failure test: mock response 1 calls `tool_a` (which returns terminal error via `ToolOutput::error()`), mock response 2 is EndTurn text — expect 2 LLM calls
- For the retry-preserved test: mock response 1 is EndTurn text without calling either tool, mock response 2 calls both tools, mock response 3 is EndTurn — expect 3 LLM calls
- For the retryable-error test: mock response 1 calls `tool_a` (retryable error), mock response 2 is EndTurn text — gate retries, expect 3 LLM calls

**Patterns to follow:**
- `test_completion_claim_guard.rs` — same pattern of setting up skills with specific behaviors and asserting LLM call counts
- `test_phantom_retry_guard.rs` — task-aware guard testing with EvalHarness

**Test scenarios:**
- Happy path: Terminal failure on required tool → EndTurn accepted, `llm_call_count == 2`
- Happy path: All required tools called successfully → EndTurn accepted, `llm_call_count == 2`
- Error path: Missing required tools with no failure → retry fires, `llm_call_count == 3`
- Error path: Missing required tools with retryable failure → retry fires, `llm_call_count == 3`

**Verification:**
- `cargo test -p mika-agent --test eval` passes
- Tests exercise the full `run_agent()` path, not just unit functions

## System-Wide Impact

- **Interaction graph:** Only the required_tools gate is modified. The other four EndTurn guards are unaffected. `ToolCallSummary` is read-only — no changes to how summaries are built.
- **Error propagation:** Terminal failures allow EndTurn to proceed. The agent's text response (explaining the failure) reaches the user/caller normally.
- **State lifecycle risks:** `required_tools_retry_done` is set to true on terminal failure detection, preventing any accidental retry path.
- **API surface parity:** No API changes. The gate is internal to the agent loop.
- **Unchanged invariants:** `tools_called` tracking, `filter_available_required_tools()`, `collect_required_tools()`, and all other guards remain unchanged. Skills that don't declare `required_tools` are unaffected.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Terminal pattern list is too narrow — misses real terminal errors | Conservative default (retry on unknown) preserves existing behavior. Pattern list is easily extensible. Log the failure details so missed patterns can be identified. |
| Terminal pattern list is too broad — catches retryable errors | Retryable patterns are checked first and explicitly excluded. Each terminal pattern targets specific, well-known error strings. |
| `output_summary` truncation (300 chars) loses error details | Terminal error messages from `gh` CLI are typically short (under 100 chars). The 300-char window is sufficient. |

## Sources & References

- Related issues: #516
- Prior plan: `docs/plans/2026-04-11-001-fix-exec-handler-github-identity-and-retry-plan.md` (Part 2 — partial fix: availability filter)
- Gate design: `docs/solutions/prompt-engineering/required-tools-enforcement-gate.md`
- Availability filter: `docs/solutions/prompt-engineering/required-tools-availability-filter.md`
- Guard pattern: `docs/solutions/architecture-patterns/completion-claim-guard-work-item-state-enforcement.md`

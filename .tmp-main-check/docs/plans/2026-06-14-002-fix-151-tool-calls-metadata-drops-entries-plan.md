---
title: "fix: Prevent tool_calls metadata from dropping entries on later agent loop steps"
type: fix
origin: mika issue#151
date: 2026-06-14
---

# fix: Prevent tool_calls metadata from dropping entries on later agent loop steps

## Summary

When the agent loop executes many tool calls across multiple steps (e.g., 9-10 steps), the `tool_calls` metadata stored on the assistant message drops entries from later steps. This makes those tool calls invisible in the dashboard's inline metadata view and the LLM history builder's `format_tool_summary_block()`. The authoritative `tool_calls` DB table is unaffected — the bug is confined to the `messages.metadata` JSON column.

## Problem Frame

Observed in production: session `5342cc77`, message ID 1066 — the agent executed 9+ tool calls across multiple steps (6× `run_shell`, `run_gh` ×2, `update_work_item_status`), but only 6 entries (steps 0-5) appeared in the message metadata. The audit_events table proves the missing tools executed. The stored metadata was 3324 bytes with full-length summaries (Phase 1 truncation was NOT applied), confirming only 6 entries existed in `all_tool_summaries` at serialization time.

Two failure modes contribute:

1. **EndTurn-with-tool_use silent drop (primary):** When the LLM returns `stop_reason: end_turn` with `tool_use` content blocks in the same response, the EndTurn match arm (`agent.rs:912`) calls `response.text()` — which filters out `ToolCall` variants — and proceeds through post-condition guards. The `tool_use` blocks are never dispatched via `process_tool_calls()`, so no `ToolCallSummary` entries are created, no `tool_calls` DB rows are persisted, and the conversation history lacks `tool_result` pairs. Some providers (notably OpenAI-compatible) can produce this shape; the Anthropic API can also do it when the model mixes text and tool calls at response boundaries.

2. **Phase 2 tail-drop without entry count logging (secondary):** When `tool_calls_metadata_json()` enters Phase 2 (last-resort tail-drop after Phase 1 field truncation fails to bring the JSON under the 4000-char cap), the warning logs `total_entries` but not how many entries were dropped or which ones were kept. For turns with 20+ tool calls (e.g., milestone workflows per #744), this is the documented limitation — but the warning should include the kept count for operational visibility.

---

## Requirements

- R1. All tool call summaries from a turn are stored in message metadata, including when the agent uses all 20 steps.
- R2. When tool_use blocks appear in an EndTurn response, process them before saving metadata.
- R3. `tool_calls_metadata_json()` logs a warning when Phase 2 drops entries, including how many were dropped vs. kept.
- R4. A test case reproduces and asserts correct behavior for a 10-step agent turn with mixed tool types (EndTurn-with-tool_use variant).
- R5. A test case reproduces and asserts correct behavior for the tail-drop scenario with diagnostic logging verification.

---

## Key Technical Decisions

### KTD-1: Process tool_use blocks in the EndTurn match arm

When the LLM response has `stop_reason: EndTurn` but also contains `tool_use` content blocks (detected via `response.has_tool_calls()`), the EndTurn arm should process those tool calls via `process_tool_calls()` before extracting text and entering the post-condition guard chain. This ensures:
- The tools execute (matching user intent implied by the LLM's output)
- `ToolCallSummary` entries are created and added to `all_tool_summaries`
- `tool_calls` DB rows are persisted
- `tool_result` blocks are added to the conversation history

**Rationale:** The EndTurn arm already has a precedent for detecting tool-related content in text form (`detect_text_based_tool_call()`, `detect_prose_style_tool_call()`). Processing structured `tool_use` blocks is more reliable than these heuristic detectors. The `extract_xml_tool_calls()` layer in `mika-common::llm::openai` already handles the text→ToolCall conversion for OpenAI-compatible providers, but some providers may emit both structured tool_use AND text in the same response with `stop_reason: end_turn`.

**Alternative considered:** Reclassifying the response as `ToolUse` when `has_tool_calls()` is true. Rejected — the EndTurn arm has 11 post-condition guards that must run on the text; changing the stop_reason would skip them. The correct fix is to process tool calls within the EndTurn arm, extending `all_tool_summaries`, then proceeding with the existing text-handling logic.

### KTD-2: Improve Phase 2 tail-drop diagnostic logging

Enhance the warning in `tool_calls_metadata_json()` Phase 2 to include `dropped_count` (total - kept) and `kept_count`. This is observability-only — the tail-drop behavior is the documented limitation per #744, where the dashboard fetches from the `tool_calls` table as the authoritative source.

### KTD-3: Increase TOOL_METADATA_MAX to accommodate typical 20-step turns

Evaluate whether raising `TOOL_METADATA_MAX` from 4000 to a higher value (e.g., 8000) would eliminate tail-drop for all but pathological cases. Per #744, the dashboard already uses the `tool_calls` table as the authoritative source, so the metadata cap primarily affects the LLM history builder's `format_tool_summary_block()`. A higher cap means the LLM gets more complete tool history context in subsequent turns.

**Decision: Defer.** The current 4000-char cap was chosen to keep message metadata small. Raising it has system prompt and context window cost implications. The fix for the EndTurn-with-tool_use silent drop (KTD-1) is the primary value — the metadata cap is a secondary concern that can be revisited independently.

---

## Scope Boundaries

### In Scope

- Fix the EndTurn-with-tool_use silent drop in `agent.rs`
- Improve Phase 2 tail-drop logging in `tool_calls_metadata_json()`
- Add eval tests reproducing both failure modes

### Deferred to Follow-Up Work

- Raising `TOOL_METADATA_MAX` beyond 4000 chars
- Adding a diagnostic query tool for metadata/audit_events count mismatches (AC item from the ticket — useful but orthogonal to the fix)
- Investigating whether any provider-specific adapter incorrectly maps stop reasons (e.g., a provider returning `end_turn` when it should return `tool_use`)

---

## Implementation Units

### U1. Handle tool_use blocks in the EndTurn match arm

**Goal:** When the LLM response has `stop_reason: EndTurn` but contains `tool_use` content blocks, process those blocks before extracting text and running post-condition guards.

**Requirements:** R1, R2

**Dependencies:** None

**Files:**
- `crates/mika-agent/src/agent.rs` (modify EndTurn match arm, ~line 912)

**Approach:**

At the top of the EndTurn match arm (line 912), before calling `response.text()`, check `response.has_tool_calls()`. When true:

1. Log a warning (`endturn_with_tool_use`) with `step`, `tool_count`, and `mode.label()` for operational visibility — this is an anomalous provider behavior.
2. Call `process_tool_calls()` with the response content, same arguments as the ToolUse arm.
3. Extend `all_tool_summaries` with the returned step summaries.
4. After processing, continue with the existing text extraction (`response.text()`) and post-condition guard chain as normal.

The key insight is that `response.text()` already filters to text-only blocks, so tool_use blocks don't interfere with text extraction. The missing piece is that they also need to be dispatched for execution before the text path runs.

Note: the `tools_called` HashSet (used for required_tools enforcement) should also be updated from the tool_use blocks in this path, mirroring what the ToolUse arm does at lines 2104-2107.

**Patterns to follow:** The ToolUse match arm at line 2101 shows the canonical pattern for processing tool calls. The `detect_text_based_tool_call()` handler at line 961 shows the precedent for detecting tool-related content in EndTurn responses.

**Test scenarios:**
- EndTurn response with both text and tool_use blocks: tools execute, summaries are in metadata, text is saved
- EndTurn response with only tool_use blocks (no text): tools execute, summaries are in metadata, empty text handled correctly
- EndTurn response with no tool_use blocks: existing behavior unchanged (regression guard)
- Multiple tool_use blocks in a single EndTurn response: all are processed and their summaries appear in metadata

---

### U2. Improve Phase 2 tail-drop logging in `tool_calls_metadata_json()`

**Goal:** Make the Phase 2 tail-drop warning actionable by including how many entries were kept vs. dropped.

**Requirements:** R3

**Dependencies:** None

**Files:**
- `crates/mika-agent/src/tool_execution/types.rs` (modify `tool_calls_metadata_json()`, ~line 82-95)

**Approach:**

In the Phase 2 loop (line 88-95), when an entry count `count` fits within the budget, the current code returns immediately. Enhance the existing `warn!` at line 83 to fire once the final `count` is determined (move it inside or after the loop), including `kept_count = count`, `dropped_count = summaries.len() - count`, and the existing `total_entries` and `max` fields.

Alternatively, keep the existing warning at Phase 2 entry (line 83) as-is for the "entering tail-drop" signal, and add a second info-level log after the loop determines the final count, reporting `kept` and `dropped`.

**Patterns to follow:** The existing `warn!` at line 83 with structured fields.

**Test scenarios:**
- Verify that when Phase 2 drops entries, the kept and dropped counts in the log are correct (unit test with tracing subscriber capture, or assert on the returned JSON entry count vs input count)
- Verify Phase 2 drops tail entries (not head) — the last entries in the input should be the ones dropped

---

### U3. Eval test for EndTurn-with-tool_use scenario

**Goal:** Add an eval harness test that reproduces the bug: an LLM response with `stop_reason: EndTurn` containing tool_use blocks, verifying that all tool calls are captured in metadata and the tool_calls DB table.

**Requirements:** R4

**Dependencies:** U1

**Files:**
- `crates/mika-agent/tests/eval/test_tool_calling.rs` (add new test)
- `crates/mika-common/src/llm/mock.rs` (may need a new helper for EndTurn-with-tool_use responses)

**Approach:**

Create a mock response sequence:
1. Step 0: normal ToolUse response with 2-3 `run_shell`-equivalent tool calls → summaries accumulated
2. Step 1: EndTurn response that contains BOTH text blocks AND tool_use blocks → these tool calls must also be processed

After the agent run, assert:
- `trace.tool_calls` contains entries from BOTH steps (the ToolUse step and the EndTurn-with-tool_use step)
- The saved message metadata contains ALL tool call summaries (not just the ones from the ToolUse step)
- The text from the EndTurn response is saved as the assistant message

A new mock helper `endturn_with_tools_response(text, tools)` may be needed in `mock.rs` — it would create an `LlmResponse` with `stop_reason: EndTurn` and a content vec containing both `Text` and `ToolCall` blocks.

**Patterns to follow:** `test_single_tool_call_then_response` and `test_multiple_parallel_tool_calls` in `test_tool_calling.rs` for harness usage. `tool_call_response` and `multi_tool_response` in `mock.rs` for response builders.

**Test scenarios:**
- 10-step turn with mixed tool types, final step is EndTurn-with-tool_use: all tool call summaries present in metadata
- Verify the message saved to DB has metadata with the complete tool call list
- Verify tool_calls table entries match the expected count

---

### U4. Unit test for improved Phase 2 tail-drop logging

**Goal:** Add a unit test verifying that the Phase 2 tail-drop preserves the correct entries and produces the expected count.

**Requirements:** R5

**Dependencies:** U2

**Files:**
- `crates/mika-agent/src/tool_execution/types.rs` (add test in `mod tests`)

**Approach:**

The existing `test_safety_net_drops_tail_on_overflow` test already verifies Phase 2 drops tail entries. Extend or add a companion test that:
- Creates a scenario where Phase 2 fires (20 tool calls with long names + max-length fields)
- Asserts that the returned JSON contains exactly the expected count of entries
- Asserts that the kept entries are the FIRST N entries (head preserved, tail dropped)
- Verifies the entry count math: `kept + dropped == total_input`

**Patterns to follow:** `test_safety_net_drops_tail_on_overflow` and `test_metadata_cap_drops_tail_on_milestone_workflow_turns` in `types.rs`.

**Test scenarios:**
- Phase 2 fires with 20 entries: assert kept entries are indices 0..N, dropped are N..20
- Phase 2 fires with 10 entries where long tool names push past the cap after Phase 1: verify entry preservation order

---

## Open Questions

- **Q1 (deferred to implementation):** Should the EndTurn-with-tool_use path set `tool_use_occurred = true`? This flag controls whether `attempt_continuation_turn` fires on max-steps-exceeded. Since EndTurn is terminal (the loop returns at line 1946), this flag has no effect in the EndTurn path — but setting it maintains consistency.

- **Q2 (deferred to implementation):** Should the EndTurn-with-tool_use path continue the loop (go to step N+1) instead of proceeding with text extraction? No — the LLM signaled EndTurn, which means it wants to stop. The tool calls should be executed as side effects, but the response text should be saved and the turn should end. The `continue` statement (which would go to the next loop iteration) is what the ToolUse arm does, and that's correct for ToolUse. EndTurn-with-tool_use should execute tools then terminate.

---

## Sources & Research

- mika issue#151 — original bug report with session evidence
- mika issue#115 — prior related bug (field-level truncation, fixed by Phase 1)
- mika issue#744 — documented tail-drop limitation, dashboard switched to `tool_calls` table as authoritative source
- `crates/mika-agent/src/agent.rs:912` — EndTurn match arm
- `crates/mika-agent/src/agent.rs:2101` — ToolUse match arm (canonical tool processing pattern)
- `crates/mika-agent/src/tool_execution/types.rs:53` — `tool_calls_metadata_json()` with Phase 1/Phase 2 truncation
- `crates/mika-agent/src/tool_execution/dispatch.rs:43` — `process_tool_calls()` tool execution and summary generation
- `crates/mika-common/src/llm/types.rs:148` — `LlmStopReason` enum
- `crates/mika-common/src/llm/types.rs:108` — `LlmResponse::text()` filters to text blocks only

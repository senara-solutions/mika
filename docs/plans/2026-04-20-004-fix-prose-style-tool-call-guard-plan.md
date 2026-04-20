---
title: "fix: Detect prose-style tool-call leaks in EndTurn output"
type: fix
status: active
date: 2026-04-20
issue: 569
---

# fix: Detect prose-style tool-call leaks in EndTurn output

## Overview

Add a new EndTurn post-condition guard `detect_prose_style_tool_call()` that catches when the LLM emits tool invocations as prose text — e.g., `check_work_item({"task_id": "..."})` — instead of using the structured tool-calling API. The guard gates matches against the current turn's registered tool names to eliminate false positives on code examples or explanatory prose.

## Problem Frame

The LLM occasionally emits tool calls as function-call-style prose text (`tool_name({...})`) instead of structured API calls. The existing five EndTurn guards do not detect this pattern:

- Layer 1 (XML extraction) and Layer 2 (`detect_text_based_tool_call`) require `<` characters — the prose pattern has none.
- Completion-claim and fabricated-action guards detect different failure classes entirely.

The result: the "call" renders as text in the TUI, never executes, and the agent proceeds as if the tool was invoked — an inconsistent state that can cascade into fabricated follow-up claims.

Observed 2026-04-14 in production: `check_work_item({"task_id": "48cbb025-..."})` appeared as rendered text with no corresponding entry in the `tool_calls` table.

## Requirements Trace

- R1. Detect `tool_name({"key": "value"})` patterns where `tool_name` matches a registered tool (builtins + skills + MCP)
- R2. Do NOT fire when the identifier is not a registered tool (code examples, explanatory prose)
- R3. Single retry on detection — re-prompt the LLM to use the structured API
- R4. Integrated into the EndTurn guard chain between Layer 2 and required-tools enforcement
- R5. Unit tests: positive match, negative (unknown identifier), multiline JSON, whitespace variations
- R6. Eval harness test: prose-style call → detection → re-prompt → structured tool call on retry

## Scope Boundaries

- Only the `name({...})` pattern — no backtick-wrapped calls, bare JSON invocations, or other formats
- No changes to existing guards — this is a new sibling in the chain
- No changes to the detection function signatures of existing guards

### Deferred to Separate Tasks

- Backtick-wrapped calls: `` `tool_name({...})` `` — different surface, separate issue
- Bare JSON invocations: `{"tool_name": "...", "arguments": {...}}` — separate issue

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/agent.rs` — EndTurn guard chain (lines 734–986), retry flag declarations (lines 584–596), `detect_text_based_tool_call()` (line 3223)
- `crates/mika-agent/src/tools/mod.rs` — `ToolRegistry` with `definitions()` returning `&[ToolDefinition]` and `get(name)` for lookup
- `filter_available_required_tools()` (agent.rs line 2901) — demonstrates checking all three tool sources: `tools.get()`, `skill_tool_map.contains_key()`, `mcp_manager.is_mcp_tool()`
- `crates/mika-agent/tests/eval/test_tool_calling.rs` — existing eval test `test_text_based_tool_call_retry()` (line 204) — direct pattern to follow

### Institutional Learnings

- `docs/solutions/runtime-errors/xml-tool-calls-not-executed.md` (#447) — predecessor guard; established fast-path substring check before deeper matching
- `docs/solutions/architecture-patterns/fabricated-action-claim-guard.md` (#308) — pattern template: `LazyLock<Regex>` + fast-path + `_retry_done` flag + push assistant + push correction + continue
- `docs/solutions/architecture-patterns/completion-claim-guard-work-item-state-enforcement.md` (#483) — false-positive avoidance strategies; scope keyword matching carefully
- `docs/solutions/architecture-patterns/per-turn-tool-use-dedup-guard.md` (#582) — lists full guard family; this becomes guard #6
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — justifies engine-level guard over prompt rule for this failure class

## Key Technical Decisions

- **Separate function, separate retry flag:** Use a new `detect_prose_style_tool_call()` function and `prose_tool_call_retry_done` flag rather than extending `detect_text_based_tool_call()`. Rationale: the detection logic is fundamentally different (regex + tool name set matching vs. XML substring checks), and a separate flag allows independent retry — a turn could theoretically contain both XML-style and prose-style leaks.

- **Two-phase detection (generic regex + tool name filter):** Use a static `LazyLock<Regex>` with the pattern `\b(\w+)\s*\(\s*\{` to extract candidate identifiers, then check each candidate against the registered tool name set. This avoids building a dynamic regex per turn (expensive) while keeping the static regex simple and fast.

- **Fast-path substring check:** Early-return `false` if the text contains no `({` substring (handles the vast majority of normal responses with zero regex cost). This follows the established pattern from `detect_text_based_tool_call()`.

- **Function signature returns `Option<String>`:** Returns the matched tool name (not just `bool`) so the guard's log message and re-prompt can name the specific tool. This follows the `detect_fabricated_action_claim()` pattern which returns `Option<(String, String)>`.

- **Tool name collection:** Build a `HashSet<String>` from `tools.definitions()` names + `skill_tool_map.keys()` + MCP tool names. Constructed at guard call site, not inside the detection function — keeps the function pure and testable.

## Open Questions

### Resolved During Planning

- **Should code blocks be excluded?** No — the issue explicitly scopes this to the `name({...})` pattern only. The tool-name gating already prevents most false positives. If code blocks containing real tool names become a problem, that's a refinement for a follow-up issue.

- **Where exactly in the guard chain?** Immediately after the `detect_text_based_tool_call()` guard block (after line 759), before the required-tools gate (line 770). This matches the issue's placement requirement and groups both "tool call leaked as text" guards together.

### Deferred to Implementation

- Exact line numbers will shift as code is inserted — use the guard chain's structural markers (EndTurn match + flag pattern) rather than pinning to line numbers.

## Implementation Units

- [x] **Unit 1: Detection function and unit tests**

**Goal:** Implement `detect_prose_style_tool_call()` with unit tests proving correct matching behavior.

**Requirements:** R1, R2, R5

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/agent.rs` (add function near existing guard functions ~line 3223, add unit tests in `#[cfg(test)] mod tests`)

**Approach:**
- Add `static PROSE_TOOL_CALL_RE: LazyLock<Regex>` with pattern `\b(\w+)\s*\(\s*\{`
- Implement `fn detect_prose_style_tool_call(text: &str, tool_names: &HashSet<String>) -> Option<String>` that:
  1. Fast-path: returns `None` if `!text.contains("({")` and no `( {` present (cheap substring scan matching `\(\s*\{`)
  2. Runs `PROSE_TOOL_CALL_RE.captures_iter(text)` to find all candidates
  3. For each capture group 1, checks if it exists in `tool_names`
  4. Returns `Some(tool_name)` on first match, `None` if no matches
- The fast-path needs to cover whitespace between `(` and `{` — check for both `({` and `( ` as substrings, or simply check for `(` (very common in normal text though). Better approach: check for `({` first (covers most cases), and if absent, only then fall through to the regex — the regex itself is the full check. Actually the simplest correct fast-path: `if !text.contains('(') { return None; }` — any prose-style tool call must contain a parenthesis.

**Patterns to follow:**
- `detect_text_based_tool_call()` at line 3223 — fast-path + detection pattern
- `detect_fabricated_action_claim()` — `LazyLock<Regex>` + `Option<T>` return pattern
- Existing unit tests at line 5051 — test naming and assertion style

**Test scenarios:**
- Happy path: `detect_prose_style_tool_call("check_work_item({\"task_id\": \"abc\"})", tools_with("check_work_item"))` → `Some("check_work_item")`
- Happy path: `detect_prose_style_tool_call("search_memory( {\"query\": \"test\"} )", tools_with("search_memory"))` → `Some("search_memory")` (whitespace between parens and brace)
- Happy path: multiline JSON — `"store_fact(\n{\"key\": \"val\",\n\"key2\": \"val2\"}\n)"` with `store_fact` registered → `Some("store_fact")`
- Edge case: unknown identifier — `"my_function({\"key\": \"val\"})"` with no `my_function` in tool set → `None`
- Edge case: empty text → `None`
- Edge case: text with parentheses but no tool pattern — `"I found some information (see details)."` → `None`
- Edge case: tool name mentioned without invocation syntax — `"Use check_work_item to verify the task status."` → `None`
- Edge case: tool name inside a code block with `({` but identifier is not a registered tool — `"Run my_func({\"x\": 1}) to test"` → `None`
- Edge case: multiple tool names in text, first one matches — returns first match
- Edge case: tool name with underscores and digits — `"tool_v2({\"a\": 1})"` with `tool_v2` registered → `Some("tool_v2")`

**Verification:**
- All unit tests pass with `cargo test -p mika-agent detect_prose_style`
- Function correctly returns `None` for all negative cases and `Some(name)` for positive cases

- [x] **Unit 2: Guard chain integration**

**Goal:** Wire `detect_prose_style_tool_call()` into the EndTurn guard chain with its own retry flag and re-prompt message.

**Requirements:** R3, R4

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/agent.rs` (add retry flag declaration, add guard block after text-based tool call guard, build tool name set at call site)

**Approach:**
- Add `let mut prose_tool_call_retry_done = false;` alongside existing retry flag declarations (~line 596)
- Before the guard block, collect tool names into a `HashSet<String>`:
  - `tools.definitions().iter().map(|d| d.name.clone())`
  - `skill_tool_map.keys().cloned()`
  - MCP tools if `mcp_manager` is available (check for an iterator/listing method, or skip MCP if no efficient listing API exists — the builtin + skill tools cover the vast majority of cases)
- Insert the guard block after the existing `text_tool_call_retry_done` guard (after ~line 759), before the required-tools gate:
  ```
  if EndTurn && !prose_tool_call_retry_done && let Some(tool_name) = detect_prose_style_tool_call(&text, &tool_names_set) {
      prose_tool_call_retry_done = true;
      warn!(...);
      push assistant response;
      push correction message naming the specific tool;
      continue;
  }
  ```
- The tool name set can be constructed once before the guard chain runs (it doesn't change within a turn), or lazily on first use. Constructing it before the chain is simpler and the cost is negligible.
- The correction message should name the detected tool: `"Your response contained a prose-style tool call for '{tool_name}' (e.g., tool_name({{...}})) instead of using the structured tool calling API. Do NOT output tool calls as text. Use the tool calling mechanism provided to you. Call {tool_name} now using the proper API."`

**Patterns to follow:**
- Text-based tool call guard block at line 734 — exact structural pattern
- Fabricated action guard at line 904 — `let Some(...)` pattern in condition for extracting matched data

**Test scenarios:**
- Test expectation: none — integration is verified by the eval harness test in Unit 3

**Verification:**
- `cargo build -p mika-agent` succeeds
- `cargo clippy -p mika-agent` passes with no new warnings
- Guard block follows the exact structural pattern of existing guards

- [x] **Unit 3: Eval harness test**

**Goal:** Add an eval harness integration test proving the full guard lifecycle: prose-style text → detection → re-prompt → structured tool call on retry.

**Requirements:** R6

**Dependencies:** Unit 2

**Files:**
- Modify: `crates/mika-agent/tests/eval/test_tool_calling.rs` (add new test function)

**Approach:**
- Follow the pattern of `test_text_based_tool_call_retry()` (line 204):
  1. Mock response sequence: (a) text response containing `search_memory({"query": "meetings"})`, (b) structured `tool_call_response("search_memory", ...)`, (c) text response with final answer
  2. Build harness with default tools (which include `search_memory`)
  3. Run with a user message
  4. Assert `assert_exact_steps(&trace, 3)` — text (retry), tool call, final response
  5. Assert `assert_tools_include(&trace, &["search_memory"])` — tool was actually called
  6. Assert `assert_has_output(&trace)` — final output exists
- Add a second test for negative case: prose-style pattern with an unknown tool name should NOT trigger the guard (1 step, no retry)

**Patterns to follow:**
- `test_text_based_tool_call_retry()` at line 204 — exact test structure
- `text_response()` and `tool_call_response()` helpers from the harness

**Test scenarios:**
- Integration: prose-style tool call with registered tool → guard fires → re-prompt → LLM uses structured API → 3 steps total
- Integration: prose-style pattern with unregistered tool name → guard does NOT fire → 1 step total (text passes through as normal response)

**Verification:**
- `cargo test -p mika-agent --test eval` passes
- New tests specifically pass: `cargo test -p mika-agent --test eval prose`

## System-Wide Impact

- **Interaction graph:** The new guard sits in the EndTurn chain between text-based tool call detection and required-tools enforcement. It consumes the same `response` and `text` variables, pushes messages onto `request.messages`, and `continue`s the loop — identical interaction surface to existing guards.
- **Error propagation:** On detection, the guard re-prompts (not errors). If the retry also fails, the response passes through — same as all other guards.
- **State lifecycle risks:** The `prose_tool_call_retry_done` flag prevents infinite retry loops. The flag resets each turn (declared inside `run_loop`). No persistent state changes.
- **API surface parity:** No API changes — this is internal agent loop behavior.
- **Integration coverage:** The eval harness test covers the full guard lifecycle including the re-prompt round-trip.
- **Unchanged invariants:** All existing guards continue to function identically. The new guard is additive — it only fires on patterns that previously passed through undetected.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| False positives on legitimate code examples containing tool names | Tool-name gating ensures only registered tools match; generic `word({...})` patterns are ignored. Code blocks with registered tool names are a theoretical risk but low-probability — the LLM rarely outputs `search_memory({...})` in code examples |
| Performance of regex on every EndTurn | Fast-path `contains('(')` check skips regex for most responses. The regex itself is simple and compiled once via `LazyLock` |
| Tool name set construction cost per turn | `HashSet` construction from ~20-50 tool definitions is negligible (microseconds) |

## Sources & References

- **Issue:** [#569](https://github.com/senara-solutions/mika/issues/569)
- Related: #447 (Layer 1 XML extraction + Layer 2 text-based tool call detector)
- Related: #483 (completion-claim guard)
- Related: #308 (fabricated action-claim guard)
- Related: #582 (per-turn tool use dedup guard)
- Related: #648 (persistence evaluation guard)
- Learnings: `docs/solutions/runtime-errors/xml-tool-calls-not-executed.md`
- Learnings: `docs/solutions/architecture-patterns/fabricated-action-claim-guard.md`
- Learnings: `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`

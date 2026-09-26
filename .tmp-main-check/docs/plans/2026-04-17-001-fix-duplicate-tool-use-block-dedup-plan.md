---
title: "fix: Deduplicate identical tool_use blocks within a single agent turn"
type: fix
status: active
date: 2026-04-17
---

# fix: Deduplicate identical tool_use blocks within a single agent turn

## Overview

Guard the agent loop against LLM responses that contain two or more `tool_use` content blocks with an identical `(tool_name, arguments)` pair inside the same turn. Execute the tool once, cache the `ToolOutput`, reuse it for every duplicate block, and persist only one `tool_calls` row / one `ToolCallSummary` per unique call.

## Problem Frame

Issue #582: trace `d71cb2c2c4c54acb8b63ea15115c9900` recorded two `tool_calls` rows for `send_message` at the same second with byte-identical `input`. Research confirms the agent loop processes each `LlmResponseContent::ToolCall` block in order with no dedup (`crates/mika-agent/src/agent.rs:1562`). A duplicate block therefore runs the tool twice, saves two DB rows, and — once delivery is restored — will produce two outbound notifications. Duplicate emission is most likely an LLM-side artifact (the assistant message contains two `tool_use` entries); the dispatch path runs each block exactly once. The engine must defend itself regardless of which provider caused it.

Grounding: the duplicate occurred in a kimi-k2.5 session. `mika-common` supports 11 providers, and recovery paths like `extract_xml_tool_calls` already cope with provider quirks — a per-turn dedup guard fits that pattern.

## Requirements Trace

- R1. Identical tool_use blocks within a single agent turn execute the underlying tool at most once.
- R2. Only one row is written to `tool_calls` per unique `(tool_name, arguments)` pair per turn, regardless of how many duplicate blocks the LLM emitted.
- R3. The assistant conversation history remains internally consistent: every tool_use id in the assistant message has a matching `tool_result` block in the same turn (API contract for all providers).
- R4. Duplicate suppression is logged with `trace_id`, `tool_name`, and `step` for observability.
- R5. Regression test feeds a response containing two identical tool_use blocks and asserts only one underlying call runs.

## Scope Boundaries

- No changes to LLM providers, prompt assembly, or retry policy.
- No new config flag — dedup is always on (false positives are negligible; LLMs do not intentionally emit byte-identical tool_use blocks in a single turn).
- No cross-turn dedup (each turn starts with a fresh state).
- No cross-step dedup (step boundary = separate LLM response, legitimate to repeat a tool).
- No changes to `send_message` tool behavior itself — the fix is at the loop layer.
- No schema migration (dedup happens before `save_tool_call`; we simply skip the extra row).

### Deferred to Separate Tasks

- Companion silent-notification issue (chat_id=0) is tracked separately; this plan only addresses duplicate emission.
- Any broader tool-use validation (e.g., schema conformance, hallucinated tool names) stays out of scope.

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/agent.rs` — `process_tool_calls()` (line 1544) is the single dispatch point for every tool_use block; both the `execute_tool` invocation and the `db.save_tool_call` call live inside one `for block in &response_content` loop. Adding the guard here fixes both the execution and the storage paths in one place.
- `crates/mika-agent/src/agent.rs:967` — `LlmStopReason::ToolUse` branch already iterates `response.content` once for `tools_called` tracking, providing the template for iterating tool_use blocks deterministically.
- `crates/mika-agent/src/tools/mod.rs` — `ToolOutput { content, is_error, images }` and `ImageData` (already `#[derive(Debug, Clone)]`). `ToolOutput` is currently `#[derive(Debug)]` only; adding `Clone` is mechanical and unblocks reusing cached results for duplicate blocks.
- `crates/mika-common/src/llm/types.rs:237` — `response_content_to_blocks()` turns the LLM response into assistant history; duplicate tool_use blocks in the assistant message are preserved as-is so the API history stays coherent when tool_results are emitted for both ids.
- `crates/mika-agent/tests/eval/` — `EvalHarness`, `MockLlmProvider`, `multi_tool_response()` helper (`crates/mika-common/src/llm/mock.rs:237`) already support seeding a single response with multiple tool_use blocks. A test that passes two tuples with the same `(name, args)` is a one-line extension.
- `crates/mika-agent/tests/eval/trace.rs:56` — `AgentTrace::calls_for_tool(name)` returns `tool_calls` rows by name; the regression test asserts `.len() == 1` after seeding two identical blocks.

### Institutional Learnings

- Guard-style defenses against provider artifacts are an established pattern in this codebase: `detect_text_based_tool_call`, `extract_xml_tool_calls`, required-tools retry, completion-claim guard, fabricated-action claim guard, phantom-retry guard. Adding a duplicate-tool_use guard fits that family — cheap, per-turn, observable via `warn!` logs.
- Agent loop guards are scoped to a single turn and never persist state across calls (no cross-turn memory of what has run).

### External References

- None — this is an internal invariant, not dependent on provider-specific API shape.

## Key Technical Decisions

- **Dedup key: `(String, String)` where the second element is `serde_json::to_string(arguments)`.** The issue reports byte-identical `input`, so string equality is sufficient. Canonical-json normalization is not needed today; if future providers reorder keys between duplicates, the guard will fail open (no dedup) rather than false-positive (important invariant). Revisit if telemetry shows near-miss duplicates.
- **Dedup is universal, not send_message-only.** An identical tool_use pair within a single turn is never a legitimate intent for any tool (the LLM can always vary arguments if it means distinct calls). Universal scope is simpler and avoids per-tool allow/deny lists.
- **Cache the `ToolOutput` and reuse it for duplicate blocks.** The LLM's assistant message keeps both tool_use ids, so the API history requires both to receive a matching `tool_result`. Reusing the real output (clone) keeps the LLM's view consistent with the first execution and avoids leaking internal engine details (e.g., "duplicate suppressed") into the next turn's context.
- **`ToolOutput` gains `#[derive(Clone)]`.** Fields (`String`, `bool`, `Vec<ImageData>`) are all `Clone`; image byte buffers are already `String`-backed base64. Clone cost is negligible at image sizes already capped by `MAX_IMAGE_BYTES_PER_STEP`.
- **Save `tool_calls` row and emit `ToolCallSummary` exactly once per unique call.** The summary is what surfaces in `messages.metadata` and in history builder output; duplicating it would leak the bug into downstream context. Logging a `warn!` on suppression is the observability channel for operators.
- **Preserve required-tools tracking as-is.** The pre-dispatch loop at line 970 that populates `tools_called` can keep iterating every block (recording `send_message` once is the same whether dedup catches it at dispatch or not — `HashSet::insert` already idempotent). No change needed there.

## Open Questions

### Resolved During Planning

- Q: Should the duplicate's tool_result echo the original output, or a "suppressed" stub? — A: Echo the original. Keeps the assistant/tool history consistent and avoids surprising the LLM with differentiated results for what it emitted as two identical calls.
- Q: Should `ToolCallSummary` include both entries? — A: No. Summaries feed back into `tool_history` context; a second identical summary would re-introduce the bug into the next turn.
- Q: Why not dedup at DB layer via a unique index on `(trace_id, tool_name, input)`? — A: DB dedup cannot prevent the double execution of side-effectful tools (`send_message` has already dispatched before the INSERT). Loop-level dedup fixes execution, storage, and summary together.

### Deferred to Implementation

- None. Implementation is mechanical: add a `HashMap<(String, String), ToolOutput>` inside `process_tool_calls`, one `warn!` log line, one `#[derive(Clone)]` addition, and one eval test.

## Implementation Units

- [ ] **Unit 1: Add per-turn dedup in `process_tool_calls`**

**Goal:** Detect identical `(tool_name, arguments)` pairs within one turn and execute each unique pair at most once.

**Requirements:** R1, R2, R3, R4

**Dependencies:** None.

**Files:**
- Modify: `crates/mika-agent/src/agent.rs`
- Modify: `crates/mika-agent/src/tools/mod.rs` (add `#[derive(Clone)]` to `ToolOutput`)

**Approach:**
- Before the `for block in &response_content` loop, introduce `let mut executed: HashMap<(String, String), ToolOutput> = HashMap::new();` scoped to the function call.
- Inside the `ToolCall` branch, compute `let dedup_key = (name.clone(), serde_json::to_string(arguments).unwrap_or_default());` and `executed.get(&dedup_key)`:
  - On hit: emit `warn!(trace_id = %tool_ctx.trace_id, tool = %name, step = step, "duplicate tool_use block suppressed; reusing prior result");`, skip `execute_tool`, skip `save_tool_call`, do **not** push to `summaries`, but still push a `LlmContentBlock::ToolResult` for the duplicate `id` using a `Clone` of the cached `ToolOutput` so the assistant/tool history stays paired.
  - On miss: run the existing path (execute, record DB row, push summary) and `executed.insert(dedup_key, output.clone())` before building the tool_result block. The clone is cheap — `ToolOutput` fields are `String`, `bool`, `Vec<ImageData>` and images are already base64 text.
- Keep the existing tool_result construction (text vs. multi-block image handling) factored out so both the miss and hit paths produce identical shapes. Preferred shape: build the `LlmContentBlock::ToolResult` from a shared helper that takes `(tool_call_id: String, output: &ToolOutput, image_bytes_budget: &mut usize)` — this lets the duplicate reuse the helper with the cached output. Directional only; implementer may inline the branches if the helper adds no readability.
- `#[derive(Clone)]` on `ToolOutput` is a one-line change; `ImageData` already derives `Clone`.

**Technical design:** *(directional, not implementation spec)*

    for block in &response_content {
        if let LlmResponseContent::ToolCall { id, name, arguments } = block {
            let key = (name.clone(), serde_json::to_string(arguments).unwrap_or_default());
            let output = match executed.get(&key) {
                Some(cached) => {
                    warn!(trace_id = %..., tool = %name, step, "duplicate tool_use ...");
                    cached.clone()
                }
                None => {
                    let o = execute_tool(...).await;
                    if store_tool_calls { db.save_tool_call(...).await; }
                    summaries.push(ToolCallSummary { ... });
                    executed.insert(key.clone(), o.clone());  // or insert a Clone
                    o
                }
            };
            tool_results.push(build_tool_result_block(id.clone(), &output, &mut image_bytes_budget));
        }
    }

**Patterns to follow:**
- Same guard style as `detect_text_based_tool_call`, `detect_completion_claim`, `detect_fabricated_action_claim` — a cheap per-turn check that logs `warn!` with `trace_id` when it fires (`crates/mika-agent/src/agent.rs`).
- Same `HashMap`/`HashSet` scoping pattern already used at line 970 for `tools_called`.

**Test scenarios:**
- Integration (eval harness): LLM response containing `multi_tool_response(vec![("send_message", json!({"text": "hi"})), ("send_message", json!({"text": "hi"}))])` followed by a text_response → assert `trace.calls_for_tool("send_message").len() == 1`, assert final output present, assert `tools_called` summary shows `send_message` exactly once. (Covered by Unit 2.)
- Integration (eval harness): LLM response with two `search_memory` blocks that have *different* `query` args → still produces two DB rows (no regression of `test_multiple_parallel_tool_calls`).
- Unit (`#[cfg(test)]` inline in `agent.rs` or `tools/mod.rs`): `ToolOutput::clone()` round-trips `content`, `is_error`, and `images` byte-for-byte. Small sanity test for the new derive.

**Verification:**
- `cargo clippy -p mika-agent` is clean.
- `cargo test -p mika-agent --test eval test_tool_calling` passes, including the new test and the unchanged parallel-tool test.
- A grep of `warn!` new call site finds the `"duplicate tool_use block suppressed"` message with `trace_id`, `tool`, and `step` fields.

- [ ] **Unit 2: Regression test for duplicate tool_use dedup**

**Goal:** Lock in the invariant so future refactors of `process_tool_calls` cannot regress it.

**Requirements:** R5

**Dependencies:** Unit 1.

**Files:**
- Modify: `crates/mika-agent/tests/eval/test_tool_calling.rs`

**Approach:**
- Add `test_duplicate_tool_use_block_deduplicated` after `test_multiple_parallel_tool_calls`.
- Feed the mock with a single `multi_tool_response` containing two tuples with identical `(name, args)` (use `send_message` as the canonical example to match the issue), followed by a `text_response`.
- Assert `trace.calls_for_tool("send_message").len() == 1` (one DB row) and that the final agent output is present.
- Optional secondary assertion: assert the `tool_call_summaries` count via whatever helper the harness exposes (`assert_exact_steps` or equivalent). Keep the assertion scoped and readable; do not over-assert implementation details.

**Patterns to follow:**
- `test_multiple_parallel_tool_calls` (same file, lines ~29–53) is the near-identical structural template.

**Test scenarios:**
- Happy path: two identical `send_message` blocks in one turn → exactly one `tool_calls` row. (This is the main assertion.)
- Sanity: final text response still reaches the user.

**Verification:**
- `cargo test -p mika-agent --test eval test_duplicate_tool_use_block_deduplicated` passes.
- Running with Unit 1 reverted makes the test fail with `.len() == 2` — proves it guards the invariant.

## System-Wide Impact

- **Interaction graph:** Only `process_tool_calls` is touched; callers (`run_loop`) are unaffected. `tools_called` tracking at line 970 keeps working because `HashSet::insert` ignores duplicates.
- **Error propagation:** A duplicate that inherits a cached error output still produces an error `tool_result` for the duplicate id; the loop's error handling is unchanged.
- **State lifecycle risks:** Zero — dedup state is stack-local to `process_tool_calls` and dies with the function. No DB state, no agent-level memory.
- **API surface parity:** None. No tool schema changes, no config keys, no dashboard DTO changes.
- **Integration coverage:** The eval test covers the full `run_agent → run_loop → process_tool_calls → save_tool_call` path under `MockLlmProvider`, which is the integration level at which the bug manifests.
- **Unchanged invariants:** `tool_calls` schema, `save_tool_call` signature, `ToolOutput` field set, `LlmResponseContent` shape, `response_content_to_blocks` behavior, conversation history format, required-tools enforcement. The fix is purely additive.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| False positive dedup for a legitimate double-call with byte-identical args | Vanishingly rare; duplicate intent can be expressed with different args. If it occurs, `warn!` log surfaces it with `trace_id` for triage. |
| Canonical-JSON reordering by a future provider produces byte-different strings for the same logical args | String comparison fails open (no dedup) — same behavior as today. Can revisit with canonicalization if telemetry shows near-misses. |
| `ToolOutput::Clone` accidentally doubles memory for large outputs | Outputs are already truncated at the DB save path (50 KB content cap) and images cap at `MAX_IMAGE_BYTES_PER_STEP`; clone overhead is bounded. |
| Cached result hides a tool flakiness that would retry on second call | Today's code does not retry duplicate-identical calls either — the second execution was never a retry mechanism. No behavior regression. |

## Documentation / Operational Notes

- `crates/mika-agent/CLAUDE.md` "Agent Loop" section already enumerates the four post-condition guards. Extend the guard list (or add a short bullet under "Post-Conditions" / nearby) to mention the duplicate-tool_use dedup guard so future contributors have a pointer. Keep it to one sentence.
- No operational rollout concerns — the change is a pure in-process guard, no migrations, no feature flag. Logs will show `duplicate tool_use block suppressed` when the guard fires; grep-able for dashboards or alerting if needed.

## Sources & References

- **Issue:** senara-solutions/mika#582
- Related code: `crates/mika-agent/src/agent.rs` (`process_tool_calls`, `run_loop::LlmStopReason::ToolUse` branch), `crates/mika-agent/src/tools/mod.rs` (`ToolOutput`), `crates/mika-common/src/llm/types.rs` (`LlmResponseContent`, `response_content_to_blocks`), `crates/mika-common/src/llm/mock.rs` (`multi_tool_response`), `crates/mika-agent/tests/eval/test_tool_calling.rs`, `crates/mika-agent/tests/eval/trace.rs` (`calls_for_tool`).
- Related prior guards for pattern precedent: required-tools retry, completion-claim guard, fabricated-action claim guard, phantom-retry guard.

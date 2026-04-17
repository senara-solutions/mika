---
title: "Provider emits duplicate tool_use blocks in a single agent turn — engine dispatches each one"
category: logic-errors
date: 2026-04-17
module: mika-agent
problem_type: logic_error
component: assistant
symptoms:
  - "Two `tool_calls` rows with same trace_id, tool_name, and byte-identical input at the same second"
  - "send_message executed twice per turn — duplicate outbound delivery once notifications are restored"
  - "`messages.metadata` shows the same ToolCallSummary twice, leaking duplication into next-turn context"
root_cause: logic_error
resolution_type: code_fix
severity: medium
related_components:
  - assistant
  - tooling
tags: [dedup, tool-use, provider-artifacts, agent-loop, kimi, non-anthropic]
related_issues: ["#582"]
---

# Provider emits duplicate tool_use blocks in a single agent turn — engine dispatches each one

## Problem

A single agent turn (trace `d71cb2c2c4c54acb8b63ea15115c9900`, kimi-k2.5) produced two `tool_calls` rows for `send_message` with byte-identical `input` at the same second. The engine's dispatch loop had no guard over duplicate `tool_use` content blocks inside a single LLM response, so it executed the tool twice, saved two DB rows, emitted two `ToolCallSummary` entries, and — once the companion silent-notification bug (chat_id=0) is fixed — would deliver two outbound messages per sprint start / retry / close-out.

The duplication is a provider-side artifact (the assistant message carried two tool_use blocks with the same `(name, arguments)`). mika supports 11 LLM providers; the engine must defend against this regardless of which one misbehaves.

## Symptoms

- Two `tool_calls` rows with same `trace_id`, `tool_name=send_message`, `tool_source=builtin`, identical `input`, both `success=1`, both `created_at = 2026-04-15T16:18:44Z`.
- `summaries.push(ToolCallSummary { ... })` fired twice, so `messages.metadata` carries the same summary twice and re-injects the duplicate into the next turn's `<context type="tool_history">` block.
- Agent's conversation history builder round-trips both tool_use blocks and both tool_results back to the provider.

## What Didn't Work

- **Prompt instruction.** Telling the model "do not emit the same tool call twice" is unreliable — the failure is a provider-side artifact, not a reasoning mistake. mika has hit this class of bug repeatedly with non-Anthropic models (see Related).
- **DB-level uniqueness on `(trace_id, tool_name, input)`.** The UNIQUE constraint would catch the second INSERT but by then `send_message` has already dispatched. DB dedup cannot prevent double execution of side-effectful tools.
- **Per-tool opt-in dedup (send_message only).** Considered and rejected. Any tool with identical `(name, arguments)` in the same turn is never a legitimate distinct intent — the LLM can always vary arguments if it means two calls. Universal dispatch-loop dedup is simpler and avoids per-tool allowlists.

## Solution

Per-turn dedup guard inside `process_tool_calls()` in `crates/mika-agent/src/agent.rs`. Executes the underlying tool once per unique `(tool_name, arguments)` pair, caches the `ToolOutput`, and reuses it for duplicate `tool_use` ids so the provider's API contract — every tool_use id must have a matching tool_result — still holds.

```rust
// Scope: function-local, one invocation per LLM response
let mut dedup_cache: HashMap<(String, String), ToolOutput> = HashMap::new();

for block in &response_content {
    if let LlmResponseContent::ToolCall { id, name, arguments } = block {
        let dedup_key = (
            name.clone(),
            serde_json::to_string(arguments).unwrap_or_default(),
        );
        let output = if let Some(cached) = dedup_cache.get(&dedup_key) {
            warn!(
                trace_id = %tool_ctx.trace_id,
                tool = %name,
                step,
                cached_was_error = cached.is_error,
                "duplicate tool_use block suppressed; reusing prior result"
            );
            // Clear images on reuse — the LLM already received them in the
            // first duplicate's tool_result. Re-emitting wastes the shared
            // `image_bytes_budget` and inflates the API request body.
            let mut reused = cached.clone();
            reused.images.clear();
            reused
        } else {
            // ...execute_tool, save_tool_call, summaries.push...
            dedup_cache.insert(dedup_key, output.clone());
            output
        };

        tool_results.push(LlmContentBlock::ToolResult { tool_call_id: id.clone(), ... });
    }
}
```

Supporting changes:
- `ToolOutput` gains `#[derive(Clone)]` (fields were already clonable: `String`, `bool`, `Vec<ImageData>`).
- Three regression tests in `crates/mika-agent/tests/eval/test_tool_calling.rs`:
  - `test_duplicate_tool_use_block_deduplicated` — two identical `send_message` blocks → one DB row, two tool_result blocks in the follow-up request.
  - `test_same_tool_different_args_not_deduplicated` — two `send_message` blocks with different text → two DB rows (pins the dedup key to `(name, arguments)`, not name alone).
  - `test_three_identical_tool_use_blocks_deduplicated` — exercises the cache-hit path twice.

## Why This Works

- **Dispatch-layer guard is the only layer that fixes execution, storage, and summary together.** Tools with side effects (`send_message` → gateway delivery, `store_fact` → memory write, `create_work_item` → DB row) have fired their effect before any DB constraint can trip. Guarding at the dispatch loop prevents the second execution entirely.
- **Per-turn scope matches the actual fault.** The provider emits duplicates inside one response; across turns or steps, repeating a tool with identical arguments is legitimate (a recurring reminder, a polling loop). The `HashMap` lives one `process_tool_calls` call and dies with it.
- **Clearing `images` on the reuse path preserves the per-step image budget.** The first duplicate's tool_result already delivered the images to the LLM; re-sending them would waste `MAX_IMAGE_BYTES_PER_STEP` and, worse, could starve a subsequent legitimate tool_result in the same turn of its image slot.
- **Fail-open JSON comparison is deliberate.** `serde_json::to_string(arguments)` is byte-equal, not canonical. If a future provider reorders object keys between otherwise-identical duplicates, the guard misses the match and the tool executes twice — same behavior as today's pre-fix code. This is safer than canonicalizing and risking a false-positive dedup that silently drops a legitimate distinct call. Revisit with canonicalization only if telemetry on `"duplicate tool_use block suppressed"` shows near-misses on OpenAI-compatible routes.

## Prevention

- **Guard pattern for provider-side artifacts.** Non-Anthropic models regularly emit structural artifacts Anthropic does not — prior cases include text-wrapped tool calls, malformed closing tags, XML-format tool calls, fabricated URL claims, phantom retries after callback, and hallucinated `canUseTool` denials. The established mika response is a cheap loop-level guard that logs `warn!` with `trace_id`, not prompt engineering. New providers should be assumed to produce their own artifacts until proven otherwise; add guards defensively.
- **Guard-reuse invariant.** When you reuse a cached result for a duplicate, clear fields whose bytes the LLM already received. Shared per-step budgets (image bytes, token limits, API request body size) otherwise silently double-count.
- **Post-condition for new write/dispatch tools.** If a tool has side effects, DB uniqueness is necessary but not sufficient. Also guard at dispatch so the side effect does not fire twice when the LLM emits identical calls.
- **Observability hook.** The `warn!` log field `duplicate tool_use block suppressed` is grep-able. Consider adding a counter metric or audit event if telemetry wants a rate trend for provider-health dashboards.

## Related

- [agent-creates-duplicates-after-compaction.md](agent-creates-duplicates-after-compaction.md) — canonical three-layer defense (prompt soft layer → DB hard constraint → tool-level graceful catch). This fix adds the loop-level layer above all three, for cases where the duplicate originates at the LLM dispatch boundary, not at the tool or DB.
- [create-work-item-duplicate-on-retry.md](create-work-item-duplicate-on-retry.md) — DB-level vs tool-level dedup trade-offs. Parallel insight: DB dedup alone is insufficient when the duplication source is upstream of storage.
- [../ui-bugs/malformed-closing-tags-non-anthropic-models.md](../ui-bugs/malformed-closing-tags-non-anthropic-models.md) — prior non-Anthropic provider structural artifact, fixed with a loop-level normalizer. Same pattern as this dedup guard.
- [../architecture-patterns/fabricated-action-claim-guard.md](../architecture-patterns/fabricated-action-claim-guard.md) — precedent for post-condition loop guards that catch provider-emitted anomalies before committing the turn.
- **Sibling guards in `process_tool_calls` / EndTurn chain:** `detect_text_based_tool_call`, `extract_xml_tool_calls`, required-tools gate, `detect_completion_claim`, `detect_fabricated_action_claim`, phantom-retry guard. This dedup joins that family as a dispatch-time (not EndTurn) check.
- **Issue:** [senara-solutions/mika#582](https://github.com/senara-solutions/repo/issues/582)

---
title: "Per-Turn tool_use Dedup Guard"
issue: "#582"
date: "2026-04-17"
tags: [agent-core, tool-dispatch, guard, defense-in-depth, provider-quirk]
category: architecture-patterns
problem_type: logic-error
module: mika-agent
component: process_tool_calls
---

# Per-Turn tool_use Dedup Guard

## Problem

A single mika-dev turn emitted two `send_message` tool_calls with byte-identical `input` in the same second (trace `d71cb2c2c4c54acb8b63ea15115c9900`, kimi-k2.5). The LLM's response contained two `tool_use` content blocks with the same `(name, arguments)`, and `process_tool_calls()` iterated them blindly — running the tool twice, saving two `tool_calls` rows, and (once delivery is restored) about to double-send every sprint-start notification. Acceptance from #582: "Identical tool_call from same turn is either deduped at emission time or executed once and cached."

## Symptoms

- Two rows in `tool_calls` with the same `trace_id`, same `tool_name`, identical `input` JSON, same `created_at` second, both `success=1`.
- Observed with `kimi-k2.5` (via `OpenAiCompatibleProvider`); Anthropic provider never emitted this pattern in production traces.
- Downstream risk: duplicate outbound notifications, duplicate `messages` rows for `send_message`, duplicate side effects for any tool the LLM redundantly emitted in one response.
- Sibling `kimi-k2.5` behavior already documented in `best-practices/list-tool-status-summary-reduces-redundant-calls.md` (redundant *cross-turn* filtered calls). This incident is the *within-turn* variant of the same provider quirk family.

## What Didn't Work

- Prompt-only defense. The system prompt already has a grounding rule ("don't claim downstream state unless a tool result confirms it") and a confirmation-before-action rule. Neither prevents the provider from serializing two identical `tool_use` blocks into a single response — the harness is the only enforcement point.
- DB-layer dedup. A unique index on `(trace_id, tool_name, input)` in the `tool_calls` table would suppress the second row, but the tool would already have executed (side effects delivered twice before the INSERT). Dedup must happen at execution dispatch time, not at persistence time.
- Single-step scope only. Scoping dedup to a single `process_tool_calls()` call is correct; extending to cross-step or cross-turn would break legitimate retry flows (the agent can legitimately call the same tool with the same args across two different steps).

## Solution

Added a fifth entry to the agent-loop guard family (see Prevention for the family list): a per-turn dedup cache inside `process_tool_calls()` keyed on `(tool_name, serde_json::to_string(arguments))`. The first block executes the tool and persists one `tool_calls` row. Subsequent blocks with the same key reuse a clone of the cached `ToolOutput`. Each duplicate `tool_use_id` still gets its own `tool_result` in the conversation history so the API contract stays paired.

```rust
// crates/mika-agent/src/agent.rs — process_tool_calls()
let mut dedup_cache: HashMap<(String, String), ToolOutput> = HashMap::new();
for block in &response_content {
    if let LlmResponseContent::ToolCall { id, name, arguments } = block {
        let dedup_key = (name.clone(), serde_json::to_string(arguments).unwrap_or_default());
        let output = if let Some(cached) = dedup_cache.get(&dedup_key) {
            warn!(
                trace_id = %tool_ctx.trace_id,
                tool = %name,
                step,
                cached_was_error = cached.is_error,
                "duplicate tool_use block suppressed; reusing prior result"
            );
            // Strip images on reuse — the LLM already received them on the
            // first duplicate's tool_result; re-emitting would double-charge
            // the shared `image_bytes_budget` and inflate the API payload.
            let mut reused = cached.clone();
            reused.images.clear();
            reused
        } else {
            let output = execute_tool(&dispatch, name, arguments.clone()).await;
            // … save_tool_call, push ToolCallSummary …
            dedup_cache.insert(dedup_key, output.clone());
            output
        };
        // … build LlmContentBlock::ToolResult with this output …
    }
}
```

Also required: `#[derive(Clone)]` on `ToolOutput` (fields are `String`, `bool`, `Vec<ImageData>`; `ImageData` was already `Clone`).

## Why This Works

- **Correct enforcement point.** The guard lives one level above `execute_tool` but below the loop's iteration. Nothing can route around it — every tool_use block from any provider funnels through `process_tool_calls`.
- **Scope matches intent.** The `HashMap` is function-local and dropped at function return. Cross-step and cross-turn duplicates are unaffected (the agent can legitimately retry an identical call across different steps).
- **API contract preserved.** Each `tool_use_id` still receives a matching `tool_result` — the LLM's conversation history stays internally consistent, and Anthropic won't reject the follow-up request for unpaired ids.
- **Observability built in.** `warn!` carries `trace_id`, `tool`, `step`, and `cached_was_error` — enough to grep production logs, correlate with a specific turn, and distinguish suppressed successes from suppressed failures.
- **Image budget stays honest.** Stripping `images` on the cache-hit path prevents the shared `image_bytes_budget` from being double-charged for bytes the LLM already received on the first duplicate's tool_result. Latent today for `send_message` (no images) but correct for any image-returning exec-handler skill.

## Prevention

Join the existing agent-loop guard family when defending against provider quirks — code enforcement beats prompt instructions when LLM nonconformance can cause real harm:

1. **Text-based tool call detection** — catches XML tool calls emitted as text (`detect_text_based_tool_call`).
2. **Required-tools gate** — keyword-matched skills declaring `[constraints] required_tools` block EndTurn without the tool call (#265).
3. **Completion-claim guard** — blocks EndTurn text claiming completion keywords without `update_task_status` (#483).
4. **Fabricated action-claim guard** — blocks EndTurn claiming a GitHub resource URL with zero tool calls (#308).
5. **Per-turn tool_use dedup guard (this doc)** — dispatch-layer, suppresses byte-identical tool_use blocks within one response (#582).

Regression tests (see `crates/mika-agent/tests/eval/test_tool_calling.rs`):

- `test_duplicate_tool_use_block_deduplicated` — two identical blocks → one `tool_calls` row, two paired `tool_result` blocks in the follow-up LLM request.
- `test_three_identical_tool_use_blocks_deduplicated` — N ≥ 2 still collapses to one execution; exercises the cache-hit path multiple times.
- `test_same_tool_different_args_not_deduplicated` — guards against a regression that keys dedup on name alone; two `send_message` calls with different `text` must still produce two rows.
- `test_multiple_parallel_tool_calls` — pre-existing; asserts genuinely distinct parallel tools still execute normally.

Known limitations (intentionally in scope for later work, not for this PR):

- **Key-order sensitivity.** `serde_json::to_string` on `Value::Object` preserves insertion order. A provider that emits logically-identical arguments with different key orderings (or int-vs-float number representations) between two blocks will miss the cache. Document if observed; revisit with canonical serialization if telemetry shows near-misses.
- **No `tool_calls` row for suppressed duplicates.** The dashboard API counts DB rows, not LLM conversation blocks. Observers correlating raw LLM history with dashboard will see fewer rows than `tool_use` blocks. The `warn!` log is the primary audit trail; add an `audit_events` entry if dashboard visibility becomes important.
- **Error caching.** If the first block errors, the duplicate inherits the same error via clone. This matches the intended semantics (duplicate inputs, no basis for a different result) but is worth noting.

## When to Add a Similar Guard

Any time a provider quirk can cause a side-effectful tool to fire twice from a single LLM turn. The pattern:

1. Identify the dispatch choke point (`process_tool_calls`, `run_gh`, webhook handler — wherever execution actually happens).
2. Scope the defense to the smallest unit that preserves intent (per-turn here; per-request elsewhere).
3. Preserve the API contract downstream (emit tool_results / response pairs even when suppressing).
4. Log `warn!` with `trace_id` so operators can grep for it.
5. Write a regression test in the eval harness (`multi_tool_response` with duplicates) plus a counter-test that guards against the dedup widening (same tool, different args).

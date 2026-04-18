---
title: "Persistence evaluation guard: turn-end knowledge persistence nudge"
category: architecture-patterns
date: 2026-04-18
tags: [agent-loop, persistence, guard, post-condition, store_fact, memory, endturn, re-prompt]
issue: 648
---

# Persistence evaluation guard: turn-end knowledge persistence nudge

## Problem

mika-dev consistently fails to call `store_fact` after substantive turns — diagnostic conclusions, design validations, incident recoveries, user corrections, institutional knowledge. She produces text and ends the turn. Future sessions lose the context.

Multiple prompt-level fixes were tried across a single 12-hour session on 2026-04-18:

1. **Hot-patch to soul.md** — ineffective (soul.md is a seed file, not the authoritative injected content)
2. **Active self_model update** — partially effective, but compliance drifts under cognitive load
3. **Stronger fabrication risk block** — agent still didn't call store_fact for substantive diagnostics

Pattern: prompt rules against a trained model gradient partially work and drift. Structural engine guards (see #645) work deterministically on first fire.

## Root Cause

LLMs default to "close turn with response text, not write-tool call." Prompt-level instructions to persist knowledge are advisory — the model can ignore or forget them, especially after conversation compaction removes instruction context. This is the same class of failure as the completion-claim guard (#483) and core_memory path guard (#645): behavioral invariants against model gradients need engine-level enforcement.

## Solution

Added a 5th post-condition guard to `run_loop()` in `agent.rs`, following the existing guard family pattern:

1. **`PERSISTENCE_WRITE_TOOLS`** — constant listing the knowledge-persistence tools: `store_fact`, `update_fact`, `update_core_memory`.

2. **`detect_informational_input(text)`** — lazy-compiled regex detecting informational signals in user input: FYI, diagnostic, maintenance check, status update, correction, heads up, etc. Fast-path substring check before regex.

3. **`detect_persistable_output(text)`** — lazy-compiled regex detecting verdict-shaped patterns in assistant output: root cause, this confirms, validated that, lesson learned, key takeaway, etc. Fast-path substring check before regex.

4. **Guard in EndTurn chain** (after fabricated-action-claim guard, before DB save):
   - Only fires on `EndTurn` (not `MaxTokens`/`ContentFilter`)
   - Only fires in conversation mode (not silent/team)
   - Checks `tools_called` against `PERSISTENCE_WRITE_TOOLS` — skips if any was called
   - Checks user input via `detect_informational_input` OR assistant text via `detect_persistable_output`
   - Single retry via `persistence_eval_retry_done` flag
   - **Nudge, not rejection** — softer language than existing guards: "Before ending this turn, consider: [reason]. If any new information, conclusions, or corrections from this conversation should be remembered for future sessions, call store_fact now. If nothing warrants persistence, you may proceed with your response."

## Key Design Decisions

- **Nudge vs rejection:** Unlike guards 1-4 which say "Your response was rejected," this guard uses softer language. The model can legitimately decide nothing is worth persisting. The guard removes the "I didn't think to persist" failure mode without becoming a semantic judge.
- **Conservative write-tool set:** Only `store_fact`, `update_fact`, `update_core_memory`. Workflow tools (`create_task`, `update_task_status`) don't indicate knowledge persistence.
- **No tool-availability gate:** All three persistence tools are in `default_tools()` and always available.
- **Conversation-mode only:** Silent/team modes are background tasks where persistence evaluation adds no value.

## Measurement

Guard fires are observable via:
- **Step count:** 2 steps = guard fired (normal is 1 for a text-only response)
- **Log lines:** `info!` with `matched` and `label` fields when the guard nudges
- **Compliance rate:** (turns where model persists after nudge) / (total nudge fires) — computed from tool_calls table post-hoc

## Prevention

The guard family now has 5 members in the EndTurn chain:
1. Text-based tool call detection
2. Required-tools gate
3. Completion-claim guard (#483)
4. Fabricated action-claim guard (#308)
5. **Persistence evaluation guard (#648)**

Plus 1 dispatch-time guard:
6. Per-turn tool_use dedup guard (#582)

Principle: rules bind at the layer that can enforce them. Behavioral invariants against model gradients need engine-level enforcement. Prompt-level instructions are defense-in-depth, not primary enforcement.

## Files Changed

- `crates/mika-agent/src/agent.rs` — detection functions, constant, guard logic, inline tests
- `crates/mika-agent/tests/eval/test_persistence_eval_guard.rs` — 8 integration tests
- `crates/mika-agent/CLAUDE.md` — guard documentation updated to 5 entries

---
title: "Completion-claim guard: task state enforcement"
category: architecture-patterns
date: 2026-04-08
tags: [agent-loop, work-items, fabrication, guard, post-condition, re-prompt]
issue: 483
---

# Completion-claim guard: task state enforcement

## Problem

mika-dev fabricates turn-ending text claiming tasks are done ("merged", "deployed", "completed") without calling `update_task_status`. The task stays `in_progress` forever while the chat history claims success. The autonomous dev loop silently stalls.

Observed twice in the same session (#480 self-dev run): (1) agent claimed "PR merged and main synced" with zero tool calls — PR was still OPEN; (2) agent processed a build callback and said "Build succeeded" but never called `update_task_status(completed)`.

Both failures share the same shape: *turn-ending completion claim without a paired state-transition tool call*.

## Root Cause

LLMs can rationalize past prompt-level instructions ("always call the tool") and produce confident completion claims without verifying via tools. Prompt-only enforcement is insufficient for agents operating autonomously — the model skips the tool call when the answer seems obvious from context.

## Solution

Added a 3rd post-condition guard to `run_loop()` in `agent.rs`, following the exact pattern of the existing text-based tool call detection and required-tools enforcement gates:

1. **`detect_completion_claim(text)`** — lazy-compiled `\b`-anchored regex (`(?i)\b(merged|deployed|completed?|shipped)\b`) with case-insensitive fast path (substring check before regex). Returns matched keyword or `None`.

2. **Guard in EndTurn chain** (after required-tools, before DB save):
   - Only fires on `EndTurn` (not `MaxTokens`/`ContentFilter`)
   - Gated on `tools.get("update_task_status").is_some()` — skips delegates and team agents that don't have this tool
   - Checks `tools_called.contains("update_task_status")` — skips when the tool was already called
   - Lazy DB query for active tasks filtered to `pending`/`in_progress` (not `blocked`)
   - Single retry via `completion_claim_retry_done` flag (same pattern as `required_tools_retry_done`)
   - Correction message includes active task IDs, statuses, and labels

3. **Keyword selection**: `merged`, `deployed`, `complete`/`completed`, `shipped`. Intentionally excludes `done` (too many false positives: "I'm done analyzing"), `built` ("I built a query"), `finished` (too generic).

### Key design decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Guard placement | After required-tools | Least critical of three; follows established ordering |
| Tool-registry gate | `tools.get(name).is_some()` | Delegates/team agents can't comply — skip them |
| Active items filter | `pending` + `in_progress` only | `blocked` items can't be completed; avoids false positives |
| Retry limit | Single retry | Matches existing pattern; prevents infinite loops |
| DB query | Lazy (only on keyword match) | Most EndTurns have no keywords; avoids unnecessary queries |

### Code pattern (mirrors required-tools gate)

```rust
if matches!(response.stop_reason, LlmStopReason::EndTurn)
    && !completion_claim_retry_done
    && let Some(keyword) = detect_completion_claim(&text)
{
    if tools.get("update_task_status").is_some()
        && !tools_called.contains("update_task_status")
    {
        let active_items = db.list_active_work_items().await
            .unwrap_or_default()
            .into_iter()
            .filter(|t| t.status == "pending" || t.status == "in_progress")
            .collect::<Vec<_>>();

        if !active_items.is_empty() {
            completion_claim_retry_done = true;
            // Push assistant response + correction message, then continue
        }
    }
}
```

## Prevention

- **Code guards over prompt instructions**: If the agent ignoring an instruction would cause real harm, enforce it in the harness. Prompts are defense-in-depth, not the sole mechanism.
- **Three-guard EndTurn chain**: text-based tool call → required-tools → completion-claim. Each uses the same `_retry_done` flag pattern with single retry.
- **Test coverage**: 14 unit tests for `detect_completion_claim()`, 7 eval harness integration tests covering: guard fires, skips (no items, no tool, tool called, no keywords, blocked-only), and single-retry enforcement.

## Related

- `docs/solutions/prompt-engineering/required-tools-enforcement-gate.md` — The pattern this guard follows
- `docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md` — "Code guards over prompt instructions" principle
- `docs/solutions/prompt-engineering/grounding-rule-downstream-state-hallucination.md` — Prompt-level anti-fabrication (defense-in-depth layer)
- `docs/solutions/architecture-patterns/work-item-status-transition-validation.md` — Task state machine

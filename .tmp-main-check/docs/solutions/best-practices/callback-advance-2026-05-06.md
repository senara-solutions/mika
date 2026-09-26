---
module: agent-loop
tags: [intent-guards, callback, milestone, autonomous-loop, structural-enforcement]
problem_type: chronic-stall
category: best-practices
---

# Engine-enforced queue advance: structural over prompt for chronic-stall classes

## Problem

After three documented incidents on 2026-05-06 (closed-issue stall mika#985, milestone wedge cancellation mika#666, heartbeat-doesn't-resume gap), a chronic pattern emerged: the engine has a queue ready to advance, the LLM is the only thing standing between the queue and progress, and the LLM elects to deliberate instead of trust. Existing prompt-level rules at `self-dev/system_prompt.md` lines 113–129 already said "advance, don't deliberate" — they failed under load.

## Principle

When a workflow has a "must-advance" obligation that depends on LLM cooperation, and the LLM is observed to drift on it under load (= prompt-level rules failed twice or more in production), the fix is engine-level:

1. **Intent-precondition guard** that rejects EndTurn unless the advance happened.
2. **Backstop trigger** that fires a second turn if the first slipped through.
3. **Auto-block** as the last resort if neither turn advances.

Prompt-level rules then serve as the **documented contract** for next-readers, not the enforcement surface.

## Solution (mika#991)

Three layers of defense, each surgical:

### Layer 1: `callback_milestone_advance` inline guard

Triggers on `[callback:` + `[milestone-parent: <id>]` markers in the user message. Satisfied by:
- **Path A (advance):** `run_claude_pilot` called (dispatch next child)
- **Path B (halt/finish):** `update_task_status` called targeting the parent with `blocked`/`completed`

Rejects EndTurn on the first attempt; single-retry semantics (guard dormant after one fire). Composes with the existing `callback_terminal_action` guard — milestone callbacks must satisfy both.

### Layer 2: `SilentTrigger::PostCallbackAdvance`

Fired by the dispatcher after a callback turn completes without advancing. The engine checks DB state (parent still `in_progress`, no new callback child) and fires a second turn with explicit advance instructions.

### Layer 3: Auto-block

If the PostCallbackAdvance turn also fails to advance, the engine marks the milestone `blocked` with a structured note. Prevents indefinite idle.

### Prompt hardening

`self-dev/system_prompt.md` Callback Entry Point rewritten to explicitly name the four permitted actions (metadata extraction, milestone advance, pipeline retry, explicit halt) and forbid the deliberation pattern. Sibling skills (`self-dev-webhook-ci`, `self-dev-webhook-qa`, `qa-review-build-callback`) get the same callout. Heartbeat trigger section added with milestone-resume logic.

## Contrapositive

- **mika#988** auto-skip uses the same pattern at a smaller scale (handler exit semantics).
- **mika#996** required-suffix-line guard uses the same pattern for output contracts.
- **mika#991** extends the pattern to LLM-loop-cooperation invariants.

## When to apply this pattern

Apply when ALL of these hold:
1. A workflow has a structural "must-advance" obligation
2. The obligation currently depends on LLM per-turn judgment
3. Prompt-level rules for the obligation have failed ≥2 times in production
4. The advance/halt actions are detectable from tool call signatures

Do NOT apply speculatively — prompt-level rules are cheaper to maintain and iterate on. Escalate to engine-level only when the prompt has demonstrably failed.

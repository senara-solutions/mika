---
title: "Required-tools-gate transport contract — thin final turn after retry"
date: 2026-04-29
module: agent-loop
problem_type: best_practice
component: assistant
severity: high
applies_when:
  - Designing or modifying post-condition guards that reject EndTurn and force a retry
  - Working on any guard in the EndTurn chain that pushes a correction message and continues
  - Debugging why a persisted assistant message is a thin pointer-summary after a guard retry
  - Extending skills that opt into required_tools constraints
related_components:
  - architect-review
  - required-tools-gate
tags:
  - required-tools
  - transport-contract
  - persistence
  - self-contained-response
  - endturn-guard
  - thin-final-turn
  - mika-arch
---

# Required-tools-gate transport contract — thin final turn after retry

## Context

The required-tools EndTurn guard (#270) rejects assistant responses that don't call skill-declared required tools, injects a User correction message, and continues the agent loop. The LLM retries with the prior reasoning visible in its in-memory `request.messages` context. But only the final `EndTurn` response is persisted to the `messages` table — mid-loop `ToolUse` turns persist tool inputs/outputs only, not the assistant's accompanying narration.

This creates a transport-contract gap: the LLM sees the prior substantive content as "already in context" and rationally produces a brief pointer-summary ("Disposition stands: ITERATE with the revised findings above"). From the LLM's perspective, the content is already there. From the persistence layer's perspective, only the final EndTurn text is saved — the substantive content is gone.

This failure family is distinct from the *evasion* family documented in `required-tools-gate-evasion-patterns-2026-04-28.md`. Evasion = the gate didn't fire because the LLM rationalized around its trigger. Transport-contract = the gate fired correctly, but the post-correction turn was thin. Same guard, different failure mode.

## Guidance

### Two-surface fix: engine correction message (primary) + skill prompt (reinforcement)

**Engine-side (primary defense):** Extend the User correction message injected by the required-tools gate to include a persistence-awareness instruction:

```
When you produce your corrected response, restate the full content — do not
reference your prior turn. Only the final response is persisted to the
conversation log; prior turns exist only in the in-memory loop context.
```

This instruction includes the *model* (the why — persistence contract) not just the *rule* (restate content). Reasoning models are likelier to comply when they understand the contract, not just the directive.

**Skill-side (defense-in-depth):** Add a `### Constraints` bullet to skill prompts that produce final-artifact assistant text:

```
**Self-contained final response.** Your final response must be self-contained.
If a prior turn was rejected (e.g., by the required-tools gate) and you
re-issued the review after fetching ground truth, restate the full annotated
findings in your final response — do not refer to prior turns with phrases
like "see above." Only the final response is persisted.
```

### Why two surfaces

The engine correction message fires at retry time — it benefits every skill that opts into `required_tools`. The skill prompt fires at system-prompt time — it internalizes the contract before the LLM even starts generating. Neither is sufficient alone:

- Engine-only: the LLM may not weight the correction message highly enough under token pressure
- Skill-only: the skill prompt fires before the retry, so the LLM may "forget" it by the time the correction loop forces a retry

### Contract-level before structural-level

A structural length-floor guard (reject EndTurn if `len(final) < 0.5 * len(rejected_step_response)`) was considered and rejected for v1. The signal is fuzzy — legitimate corrections may genuinely shorten when the rejected response contained fabricated content that the corrected one rightly drops. The contract-level fix (instruct the LLM) is the right first layer. Promote to structural only if the contract proves insufficient (N=2 recurrence).

This follows the established pattern from `engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`: start with the cheapest effective layer, escalate to structural when prompt-level proves fragile.

## Why This Matters

Every downstream consumer that depends on the final assistant message being a complete artifact is broken by a thin pointer-summary. In the observed incident, `/mika-groom-ticket` Phase 4 received "Two structural deviations from issue body (key shape)" with zero citations — unactionable for the grooming pipeline. The substantive review content (~10k tokens across steps 0 and 2) was in the in-memory context during the session but not in `messages` — and `llm_calls` is metadata-only by design.

The persistence contract (one row per completed turn, `EndTurn`/`MaxTokens`/`StopSequence` only) is load-bearing for conversation context windows, compaction thresholds, and the A2A single-writer invariant. Do not change it to fix this class of problem. The fix must be at the instruction layer.

## When to Apply

- **Any post-condition guard that rejects EndTurn and forces a retry.** The required-tools gate is the first instance, but the pattern applies to any guard in the chain that pushes a correction and continues. The chain-composition invariant (guards append to `request.messages`, they don't reset it) means the self-contained instruction persists across subsequent guard retries.
- **Skills that produce final-artifact text.** Review verdicts, plans, completion summaries — any skill where the persisted assistant message IS the deliverable, not just a side-channel.
- **NOT for pipeline skills** where the LLM passes tool outputs to a downstream stage. Those skills' value is in tool calls, not assistant text.

## Examples

**Before (thin pointer-summary, 363 chars persisted):**

```
Disposition stands: ITERATE with the revised findings above. The prior
turn's review was issued without ground-truth verification of the issue
body — required-tools gate correctly forced the correction. Two structural
deviations from issue body (key shape) and one architectural concern
(record-at-dispatch failure mode) require resolution before second pass.
```

**After (self-contained, full findings restated):**

```
[KEY-FINDING-1] Concern: the key shape in the issue body does not match
the schema defined in architecture.md section 4 — confirmed by ground-truth
verification via gh_read.

[KEY-FINDING-2] Concern: record-at-dispatch failure mode — if the dispatch
call fails after task status is set to in_progress, there is no rollback
path. Confirmed by issue body review.

Disposition: ITERATE

Two structural deviations require resolution before second pass.
```

## Testing

Marker-based assertions (not length-floor) in `tests/eval/grounding_regressions/required_tools_retry_thin_final_turn.rs`:

- **Regression-reproduction:** the pre-fix thin pointer-summary does NOT contain `[KEY-FINDING-1]` and `[KEY-FINDING-2]` markers — proves the assertion has teeth
- **Post-fix happy path:** self-contained final turn DOES contain both markers in order
- **Correction message check:** the User correction message contains "restate the full content" and "Only the final response is persisted"

Marker-based assertions are preferred over length-floor because legitimate corrections may shorten when dropping fabricated content from the rejected step.

## Citations

- mika#890 — origin ticket
- mika#270 — required-tools gate post-condition origin
- mika#272 — observability doc establishing `llm_calls` as metadata-only
- `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` — sibling failure family (evasion vs transport-contract)
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — the broader pattern this fix follows
- `docs/architecture/architecture.md` section 6 — persistence contract (one row per completed turn)

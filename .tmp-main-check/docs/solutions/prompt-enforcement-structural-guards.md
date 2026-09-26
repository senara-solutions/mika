---
module: agent-loop
tags: [post-conditions, idempotency, prompt-enforcement, qa-review]
problem_type: design-pattern
---

# Prompt Enforcement: Structural Guards Replace Text Rules

## Problem

Prompt-level rules (e.g., "do NOT post a second review") are unreliable across LLM providers and can be silently ignored, especially after forced continuation by post-condition guards. Text-only rules have no enforcement mechanism — they rely on the LLM self-policing, which fails under pressure.

## Pattern: Two-Layer Structural Enforcement

When a prompt rule is load-bearing (system correctness depends on it), replace it with structural enforcement:

### Layer 1: Post-condition early-accept

If the primary action of a workflow already succeeded (detectable via `all_tool_summaries`), accept EndTurn immediately — skip remaining guards that might force continuation. This prevents the "forced continuation → re-execution" failure mode.

**Implementation pattern:**
```rust
let skip_remaining_guards = matches!(response.stop_reason, LlmStopReason::EndTurn)
    && has_successful_primary_action(&all_tool_summaries);
```

### Layer 2: Tool-side dedup via ToolContext

Add a per-turn flag (AtomicBool on ToolContext) that tracks whether the action was already performed. The tool checks this flag before executing and returns a structured error if set.

**Implementation pattern:**
```rust
// In the tool handler:
if ctx.action_flag.load(Ordering::Acquire) {
    return ToolOutput::error(structured_error_json);
}
// ... execute ...
ctx.action_flag.store(true, Ordering::Release);
```

### Layer 3: Prompt documentation (non-load-bearing)

Update the prompt to document that the rule is now enforced structurally. The prompt text becomes documentation, not a control mechanism.

## Key Insight

The root cause of the duplicate PR review (#695) was not the LLM ignoring the prompt rule — it was the engine's post-condition guard forcing continuation after a legitimate EndTurn. The LLM then rationally re-did work it had already completed. Structural guards address the actual failure mode (engine-caused re-execution), not the symptom (LLM non-compliance).

## When to Apply

- The rule prevents duplicate side effects (webhooks, API calls, notifications)
- Violation is detectable via tool-call history (name + args pattern)
- The action is irreversible or has observable external effects
- Multiple LLM providers must respect the rule

## When NOT to Apply

- Rules about response format or tone (no side effects)
- Rules where violation is harmless (redundant reads)
- Rules that require semantic judgment (not detectable by pattern matching)

## References

- #695: qa-review duplicate PR review incident
- `feedback_prompt_enforcement_fragile` pattern (prior art)
- #582: per-response dedup (same tool call within one LLM response)
- #695 differs from #582: cross-step dedup within a turn, not within a response

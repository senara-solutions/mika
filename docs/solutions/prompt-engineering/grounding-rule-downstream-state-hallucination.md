---
title: "Grounding Rule: Preventing Downstream State Hallucination from Valid Tool Results"
category: prompt-engineering
date: 2026-03-24
severity: high
tags: [hallucination, prompt-guardrail, confabulation, callback-framing, grounding, anti-hallucination]
modules: [crates/mika-agent/src/prompt.rs, crates/mika-agent/src/agent.rs]
issue: "#141"
related: ["#135"]
---

# Grounding Rule: Preventing Downstream State Hallucination from Valid Tool Results

## Problem

The agent fabricated downstream state claims after receiving valid tool results. A build tool returned "Compilation succeeded" and the agent responded with "PR #140 is solid -- ready for your review when you want to merge." No tool result contained PR review status, CI status, or merge readiness information.

This is a distinct hallucination subclass from #135 (which fixed misreading `tool_history` metadata as executed actions). The agent was over-extrapolating from a real, valid result to claim states that were never confirmed by any tool.

**Observed in:** `mika-dev` agent, session `fa2bdebf-b9e3-4015-8a8c-485fca43c731`, message [982] -- post-build callback turn.

## Root Cause

The system prompt lacked a guardrail against confabulating downstream state from valid results. The existing guardrails covered:

- **General fabrication** ("Never fabricate information") -- too generic for this pattern
- **Tool history misattribution** (#135) -- different subclass (misreading metadata as actions)
- **Failure transparency** -- about disclosing failures, not about limiting success claims
- **Confirmation before action** -- about not starting workflows, not about state claims

None addressed the specific pattern: valid tool result -> fabricated claim about a *different* system.

## Solution

A three-point fix following the codebase's proven four-part anti-hallucination formula (from `docs/solutions/integration-issues/mcp-self-knowledge-command-hallucination.md`):

### 1. Grounding guardrail in `build_system_prompt` Instructions section

Added to `crates/mika-agent/src/prompt.rs` alongside existing #135 guardrails:

```rust
prompt.push_str(
    "- **Grounding rule:** NEVER claim the state of a downstream or adjacent system \
     (PR status, CI pipeline, deploy, merge readiness, branch health) unless a tool \
     result in this conversation explicitly confirms that exact state. A successful \
     tool result confirms only the specific action that tool performed — do not \
     extrapolate.\n  \
     BAD: Build tool returns \"Compilation succeeded\" → you say \"PR is ready for review.\"\n  \
     GOOD: Build tool returns \"Compilation succeeded\" → you say \"The build passed.\"\n  \
     If you need downstream status, call the appropriate tool (e.g., check_work_item, \
     query_timeline) to verify it first.\n",
);
```

Uses the four-part formula: (1) CRITICAL/NEVER prohibition, (2) concrete BAD example, (3) concrete GOOD example, (4) tool suggestion for verification.

### 2. Grounding instruction in `format_callback_framing`

Added to `crates/mika-agent/src/agent.rs` after the existing untrusted-content warning:

```rust
"Report only what this result explicitly states. Do not infer the state of any \
 system, artifact, or process not mentioned in the result."
```

This is the single chokepoint for both CLI and server callback paths -- cross-point reinforcement.

### 3. Grounding instruction in silent callback trigger context

Added to the `SilentTrigger::Callback` arm in `agent.rs`:

```rust
"IMPORTANT: A successful result confirms only the specific action performed. \
 NEVER extrapolate to downstream states (PR status, CI health, deploy readiness) \
 that the result does not explicitly mention."
```

Critical because `build_silent_prompt` has NO Instructions section -- this is the only grounding for the production server-mode callback path.

## Key Design Decision: Coverage Matrix

The fix targets all six prompt paths through which the agent processes information:

| Prompt Path | Has Instructions | Has Callback Framing | Fix Coverage |
|---|---|---|---|
| Interactive normal | Yes | No | System prompt grounding rule |
| Interactive callback | Yes | Yes | System prompt + enhanced framing |
| Silent callback | **No** | Yes | Enhanced framing + trigger context |
| Silent heartbeat/reflection | **No** | No | Not needed (no tool results) |
| Team agent | Yes | No | System prompt grounding rule |
| Delegate task | Yes | No | System prompt grounding rule |

The silent callback path was the most critical gap -- it's the production server path and lacked the Instructions section entirely.

## Over-Restriction Avoidance

The guardrail is scoped to "downstream or adjacent system" to avoid preventing valid direct-result summaries:

- **Valid:** `create_reminder` returns success -> "I set a reminder for Monday at 9am" (direct result)
- **Valid:** `cargo build` returns success -> "The build passed" (direct result)
- **Invalid:** `cargo build` returns success -> "The PR is ready for review" (downstream state)
- **Invalid:** `delegate_task` returns "tests pass" -> "CI pipeline is green" (different system)

## Prevention

1. **New guardrails should follow the four-part formula:** (1) CRITICAL/NEVER prohibition, (2) concrete bad example, (3) concrete good example, (4) cross-point reinforcement at multiple prompt injection sites.
2. **Check all prompt paths:** `build_system_prompt` (interactive) and `build_silent_prompt` (background) have different instruction sets. Changes to one may need equivalent coverage in the other.
3. **`format_callback_framing` is the cross-path chokepoint:** Any grounding instruction placed here covers both CLI and server callback paths automatically.
4. **LLM instruction strength matters:** "Avoid" and "Do not" are treated as soft suggestions. "CRITICAL: NEVER" with concrete examples is more effective (documented in `mcp-self-knowledge-command-hallucination.md`).

## Related

- Issue #135 / commit `0b0ed63`: Tool history context guardrails (prior hallucination subclass)
- `docs/solutions/integration-issues/mcp-self-knowledge-command-hallucination.md`: Four-part anti-hallucination formula
- `docs/solutions/architecture/rewind-context-marker-confabulation-prevention.md`: Confabulation prevention via context markers
- `docs/solutions/logic-errors/agent-creates-duplicates-after-compaction.md`: Proactive state checking pattern

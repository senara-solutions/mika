# Periodic Memory Reflection

**Date:** 2026-03-03
**Status:** Brainstorm complete

## What We're Building

A daily memory reflection system that runs as a background agent pass at the end of the user's active hours. The reflection agent reviews the day's conversations and new facts, then performs two jobs:

1. **Memory housekeeping** — consolidate duplicate facts, prune stale information, promote important patterns from Layer 2 (structured facts) into Layer 1 (core memory)
2. **Insight discovery** — surface connections the agent missed during real-time conversation, identify evolving priorities, recognize recurring themes

The feature is **disabled by default** and configurable per-agent via `identity.toml`.

## Why This Approach

### Problem

Mika's memory updates are 100% reactive. The agent only updates memory when it happens to notice something important mid-conversation. This leads to:

- Core memory (2000 tokens, always in system prompt) getting stale — important context buried in Layer 2 facts that should be promoted
- Redundant or conflicting facts accumulating in Layer 2 with no cleanup
- Patterns that emerge across multiple conversations going unnoticed (e.g., user mentions a topic 5 times over 2 weeks but it never makes it to core memory)

### Solution: Daily end-of-day reflection

- Runs once per day at a configurable time (default: 20:00 user local time)
- Independent schedule from heartbeat — reflection is fundamentally different from "should I message the user?"
- Skips if no conversations or new facts that day (zero-cost on quiet days)
- Uses existing `run_silent_agent` infrastructure via a new `SilentTrigger::Reflection` variant

## Key Decisions

### 1. Disabled by default, opt-in via config

```toml
# identity.toml
[reflection]
enabled = false          # opt-in
time = "20:00"           # user local time, end of active hours
notify = false           # opt-in summary via send_message
```

Users who want background memory maintenance enable it explicitly. No surprises.

### 2. Independent daily schedule (not piggybacking on heartbeat)

Heartbeat asks "is there something I should tell the user right now?" — fast, cheap, engagement-focused.
Reflection asks "what have I learned today and how should I update my understanding?" — heavier, analytical, memory-focused.

Mixing them muddies both. Once per day is predictable, cheap, and sufficient.

### 3. Evidence-required tool parameter in reflection mode

During reflection, memory tools require an `evidence` field citing the specific conversation timestamp and quote that justifies the change. Empty evidence = tool call rejected.

```rust
if ctx.is_reflection && evidence.is_empty() {
    return Err(ToolError::EvidenceRequired(
        "Reflection mode requires evidence for memory changes"
    ));
}
```

During normal conversation, the field is optional (evidence is implicit — the user just said it).

Combined with a conservative prompt: "Only update based on things the user explicitly said or did. Never infer from a single data point. If unsure, skip it."

Belt and suspenders — prompt sets intent, required field enforces it.

### 4. Core memory edit cap: 5 per reflection

- Conversation mode: 3 edits/session (existing)
- Reflection mode: 5 edits/session (new, independent budget)
- Guards against overenthusiastic rewrites after one unusual day
- 5 is generous for daily maintenance (most days: 1-2 edits)
- Consistently hitting 5 is a signal for prompt drift — visible in audit log

### 5. Audit log + opt-in notification

All reflection changes always logged to `memory_events` with `session_type = "reflection"`.

When `notify = true`, send a brief summary of meaningful changes only:

```
Daily reflection — 2 updates:
  - Moved "preparing Series A fundraise" to current priorities
  - Noted you prefer async communication with Thomas
```

No message if nothing changed, even with notifications on.

### 6. Implementation: New `SilentTrigger::Reflection` variant

Follows the established pattern (Heartbeat and Reminder are both SilentTrigger variants). Reuses `run_silent_agent` with a reflection-specific code path for:

- Loading today's conversations and recent facts as context
- Reflection-specific system prompt
- Evidence-required enforcement on memory tools
- 5-edit limit instead of 3

## Reflection Prompt (Draft)

```
You are in REFLECTION mode. This is your daily end-of-day review.

Your job: Review today's conversations and recently stored facts. Update your
memory to better serve the user tomorrow.

## What to do

1. HOUSEKEEPING: Scan for duplicate or redundant facts. Consolidate them.
   Remove stale information that's no longer relevant.

2. PROMOTION: If important patterns in Layer 2 facts deserve a place in core
   memory, promote them. Core memory is precious (2000 tokens) — only promote
   information that will be useful in most future conversations.

3. INSIGHT: Look for themes across today's conversations. Has the user's
   focus shifted? Are there emerging priorities? New people becoming important?

## Rules

- Only update based on things the user EXPLICITLY said or did
- Never infer preferences from a single data point — wait for confirmation
  across multiple conversations
- The evidence field MUST cite a specific conversation timestamp and quote
- If unsure whether to update, DON'T — you can always learn it more clearly
  tomorrow
- Prioritize: you have a maximum of 5 memory edits this session

## Context provided

- Your current core memory (Layer 1)
- Today's conversations (summaries)
- New facts stored today (Layer 2)
- Recent memory events (what changed recently)
```

## Open Questions

None — all key decisions resolved during brainstorming.

## Technical Notes

### Pre-filters (skip reflection when unnecessary)

- No conversations today AND no new facts → skip entirely
- Agent lock busy → defer (same as heartbeat try_lock behavior)
- Outside active hours → shouldn't trigger (scheduler respects configured time)

### Data needed for reflection context

- Today's conversations: need new DB query `get_conversations_since(hours: 24)` or similar
- Today's new facts: need new DB query `get_facts_since(hours: 24)`
- Recent memory events: already available via existing queries
- Current core memory: already loaded by `build_silent_prompt`

### Cost estimate

- ~1 Claude API call per day (when conversations exist)
- Input: system prompt + core memory + conversation summaries + recent facts (~3-5K tokens)
- Output: tool calls + reasoning (~500-1K tokens)
- Roughly $0.03-0.05/day at Sonnet pricing — negligible for a daily user

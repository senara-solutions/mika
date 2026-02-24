---
status: pending
priority: p1
issue_id: "074"
tags: [code-review, agent-native, functional]
dependencies: []
---

# Inject current time and timezone into system prompts

## Problem Statement
Neither the conversation-mode system prompt nor the silent-mode prompt provides the current UTC time or user timezone. The `create_reminder` tool requires ISO 8601 datetime, but the LLM has no reliable way to compute "in 2 hours" or "tomorrow at 3pm" without knowing the current time and user's timezone.

## Findings
- `build_system_prompt` (prompt.rs:65-111) does not inject current time
- `build_silent_prompt` (prompt.rs:124-166) does not inject current time
- `create_reminder` tool description says "Parse the user's natural language time into ISO 8601" but provides no time context
- User timezone stored in `customer_config` table but never loaded into prompt
- LLM will hallucinate current time, leading to reminders at wrong times or past-time rejection errors
- Flagged by: Agent-Native Reviewer (P1), Architecture Strategist (P2)

## Proposed Solutions

### Option 1: Inject time in prompt builder (Recommended)
Add current UTC time to both prompts via `chrono::Utc::now()`:
```rust
write!(prompt, "## Current Time\nUTC: {}\n\n", chrono::Utc::now().to_rfc3339()).unwrap();
```
And inject timezone from customer_config if available.

**Pros:** Clean, simple, all prompts get time awareness
**Cons:** Adds DB read for timezone (already in hot path though)
**Effort:** 30 minutes
**Risk:** Low

### Option 2: Add time to PromptContext struct
Add `current_time: DateTime<Utc>` and `timezone: Option<String>` to `PromptContext` and `SilentPromptContext`. Caller provides the values.

**Pros:** No hidden DB access in prompt builder, testable
**Cons:** More fields to thread through
**Effort:** 45 minutes
**Risk:** Low

## Recommended Action
Option 2 — cleaner separation of concerns, more testable.

## Technical Details
**Affected files:**
- `crates/mika-agent/src/prompt.rs` — add time/tz to context structs and prompt output
- `crates/mika-agent/src/agent.rs` — load timezone, pass to prompt context
- `crates/mika-agent/src/db.rs` — `get_customer_config("timezone")` already exists

## Acceptance Criteria
- [ ] System prompt includes current UTC time
- [ ] System prompt includes user timezone when configured
- [ ] Silent prompt includes current UTC time
- [ ] Prompt tests updated to verify time injection
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review)
**Actions:** Identified missing time awareness blocking reliable reminder creation

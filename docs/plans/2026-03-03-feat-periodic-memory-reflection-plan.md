---
title: "feat: Add periodic memory reflection"
type: feat
status: completed
date: 2026-03-03
brainstorm: docs/brainstorms/2026-03-03-periodic-memory-reflection-brainstorm.md
---

# feat: Add periodic memory reflection

## Overview

Add a daily end-of-day memory reflection system that runs as a background agent pass. The reflection agent reviews the day's conversations and facts, then performs memory housekeeping (consolidate, prune, promote) and insight discovery (surface patterns, evolving priorities). Disabled by default, opt-in via `identity.toml`.

## Problem Statement

Mika's memory updates are 100% reactive — the agent only updates memory when it happens to notice something important mid-conversation. This leads to:

- Core memory getting stale (important context buried in Layer 2 that should be promoted)
- Redundant or conflicting facts accumulating with no cleanup
- Cross-conversation patterns going unnoticed (user mentions a topic 5 times over 2 weeks but it never reaches core memory)

## Proposed Solution

A new `SilentTrigger::Reflection` variant reusing the existing `run_silent_agent` infrastructure. The scheduler fires it daily at a configurable time (default 20:00 local). Conservative by design: evidence-required for memory changes, 5-edit cap, skip if no activity.

## Implementation Phases

### Phase 1: Configuration & Data Layer

**1a. Add `ReflectionConfig` to `Identity` struct**

File: `crates/mika-agent/src/prompt.rs`

```rust
#[derive(Debug, Deserialize, Clone)]
pub struct ReflectionConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_reflection_time")]
    pub time: String,       // HH:MM format, validated with chrono::NaiveTime
    #[serde(default)]
    pub notify: bool,
}

fn default_reflection_time() -> String { "20:00".to_string() }
```

Add to `Identity`:
```rust
pub struct Identity {
    pub name: String,
    pub emoji: String,
    #[serde(default)]
    pub reflection: Option<ReflectionConfig>,
}
```

Add validation helper: `ReflectionConfig::parse_time() -> Option<NaiveTime>` that parses `HH:MM` format. Log warning and treat as disabled if invalid.

**1b. Database migration (schema v10)**

File: `crates/mika-agent/src/db.rs`

New table:
```sql
CREATE TABLE IF NOT EXISTS reflection_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ran_at INTEGER NOT NULL,          -- Unix timestamp
    status TEXT NOT NULL DEFAULT 'completed',  -- completed, partial, failed
    changes_made INTEGER DEFAULT 0,   -- number of memory mutations
    summary TEXT,                      -- brief description of what changed
    created_at TEXT DEFAULT (datetime('now'))
);
CREATE INDEX idx_reflection_runs_ran_at ON reflection_runs(ran_at);
```

**1c. New database queries**

File: `crates/mika-agent/src/db.rs`

- `get_conversations_since(since_utc: i64) -> Result<Vec<ConversationMessage>>` — messages with `created_at >= since_utc`, excluding channel_type "reflection"
- `get_memory_events_since(since_utc: i64) -> Result<Vec<MemoryEvent>>` — audit log entries since timestamp
- `record_reflection_run(status: &str, changes: i64, summary: Option<&str>) -> Result<()>`
- `last_reflection_run_today(tz: &str) -> Result<bool>` — check if a reflection (any status) exists for today in user's timezone

Add async wrappers in `async_db.rs`.

### Phase 2: Tool Modifications

**2a. Add `is_reflection` flag to `ToolContext`**

File: `crates/mika-agent/src/tools/mod.rs`

```rust
pub struct ToolContext<'a> {
    // ... existing fields ...
    pub is_reflection: bool,
}
```

**2b. Add `evidence` parameter to memory write tools**

Files:
- `crates/mika-agent/src/tools/update_core_memory.rs`
- `crates/mika-agent/src/tools/store_fact.rs`
- `crates/mika-agent/src/tools/update_fact.rs`

Add optional `"evidence"` field to each tool's JSON schema. In reflection mode, reject if empty:

```rust
if ctx.is_reflection {
    let evidence = input.get("evidence").and_then(|v| v.as_str()).unwrap_or("");
    if evidence.trim().is_empty() {
        return Ok(ToolOutput::error(
            "Reflection mode requires an evidence field citing specific conversation content."
        ));
    }
}
```

Store evidence in `memory_events.reasoning` field (prepend `[evidence] ` marker to distinguish from normal reasoning).

**2c. Reflection-specific edit cap**

File: `crates/mika-agent/src/tools/update_core_memory.rs`

```rust
const MAX_CORE_MEMORY_EDITS_PER_SESSION: u32 = 3;
const MAX_CORE_MEMORY_EDITS_REFLECTION: u32 = 5;

// In execute():
let max_edits = if ctx.is_reflection {
    MAX_CORE_MEMORY_EDITS_REFLECTION
} else {
    MAX_CORE_MEMORY_EDITS_PER_SESSION
};
if current_edits >= max_edits && !ctx.is_onboarding {
    return Ok(ToolOutput::error(...));
}
```

### Phase 3: Silent Agent Extension

**3a. Add `SilentTrigger::Reflection` variant**

File: `crates/mika-agent/src/agent.rs`

```rust
pub enum SilentTrigger {
    Heartbeat,
    Reminder { id: i64, message: String },
    Reflection,
}
```

**3b. Extend `run_silent_inner` for Reflection**

In the trigger match block:
- Set `channel_type = "reflection"`
- Set `session_id = format!("reflection-{date}")` where date is today in user's timezone
- Load reflection context data (conversations since midnight, memory events since midnight, recent facts)
- Build trigger-specific context string with the reflection prompt
- Use `safe_always_on_skills()` (same as heartbeat)
- Set `is_reflection: true` on `ToolContext`

**3c. Reflection prompt and context assembly**

File: `crates/mika-agent/src/prompt.rs`

Add to `SilentPromptContext`:
```rust
pub recent_conversations: Option<&'a str>,  // Pre-formatted conversation digest
pub recent_memory_events: Option<&'a str>,  // Pre-formatted memory event digest
```

In `build_silent_prompt`, when these fields are `Some`, add sections:
```
## Today's Conversations
{recent_conversations}

## Recent Memory Changes
{recent_memory_events}
```

The reflection trigger context (passed as `trigger_context`):

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
- Never infer preferences from a single data point
- The evidence field MUST cite a specific conversation timestamp and quote
- If unsure whether to update, DON'T — you can learn it more clearly tomorrow
- Prioritize: you have a maximum of 5 memory edits this session
```

**Context data preparation** (in `run_silent_inner` before prompt build):

1. Compute `local_midnight_utc` from user's timezone
2. Load conversations since midnight (cap at 50,000 chars, matching compaction limit)
3. Load memory events since midnight
4. Format into readable digest strings
5. If conversation compaction summary exists, include it for older context

### Phase 4: Scheduler Integration

**4a. Add reflection check to `ReminderScheduler`**

File: `crates/mika-agent/src/scheduler.rs`

Add `check_and_fire_reflection()` method to `ReminderScheduler` (no new struct needed — reuse existing scheduler that already has all dependencies):

```rust
async fn check_and_fire_reflection(&self) {
    // 1. Load identity, check reflection.enabled
    let identity = load_identity_async(&self.home_dir).await;
    let config = match identity.reflection {
        Some(ref c) if c.enabled => c,
        _ => return,
    };

    // 2. Parse configured time
    let time = match config.parse_time() {
        Some(t) => t,
        None => return,  // invalid time, warning already logged at startup
    };

    // 3. Get user timezone, compute current local time
    let tz_str = self.db.get_customer_config("timezone").await...;
    let now_local = Utc::now().with_timezone(&tz);

    // 4. Check if it's past the configured time
    if now_local.time() < time { return; }

    // 5. Check if already ran today
    if self.db.last_reflection_run_today(&tz_str).await... { return; }

    // 6. Check recent user activity (defer if within 30 min)
    if let Ok(Some(last_msg)) = self.db.last_user_message_time().await {
        if elapsed < chrono::TimeDelta::minutes(30) { return; }
    }

    // 7. Check conversation volume (skip if no activity today)
    let midnight_utc = compute_midnight_utc(now_local, &tz);
    let conversations = self.db.get_conversations_since(midnight_utc).await...;
    if conversations.is_empty() { return; }

    // 8. Acquire agent lock (server mode)
    if let Some(ref lock) = self.agent_lock {
        let guard = match lock.try_lock() {
            Ok(g) => g,
            Err(_) => return,  // agent busy, try next tick
        };
        // ... run reflection with guard held
    }

    // 9. Run silent agent with SilentTrigger::Reflection
    // 10. Record result in reflection_runs table
}
```

**4b. Call from poller**

In `spawn_poller()`, call `check_and_fire_reflection()` on each tick alongside `check_and_fire_reminders()`. The reflection method has its own "already ran today" check, so 60s polling is fine — it'll be a no-op after the first successful run.

**4c. Recovery on startup**

In `recover()`, check if reflection was missed today (enabled, past configured time, no record). If missed and within 4 hours of configured time, fire once.

### Phase 5: Notification

In `run_silent_inner` for reflection trigger, after the agent loop completes:

1. Count memory mutations made during this session (from `memory_events` with this session_id)
2. If `notify = true` AND `has_message_sender` AND changes > 0:
   - Build brief summary from the memory events
   - Call `send_message` with formatted summary
3. Record `reflection_runs` entry with status, change count, and summary

## Acceptance Criteria

- [x] `identity.toml` `[reflection]` section parsed with defaults (enabled=false, time="20:00", notify=false)
- [x] Invalid `time` format logs warning and disables reflection
- [x] Reflection runs daily at configured local time (verified via poller)
- [x] Skips when: disabled, no conversations today, already ran today, user active within 30min, agent lock busy
- [x] `SilentTrigger::Reflection` routes through `run_silent_agent` correctly
- [x] Memory tools require `evidence` field in reflection mode, reject if empty
- [x] Core memory edit cap is 5 in reflection mode, 3 in conversation mode
- [x] All mutations logged to `memory_events` with session_id `reflection-{date}`
- [x] `reflection_runs` table records each run with status and change count
- [x] Opt-in notification sends summary via `send_message` only when changes > 0
- [x] CLI mode: reflection fires if process is running at configured time
- [x] Server mode: uses `try_lock()`, defers if busy
- [x] Recovery on startup fires missed reflection (within 4-hour window)
- [x] Schema migration v10 applied cleanly

## Technical Considerations

### Edge cases resolved

- **DST transitions:** `chrono-tz` resolves correctly. The "already ran today" check uses timezone-aware date, so spring-forward/fall-back are handled.
- **Timezone changes mid-day:** Takes effect on next poller tick. Could cause double-reflection (acceptably rare). Could cause missed reflection (catches up tomorrow).
- **Partial failures:** Accepted as "good enough." Audit log captures all changes. Next day's reflection sees the partial state and can continue. Record as `status = "partial"`.
- **Prompt size:** Cap conversation digest at 50,000 chars (matching compaction limit). Prefer compacted summaries for earlier periods.
- **Multi-agent:** Each agent reflects independently based on its own `identity.toml`. This is correct — agents have separate databases.
- **No message sender (CLI):** `send_message` call will be a no-op. Notification is best-effort.

### Not in scope (future work)

- Manual reflection trigger (`mika reflect` command)
- Cross-agent reflection in multi-agent setups
- Configurable edit cap (hardcoded at 5)
- Weekly/monthly deeper reflection passes

## Files to Modify

| File | Change |
|------|--------|
| `crates/mika-agent/src/prompt.rs` | Add `ReflectionConfig`, extend `Identity`, extend `SilentPromptContext`, extend `build_silent_prompt` |
| `crates/mika-agent/src/db.rs` | Schema v10 migration, new queries (conversations_since, memory_events_since, reflection_runs) |
| `crates/mika-agent/src/async_db.rs` | Async wrappers for new queries |
| `crates/mika-agent/src/agent.rs` | `SilentTrigger::Reflection` variant, reflection code path in `run_silent_inner` |
| `crates/mika-agent/src/tools/mod.rs` | Add `is_reflection` to `ToolContext` |
| `crates/mika-agent/src/tools/update_core_memory.rs` | Evidence enforcement, reflection edit cap |
| `crates/mika-agent/src/tools/store_fact.rs` | Evidence enforcement in reflection mode |
| `crates/mika-agent/src/tools/update_fact.rs` | Evidence enforcement in reflection mode |
| `crates/mika-agent/src/scheduler.rs` | `check_and_fire_reflection()`, recovery logic |
| `crates/mika-cli/src/commands/chat.rs` | Pass-through (poller already handles reflection) |
| `crates/mika-agent/src/server/mod.rs` | Pass-through (poller already handles reflection) |

## Verification

1. **Unit tests:** Test `ReflectionConfig` parsing (valid, invalid, missing), `last_reflection_run_today` query, `get_conversations_since` query, evidence rejection logic, edit cap switching
2. **Integration test:** Mock the full reflection flow — set up conversations and facts, run `check_and_fire_reflection`, verify memory_events and reflection_runs records
3. **Manual test (CLI):** Enable reflection in `identity.toml`, set time to 1 minute from now, have a conversation, wait for reflection to fire, check `mika memory` for changes
4. **Manual test (Server):** Same via `/heartbeat`-style trigger or wait for poller

## References

- Brainstorm: `docs/brainstorms/2026-03-03-periodic-memory-reflection-brainstorm.md`
- Heartbeat pattern: `crates/mika-agent/src/server/handlers.rs:286-437`
- Reminder poller: `crates/mika-agent/src/scheduler.rs:290-302`
- Silent agent: `crates/mika-agent/src/agent.rs:975-1036`
- Core memory tool: `crates/mika-agent/src/tools/update_core_memory.rs`
- Solution: reminders-never-fire: `docs/solutions/runtime-errors/reminders-never-fire-at-scheduled-time.md`

---
status: pending
priority: p2
issue_id: 725
tags: [code-review, performance, prompt]
---

# Filter preferences to task_policy_* in heartbeat prompt

## Problem Statement

`run_silent_agent()` calls `db.list_preferences()` which loads ALL preferences for the agent (notification prefs, display prefs, communication style, etc.) and injects them into the heartbeat prompt. The `<task-health-instructions>` block specifically references `task_policy_` prefixed preferences. Loading all preferences wastes tokens and adds noise that is irrelevant to task health reasoning.

## Findings

- `list_preferences()` has no filter — returns all rows from `preferences` table for the agent
- `<task-health-instructions>` instruction 7 tells the agent to store policies with `task_policy_` prefix
- `search_preferences(agent_id, query)` already exists and does a `LIKE` query on category and value
- Preferences table has no LIMIT clause — could grow unbounded (though UPSERT deduplication prevents runaway growth)

## Proposed Solutions

### Option A: Filter in agent.rs using search_preferences
```rust
let stored_preferences = db.search_preferences("task_policy_").await.unwrap_or_default();
```
- **Pros:** No new code, uses existing method
- **Cons:** `search_preferences` does LIKE on both category AND value — might match non-policy prefs

### Option B: Add a dedicated list_preferences_by_prefix method
- **Pros:** Precise filtering on category prefix only
- **Cons:** New method for a narrow use case

### Option C: Cap the preferences with `.iter().take(20)` in prompt rendering
- **Pros:** Simple, prevents unbounded growth
- **Cons:** Doesn't reduce noise from non-task preferences

**Recommended:** Option A — simplest, effective enough given the `task_policy_` naming convention.

## Acceptance Criteria

- [ ] Only `task_policy_*` prefixed preferences are injected into heartbeat prompt
- [ ] Non-task preferences are excluded from the `<stored-preferences>` block

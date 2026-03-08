---
status: complete
priority: p2
issue_id: 565
tags:
  - code-review
  - agent-native
  - ux
dependencies: []
---

# Constraint-violation messages lack details for agent to give useful response

## Problem Statement

When the DB constraint fires, the agent receives generic messages:
- `"A similar reminder already exists and is still active. No duplicate created."`
- `"A similar event already exists. No duplicate created."`

The agent has no idea *which* existing entry matched — no ID, label, or schedule. It cannot tell the user "You already have a reminder 'Year-end review' (ID 42) scheduled for Dec 31." This is especially important because the constraint path fires without the agent having called `list_reminders` first (the whole point of DB-level defense).

## Findings

- **Files:** `crates/mika-agent/src/tools/create_reminder.rs:154-156`, `crates/mika-agent/src/tools/store_fact.rs:268-270`
- **Flagged by:** Agent-Native Reviewer
- The constraint fires precisely when the agent skipped the proactive check — so the message is the agent's only source of context
- Querying for the existing entry after the constraint fires is cheap (the index already proved it exists)

## Proposed Solutions

### Option A: Query for existing entry after constraint violation (Recommended)

After catching the violation, do a SELECT to find the matching entry and include its details in the message.

For reminders:
```
"A reminder with label 'Year-end review' already exists (ID abc123, fires 2099-12-31 23:59:59 UTC). No duplicate created."
```

For events:
```
"An event 'Board meeting' on 2026-04-15 already exists (ID 7). No duplicate created."
```

- **Pros:** Agent can give user a rich response, cheap query
- **Cons:** Extra DB round-trip on duplicate path (rare)
- **Effort:** Small
- **Risk:** Low

### Option B: Keep messages generic

- **Pros:** Simpler code
- **Cons:** Poor UX — agent parrots vague message
- **Effort:** None
- **Risk:** Low

## Recommended Action

Option A. The duplicate path is rare, and the extra query is trivial.

## Technical Details

- **Affected files:** `crates/mika-agent/src/tools/create_reminder.rs`, `crates/mika-agent/src/tools/store_fact.rs`

## Acceptance Criteria

- [ ] Constraint-violation messages include existing entry ID and key details
- [ ] Tests verify the enriched messages

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Found during agent-native review | Agent needs context to give good UX |

# Brainstorm: Conversation Rewind

**Date:** 2026-03-11
**Status:** Ready for planning

## What We're Building

A rewind feature that lets the user go back in time by deleting recent messages AND reversing the memory/fact changes those messages caused. The audit_events table already stores before_value/after_value for every mutation — this is effectively an undo log.

**The problem:** During development and testing, deleting recent messages leaves Mika with knowledge from conversations that no longer exist. Memory state and message state diverge.

## Why This Approach

The audit_events table is already an append-only mutation log with trace_id correlation to messages. Rewind is a natural extension: collect trace_ids from messages being deleted, query audit_events for those trace_ids, reverse each mutation using before_value. No new infrastructure needed — just a reversal engine that reads the existing undo log.

## Key Decisions

### 1. Fix audit trail first (prerequisite)

`store_fact` for person and preference upserts does NOT capture `before_value` today. The rewind engine is only as good as the audit trail. Fix: read current value before upsert, log as `before_value`. A SELECT before the INSERT OR REPLACE, a few lines per tool.

### 2. Make `after_value` nullable

The `audit_events.after_value` column is currently `NOT NULL`. Deletion reversals need `(Some(before), None)` which can't exist today. Make it nullable in the same schema migration.

### 3. Add `rewound_by_trace_id` column

Add a nullable `TEXT` column to `audit_events` that references the rewind operation's trace_id. Preserves full history, enables "what was rewound and by which operation" queries. Same migration.

### 4. Single schema migration (v8 -> v9)

All three changes in one migration:
- `ALTER TABLE audit_events ADD COLUMN rewound_by_trace_id TEXT`
- Recreate `audit_events` to make `after_value` nullable (SQLite limitation — ALTER TABLE can't change nullability, need table rebuild)
- No new tables needed

### 5. Search index re-index as post-rewind step

After reversal, run re-indexing on affected facts to update FTS5 + embeddings. Not a blocker for the feature — handle as a cleanup step in the rewind function.

### 6. Reversal order: reverse chronological

If turn 1 set core_memory to "A" and turn 2 set it to "B", rewinding must restore "B" -> "A" first (turn 2 reversal), then "A" -> original (turn 1 reversal). Newest first ensures correct state at each step.

## Rewind Targets

All tables mutated by agent tool calls:

| Table | Tool | before_value today? | Fix needed? |
|-------|------|---------------------|-------------|
| core_memory | update_core_memory | Yes | No |
| people | store_fact (person) | No (upserts) | Yes — read before upsert |
| commitments | store_fact (commitment) | No (creation only) | No — creation = before is NULL |
| commitments | update_fact | Yes (status) | No |
| preferences | store_fact (preference) | No (upserts) | Yes — read before upsert |
| events | store_fact (event) | No (creation only) | No — creation = before is NULL |
| tasks | create_work_item/reminder | No (creation only) | No — creation = before is NULL |
| tasks | update_work_item_status | Yes (status) | No |

## UX Design

### TUI Slash Commands

```
/undo              — delete last exchange + reverse memory changes
/rewind 3          — rewind last 3 exchanges
/rewind to <id>    — rewind to specific message ID
```

Each shows a preview before executing:

```
Rewinding 3 exchanges (messages 58-63):
- Will restore core_memory.current_priorities to previous value
- Will delete person "Sarah" (created in message 60)
- Will delete reminder "Call Sarah Thursday" (created in message 62)

Proceed? [y/N]
```

### Dashboard

Click any message -> "Rewind to here" button -> preview -> confirm -> API call to mika-spirit.

### Server API

New endpoint: `POST /api/v1/rewind` (dashboard auth) with body `{ session_id, after_message_id }`. Returns `RewindSummary` (preview mode) or executes (with `confirm: true`).

## Reversal Engine (pseudocode)

```rust
fn rewind_to(session_id: &str, after_message_id: i64) -> Result<RewindSummary> {
    // 1. Get messages to delete
    let messages = db.get_messages_after(session_id, after_message_id)?;
    let trace_ids: Vec<&str> = messages.iter()
        .filter_map(|m| m.trace_id.as_deref())
        .collect();

    // 2. Get all mutations caused by these messages
    let mutations = db.get_audit_events_by_trace_ids(&trace_ids)?;

    // 3. Reverse in REVERSE chronological order (newest first)
    for mutation in mutations.iter().rev() {
        match (mutation.before_value.as_deref(), mutation.after_value.as_deref()) {
            (Some(before), Some(_)) => restore_to_before_value(mutation, before),
            (None, Some(_))         => delete_created_record(mutation),
            (Some(before), None)    => reinsert_deleted_record(mutation, before),
            (None, None)            => {} // no-op
        }
    }

    // 4. Delete the messages
    db.delete_messages_after(session_id, after_message_id)?;

    // 5. Mark audit entries as rewound
    db.mark_audit_events_rewound(&trace_ids, rewind_trace_id)?;

    // 6. Re-index affected facts
    reindex_affected_facts(&mutations)?;

    // 7. Log the rewind itself
    db.log_audit_event("rewind", summary_key, None, Some(summary_json), rewind_trace_id)?;

    Ok(summary)
}
```

## Limitations

- **Compaction boundary:** Cannot rewind past compacted messages (hard-deleted). Error if rewind point is before `compacted_through_id`.
- **Pruned audit logs:** Cannot rewind if audit entries for those trace_ids have been pruned. Error with clear message.
- **Actioned tasks:** Tasks that have been fired/completed/have children are cancelled, not deleted. Warning in preview.
- **Team runs:** Refuse partial team rewinds. Must rewind entire team run or nothing.
- **Embedding re-generation:** If OpenAI embeddings were generated for reversed facts, they need re-computation. Handle as best-effort in the re-index step.

## Constraints

- Rewind is user-initiated only — agent cannot rewind itself
- Always show preview and require confirmation
- Reverse mutations in reverse chronological order
- Preserve audit trail of the rewind itself (rewound_by_trace_id)
- Do not rewind across compaction boundaries
- Do not delete actioned tasks — cancel them instead

## Open Questions

None — all questions resolved during brainstorming.

## Implementation Phases

1. **Audit trail fix** — Add before_value to store_fact upserts, make after_value nullable, add rewound_by_trace_id (schema v9)
2. **Reversal engine** — Core `rewind_to()` function in mika-agent with preview/execute modes
3. **TUI commands** — `/undo` and `/rewind` slash commands with confirmation flow
4. **Dashboard + API** — Server endpoint + dashboard "Rewind to here" button

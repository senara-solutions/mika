---
title: "Agent creates duplicate reminders/facts after conversation compaction"
date: "2026-03-08"
module: "prompt"
severity: "medium"
tags:
  - "system-prompt"
  - "duplicates"
  - "compaction"
  - "proactive-checking"
  - "reminders"
  - "facts"
related_files:
  - "crates/mika-agent/src/prompt.rs"
  - "crates/mika-agent/src/compaction.rs"
  - "crates/mika-agent/src/tools/create_reminder.rs"
  - "crates/mika-agent/src/tools/store_fact.rs"
---

# Agent Creates Duplicate Entries After Conversation Compaction

## Problem Statement

When a user asks Mika to create a reminder (e.g., "pickup the kids at school tomorrow at 5pm"), Mika creates it successfully. If the conversation history is later compacted (or messages are deleted), and the user makes a similar request, Mika creates a second identical reminder. The agent has no memory of the prior action beyond what's in the conversation window.

This affects all write operations: reminders, facts, people, and events.

## Root Cause

The agent was never instructed to check existing state before performing write operations. After compaction, the conversation evidence of prior tool calls is replaced by a summary. The summary might mention "created a reminder" but the agent doesn't consult it — or more importantly, doesn't query the actual database state via `list_reminders` or `search_memory` before creating new entries.

**What exists at the DB level:**
- People: `UNIQUE (agent_id, canonical_name)` with upsert — prevents exact-name duplicates
- Commitments: `UNIQUE (agent_id, description)` with upsert — prevents exact duplicates
- Preferences: PK on `(agent_id, category)` with upsert — natural dedup
- Recurring reminders: Partial unique index on `(agent_id, label)` — prevents exact-label duplicates

**What has NO dedup:**
- One-shot reminders — no uniqueness constraint
- Events — plain INSERT, no constraint at all

**What was missing:** A system prompt instruction telling the agent to check before writing.

## Solution

Added one general instruction to the `## Tool Usage` section of `build_system_prompt()`:

```
Before creating or storing anything (reminders, facts, people, events),
first check existing state using the appropriate query tool (list_reminders,
search_memory). If a similar entry already exists, inform the user rather
than creating a duplicate. After compaction, conversation history may be
summarized -- always verify current state through tools rather than relying
on memory of past actions.
```

**Why a general principle, not per-tool instructions:**
- Covers all current and future write tools with one instruction
- Shorter prompt, fewer tokens per API call
- LLM applies judgment about which query tool to use
- Avoids prompt bloat as the tool surface grows

Also documented as a "Proactive state checking" convention in CLAUDE.md.

## Prevention Strategies

1. **New write tools need query counterparts.** When adding a tool that creates data, ensure there's a corresponding tool to query existing data. Document this in CLAUDE.md.

2. **Consider DB-level constraints for all tables.** The events table currently has no uniqueness constraint — this is a future hardening opportunity.

3. **Compaction should preserve action details.** The current summarization prompt preserves "key decisions, action items, commitments" but not specific tool call parameters. A future improvement could include richer tool call summaries in compacted output.

## Related

- `docs/solutions/logic-errors/agent-skips-multi-action-and-reasking-answered-questions.md` — Related prompt behavior fix
- `docs/brainstorms/2026-03-08-proactive-state-checking-brainstorm.md` — Full brainstorm with dedup matrix

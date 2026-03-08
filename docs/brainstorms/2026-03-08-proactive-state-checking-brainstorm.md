---
title: "Proactive State Checking Before Write Operations"
date: 2026-03-08
status: decided
---

# Proactive State Checking Before Write Operations

## What We're Building

A system prompt instruction that makes Mika check existing state before creating anything new. Today, Mika blindly creates reminders, facts, events, and people entries without checking if similar ones already exist. This leads to duplicates -- especially after conversation compaction removes the message history that would otherwise remind Mika it already performed an action.

### The Core Problem

1. User says "create a reminder to pickup the kids at school tomorrow at 5pm"
2. Mika creates the reminder
3. After compaction (or message deletion), the conversation evidence is gone
4. User asks again (or Mika re-processes a similar request)
5. Mika creates a second identical reminder -- now there are two

This affects all write operations: reminders, facts, events, people. The agent has query tools (`list_reminders`, `search_memory`) but is never instructed to use them before writes.

## Why This Approach

**Prompt-only solution chosen over code-level guards because:**

- Covers all current and future write tools with one instruction
- No code to maintain or update when new tools are added
- The query tools already exist -- the agent just needs to be told to use them
- DB-level constraints already handle exact-match dedup for people, commitments, and preferences
- The gap is specifically in the LLM's decision-making, not in missing infrastructure

**General principle over per-tool instructions because:**

- Shorter prompt, fewer tokens per API call
- Automatically applies to new tools
- LLM can apply judgment about which query tool to use
- Avoids prompt bloat as the tool surface grows

## Key Decisions

1. **Prompt-only implementation** -- no code-level duplicate detection in tool execute() methods. The DB schema handles exact-match dedup; the prompt handles semantic/fuzzy awareness.

2. **Single general instruction** -- one bullet point covering all write operations rather than per-tool instructions. The agent uses judgment to pick the right query tool.

3. **Include people in the check** -- even though `upsert_person()` prevents exact-name duplicates at the DB level, searching first lets the agent see existing info and make smarter updates rather than blind overwrites.

4. **Post-compaction guidance** -- explicitly tell the agent that conversation history may be summarized and it should verify state through tools, not memory of past actions.

5. **Document in CLAUDE.md** -- add a "Proactive State Checking" convention so future tool developers follow the pattern.

## The Instruction

```
Before creating or storing anything (reminders, facts, people, events), first check
existing state using the appropriate query tool (list_reminders, search_memory). If a
similar entry already exists, inform the user rather than creating a duplicate. After
compaction, conversation history may be summarized -- always verify current state through
tools rather than relying on memory of past actions.
```

## Current State of Dedup (for reference)

| Tool / Table | DB-Level Dedup | Tool-Level Pre-Check | Prompt Guidance |
|---|---|---|---|
| create_reminder (one-shot) | None | None | None |
| create_reminder (recurring) | Partial unique index on label | None | None |
| store_fact (person) | UNIQUE on canonical_name + upsert | None | None |
| store_fact (commitment) | UNIQUE on description + upsert | None | None |
| store_fact (preference) | PK on category + upsert | None | None |
| store_fact (event) | **None** | None | None |
| update_core_memory | UNIQUE on key + upsert | Reads existing value | None |

After this change, all rows get "Prompt Guidance: Yes (general principle)".

## Scope

**In scope:**
- System prompt instruction in `build_system_prompt()` Tool Usage section
- CLAUDE.md convention documenting the pattern
- Tests verifying the instruction appears in the generated prompt

**Out of scope (future work):**
- Code-level fuzzy duplicate detection in tool execute() methods
- DB-level uniqueness constraints for events table
- Semantic similarity matching for near-duplicate detection
- Tool return messages distinguishing "created new" vs "updated existing"

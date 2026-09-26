---
title: "fix: Proactive state checking before write operations"
type: fix
status: completed
date: 2026-03-08
origin: docs/brainstorms/2026-03-08-proactive-state-checking-brainstorm.md
---

# fix: Proactive state checking before write operations

## Overview

Add a system prompt instruction that makes Mika check existing state before creating reminders, facts, people, or events. Prevents duplicates that occur after conversation compaction removes evidence of prior actions. Also document this as a convention in CLAUDE.md.

## Problem Statement

Mika blindly creates entries without checking if similar ones exist. After compaction (or any loss of conversation context), the agent re-creates items, leading to duplicates. The agent has query tools (`list_reminders`, `search_memory`) but is never instructed to use them before writes.

(see brainstorm: docs/brainstorms/2026-03-08-proactive-state-checking-brainstorm.md)

## Proposed Solution

Two changes:

### Change 1: System prompt instruction

Add one bullet to the `## Tool Usage` section in `build_system_prompt()` (`crates/mika-agent/src/prompt.rs`):

```rust
prompt.push_str(
    "- Before creating or storing anything (reminders, facts, people, events), \
     first check existing state using the appropriate query tool (list_reminders, \
     search_memory). If a similar entry already exists, inform the user rather than \
     creating a duplicate. After compaction, conversation history may be summarized — \
     always verify current state through tools rather than relying on memory of past actions.\n",
);
```

Place after the existing multi-action batching instruction and before the commitments instruction.

### Change 2: CLAUDE.md convention

Add to the Conventions section:

```
- **Proactive state checking:** The system prompt instructs the agent to check existing state (via `list_reminders`, `search_memory`) before any write operation. This prevents duplicates after conversation compaction. New write tools should have a corresponding query tool.
```

### File: `crates/mika-agent/src/prompt.rs`

### File: `CLAUDE.md`

## Acceptance Criteria

- [x] System prompt includes proactive state checking instruction in `## Tool Usage` section
- [x] CLAUDE.md Conventions section documents the pattern
- [x] New test verifying the instruction appears in the generated prompt
- [x] Existing prompt tests still pass
- [x] `cargo test` passes
- [x] `cargo clippy` clean

## Sources

- **Origin brainstorm:** [docs/brainstorms/2026-03-08-proactive-state-checking-brainstorm.md](docs/brainstorms/2026-03-08-proactive-state-checking-brainstorm.md)
- `crates/mika-agent/src/prompt.rs:296-377` — Tool Usage section
- `crates/mika-agent/src/tools/create_reminder.rs` — No duplicate detection
- `crates/mika-agent/src/tools/store_fact.rs` — DB-level upsert only, no pre-check

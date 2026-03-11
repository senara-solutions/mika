---
title: Add workflows core memory block with code-enforced delegation guard
type: feat
status: completed
date: 2026-03-11
---

# Add workflows core memory block with code-enforced delegation guard

## Overview

Enforce that Mika always creates a work item before delegating work. Three layers of defense:

1. **Code guard (primary):** `delegate_task` and long-running exec handlers require a `work_item_id` parameter. Missing or invalid → error returned to agent.
2. **Core memory block:** New `workflows` block with the delegation rule as default text.
3. **System prompt instruction:** Durable fallback near delegation tool descriptions.

## Implementation

### 1. Code guard on `delegate_task` (`crates/mika-agent/src/tools/delegate_task.rs`)

Add `work_item_id` as a required string parameter to the tool's JSON schema. In `execute()`:
- Validate `work_item_id` is non-empty
- Query the DB to verify a work item with that ID exists and is in an active state (`pending`, `in_progress`, `blocked`)
- If missing or invalid: return error `"You must create a work item first using create_work_item, then pass its ID here. No delegation without tracking."`

### 2. Code guard on long-running exec handlers (`crates/mika-agent/src/tools/skill.rs` or executor)

For `ToolHandler::Exec` with `long_running: true`:
- Add `work_item_id` as a parameter in the skill tool schema when `long_running` is set
- Validate the work item exists before spawning the background process
- Same error message as `delegate_task`

### 3. Add `workflows` to `CORE_MEMORY_SECTIONS` (`crates/mika-agent/src/db.rs:59`)

```rust
("workflows", "Delegate-then-forget is not allowed. Any work sent to Claude Code must have a corresponding work item created first (via create_work_item). No exceptions."),
```

### 4. Add delegation instruction to system prompt (`crates/mika-agent/src/prompt.rs`)

Near the work item / delegation tool descriptions:

```
**Delegation Rule:** Before delegating any implementation work (via delegate_task, tmux, or long-running skills), you MUST first create a work item using create_work_item, then pass the work_item_id to the delegation tool. The tool will reject calls without a valid work_item_id.
```

### 5. Update documentation

- `CLAUDE.md` — update core memory section (5 blocks), add delegation guard to conventions
- `docs/runtime-structure.md` — update core memory listing

## Acceptance Criteria

- [x] `delegate_task` has required `work_item_id` parameter; rejects calls without valid active work item
- [x] Long-running exec handler skills require `work_item_id`; reject calls without valid active work item
- [x] `CORE_MEMORY_SECTIONS` includes `workflows` block with delegation rule
- [x] System prompt includes delegation rule instruction
- [x] Existing tests pass; new tests cover the guard (valid ID, missing ID, invalid/completed ID)
- [x] `CLAUDE.md` and `docs/runtime-structure.md` updated

## Key Files

- `crates/mika-agent/src/tools/delegate_task.rs` — add `work_item_id` param + DB validation
- `crates/mika-agent/src/tools/skill.rs` — long-running exec guard
- `crates/mika-agent/src/db.rs:59-64` — `CORE_MEMORY_SECTIONS` constant
- `crates/mika-agent/src/prompt.rs` — system prompt delegation instruction
- `crates/mika-agent/src/tools/update_core_memory.rs` — auto-adapts via `core_memory_section_names()`
- `CLAUDE.md` — project documentation
- `docs/runtime-structure.md` — runtime structure docs

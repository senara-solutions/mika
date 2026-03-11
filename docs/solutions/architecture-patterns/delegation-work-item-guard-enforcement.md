---
title: "Delegation Work Item Guard: Code-Level Enforcement Over Prompt Instructions"
date: 2026-03-11
category: architecture-patterns
severity: medium
tags:
  - delegation
  - work-items
  - code-guard
  - defense-in-depth
  - skills
  - long-running
modules:
  - crates/mika-agent/src/tools/mod.rs
  - crates/mika-agent/src/tools/delegate_task.rs
  - crates/mika-agent/src/skills/executor.rs
  - crates/mika-agent/src/skills/index.rs
  - crates/mika-agent/src/db.rs
  - crates/mika-agent/src/prompt.rs
symptoms:
  - "Agent delegates work without creating tracking records"
  - "Prompt-only enforcement is unreliable — agent ignores instructions after compaction"
  - "No audit trail for delegated implementation tasks"
root_cause: >
  The agent's delegation workflow (delegate_task tool and long-running skill
  executions) had no code-level requirement to track work. Prompt instructions
  alone proved insufficient because the LLM can ignore or forget them,
  especially after conversation compaction.
---

# Delegation Work Item Guard: Code-Level Enforcement Over Prompt Instructions

## Problem

When Mika delegates implementation work (via `delegate_task` or long-running background
skills), there was no guarantee the work would be tracked. Prompt instructions telling
the agent to "always create a work item first" were unreliable — the agent would skip
the step, especially after conversation compaction removed the instruction context.

## Key Insight

**Prompt-only enforcement doesn't work for critical workflows.** Every other safety
guard in Mika (loop prevention, callback guards, self-delegation blocks, orchestrator
checks) is enforced at the code level. Delegation tracking should follow the same
pattern.

## Solution: Three-Layer Defense

### Layer 1: Code Guard (Primary Enforcement)

Both `delegate_task` and long-running skill executions require a `work_item_id`
parameter. The tool rejects calls without a valid, active work item.

**Shared validation helper** in `tools/mod.rs`:

```rust
pub(crate) async fn validate_work_item(
    db: &AsyncDatabase,
    work_item_id: &str,
) -> Option<String> {
    // Returns Some(error_message) if invalid, None if valid
    // Checks: non-empty, exists in DB, trigger_type=manual, active status
}
```

Used by:
- `delegate_task.rs` — validates before delegation
- `executor.rs` — validates in `execute_long_running()` before spawning background process

**Schema injection** in `skills/index.rs`:

```rust
fn inject_work_item_id_field(schema: &mut serde_json::Value)
```

Called during `load_tools_json()` for `ToolHandler::Exec { long_running: true }` handlers.
Adds `work_item_id` as a required field to the tool's JSON schema at load time, so the
LLM sees it as a required parameter and includes it in tool calls.

### Layer 2: Core Memory Block (Persistent Reminder)

A `workflows` core memory block (always in system prompt) contains the delegation rule:

> "Delegate-then-forget is not allowed. Any work sent to Claude Code must have a
> corresponding work item created first (via create_work_item). No exceptions."

### Layer 3: System Prompt Instruction (Guidance)

The prompt builder includes an explicit instruction about the delegation rule and the
fact that tools will reject calls without a valid `work_item_id`.

## Design Decisions

1. **DRY validation:** Single `validate_work_item()` helper shared across delegate_task
   and executor, avoiding duplicated logic.

2. **Schema injection over manual editing:** Long-running skill schemas come from
   `tools.json` files in skill directories. Rather than requiring every skill author to
   add `work_item_id`, the system injects it automatically at load time.

3. **Validation ordering:** Cheap checks (empty strings, name validation, self-delegation)
   run before the DB query for work item validation to minimize unnecessary I/O.

4. **Active status check:** Only work items with status `pending`, `in_progress`, or
   `blocked` are accepted. Completed/cancelled items are rejected to prevent reuse.

## Pattern: Code Guards Over Prompt Instructions

This follows an established pattern in Mika's architecture:

| Guard | Enforcement Point |
|-------|-------------------|
| Loop prevention (max steps) | Agent loop counter |
| Callback turn restrictions | `is_callback_turn` flag on ToolContext |
| Self-delegation block | `delegate_task` code check |
| Orchestrator-only delegation | `is_orchestrator()` helper |
| **Delegation work item** | **`validate_work_item()` helper** |

**Rule of thumb:** If the agent ignoring an instruction would cause real harm (lost work,
no audit trail, infinite loops), enforce it in code. Use prompt instructions as
defense-in-depth, never as the sole mechanism.

## Files Changed

- `crates/mika-agent/src/tools/mod.rs` — `validate_work_item()` shared helper
- `crates/mika-agent/src/tools/delegate_task.rs` — `work_item_id` required parameter + validation
- `crates/mika-agent/src/skills/executor.rs` — work item validation in `execute_long_running()`
- `crates/mika-agent/src/skills/index.rs` — `inject_work_item_id_field()` for schema mutation
- `crates/mika-agent/src/db.rs` — `workflows` core memory block added to `CORE_MEMORY_SECTIONS`
- `crates/mika-agent/src/prompt.rs` — delegation rule instruction in system prompt
- `crates/mika-agent/src/test_utils.rs` — `create_test_work_item()` shared test helper

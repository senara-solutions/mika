---
title: UUID Validation at Tool Boundary
date: 2026-04-13
last_updated: 2026-04-18
category: best-practices
module: mika-agent/tools
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding a new tool that accepts a UUID-typed argument (task_id, task_id, parent_task_id, etc.)
  - LLM is fabricating or hallucinating UUIDs during recovery or multi-step workflows
  - Soft "not found" errors from DB lookups are not teaching the LLM what went wrong structurally
tags:
  - uuid-validation
  - tool-boundary
  - hallucination-defense
  - structural-guard
  - input-validation
---

# UUID Validation at Tool Boundary

## Context

On 2026-04-11, mika-dev (qwen3-coder) called `run_claude_pilot` with a fabricated task_id — it pattern-matched the UUID shape from prior context and fabricated the suffix `a123456789ab`. The tool accepted it (only checking `len > 36`), hit the DB, and returned a soft "Task not found" error. The LLM simply retried with the correct ID, wasting a tool step and DB query.

Prompt-level instructions ("use the exact task_id") are unreliable for this failure class (per `feedback_prompt_enforcement_fragile.md`). The project philosophy is: "If the agent ignoring an instruction would cause real harm, enforce it in the harness."

## Guidance

Use the shared `validate_uuid(field_name, value)` helper in `tools/mod.rs` for every tool argument that should be a UUID. The helper:

1. Parses the value with `uuid::Uuid::parse_str()`
2. Returns `Ok(Uuid)` on success or `Err(ToolOutput::error(...))` with a structured JSON error
3. Truncates the `received` value to 50 characters (using char boundaries for UTF-8 safety)

The structured error format:
```json
{
  "error": "invalid_uuid",
  "field": "task_id",
  "received": "eda3190e-764c-4b0f-a123456789ab",
  "reason": "string is not a well-formed UUID (expected 8-4-4-4-12 hex segments)"
}
```

Call pattern in tool `execute()` methods:
```rust
let id = input["id"].as_str().unwrap_or("").trim();
if id.is_empty() {
    return Ok(ToolOutput::error("'id' is required."));
}
if let Err(e) = super::validate_uuid("id", id) {
    return Ok(e);
}
// id is now known to be a valid UUID format — proceed to DB lookup
```

Keep the empty-string check before `validate_uuid()` — it produces a more actionable "'id' is required" error than the generic UUID format error.

For shared helpers like `validate_task()` that return `Option<String>`, extract the error content:
```rust
if let Err(tool_output) = validate_uuid("task_id", task_id) {
    return Some(tool_output.content);
}
```

## Why This Matters

- **Structural enforcement beats prompt instructions.** The LLM cannot rationalize past a tool-boundary validator. Prompt-level "always use the exact task_id" is a soft suggestion the LLM can ignore under recovery load.
- **Structured errors enable self-correction.** A plain "not found" error doesn't tell the LLM *why* the ID failed. The structured `invalid_uuid` error with field name and received value teaches the LLM the structural rule.
- **Wasted work is prevented.** DB lookups, subprocess calls, and network requests don't happen for obviously malformed inputs.
- **Defense-in-depth.** This complements #525 (dispatch-readiness guard) which catches valid-but-stale UUIDs. Together they fully close the hallucinated-task_id vulnerability.

## When to Apply

- Every tool in `crates/mika-agent/src/tools/` that accepts a `task_id`, `task_id`, `parent_task_id`, or similar UUID-typed field
- NOT for `session_id` (uses prefixed formats like `delegate-{uuid}`, `system-{agent_id}`) or `trace_id` (32-char hex, not hyphenated UUID)
- NOT for optional filter parameters where an invalid value simply returns empty results (e.g., `get_team_status` `run_id`)

## Examples

**Before (weak validation):**
```rust
if id.len() > 36 {
    return Ok(ToolOutput::error("'id' must be a valid task UUID (36 characters)."));
}
// "abc" passes this check and hits the DB
```

**After (proper UUID validation):**
```rust
if let Err(e) = super::validate_uuid("id", id) {
    return Ok(e);
}
// Only well-formed UUIDs reach the DB
```

## Three-Layer Validation Chain (#596)

The validation system follows a layered architecture. Each layer builds on the previous:

| Layer | Helper | What it checks | Returns |
|-------|--------|----------------|---------|
| **1. Format** | `validate_uuid(field, value)` | UUID is syntactically valid (8-4-4-4-12 hex) | `Result<Uuid, ToolOutput>` |
| **2. Existence** | `validate_task_exists(db, field, value)` | Format (layer 1) + task exists in DB + agent-scoped | `Result<Task, ToolOutput>` |
| **3. Business rules** | `validate_work_item(db, work_item_id)` | Existence (layer 2) + trigger_type=manual + active status | `Option<String>` |

**Layer 2: `validate_task_exists`** (added in #596) is the primary defense against fabricated UUIDs reaching tool handlers. It:
- Calls `validate_uuid()` for format checking (layer 1)
- Queries `db.get_task(value)` which is agent-scoped (`WHERE id = ?1 AND agent_id = ?2`)
- Returns structured JSON errors distinguishable from format errors:

```json
{
  "error": "task_not_found",
  "field": "task_id",
  "task_id": "00000000-0000-0000-0000-000000000000",
  "reason": "no task with this ID exists for the current agent"
}
```

Call pattern in tool `execute()` methods (replaces the old two-step pattern):
```rust
let id = input["id"].as_str().unwrap_or("").trim();
if id.is_empty() {
    return Ok(ToolOutput::error("'id' is required."));
}
let task = match super::validate_task_exists(ctx.db, "id", id).await {
    Ok(t) => t,
    Err(e) => return Ok(e),
};
// task is now a validated Task struct — proceed to business logic
```

**Design decisions:**
- Cross-agent UUIDs return the same `task_not_found` error as non-existent UUIDs (no information disclosure)
- DB errors fail closed (`db_error` structured JSON) rather than passing through
- The helper returns `Task` directly, eliminating redundant follow-up DB queries
- `cancel_task_and_kill` and other infrastructure code are NOT in scope — only tool `execute()` methods

**Layer 3: `validate_work_item`** now calls `validate_task_exists` internally, then layers trigger_type and status checks. The return type stays `Option<String>` for backward compatibility with `delegate_task` and dispatch-readiness callers.

## Related

- [Dispatch-readiness guard](../architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md) — catches valid-but-stale UUIDs (#525)
- [Fabricated action-claim guard](../architecture-patterns/fabricated-action-claim-guard.md) — structural guard philosophy
- [Team workspace hardening](../security-issues/team-workspace-ref-dir-validation-hardening.md) — precedent for `Uuid::parse_str()` at entry boundaries
- GitHub issues: [#531](https://github.com/senara-solutions/mika/issues/531) (format validation), [#596](https://github.com/senara-solutions/mika/issues/596) (existence validation)

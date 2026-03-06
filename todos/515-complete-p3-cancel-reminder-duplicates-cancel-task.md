---
status: complete
priority: p3
issue_id: "515"
tags: [code-review, simplicity, duplication]
dependencies: []
---

# `cancel_reminder` Duplicates `cancel_task` — Should Delegate Instead

## Problem Statement

`cancel_reminder.rs` and `cancel_task.rs` are structurally identical: same logic, same input schema, same validation, same DB method call, same test patterns. `cancel_reminder` exists as a backwards-compatibility alias. It should delegate to `cancel_task` rather than duplicate its implementation.

## Findings

- **Source**: simplicity-reviewer (F-1, most impactful)
- **Location**: `crates/mika-agent/src/tools/cancel_reminder.rs`, `crates/mika-agent/src/tools/cancel_task.rs`

Both files share:
- Same `execute` logic (both call `ctx.db.cancel_task(id)`)
- Same input schema (single `id` field)
- Same guard clause and error messages (modulo noun)
- Same test patterns
- Same `log_memory_event` call

The tests in `cancel_reminder.rs` even call `harness.db.cancel_task(...)` directly — confirming they test the same DB method. This is ~100 LOC of duplication for zero additional functionality.

Both tools must remain registered in `default_tools()` since the LLM knows both names.

## Proposed Solutions

### Option A: Have `cancel_reminder` delegate to `cancel_task::CancelTaskTool` (Recommended)

```rust
// cancel_reminder.rs — reduced from ~100 lines to ~30
pub struct CancelReminderTool;

#[async_trait]
impl Tool for CancelReminderTool {
    fn name(&self) -> &str { "cancel_reminder" }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "cancel_reminder".to_string(),
            description: "Cancel a pending reminder by UUID. Alias for cancel_task.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Full UUID of the reminder to cancel" }
                },
                "required": ["id"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        super::cancel_task::CancelTaskTool.execute(input, ctx).await
    }
}
```

Removes ~70 LOC of duplication. Aliasing relationship is explicit.

- **Effort**: Small | **Risk**: None (behavior identical)

## Acceptance Criteria

- [ ] `cancel_reminder.rs` delegates to `CancelTaskTool.execute()`
- [ ] Both tools remain registered in `default_tools()`
- [ ] Tests for `cancel_reminder` pass unchanged (behavior is identical)
- [ ] LOC in `cancel_reminder.rs` reduced from ~100 to ~30

## Work Log

- 2026-03-06: Identified by simplicity-reviewer of feat/unified-task-engine

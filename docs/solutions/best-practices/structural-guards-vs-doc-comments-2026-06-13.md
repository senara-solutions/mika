---
module: tools
tags: [newtype, structural-guard, ownership, compile-time-safety]
problem_type: bug-class-prevention
category: best-practices
---

# Structural guards vs doc comments

## Problem

mika#752 found a bug where `validate_task_exists` (named for existence semantics)
silently enforced agent-ownership semantics. A correlation path called it when it
should have called `db.get_task_unscoped`. The fix landed as bidirectional doc
comments cross-linking the two functions.

Doc comments are prompt-level enforcement — a future developer (human or LLM) can
ignore them and pick the wrong function for the wrong context. The next subtle bug
is one read-skim away.

## Solution

mika#755 promoted the doc-comment guard to a compile-time newtype:

```rust
pub(crate) struct AgentScopedTaskId(String);

impl AgentScopedTaskId {
    pub(crate) fn from_tool_context(_ctx: &ToolContext<'_>, raw: &str) -> Result<Self, ToolOutput> {
        validate_uuid("task_id", raw)?;
        Ok(Self(raw.to_string()))
    }
}
```

`validate_task_exists` now takes `&AgentScopedTaskId` instead of `&str`. Non-tool
paths that correctly use `db.get_task_unscoped` with raw `&str` are unaffected. A
future caller that tries to pass a raw `&str` to `validate_task_exists` gets a
compile error.

## Pattern

When doc comments are the only thing preventing a wrong choice between two functions
with the same parameter types, promote to a newtype:

1. Identify the invariant the doc comment is guarding (e.g., "this string was
   obtained in an agent-scoped context").
2. Create a newtype whose constructor requires proof of that invariant (e.g., a
   `ToolContext` in scope).
3. Change the guarded function to accept the newtype instead of the raw type.
4. Callers that hold the invariant construct the newtype; callers that don't use
   the alternative function — and the compiler enforces the split.

## References

- `feedback_prompt_enforcement_fragile.md` — institutional principle
- mika#752 — the original bug (correlation path used scoped validator)
- mika#755 — this structural fix
- `crates/mika-agent/src/tools/mod.rs` — `AgentScopedTaskId` definition

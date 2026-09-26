---
status: complete
priority: p3
issue_id: "096"
tags: [code-review, performance]
dependencies: []
---

# Cache ToolRegistry definitions at registration time

## Problem Statement
`ToolRegistry::definitions()` creates a new `Vec<ToolDefinition>` with fresh allocations every time it's called. Each `ToolDefinition` contains a `String` name, `String` description, and `serde_json::Value` schema. With 8 tools, this is 24 heap allocations per agent invocation.

## Findings
- File: `crates/mika-agent/src/tools/mod.rs:89-91`
- Called once per `run_agent` / `run_silent_agent` invocation
- Tool definitions are static after registration — never change during runtime
- Flagged by: Performance Oracle (OPT-4)

## Proposed Solutions

### Option 1: Cache at registration time (Recommended)
```rust
pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
    cached_defs: Vec<ToolDefinition>,
}
pub fn definitions(&self) -> &[ToolDefinition] {
    &self.cached_defs
}
```
Build `cached_defs` in `register()`.
**Effort:** Small
**Risk:** Low

## Technical Details
**Affected files:** `crates/mika-agent/src/tools/mod.rs`

## Acceptance Criteria
- [ ] `definitions()` returns `&[ToolDefinition]` without allocation
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v2)
**Actions:** Identified unnecessary per-call allocation of static tool definitions

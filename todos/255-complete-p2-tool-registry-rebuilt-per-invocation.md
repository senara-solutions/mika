---
status: complete
priority: p2
issue_id: 255
tags: [code-review, performance]
dependencies: []
---

# Tool Registry Rebuilt Per Agent Invocation

## Problem Statement

`run_agent()` in engine.rs rebuilds the tool registry (8 default tools + 3 workspace tools) on every agent invocation. Each invocation allocates Box'd tools with `serde_json::Value` schemas. In a typical team flow this happens N+3 times per iteration (N specialist agents + planner + synthesizer + coordinator), causing unnecessary allocations.

## Findings

- **File:** `crates/mika-agent/src/teams/engine.rs` lines 317-348
- The tool registry is constructed fresh on every call to `run_agent()`
- Each construction allocates Box'd trait objects and builds JSON schema values via serde_json
- The workspace tools use immutable `PathBuf` references, and the registry is not mutated after construction
- For a 5-agent team, this means ~8 registry constructions per iteration, each allocating 11 Box'd tools

## Proposed Solutions

Build the tool registry once in `TeamEngine::new()` and store it as a field on the struct. The workspace tools use immutable `PathBuf` and the registry is never mutated after construction, so it can be safely reused.

```rust
struct TeamEngine {
    // ... existing fields
    tool_registry: Vec<Box<dyn Tool>>,  // or appropriate type
}

impl TeamEngine {
    fn new(...) -> Self {
        let tool_registry = build_tools(workspace_path, ...);
        Self {
            // ... existing fields
            tool_registry,
        }
    }
}
```

## Technical Details

- The `Tool` trait objects need to be `Send + Sync` for reuse across async calls (they likely already are)
- If tools hold mutable state (e.g., counters), they would need interior mutability; verify this is not the case
- The JSON schemas (`serde_json::Value`) are built from static definitions and never change
- Consider whether the tool registry type needs to be `Arc`-wrapped for concurrent access (relevant to issue #253)

## Acceptance Criteria

- [ ] Tool registry is built once per `TeamEngine` instance, not per agent invocation
- [ ] No per-invocation allocation of Box'd tools or JSON schemas
- [ ] All existing tests pass
- [ ] No regression in tool behavior

## Work Log

| Date | Note |
|------|------|
| 2026-02-25 | Created from PR #13 code review |

## Resources

- PR: https://github.com/senara-solutions/mika/pull/13

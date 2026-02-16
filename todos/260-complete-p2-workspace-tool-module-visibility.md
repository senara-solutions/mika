---
status: complete
priority: p2
issue_id: 260
tags: [code-review, quality, patterns]
dependencies: []
---

# Workspace Tool Module Visibility Should Be Private

## Problem Statement

Workspace tool modules are declared `pub mod` in `tools/mod.rs` but all existing tool modules are private `mod`. The `pub` is unnecessary since the `team_tools()` factory function is in the same module and handles construction. This leaks internal types into the crate's public API, breaking the established encapsulation pattern.

## Findings

- **File:** `crates/mika-agent/src/tools/mod.rs` lines 4-5, 11
- Three workspace tool modules are declared as `pub mod`:
  - `pub mod list_workspace`
  - `pub mod read_workspace`
  - `pub mod write_workspace`
- All other tool modules in the same file use private `mod`:
  - `mod update_core_memory`
  - `mod store_fact`
  - `mod update_fact`
  - `mod search_memory`
  - `mod send_message`
  - `mod manage_reminders`
  - etc.
- The `team_tools()` function in `tools/mod.rs` constructs and returns the workspace tools, so external code never needs to reference the modules directly
- The `pub` visibility exposes internal struct types (e.g., `ListWorkspaceTool`, `ReadWorkspaceTool`, `WriteWorkspaceTool`) to other crates

## Proposed Solutions

Change the three `pub mod` declarations to `mod`:

```rust
// Before:
pub mod list_workspace;
pub mod read_workspace;
pub mod write_workspace;

// After:
mod list_workspace;
mod read_workspace;
mod write_workspace;
```

This is a minimal, low-risk change that aligns with the established pattern.

## Technical Details

- Verify no code outside `crates/mika-agent/src/tools/` directly imports from these modules
- If any external code does reference them, it should be refactored to go through the `team_tools()` factory
- This is a purely compile-time visibility change with no runtime impact

## Acceptance Criteria

- [ ] `list_workspace`, `read_workspace`, and `write_workspace` modules changed from `pub mod` to `mod`
- [ ] Code compiles with no errors (confirming no external dependencies on these modules)
- [ ] All existing tests pass
- [ ] Consistent with visibility pattern of all other tool modules

## Work Log

| Date | Note |
|------|------|
| 2026-02-25 | Created from PR #13 code review |

## Resources

- PR: https://github.com/senara-solutions/mika/pull/13

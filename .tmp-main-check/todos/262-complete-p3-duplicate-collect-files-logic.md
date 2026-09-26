---
status: complete
priority: p3
issue_id: 262
tags: [code-review, quality, simplification]
dependencies: []
---

# Remove Duplicate collect_files Logic from Prompt Builder

## Problem Statement

`collect_files_simple()` in prompt.rs duplicates the recursive directory walk from `collect_files()` in list_workspace.rs. The workspace listing is injected into the orchestrator prompt, but the orchestrator already has access to the `list_workspace` tool and can call it naturally when needed.

## Findings

- `collect_files_simple()` in prompt.rs implements a recursive directory traversal that mirrors the logic in `collect_files()` from list_workspace.rs.
- The `workspace_listing()` function in prompt.rs calls `collect_files_simple()` and formats the result for inclusion in orchestrator prompts.
- `build_orchestrator_context()` accepts a workspace_listing parameter and embeds it directly in the system prompt.
- Since the orchestrator agent already has access to the `list_workspace` tool, pre-injecting the workspace listing is redundant. The orchestrator can discover workspace contents on demand.

## Proposed Solutions

1. Remove `workspace_listing()` and `collect_files_simple()` from prompt.rs.
2. Remove the workspace_listing parameter from `build_orchestrator_context()`.
3. Let the orchestrator call `list_workspace` tool naturally when it needs to inspect the workspace.

Estimated ~25 lines saved.

## Technical Details

**Files affected:**
- `crates/mika-agent/src/teams/prompt.rs` — remove `collect_files_simple()` and `workspace_listing()` functions; remove workspace_listing parameter from `build_orchestrator_context()`
- `crates/mika-agent/src/tools/list_workspace.rs` — contains the canonical `collect_files()` implementation (no changes needed)
- Any callers of `build_orchestrator_context()` — update to remove the workspace_listing argument

## Acceptance Criteria

- [ ] `collect_files_simple()` removed from prompt.rs
- [ ] `workspace_listing()` removed from prompt.rs
- [ ] `build_orchestrator_context()` no longer accepts workspace_listing parameter
- [ ] All callers of `build_orchestrator_context()` updated
- [ ] No duplicate directory walk logic exists
- [ ] Orchestrator still functions correctly (can discover workspace via `list_workspace` tool)
- [ ] All tests pass (`cargo test`)

## Work Log

| Date | Note |
|------|------|
| 2026-02-25 | Created from code review of PR #13 |

## Resources

- PR: https://github.com/senara-solutions/mika/pull/13

---
status: pending
priority: p3
issue_id: "492"
tags: [code-review, quality, tools]
dependencies: []
---

# read_file Has Redundant canonicalize Containment Check After validate_and_resolve_path

## Problem Statement

`read_file` calls `validate_and_resolve_path` (which already does component-based traversal
rejection, parent canonicalization, and containment check), then additionally calls
`Path::canonicalize` on the full resolved path for a redundant containment check at lines
61–82. The extra check cannot catch anything that wasn't already caught by `validate_and_resolve_path`
because: (1) traversal components were already rejected, (2) the parent was already canonicalized
and verified, (3) symlinks in the parent chain were already rejected. The `Path::canonicalize`
call is also the synchronous blocking variant.

## Findings

- **Source**: code-simplicity-reviewer and performance-oracle reviews
- **Location**: `crates/mika-agent/src/tools/read_file.rs:61–82`
- Legitimate check that remains: `symlink_metadata` at line 44 (checks the file itself for symlinks,
  which `validate_and_resolve_path` does not check for the final file target)
- Redundant: the `canonicalize` block at lines 61–82

## Proposed Solutions

### Option A: Remove the canonicalize block, keep symlink_metadata (Recommended)
Remove lines 61–82. The `symlink_metadata` check at line 44 provides the necessary
defense-in-depth for the final file target. The `validate_and_resolve_path` call covers everything else.
- **Effort**: Tiny | **Risk**: None (redundant code removal)

### Option B: Replace with tokio::fs::canonicalize
If keeping the check, replace `Path::canonicalize` (sync) with `tokio::fs::canonicalize` (async).
- **Effort**: Tiny | **Risk**: None

## Acceptance Criteria

- [ ] Redundant canonicalize block removed from read_file.rs
- [ ] symlink_metadata check retained
- [ ] All existing security tests still pass
- [ ] `cargo clippy` passes with no new warnings

## Work Log

- 2026-03-06: Identified by code-simplicity-reviewer and performance-oracle reviews of feat/unified-task-engine

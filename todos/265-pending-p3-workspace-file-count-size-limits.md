---
status: pending
priority: p3
issue_id: 265
tags: [code-review, quality, performance]
dependencies: []
---

# Add Workspace File Count and Size Limits

## Problem Statement

There are no limits on workspace file count or total size. `MAX_INPUT_LEN` limits individual writes to 10,000 characters, but agents in a loop can write many files without restriction. Additionally, `collect_files` performs an unbounded recursive traversal without depth limits, risking stack overflow with deeply nested directories.

## Findings

- `write_workspace.rs` enforces `MAX_INPUT_LEN` per individual write but has no mechanism to limit the total number of files or cumulative size of the workspace.
- An agent in a tool loop could theoretically create hundreds of files, each just under the per-write limit.
- `collect_files` in `list_workspace.rs` recursively traverses directories without any depth limit. A deeply nested directory structure (whether created by the agent or pre-existing) could cause stack overflow.
- No total workspace size budget exists, so aggregate disk usage is uncontrolled.

## Proposed Solutions

1. **Add max directory depth to `collect_files`:** Limit recursive traversal to 5 levels deep. Return an indicator when the depth limit is reached so the caller knows the listing is truncated.

2. **Add max file count to workspace:** Enforce a limit of 100 files in the workspace. Check the count before allowing `write_workspace` to create new files.

3. **Consider max total workspace size:** Optionally track cumulative workspace size and enforce a budget (e.g., 1 MB total).

```rust
// collect_files with depth limit
const MAX_DEPTH: usize = 5;

fn collect_files(dir: &Path, depth: usize) -> Result<Vec<FileEntry>> {
    if depth > MAX_DEPTH {
        return Ok(vec![]); // or return truncation indicator
    }
    // ... existing logic with collect_files(subdir, depth + 1)
}
```

```rust
// write_workspace file count check
const MAX_WORKSPACE_FILES: usize = 100;

// Before creating a new file:
let existing_count = collect_files(&workspace_root, 0)?.len();
if existing_count >= MAX_WORKSPACE_FILES {
    return Ok(ToolOutput::error(
        format!("Workspace file limit reached ({MAX_WORKSPACE_FILES} files)")
    ));
}
```

## Technical Details

**Files affected:**
- `crates/mika-agent/src/tools/list_workspace.rs` — add depth parameter to `collect_files`
- `crates/mika-agent/src/tools/write_workspace.rs` — add file count check before write

**Constants to define:**
- `MAX_DEPTH: usize = 5` — maximum directory traversal depth
- `MAX_WORKSPACE_FILES: usize = 100` — maximum files in workspace

**Considerations:**
- The depth limit affects `list_workspace` tool output; the truncation should be communicated to the agent
- File count check adds a directory scan before each write; consider caching if performance is a concern
- Total size limit is optional and more complex to implement (requires summing file sizes)

## Acceptance Criteria

- [ ] `collect_files` has a maximum directory depth of 5 levels
- [ ] Truncation is indicated when depth limit is reached
- [ ] `write_workspace` enforces a maximum file count (100 files)
- [ ] Clear error message when file count limit is reached
- [ ] Tests for depth limit behavior
- [ ] Tests for file count limit behavior
- [ ] All existing tests pass (`cargo test`)

## Work Log

| Date | Note |
|------|------|
| 2026-02-25 | Created from code review of PR #13 |

## Resources

- PR: https://github.com/senara-solutions/mika/pull/13

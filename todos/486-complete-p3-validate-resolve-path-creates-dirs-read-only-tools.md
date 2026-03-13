---
status: complete
priority: p3
issue_id: "486"
tags: [code-review, tools, quality]
dependencies: []
---

# validate_and_resolve_path Creates Parent Directories in Read-Only Tools

## Problem Statement

`validate_and_resolve_path` calls `tokio::fs::create_dir_all(parent)` as part of its path
resolution. This is appropriate for write operations (`write_agent_file`, `write_workspace`) but is
an unexpected side effect for read-only tools (`read_file`, `list_files`). When an agent calls
`read_file(path: "nonexistent/deep/path/file.txt")`, the directories `nonexistent/deep/path/`
are silently created in the agent home directory even though the file doesn't exist and the call
returns an error. Read operations should not modify the filesystem.

## Findings

- **Source**: security-sentinel review
- **Location**: `crates/mika-agent/src/tools/mod.rs:233–235` (create_dir_all in validate_and_resolve_path)
- Effects: creates empty directory clutter; can confuse `list_files` results with empty dirs

## Proposed Solutions

### Option A: Add create_parents parameter to validate_and_resolve_path
```rust
pub async fn validate_and_resolve_path(
    path: &str, base_dir: &Path, create_parents: bool
) -> Result<PathBuf, ToolOutput>
```
Pass `false` from `read_file` and `list_files`, `true` from `write_agent_file`/`write_workspace`.
- **Effort**: Small | **Risk**: Low

### Option B: Create validate_read_path helper
A simpler read-only variant that omits `create_dir_all`.
- **Effort**: Small | **Risk**: None

## Acceptance Criteria

- [ ] `read_file` and `list_files` do not create directories as a side effect of path validation
- [ ] `write_agent_file` and `write_workspace` still create parent directories
- [ ] Existing tests pass

## Work Log

- 2026-03-06: Identified by security-sentinel review of feat/unified-task-engine

---
status: complete
priority: p3
issue_id: "489"
tags: [code-review, agent-native, tools]
dependencies: []
---

# list_files Output Omits File Sizes and Modification Timestamps

## Problem Statement

`list_files` output shows only file name and type (file/dir), e.g., `notes.md (file)`. The
agent cannot determine which file was most recently modified, whether a file is empty, or
whether it is worth reading before calling `read_file`. A user browsing the TUI can see file
metadata via OS tools. The agent cannot make the same informed decisions, reducing its ability
to act autonomously on file contents.

## Findings

- **Source**: agent-native-reviewer review
- **Location**: `crates/mika-agent/src/tools/list_files.rs:79–85`
- File metadata (size, mtime) is available from `entry.metadata()` during traversal

## Proposed Solutions

### Option A: Add size and relative mtime to each entry line (Recommended)
```
notes.md (file, 2.3 KB, 3 hours ago)
projects/ (dir, modified 2026-03-05)
```
Use `entry.metadata().len()` for size and `metadata().modified()` for mtime.
- **Effort**: Small | **Risk**: None

### Option B: Add optional verbosity flag
Add `verbose: bool` parameter; default output stays minimal, verbose adds metadata.
- **Effort**: Small | **Risk**: None

## Acceptance Criteria

- [ ] Each file entry includes size in human-readable form (KB, MB) and relative modification time
- [ ] Directory entries include modification time
- [ ] Metadata errors are gracefully skipped (not tool failures)

## Work Log

- 2026-03-06: Identified by agent-native-reviewer of feat/unified-task-engine

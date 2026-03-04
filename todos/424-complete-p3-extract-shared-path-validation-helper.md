---
status: complete
priority: p3
issue_id: 424
tags: [code-review, refactor, quality]
dependencies: []
---

# Extract shared path validation helper

## Problem Statement

`write_file.rs`, `write_workspace.rs`, and `read_workspace.rs` duplicate ~30 lines of identical path validation logic (empty check, length check, absolute path rejection, component traversal inspection, symlink checks, canonicalize containment). A shared `validate_path(path, base_dir)` helper would eliminate ~60 lines of duplication.

## Findings

- Pre-existing duplication — the new `write_file` correctly followed the pattern rather than introducing a refactor in a feature PR.
- Source: code-simplicity-reviewer

## Proposed Solutions

### Option A: Extract to `tools/mod.rs` helper
- **Pros:** Simple, close to usage
- **Cons:** Couples all file tools more tightly
- **Effort:** Small

## Acceptance Criteria

- [ ] Shared helper function exists
- [ ] All 3 tools use it
- [ ] Tests still pass

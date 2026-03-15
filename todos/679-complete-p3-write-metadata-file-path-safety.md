---
status: complete
priority: p3
issue_id: "679"
tags: [code-review, security, defense-in-depth]
dependencies: []
---

# write_metadata_file should validate filename has no path separators

## Problem Statement

`write_metadata_file()` in `engine.rs` bypasses `validate_and_resolve_path()` and writes directly via `std::fs::write()`. All current call sites use hardcoded string literals, but a future developer could pass a user/LLM-controlled name enabling path traversal.

## Proposed Solutions

Add a `debug_assert!` or runtime check:
```rust
debug_assert!(!name.contains('/') && !name.contains('\\') && !name.contains(".."));
```

- **Effort:** Small (one line)
- **Risk:** None

## Acceptance Criteria

- [ ] `write_metadata_file` rejects names with path separators

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-15 | Created from code review | Security sentinel finding #2 |

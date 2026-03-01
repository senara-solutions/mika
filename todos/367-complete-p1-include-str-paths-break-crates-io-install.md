---
status: complete
priority: p1
issue_id: 367
tags: [code-review, publishing, crates-io, build]
dependencies: []
---

# include_str! paths outside crate root break crates.io install

## Problem Statement

`crates/mika-agent/src/bundled_skills.rs` and `crates/mika-agent/src/skills/builtin_handlers.rs` use `include_str!` with relative paths that traverse outside the crate directory into the workspace root (`../../../templates/...`, `../../../../docs/...`). When `cargo publish` packages a crate, only files within the crate directory are included. When a downstream user runs `cargo install mika-ai`, Cargo downloads `mika-agent` from the registry and attempts to compile it — the `include_str!` macros will fail because the referenced files don't exist in the packaged crate.

This is a **publish blocker**. `cargo install mika-ai` will produce a hard compile error.

## Findings

**Files with external `include_str!` paths:**

1. `crates/mika-agent/src/bundled_skills.rs` — All bundled skills reference `../../../templates/skills/...`:
   - tmux (7 files)
   - shell-exec (4 files)
   - web-search (3 files)
   - file-reader (likely more)
   - github (likely more)

2. `crates/mika-agent/src/skills/builtin_handlers.rs`:
   - Line 13: `include_str!("../../../../docs/openapi/mika-server.yaml")`
   - Line 16: `include_str!("../../../../docs/architecture.md")`

3. `crates/mika-agent/src/server/openapi.rs`:
   - Line 80: `include_str!("../../../../docs/openapi/mika-server.yaml")`

## Proposed Solutions

### Option 1: Move templates into the crate directory
- Move `templates/skills/` to `crates/mika-agent/templates/skills/`
- Move referenced docs into the crate or copy them at build time
- Update all `include_str!` paths to be crate-relative
- **Pros:** Clean, self-contained crate; no build script needed
- **Cons:** Duplicates docs files; templates no longer at workspace root
- **Effort:** Medium
- **Risk:** Low

### Option 2: Use a build script to copy files
- Add `build.rs` to `mika-agent` that copies external files into `OUT_DIR`
- Use `include_str!(concat!(env!("OUT_DIR"), "/..."))` pattern
- **Pros:** No file duplication; workspace layout unchanged
- **Cons:** More complex build; `OUT_DIR` pattern is less idiomatic
- **Effort:** Medium
- **Risk:** Medium (build scripts can be fragile)

### Option 3: Inline the content as string literals
- For small files, embed content directly in Rust source
- **Pros:** Zero external dependencies; simplest packaging
- **Cons:** Ugly for large files; harder to maintain handler scripts
- **Effort:** Large (many files)
- **Risk:** Low

## Recommended Action

(To be decided during triage)

## Technical Details

- **Affected files:** `crates/mika-agent/src/bundled_skills.rs`, `crates/mika-agent/src/skills/builtin_handlers.rs`, `crates/mika-agent/src/server/openapi.rs`
- **Affected components:** mika-agent crate packaging, downstream mika-ai installation
- **Impact:** Complete failure of `cargo install mika-ai` from crates.io

## Acceptance Criteria

- [ ] `cargo package -p mika-agent` succeeds (after mika-common is published)
- [ ] `cargo package -p mika-ai` succeeds (after dependencies published)
- [ ] All `include_str!` paths resolve within the packaged crate
- [ ] Bundled skills and builtin handlers still work correctly after change

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-01 | Created from code review of commit 2eca502 | Security sentinel identified compile-time path resolution issue |

## Resources

- Commit: 2eca502 "Prepare crates for publishing to crates.io"
- [Cargo packaging docs](https://doc.rust-lang.org/cargo/commands/cargo-package.html)

---
status: complete
priority: p3
issue_id: 371
tags: [code-review, publishing, crates-io]
dependencies: []
---

# Add rust-version field for crates.io users

## Problem Statement

None of the four crates set `rust-version`. The project uses edition 2024 which requires Rust 1.85+. Without this field, users on older toolchains get confusing compile errors instead of a clear "minimum Rust version required" message from Cargo.

## Proposed Solutions

### Option 1: Add rust-version to workspace.package
- Add `rust-version = "1.85"` (minimum for edition 2024) to `[workspace.package]` in root `Cargo.toml`. Note: project currently uses Rust 1.93 but 1.85 is the minimum required.
- Each crate inherits via `rust-version.workspace = true`
- **Effort:** Small
- **Risk:** Low

## Technical Details

- **Affected files:** `Cargo.toml`, all 4 crate Cargo.toml files

## Acceptance Criteria

- [ ] `rust-version` set in workspace and inherited by all crates
- [ ] `cargo install mika-ai` on Rust < 1.85 shows clear version error

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-01 | Created from code review of commit 2eca502 | Architecture strategist recommended this |

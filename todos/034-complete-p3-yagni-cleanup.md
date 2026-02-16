---
status: complete
priority: p3
issue_id: "034"
tags: [code-review, simplicity, rust-v2]
dependencies: []
---

# YAGNI Cleanup: Remove Empty Routing Crate and Unused Code

## Problem Statement

Several items are scaffolded for future features but have zero consumers:
- `mika-routing` crate: 3 lines of content, 12 dependencies (axum, sqlx, tower-http)
- `mika-common/src/types.rs`: InboundMessage, OutboundMessage, TypingRequest — no importers
- `Settings.openai_api_key`: no OpenAI code exists
- `ToolContext.customer_id` and `routing_url`: no tool reads them
- `schedules` table: zero code paths
- `events` table: `add_event` method exists but no tool is wired

**Why it matters:** Empty routing crate adds ~100 transitive deps to Cargo.lock and slows builds. Unused code creates false signals about project maturity.

## Findings

- **Source:** Code Simplicity Reviewer (findings 1, 2, 5, 8, 9, 10)
- **Locations:** Multiple files across workspace

## Proposed Solutions

### Option A: Remove all YAGNI items (Recommended)
- Delete `crates/mika-routing/` entirely, remove from workspace members
- Delete `types.rs`, remove from lib.rs
- Remove `openai_api_key` from Settings
- Simplify `ToolContext` to just `{ db: &Database }`
- Remove `schedules` table from migration
- Either wire `events` to a tool or remove the table
- Remove `axum`, `sqlx`, `tower-http` from workspace deps
- Remove `async-trait` from mika-common Cargo.toml (unused there)
- **Pros:** Faster builds, cleaner codebase, no false signals
- **Cons:** Need to re-add when features are implemented
- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] No empty crate skeletons in workspace
- [ ] No unused dependencies in Cargo.toml
- [ ] All database tables have at least one tool or code path
- [ ] ToolContext only contains fields that tools read
- [ ] `cargo build` and `cargo test` still pass

---
status: complete
priority: p2
issue_id: "238"
tags: [code-review, architecture, duplication]
dependencies: []
---

# Duplicated `make_embedding_client` function

## Problem Statement

`make_embedding_client` is defined identically in two crates: `server/mod.rs` (takes `&Settings`) and `chat.rs` (takes `&AppContext`). Both extract the OpenAI API key, filter empties, and create an `EmbeddingClient`. This violates DRY.

## Findings

- **Source:** Architecture Strategist, Pattern Recognition, Code Simplicity
- **Files:** `crates/mika-agent/src/server/mod.rs:50-63`, `crates/mika-cli/src/commands/chat.rs:32-45`

## Proposed Solutions

### Option A: Method on Settings [Recommended]
Add `Settings::make_embedding_client(&self) -> Option<EmbeddingClient>` in `mika-common/src/config.rs`.

- **Pros:** Single source of truth, both callers have access to Settings
- **Cons:** Adds dependency on EmbeddingClient to mika-common (already exists)
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] Single `make_embedding_client` function exists in `mika-common`
- [ ] Both server and CLI call the shared version

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-25 | Created from PR #12 code review | Found by 3 agents independently |

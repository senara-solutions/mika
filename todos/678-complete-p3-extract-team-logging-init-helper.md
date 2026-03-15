---
status: complete
priority: p3
issue_id: "678"
tags: [code-review, quality, dry]
dependencies: []
---

# Extract duplicated OTel/logging setup in main.rs

## Problem Statement

The Chat and Ask team-mode branches in `main.rs` (lines ~45-60 and ~66-80) contain identical 15-line blocks for loading settings, initializing OTel, and configuring logging. This violates DRY and will drift over time.

## Findings

- **Architecture Strategist**, **Performance Oracle**, **Code Simplicity**: All flagged this duplication independently. ~12 lines saved by extraction.

**Affected files:**
- `crates/mika-cli/src/main.rs`

## Proposed Solutions

### Option A: Extract helper function (Recommended)
Create `init_team_logging(global_home: &Path, team_name: &str) -> (LogGuard, Option<TelemetryGuard>)`.
- **Effort:** Small (~15 min)
- **Risk:** None

## Acceptance Criteria

- [ ] No duplicated OTel/logging blocks in main.rs
- [ ] Helper function used by both Chat and Ask branches

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-15 | Created from code review | 3 agents flagged independently |

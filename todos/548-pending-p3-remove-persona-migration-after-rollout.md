---
status: pending
priority: p3
issue_id: "548"
tags: [code-review, tech-debt, migration]
dependencies: []
---

# Add TODO to remove migrate_persona_to_self_model after rollout

## Problem Statement

`migrate_persona_to_self_model` runs on every startup as a no-op UPDATE (0 rows matched) after the first successful migration. It should be removed after all users have migrated to avoid accumulating perpetual no-op migrations.

## Findings

- **Source:** Performance oracle + code simplicity review agents
- **Location:** `crates/mika-agent/src/db.rs` line 1478, `crates/mika-agent/src/startup.rs` line 15

## Proposed Solutions

Add a `// TODO(2026-06): remove after all users have migrated from persona -> self_model` comment.

## Acceptance Criteria

- [ ] TODO comment added with a target date for removal

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-07 | Created from code review | Transitional migration, zero performance impact but should be cleaned up |

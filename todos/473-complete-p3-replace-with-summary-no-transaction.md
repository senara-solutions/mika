---
status: complete
priority: p3
issue_id: "473"
tags: [code-review, correctness, database, compaction]
dependencies: []
---

# 473 · `replace_with_summary` executes 3 SQL statements without a transaction

## Problem Statement

`replace_with_summary` runs DELETE (old messages), DELETE (old summary), INSERT
(new summary) as three separate non-transactional statements. A crash between
statements 1 and 3 permanently deletes conversations without any summary
replacement — unrecoverable memory loss.

## Findings

- **Location:** `crates/mika-agent/src/db.rs:1059–1083`
- The individual statements are not wrapped in `BEGIN`/`COMMIT`
- Previously flagged as a pattern: todo #099 (TOCTOU in compaction)

## Proposed Solutions

### Option A — Wrap in a transaction
```rust
self.conn.execute_batch("BEGIN")?;
// ... three statements ...
self.conn.execute_batch("COMMIT")?;
```
Or use `self.conn.execute_batch("BEGIN; DELETE ...; DELETE ...; INSERT ...; COMMIT;")?`

**Effort:** Small | **Risk:** Low

## Acceptance Criteria

- [ ] All three statements wrapped in a single transaction
- [ ] Test: verify that a partial execution leaves the DB in a consistent state (mock crash between deletes)

## Work Log

- 2026-03-06: Identified by security review agent (COR-11)

---
status: pending
priority: p3
issue_id: "475"
tags: [code-review, correctness, database]
dependencies: []
---

# 475 · `ConversationMessage.created_at` typed as `String` but schema stores `INTEGER`

## Problem Statement

The `conversations` table schema (v1) stores `created_at` as
`INTEGER NOT NULL DEFAULT (unixepoch())`. The `ConversationMessage` struct
maps `created_at` as `String`. SQLite will coerce the integer to a string
on read, but any code that formats or parses `created_at` as a date string
will receive a raw Unix epoch integer string (e.g. `"1741234567"`) rather
than an ISO 8601 datetime. This is a latent bug waiting for date-formatting
code to break.

## Findings

- **Location:** `crates/mika-agent/src/db.rs` — `ConversationMessage` struct and `conversations` CREATE TABLE
- Pattern: previously fixed for reminders (todo for datetime mismatch, docs/solutions/database-issues/sqlite-datetime-format-mismatch.md)

## Proposed Solutions

### Option A — Change struct field to `i64` (recommended)
```rust
pub created_at: i64,  // Unix timestamp
```
Update all callsites that format `created_at` to use `chrono::DateTime::from_timestamp(created_at, 0)`.

**Effort:** Small | **Risk:** Low

### Option B — Add `strftime` to the SQL query
```sql
SELECT strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch') as created_at, ...
```
Keeps the `String` type in the struct.

**Effort:** Small | **Risk:** Low

## Recommended Action

Option A — store as `i64`, format at the presentation layer.

## Technical Details

- **Affected files:** `crates/mika-agent/src/db.rs`

## Acceptance Criteria

- [ ] `ConversationMessage.created_at` is `i64`
- [ ] All references to `created_at` as a date string are updated

## Work Log

- 2026-03-06: Identified by architecture review agent (ARCH-13)

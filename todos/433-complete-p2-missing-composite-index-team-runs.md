---
status: complete
priority: p2
issue_id: "433"
tags: [code-review, performance, database]
dependencies: []
---

# Missing Composite Index on team_runs(team_name, started_at)

## Problem Statement

The `load_team_runs` query filters by `team_name` and orders by `started_at DESC`:

```sql
SELECT ... FROM team_runs WHERE team_name = ?1 ORDER BY started_at DESC LIMIT ?2
```

The only index is `idx_team_runs_started ON team_runs(started_at)`, which does not cover the `WHERE team_name` clause. SQLite performs a full table scan filtered by `team_name`.

## Findings

- Performance agent identified this as the most impactful query performance issue
- For long-lived teams with hundreds of runs, this degrades to O(n) per query
- The existing `idx_team_runs_started` on just `started_at` is unhelpful for this query

## Proposed Solutions

### Option A: Replace with composite index (Recommended)

Add migration v12 (or amend v11 if not shipped):

```sql
DROP INDEX IF EXISTS idx_team_runs_started;
CREATE INDEX IF NOT EXISTS idx_team_runs_team_started ON team_runs(team_name, started_at);
```

- **Pros:** Covers both the equality filter and ORDER BY in a single index scan
- **Cons:** Requires a new migration version
- **Effort:** Small
- **Risk:** None (additive, index replacement)

## Technical Details

- **File:** `crates/mika-agent/src/db.rs`, migration v11 (line ~522)
- **Components:** Team persistence layer

## Acceptance Criteria

- [ ] Composite index `(team_name, started_at)` replaces the `started_at`-only index
- [ ] `load_team_runs` query uses the new index (verify with EXPLAIN QUERY PLAN)

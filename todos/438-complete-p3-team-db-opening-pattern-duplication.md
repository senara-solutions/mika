---
status: pending
priority: p3
issue_id: "438"
tags: [code-review, duplication, refactor]
dependencies: []
---

# Team DB Opening Pattern Repeated 4 Times

## Problem Statement

The team DB opening pattern (check dir exists, build path, open Database, wrap in AsyncDatabase, handle errors) is duplicated in 4 locations:

1. `tools/get_team_history.rs`
2. `tools/get_team_status.rs`
3. `tools/run_team.rs`
4. `commands/teams.rs`

## Proposed Solutions

### Option A: Extract shared helper (Recommended)

```rust
pub fn open_team_db(home_dir: &Path, team_name: &str) -> Result<AsyncDatabase, String> { ... }
```

- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] Shared helper extracted
- [ ] All 4 call sites updated
- [ ] Consistent error messages

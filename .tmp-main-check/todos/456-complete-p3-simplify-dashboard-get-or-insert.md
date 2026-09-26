---
status: complete
priority: p3
issue_id: "456"
tags: [code-review, quality]
dependencies: []
---

# Simplify Dashboard State Creation with get_or_insert_with

## Problem Statement

The `PhaseChanged` handler creates a `TeamDashboardState` in the else branch, then immediately re-borrows it with `if let Some`. Can be simplified to `get_or_insert_with`.

## Proposed Fix

```rust
let dash = self.team_dashboard.get_or_insert_with(TeamDashboardState::new);
dash.phase = Some(phase);
dash.iteration = iteration;
dash.agents.clear();
```

- **Location**: `crates/mika-cli/src/tui/app.rs:663-679`
- **Effort**: Small (6 lines removed)

## Acceptance Criteria

- [ ] Dashboard creation uses `get_or_insert_with`
- [ ] No redundant match/borrow

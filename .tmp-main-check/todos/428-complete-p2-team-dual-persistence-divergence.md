---
status: complete
priority: p2
issue_id: 428
tags: [code-review, architecture, team-mode, agent-native]
dependencies: []
---

# Team history diverges between TUI (SQLite) and agent tools (TOML)

## Problem Statement

Team conversation persistence uses two parallel storage mechanisms:
- **TUI team mode** (`mika --team <name>`): Saves user goals and deliverables to `~/.mika/teams/{name}/data/mika.db`
- **Agent tools** (`run_team`, `get_team_history`): Writes/reads TOML history files at `~/.mika/teams/{name}/history/*.toml`

This means the agent's view of team history diverges from the TUI's view over time. A user who sends goals via TUI won't see those interactions when querying via `get_team_history`, and vice versa.

## Findings

- Source: agent-native-reviewer
- The `run_team` tool (`crates/mika-agent/src/tools/run_team.rs`) passes `None` for progress callback
- The `get_team_history` tool reads from TOML files only
- The TUI team mode reads from SQLite DB first, falls back to TOML
- 5/7 team capabilities are fully agent-accessible; 2/7 are TUI-only (progress streaming, persistent conversations)

## Proposed Solutions

### Option A: Have agent tools also read from team DB
- `get_team_history` checks both TOML and SQLite, merges results
- **Pros:** Unified view of team history
- **Effort:** Medium
- **Risk:** Low

### Option B: Have TUI team mode also write TOML history
- After each team run completes, write the TOML history file
- **Pros:** Backward-compatible, agent tools work unchanged
- **Effort:** Small (write TOML on deliverable receipt)
- **Risk:** Low

### Option C: Consolidate on SQLite only (long-term)
- Migrate all team persistence to SQLite
- **Pros:** Single source of truth
- **Effort:** Large (requires updating TeamEngine, all tool reads)
- **Risk:** Medium (migration needed)

## Acceptance Criteria

- [ ] Agent tools and TUI team mode see the same team history
- [ ] No data loss when switching between interaction modes

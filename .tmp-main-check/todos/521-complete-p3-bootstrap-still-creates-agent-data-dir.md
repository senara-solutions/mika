---
status: complete
priority: p3
issue_id: 521
tags: [code-review, cleanup, database]
dependencies: []
---

# bootstrap() Still Creates Per-Agent data/ Directory

## Problem Statement

`home::bootstrap()` at `crates/mika-common/src/home.rs:175` still creates a `data/` directory under each agent's home directory (e.g. `~/.mika/agents/main/data/`). Since the DB consolidation, this directory is never used — the database lives at `~/.mika/data/mika.db`.

The empty directory is harmless but confusing for users inspecting the file layout.

## Findings

- `crates/mika-common/src/home.rs:175` — `std::fs::create_dir_all(home_dir.join("data"))`
- Called by `bootstrap_agent()` for every new agent
- Test at line 425 asserts `agent.join("data").is_dir()`

## Proposed Solutions

### Option A: Remove data/ from bootstrap()
- Remove `create_dir_all(home_dir.join("data"))` from bootstrap
- Update test assertion
- **Effort:** Small
- **Risk:** None

### Option B: Leave as-is
- Harmless empty directory
- May be used in future for per-agent caches
- **Effort:** None

## Acceptance Criteria

- [ ] Per-agent data/ directory either removed from bootstrap or documented as intentional

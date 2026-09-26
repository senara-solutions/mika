---
status: pending
priority: p1
issue_id: "463"
tags: [code-review, correctness, multi-agent, startup]
dependencies: []
---

# 463 · `startup.rs` hardcodes `"main"` agent ID for core memory seeding

## Problem Statement

`seed_core_memory_if_empty` calls `db.get_all_core_memory("main")` and
`db.seed_core_memory("main", ...)` with a hardcoded `"main"` agent ID.
In multi-agent mode, the server calls `init_agent` for each named agent
(e.g., "work", "code"). Core memory for every non-main agent is seeded under
the `"main"` key, leaving those agents with empty core memory at startup.

## Findings

- **Location:** `crates/mika-agent/src/startup.rs:14, 20`
- Function receives a `&Database` (sync), not `AsyncDatabase`, so the `agent_id` is not injected automatically
- The function is called from `init_agent` where the agent name is known — it just is not passed down

## Proposed Solutions

### Option A — Pass agent_name as parameter (recommended)
```rust
pub fn seed_core_memory_if_empty(db: &Database, home_dir: &Path, agent_name: &str) -> Result<()>
```
Update the two hardcoded `"main"` references to use `agent_name`.

**Effort:** Small | **Risk:** Low

### Option B — Switch signature to `&AsyncDatabase`
`AsyncDatabase` carries `agent_id` internally — no parameter needed.
**Cons:** Requires `async`, changes all callers.
**Effort:** Medium | **Risk:** Medium

## Recommended Action

Option A.

## Technical Details

- **Affected files:** `crates/mika-agent/src/startup.rs`, all callers of `seed_core_memory_if_empty`

## Acceptance Criteria

- [ ] `seed_core_memory_if_empty` accepts `agent_name: &str`
- [ ] All callers updated to pass the agent name
- [ ] Test: seed core memory for agent "work", assert `get_all_core_memory("work")` returns seeded values and `get_all_core_memory("main")` returns empty

## Work Log

- 2026-03-06: Identified by architecture review agent (ARCH-14)

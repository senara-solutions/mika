---
title: "Move skill enabled state from filesystem marker to DB with registry eviction"
category: architecture-patterns
date: 2026-04-18
tags: [skills, overrides, migrations, database, registry, eviction, tri-state]
pr: senara-solutions/mika#629
issue: senara-solutions/mika#629
---

# Move skill enabled state from filesystem marker to DB with registry eviction

## Problem

`mika skills disable <name>` wrote a `.disabled` marker file in the skill
directory. The engine scanned the marker during `scan_skills_dir()` and set
`SkillEntry.enabled = false`, but disabled skills still loaded into
`SkillRegistry.entries`. They were only filtered at match time
(`matcher.rs:43-46`) and in `always_on_skills()`. Consequences:

- Disabled skills consumed memory
- Their prompts counted against the prompt budget (when `prompt_snippet`
  was populated)
- The "skills loaded" log conflated disabled and active skills
- `disable` didn't actually reduce the agent's surface — it was closer to
  a hint than a state change

Four logical states existed (Installed, Disabled, Loaded, Skipped); the
code modeled roughly two.

## Root cause

Two conflations:

1. **State location.** `.disabled` was a filesystem marker co-located with
   skill source. But `skill_overrides` already existed in SQLite as the
   "operator overrides" layer for `always_on` and per-skill LLM. Enabled
   state logically belonged in that same layer.
2. **State semantics.** "Enabled" was a `bool` on `SkillEntry` — so a
   disabled skill was an entry with one flag flipped, not an absence.
   Every downstream consumer had to remember to check the flag.

## Solution

Make "disabled" mean "not in the registry." Move the state to a
nullable `enabled` column on `skill_overrides`, evict disabled skills out
of `SkillRegistry.entries` during `apply_overrides()`, and migrate any
existing `.disabled` markers into DB rows at startup.

### Tri-state nullable column

```sql
ALTER TABLE skill_overrides ADD COLUMN enabled INTEGER;
-- NULL = default (enabled), 0 = explicitly disabled, 1 = explicitly enabled
```

Follows the same shape as `always_on`. `NULL` is the zero-cost default —
no row needed just to say "this skill is in the normal state."

### Default-equals-delete rule

When every override column on a row becomes `NULL`, delete the row.
Extends the pattern already used by `delete_skill_llm_override()`.
`set_skill_enabled(agent, name, true)` on a row where `always_on` and
all LLM override columns are `NULL` removes the row entirely. Prevents
`skill_overrides` from accumulating no-op rows as operators toggle skills
on and off.

### Eviction in `apply_overrides()` (stage-then-extend)

The registry's `entries: Vec<SkillEntry>` can't be mutated with `retain()`
while simultaneously pushing evicted entries into `self.disabled` — the
closure can't borrow `self.disabled` while `self.entries` is mutably
borrowed. Stage into a local vec, then extend:

```rust
let mut staging: Vec<DisabledSkill> = Vec::new();
self.entries.retain(|e| {
    if disabled_names.contains(&e.name) {
        staging.push(DisabledSkill::from_entry(e, "operator override"));
        false
    } else {
        true
    }
});
self.disabled.extend(staging);
```

Eviction runs **before** `always_on` and LLM override application, so
`enabled=false` always wins over `always_on=true` — "disabled" means
"removed from consideration," not "removed unless forced on."

### One-shot migration of `.disabled` markers

At startup, before `scan_skills_dir()`, walk the skills directory and
for each `.disabled` marker: write `skill_overrides.enabled = 0`, then
remove the marker file. Idempotent (no markers left on subsequent runs).
Fail-open on marker removal — write the DB row, log a warning, continue.
The belt-and-suspenders match-time filter (kept until the follow-up
cleanup in #630) handles any dual-state edge.

### Belt-and-suspenders: keep the match-time filter in this PR

Don't delete `matcher.rs:43-46` in the same PR that introduces eviction.
During rollout, both mechanisms run — if eviction has a bug, the filter
still hides disabled skills. Filter removal ships separately after the
eviction path has baked.

## Why it works

- **One source of truth.** `skill_overrides.enabled` is the authoritative
  state; both the agent tool and CLI write it the same way.
- **Registry integrity.** After `apply_overrides()`, `entries` contains
  exactly the active skills. Downstream code (matcher, prompt assembly,
  logging) sees an accurate count without needing to re-check flags.
- **Explicit state model.** `SkillRegistry` now exposes `skills()`,
  `skipped()`, and `disabled()` — the four logical states (Installed,
  Disabled, Loaded, Skipped) collapse cleanly onto three collections
  plus filesystem presence.
- **Zero-cost default.** Tri-state nullable means skills in their normal
  state occupy zero rows. Operators only pay storage for explicit
  overrides.

## Traps and guardrails

### The `migrate_v1` clean-slate trap

`migrate_v1` recreates `skill_overrides` from scratch on fresh DBs. A
new column added via `migrate_vN_to_vN+1` **must also be added to
`migrate_v1`**. Fresh installs go through `migrate_v1` and skip the
incremental migration — if `migrate_v1` is stale, fresh DBs miss the
column and tests on in-memory DBs fail cryptically.

This is the same trap documented in
`skill-llm-override-db-layer-and-linked-unblock.md`. When adding a
column to `skill_overrides`, update both migrations in the same commit
and grep for any other `CREATE TABLE skill_overrides` in the codebase.

### CLI DB access is already solved

The 2026-03-09 skill plan deferred CLI DB writes with the rationale
"CLI has no DB access." That constraint no longer holds — `skills.rs`
opens the DB at line 129 for LLM overrides. The enable/disable path
reuses the same connection. SQLite WAL mode handles concurrent CLI +
agent readers/writers; use a short busy timeout on the CLI connection.

### Cross-process reload is not in scope

CLI writes to DB while the server is running — the server won't see
the change until the next `skills_dirty` trigger or restart. This
matches prior `.disabled` file behavior (the server didn't watch the
file either) and is documented as a known limitation. Cross-process
`skills_dirty` propagation is a separate change.

### Bundled vs community skills

Bundled skills (seeded by `seed_bundled_skills()` into
`~/.mika/agents/<name>/skills/bundled/`) have filesystem directories
like community skills. The `toggle_skill` tool's `skill_dir.exists()`
check still applies. No special-casing needed — bundled skills use the
same DB override path.

## Test coverage (the ones that actually catch bugs)

- Round-trip: disable → enable returns to clean state (row deleted,
  not just `enabled=NULL`)
- Precedence: `always_on=true` + `enabled=false` → evicted
- Preservation: `enabled=false` on a row with LLM override → both
  columns intact, row not deleted
- Migration: `.disabled` marker → DB row created, marker removed;
  second run is a no-op
- Fresh DB: `migrate_v1` path includes `enabled` column (not just the
  incremental migration)
- Eviction count: `disabled().len()` matches the number of
  `enabled=false` overrides after `apply_overrides()`

## When to reach for this pattern

Reach for this when:

- Operator-controlled state currently lives as a filesystem marker
  co-located with source (a smell — state and source have different
  write patterns)
- Downstream code has to remember to filter a boolean flag on every
  use (a smell — absence is a cleaner representation than presence
  with a flag)
- You already have an overrides table that logically owns this kind
  of state

Don't reach for this when the filesystem marker is the source of truth
for a filesystem-scoped concept (`.gitignore`, lock files). Here the
state was fundamentally about "what this operator wants right now" —
which is exactly what `skill_overrides` already stored.

## References

- Issue: senara-solutions/mika#629
- Follow-up (match-time filter removal): senara-solutions/mika#630
- Plan: `docs/plans/2026-04-18-001-feat-skill-enabled-state-db-eviction-plan.md`
- Related: `docs/solutions/architecture-patterns/skill-llm-override-db-layer-and-linked-unblock.md`
- Related: `docs/solutions/database-issues/skill-override-persistence-via-db-layer.md`
- Related: `docs/solutions/best-practices/agent-tool-must-call-apply-load-safety-check-on-skill-registry.md`

---
title: "Skill override persistence via DB layer"
category: database-issues
date: 2026-03-09
tags: [skills, sqlite, migration, seed-data, override, always_on]
modules: [db, skills, update_skill, list_skills, delete_skill]
issue: "#73"
---

# Skill override persistence via DB layer

## Problem

`seed_bundled_skills()` overwrites `skill.toml` on every startup, resetting any user-modified `always_on` value. Users who changed a built-in skill's `always_on` flag via `update_skill` would lose their preference on every restart.

## Root Cause

The skill system used a single source of truth (`skill.toml` on disk) for both bundled defaults and user preferences. Since `seed_bundled_skills()` must overwrite files to propagate template updates (security patches to handler scripts, etc.), user edits to `skill.toml` are inherently non-persistent for built-in skills.

## Solution

Introduced a `skill_overrides` SQLite table (schema v7) as a separate override layer:

```sql
CREATE TABLE skill_overrides (
    agent_id   TEXT NOT NULL COLLATE NOCASE,
    skill_name TEXT NOT NULL COLLATE NOCASE,
    always_on  INTEGER,  -- NULL=use default, 0=off, 1=on
    PRIMARY KEY (agent_id, skill_name)
);
```

**Key design decisions:**

1. **Post-scan overlay pattern:** `SkillRegistry::apply_overrides()` mutates entries in-place after `scan_skills_dir()` loads manifests from disk. This avoids modifying the `scan_skills_dir()` signature (called from ~8 sites).

2. **Built-in detection routing:** `update_skill` checks `is_bundled_skill(name)` — built-in skills write `always_on` to DB via UPSERT, custom/marketplace skills write to `skill.toml` as before.

3. **Default-equals-delete (D4):** Setting `always_on` to the bundled default deletes the override row. This prevents stale overrides from blocking future bundled default changes.

4. **`enabled` stays file-based:** The `.disabled` marker file already survives `seed_bundled_skills()` (it never removes `.disabled` files). No need to add `enabled` to the DB.

## Key Files

- `crates/mika-agent/src/db.rs` — migration v6→v7, CRUD methods
- `crates/mika-agent/src/skills/mod.rs` — `SkillRegistry::apply_overrides()`
- `crates/mika-agent/src/tools/update_skill.rs` — built-in detection + DB routing
- `crates/mika-agent/src/tools/list_skills.rs` — `[override]` badge
- `crates/mika-agent/src/tools/delete_skill.rs` — override cleanup

## Gotchas

- `teams/engine.rs` uses sync `db.get_skill_overrides()` (not async) because `init_resources()` is a sync function. The call must happen before wrapping `Database` into `AsyncDatabase`.
- Adding `has_override: bool` to `SkillEntry` requires updating all test helper functions that construct `SkillEntry` instances.
- `COLLATE NOCASE` on both PK columns ensures case-insensitive matching consistent with the rest of the schema.

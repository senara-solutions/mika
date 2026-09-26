---
title: "feat: Move skill enabled state from .disabled marker to DB with eviction"
type: feat
status: active
date: 2026-04-18
---

# Move skill enabled state from .disabled marker to DB with eviction

## Overview

Move the skill enabled/disabled state from `.disabled` marker files to the `skill_overrides` DB table. Disabled skills are evicted from `SkillRegistry.entries` during `apply_overrides()` instead of being filtered at match time. Existing `.disabled` markers are migrated to DB rows on first startup.

## Problem Frame

`mika skills disable <name>` creates a `.disabled` marker file. The engine reads it during `scan_skills_dir()` and stores `enabled: bool` on `SkillEntry`. But disabled skills still load into `SkillRegistry.entries` — they're only filtered at match time (`matcher.rs:43-46`) and in `always_on_skills()`. This means disabled skills consume memory, their prompts count against budget, the "skills loaded" count conflates disabled and active skills, and `disable` doesn't actually reduce the agent's surface.

## Requirements Trace

- R1. `mika skills disable foo` writes to DB, not filesystem
- R2. Disabled skills don't appear in `SkillRegistry.entries` or the "skills loaded" count
- R3. `/skills` CLI output shows disabled skills in a separate section with accurate count
- R4. Existing `.disabled` markers are migrated on first startup and markers are removed
- R5. DB override survives agent restart
- R6. Tests: migration idempotency, eviction, override survival, enable/disable round-trip
- R7. Match-time filter kept as belt-and-suspenders (removal deferred to #630)

## Scope Boundaries

- Does NOT delete match-time filter in `matcher.rs:43-46` — that's #630
- Does NOT change overflow behavior, log format, or skill prompt budget
- Does NOT add cross-process notification for CLI→server skill reload

### Deferred to Separate Tasks

- Match-time filter removal: #630 (P2)
- Cross-process `skills_dirty` propagation: future iteration

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/db.rs` — `skill_overrides` table (schema v7, extended v20), `SkillOverride` struct (line ~419), `get_skill_overrides()`, `set_skill_override()`, `delete_skill_override()`, default-equals-delete in `delete_skill_llm_override()`
- `crates/mika-agent/src/skills/mod.rs` — `SkillRegistry` struct, `apply_overrides()`, `skipped()` accessor, `from_dir()`
- `crates/mika-agent/src/skills/index.rs` — `SkillEntry.enabled` (line ~543: `!path.join(".disabled").exists()`), `scan_skills_dir()`
- `crates/mika-agent/src/skills/matcher.rs` — match-time filter at lines 43-46
- `crates/mika-agent/src/tools/toggle_skill.rs` — agent tool, creates/removes `.disabled` marker, sets `skills_dirty`
- `crates/mika-cli/src/commands/skills.rs` — CLI `toggle_skill()` (line ~657), already opens DB at line 129 for LLM overrides
- `crates/mika-agent/src/tools/delete_skill.rs` — calls `delete_skill_override()` for full row cleanup

### Institutional Learnings

- **skill-override-persistence-via-db-layer** (docs/solutions/): Documents how `always_on` was moved to `skill_overrides` via post-scan overlay pattern. Original rationale kept `enabled` file-based — this issue intentionally reverses that decision.
- **skill-llm-override-db-layer** (docs/solutions/): Documents the **migrate_v1 trap** — clean-slate migration must include new columns or fresh DB tests fail.
- **agent-tool-must-call-validate-loaded** (docs/solutions/): All `SkillRegistry` construction sites must follow `from_dir → apply_overrides → validate_loaded` sequence.

## Key Technical Decisions

- **Tri-state nullable column:** `enabled INTEGER` — `NULL` = default (enabled), `0` = disabled, `1` = explicitly enabled. Follows `always_on` pattern exactly.
- **Default-equals-delete:** When `enabled` is set back to default (NULL/true) and all other override columns are also NULL, delete the row. Extends existing pattern in `delete_skill_llm_override()`.
- **Eviction into `disabled` vec:** `apply_overrides()` moves disabled skills from `entries` into a new `disabled: Vec<DisabledSkill>` field using a staging vec (can't borrow `self.disabled` while `self.entries` is mutably borrowed via `retain()`).
- **`enabled = false` wins over `always_on = true`:** Eviction runs before `always_on` application in `apply_overrides()`, so disabled skills are removed from the registry regardless of other overrides. This is the intuitive behavior — "disabled" means "fully removed from consideration."
- **Separate `set_skill_enabled()` method:** Parallel to `set_skill_override()` for `always_on`, using same UPSERT pattern. Cleaner than overloading `set_skill_override()`.
- **CLI DB access already exists:** `skills.rs` line 129 opens the DB for LLM overrides. The enable/disable path reuses this same connection.
- **Migration fail-open:** If a `.disabled` marker can't be removed (read-only filesystem), log warning and continue. Belt-and-suspenders match-time filter (kept until #630) handles the dual state.
- **CLI changes require server restart:** Cross-process `skills_dirty` propagation is out of scope. This matches current behavior — CLI `.disabled` file changes also only take effect on next `skills_dirty` trigger or restart.

## Open Questions

### Resolved During Planning

- **CLI DB access:** Already established at `skills.rs:129` for LLM overrides. Same `Database::open()` path reused.
- **`always_on` vs `enabled` precedence:** `enabled = false` always wins. Eviction in `apply_overrides()` runs before `always_on` application.
- **Migration error handling:** Fail-open. Write DB row, log warning if marker can't be removed.
- **Bundled skills:** Have filesystem directories via `seed_bundled_skills()`. `toggle_skill` tool's `skill_dir.exists()` check still valid. No changes needed.
- **`delete_skill` cleanup:** Already calls `delete_skill_override()` which DELETEs the entire row — naturally covers new `enabled` column.

### Deferred to Implementation

- Exact staging vec pattern in `apply_overrides()` — depends on borrow checker constraints with `self.disabled` and `self.entries`.

## Implementation Units

- [x] **Unit 1: Schema migration v23→v24 and DB methods**

**Goal:** Add `enabled` column to `skill_overrides` and expose DB read/write methods.

**Requirements:** R1, R5

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/db.rs`
- Modify: `crates/mika-agent/src/async_db.rs`
- Test: `crates/mika-agent/src/db.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- Add `migrate_v23_to_v24()`: `ALTER TABLE skill_overrides ADD COLUMN enabled INTEGER;`
- **Update `migrate_v1()` clean-slate CREATE TABLE** to include `enabled INTEGER` — this is the documented trap from the LLM override learnings doc
- Add `enabled: Option<bool>` to `SkillOverride` struct
- Update `get_skill_overrides()` SELECT to include `enabled` column
- Add `set_skill_enabled(agent_id, skill_name, enabled: bool)` — UPSERT via `ON CONFLICT DO UPDATE SET enabled = ?3`. If setting to default (enabled=true), check if all other columns are NULL and delete the row (default-equals-delete)
- Update `delete_skill_llm_override()` default-equals-delete check to include `enabled IS NULL` in the cleanup condition
- Add async wrappers in `async_db.rs`
- Bump `CURRENT_SCHEMA_VERSION` to 24

**Patterns to follow:**
- `set_skill_override()` for UPSERT pattern
- `delete_skill_llm_override()` for default-equals-delete pattern
- `migrate_v22_to_v23()` for migration pattern

**Test scenarios:**
- Happy path: `set_skill_enabled(agent, "foo", false)` creates row with `enabled = 0`, `get_skill_overrides()` returns it with `enabled: Some(false)`
- Happy path: `set_skill_enabled(agent, "foo", true)` on a row with no other overrides deletes the row (default-equals-delete)
- Happy path: `set_skill_enabled(agent, "foo", true)` on a row with `always_on = Some(true)` preserves the row with `enabled` set to NULL (not deleted because `always_on` is non-NULL)
- Edge case: `set_skill_enabled` on a skill with existing LLM override preserves both columns
- Edge case: migration from v23 to v24 on DB with existing `skill_overrides` rows — `enabled` defaults to NULL
- Happy path: round-trip — disable then enable returns to clean state

**Verification:**
- `cargo test -p mika-agent` passes with new migration and DB method tests
- Fresh in-memory DB (via `migrate_v1`) includes `enabled` column

- [x] **Unit 2: SkillRegistry eviction in `apply_overrides()`**

**Goal:** Evict disabled skills from `entries` into a new `disabled` vec during override application.

**Requirements:** R2, R3

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/skills/mod.rs`
- Modify: `crates/mika-agent/src/skills/index.rs` (for `DisabledSkill` type)
- Test: `crates/mika-agent/src/skills/mod.rs` (inline tests)

**Approach:**
- Define `DisabledSkill` struct (name, dir, reason) in `index.rs`, parallel to `SkippedSkill`
- Add `disabled: Vec<DisabledSkill>` field to `SkillRegistry`
- Add `disabled()` accessor method (parallels `skipped()`)
- In `apply_overrides()`: before applying `always_on`/LLM overrides, collect names of skills with `enabled == Some(false)` from overrides. Use a staging vec: iterate `entries` with `retain()`, push non-retained to staging vec as `DisabledSkill`, then extend `self.disabled`
- Update "skills loaded" log in `from_dir()` — this happens before `apply_overrides()`, so the count is pre-eviction. The post-eviction count is logged by callers or visible via `skills().len()`. Consider adding a log line after `apply_overrides()` that reports the disabled count
- Keep `SkillEntry.enabled` field and `scan_skills_dir()` marker check — they become redundant but serve as belt-and-suspenders until #630

**Patterns to follow:**
- `skipped()` accessor and `SkippedSkill` struct for the parallel `disabled` pattern
- Existing `apply_overrides()` match-by-name loop

**Test scenarios:**
- Happy path: skill with `enabled: Some(false)` override is evicted from `entries` and appears in `disabled()`
- Happy path: skill with no `enabled` override (NULL) stays in `entries`
- Happy path: skill with `enabled: Some(true)` stays in `entries`
- Edge case: skill with both `always_on: Some(true)` and `enabled: Some(false)` — evicted (disabled wins)
- Edge case: empty overrides list — no eviction, `disabled` is empty
- Edge case: override for non-existent skill — silently skipped (existing behavior)
- Integration: `from_dir()` → `apply_overrides()` → `validate_loaded()` — disabled skills not passed to validation

**Verification:**
- `cargo test -p mika-agent` passes
- `disabled().len()` returns correct count after `apply_overrides()`

- [x] **Unit 3: Agent tool `toggle_skill` writes to DB**

**Goal:** Make the agent tool write `skill_overrides.enabled` instead of `.disabled` marker files.

**Requirements:** R1

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/tools/toggle_skill.rs`
- Test: `crates/mika-agent/src/tools/toggle_skill.rs` (inline tests or eval harness)

**Approach:**
- Replace `.disabled` marker file creation/removal with `ctx.db.set_skill_enabled(agent_id, name, !current_enabled)` call
- Determine current state from DB (`get_skill_overrides`) rather than filesystem marker
- Keep `skills_dirty.store(true)` — the reload mechanism still works (rebuilds registry from disk + DB)
- Remove all `.disabled` marker file I/O from this tool

**Patterns to follow:**
- `update_skill` tool's DB write pattern for `always_on` overrides

**Test scenarios:**
- Happy path: disable a skill → DB row created with `enabled = 0`, `skills_dirty` set to true
- Happy path: enable a previously disabled skill → DB row cleaned up (default-equals-delete), `skills_dirty` set
- Error path: toggle non-existent skill → error response (existing behavior preserved)
- Edge case: toggle skill that has existing `always_on` override → `enabled` column updated, `always_on` preserved

**Verification:**
- `cargo test -p mika-agent` passes
- No references to `.disabled` marker in `toggle_skill.rs`

- [x] **Unit 4: CLI `mika skills enable/disable` writes to DB**

**Goal:** Make CLI enable/disable commands write to DB instead of filesystem.

**Requirements:** R1, R3

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-cli/src/commands/skills.rs`
- Test: `crates/mika-cli/src/commands/skills.rs` (inline tests if feasible, otherwise manual verification)

**Approach:**
- Refactor `toggle_skill()` to accept a `&Database` parameter and call `db.set_skill_enabled()`
- The DB is already opened at line 129 for LLM overrides — route `Enable`/`Disable` commands through the same DB-opening path
- For the `list` command: show disabled skills from DB overrides in a separate "[disabled]" section. The CLI already constructs a `SkillRegistry` for listing — after `apply_overrides()`, the `disabled()` accessor provides the data
- Remove all `.disabled` marker file I/O from the CLI

**Patterns to follow:**
- Existing `SkillLlmAction` handling at line 132+ for DB-backed CLI operations
- `skills list` output formatting for the new disabled section

**Test scenarios:**
- Happy path: `mika skills disable foo` writes DB row, prints confirmation
- Happy path: `mika skills enable foo` clears DB row, prints confirmation
- Happy path: `mika skills list` shows disabled skills in separate section with count
- Error path: disable non-existent skill → error message (preserve existing behavior)
- Edge case: disable when DB doesn't exist → clear error message (DB path check at line 123)

**Verification:**
- `cargo build -p mika-cli` succeeds
- No references to `.disabled` marker in `skills.rs`

- [x] **Unit 5: `.disabled` marker migration**

**Goal:** Migrate existing `.disabled` markers to DB rows on startup and remove the marker files.

**Requirements:** R4

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/skills/mod.rs` or `crates/mika-agent/src/skills/index.rs`
- Test: inline tests

**Approach:**
- Add `migrate_disabled_markers(skills_dir: &Path, db: &Database, agent_id: &str) -> Result<()>` function
- Iterate skill directories, check for `.disabled` marker, call `db.set_skill_enabled(agent_id, name, false)`, remove marker file
- On marker removal failure (read-only FS): log warning, continue (fail-open)
- Call this function at startup BEFORE `scan_skills_dir()` — the migration runs once (idempotent: no markers left after first pass)
- Hook point: callers of `SkillRegistry::from_dir()` that have DB access. The three startup sites are `chat.rs`, `ask.rs`, and `server/mod.rs`. Also `teams/engine.rs` sync path. Either plumb DB into `from_dir()` or call migration separately before `from_dir()`

**Patterns to follow:**
- `seed_bundled_skills()` for startup-time filesystem+DB interaction pattern

**Test scenarios:**
- Happy path: directory with `.disabled` marker → DB row created, marker removed
- Happy path: directory without `.disabled` marker → no DB write, no error
- Edge case: idempotency — run twice, second pass is a no-op (no markers left)
- Edge case: marker file can't be removed → warning logged, DB row still created
- Edge case: skill directory doesn't exist in DB overrides yet → creates new row

**Verification:**
- `cargo test -p mika-agent` passes
- After migration, no `.disabled` files remain in test fixtures

- [x] **Unit 6: Remove `.disabled` marker from `scan_skills_dir()`**

**Goal:** Stop reading `.disabled` markers during skill scanning. The enabled state now comes from DB via `apply_overrides()`.

**Requirements:** R2

**Dependencies:** Unit 2, Unit 5

**Files:**
- Modify: `crates/mika-agent/src/skills/index.rs`

**Approach:**
- Remove `let enabled = !path.join(".disabled").exists();` from `scan_skills_dir()`
- Set `enabled: true` unconditionally on all scanned skills (the field is still used by the belt-and-suspenders match-time filter until #630, but now always true for scanned skills — the eviction in `apply_overrides()` handles the actual disabling)
- Keep the `enabled` field on `SkillEntry` — it's still read by `matcher.rs` and `always_on_skills()` until #630

**Test expectation:** Existing `scan_skills_dir` tests that create `.disabled` markers should be updated to verify the field is always `true`.

**Verification:**
- `cargo test -p mika-agent` passes
- No references to `.disabled` in `scan_skills_dir()`

## System-Wide Impact

- **Interaction graph:** `apply_overrides()` is called from all startup sites (`chat.rs`, `ask.rs`, `server/mod.rs`, `teams/engine.rs`) and from `list_skills` tool. All sites already pass `SkillOverride` data from DB — the new `enabled` field flows automatically via the updated `get_skill_overrides()` SELECT.
- **Error propagation:** DB write failures in `set_skill_enabled()` propagate as `anyhow::Result` to tool/CLI callers. Migration failures at startup are logged but don't block startup (fail-open on marker removal).
- **State lifecycle risks:** CLI writes to DB while server is running — server won't see the change until next `skills_dirty` trigger or restart. This matches current `.disabled` file behavior and is documented as a known limitation.
- **API surface parity:** The `toggle_skill` agent tool and CLI `skills enable/disable` both write to DB — they produce identical state.
- **Unchanged invariants:** `skills_dirty` reload mechanism, `validate_loaded()` post-processing, `SkippedSkill` tracking, exec handler security, bundled skill seeding — all unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Clean-slate `migrate_v1` missing new column | Documented trap — Unit 1 explicitly includes updating `migrate_v1` |
| CLI concurrent write with running server | SQLite WAL mode handles this; CLI uses short-lived connection with busy timeout |
| `.disabled` markers on read-only filesystem during migration | Fail-open: write DB row, log warning, marker stays (belt-and-suspenders filter handles dual state) |
| `always_on` + `enabled=false` interaction confusion | Documented: `enabled=false` always wins. Eviction runs first in `apply_overrides()` |

## Sources & References

- Related issue: #629
- Follow-up: #630 (remove match-time filter)
- Learnings: `docs/solutions/database-issues/skill-override-persistence-via-db-layer.md`
- Learnings: `docs/solutions/architecture-patterns/skill-llm-override-db-layer-and-linked-unblock.md`
- Learnings: `docs/solutions/best-practices/agent-tool-must-call-validate-loaded-on-skill-registry.md`

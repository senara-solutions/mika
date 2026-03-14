---
title: Remove builtin calendar skill
type: refactor
status: completed
date: 2026-03-14
---

# Remove builtin calendar skill

The builtin `calendar` skill is a non-functional placeholder that hardcodes HTTP handler URLs to `localhost:8080/api/events` — an endpoint that doesn't exist. Superseded by `google-workspace` skill. Pure deletion, no replacement needed.

Closes #154.

## Acceptance Criteria

- [x] `crates/mika-agent/templates/skills/calendar/` directory deleted (skill.toml, tools.json, system_prompt.md)
- [x] `CALENDAR_SKILL` static and `BUNDLED_SKILLS` entry removed from `crates/mika-agent/src/bundled_skills.rs`
- [x] Test fixture in `crates/mika-agent/src/skills/index.rs` (~line 850) updated to use a different skill name
- [x] Documentation references removed:
  - `docs/skills.md` — remove calendar row from bundled skills table
  - `docs/runtime-structure.md` — remove `calendar/` from directory tree
  - `docs/slash-commands.md` — remove calendar example output
  - `docs/getting-started.md` — update or remove calendar example
- [x] Crate-local doc copies updated (via `scripts/sync-agent-docs.sh` or manual edit):
  - `crates/mika-agent/docs/skills.md`
  - `crates/mika-agent/docs/runtime-structure.md`
  - `crates/mika-agent/docs/slash-commands.md`
  - `crates/mika-agent/docs/getting-started.md`
- [x] `cargo build` succeeds
- [x] `cargo test` passes

## Context

- **Calendar skill location:** `crates/mika-agent/templates/skills/calendar/` (3 files)
- **Rust registration:** `bundled_skills.rs` — `static CALENDAR_SKILL` (lines 85-89), `BUNDLED_SKILLS` array (line 127)
- **Test fixture:** `skills/index.rs` line 850 — `is_legacy_format` test uses "calendar" as a fixture name
- **Note:** `builtin_handlers.rs` references "calendar" as a GWS subcommand — this is unrelated and must NOT be touched
- **Skill loading:** Skills are embedded via `include_str!` at compile time, then seeded to filesystem at startup. Removing the static + template files is sufficient — no runtime discovery code needs changing.

## Sources

- GitHub issue: #154

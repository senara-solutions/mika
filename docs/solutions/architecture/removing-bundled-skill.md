---
title: Removing a bundled skill
category: architecture
tags: [skills, bundled-skills, deletion, refactoring]
date: 2026-03-14
severity: low
component: mika-agent
related_issues: ["#154"]
---

# Removing a Bundled Skill

## Problem

Bundled skills are compile-time embedded via `include_str!` in `bundled_skills.rs`. Deleting only the template directory breaks the build because the macro references now-missing files. All references must be removed in lockstep.

## Checklist

1. **Delete template directory:** `crates/mika-agent/templates/skills/<name>/`
2. **Remove Rust registration:** Delete `static <NAME>_SKILL` and its `&<NAME>_SKILL` entry in `BUNDLED_SKILLS` array in `crates/mika-agent/src/bundled_skills.rs`
3. **Update test fixtures:** Grep for the skill name in `crates/mika-agent/src/skills/` tests — replace with a neutral fixture name
4. **Update documentation** (both canonical and crate-local copies):
   - `docs/skills.md` — remove from bundled skills table
   - `docs/runtime-structure.md` — remove from directory tree
   - `docs/slash-commands.md` — update any example output referencing the skill
   - `docs/getting-started.md` — update any usage examples
5. **Sync crate-local copies:** Run `scripts/sync-agent-docs.sh`
6. **Verify:** `cargo build && cargo test`

## What you do NOT need to change

- **Runtime discovery code:** `scan_skills_dir` is filesystem-driven, not registry-driven. No changes needed.
- **`is_bundled_skill()` function:** Derives from `BUNDLED_SKILLS` dynamically — removing the entry is sufficient.
- **Seed/cleanup logic:** `seed_bundled_skills` only writes, never prunes. No changes needed.

## Orphaned directories on existing installs

After removal, existing installs retain `~/.mika/skills/<name>/` on disk. The seeder won't touch it (no longer in `BUNDLED_SKILLS`), and `scan_skills_dir` will pick it up as a custom skill. This is harmless for non-functional skills. Users can clean up with `mika skills uninstall <name>` or `rm -rf ~/.mika/skills/<name>/`.

Document this migration step in the PR description per the pre-1.0 breaking changes policy.

## Reference

- PR: Remove calendar skill (#154)
- Pattern established in: `crates/mika-agent/src/bundled_skills.rs`

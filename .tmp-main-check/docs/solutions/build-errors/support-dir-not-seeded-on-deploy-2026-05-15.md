---
module: mika-agent
tags: [bundled-skills, deploy, _shared, dispatch-lib, build-time-discovery]
problem_type: build-deploy-mismatch
category: build-errors
ticket: mika#923
date: 2026-05-15
---

# `_shared/` support directory not propagated by `mika skills update`

## Problem

After `make deploy`, all `run_claude_pilot` dispatches from mika-dev failed with:

```
_shared/dispatch-lib.sh: No such file or directory
```

The `_shared/` directory (shared dispatch plumbing for dev-pilot and dev-groom) was correctly excluded from build-time skill discovery (`_` prefix convention) but was also incorrectly excluded from the install path that seeds bundled skills into deployed agent homes.

## Root cause

Two-layer interaction:

1. `build.rs` discovers skills in `skills/bundled/` and excludes `_`-prefixed directories (correct — `_shared/` is not a skill).
2. `seed_bundled_skills()` reads the generated `ENTRIES` table and copies each skill to the deployed agent home. Since `_shared/` is not in `ENTRIES`, it is never copied.

Skills like `dev-pilot` source `../../_shared/dispatch-lib.sh` at runtime via relative path. The path is valid in the source tree but broken in the deployed tree because the install step only copies skills, not support directories.

## Fix

Added a parallel discovery/seeding path for underscore-prefixed support directories:

1. **Build-time:** `discover_support_dirs()` finds `_`-prefixed dirs. `build.rs` generates a `SUPPORT_DIRS` table (mirrors `ENTRIES` pattern). Test files (`*test*`) are excluded.
2. **Install-time:** `seed_support_dirs()` writes support dir files to `~/.mika/agents/<name>/skills/<dir>/`. Called from two places:
   - `seed_bundled_skills()` — ensures support dirs are present whenever skills are seeded
   - `seed_bundled_skills_if_needed()` (in `startup.rs`) — called **unconditionally before the `disabled` guard**, so support dirs are seeded even when `MIKA_DISABLE_BUNDLED_SKILLS=true`
3. **Shared helper:** `write_dir_files()` extracted from `write_skill()` for reuse by both skill and support dir seeding. Same symlink defense-in-depth.

## Key insight

The `MIKA_DISABLE_BUNDLED_SKILLS=true` flag is intended for hot-patching skill prompts during development, not for breaking the dispatch infrastructure. Support directories are dispatch plumbing, not skill prompts, so they must be seeded regardless of the flag.

## Files changed

- `crates/mika-agent/build_support/bundled_skills_discover.rs` — `discover_support_dirs()`, `collect_support_dir_files()`
- `crates/mika-agent/build.rs` — `SUPPORT_DIRS` table generation
- `crates/mika-agent/src/bundled_skills.rs` — `SupportDir` struct, `write_dir_files()`, `seed_support_dirs()`
- `crates/mika-agent/src/startup.rs` — unconditional `seed_support_dirs()` call before disabled guard
- `CLAUDE.md` — updated Build-Time Discovery section

## Prevention

Future underscore-prefixed support directories in `skills/bundled/` are automatically discovered and seeded by the same mechanism — no per-directory plumbing required.

---
ticket: mika#923
type: fix
branch: fix/923/skills-install-shared-shared-library
date: 2026-05-15
---

# Plan: Propagate `_shared/` support directories during bundled skills seeding

## Problem

`_shared/dispatch-lib.sh` is correctly excluded from build-time skill discovery (it's not a skill — no `skill.toml`). But it's also excluded from the seeding path that writes bundled skills to `~/.mika/agents/<name>/skills/`. Skills like `dev-pilot` source `../../_shared/dispatch-lib.sh` at runtime via relative path, so when `_shared/` is absent from the deployed skills tree, dispatches fail with "No such file or directory."

Introduced by mika#893 (refactor to shared dispatch lib), first surfaced 2026-05-01 on `make deploy`.

## Fix shape: Option 1 from ticket — embed and seed support directories

The same compile-time embedding pattern used for skills (`include_str!` + generated Rust source) is extended to underscore-prefixed support directories. No new constants or parallel discovery paths — the generated source file gains a `SUPPORT_DIRS` table alongside the existing `ENTRIES` table.

## Files to modify

### 1. `crates/mika-agent/build_support/bundled_skills_discover.rs`

Add `discover_support_dirs(base: &Path) -> Vec<DiscoveredSupportDir>`:
- Walk `base` for immediate subdirectories starting with `_`
- Skip dotfiles and symlinks (same defense-in-depth as skill discovery)
- Recursively collect all files (no `skill.toml` requirement, no `handlers/` depth limit — support dirs are flat or shallow)
- Mark `.sh` files as `executable: true`
- Return sorted by name

New struct:
```rust
pub struct DiscoveredSupportDir {
    pub name: String,        // e.g. "_shared"
    pub abs_dir: PathBuf,
    pub files: Vec<DiscoveredFile>,  // reuse existing DiscoveredFile
}
```

### 2. `crates/mika-agent/build.rs` — `generate_bundled_skills_table()`

After the existing `ENTRIES` generation block, add a parallel block for support directories:
- Call `discover_support_dirs(&bundled_root)`
- Emit `cargo:rerun-if-changed` for each support directory and its files
- Generate `static SUPPORT_DIR_{idx}_FILES: &[SkillFile]` arrays (reuse `SkillFile` type)
- Generate `static SUPPORT_DIRS: &[SupportDir] = &[...]` table

The `SupportDir` struct mirrors `BundledSkill` minus `content_hash` (support dirs don't need drift detection):
```rust
struct SupportDir {
    name: &'static str,
    files: &'static [SkillFile],
}
```

### 3. `crates/mika-agent/src/bundled_skills.rs`

Add `SupportDir` struct (compile-time shape, like `BundledSkill`):
```rust
struct SupportDir {
    name: &'static str,
    files: &'static [SkillFile],
}
```

The `include!()` of the generated file already exists — `SUPPORT_DIRS` is emitted into the same generated file.

Add `seed_support_dirs(skills_dir: &Path)`:
- For each entry in `SUPPORT_DIRS`, write files to `skills_dir/<name>/`
- Same symlink defense-in-depth as `write_skill()`
- Reuse `write_skill()` internals (or extract shared `write_dir_files()` helper)

Call `seed_support_dirs()` from `seed_bundled_skills()` after the skill seeding loop. Order doesn't matter (no cross-dependency at write time), but putting support dirs first is slightly cleaner since skills depend on them at runtime.

### 4. `crates/mika-agent/tests/bundled_skills_load.rs` (or inline `#[cfg(test)]` in `bundled_skills.rs`)

Add test: `test_seed_creates_support_dirs`
- Call `seed_bundled_skills()` into a tempdir
- Assert `_shared/dispatch-lib.sh` exists in the seeded directory
- Assert the file is non-empty
- Assert the file is executable (Unix)

Add test: `test_seed_support_dirs_idempotent`
- Seed twice, verify no duplicates or errors

Add test: `test_support_dir_symlink_rejected`
- Replace support dir with symlink, verify write is refused (same as existing skill symlink test)

### 5. `CLAUDE.md` (mika root) — Build-Time Discovery section

Update the existing paragraph:
> Directories starting with `.` (dotfiles) or `_` (convention-reserved for shared support libraries like `_shared/`) are excluded from **skill** discovery. **Support directories** (underscore-prefixed) are discovered separately and seeded alongside skills by `seed_bundled_skills()`, so sibling skills can source them at runtime via relative path.

## Acceptance criteria mapping

| AC | Covered by |
|----|------------|
| `_shared/dispatch-lib.sh` copied on `mika skills update` | `seed_support_dirs()` in `seed_bundled_skills()` |
| Idempotent re-run | Same overwrite semantics as `write_skill()` |
| Fresh `make deploy` produces working dev-pilot | Support dir seeded before handler execution |
| Behavioral test with `_test_support/` | Test in `bundled_skills.rs` (or integration test with fixture) |
| Existing `bundled_skills_load` tests pass | Additive change — `ENTRIES` generation unchanged |
| CLAUDE.md updated | Section 5 above |

## What this does NOT change

- Build-time skill discovery (`discover_bundled_skills`) — unchanged, still skips `_`-prefixed dirs
- Runtime skill scan (`scan_skills_dir`) — unchanged, still skips `_`-prefixed dirs
- The `_` prefix convention itself
- `dev-pilot/handlers/run.sh` relative path sourcing
- Marketplace install path (`copy_skill_dir`) — not involved in the bundled skills flow

## Risk assessment

**Low.** The change is additive — a new parallel discovery/seeding path that doesn't touch the existing skill pipeline. The only integration point is the call to `seed_support_dirs()` inside `seed_bundled_skills()`. Failure in support dir seeding is isolated (same `warn!` + continue pattern as skill seeding).

## Implementation estimate

~150 lines of Rust across build.rs, discover, and bundled_skills.rs. ~80 lines of tests. Straightforward mirroring of existing patterns.

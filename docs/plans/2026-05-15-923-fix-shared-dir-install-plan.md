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

## Design rationale — Option 2 over Option 1

The issue body states "Option 1 is preferred — minimal change, follows the existing convention, no new generated constants." This plan implements **Option 2** (compile-time embedding with a `SUPPORT_DIRS` table) instead. Rationale:

1. **Consistency with existing pipeline.** All bundled skill content reaches the deployed agent via compile-time `include_str!` embedding → generated Rust source → `seed_bundled_skills()` write. Option 1 would introduce a runtime filesystem walk in the install path — a pattern that has no analog in the existing bundled-skills flow. The binary would need to know where the source tree is at runtime (it doesn't today — it's self-contained).

2. **Self-contained binary.** With Option 2, `mika-spirit` carries `_shared/` content in the binary itself. No dependency on source tree presence at deploy time. This matters for Docker images and crates.io packaging.

3. **All ACs are met regardless of approach.** The issue body's acceptance criteria are behavioral (files land, idempotent, tests pass) — they don't constrain implementation approach.

If Vincent's preference for Option 1 is firm, this should be escalated before implementation.

## Phase 0 — Code pins

All pins against `main` at `22d3f5d9` (2026-05-15).

### Pin A — Underscore-prefix exclusion in `discover_bundled_skills()`

`crates/mika-agent/build_support/bundled_skills_discover.rs:78-82`:
```rust
        // Skip underscore-prefixed directories (_shared/, _templates/, etc.)
        // These are convention-reserved for non-skill support directories.
        if name_str.starts_with('_') {
            continue;
        }
```
This is the exclusion that causes the bug. Support dir discovery mirrors the same walk but inverts the filter — only `_`-prefixed dirs.

### Pin B — `ENTRIES` table codegen in `build.rs`

`crates/mika-agent/build.rs:130-200`: The `generate_bundled_skills_table()` function calls `discover_bundled_skills(&bundled_root)` and emits `static ENTRIES: &[BundledSkill]`. The `SUPPORT_DIRS` table generation is appended after this block, into the same output file (`bundled_skills_generated.rs`). No namespace collision risk — `ENTRIES` and `SUPPORT_DIRS` are distinct constant names.

### Pin C — `seed_bundled_skills()` body

`crates/mika-agent/src/bundled_skills.rs:259-275`:
```rust
pub fn seed_bundled_skills(skills_dir: &Path) {
    for skill in all_bundled_skills() {
        let skill_dir = skills_dir.join(skill.name);
        let is_update = skill_dir.exists();

        if let Err(e) = write_skill(&skill_dir, skill) {
            warn!(skill = skill.name, error = %e, "failed to seed bundled skill");
            if !is_update {
                let _ = std::fs::remove_dir_all(&skill_dir);
            }
        } else if is_update {
            debug!(skill = skill.name, "updated bundled skill");
        } else {
            info!(skill = skill.name, "seeded bundled skill");
        }
    }
}
```
`seed_support_dirs()` is called from within this function, after the skill loop.

### Pin D — `write_skill()` signature and symlink defense

`crates/mika-agent/src/bundled_skills.rs:278-313`:
```rust
fn write_skill(skill_dir: &Path, skill: &BundledSkill) -> std::io::Result<()> {
    if skill_dir.exists() && skill_dir.symlink_metadata()?.file_type().is_symlink() {
        return Err(std::io::Error::other("skill directory is a symlink, refusing to write"));
    }
    for file in skill.files { ... }
}
```
`write_skill()` takes `&BundledSkill` which carries `files: &'static [SkillFile]`. To reuse this for support dirs, we extract the file-writing loop into `write_dir_files(dir: &Path, files: &[SkillFile]) -> io::Result<()>` and call it from both `write_skill()` and the support dir seeder. The symlink check stays in both callers (not extracted — each call site names its error context).

### Pin E — `seed_bundled_skills_if_needed()` guard structure

`crates/mika-agent/src/startup.rs:33-58`:
```rust
pub fn seed_bundled_skills_if_needed(home_dir: &Path, disabled: bool) {
    let skills_dir = home_dir.join("skills");
    if disabled {
        tracing::warn!("bundled skill seeding disabled by config ...");
        // Drift detection (#984)
        ...
        return;  // ← early return skips seed_bundled_skills()
    }
    if skills_dir.is_dir() {
        crate::bundled_skills::seed_bundled_skills(&skills_dir);
    }
}
```
**Critical:** The `disabled` guard causes an early return that skips `seed_bundled_skills()` entirely. Support dirs must NOT be gated behind this flag — see F2 resolution below.

### Pin F — Call sites

Two call sites invoke `seed_bundled_skills_if_needed`:
- `src/server/mod.rs:383` — server startup
- `src/tools/create_agent.rs:119` — `seed_bundled_skills_if_needed(&agent_home, false)` (always `false` — agent creation always seeds)

Both paths reach `seed_bundled_skills()` → support dirs are seeded in both.

## Files to modify

### 1. `crates/mika-agent/build_support/bundled_skills_discover.rs`

Add `discover_support_dirs(base: &Path) -> Vec<DiscoveredSupportDir>`:
- Walk `base` for immediate subdirectories starting with `_`
- Skip dotfiles and symlinks (same defense-in-depth as skill discovery)
- Recursively collect all files (no `skill.toml` requirement, no `handlers/` depth limit — support dirs are flat or shallow)
- Mark `.sh` files as `executable: true`
- Exclude files matching `*test*` (case-insensitive) — `test-dispatch-lib.sh` is a test fixture, not a production runtime dependency. Deployed agents don't need test scaffolding.
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

Extract `write_dir_files(dir: &Path, files: &[SkillFile]) -> io::Result<()>` from `write_skill()`:
- Contains the file-writing loop (create parent dirs, symlink check per-file, write content, set executable)
- `write_skill()` becomes: symlink-check-dir + `write_dir_files()`
- Support dir seeder uses: symlink-check-dir + `write_dir_files()`

Add `seed_support_dirs(skills_dir: &Path)`:
- For each entry in `SUPPORT_DIRS`, write files to `skills_dir/<name>/`
- Same symlink defense-in-depth as `write_skill()`: check dir-level symlink before writing
- Same `warn!` + continue on failure, with `info!`/`debug!` for new/updated

Make `seed_support_dirs` public and call it from two places:
1. From `seed_bundled_skills()` (covers the `disabled=false` path)
2. From `seed_bundled_skills_if_needed()` in `startup.rs`, **before the `disabled` early return** (covers the `disabled=true` path)

This ensures support dirs are seeded unconditionally — see F2 rationale below.

### 4. `crates/mika-agent/src/startup.rs` — `seed_bundled_skills_if_needed()`

Add unconditional support dir seeding before the `disabled` guard:
```rust
pub fn seed_bundled_skills_if_needed(home_dir: &Path, disabled: bool) {
    let skills_dir = home_dir.join("skills");

    // Support dirs are infrastructure (dispatch plumbing), not skill prompts.
    // They must be seeded even when MIKA_DISABLE_BUNDLED_SKILLS=true — that
    // flag is for hot-patching skill prompts during dev, not for breaking
    // the dispatch pipeline. See mika#923, mika#984.
    if skills_dir.is_dir() {
        crate::bundled_skills::seed_support_dirs(&skills_dir);
    }

    if disabled {
        tracing::warn!("bundled skill seeding disabled by config ...");
        ...
        return;
    }
    if skills_dir.is_dir() {
        crate::bundled_skills::seed_bundled_skills(&skills_dir);
    }
}
```

### 5. `crates/mika-agent/tests/bundled_skills_load.rs` (or inline `#[cfg(test)]` in `bundled_skills.rs`)

Add test: `test_seed_creates_support_dirs`
- Call `seed_bundled_skills()` into a tempdir
- Assert `_shared/dispatch-lib.sh` exists in the seeded directory
- Assert the file is non-empty
- Assert the file is executable (Unix)

Add test: `test_seed_support_dirs_idempotent`
- Seed twice, verify no duplicates or errors

Add test: `test_support_dir_symlink_rejected`
- Replace support dir with symlink, verify write is refused (same as existing skill symlink test)

Add test: `test_support_dirs_seeded_when_bundled_skills_disabled`
- Call `seed_bundled_skills_if_needed(home, true)` (disabled=true)
- Assert `_shared/dispatch-lib.sh` exists (support dirs seeded despite disabled flag)
- Assert no skill directories exist (skills correctly skipped)

### 6. `CLAUDE.md` (mika root) — Build-Time Discovery section

Update the existing paragraph:
> Directories starting with `.` (dotfiles) or `_` (convention-reserved for shared support libraries like `_shared/`) are excluded from **skill** discovery. **Support directories** (underscore-prefixed) are discovered separately at build time and seeded unconditionally by `seed_bundled_skills_if_needed()` (even when `MIKA_DISABLE_BUNDLED_SKILLS=true`), so sibling skills can source them at runtime via relative path.

## Acceptance criteria mapping

| AC | Covered by |
|----|------------|
| `_shared/dispatch-lib.sh` copied on `mika skills update` | `seed_support_dirs()` in `seed_bundled_skills()` + unconditional call in `startup.rs` |
| Idempotent re-run | Same overwrite semantics as `write_skill()` via shared `write_dir_files()` |
| Fresh `make deploy` produces working dev-pilot | Support dir seeded before handler execution |
| Behavioral test with `_test_support/` | Test in `bundled_skills.rs` (or integration test with fixture) |
| Existing `bundled_skills_load` tests pass | Additive change — `ENTRIES` generation unchanged |
| CLAUDE.md updated | Section 6 above |

## What this does NOT change

- Build-time skill discovery (`discover_bundled_skills`) — unchanged, still skips `_`-prefixed dirs
- Runtime skill scan (`scan_skills_dir`) — unchanged, still skips `_`-prefixed dirs
- The `_` prefix convention itself
- `dev-pilot/handlers/run.sh` relative path sourcing
- Marketplace install path (`copy_skill_dir`) — not involved in the bundled skills flow

## Risk assessment

**Low.** The change is additive — a new parallel discovery/seeding path that doesn't touch the existing skill pipeline. The only integration points are: (1) `seed_support_dirs()` call inside `seed_bundled_skills()`, (2) unconditional `seed_support_dirs()` call in `startup.rs` before the disabled guard. Failure in support dir seeding is isolated (same `warn!` + continue pattern as skill seeding). The `write_dir_files()` extraction is a pure refactor of existing logic.

## Implementation estimate

~180 lines of Rust across build.rs, discover, bundled_skills.rs, and startup.rs. ~100 lines of tests. Straightforward mirroring of existing patterns plus one helper extraction.

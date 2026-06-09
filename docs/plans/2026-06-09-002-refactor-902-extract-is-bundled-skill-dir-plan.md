---
title: "refactor: Extract is_bundled_skill_dir shared predicate (mika#902)"
status: active
created: 2026-06-09
plan_depth: lightweight
origin: https://github.com/senara-solutions/mika/issues/902
---

# refactor: Extract `is_bundled_skill_dir` shared predicate

## Summary

Extract a shared `is_bundled_skill_dir(name: &str) -> bool` predicate from two independently inlined directory-name filters in the build-time discovery module and the runtime skill scanner. Place it in `build_support/bundled_skills_discover.rs` (the existing shared-helpers home) and consume it at runtime via the existing `#[path = ...]` mod pattern. Add unit tests covering edge cases.

## Problem Frame

Two scanners independently filter `skills/bundled/` subdirectories using the same rule: skip names starting with `.` or `_`. The duplication was exposed empirically by mika#893/PR mika#898 — the runtime site lacked the `_` filter until commit `2d1c2e97` added it inline. Future convention changes (e.g., `_templates/`, `_overrides/`) would require remembering to update both sites.

## Requirements

- R1. A single `is_bundled_skill_dir(name: &str) -> bool` predicate defines the filter rule
- R2. Both `discover_bundled_skills` (build-time) and `scan_skills_dir` (runtime) use the shared predicate
- R3. Unit tests cover: empty name, dotfile prefix, underscore prefix, valid names, names with `_` mid-name
- R4. No behavioral change — the filter rule stays `!empty && !starts_with('.') && !starts_with('_')`

## Key Technical Decisions

**Location: `build_support/bundled_skills_discover.rs` (option a from ticket).**
This module is already the shared-helpers home consumed by both `build.rs` (via `#[path = "build_support/..."]`) and integration tests (via `#[path = "../build_support/..."]`). Runtime code in `src/skills/index.rs` can import it the same way the test file `tests/bundled_skills_directory_source.rs` already does. No new module or crate boundary needed.

**`discover_support_dirs` is out of scope.**
Its filter has inverted semantics (requires `_` prefix, skips dotfiles separately). It does not share the same predicate shape, so forcing it through `is_bundled_skill_dir` would obscure intent.

---

## Implementation Units

### U1. Add `is_bundled_skill_dir` predicate and unit tests

**Goal:** Define the shared predicate in `bundled_skills_discover.rs` with inline unit tests.

**Requirements:** R1, R3

**Dependencies:** None

**Files:**
- `crates/mika-agent/build_support/bundled_skills_discover.rs` (modify)

**Approach:** Add a `pub fn is_bundled_skill_dir(name: &str) -> bool` that returns `!name.is_empty() && !name.starts_with('.') && !name.starts_with('_')`. Add a `#[cfg(test)] mod tests` block with unit tests. The `#[cfg(test)]` block will only compile when this module is included in a test binary (the integration test file already pulls it in via `#[path = ...]`). The `build.rs` consumer is unaffected since `build.rs` does not run tests.

**Patterns to follow:** Existing `DiscoveredSkill`, `DiscoveredFile` structs in the same module — public, `#[allow(dead_code)]` for cross-compilation-unit consumption.

**Test scenarios:**
- Empty string returns `false`
- `"."` and `".hidden"` return `false` (dotfile prefix)
- `"_shared"` and `"_templates"` return `false` (underscore prefix)
- `"self-dev"` returns `true` (valid skill name)
- `"mika-arch-groom-ticket"` returns `true` (valid name with hyphens)
- `"my_skill"` returns `true` (underscore mid-name, not prefix)
- `"a"` returns `true` (minimal valid name)

### U2. Replace inline filters with predicate calls

**Goal:** Both `discover_bundled_skills` and `scan_skills_dir` call `is_bundled_skill_dir` instead of inlining the filter.

**Requirements:** R2, R4

**Dependencies:** U1

**Files:**
- `crates/mika-agent/build_support/bundled_skills_discover.rs` (modify — `discover_bundled_skills` function)
- `crates/mika-agent/src/skills/index.rs` (modify — `scan_skills_dir` function)

**Approach:**

*Build-time site* (`discover_bundled_skills`, ~line 204-212): Replace the two separate `if` blocks (dotfile skip + underscore skip) with a single `if !is_bundled_skill_dir(&name_str) { continue; }`.

*Runtime site* (`scan_skills_dir`, ~line 494): Replace the combined `if dir_name.starts_with('.') || dir_name.starts_with('_')` with `if !is_bundled_skill_dir(dir_name)`. Remove the DRY-violation comment block (lines 488-493) since the duplication is resolved. To consume the predicate, add a `#[path = ...]` mod import at the top of `index.rs` (or in the parent `skills/mod.rs`) mirroring the pattern used by `tests/bundled_skills_directory_source.rs`.

**Patterns to follow:** `tests/bundled_skills_directory_source.rs` line 9 — `#[path = "../build_support/bundled_skills_discover.rs"] mod discover;` shows the existing consumption pattern.

**Test scenarios:**
- Existing test `bundled_skills_load_without_oversized_prompts` passes (regression gate — exercises `scan_skills_dir` against real `skills/bundled/`)
- Existing test `subdirectory_without_skill_toml_is_skipped` passes (exercises `discover_bundled_skills` via fixture, includes dotfile skip)
- Add a fixture test: create a `_support` directory alongside a valid skill in the fixture tree; verify `discover_bundled_skills` skips it (the existing fixture tree at `tests/fixtures/skills/bundled/` has only `test-echo`)

**Verification:** `cargo test -p mika-agent` passes. `cargo build` succeeds (build.rs consumes the module). `cargo clippy` clean.

---

## Scope Boundaries

### In Scope
- Extract predicate, update two call sites, add tests

### Out of Scope
- `discover_support_dirs` filter changes (inverted semantics)
- Other `starts_with('.')` checks in `index.rs` (provider/model variant filtering — different domain, different filter shape)
- Filter rule changes (the shape is canonical per CLAUDE.md)
- `marketplace.rs::is_hidden_or_git` (different filter, different purpose)

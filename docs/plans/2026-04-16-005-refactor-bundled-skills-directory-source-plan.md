---
title: "refactor(agent): source bundled skills from skills/bundled/ directory at build time"
type: refactor
status: active
date: 2026-04-16
origin: ../mika-platform/docs/brainstorms/2026-04-16-bundle-engine-coupled-skills-into-mika-brainstorm.md
issue: 598
---

# refactor(agent): source bundled skills from skills/bundled/ directory at build time

## Overview

Today `seed_bundled_skills()` in `crates/mika-agent/src/bundled_skills.rs` enumerates 12 hardcoded `BundledSkill` statics, each using the `skill!` macro with explicit `include_str!` references into `crates/mika-agent/templates/skills/<name>/`. This ticket adds a **parallel** code path: a `build.rs`-generated `ENTRIES` table populated by walking a new top-level directory `skills/bundled/*/skill.toml`. The generated table is consumed by `seed_bundled_skills()` alongside the existing hardcoded list, with `ENTRIES` winning on name collision.

No skill moves. No behavior changes. The hardcoded list and the new path coexist. The empty case (no directories under `skills/bundled/`) must compile and run cleanly. A single sentinel skill proves the path end-to-end.

## Problem Frame

Engine-coupled skills (self-dev, claude-pilot, permission-policy, etc.) live in the separate `mika-skills` repo, which produces repeated cross-repo drift bugs — tool schemas, callback contracts, and prompt-discipline rules get out of lockstep with Rust code (see mika#586/mika-skills#150, mika#588 unstaged residue, mika#595 hallucination).

The fix class, per the brainstorm, is to bundle those skills inside `mika/` as `skills/bundled/`. That migration PR must move 11 skill directories and flip the installation path in one shot. This ticket is the **preparatory refactor** that makes the install path a directory walk rather than a hardcoded list — so the migration PR is a pure `git mv` + Rust-side list trim.

## Requirements Trace

- R1. `build.rs` walks `skills/bundled/*/skill.toml` at build time and generates a Rust module exposing `ENTRIES: &[BundledSkill]` (or equivalent shape compatible with today's struct).
- R2. An empty or non-existent `skills/bundled/` directory compiles cleanly and produces an empty table — no errors, warnings, or panics.
- R3. Adding a test skill directory (e.g., `skills/bundled/test-echo/` with a minimal `skill.toml` + `tools.json`) is picked up at build time and seeded at startup identically to a hardcoded bundled skill.
- R4. Existing bundled skills (the 12 hardcoded statics) continue to work. The refactor adds a **parallel** path; it does not replace the hardcoded list.
- R5. `seed_bundled_skills()` processes both the legacy hardcoded set AND the new `ENTRIES` set. On name collision, `ENTRIES` wins. Zero collisions are expected during this ticket.
- R6. Uninstall is still rejected for skills present in either set (`is_bundled_skill` must return true for both sources).
- R7. Bundled skills (either source) still win over marketplace skills with the same name.
- R8. `[built-in]` origin marker is preserved for both sources.
- R9. All existing tests still pass. Incremental rebuilds triggered by changes under `skills/bundled/` via `cargo:rerun-if-changed`.

## Scope Boundaries

- **No skill migration.** The 11 engine-coupled skills in `mika-skills/` stay where they are. That's a separate follow-up ticket.
- **No removal of the hardcoded list.** It remains the production path for the 12 existing skills until a future ticket flips them one-by-one.
- **No `mika skills install` CLI changes.** Bundled skills remain non-installable from the marketplace.
- **No versioning scheme for bundled skills.** They ride mika's version; `version` field in `skill.toml` is informational.
- **No prompt-variant generation, review gating, or trust-critical classification for the new path.** Those concerns apply uniformly once skills are present; this ticket only deals with the installation source.

### Deferred to Separate Tasks

- Migration of 11 engine-coupled skills into `skills/bundled/`: follow-up ticket, gated on this.
- Deletion of those skills from `mika-skills`: follow-up PR after migration lands.
- Collapsing the hardcoded list once all production bundled skills have moved: future cleanup.
- Removal of the stale `run_claude_pilot` "requires both" calibration rule in `self-dev/system_prompt.md:231`: handled by the migration ticket, not this one.

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/bundled_skills.rs` — current hardcoded `BundledSkill` statics, the `skill!` macro, `BUNDLED_SKILLS: &[&BundledSkill]`, and `seed_bundled_skills()`. All tests under `#[cfg(test)] mod tests` iterate `BUNDLED_SKILLS`.
- `crates/mika-agent/build.rs` — existing `build.rs` already copies `docs/` and dashboard assets into `OUT_DIR`. Uses `cargo:rerun-if-changed` and `include_str!`-ready layouts. Pattern to mirror for the new generator.
- `crates/mika-agent/templates/skills/<name>/` — current on-disk layout of bundled skill files. Each has `skill.toml`, `system_prompt.md`, `tools.json`, and optional `handlers/*.sh`.
- `crates/mika-agent/src/startup.rs`, `crates/mika-agent/src/server/mod.rs`, `crates/mika-agent/src/tools/create_agent.rs` — callers of `seed_bundled_skills()`. No signature change needed.
- `is_bundled_skill()` / `is_trust_critical_skill()` — membership queries that must now consult both sources.

### Institutional Learnings

- **Build tooling separation** (user memory `feedback_build_tooling.md`): keep Rust and Node.js build systems decoupled. This plan stays Rust-side only (no npm dependency).
- **Doc sync pattern** (`crates/mika-agent/build.rs`, `scripts/sync-agent-docs.sh`): existing precedent for `build.rs` writing into `OUT_DIR` + `include_str!` from `concat!(env!("OUT_DIR"), ...)`. Use the same pattern, not runtime filesystem reads.
- **mika-agent has `publish = false`**: no crates.io fallback needed. The build can assume the workspace layout is always present. This avoids the dual-source fallback complexity that `docs/` sync has.

### External References

None required. The feature is internal refactoring with no new framework or library.

## Key Technical Decisions

- **Build-time discovery via `build.rs`** (not runtime filesystem reads). Matches today's embedded-bytes model, zero runtime surprises, works in Docker images without bind mounts.
- **Generator writes a single `.rs` file into `OUT_DIR`**, included via `include!(concat!(env!("OUT_DIR"), "/bundled_skills_generated.rs"))`. Same pattern as `docs/` sync.
- **Generated file declares a `static ENTRIES: &[BundledSkill] = &[ ... ]`** matching the existing `BundledSkill` struct shape — so both sources can be concatenated in one iteration path.
- **Handler executability inferred from file extension + path heuristic.** Generator marks files under `handlers/` ending in `.sh` as `executable: true` (matches existing convention). No `skill.toml` schema change.
- **Merge strategy in `seed_bundled_skills()`:** build a per-skill map keyed by name, folding `BUNDLED_SKILLS` (legacy) first, then overlaying `ENTRIES` (generated) to implement "ENTRIES wins". The result is a single vec of `&BundledSkill`. Membership functions (`is_bundled_skill`) consult the same merged view.
- **Directory location: `mika/skills/bundled/`** (top-level, not nested under `crates/mika-agent/`). Matches the brainstorm's directory layout and leaves room for future `skills/dev-only/` or similar. `build.rs` resolves the path via `CARGO_MANIFEST_DIR + "../../skills/bundled/"`.
- **Empty directory handling:** if the base dir does not exist OR has no subdirectories, the generator emits `static ENTRIES: &[BundledSkill] = &[];`. No panic, no warning. A `.gitkeep` in `skills/bundled/` keeps the directory present in git without being treated as a skill (generator only considers subdirectories containing `skill.toml`).
- **No `skill.toml` parsing at build time.** The generator only verifies the file exists; it does not parse or validate it. Validation remains the runtime job of `SkillRegistry::validate_loaded()` once the skill is installed. This keeps `build.rs` simple and prevents build-time dependency on the `toml` crate.
- **File discovery is shallow + one level of nesting.** Generator enumerates files directly in the skill dir AND a single `handlers/` subdirectory. Matches today's template layout; anything deeper is a YAGNI surface we're explicitly not building.
- **Rerun hooks:** emit `cargo:rerun-if-changed=skills/bundled/` plus one line per discovered file, so adding/removing skills or editing any embedded file re-triggers the build.

## Open Questions

### Resolved During Planning

- **Should this change the `BundledSkill` struct?** No. Keep the struct identical so both sources produce the same type.
- **Should ENTRIES be populated with production skills in this PR?** No. ENTRIES is empty in production; only a single `test-echo` fixture exists under test-only configuration to prove the path.
- **Where does `test-echo` live?** Under `crates/mika-agent/tests/fixtures/skills/bundled/test-echo/` — NOT the production `skills/bundled/` tree. See technical decision below.
- **How are handlers marked executable?** Path heuristic: files under `handlers/` with `.sh` suffix. No `skill.toml` schema change.

### Deferred to Implementation

- **Final helper function name in `bundled_skills.rs`** (e.g., `all_bundled_skills()` vs inlining the merge in `seed_bundled_skills()`). Implementer picks the shape that reads best after touching the code.
- **Whether `is_bundled_skill()` internally memoizes the merged iterator or rebuilds each call.** Performance is not a concern at current scale; pick the simpler form.
- **Exact generator code shape** (macro-based vs straight-line `writeln!` calls). Implementer's call; the only hard requirement is that `OUT_DIR/bundled_skills_generated.rs` declares `static ENTRIES: &[BundledSkill]`.

## Output Structure

```
mika/
  skills/
    bundled/
      .gitkeep                   # new — keeps empty directory in git
  crates/
    mika-agent/
      build.rs                   # modified — adds generator call
      src/
        bundled_skills.rs        # modified — consumes ENTRIES alongside BUNDLED_SKILLS
      tests/
        fixtures/
          skills/
            bundled/
              test-echo/         # new — test-only fixture skill
                skill.toml
                tools.json
                system_prompt.md
```

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

**Build-time generator (sketch):**

```
// build.rs adds a new step after copy_dashboard_assets:
generate_bundled_skills_table(&manifest_dir, &out_dir);

fn generate_bundled_skills_table(manifest_dir, out_dir):
    bundled_root = manifest_dir/"../../skills/bundled"
    output = out_dir/"bundled_skills_generated.rs"
    emit "cargo:rerun-if-changed={bundled_root}"

    entries = []
    if bundled_root exists and is_dir:
        for entry in read_dir(bundled_root) sorted by name:
            if not is_dir or is_symlink: skip
            skill_toml = entry/"skill.toml"
            if not skill_toml.exists: skip   # .gitkeep and stray files ignored

            files = enumerate_skill_files(entry)
                    # shallow + handlers/ only
                    # mark executable=true if path matches handlers/*.sh
            entries.push({ name: entry.file_name(), files })

            for file in files:
                emit "cargo:rerun-if-changed={absolute_path}"

    write output containing:
        static ENTRIES: &[BundledSkill] = &[ /* generated entries with include_str! */ ];
```

**Runtime consumption (sketch):**

```
// bundled_skills.rs adds:
include!(concat!(env!("OUT_DIR"), "/bundled_skills_generated.rs"));

fn all_bundled_skills() -> Vec<&'static BundledSkill>:
    // ENTRIES wins on collision
    let by_name = BUNDLED_SKILLS.iter().copied()
        .map(|s| (s.name, s))
        .collect::<HashMap<_,_>>();
    for entry in ENTRIES:
        by_name.insert(entry.name, entry);   // overwrite on collision
    by_name.into_values().collect()

fn seed_bundled_skills(skills_dir):
    for skill in all_bundled_skills():
        write_skill(...)  // unchanged

fn is_bundled_skill(name) -> bool:
    all_bundled_skills().any(|s| name_eq_ignore_case)
```

The `BundledSkill` struct signature stays byte-identical, so the generated file's `include_str!(...)` calls resolve against absolute paths emitted by `build.rs` (which is exactly how `include_str!` already resolves any path — it's rooted at the file including it, but we can embed absolute paths in the generated literal).

## Implementation Units

- [ ] **Unit 1: Create `skills/bundled/` with `.gitkeep` placeholder**

**Goal:** Establish the new top-level directory. Empty in production; keeps the directory in git.

**Requirements:** R1, R2

**Dependencies:** None

**Files:**
- Create: `skills/bundled/.gitkeep`

**Approach:**
- Empty file. Generator's "no `skill.toml` present" path handles this case as "skip".

**Execution note:** None — pure scaffolding.

**Patterns to follow:**
- Other `.gitkeep` usages in the repo (none load-bearing; just presence).

**Test scenarios:**
- Test expectation: none — this is pure scaffolding with no behavioral surface.

**Verification:**
- `git status` shows the new file. `cargo build` still succeeds (covered by Unit 2's tests).

---

- [ ] **Unit 2: Extend `build.rs` to generate `bundled_skills_generated.rs`**

**Goal:** Walk `skills/bundled/*/skill.toml` at build time, emit a Rust file in `OUT_DIR` declaring `static ENTRIES: &[BundledSkill] = &[...]`, and wire up `cargo:rerun-if-changed` for the directory and every embedded file.

**Requirements:** R1, R2, R3, R9

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/build.rs`

**Approach:**
- Add a new `generate_bundled_skills_table(manifest_dir, out_dir)` function called from `main`.
- Base path: `Path::new(manifest_dir).join("../../skills/bundled")`.
- Enumerate subdirectories sorted alphabetically for deterministic output.
- Skip any subdir without `skill.toml` (handles stray files, `.gitkeep`, etc.).
- File discovery per skill: read top-level files + files in `handlers/` if it exists. Skip symlinks (defense-in-depth — mirrors the runtime symlink refusal in `bundled_skills.rs`). Skip nested subdirectories beyond `handlers/`.
- Executable flag: `true` if the file path matches `handlers/*.sh`.
- Emit `cargo:rerun-if-changed` for the base dir AND each discovered file.
- Emit `cargo:rerun-if-changed` for the base dir itself even when empty, so adding the first skill re-triggers the build.
- Generated file shape: fully qualified struct literals that match today's `BundledSkill`/`SkillFile` layout exactly. Use absolute paths in `include_str!`/`include_bytes!` so no relative-path resolution surprises arise.
- Empty case: write `static ENTRIES: &[BundledSkill] = &[];` with no entries.
- Do NOT parse `skill.toml`. Existence check only.

**Execution note:** None — the new function is pure code generation; integration coverage lives in Unit 3.

**Technical design:** See High-Level Technical Design above for the generator sketch.

**Patterns to follow:**
- Existing `copy_dashboard_assets()` in `build.rs` for symlink filtering, `cargo:rerun-if-changed` emission, and empty-directory handling with a `cargo:warning`.
- `DOCS` array pattern in `build.rs` for manifest-dir path arithmetic.

**Test scenarios:**
- Happy path: `cargo build -p mika-agent` with empty `skills/bundled/` succeeds; `bundled_skills_generated.rs` declares `ENTRIES` as an empty slice.
- Integration: touch any file under `skills/bundled/` (after Unit 4 adds a fixture) and re-run `cargo build -p mika-agent` — rebuild occurs (verify via `cargo build -v` or an `env!("OUT_DIR")` inspection test in Unit 3).

**Verification:**
- `cat $(find target -name bundled_skills_generated.rs)` shows a well-formed Rust file declaring `ENTRIES`.
- `cargo build -p mika-agent` succeeds with `skills/bundled/` empty.

---

- [ ] **Unit 3: Consume `ENTRIES` from `bundled_skills.rs` and merge with `BUNDLED_SKILLS`**

**Goal:** Wire the generated `ENTRIES` into `seed_bundled_skills()` and membership queries, with `ENTRIES` winning on name collision. Preserve existing tests.

**Requirements:** R4, R5, R6, R7, R8, R9

**Dependencies:** Unit 2

**Files:**
- Modify: `crates/mika-agent/src/bundled_skills.rs`
- Test: `crates/mika-agent/src/bundled_skills.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- Add `include!(concat!(env!("OUT_DIR"), "/bundled_skills_generated.rs"));` near the top of the file, after the `BundledSkill`/`SkillFile` definitions so the generated file can reference those types.
- Add a private helper `all_bundled_skills() -> Vec<&'static BundledSkill>` that merges `BUNDLED_SKILLS` and `ENTRIES` with ENTRIES winning on name collision (case-insensitive match on `name`).
- Refactor `seed_bundled_skills()`, `is_bundled_skill()`, and existing tests that iterate `BUNDLED_SKILLS` to call `all_bundled_skills()` where they should see the merged view. Keep direct `BUNDLED_SKILLS` references only where the intent is specifically the legacy set (e.g., the `test_trust_critical_skills_are_subset_of_bundled` test semantics — update its assertion to use the merged view).
- `is_trust_critical_skill()` stays hardcoded against `TRUST_CRITICAL_SKILLS`. Trust-critical classification is not inferred from directory contents — it's an explicit Rust-side declaration. If a future bundled skill needs trust-critical status, the developer adds it to `TRUST_CRITICAL_SKILLS`. The `test_trust_critical_skills_are_subset_of_bundled` test validates this invariant against the merged view.
- Preserve the symlink refusal, executable-bit handling, and extra-file preservation semantics of `write_skill()` — it receives a `&BundledSkill` regardless of source.

**Execution note:** None.

**Patterns to follow:**
- `BUNDLED_SKILLS: &[&BundledSkill]` shape — mirror the reference-to-static-slice style.
- Case-insensitive name comparison: `str::eq_ignore_ascii_case` (already used in `is_bundled_skill`/`is_trust_critical_skill`).

**Test scenarios:**
- Happy path: with empty `ENTRIES`, `seed_bundled_skills()` behavior is byte-identical to today (existing tests must pass unchanged).
- Happy path: `all_bundled_skills()` returns every name in `BUNDLED_SKILLS` exactly once when `ENTRIES` is empty.
- Edge case: `is_bundled_skill("tmux")` and `is_bundled_skill("TMUX")` return `true` (case-insensitive preserved).
- Edge case: `is_bundled_skill("nonexistent-skill")` returns `false`.
- Integration: `test_trust_critical_skills_are_subset_of_bundled` still passes against the merged view.
- Integration (reserved for Unit 4): collision behavior — covered by Unit 4's fixture test.

**Verification:**
- `cargo test -p mika-agent bundled_skills` passes (all existing tests plus new case-insensitive assertions).
- No API signature changes to callers in `startup.rs`, `server/mod.rs`, `tools/create_agent.rs`.

---

- [ ] **Unit 4: Prove the path end-to-end with a test-only fixture skill**

**Goal:** Demonstrate that a skill dropped into `skills/bundled/` is picked up at build time, seeded at runtime, and wins on name collision — without polluting production builds.

**Requirements:** R3, R5

**Dependencies:** Unit 3

**Files:**
- Create: `crates/mika-agent/tests/fixtures/skills/bundled/test-echo/skill.toml`
- Create: `crates/mika-agent/tests/fixtures/skills/bundled/test-echo/tools.json`
- Create: `crates/mika-agent/tests/fixtures/skills/bundled/test-echo/system_prompt.md`
- Create: `crates/mika-agent/tests/bundled_skills_directory_source.rs` (integration test)

**Approach:**
- Two sub-decisions:
  1. **Production `skills/bundled/` stays empty in this PR.** Shipping a `test-echo` in the production tree would be installed on every user's machine, violating "no migration yet".
  2. **Fixture lives under `tests/fixtures/`.** The integration test programmatically invokes the same discovery logic on the fixture path — or, alternatively, uses a separate `tests/fixtures_build.rs` pattern (see below).
- **Recommended approach:** Factor the generator's discovery logic into a pub(crate) `discover_bundled_skills(base: &Path) -> Vec<BundledSkillDescriptor>` helper in a shared module that both `build.rs` and integration tests can call. `build.rs` uses it against `skills/bundled/`; the test calls it against `tests/fixtures/skills/bundled/`. This avoids generating a parallel OUT_DIR artifact just for tests.
- **Alternative if factoring is awkward:** Keep the generator in `build.rs` only; have the integration test shell out to `cargo build -p mika-agent` with a temp scratch tree. Heavier but zero coupling. Prefer the factored approach; fall back if implementation reveals circular dependencies.
- **Collision test:** fixture includes a skill named `tmux` (same as a legacy hardcoded skill) with a distinct marker in its `skill.toml`. The test asserts the merged view returns the fixture's version, proving ENTRIES-wins semantics. Note: this is test-only — production ENTRIES is empty, so no real collision ever occurs at runtime.

**Execution note:** None.

**Patterns to follow:**
- `crates/mika-agent/tests/eval/` for integration-test layout (separate `tests/` dir with per-file harnesses).
- `tempfile::tempdir()` for the seeding target directory (mirrors `bundled_skills.rs` tests).

**Test scenarios:**
- Happy path: `discover_bundled_skills(fixture_path)` returns exactly one `test-echo` descriptor with the expected files.
- Happy path: seeding a tempdir with the fixture's test-echo descriptor produces the expected files on disk with correct contents.
- Edge case: `discover_bundled_skills(nonexistent_path)` returns an empty vec, no panic.
- Edge case: subdirectory with no `skill.toml` (e.g., a stray `.gitkeep` holder) is skipped.
- Integration: merge with a synthetic "legacy" list containing a skill named `test-echo` — the fixture's version wins on name collision (case-insensitive).
- Integration: `cargo build -p mika-agent` on a clean build emits `cargo:rerun-if-changed=skills/bundled` (or equivalent) — verifiable by inspecting the build log with `-v` in CI; this test scenario may be covered by manual verification rather than an assertion.

**Verification:**
- `cargo test -p mika-agent --test bundled_skills_directory_source` passes.
- Production ENTRIES remains empty after this unit — no test-echo installed on real users (confirm by running `cargo build -p mika-agent` and inspecting the generated file).

## System-Wide Impact

- **Interaction graph:** Callers of `seed_bundled_skills()` (`startup.rs`, `server/mod.rs`, `tools/create_agent.rs`) see no signature change and no behavioral change while production ENTRIES is empty. `is_bundled_skill()` is called from skill install/uninstall/update guards — its case-insensitive semantics must not regress.
- **Error propagation:** `write_skill()` continues to log `warn!` on failure and skip to the next skill — bundled source is transparent to error handling. Generator failures (e.g., I/O errors reading `skills/bundled/`) cause `build.rs` to panic, which is desirable: better to fail the build than ship a silently-empty bundled table.
- **State lifecycle risks:** None new. Bundled skill writes are idempotent, symlink-safe, and preserve extra user files. The merge operates on static data computed at compile time; no runtime state.
- **API surface parity:** `is_bundled_skill()`, `is_trust_critical_skill()`, and `trust_critical_skill_names()` all stay compatible. Public API unchanged.
- **Integration coverage:** Unit 4's fixture test is the proof-of-path. Existing seeding tests (`test_seed_creates_all_skills`, `test_seed_is_idempotent`, `test_seed_updates_existing_bundled_skills`, `test_seed_preserves_extra_files_in_bundled_dir`, `test_symlinked_skill_dir_is_skipped`, `test_handlers_are_executable`) exercise the merged view transparently and must pass without modification.
- **Unchanged invariants:**
  - `BundledSkill` / `SkillFile` struct shape.
  - `seed_bundled_skills()` signature.
  - `TRUST_CRITICAL_SKILLS` list and its manual-declaration semantics.
  - The `skill!` macro and the 12 hardcoded `BundledSkill` statics.
  - Symlink-refusal + extra-file-preservation in `write_skill()`.
  - `[built-in]` origin marker via the non-marketplace path.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| `include_str!` in a generated file can't resolve relative paths the way hand-written code does | Use absolute paths (emitted by `build.rs` via `manifest_dir.join(...).canonicalize()`) in the generated `include_str!` calls. |
| Production ENTRIES accidentally ships with `test-echo` or similar fixture | Fixture lives under `tests/fixtures/`, NOT `skills/bundled/`. `build.rs` only walks `skills/bundled/`; test uses a factored helper with an explicit base path. |
| Incremental rebuilds don't fire when a bundled skill file changes | Emit `cargo:rerun-if-changed` for every discovered file in addition to the base directory. Test by touching a fixture file and re-running `cargo build -v`. |
| Non-existent `skills/bundled/` (e.g., fresh clone before Unit 1 lands, or `cargo package` verify) breaks build | Generator handles missing dir gracefully (empty ENTRIES, no panic). `.gitkeep` in Unit 1 guarantees the dir exists in all clones. |
| Developer adds a production bundled skill without updating `TRUST_CRITICAL_SKILLS` when needed | Trust-critical classification is declarative in Rust — `test_trust_critical_skills_are_subset_of_bundled` runs against the merged view, so forgetting to add a truly trust-critical name is caught at test time. Not forgetting in the first place is a developer-discipline concern outside this ticket's scope. |
| Build-time `build.rs` walks become slow if `skills/bundled/` grows large | Current scope is 0-11 skills. Revisit if the directory exceeds ~50 skills. |

## Documentation / Operational Notes

- **CLAUDE.md:** `crates/mika-agent/CLAUDE.md` mentions `seed_bundled_skills()` behavior. Update the "Skills System" section with a one-line note that bundled skills are sourced from both the hardcoded list and `skills/bundled/` (generated at build time). Defer the full split-vs-community description to the migration ticket.
- **docs/skills.md:** No user-facing behavior change. Skip.
- **No new env vars, config keys, or migration steps.**
- **Rollback:** revert the PR. The hardcoded list remains the sole production source regardless.

## Sources & References

- **Origin document:** `../mika-platform/docs/brainstorms/2026-04-16-bundle-engine-coupled-skills-into-mika-brainstorm.md` ("Concrete Next Tickets" item 1)
- **GitHub issue:** mika#598
- Related code: `crates/mika-agent/src/bundled_skills.rs`, `crates/mika-agent/build.rs`, `crates/mika-agent/templates/skills/`
- Related tickets (follow-ups): migrate 11 engine-coupled skills (separate ticket), remove bundled skills from `mika-skills` (separate ticket), `mika-platform` CLAUDE.md split documentation (separate ticket).
- Unblocks: `mika-platform#41`, `mika-platform#42` (milestone refactor).

---
title: "feat: Skill dependency resolution at install time"
type: feat
status: active
date: 2026-03-19
origin: ../brainstorms/2026-03-19-skill-dependency-management-brainstorm.md
---

# feat: Skill dependency resolution at install time

## Overview

Extend `mika skills install` to automatically resolve and install dependencies declared in `skill.toml`. Today, the `dependencies` field is only used at runtime (skill activation). The installer ignores it completely, forcing users to manually install each dependency.

**Motivating example:** `mika skills install --link ./mika-skills/self-dev` should also link `build-mika` automatically, logging each dependency as it's installed.

## Problem Statement

The skill ecosystem is growing. Manual dependency management doesn't scale:

1. `mika skills install --link self-dev` succeeds but `build-mika` is not installed — runtime silently skips missing deps
2. Users have no way to know what dependencies a skill needs until it fails at runtime
3. The `dependencies` field in `skill.toml` is effectively decorative at install time

## Proposed Solution

Add dependency resolution to the install flow with these behaviors (see brainstorm: `docs/brainstorms/2026-03-19-skill-dependency-management-brainstorm.md`):

1. **Auto-install** dependencies without prompting, with log output per dependency
2. **Same-source resolution** — look for deps as siblings in the source directory/repo
3. **Bundled passthrough** — skip deps that match bundled skill names (always available)
4. **Already-installed passthrough** — skip deps already present in the agent's skills dir
5. **`--link` propagation** — deps from the same source are linked when parent is linked
6. **Full transitive resolution** with cycle detection
7. **Best-effort for unresolvable deps** — warn (don't fail) if a dep can't be found in same-source or as bundled, but IS already installed. Fail only if truly missing everywhere.
8. **Orphan cleanup on uninstall** — prompt per orphaned dependency (interactive), keep all (non-interactive) or `--remove-deps` flag

## Technical Approach

### Phase 1: Data Model Changes

#### 1a. Add `dependencies` to `SkillCandidate`

**File:** `mika/crates/mika-agent/src/skills/marketplace.rs`

The scanner (`try_load_candidate`) already parses the full `SkillManifest` but discards `dependencies`. Surface it on `SkillCandidate`:

```rust
// marketplace.rs — SkillCandidate
pub struct SkillCandidate {
    pub name: String,
    pub description: String,
    pub relative_path: String,
    pub absolute_path: PathBuf,
    pub has_exec_handlers: bool,
    pub dependencies: Vec<String>,  // NEW
}
```

Update `try_load_candidate` to populate from `manifest.skill.dependencies`.

#### 1b. Add `installed_as_dependency_of` to `MarketplaceEntry`

**File:** `mika/crates/mika-agent/src/skills/marketplace.rs`

```rust
// marketplace.rs — MarketplaceEntry
pub struct MarketplaceEntry {
    pub url: String,
    pub path: String,
    pub commit: String,
    pub linked: bool,
    #[serde(default)]
    pub installed_as_dependency_of: Vec<String>,  // NEW — diamond deps require Vec
    pub installed_at: String,
    pub updated_at: String,
}
```

`#[serde(default)]` ensures backward compatibility — existing lock files without this field deserialize as empty vec (meaning "manually installed").

### Phase 2: Dependency Resolution Engine

#### 2a. Resolution function

**File:** `mika/crates/mika-agent/src/skills/install.rs` (new function)

```rust
/// Resolve the full transitive dependency tree for a set of skill candidates.
/// Returns an ordered list of (candidate, is_new_install) tuples, dependencies first.
///
/// Resolution strategy per dependency name:
/// 1. Already installed in agent's skills_dir → skip (passthrough)
/// 2. Bundled skill name → skip (always available)
/// 3. Found in `available_candidates` (same-source siblings) → include for install
/// 4. Not found → error with clear message
///
/// Cycle detection: tracks the resolution chain, errors with full cycle path.
/// Max depth: 10 levels (guard against pathological chains).
pub fn resolve_dependencies(
    roots: &[&SkillCandidate],
    available_candidates: &[SkillCandidate],  // all skills found in same source
    skills_dir: &Path,
    agent_home: &Path,
) -> Result<Vec<ResolvedDep>>
```

```rust
pub struct ResolvedDep {
    pub candidate: SkillCandidate,
    pub depth: usize,             // 0 = root, 1 = direct dep, 2+ = transitive
    pub required_by: String,      // name of the skill that depends on this one
}
```

**Algorithm:** BFS traversal with a `HashSet<String>` visited set and a `Vec<String>` chain for cycle detection. Process roots first, then their deps breadth-first. For each dependency name:

1. If in visited set → skip (already resolved, handles diamonds)
2. If in current chain → error: `Circular dependency detected: A -> B -> C -> A`
3. If `skills_dir.join(name).exists()` → mark visited, skip (already installed)
4. If `is_bundled_skill(name)` → mark visited, skip (always available)
5. If found in `available_candidates` by case-insensitive name match → add to results, recurse into its dependencies
6. Not found → `bail!("Dependency \"{name}\" required by \"{parent}\" not found in source or as bundled skill. Install it manually first.")`

**Self-dependency check:** If a skill lists itself in `dependencies`, reject with a clear error.

#### 2b. Wire into install flow

**File:** `mika/crates/mika-cli/src/commands/skills.rs`

In both `install_from_git()` and `install_from_local()`, after candidate selection but before installing:

1. Scan the source for all available candidates (already done — `scan_repo_for_skills`)
2. Call `resolve_dependencies()` with selected candidates and all available candidates
3. Install resolved deps first (in BFS order — leaves before roots)
4. For each dep installed, log: `  Installing dependency: {name} (required by {parent})`
5. If `--link` is set, deps from the same source directory are also linked
6. Install the primary skill(s) last

**Multi-skill picker interaction:** If the user selected skill A from an interactive picker but deselected B, and A depends on B, auto-add B with a log message: `  Auto-selecting "{name}" (dependency of "{parent}")`

### Phase 3: Lock File Dependency Tracking

**File:** `mika/crates/mika-agent/src/skills/install.rs`

When installing a dependency, set `installed_as_dependency_of: vec![parent_name.to_string()]`.

When installing a dependency that is already installed (diamond case): read existing lock entry, append the new parent to `installed_as_dependency_of`, write back.

When manually installing a skill that was previously a dependency: clear `installed_as_dependency_of` (it's now intentionally installed).

### Phase 4: Uninstall Orphan Detection

**File:** `mika/crates/mika-cli/src/commands/skills.rs`

When `mika skills uninstall <name>`:

1. Remove the skill (existing behavior)
2. Scan the lock file for entries where `installed_as_dependency_of` contains the removed skill name
3. For each, remove the uninstalled skill from the vec
4. If the vec is now empty AND the skill is not manually installed (was purely a dependency):
   - **Interactive (TTY):** Prompt: `Dependency "{name}" is no longer needed. Remove? [y/N]`
   - **Non-interactive:** Keep it (safe default), log: `Orphaned dependency "{name}" kept (non-interactive mode). Use --remove-deps to remove.`
5. `--remove-deps` flag: remove all orphans without prompting
6. Recurse: removing an orphan may orphan its own dependencies (cascading cleanup)

### Phase 5: Transitive Runtime Resolution

**File:** `mika/crates/mika-agent/src/skills/matcher.rs`

Replace the one-level dependency expansion in `match_skills()` with a BFS that resolves the full tree:

```rust
// Current: single pass over direct dependencies
// New: BFS with visited set
let mut queue: VecDeque<usize> = initial_indices.iter().copied().collect();
let mut visited: HashSet<usize> = initial_indices.clone();

while let Some(idx) = queue.pop_front() {
    for dep_name in &skills[idx].dependencies {
        if let Some(dep_idx) = find_by_name(skills, dep_name) {
            if skills[dep_idx].enabled && visited.insert(dep_idx) {
                queue.push_back(dep_idx);
            }
        }
    }
}
```

**Critical: `safe_always_on_skills()` remains untouched.** No dependency resolution in safe mode — this prevents exec/http handler skills from being pulled into heartbeat/reflection contexts. Add a test for this explicitly.

**Disabled mid-chain behavior:** If A → B → C and B is disabled, C is NOT loaded. Disabling a skill breaks its sub-tree (strict-chain interpretation). This is the safe default.

## System-Wide Impact

- **`safe_always_on_skills()`**: Explicitly exempt from transitive resolution. Must not change. Add test.
- **`mika skills update`**: No change — updates individual skills independently. Dependencies are updated when their own entry is updated.
- **`mika skills validate`**: Could optionally check that declared dependencies are installed, but not in scope for this plan.
- **`--name` alias**: Aliased skills are found by their installed name, not their manifest name. If skill A is aliased as "my-a", dependencies looking for "A" won't find "my-a". This is existing behavior and acceptable — aliasing is a power-user feature with known trade-offs.

## Acceptance Criteria

- [ ] `mika skills install --link ./mika-skills/self-dev --agent mika-dev` also links `build-mika` with log output
- [ ] `mika skills install user/repo` where repo has A depending on B installs both, B first
- [ ] Transitive deps: A → B → C installs all three in correct order
- [ ] Circular dependency A → B → A produces clear error with cycle path
- [ ] Already-installed deps are skipped with log message
- [ ] Bundled skill deps (e.g., `browser-control`) are skipped silently
- [ ] Missing dep that can't be resolved fails install with actionable error message
- [ ] `--link` propagates to deps from same source directory
- [ ] Lock file records `installed_as_dependency_of` for each dependency
- [ ] Diamond deps: C depended on by both A and B has both in `installed_as_dependency_of`
- [ ] `mika skills uninstall A` prompts for orphaned deps in interactive mode
- [ ] `mika skills uninstall A --remove-deps` removes orphans without prompting
- [ ] Non-interactive uninstall keeps orphans with log message
- [ ] Runtime `match_skills()` resolves full transitive tree
- [ ] `safe_always_on_skills()` does NOT resolve any dependencies (test)
- [ ] Disabled mid-chain dep breaks its sub-tree at runtime
- [ ] Self-dependency in manifest is rejected
- [ ] Max depth (10) guard prevents pathological chains
- [ ] Existing lock files without `installed_as_dependency_of` deserialize correctly (empty vec)

## Implementation Phases

### Phase 1: Data Model (Small, foundational)
- Add `dependencies` to `SkillCandidate` in `marketplace.rs`
- Add `installed_as_dependency_of` to `MarketplaceEntry` in `marketplace.rs`
- Verify backward compat with existing lock files

### Phase 2: Resolution Engine (Core logic)
- Implement `resolve_dependencies()` in `install.rs`
- Unit tests: happy path, cycles, diamonds, max depth, self-dep, missing dep, bundled passthrough, already-installed passthrough

### Phase 3: Install Integration (Wire it up)
- Integrate resolution into `install_from_git()` and `install_from_local()` in CLI
- Handle `--link` propagation
- Handle multi-skill picker auto-selection
- Lock file writes with `installed_as_dependency_of`

### Phase 4: Uninstall Orphan Cleanup
- Implement orphan detection in `uninstall` command
- Add `--remove-deps` flag
- Handle non-interactive mode
- Cascading orphan cleanup

### Phase 5: Transitive Runtime Resolution
- Replace one-level expansion in `match_skills()` with BFS
- Invert `test_no_transitive_dependencies` test
- Add `safe_always_on_skills()` exemption test
- Add disabled-mid-chain test

## Files Changed

| File | Change | Phase |
|------|--------|-------|
| `mika/crates/mika-agent/src/skills/marketplace.rs` | Add `dependencies` to `SkillCandidate`, `installed_as_dependency_of` to `MarketplaceEntry` | 1 |
| `mika/crates/mika-agent/src/skills/install.rs` | Add `resolve_dependencies()`, `ResolvedDep` struct | 2 |
| `mika/crates/mika-cli/src/commands/skills.rs` | Wire resolution into install flow, add `--remove-deps` to uninstall, orphan cleanup | 3, 4 |
| `mika/crates/mika-agent/src/skills/matcher.rs` | BFS transitive resolution in `match_skills()` | 5 |
| `mika/crates/mika-cli/src/cli.rs` | Add `--remove-deps` flag to `SkillsCommand::Uninstall` | 4 |

## Sources & References

- **Origin brainstorm:** [docs/brainstorms/2026-03-19-skill-dependency-management-brainstorm.md](docs/brainstorms/2026-03-19-skill-dependency-management-brainstorm.md) — Key decisions: auto-install without prompting, same-source resolution, --link propagation, full transitive resolution, uninstall prompts per orphan
- Install flow: `mika/crates/mika-agent/src/skills/install.rs` (install_skill_inner)
- Lock file: `mika/crates/mika-agent/src/skills/marketplace.rs` (MarketplaceEntry, MarketplaceLock)
- Runtime matcher: `mika/crates/mika-agent/src/skills/matcher.rs` (match_skills)
- CLI commands: `mika/crates/mika-cli/src/commands/skills.rs` (install_skill, install_from_git, install_from_local)
- Real-world dep example: `mika-skills/self-dev/skill.toml` (dependencies = ["build-mika", "claude-pilot", "browser-control"])

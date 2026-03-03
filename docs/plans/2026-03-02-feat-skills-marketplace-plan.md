---
title: "feat: Add git-based skills marketplace"
type: feat
status: active
date: 2026-03-02
brainstorm: docs/brainstorms/2026-03-02-skills-marketplace-brainstorm.md
---

# feat: Add git-based skills marketplace

## Overview

Add CLI commands (`mika skills install/uninstall/update`) that let users install community-created skills from Git repositories. Skills are cloned to a staging area, validated, and copied into the agent's `skills/` directory. A `marketplace.lock` file tracks installed skills with repo URL, path, and pinned commit hash.

## Problem Statement / Motivation

Mika's skill system is powerful but currently limited to bundled skills and locally-created skills. There is no way to share or distribute skills between users. A git-based marketplace allows the community to publish and install skills with zero infrastructure — just push a repo and share the URL.

## Proposed Solution

Three new CLI subcommands under `mika skills`:

- `install <url> [--name <alias>]` — Clone, scan, validate, copy, lock
- `uninstall <name>` — Remove directory + lock entry
- `update [name]` — Re-clone, re-extract, update lock

Plus integration changes: origin detection in `list_skills`, marketplace metadata in `info`, bundled re-sync skip.

## Technical Approach

### Architecture

```
User runs: mika skills install user/repo
                    |
                    v
          [1] Resolve URL (GitHub shorthand)
                    |
                    v
          [2] Check git available
                    |
                    v
          [3] Clone to temp dir (depth=1)
                    |
                    v
          [4] Scan for skill.toml (depth <= 2)
                    |
                    v
      +-----------+-----------+
      |           |           |
    0 found    1 found    N found
    (error)   (install)   (picker)
                    |           |
                    v           v
          [5] Validate manifest (parse skill.toml)
                    |
                    v
          [6] Check name collisions (bundled, custom, marketplace)
                    |
                    v
          [7] Copy skill dir to skills/<name>/ (no .git, symlink check)
                    |
                    v
          [8] Update marketplace.lock
                    |
                    v
          [9] Cleanup temp dir
                    |
                    v
          [10] Print success + security warning (if exec handlers)
```

### Implementation Phases

#### Phase 1: Lock File & Marketplace Data Model

**Files to create/modify:**
- `crates/mika-agent/src/skills/marketplace.rs` (NEW) — Lock file model + read/write
- `crates/mika-agent/src/skills/mod.rs` — Re-export marketplace module

**Lock file location:** `~/.mika/agents/<agent>/marketplace.lock` (agent root, NOT inside `skills/`)

**Rust data model:**

```rust
// crates/mika-agent/src/skills/marketplace.rs

use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct MarketplaceLock {
    #[serde(default)]
    pub skills: BTreeMap<String, MarketplaceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceEntry {
    /// Git clone URL
    pub url: String,
    /// Path within repo to the skill directory ("." for root)
    pub path: String,
    /// Pinned commit hash at install/update time
    pub commit: String,
    /// ISO 8601 timestamp of first install
    pub installed_at: String,
    /// ISO 8601 timestamp of last update
    pub updated_at: String,
}
```

**Functions:**

```rust
/// Read lock file. Returns empty MarketplaceLock if file doesn't exist or is corrupt (log warning).
pub fn read_lock(agent_home: &Path) -> MarketplaceLock

/// Write lock file atomically (write to .tmp, rename). Set 0o600 permissions.
pub fn write_lock(agent_home: &Path, lock: &MarketplaceLock) -> Result<()>

/// Check if a skill name is marketplace-installed.
pub fn is_marketplace_skill(agent_home: &Path, name: &str) -> bool
```

**Acceptance criteria:**
- [ ] `MarketplaceLock` serde roundtrips correctly (TOML)
- [ ] `read_lock` returns default on missing file
- [ ] `read_lock` logs warning and returns default on corrupt file
- [ ] `write_lock` uses atomic write (temp + rename)
- [ ] `write_lock` sets 0o600 permissions on Unix
- [ ] `is_marketplace_skill` returns true/false based on lock file

**Tests:** Unit tests for serde roundtrip, missing file, corrupt file, atomic write.

---

#### Phase 2: Git Operations Module

**Files to create/modify:**
- `crates/mika-agent/src/skills/git.rs` (NEW) — Git command wrappers
- `crates/mika-agent/src/skills/mod.rs` — Re-export

**Functions:**

```rust
/// Check if git is available on PATH. Returns version string or error.
pub fn check_git() -> Result<String>

/// Clone a repo to a temp directory. Uses --depth=1 for shallow clone.
/// Returns the temp directory path.
pub fn clone_to_temp(url: &str) -> Result<TempDir>

/// Get HEAD commit hash from a directory.
pub fn get_head_commit(repo_dir: &Path) -> Result<String>

/// Resolve GitHub shorthand: "user/repo" -> "https://github.com/user/repo.git"
/// Pass through full URLs unchanged.
pub fn resolve_url(source: &str) -> Result<String>
```

**URL resolution rules:**
- `https://...` or `git@...` or `ssh://...` → pass through
- `user/repo` (exactly one `/`, no protocol) → `https://github.com/user/repo.git`
- Everything else → error with helpful message

**Security:**
- Validate URL has no shell metacharacters before passing to `Command::new("git")`
- Use `Command::new("git").arg(url)` not `Command::new("sh").arg(format!("git clone {url}"))` — args are not shell-interpolated
- Scrub `MIKA_*` env vars from git process (defense-in-depth, matching github handler pattern)

**Acceptance criteria:**
- [ ] `check_git()` returns version or clear error
- [ ] `clone_to_temp()` clones to a `tempfile::TempDir`
- [ ] `clone_to_temp()` uses `--depth 1` for efficiency
- [ ] `clone_to_temp()` scrubs MIKA_* env vars from subprocess
- [ ] `get_head_commit()` returns 40-char hex hash
- [ ] `resolve_url()` handles GitHub shorthand, full HTTPS, SSH URLs
- [ ] `resolve_url()` rejects empty strings and obviously invalid input
- [ ] Temp dir is cleaned up automatically via `TempDir` drop

**Tests:** Unit tests for URL resolution (shorthand, https, ssh, invalid). Integration test for clone + commit hash (can use a known public repo or mock).

---

#### Phase 3: Skill Scanner for Cloned Repos

**Files to create/modify:**
- `crates/mika-agent/src/skills/marketplace.rs` — Add scanner function

**Function:**

```rust
/// Scan a cloned repo for skill.toml files up to depth 2.
/// Returns a list of (skill_name, relative_path) tuples.
/// Depth 0: ./skill.toml (single-skill repo at root)
/// Depth 1: ./foo/skill.toml (multi-skill repo, one level)
/// Depth 2: ./foo/bar/skill.toml (nested, two levels)
pub fn scan_repo_for_skills(repo_dir: &Path) -> Result<Vec<SkillCandidate>>

pub struct SkillCandidate {
    /// Manifest name from skill.toml
    pub name: String,
    /// Description from skill.toml
    pub description: String,
    /// Path relative to repo root (e.g., ".", "weather", "skills/weather")
    pub relative_path: String,
    /// Absolute path to the skill directory in the temp clone
    pub absolute_path: PathBuf,
}
```

**Scan depth:** Max 2 levels. The parent directory of each `skill.toml` is the skill directory.

**Validation during scan:**
- Parse `skill.toml` with existing `SkillManifest` parser
- Run `validate_skill_name()` on the manifest's `name` field
- Skip skills that fail validation (log warning, continue scanning)

**Acceptance criteria:**
- [ ] Finds `skill.toml` at repo root (single-skill repo)
- [ ] Finds `skill.toml` in immediate subdirectories (multi-skill repo)
- [ ] Finds `skill.toml` at depth 2 (nested structure)
- [ ] Does NOT recurse beyond depth 2
- [ ] Skips skills with invalid manifests (with warning)
- [ ] Returns empty vec for repos with no skills

**Tests:** Create temp dirs with various structures and verify scan results.

---

#### Phase 4: Install Command

**Files to create/modify:**
- `crates/mika-cli/src/cli.rs` — Add `Install` variant to `SkillsCommand`
- `crates/mika-cli/src/commands/skills.rs` — Implement `install` handler

**CLI signature:**

```
mika skills install <source> [--name <alias>]
```

**Implementation steps:**

1. `resolve_url(source)` — Resolve GitHub shorthand
2. `check_git()` — Verify git is available
3. `clone_to_temp(url)` — Clone to staging
4. `scan_repo_for_skills(temp_dir)` — Find skills
5. **Select skill(s):**
   - 0 found → error: "No valid skills found in repository"
   - 1 found → auto-select
   - N found → interactive picker (if TTY) or error (if not TTY: "Multiple skills found. Re-run with --name to select.")
6. **For each selected skill:**
   a. Determine install name: `--name` alias OR manifest `skill.name`
   b. Run `validate_skill_name(install_name)`
   c. Check collision: `is_bundled_skill(name)` → error "Collides with built-in skill. Use --name."
   d. Check collision: skill dir already exists → error "Skill '<name>' already exists. Use --name for a different name, or mika skills update to update."
   e. Copy skill directory to `skills/<name>/` (exclude `.git*` dirs, validate no symlink escapes)
   f. Update `marketplace.lock` with new entry
7. Print success message
8. If skill has exec handlers: print warning "This skill contains exec handlers that run shell commands. Review the source: <url>"

**Interactive picker (multi-skill):**
- Use `dialoguer::MultiSelect` or `dialoguer::Select` (add `dialoguer` dependency)
- Show: `[1] weather - "Get weather forecasts" (at: weather/)`
- Non-TTY: error with list of found skills and suggestion to use `--name`

**Copy logic:**
- Recursive directory copy from temp clone to `skills/<name>/`
- Skip `.git/` directories during copy
- For each file/dir: `canonicalize()` source, verify it starts with the skill directory prefix (symlink escape check)
- Preserve file permissions (git preserves execute bits; `std::fs::copy` preserves them)
- Do NOT set `chmod +x` automatically (user decision: trust git permissions)

**Acceptance criteria:**
- [ ] Installs single-skill repo from GitHub shorthand (`user/repo`)
- [ ] Installs single-skill repo from full HTTPS URL
- [ ] Installs from multi-skill repo with interactive picker (TTY)
- [ ] Errors cleanly for multi-skill repo without TTY
- [ ] `--name` alias works, directory named by alias
- [ ] Refuses install on bundled skill name collision
- [ ] Refuses install on existing skill name collision
- [ ] Lock file updated with correct URL, path, commit hash, timestamps
- [ ] Temp dir cleaned up on success and failure
- [ ] Security warning printed for exec handler skills
- [ ] `.git/` excluded from copied files
- [ ] Symlink escape prevented during copy
- [ ] Errors clearly when git is not installed
- [ ] Errors clearly on network failure

**Tests:** Integration tests with temp git repos (create repo in test, install from it).

---

#### Phase 5: Uninstall Command

**Files to create/modify:**
- `crates/mika-cli/src/cli.rs` — Add `Uninstall` variant to `SkillsCommand`
- `crates/mika-cli/src/commands/skills.rs` — Implement `uninstall` handler

**CLI signature:**

```
mika skills uninstall <name>
```

**Implementation steps:**

1. Check `is_bundled_skill(name)` → error "Cannot uninstall built-in skill. Use 'mika skills disable' instead."
2. Read `marketplace.lock`
3. If name not in lock → check if dir exists:
   - Dir exists → error "Skill '<name>' is not a marketplace skill. Remove it manually or use the delete_skill agent tool."
   - Dir doesn't exist → error "Skill '<name>' not found."
4. If name in lock but dir doesn't exist → remove from lock, print "Cleaned up stale lock entry for '<name>'."
5. Normal path: `verify_skill_path()`, `remove_dir_all()`, remove from lock, write lock
6. Print "Uninstalled '<name>'."

**Acceptance criteria:**
- [ ] Removes marketplace skill directory
- [ ] Removes lock file entry
- [ ] Protects bundled skills (suggests `disable`)
- [ ] Handles stale lock entries (dir manually deleted)
- [ ] Handles non-marketplace skills (suggests manual removal)
- [ ] Uses `verify_skill_path()` for symlink safety

**Tests:** Unit tests for each error case. Integration test for happy path.

---

#### Phase 6: Update Command

**Files to create/modify:**
- `crates/mika-cli/src/cli.rs` — Add `Update` variant to `SkillsCommand`
- `crates/mika-cli/src/commands/skills.rs` — Implement `update` handler

**CLI signature:**

```
mika skills update [name]
```

**Implementation steps (single skill):**

1. Read `marketplace.lock`
2. Look up entry by name → error if not found
3. `clone_to_temp(entry.url)`
4. `scan_repo_for_skills(temp_dir)` → find skill at `entry.path`
5. If skill not found at recorded path → error "Skill not found at path '<path>' in repo. It may have been moved or removed."
6. `get_head_commit(temp_dir)` → if same as locked commit → print "<name>: already up to date" and return
7. Remove existing skill directory
8. Copy new skill directory to `skills/<name>/` (same copy logic as install)
9. Update lock entry (new commit hash, updated_at timestamp)
10. Print "<name>: updated (abc123 -> def456)"

**Implementation steps (all skills — no name argument):**

1. Read `marketplace.lock`
2. If empty → print "No marketplace skills installed."
3. For each entry: run single-skill update logic
4. Continue on failure (don't stop on first error)
5. Print summary: "Updated 3/5 skills. Failed: skill-a (reason), skill-b (reason)"

**Acceptance criteria:**
- [ ] Updates single named skill
- [ ] Updates all marketplace skills when no name given
- [ ] Detects "already up to date" (same commit hash)
- [ ] Handles skill removed from repo (clear error, existing install untouched)
- [ ] Continues on partial failure when updating all
- [ ] Prints summary with success/failure counts
- [ ] Lock file updated with new commit hash and timestamp

**Tests:** Integration tests with temp git repos (commit change, verify update detects it).

---

#### Phase 7: Integration — Origin Detection & Existing Commands

**Files to modify:**
- `crates/mika-agent/src/tools/list_skills.rs` — Add `[marketplace]` origin
- `crates/mika-agent/src/tools/delete_skill.rs` — Handle marketplace skills (remove from lock)
- `crates/mika-agent/src/bundled_skills.rs` — Skip marketplace skills during re-sync
- `crates/mika-cli/src/commands/skills.rs` — Update `info` command to show marketplace metadata

**list_skills changes:**

```rust
// Current:
let origin = if is_bundled_skill(&name) { " [built-in]" } else { " [custom]" };

// New:
let origin = if is_bundled_skill(&name) {
    " [built-in]"
} else if is_marketplace_skill(&agent_home, &name) {
    " [marketplace]"
} else {
    " [custom]"
};
```

Note: `list_skills` tool needs access to `agent_home` to read the lock file. This may require adding `agent_home` to `ToolContext` if not already available, OR passing it as a parameter. Check if `ToolContext.home_dir` already points to the agent home (it does — `home_dir: PathBuf` in ToolContext).

**delete_skill changes:**

When deleting a marketplace skill, also remove the lock file entry. Read lock → remove entry → write lock. Same atomic write pattern.

**bundled_skills re-sync:**

The brainstorm decided to refuse install when name collides with a bundled skill. This means `seed_bundled_skills()` can NEVER encounter a marketplace skill with a bundled name. **No changes needed** to `seed_bundled_skills()`. The invariant is maintained by the install-time collision check.

Document this invariant with a code comment.

**info command enhancement:**

For marketplace skills, additionally show:
```
Source: https://github.com/user/repo
Path:   weather/
Commit: abc123def456
Installed: 2026-03-02T10:30:00Z
Updated:   2026-03-02T10:30:00Z
```

**Acceptance criteria:**
- [ ] `list_skills` shows `[marketplace]` for marketplace-installed skills
- [ ] `delete_skill` agent tool removes marketplace lock entry when deleting marketplace skills
- [ ] `mika skills info <name>` shows marketplace metadata
- [ ] `seed_bundled_skills()` has comment documenting invariant (no marketplace/bundled name overlap)
- [ ] `ToolContext.home_dir` is used to locate `marketplace.lock`

**Tests:** Unit tests for origin detection. Integration tests for list output format.

---

#### Phase 8: Documentation & ADR

**Files to create/modify:**
- `docs/adr/006-git-based-skills-marketplace.md` (NEW) — Architecture Decision Record
- `docs/skills.md` — Add "Marketplace" section
- `CLAUDE.md` — Update skills section with marketplace commands

**ADR-006 content:**
- Context: Need for skill distribution, no central infrastructure
- Decision: Git-based clone + lock file, per-agent scope
- Consequences: Requires git, no discovery mechanism, trust model

**docs/skills.md additions:**
- Installing marketplace skills
- Publishing skills (repo conventions)
- Lock file format reference
- Single-skill and multi-skill repo examples

**CLAUDE.md additions:**
- `mika skills install <url>` / `uninstall` / `update` commands
- Marketplace lock file mention

**Acceptance criteria:**
- [ ] ADR-006 written following existing ADR template
- [ ] docs/skills.md updated with marketplace section
- [ ] CLAUDE.md updated with new commands

---

## Dependency Graph

```
Phase 1 (Lock File Model)
    |
    +---> Phase 2 (Git Operations)
    |         |
    |         +---> Phase 3 (Skill Scanner)
    |                   |
    +-------------------+
    |
    +---> Phase 4 (Install Command) --- depends on Phases 1, 2, 3
    |
    +---> Phase 5 (Uninstall Command) --- depends on Phase 1
    |
    +---> Phase 6 (Update Command) --- depends on Phases 1, 2, 3
    |
    +---> Phase 7 (Integration) --- depends on Phase 1
    |
    +---> Phase 8 (Documentation) --- depends on all above
```

Phases 1, 2, 3 can be built sequentially as foundation.
Phases 4, 5, 6, 7 can be parallelized after the foundation is done.
Phase 8 is last.

## Technical Considerations

### New dependencies

- `tempfile` — For temp directories during clone/staging. Check if already in `Cargo.toml`.
- `dialoguer` — For interactive multi-skill picker. Lightweight, well-maintained. Only needed in `mika-cli`.

### Security

- **Exec handlers from untrusted repos:** Print warning during install. No sandboxing in v1. Users must review.
- **Symlink escape:** Validate during copy phase using `canonicalize()` + prefix check (reuse `verify_skill_path` pattern).
- **MIKA_* env scrubbing:** Git subprocess inherits environment but should scrub MIKA_* vars (defense-in-depth).
- **URL injection:** Use `Command::new("git").arg(url)` not shell interpolation. Args are not shell-expanded.

### Known limitations

- **No discovery mechanism.** Users must find skill repo URLs externally (GitHub search, community lists, READMEs).
- **No sandboxing.** Exec handler skills run with full user permissions.
- **No automatic chmod.** If a skill author forgets to set execute bits in git, exec handlers will fail at runtime.
- **No local path support.** v1 only supports HTTPS URLs and GitHub shorthand. Local `file://` paths deferred.
- **No rollback on update.** If update copies a broken skill, the old version is gone. Users can re-install from a specific commit in the future.

### Edge cases handled

| Edge case | Behavior |
|-----------|----------|
| Git not installed | Clear error: "git is required but not found on PATH" |
| Network failure during clone | Git error propagated, temp dir cleaned up |
| No skill.toml in repo | Error: "No valid skills found in repository" |
| Name collides with bundled skill | Error suggesting `--name` alias |
| Name collides with existing skill | Error suggesting `--name` or `update` |
| Lock file corrupt | Log warning, treat as empty (graceful degradation) |
| Lock file missing | Treat as no marketplace skills installed |
| Skill dir manually deleted | `uninstall` cleans up stale lock entry |
| Lock entry but no dir | `update` re-installs; `uninstall` removes entry |
| Repo removed skill path | `update` errors, existing install untouched |
| Non-TTY with multi-skill repo | Error listing found skills, suggests `--name` |
| Partial failure during update-all | Continue updating others, print summary |

## Acceptance Criteria

### Functional Requirements

- [ ] `mika skills install <url>` installs a skill from a git repo
- [ ] `mika skills install user/repo` resolves GitHub shorthand
- [ ] `mika skills install <url> --name alias` installs with custom name
- [ ] Multi-skill repos present interactive picker
- [ ] `mika skills uninstall <name>` removes skill and lock entry
- [ ] `mika skills update <name>` updates a specific skill
- [ ] `mika skills update` updates all marketplace skills
- [ ] `mika skills list` shows `[marketplace]` origin
- [ ] `mika skills info <name>` shows marketplace metadata
- [ ] `marketplace.lock` tracks all installed skills with URL, path, commit, timestamps

### Non-Functional Requirements

- [ ] All existing tests pass (no regressions)
- [ ] New unit tests for lock file model, URL resolution, skill scanner
- [ ] Integration tests for install/uninstall/update flows
- [ ] Symlink escape prevention in copy logic
- [ ] MIKA_* env vars scrubbed from git subprocess
- [ ] Atomic lock file writes (temp + rename)
- [ ] Graceful degradation on corrupt lock file

## References & Research

### Internal References

- Skills system: `crates/mika-agent/src/skills/` (index.rs, manifest.rs, executor.rs)
- Bundled skills: `crates/mika-agent/src/bundled_skills.rs:123-152`
- CLI commands: `crates/mika-cli/src/commands/skills.rs`
- Skill CRUD tools: `crates/mika-agent/src/tools/` (create, update, list, toggle, delete)
- Validators: `crates/mika-agent/src/tools/create_skill.rs:25-104`
- Home directory: `crates/mika-common/src/home.rs:61-67`
- ADR-002: `docs/adr/002-filesystem-skill-registry.md`
- ADR-005: `docs/adr/005-delete-skill-crud-completion.md`

### External Inspirations

- OpenClaw ClawHub: Three-tier distribution, content-hash versioning
- Claude Code plugins: GitHub-based distribution with marketplace.json
- Vim plugin managers (vim-plug): Git-clone-into-directory model
- Go modules: Git-based distribution with commit pinning

### Brainstorm

- `docs/brainstorms/2026-03-02-skills-marketplace-brainstorm.md`

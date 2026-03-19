---
title: "feat: Local source support for mika skills install with --link mode"
type: feat
status: completed
date: 2026-03-19
origin: docs/brainstorms/2026-03-02-skills-marketplace-brainstorm.md
issue: "#210"
---

# Local Source Support for `mika skills install` with `--link` Mode

## Overview

Extend the skills marketplace to accept local filesystem paths (absolute paths or `file://` URIs) as install sources, with an optional `--link` flag that creates symlinks instead of copies. This eliminates the commit-push-update cycle for skill authors developing locally.

## Problem Statement / Motivation

The marketplace install flow only supports remote git repositories. Skill authors managing local repos (e.g., `mika-skills/` with `self-dev`, `self-check`, `desktop`) must commit, push, then `mika skills update` before the agent sees changes. This friction slows iterative development.

Two capabilities solve this:
1. **Local path install** — snapshot copy from disk, updatable via `mika skills update`
2. **`--link` mode** — symlink to source directory, changes reflected immediately

Analogous to `npm link` and `pip install -e`.

## Proposed Solution

### Phase 1: Source Resolution Refactor

Rename `resolve_url` → `resolve_source` in `git.rs`, returning a typed enum:

```rust
// crates/mika-agent/src/skills/git.rs

pub enum SourceKind {
    Git(String),      // normalized git URL
    Local(PathBuf),   // canonicalized absolute path
}

pub fn resolve_source(input: &str) -> Result<SourceKind> {
    // Strip file:// prefix
    if let Some(path) = input.strip_prefix("file://") {
        let p = PathBuf::from(path);
        anyhow::ensure!(p.is_absolute(), "file:// URI must use absolute path");
        anyhow::ensure!(p.exists(), "path does not exist: {}", p.display());
        return Ok(SourceKind::Local(std::fs::canonicalize(&p)?));
    }
    // Bare absolute path
    let p = Path::new(input);
    if p.is_absolute() && p.exists() {
        return Ok(SourceKind::Local(std::fs::canonicalize(p)?));
    }
    // Fall through to existing git URL resolution
    Ok(SourceKind::Git(resolve_git_url(input)?))
}
```

The existing `resolve_url` is renamed to `resolve_git_url` (private) and called only from the `Git` branch.

### Phase 2: Lock File Schema Extension

Add `linked: bool` to `MarketplaceEntry` with backward-compatible serde default:

```rust
// crates/mika-agent/src/skills/marketplace.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceEntry {
    pub url: String,           // git URL or "file:///abs/path"
    pub path: String,          // "." for local sources
    pub commit: String,        // "" for local sources
    #[serde(default)]
    pub linked: bool,          // NEW — false for git/snapshot, true for symlink
    pub installed_at: String,
    pub updated_at: String,
}
```

Convention: local sources use `url = "file:///absolute/path/to/skill-dir"` and `commit = ""`. This lets `update_skill` detect local vs git by URL scheme, and linked vs snapshot by the `linked` field.

### Phase 3: Install Dispatch

Refactor `install_skill` in CLI `skills.rs` to route on `SourceKind`:

```
resolve_source(input)
    ├─ Git    → existing flow (clone → scan → pick → copy → lock)
    └─ Local  → scan in-place → pick → ...
                    ├─ --link  → validate → symlink → lock (linked=true)
                    └─ no flag → validate → copy    → lock (linked=false)
```

**Local install specifics:**
- Scan the local path with `scan_repo_for_skills()` (reuse existing scanner)
- If path points directly to a `skill.toml` dir, skip picker
- If multiple skills found, show interactive picker (same as git)
- Validate `skill.toml` parses correctly before install
- Reject if source is under the target `skills_dir` (self-referential)
- `--link` with git source → error: `"--link requires a local path source"`

**Symlink creation:**
- Always create absolute symlinks (canonicalize source first)
- Symlink points to the directory containing `skill.toml`, never a parent
- Use `std::os::unix::fs::symlink(src_skill_dir, dst_skill_dir)`

### Phase 4: Update Dispatch

Extend `update_skill` in `install.rs`:

```rust
fn update_skill(entry: &MarketplaceEntry, skill_name: &str, ...) -> Result<UpdateResult> {
    if entry.linked {
        // Linked skills track source automatically
        return Ok(UpdateResult::LinkedNoOp);
    }
    if entry.url.starts_with("file://") {
        // Local snapshot: re-copy from original path
        let src = url_to_path(&entry.url)?;
        anyhow::ensure!(src.exists(), "Source directory '{}' no longer exists. \
            The installed snapshot is still available. Remove and reinstall from the new location.", src.display());
        let dst = skills_dir.join(skill_name);
        std::fs::remove_dir_all(&dst)?;
        copy_skill_dir(&src, &dst)?;
        // Update timestamp in lock
        return Ok(UpdateResult::Updated);
    }
    // Existing git flow unchanged
}
```

**Update-all summary format:** `"Updated 1/3 skills. Linked (no-op): 1. Up to date: 0. Failed: 0."`

### Phase 5: Uninstall — Symlink Awareness

Modify `uninstall_skill` in `install.rs`:

```rust
fn uninstall_skill(agent_home: &Path, name: &str) -> Result<()> {
    // ... existing bundled check, lock check ...
    let skill_path = skills_dir.join(name);
    let meta = std::fs::symlink_metadata(&skill_path)?;
    if meta.file_type().is_symlink() {
        // Linked skill: remove symlink only (not target)
        std::fs::remove_file(&skill_path)?;
    } else {
        // Regular directory: existing verify + remove_dir_all
        verify_skill_path(&skill_path, &skills_dir)?;
        std::fs::remove_dir_all(&skill_path)?;
    }
    // Remove lock entry
}
```

### Phase 6: Broken Symlink Detection in `scan_skills_dir`

Add explicit check in `index.rs`:

```rust
// In scan_skills_dir, after getting dir_entry
let meta = std::fs::symlink_metadata(&path);
if let Ok(m) = &meta {
    if m.file_type().is_symlink() && !path.exists() {
        warn!("Broken symlink for skill '{}': target no longer exists at {:?}. \
               Reinstall or remove with 'mika skills uninstall {}'", name, std::fs::read_link(&path).ok(), name);
        continue; // skip this skill
    }
}
```

### Phase 7: CLI Changes

**`crates/mika-cli/src/cli.rs`:**
```rust
Install {
    /// Git URL, GitHub shorthand, or local path
    source: String,
    /// Install under a different name (alias)
    #[arg(long)]
    name: Option<String>,
    /// Create symlink instead of copy (local sources only)
    #[arg(long)]
    link: bool,
}
```

**`crates/mika-cli/src/commands/skills.rs`:**
- Pass `link` flag through to install dispatch
- Update `show_skill_detail` to display `Source: /path (linked)` or `Source: /path (local copy)`

### Phase 8: Display Changes

**`list_skills` tool** (`crates/mika-agent/src/tools/list_skills.rs`):
```rust
let origin = if is_bundled_skill(name) {
    " [built-in]"
} else if let Some(entry) = get_marketplace_entry(ctx.home_dir, name) {
    if entry.linked {
        " [marketplace/linked]"
    } else {
        " [marketplace]"
    }
} else {
    " [custom]"
};
```

**CLI `mika skills list`** — same `[marketplace/linked]` label.

### Phase 9: Documentation Updates

- **`docs/skills.md`** — Add "Local Sources" section documenting `mika skills install /path`, `file://` URIs, `--link` flag, update behavior differences
- **`docs/adr/006-git-based-skills-marketplace.md`** — Add decisions 11-13: local path support, symlink mode, absolute symlink invariant
- **`crates/mika-agent/src/skills/executor.rs`** — Add read-only invariant comment at top of module

## Technical Considerations

### Security

- **Linked skills have a different trust model.** Source can change between agent turns without any marketplace operation. The exec handler warning is already shown at install time; enhance it for `--link` to note the mutable-source risk.
- **Self-referential rejection.** Canonicalize source, reject if under `skills_dir`.
- **Absolute symlinks only.** Prevents breakage from CWD changes.
- **`seed_bundled_skills` already has symlink guards** (lines 180, 194 in `bundled_skills.rs`) — no changes needed there.

### Backward Compatibility

- `linked: bool` uses `#[serde(default)]` — old lock files without the field default to `false` (existing git installs unaffected)
- `resolve_source` falls through to git URL resolution for anything that isn't a local path — existing install commands work unchanged
- `scan_skills_dir` already follows symlinks via `path.is_dir()` — linked skills scan correctly without changes to the core scanner

### Docker/Container Context

`--link` pointing to host paths won't work inside containers. This is expected — `--link` is a developer convenience for local development. Document this limitation.

## System-Wide Impact

- **Interaction graph:** Install → lock file write → `scan_skills_dir` on next agent turn → skill available. For linked: immediate. For snapshot: after copy.
- **Error propagation:** Source-not-found on update → `anyhow` error → CLI prints message. No agent-side impact (installed copy remains).
- **State lifecycle risks:** Broken symlink → skill silently absent without new warning. Fixed by Phase 6.
- **API surface parity:** `list_skills` tool and CLI `mika skills list` both need the new origin label.

## Acceptance Criteria

- [x] `mika skills install /abs/path` copies skill files into `skills/<name>/`
- [x] `mika skills install file:///abs/path` works identically
- [x] Lock file records `url = "file:///..."`, `commit = ""`, `linked = false`
- [x] `mika skills install /path --link` creates symlink, lock has `linked = true`
- [x] `mika skills list` shows `[marketplace/linked]` for linked skills
- [x] Editing source has no effect on snapshot installs until `mika skills update`
- [x] Editing source takes effect immediately for linked installs
- [x] `mika skills update` on linked skill prints no-op message
- [x] `mika skills update` on local snapshot re-copies from source
- [x] `mika skills update` on local snapshot with missing source shows clear error
- [x] `mika skills update` (all) summary includes linked skip count
- [x] `--link` with git source rejected with clear error message
- [x] Non-existent path rejected with clear error message
- [x] Broken symlink at startup logs warning, skips skill, no crash
- [x] `mika skills uninstall` on linked skill removes symlink only (not target)
- [x] Self-referential path (source inside skills_dir) rejected
- [x] Multi-skill local repo shows interactive picker
- [x] Symlinks point to skill subdir, never repo root
- [x] `--name` alias works with both local and `--link` installs
- [x] Existing git install flow unchanged
- [x] Old lock files without `linked` field still parse correctly
- [x] All existing tests pass

## Files to Change

| File | Change |
|---|---|
| `crates/mika-agent/src/skills/git.rs` | Add `SourceKind` enum, rename `resolve_url` → `resolve_source` (keep `resolve_git_url` private) |
| `crates/mika-agent/src/skills/marketplace.rs` | Add `linked: bool` with `#[serde(default)]` to `MarketplaceEntry` |
| `crates/mika-agent/src/skills/install.rs` | Add local install path, symlink creation, symlink-aware uninstall, local update re-copy |
| `crates/mika-agent/src/skills/index.rs` | Add broken symlink detection + warning in `scan_skills_dir` |
| `crates/mika-cli/src/cli.rs` | Add `link: bool` field to `SkillsCommand::Install` |
| `crates/mika-cli/src/commands/skills.rs` | Route on `SourceKind`, pass `link` flag, update `show_skill_detail` and `list_skills` display |
| `crates/mika-agent/src/tools/list_skills.rs` | Add `[marketplace/linked]` origin variant |
| `crates/mika-agent/src/skills/executor.rs` | Add read-only invariant comment |
| `docs/skills.md` | Document local source and `--link` in marketplace section |
| `docs/adr/006-git-based-skills-marketplace.md` | Add decisions 11-13 for local source and link mode |

## Dependencies & Risks

- **`std::os::unix::fs::symlink`** — Unix-only. Windows not supported (documented as out of scope).
- **Risk: broken symlinks in production.** Mitigated by startup warning (Phase 6) and clear error on update.
- **Risk: mutable linked skills.** By design for dev workflow. Exec handler warning enhanced at install time.

## Sources & References

- **Origin brainstorm:** [docs/brainstorms/2026-03-02-skills-marketplace-brainstorm.md](docs/brainstorms/2026-03-02-skills-marketplace-brainstorm.md) — established marketplace architecture, lock file format, trust model
- **ADR-006:** [docs/adr/006-git-based-skills-marketplace.md](docs/adr/006-git-based-skills-marketplace.md) — design decisions for git-based distribution
- **Issue:** [#210](https://github.com/senara-solutions/mika/issues/210) — full spec with acceptance scenarios
- **Learnings:** Tilde expansion patterns (`docs/solutions/logic-errors/tilde-home-expansion-file-tools.md`), path validation (`docs/solutions/security-issues/team-workspace-ref-dir-validation-hardening.md`), skill validation infrastructure (`docs/solutions/integration-issues/skills-doc-code-drift-and-validation-infrastructure.md`)

---
status: complete
priority: p2
issue_id: 302
tags: [code-review, security, bundled-skills]
dependencies: []
---

# Add symlink guard to seed_bundled_skills write path

## Problem Statement

`seed_bundled_skills()` now always overwrites bundled skill files on startup. If an attacker replaces a bundled skill directory with a symlink before the application starts, `write_skill()` would follow the symlink and write files (including executable handler scripts with mode 0o700) to an arbitrary location.

This is a defense-in-depth concern. Practical exploitability is low because the attacker would need container access, and the written content is compile-time embedded (not attacker-controlled). However, `create_skill` already has a symlink guard (todo #281), and `seed_bundled_skills` should have parity now that it overwrites existing directories.

## Findings

- **Security Sentinel:** The `write_skill` function at `bundled_skills.rs:141` does not check for symlinks before writing. Between `skill_dir.exists()` (line 125) and `write_skill()` (line 127), the directory could be replaced with a symlink (TOCTOU).
- **Learnings Researcher:** Related to completed todo #281 (symlink guard for create_skill). Same pattern should be applied here.

## Proposed Solutions

### Solution A: Symlink check before write_skill (Recommended)

Add a symlink check at the top of `write_skill()`:

```rust
fn write_skill(skill_dir: &Path, skill: &BundledSkill) -> std::io::Result<()> {
    if skill_dir.exists() && skill_dir.symlink_metadata()?.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "skill directory is a symlink, refusing to write",
        ));
    }
    // ... existing write logic
}
```

- Effort: Small
- Risk: Low — only adds a guard, no behavior change for normal operation

### Solution B: Canonicalize + containment check (matching create_skill)

Use the same pattern as `create_skill`: canonicalize the path after creation and verify it's still within the expected skills directory. More robust against mount-point attacks.

- Effort: Small
- Risk: Low

## Technical Details

- **Affected files:** `crates/mika-agent/src/bundled_skills.rs` (write_skill function)
- **Related:** Todo #281 (create_skill symlink guard, complete)

## Acceptance Criteria

- [ ] `write_skill()` refuses to write into symlinked skill directories
- [ ] Warning logged when a symlink is detected
- [ ] Existing tests still pass
- [ ] New test: create symlink to tempdir, verify seed_bundled_skills logs warning and skips

## Work Log

- 2026-02-26: Created during code review of cross-channel polling + bundled skill update PR

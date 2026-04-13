---
title: "fix: Canonicalize both paths in handler symlink containment check"
type: fix
status: active
date: 2026-04-13
---

# fix: Canonicalize both paths in handler symlink containment check

## Overview

The skills validator's symlink containment check produces false-positive "resolves outside skill directory" warnings for every exec handler in symlinked skills. The fix is to canonicalize both the handler path and the skill directory before comparing them.

## Problem Frame

When a skill is installed via `--link` (symlink), the skill directory inside the agent home (e.g., `~/.mika/agents/mika-qa/skills/qa-review`) is a symlink pointing to the source directory (e.g., `mika-skills/qa-review`). The validator canonicalizes the handler command path (which resolves the symlink), but compares against the non-canonical skill directory. Since the canonical handler path is under the symlink target while the skill directory is the symlink source, `starts_with()` always fails — producing a warning for every handler in every linked skill.

This erodes operator trust in validator output and discourages use of `--link` mode for active skill development.

## Requirements Trace

- R1. Symlinked skills with handlers inside the skill directory must not produce false-positive "resolves outside" warnings
- R2. Handlers that genuinely escape the skill directory (e.g., `../escapeme.sh`) must still produce warnings
- R3. Unit tests cover both the symlink false-positive case and the genuine-escape case

## Scope Boundaries

- Only the symlink containment check in `validate_skill()` is affected
- No changes to skill installation, loading, or execution logic
- No changes to `validate_and_resolve_path()` in `tools/mod.rs` (different purpose)

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/skills/index.rs:773-774` — the bug: `cmd_path.canonicalize()` compared against non-canonical `skill_dir`
- `crates/mika-agent/src/skills/index.rs:637` — `validate_skill(skill_dir: &Path)` function signature
- Existing test pattern: `tempfile::tempdir()` + filesystem setup + call `validate_skill()` + assert on diagnostics

## Key Technical Decisions

- **Canonicalize `skill_dir` alongside `cmd_path`:** Both paths go through `canonicalize()` so symlinks on either side are resolved consistently. If either canonicalization fails, the check is silently skipped (existing behavior for `cmd_path`; extending to `skill_dir` is consistent).

## Implementation Units

- [x] **Unit 1: Fix canonicalization comparison**

**Goal:** Canonicalize both `cmd_path` and `skill_dir` before the `starts_with()` comparison.

**Requirements:** R1, R2

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/skills/index.rs`
- Test: `crates/mika-agent/src/skills/index.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- Change the `if let` at line 773 from canonicalizing only `cmd_path` to canonicalizing both paths:
  ```
  if let (Ok(canonical_cmd), Ok(canonical_dir)) = (cmd_path.canonicalize(), skill_dir.canonicalize())
      && !canonical_cmd.starts_with(&canonical_dir)
  ```
- No other code changes needed — the warning message and diagnostic level remain the same.

**Patterns to follow:**
- Existing `if let Ok(canonical) = ...` pattern at the same site
- Existing test patterns using `tempfile::tempdir()` and `validate_skill()`

**Test scenarios:**
- Happy path: symlinked skill with handler inside — `tmpdir/source/handlers/h.sh` + symlink `tmpdir/agent/skills/my-skill -> tmpdir/source` — call `validate_skill()` on the symlink path, assert zero "resolves outside" warnings
- Happy path: non-symlinked skill with handler inside — no warnings
- Error path: handler escaping via `../escapeme.sh` — assert warning is produced
- Edge case: symlinked skill where handler itself is a symlink pointing outside the skill — assert warning is produced

**Verification:**
- `cargo test -p mika-agent -- test_validate_skill_symlinked_handler` passes
- `cargo test -p mika-agent -- test_validate_skill_handler_escape` passes
- `cargo clippy -p mika-agent` clean

## System-Wide Impact

- **Interaction graph:** None — `validate_skill()` is a pure diagnostic function with no side effects
- **Error propagation:** Warnings are collected in `Vec<SkillDiagnostic>` and displayed; no change to propagation
- **Unchanged invariants:** Startup validation (`SkillRegistry::validate_loaded()`) and `is_skip_worthy_failure()` classification are unaffected — they only act on `Fail` diagnostics, not `Warn`

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| `canonicalize()` on `skill_dir` could fail for non-existent paths | Both canonicalizations are in a single `if let` tuple — if either fails, the entire check is skipped (safe default) |

## Sources & References

- Related issue: #526

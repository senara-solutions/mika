---
title: Skills validator handler-path canonicalization false positive on symlinked skills
date: 2026-04-13
category: logic-errors
module: mika-agent/skills
problem_type: logic_error
component: tooling
symptoms:
  - "validate_skill() emits 'handler command resolves outside skill directory' warning for every exec handler in symlinked skills"
  - "mika skills validate produces spurious warnings when skill is installed via --link"
root_cause: logic_error
resolution_type: code_fix
severity: low
tags:
  - skills-validator
  - symlink
  - canonicalize
  - false-positive
  - handler-path
  - link-mode
---

# Skills validator handler-path canonicalization false positive on symlinked skills

## Problem

`mika --agent <name> skills validate <skill>` produced false-positive "resolves outside skill directory" warnings for every exec handler when the skill was installed via symlink (`mika skills install <path> --link`). The warnings eroded operator trust in validator output and discouraged use of `--link` mode for active skill development.

## Symptoms

- Every exec handler in a `--link`-installed skill triggered: `[WARN] tool '<name>': handler command '<path>' resolves outside skill directory`
- The handler files were actually inside the skill directory — no real escape
- Non-symlinked skills validated cleanly

## What Didn't Work

The original containment check canonicalized only the handler command path (`cmd_path.canonicalize()`) but compared against the raw (non-canonical) skill directory. For symlinked skills, the canonical handler path landed in the symlink target (e.g., `mika-skills/qa-review/handlers/...`) while the skill directory was the symlink source (`~/.mika/agents/mika-qa/skills/qa-review`). The two paths could never share a prefix.

## Solution

Canonicalize **both** the handler path and the skill directory before comparison:

```rust
// Before — only cmd_path canonicalized:
if let Ok(canonical) = cmd_path.canonicalize()
    && !canonical.starts_with(skill_dir)

// After — both paths canonicalized (#526):
if let (Ok(canonical_cmd), Ok(canonical_dir)) = (
    cmd_path.canonicalize(),
    skill_dir.canonicalize(),
) && !canonical_cmd.starts_with(&canonical_dir)
```

File: `crates/mika-agent/src/skills/index.rs`, inside the exec handler validation loop in `validate_skill()`.

## Why This Works

`Path::canonicalize()` resolves all symlinks, `.`, and `..` components, returning the absolute real path. When both sides are canonicalized, `starts_with()` compares real filesystem locations — the symlink indirection is transparent. Handlers genuinely inside the skill directory (even through symlinks) produce matching prefixes, while handlers that escape via `../` or external symlinks still fail the check.

If either `canonicalize()` call fails (e.g., path deleted between existence check and canonicalization), the `if let` tuple match silently skips the check — consistent with the pre-existing fail-open behavior for `cmd_path.canonicalize()`.

## Prevention

- When comparing filesystem paths for containment, always canonicalize both sides. Raw path comparison breaks when either side involves symlinks.
- The `--link` install mode is the recommended workflow for active skill development (per CLAUDE.md). Validator checks must work correctly for linked skills, not just copied ones.
- Three unit tests now cover the containment check: symlinked skill (no false positive), handler escape via `../` (still warns), and handler symlink pointing outside (still warns).

## Related Issues

- Issue: #526
- Related doc: `docs/solutions/integration-issues/is-legacy-format-false-positive-on-valid-skills.md` — prior false-positive in skill validator (different check)
- Related doc: `docs/solutions/architecture-patterns/local-source-skills-install-link-mode.md` — `--link` mode implementation and canonicalization rules
- Related doc: `docs/solutions/architecture-patterns/startup-skill-validation-structural-enforcement.md` — startup validation (#530) that surfaces these diagnostics

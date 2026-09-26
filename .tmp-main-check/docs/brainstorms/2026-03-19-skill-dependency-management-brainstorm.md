# Skill Dependency Management

**Date:** 2026-03-19
**Status:** Draft
**Scope:** mika (install/runtime), mika-skills (manifests)

## What We're Building

Extend the skill installation system so that `mika skills install` automatically resolves and installs skill dependencies declared in `skill.toml`. Today, the `dependencies` field only affects runtime activation (which skills' tools are available when a skill triggers). It is completely ignored during installation, leaving users to manually install each dependency.

**Motivating example:** Installing `self-dev` with `--link` for `mika-dev` should also link `build-mika` (and any other declared dependencies) automatically, with clear log output showing what was installed and why.

## Why This Approach

The skill ecosystem is growing. Manual dependency management doesn't scale and leads to silent failures — a skill installs successfully but breaks at runtime because a dependency is missing. The `dependencies` field already exists in the manifest; we just need the installer to act on it.

## Key Decisions

### 1. Auto-install without prompting

When a skill is installed, its dependencies are automatically resolved and installed. Each dependency installation is logged (e.g., `Installing dependency: build-mika (from ../build-mika/)`). No interactive confirmation — the dependency list is in the manifest, so the user implicitly agreed by choosing to install the parent skill. Installation fails if any required dependency can't be resolved.

### 2. Same-source resolution strategy

Dependencies are resolved by name using a "same source first" strategy:

- **Local path install:** Look for a sibling directory with the dependency name in the same parent directory as the source skill. E.g., installing from `/path/to/mika-skills/self-dev/` looks for `/path/to/mika-skills/build-mika/`.
- **Git install:** Look for the dependency in the same cloned repository (skills repos often contain multiple skills).
- **Bundled skill:** If the dependency name matches a bundled skill, it's already present — skip installation, just verify it exists and isn't disabled.
- **Already installed:** If the dependency is already installed for the target agent, skip it (log: `Dependency already installed: build-mika`).
- **Not found:** Fail the install with a clear error: `Dependency "foo" not found in source or as bundled skill`.

### 3. All dependencies are required

No optional/required distinction. Every dependency listed in `dependencies` must be resolvable or the install fails. Bundled skills are always present, so listing them as dependencies is fine — they just pass resolution immediately.

### 4. `--link` propagates to dependencies

When the parent skill is installed with `--link`, dependencies resolved from the same source directory are also linked. This matches the development workflow where you want live changes to all related skills. Dependencies resolved from other sources (bundled, already installed) are unaffected.

### 5. Full transitive dependency resolution

Dependencies are resolved recursively. If skill A depends on B, and B depends on C, installing A installs both B and C. Circular dependencies are detected and produce a clear error. This replaces the current one-level-only runtime behavior — both install-time and runtime should resolve the full tree.

**Cycle detection:** Track the resolution chain. If a skill name appears twice in the chain, error with the cycle path (e.g., `Circular dependency detected: A -> B -> C -> A`).

### 6. Uninstall asks per dependency

When uninstalling a skill, check each of its dependencies. For dependencies not required by any other installed skill, prompt the user: `Dependency "build-mika" is no longer needed by any other skill. Remove it? [y/N]`. This prevents accidental removal of shared dependencies while keeping the system clean.

### 7. Manifest format unchanged

The existing `dependencies = ["build-mika", "claude-pilot", "browser-control"]` format in `skill.toml` is sufficient. No schema changes needed — the names are resolved against the source and bundled skill registry.

## Affected Code

| File | Change |
|------|--------|
| `mika/crates/mika-agent/src/skills/install.rs` | Add dependency resolution during `install_skill()` and `install_skill_linked()` |
| `mika/crates/mika-agent/src/skills/manifest.rs` | No schema change needed, but add helper to parse dependencies |
| `mika/crates/mika-agent/src/skills/marketplace.rs` | Track which skills were installed as dependencies in the lock file (new field: `installed_as_dependency_of`) |
| `mika/crates/mika-agent/src/skills/matcher.rs` | Update runtime resolution to be transitive (remove one-level limit) |
| `mika/crates/mika-cli/src/commands/skills.rs` | Wire dependency install into CLI install flow, add uninstall dependency prompts |
| `mika/crates/mika-agent/src/skills/git.rs` | Support scanning for sibling skills in cloned repos |

## Open Questions

None — all key decisions resolved during brainstorm.

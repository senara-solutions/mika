---
title: "Add Contributing Guide"
type: docs
status: completed
date: 2026-03-02
---

# Add Contributing Guide

## Overview

Add a `CONTRIBUTING.md` file explaining that Mika is developed using Claude Code with the compound engineering plugin, and that contributors should use the `/mika` command as the primary development workflow. Update `README.md` to link to the new guide.

## Problem Statement / Motivation

- No `CONTRIBUTING.md` exists in the repository
- The README's "Development" section lists only 5 cargo commands with no workflow guidance
- The `/mika` command (Claude Code slash command) chains plan → work → review → resolve todos → doc audit → compound, but this workflow is undiscoverable to new contributors
- Commit conventions (conventional commits for release-plz) are not documented for contributors
- Quality gates (fmt, clippy -D warnings, test) are only visible in CI config

## Proposed Solution

Create a concise `CONTRIBUTING.md` with these sections:

1. **Introduction** — Brief statement that the project uses Claude Code + compound engineering plugin
2. **Prerequisites** — Rust >= 1.91, system deps (`jq`), Claude Code + compound engineering plugin (recommended)
3. **Getting Started** — Clone, build, run tests
4. **Development Workflow with Claude Code** — The `/mika` command explained step by step
5. **Manual Workflow** — Equivalent steps for contributors without Claude Code
6. **Branch Naming** — `type/description-kebab-case` (feat/, fix/, refactor/, docs/, etc.)
7. **Commit Conventions** — Conventional commits with exact prefixes from `release-plz.toml`: feat, fix, refactor, perf, doc (changelog-visible) and test, ci, chore, style (skipped in changelog)
8. **Quality Gates** — Exact CI commands: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`
9. **Testing** — Inline `#[cfg(test)] mod tests`, no API key required for tests
10. **Documentation** — Point to `/mika-doc-audit` and list which docs to update for which changes
11. **Architecture Decision Records** — When to write an ADR, where they live (`docs/adr/`), numbering scheme
12. **Security** — Env var redaction patterns, no secrets in logs, `env_clear()` for child processes

Update `README.md`:
- Add a "Contributing" row to the Documentation table (audience: Contributors)
- Add a one-liner in the Development section linking to CONTRIBUTING.md
- Fix test count inconsistency (~703 → ~745)

## Technical Considerations

- Keep CONTRIBUTING.md concise and scannable — this is a contributor's first touchpoint
- Claude Code is a **strong recommendation**, not a hard requirement — include manual fallback
- The compound engineering plugin provides `/workflows:*`, `/compound-engineering:*`, and `/ralph-loop:*` namespaces — list the plugin name and link to installation
- Tests are fully mocked (no API key needed) — this is a selling point for easy onboarding
- The `/mika` command in `.claude/commands/mika.md` is the source of truth for the workflow

## Acceptance Criteria

- [x] `CONTRIBUTING.md` exists at repo root
- [x] Explains Claude Code + compound engineering plugin as the recommended dev tool
- [x] Documents the `/mika` command and what each step does
- [x] Includes manual workflow equivalent (cargo commands + manual review)
- [x] Lists exact conventional commit prefixes matching `release-plz.toml`
- [x] Lists exact CI quality gate commands
- [x] Mentions system dependencies (Rust >= 1.91, `jq`)
- [x] States that tests do not require an API key
- [x] Links to existing docs (architecture, ADRs, skills, etc.)
- [x] `README.md` updated: Documentation table row + Development section link + test count fix
- [x] No new Rust code changes — documentation only

## Dependencies & Risks

- **Risk:** Claude Code plugin installation instructions may change — link to official docs rather than inlining steps
- **Risk:** Test count will continue drifting — use "~750" or similar approximate number
- **Dependency:** None — this is a pure documentation change

## Files to Create/Modify

- **Create:** `CONTRIBUTING.md` (repo root)
- **Modify:** `README.md` (add Contributing link, fix test count)

## References & Research

- `.claude/commands/mika.md` — The `/mika` command definition
- `.claude/commands/mika-doc-audit.md` — Doc audit rules
- `release-plz.toml` — Commit prefix parsers (lines 42-52)
- `.github/workflows/ci.yml` — CI quality gates
- `docs/solutions/ci-cd/rust-workspace-release-plz-github-actions.md` — CI/CD solution doc

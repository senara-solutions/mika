---
status: complete
priority: p2
issue_id: 368
tags: [code-review, documentation, publishing, rename]
dependencies: []
---

# Stale mika-cli references in documentation after rename to mika-ai

## Problem Statement

The package was renamed from `mika-cli` to `mika-ai` in commit 2eca502, but several user-facing and developer-facing documents still reference the old package name. The directory `crates/mika-cli/` was intentionally kept (only the Cargo package name changed), but docs that reference the package name (vs. directory path) need updating for consistency.

## Findings

**High-visibility (user-facing):**

1. `docs/getting-started.md:50` — `cargo install --path crates/mika-cli` (should add `cargo install mika-ai` as primary install method)
2. `docs/architecture.md:63` — Crate table shows `mika-cli` as the package name (should be `mika-ai`)
3. `README.md:83` — Shows `mika-cli/` in project structure without noting the published name is `mika-ai`

**Developer-facing:**

4. `CLAUDE.md:27` — Directory structure entry doesn't mention `mika-ai` package name
5. `docs/slash-commands.md:11` — References `crates/mika-cli/src/tui/commands/mod.rs`

**Low-priority (internal reference files):**

6. ~60 `todos/*.md` files reference `crates/mika-cli/src/...` paths
7. Several `docs/plans/` and `docs/solutions/` files reference old paths

## Proposed Solutions

### Option 1: Update package name references only (keep directory name)
- Update docs 1-5 to clarify: directory is `crates/mika-cli/`, published package is `mika-ai`
- Leave todos and plan files as-is (they reference directory paths which are still correct)
- **Pros:** Minimal churn; directory paths remain valid
- **Cons:** Perpetual divergence between package and directory names
- **Effort:** Small
- **Risk:** Low

### Option 2: Rename directory to `crates/mika-ai/` + bulk update
- Rename directory from `crates/mika-cli/` to `crates/mika-ai/`
- Bulk replace `crates/mika-cli` → `crates/mika-ai` across all files
- Update workspace `Cargo.toml` members pattern (uses `crates/*` glob, so no change needed)
- **Pros:** Full consistency; no naming divergence
- **Cons:** Large diff touching ~80+ files; Dockerfile paths break; more review needed
- **Effort:** Medium
- **Risk:** Medium (many files touched, Docker builds affected)

## Recommended Action

(To be decided during triage)

## Technical Details

- **Affected files:** README.md, CLAUDE.md, docs/getting-started.md, docs/architecture.md, docs/slash-commands.md, ~60 todos files, ~10 docs/plans and docs/solutions files
- **Affected components:** Documentation, developer experience

## Acceptance Criteria

- [ ] `docs/getting-started.md` shows `cargo install mika-ai` as primary install command
- [ ] `docs/architecture.md` crate table shows correct package name `mika-ai`
- [ ] `README.md` clarifies the package name vs directory name
- [ ] CLAUDE.md notes the mika-ai package name

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-01 | Created from code review of commit 2eca502 | Multiple agents (architecture, pattern, agent-native) independently flagged same docs |

## Resources

- Commit: 2eca502 "Prepare crates for publishing to crates.io"

---
status: complete
priority: p3
issue_id: 234
tags: [code-review, documentation, simplification]
dependencies: [233]
---

# Reduce Documentation Verbosity

## Problem Statement

Some documentation files are more verbose than necessary. Inline Rust code snippets in architecture.md will go stale as the codebase evolves. Manual tables of contents add maintenance burden.

## Findings

1. **slash-commands.md** is 454 lines for 13 commands — could be more concise while retaining all information.

2. **architecture.md** includes Rust code snippets (struct definitions, function signatures) that will diverge from actual code over time. Better to describe behavior and reference file paths.

3. **Manual ToCs** in skills.md and deployment.md will go stale as sections change.

4. **getting-started.md** CLI subcommands table could be shortened to just the essentials with a note to run `mika --help`.

## Proposed Solutions

### Solution A: Trim and reference (Recommended)
- Remove inline Rust code from architecture.md, replace with file:line references
- Remove manual ToCs (readers can use editor/GitHub outline)
- Condense slash-commands.md formatting
- Shorten CLI table in getting-started.md
- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] No inline Rust code snippets in architecture.md
- [ ] No manual ToCs
- [ ] slash-commands.md under 350 lines
- [ ] getting-started.md CLI section condensed

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-25 | Created from review | Code simplicity reviewer identified verbosity issues |

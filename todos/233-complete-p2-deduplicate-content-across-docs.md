---
status: complete
priority: p2
issue_id: 233
tags: [code-review, documentation, duplication]
dependencies: []
---

# Deduplicate Content Across Documentation Files

## Problem Statement

Multiple documentation files reproduce the same tables, directory layouts, and reference material. This creates a maintenance burden — when the codebase changes, multiple files must be updated in sync, increasing the chance of drift.

## Findings

1. **Directory layout** reproduced in 3 docs (getting-started.md, configuration.md, architecture.md). Should live in one place with cross-references.

2. **Slash command table** duplicated between getting-started.md and slash-commands.md. getting-started.md should link to the reference instead.

3. **Gateway env vars** duplicated between configuration.md and deployment.md. Configuration.md is the canonical source; deployment.md should reference it.

4. **Gateway endpoints table** duplicated between architecture.md and deployment.md.

5. **Token generation command** (`openssl rand -hex 32`) repeated 6+ times in deployment.md. Should appear once with back-references.

6. **CLI subcommands table** in getting-started.md duplicates `mika --help` output.

## Proposed Solutions

### Solution A: Cross-reference with links (Recommended)
- Keep canonical content in one file
- Replace duplicates with "See [Configuration Reference](configuration.md#section)" links
- Keep minimal context at each reference point (1-2 sentences)
- **Effort:** Medium
- **Risk:** Low

## Acceptance Criteria

- [ ] Directory layout appears in only one file (configuration.md)
- [ ] Slash command details only in slash-commands.md
- [ ] Gateway env vars only in configuration.md
- [ ] Token generation appears once in deployment.md
- [ ] Each removed duplicate replaced with a cross-reference link

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-25 | Created from review | Code simplicity reviewer identified ~247 LOC of duplication |

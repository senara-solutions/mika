---
title: Automated documentation audit step in development workflow
date: 2026-02-27
category: documentation-gaps
severity: medium
component:
  - .claude/commands/mika.md (project-level slash command)
  - .claude/commands/mika-doc-audit.md (standalone doc audit command)
  - ~/.claude/commands/letsgo.md (global slash command)
  - CLAUDE.md (project instructions)
tags:
  - documentation-drift
  - workflow-automation
  - slash-commands
  - claude-code
  - developer-experience
symptoms: |
  Documentation drifted from code reality because the automated development workflow
  (plan -> work -> review -> resolve -> compound) had no documentation audit step.
  Real example: OAuth subscription token support was fully implemented but three
  user-facing docs (README, getting-started, configuration) were never updated.
  CLAUDE.md test count was stale (~236 vs actual ~475).
root_cause: >
  Documentation updates were treated as optional post-implementation polish rather
  than an integral part of the development workflow. The automated pipeline had no
  enforcement mechanism to validate that project metadata, user-facing guides, and
  architecture docs stayed synchronized with code changes.
---

# Automated documentation audit step in development workflow

## Problem

The Mika project's development workflow (global `/letsgo` Claude Code command) chained
plan -> work -> review -> resolve -> compound but had no documentation audit step.
This caused documentation to drift from code reality.

**Real-world example:** OAuth subscription token support was fully implemented in code
(commit `44916b5`) with auto-detection logic, but three user-facing documentation files
were never updated (fixed reactively in commit `749b6db`):
- `README.md` — Quick Start only mentioned API keys
- `docs/getting-started.md` — No mention of OAuth tokens
- `docs/configuration.md` — Settings tables only referenced API keys

Additionally, `CLAUDE.md` had a stale test count (~236 tests, actual was ~475 after
the skills system, Layer 3 vector search, and other additions).

## Root Cause

Documentation updates were treated as optional polish rather than a feature deliverable.
The automated pipeline had no enforcement mechanism. Developer discipline alone is
insufficient because:
- Developers juggle competing priorities
- A feature "works" without docs — no immediate penalty for skipping
- Documentation debt compounds silently until discovered by new users

## Solution

### 1. Created project-level `/mika` command

File: `.claude/commands/mika.md`

8-step pipeline with documentation audit between resolve-todos and compound:

```
1. /ralph-loop (unconditional)
2. /workflows:plan $ARGUMENTS
3. /workflows:work
4. /workflows:review
5. /compound-engineering:resolve_todo_parallel
6. /mika-doc-audit (checks git diff, updates affected docs)
7. /workflows:compound
8. Output completion promise
```

### 2. Extracted `/mika-doc-audit` command (Step 6)

The audit is extracted into a standalone command (`.claude/commands/mika-doc-audit.md`) so it can be run independently. It is conditional and systematic:

- **Always audit:** `CLAUDE.md` for accuracy (architecture, conventions, commands,
  env vars, test count, schema version, pending work)
- **If new env vars:** Update `.env.example` and `docs/configuration.md`
- **If schema/DB changes:** Update `docs/architecture.md` and CLAUDE.md Architecture
- **If new CLI commands or tools:** Update `README.md`, `docs/getting-started.md`,
  `docs/slash-commands.md`
- **If skill changes:** Update `docs/skills.md`
- **If infra changes** (Helm, Docker, K8s): Update `docs/deployment.md`
- **If new config fields:** Update `docs/configuration.md`

### 3. Updated global `/letsgo`

Removed stale conditional ("Start with step 2 now (or step 1 if ralph-wiggum is
available)") since ralph-wiggum is always installed. Now unconditionally starts
with ralph-loop as step 1.

### 4. Caught stale metadata during first audit

Test count in CLAUDE.md: ~236 -> ~475. Directory structure updated to include
`.claude/commands/`.

## Verification

- `.claude/commands/mika.md` created with correct frontmatter and pipeline steps
- Global `~/.claude/commands/letsgo.md` updated (conditional removed)
- `CLAUDE.md` test count corrected, directory structure updated
- `cargo fmt && cargo clippy && cargo test` — all 475 tests pass

## Prevention

### The "Documentation-first checklist" pattern

Treat documentation as a first-class feature component, not optional polish:

1. **Design phase:** Document intended behavior in plans
2. **Implementation phase:** Update internal docs (CLAUDE.md, .env.example, code comments)
3. **Audit phase:** Run the automated checklist against git diff
4. **Review gate:** Documentation audit results are part of the workflow pipeline
5. **Compound phase:** Only after audit passes does the agent document the solution

### Key insight: enforcement via workflow automation

Manual reminders ("remember to update docs") fail because there is no immediate
penalty for skipping. Automated enforcement works because:
- Friction is applied at the right moment (after code review, before shipping)
- No judgment required — the checklist is objective
- Scalable — applies the same logic to every change
- Feedback is specific — reports which files are likely affected

### When to extend the checklist

Update the audit checklist in `/mika-doc-audit` when:
- New documentation files are added to the project
- New crates or binaries are introduced
- New public APIs or configuration fields are created
- New deployment targets or infrastructure components are added

### Known limitations

- Agent-driven, so quality depends on diff analysis heuristics
- Cannot determine developer intent from code alone
- Large multi-file diffs are harder to reason about
- Checklist coverage is point-in-time — new doc files need manual addition

## Related

- `docs/solutions/documentation-gaps/oauth-subscription-auth-docs.md` — The original
  documentation drift incident that motivated this solution
- `todos/066-complete-p2-stale-claude-md.md` — CLAUDE.md staleness tracking
- `todos/107-complete-p3-claudemd-stale-async-db.md` — CLAUDE.md stale async DB refs
- `todos/058-complete-p2-stale-readme-encryption-refs.md` — README stale encryption refs
- `todos/233-complete-p2-deduplicate-content-across-docs.md` — Documentation duplication
  as a drift vector
- `docs/plans/2026-02-27-feat-mika-development-workflow-command-plan.md` — Implementation plan

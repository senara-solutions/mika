---
title: Mika-specific development workflow slash command
type: feat
status: completed
date: 2026-02-27
---

# Mika-specific development workflow slash command

## Overview

Create a project-level `/letsgo` Claude Code slash command at `.claude/commands/letsgo.md` that extends the global workflow with Mika-specific quality gates and documentation update steps. Also update the global `/letsgo` to unconditionally start ralph-loop.

## Problem Statement / Motivation

The current global `/letsgo` command at `~/.claude/commands/letsgo.md` has two gaps:

1. **Conditional ralph-loop**: Says "Start with step 2 now (or step 1 if ralph-wiggum is available)" — ralph-wiggum is always installed, so this conditional is unnecessary
2. **No documentation updates**: The workflow produces code changes but never audits documentation. This has already caused real drift (see `docs/solutions/documentation-gaps/oauth-subscription-auth-docs.md` where OAuth support was implemented but user-facing docs were not updated)
3. **No Rust quality gates**: `cargo fmt`, `cargo clippy`, and `cargo test` are not part of the automated pipeline — quality checks are manual

## Proposed Solution

### Two changes:

**1. Update global `/letsgo`** — Remove the conditional; always start with ralph-loop as step 1.

**2. Create project-level `/letsgo`** at `/data/workspace/senara-solutions/mika/.claude/commands/letsgo.md` with this pipeline:

```
Step 1:  /ralph-loop (unconditional)
Step 2:  /workflows:plan $ARGUMENTS
Step 3:  /workflows:work
Step 4:  Quality gate — cargo fmt && cargo clippy && cargo test
Step 5:  /workflows:review
Step 6:  /compound-engineering:resolve_todo_parallel
Step 7:  Quality gate re-run (same as step 4, catches regressions from fixes)
Step 8:  Documentation audit (CLAUDE.md + user-facing docs based on diff analysis)
Step 9:  /workflows:compound
Step 10: Output <promise>DONE</promise>
```

### Documentation audit step (Step 8) details

This step operationalizes the "Documentation-first checklist" pattern from `docs/solutions/documentation-gaps/oauth-subscription-auth-docs.md`. The agent should:

1. **Always review CLAUDE.md** — Check that Architecture, Stack, Conventions, Commands, and Environment Variables sections reflect the current code. Update test count, schema version, and Pending Work as needed.
2. **Check git diff for triggers** and update the corresponding files:
   - New `MIKA_*` env vars → `.env.example`, `docs/configuration.md`
   - Schema/DB changes → `docs/architecture.md`, CLAUDE.md Architecture section
   - New CLI commands or tools → `README.md`, `docs/getting-started.md`, `docs/slash-commands.md`
   - New skills or skill changes → `docs/skills.md`
   - Infrastructure changes (Helm, Docker, K8s) → `docs/deployment.md`
   - New config fields → `docs/configuration.md`
3. **Update Claude Code memory** — If significant patterns, conventions, or debugging insights were discovered during the session, update `/home/samidarko/.claude/projects/-data-workspace-senara-solutions-mika/memory/MEMORY.md`.

## Acceptance Criteria

- [x] Global `/letsgo` updated: ralph-loop starts unconditionally (step 1 always), conditional text removed
- [x] Project-level `.claude/commands/mika.md` created with full pipeline (named `/mika` instead of `/letsgo`)
- [x] Quality gates handled by `/workflows:work` and `/workflows:review` (no explicit cargo commands needed)
- [x] Pipeline includes documentation audit step between resolve_todos and compound
- [x] Documentation audit step covers: CLAUDE.md, .env.example, README.md, docs/getting-started.md, docs/configuration.md, docs/architecture.md, docs/skills.md, docs/deployment.md, docs/slash-commands.md
- [x] Memory/learnings handled by `/workflows:compound` step
- [x] `disable-model-invocation: true` set so steps execute without prompting

## Technical Considerations

### Command precedence
Claude Code uses project-level commands with precedence over global ones when the name matches. Creating `.claude/commands/letsgo.md` at the project level will override the global `/letsgo` when working in the Mika repo. The global command remains available for other projects.

### Quality gate design
- `cargo fmt` — auto-fixes formatting (not `--check`, since we want it fixed)
- `cargo clippy` — linting with default settings (project already uses this convention)
- `cargo test` — runs all ~236 tests
- Chained with `&&` so failures halt the pipeline and the agent can fix them before proceeding

### Documentation audit is agent-driven
Since `disable-model-invocation: true` means the command runs as literal steps, the documentation audit step needs to be written as an explicit instruction to the agent. The agent reads the git diff, applies the checklist, and updates files as needed.

## MVP

### `.claude/commands/letsgo.md` (project-level)

```markdown
---
name: letsgo
description: Mika full autonomous workflow with quality gates and docs
argument-hint: "[feature description]"
disable-model-invocation: true
---

Run these slash commands in order. Do not do anything else. Do not stop between steps — complete every step through to the end.

1. `/ralph-loop "finish all slash commands" --completion-promise "DONE"`
2. `/workflows:plan $ARGUMENTS`
3. `/workflows:work`
4. Run `cargo fmt 2>&1 && cargo clippy 2>&1 && cargo test 2>&1` to verify code quality. If any step fails, fix the issues and re-run until clean.
5. `/workflows:review`
6. `/compound-engineering:resolve_todo_parallel`
7. Run `cargo fmt 2>&1 && cargo clippy 2>&1 && cargo test 2>&1` again to catch regressions from review fixes. Fix any issues.
8. **Documentation audit** — Review the git diff (`git diff main...HEAD`) and update all affected documentation:
   - **Always**: Review `CLAUDE.md` for accuracy (architecture, conventions, commands, env vars, test count, schema version, pending work)
   - **If new env vars**: Update `.env.example` and `docs/configuration.md`
   - **If schema/DB changes**: Update `docs/architecture.md` and CLAUDE.md Architecture section
   - **If new CLI commands or tools**: Update `README.md`, `docs/getting-started.md`, `docs/slash-commands.md`
   - **If skill changes**: Update `docs/skills.md`
   - **If infra changes** (Helm, Docker, K8s): Update `docs/deployment.md`
   - **If new config fields**: Update `docs/configuration.md`
   - **If significant patterns or insights discovered**: Update memory at `~/.claude/projects/-data-workspace-senara-solutions-mika/memory/MEMORY.md`
9. `/workflows:compound`
10. Output `<promise>DONE</promise>` when complete

Start with step 1 now.
```

### Global `~/. claude/commands/letsgo.md` (updated)

```markdown
---
name: letsgo
description: Full autonomous workflow
argument-hint: "[feature description]"
disable-model-invocation: true
---

Run these slash commands in order. Do not do anything else. Do not stop between steps — complete every step through to the end.

1. `/ralph-loop "finish all slash commands" --completion-promise "DONE"`
2. `/workflows:plan $ARGUMENTS`
3. `/workflows:work`
4. `/workflows:review`
5. `/compound-engineering:resolve_todo_parallel`
6. `/workflows:compound`
7. Output `<promise>DONE</promise>` when complete

Start with step 1 now.
```

Only change: removed the conditional "Start with step 2 now (or step 1 if ralph-wiggum is available)" and replaced with "Start with step 1 now."

## References

- Global `/letsgo`: `/home/samidarko/.claude/commands/letsgo.md`
- Documentation-first checklist pattern: `docs/solutions/documentation-gaps/oauth-subscription-auth-docs.md`
- Claude Code memory directory: `/home/samidarko/.claude/projects/-data-workspace-senara-solutions-mika/memory/`
- Project settings: `.claude/settings.local.json` (already allows `cargo fmt`, `cargo clippy`, `cargo test`)
- CLAUDE.md staleness tracking precedent: `todos/066-complete-p2-stale-claude-md.md`, `todos/107-complete-p3-claudemd-stale-async-db.md`

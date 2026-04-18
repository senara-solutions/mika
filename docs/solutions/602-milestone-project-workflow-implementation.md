# Solution: Milestone and Project Workflow Branches

## Issue
https://github.com/senara-solutions/mika/issues/602

## Changes Made

### 1. self-dev/system_prompt.md — Added new workflow branches

Added "Milestone Workflow" and "Project Workflow" sections after the existing per-issue flow. These branches:

- Create parent tasks with `type='milestone'` or `type='project'`
- Fetch issues from GitHub milestones or Projects v2
- Create child tasks with `parent_task_id` linkage
- Execute children sequentially via existing per-issue flow
- Handle resume semantics for interrupted runs

Key patterns folded from self-dev-sprint:
- Serial execution with state machine
- Retry tracking (pipeline_retry_count, qa_retry_count, ci_fix_count)
- Stop/continue manual commands
- Completion summary with cost aggregation

### 2. Deleted self-dev-sprint/

Removed redundant `mika/skills/bundled/self-dev-sprint/` directory entirely. Its patterns are now in self-dev.

### 3. self-dev-webhook-qa/system_prompt.md — Terminology updates

Updated webhook handling to recognize child tasks by `parent_task_id` and clarified that milestones/projects use the same PR review webhook path.

### 4. CLAUDE.md — Documentation

Updated architecture docs to reflect:
- `skills/bundled/` directory structure
- Engine-coupled skills live next to dependent code
- Community skills remain in mika-skills repo

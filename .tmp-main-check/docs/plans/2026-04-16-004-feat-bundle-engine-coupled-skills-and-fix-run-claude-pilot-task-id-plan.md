---
title: Bundle engine-coupled skills + fix run_claude_pilot task_id fabrication
date: 2026-04-16
status: implemented
---

# Bundle engine-coupled skills + fix run_claude_pilot task_id fabrication

## Why

Three cross-repo drift incidents in 24 hours motivated the bundling decision (see [brainstorm in mika-platform](https://github.com/senara-solutions/mika-platform/blob/main/docs/brainstorms/2026-04-16-bundle-engine-coupled-skills-into-mika-brainstorm.md)). This PR is the keystone migration that makes atomic cross-concern changes possible.

The immediate trigger was mika#595: the `run_claude_pilot` tool exposed two UUID fields (`task_id` + `work_item_id`) that the executor internally treated as the same value. The agent's LLM fabricated a UUID for one of them, and the handler forwarded it to claude-pilot, breaking every relay callback for 23 minutes.

## What this PR does

Three changes, atomic because the skills are now in the same repo as the engine:

### 1. Migrate 11 engine-coupled skills into `mika/skills/bundled/`

Moved from `mika-skills/` (via `cp -r`, deletion from source is a follow-up PR):
- self-dev, self-dev-webhook-qa, self-dev-webhook-ci, self-dev-sprint
- qa-review, claude-pilot, build-mika, deploy-mika, permission-policy

Moved from `mika/crates/mika-agent/templates/skills/` (via `git mv`, since they were already in this repo):
- skill-review, agents-teams

These 11 are the ones whose correctness depends on staying in lockstep with Rust engine code — tool schemas, callback contracts, prompt-discipline rules encoded as Rust guards. Discovered at build time by the existing `build.rs` walk (#600).

`BUNDLED_SKILLS` static list now contains only the 10 community-category skills (tmux, shell-exec, web-search, file-reader, self-knowledge, git-ops, google-workspace, github, mcp, browser-control) that happen to be embedded for convenience but don't depend on engine internals.

### 2. Fix `run_claude_pilot` UUID fabrication vector (mika#595 / mika#596 / mika-skills#151)

- **Executor:** `execute_long_running` now reads `task_id` from input (both for `validate_work_item` and for the callback task's `parent_task_id`). Previously read `work_item_id` — a second redundant field that invited the LLM to fabricate.
- **Handler `run.sh`:** claude-pilot's `--task-id` argument now uses `$TASK_ID` (the executor-injected `__mika_task_id`, always a real DB UUID) instead of `$USER_TASK_ID` (agent-provided, fabricatable). Defense in depth.
- **self-dev prompt:** removed the stale Rule 4 entry that told the agent to pass BOTH `task_id` and `work_item_id`. Updated to match the new one-slot reality.

Result: one UUID slot in the tool contract, sourced from a single field the agent passes as the work item ID it already has in context. The LLM can't disagree with itself.

### 3. Engine-side bookkeeping

- `bundled_skills.rs`: removed `SKILL_REVIEW_SKILL` and `AGENTS_TEAMS_SKILL` static entries (now discovered from `skills/bundled/` via `build.rs`).
- Removed the temporary guard test `test_production_entries_is_empty_until_migration` — its comment literally said "DELETE THIS TEST when the migration ticket lands."
- Updated one test's `include_str!` path from `../templates/skills/skill-review/` to `../../../skills/bundled/skill-review/`.
- Updated executor test harness (25 tests) to pass `task_id` instead of `work_item_id` in test input JSON.

## Scope decisions

**In scope:**
- Move skills, fix fabrication, update engine code references, pass all tests.

**Out of scope:**
- Deleting the 11 migrated skills from `mika-skills/` — separate PR on `mika-skills`.
- Moving community skills from `templates/skills/` into `skills/bundled/` (or into `mika-skills/`) — they're fine where they are.
- Renaming `work_item_id` internal variable names in executor — kept for clarity; only the input JSON field name changed.
- Tool-boundary UUID existence validation (mika#596) — structural defense layered on top of this fix; separate PR.

## Verification

- `cargo check -p mika-agent` — clean
- `cargo clippy -p mika-agent` — clean
- `cargo test -p mika-agent --lib` — **1658 passed, 0 failed, 2 ignored**
- Plan + compound docs committed for `verify-pipeline.sh` gate.
- Manual inspection: `ls skills/bundled/` shows all 11 migrated dirs.

## Follow-up tickets

- Delete migrated skills from `mika-skills/` + update its CLAUDE.md.
- `mika#596` — tool-boundary UUID existence validation (defense in depth).
- `mika-skills#149` — self-dev milestone/project workflow branches. Unblocked by this PR.
- `mika-platform#41` — retire `/mika-sprint`, rename audit. Unblocked.
- `mika-platform#42` — live acceptance test of milestone dispatch. Unblocked.

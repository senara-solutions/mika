---
title: Migrate 5 more engine-coupled skills into skills/bundled/
date: 2026-04-16
status: implemented
---

# Migrate 5 more engine-coupled skills into skills/bundled/

## Why

Follow-up to mika#601. The initial bundling migration missed five engine-coupled skills:

- `qa-review-build-callback` — build callback resumption for qa-review. Tightly paired with the qa-review verdict/callback flow in the engine.
- `self-dev-iterate` — self-dev variant for PR iteration mode (Step 3a/3b). Depends on the same run_claude_pilot contract that self-dev does.
- `address-pr-comments` — launches a focused claude-pilot session to handle PR review comments. Engine-coupled via the same dispatch contract.
- `resolve-pr-conflicts` — long-running exec handler with `task_id` schema parameter. Spawns claude-pilot. Same dispatch contract as run_claude_pilot; same fabrication-vector class as mika#595.
- `self-check` — diagnostic skill that reads engine-owned DB schema (`tasks`, `audit_log`, `unified_timeline`, `sessions`, `team_runs`). When engine schema changes (like schema v23's `tasks.type` column), self-check's queries need to update in lockstep.

The first three were flagged in mika-skills#152's PR body. The last two were caught on a follow-up review pass — they both launch claude-pilot or query engine tables, which makes them engine-coupled by the same test that applied to self-dev and qa-review.

## Scope

Copy 3 directories from `mika-skills/` into `mika/skills/bundled/`. No code changes. No prompt changes. No schema changes. Pure migration.

## Verification

- `cargo check -p mika-agent` — clean
- `cargo test -p mika-agent --lib` — **1658 passed, 0 failed**
- `build.rs` walks `skills/bundled/` and picks up the 3 new skills automatically (from mika#600).

## Follow-up

- Delete these 3 skills from `mika-skills/` (additional PR on mika-skills, or amend the pending PR #152).
- Full skill discovery should be inspected to ensure no further engine-coupled skills remain in the marketplace repo.

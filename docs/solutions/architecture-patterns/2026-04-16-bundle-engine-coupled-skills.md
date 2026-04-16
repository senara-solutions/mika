---
title: Bundle engine-coupled skills into the engine repo
date: 2026-04-16
category: architecture-patterns
status: applied
---

# Bundle engine-coupled skills into the engine repo

## The problem

A skill whose correctness depends on engine code — tool schema shape, callback contract, prompt-discipline rule enforced by an engine guard — living in a separate marketplace repo creates a drift class:

- Engine change lands in `mika`. Matching skill change lands in `mika-skills`. Two PRs.
- The secondary PR bypasses the primary repo's `verify-pipeline.sh` (plan + compound artifact gates run per-repo, not cross-repo).
- Milestones are repo-scoped, so you can't put both halves of a coupled change in one milestone.
- Inevitably: secondary PRs ship without artifacts. Both halves diverge over time. Incidents like mika#586+mika-skills#150 (engine guard + its matching prompt rule shipped separately; secondary PR nearly got discarded) or mika#595 (schema fabrication that required fixing both engine and handler) keep recurring.

## The test

*Does this skill's correctness depend on staying in lockstep with engine code?* If yes, it's engine-coupled. Concretely, engine-coupled means one or more of:

- Tool schema dependency (the Rust executor reads specific input fields the skill declares).
- Callback/webhook contract (e.g., `verdict_handler`, `ci_success_handler` parse specific formats).
- Prompt-discipline rule enforced by an engine guard (e.g., dispatch readiness, phantom retry, completion-claim).

The 11 skills that passed the test in mika's case:
self-dev, self-dev-webhook-qa, self-dev-webhook-ci, self-dev-sprint, qa-review, skill-review, claude-pilot, build-mika, deploy-mika, permission-policy, agents-teams.

Skills that failed the test (stay in the marketplace):
memory, reminders, google-workspace, web-search, github (generic), browser-control, tmux, file-reader, mcp — they use the public skill API (tools.json, handler protocol, context injection) and don't depend on engine internals.

## The fix shape

1. Add a `skills/bundled/` directory inside the engine repo.
2. Engine's build script walks it at compile time, generates a Rust table of embedded skills.
3. Migrate engine-coupled skills from the marketplace into that directory.
4. Engine's skill loader merges: hardcoded community bundle + `skills/bundled/` table + user marketplace. `skills/bundled/` wins on name collisions.

## What this unlocks

- **Atomic cross-concern PRs.** Engine change + matching tool schema + matching prompt rule in one diff. `verify-pipeline.sh` runs once with full visibility.
- **Milestone coherence.** A milestone on the engine repo can hold all related work for a feature, regardless of whether it touches engine code, tool schema, or prompt discipline.
- **One clear rule for contributors.** "Does this depend on engine code?" answers where to put new skills. Drift becomes unlikely because the boundary is structural, not conventional.
- **Fabrication-vector class elimination.** Because the tool schema and the prompt that references it now live together, you can catch redundant-field fabrication vectors like mika#595's (two UUID slots that should agree) at review time rather than discovering them in production.

## Cost

- One-time migration of ~10 skill directories.
- Install-path refactor in the engine (covered by mika#598/PR #600): walk a directory at build time instead of hardcoding `include_str!` lists.
- Minor noise in engine repo layout (`skills/bundled/` next to `crates/`) — low cost, and it matches the structural reality (these skills ARE part of the engine's contract surface).

## Related patterns

This is the same pattern as:
- **Vendored dependencies vs. package-manager dependencies.** Vendor when you can't afford drift.
- **In-tree tests vs. separate integration repos.** Keep tight coupling tight.
- **Monorepo arguments in general.** The boundary should match where atomic changes need to happen.

The wisdom: *the repo boundary should encode where atomic changes are required, not where domain labels are convenient.* "Skill" sounded like a cleaner boundary than "engine" — but if the skill's prompt has to be rewritten every time the engine's tool schema changes, they're not actually separable.

## Applied in

- PR linking to this doc: migration of 11 engine-coupled skills from `mika-skills/` into `mika/skills/bundled/`, concurrent with fixing the mika#595 `run_claude_pilot` schema fabrication (one atomic PR, impossible before bundling).
- Follow-up: delete migrated skills from `mika-skills/`, file follow-ups for engine-side UUID existence validation (mika#596) and downstream milestone refactor (mika-platform#41/#42).

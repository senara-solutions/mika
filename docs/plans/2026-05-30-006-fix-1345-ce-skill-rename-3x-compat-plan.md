# Plan: fix(dispatch-lib): /ce:work → /ce-work for compound-engineering 3.x (mika#1345)

## Problem

After today's 18:10Z `make deploy` (which restarted mika-spirit), every autonomous-loop IMPL dispatch dies in 7ms with `[error] pipeline_incomplete:` (no API call), while every GROOM dispatch succeeds in 21–23 turns. The pattern is reproducible:

| session | prompt | turns | cost | result |
|---|---|---|---|---|
| c0ded7b3 (mika#1193 IMPL) | `/ce:work docs/plans/...` | 2 | $0.00 | 7ms death |
| b020b885 (mika#1188 IMPL) | `/ce:work docs/plans/...` | 2 | $0.00 | 7ms death |
| 6313f920 (mika#949 GROOM) | `/mika-groom-plan-only mika#949` | 21 | $1.37 | success |
| a88ac3b5 (mika#806 GROOM) | `/mika-groom-plan-only mika#806` | 23 | $1.82 | success |

Root cause: **compound-engineering 3.x renamed every skill from `name: ce:work` (colon) to `name: ce-work` (dash)** — see plugin CHANGELOG entry #345: *"cli: rename all skills and agents to consistent ce- prefix (#503)"*. The plugin's old `/ce:work` slash command was removed; the canonical invocation is now `/ce-work`. Verified in `~/.claude/plugins/cache/every-marketplace/compound-engineering/3.9.3/skills/ce-work/SKILL.md` frontmatter (`name: ce-work`) versus 2.65.0 (`name: ce:work`).

`dispatch-lib.sh:1501` still emits `/ce:work` — Claude Code CLI can't resolve it under 3.x, so the subprocess exits before any LLM call.

## Acceptance criteria

- [ ] `dispatch-lib.sh:1501` (`_detect_plan_on_branch`) emits `/ce-work $PLAN_PATH` (with dash, not colon).
- [ ] `test-dispatch-lib.sh:208` assertion verifies `/ce-work` substring (not `/ce:work`).
- [ ] `bash skills/bundled/_shared/test-dispatch-lib.sh` shows the same pre-existing-failure count as `main` (no NEW failures introduced by this change).
- [ ] PR body explicitly documents the `/mika` pipeline bypass — implementing the fix for the wedge that prevents `/mika` from running.

## Out of scope

- Doc/comment updates referencing `/ce:work` (CONTRIBUTING.md, CLAUDE.md, etc.) — non-runtime, harmless drift. Tracked as follow-up if desired.
- Updates to `/ce:plan` invocation log-grep at line 645 — pattern `ce[.:\-_]plan` already tolerates both colon and dash forms.
- mika-skills + claude-pilot-py callers — none directly invoke `/ce:work` outside dispatch-lib's runtime emit.

## Pipeline bypass justification

This fix targets the **bug that breaks the `/mika` pipeline itself**: dispatching through `/mika` would call `/ce:plan` → `/ce:work` → die at 7ms with no plan or impl ever written. Substrate fix shipped directly per `feedback_substrate_ownership_change_the_tyre` ("don't drive on the flat tyre") + `feedback_pipeline_match_severity` (hot-fix scales).

## Verification (post-deploy)

1. `make deploy` after merge — refreshes `~/.mika/skills/_shared/dispatch-lib.sh`.
2. Re-apply `ready` label to one of #1188 / #1191 / #943 / #736 / #1335 / #1193 (currently stripped to halt the failed-rescue cascade).
3. Observe IMPL dispatch firing `/ce-work` — should run 10+ turns + emit `[done] Success` per the working-groom shape.
4. Re-apply remaining `ready` labels if test succeeds; surface to operator if it fails.

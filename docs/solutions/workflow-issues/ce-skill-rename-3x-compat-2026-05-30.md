---
module: skills/bundled/_shared
tags: [autonomous-loop, dispatch-lib, compound-engineering, plugin-compatibility, slash-command]
problem_type: bug
category: workflow-issues
date: 2026-05-30
ticket: mika#1345
resolution_type: substrate_hotfix
---

# /ce:work → /ce-work — compound-engineering 3.x slash-command rename — 2026-05-30

## Problem

After today's `make deploy` (18:10Z UTC), every autonomous-loop IMPL dispatch died in 7ms with `[error] pipeline_incomplete:` and exit code 1. GROOM dispatches kept succeeding at 21–23 turns. Dispatch-lib's auto-rescue path opened two misleading "wip(impl) rescued" PRs (#1344, #1346) containing only groom artifacts.

## Diagnosis

The failure signature pointed at the slash command, not the worktree or substrate. The two surfaces split on prompt content:

- IMPL dispatches: `/ce:work docs/plans/<file>.md mika#N` → 7ms exit
- GROOM dispatches: `/mika-groom-plan-only mika#N` → 21–23 turns success

`/mika-groom-plan-only` resolves from the worktree's `.claude/commands/`; `/ce:work` resolves from the global compound-engineering plugin under `~/.claude/plugins/cache/every-marketplace/compound-engineering/`. The plugin was upgraded from 2.65.0 to 3.9.3 at 14:28Z UTC today (per `installed_plugins.json.lastUpdated`); the running mika-spirit kept stale plugin state until the 18:10Z restart loaded the new layout.

Comparison of frontmatter between versions in `skills/ce-work/SKILL.md`:

```
2.65.0:  name: ce:work     ← colon
3.9.3:   name: ce-work     ← dash
```

The plugin's own changelog records the rename at #503 — *"cli: rename all skills and agents to consistent ce- prefix"* — but `dispatch-lib.sh` was never updated. Claude Code CLI under 3.x can't resolve `/ce:work`; the subprocess dies before any API call, hence the 7ms / $0.00 / 2-turn signature.

## What didn't work

- **Cold-start hypothesis (mika#1345 initial filing).** First N=1 instance landed at T+4min after mika-spirit restart, suggesting a relay-warmup race. Wrong: N=2 instance at T+36min showed the timing didn't matter — the slash command did.
- **Plugin install check.** `installed_plugins.json` showed compound-engineering 3.9.3 cleanly installed. The plugin was working; only the *command name* changed.
- **`claude-pilot --no-relay` smoke test.** The permission classifier correctly forbids bypassing the relay safety surface for diagnostic purposes (per `feedback_never_suggest_dangerously_skip_permissions`). Diagnosis converged via static analysis of plugin SKILL.md frontmatter + CHANGELOG.

## Resolution

Single-site change in `dispatch-lib.sh:1501`:

```bash
# before
ENTRY_COMMAND="/ce:work $PLAN_PATH"

# after
ENTRY_COMMAND="/ce-work $PLAN_PATH"
```

Plus paired test assertion in `test-dispatch-lib.sh:208` (now checks `/ce-work` substring).

The `/ce:plan` invocation-detection log-grep at line 645 (`grep -qiE 'ce[.:\-_]plan'`) already tolerates both colon-and-dash forms, so no change needed there.

## Lessons compounded

1. **Plugin-version drift is a substrate hazard.** `installed_plugins.json` records install state but doesn't surface breaking renames inside the plugin tree. When third-party plugin slash commands are baked into our autonomous-loop dispatch path, a plugin upgrade is a substrate-affecting event — track plugin SKILL.md frontmatter changes alongside our own deploys.
2. **Failure-shape isolation > timing correlation.** The first N=1 instance (post-deploy cold-start window) tempted the wrong hypothesis. N=2 collapsed it because the timing didn't fit, and the prompt-content axis was the load-bearing distinguisher. Per `feedback_n_equals_2_is_the_signal`: the second occurrence is the signal — don't dispatch more upstream activity that will re-fail the same way.
3. **Dispatch-lib's auto-rescue surface area is a feature when the pilot dies cleanly mid-impl, and a misleading-PR hazard when the pilot dies at init.** PRs #1344 + #1346 both had titles "wip(impl) rescued" but contents were only the groom plan + verdict trail log (no impl). Operators expect "wip(impl)" to mean "partial impl shipped"; here it meant "pilot couldn't even start." Consider adding a turns/cost threshold to the rescue-PR path: <3 turns + $0.00 cost = don't open a PR, just log the failure (separate follow-up).

## Operational notes

- All `ready` labels were stripped from queued IMPL multipliers (#1188, #1191, #943, #736, #1335, #1193) during the wedge to halt the failed-rescue cascade. Re-apply after this fix deploys.
- Groom dispatches kept working throughout — `/mika-groom-plan-only` resolves from the worktree, not the plugin.
- PRs #1344 + #1346 closed with diagnostic comments referencing this fix.

## Related

- mika#1097 (closed 2026-05-13) — original "claude-pilot zero-artifact exit" family; same exit shape, different trigger (vocabulary mismatch vs slash-command rename).
- mika#1282 — dispatch-lib's `wip()` git-workflow recovery path that produced the misleading PRs.
- mika#1340 — preceding substrate fix this week; deployed alongside.
- compound-engineering plugin CHANGELOG #503 — the upstream rename commit.

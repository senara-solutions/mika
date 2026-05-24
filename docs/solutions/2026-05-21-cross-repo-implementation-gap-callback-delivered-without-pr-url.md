---
title: "Cross-repo tickets stall mid-implementation when claude-pilot can't coordinate paired-repo PRs"
date: 2026-05-21
category: agent-quality
module: dev-pilot
problem_type: behavior_drift
component: cross-repo
symptoms:
  - "Supervisor task `result` shows `callback_delivered_without_pr_url`"
  - "Pilot exits with `Success` after 30-60 turns ($1-$5)"
  - "Worktree edits made in BOTH mika/ and claude-pilot-py/ but neither committed"
  - "HEAD unchanged on the primary branch"
  - "Operator-visible send_message: 'mika#N pipeline failed — BLOCKED. claude-pilot exited with error after N turns'"
root_cause: configuration_error
resolution_type: workflow
severity: high
tags:
  - dev-pilot
  - cross-repo
  - claude-pilot-py
  - paired-pr
  - callback-delivered-without-pr-url
related:
  - mika#943
  - mika#944
  - mika#946
---

## Symptoms

A cross-repo ticket (e.g., mika ticket whose implementation touches both `mika/` Rust source AND `claude-pilot-py/` Python source) gets dispatched to dev-pilot via canonical autonomous loop. Pilot:

- Spawns in the **primary** worktree (mika/), per the body callout's branch reference
- Makes edits to files in BOTH `mika/crates/...` and `claude-pilot-py/src/...` paths
- Runs ~30-60 turns, $1-$5
- Exits with `Success` (no errors)
- BUT: **zero commits produced** in either repo, neither branch pushed, no PR opened

Supervisor task's `result` shows `callback_delivered_without_pr_url`. Mika-dev's callback handler correctly classifies this as pipeline-incomplete and surfaces a Telegram-shaped notification to operator.

## Trigger

Observed 2026-05-20 on multiple security cohort tickets:
- mika#943 (output redirect Rust TIER3 — touches `permission_pre_classifier.rs` + `tier1.py`)
- mika#944 (ANSI-C quoting bypass — touches both layers)
- mika#946 (quote-aware metacharacter — touches both, but SHIPPED via separate paired PRs — see "Counterexample" below)

In all three cases, the ticket's plan correctly described the cross-repo nature (separate sections for Rust changes and Python changes). The pilot's worktree was `mika/.claude/worktrees/<branch>/mika`. But the corresponding claude-pilot-py worktree was either absent or wasn't navigated into for commits.

## Root cause

The `dev-pilot` skill's system prompt does not have explicit cross-repo coordination logic. The pilot:

1. Reads the plan (covers both repos)
2. Edits files in BOTH repos (via Bash with absolute paths)
3. Reaches "commit + push + open PR" phase
4. Tries to commit from the primary worktree — but the changes outside `mika/` aren't in the primary worktree's git tree
5. Either fails silently or commits only the mika-side changes (without pushing or opening PR) because the cross-repo state is inconsistent

The post-condition guard `dispatch_no_grooming_marker` checked at dispatch start, NOT pilot post-completion. There is no "PR-must-exist" post-condition guard yet, just the heuristic in mika-dev's callback handler that infers it from result text.

## Counterexample: mika#946 (shipped successfully)

mika#946 DID ship via dev-pilot — PR #1229 closes it. The architect's grooming verdict explicitly noted cross-repo shape: *"Cross-repo: ticket on mika, code change in claude-pilot-py/src/claude_pilot/tier1.py. Implementation will produce paired PRs (claude-pilot-py for code+tests, mika for the F5 sentinel comment update)."*

#946's pilot wrote ONE comment in mika (a sentinel update) and the actual logic change in claude-pilot-py via a SEPARATE PR (claude-pilot-py#16, also merged 2026-05-20). So the pilot DID open paired PRs — when the work was structured as "comment in mika + logic in pilot-py."

#943 and #944 had the OPPOSITE structure: substantial logic in BOTH repos. Pilot didn't know to split.

## Recovery

**Operator-side:**

1. **Manual paired commit + push + PRs:**
   ```bash
   cd mika/.claude/worktrees/<branch>/mika
   git status                                                # see Rust changes
   git add <files> && git commit -m "..."
   git push -u origin <branch>
   gh pr create --repo senara-solutions/mika ...

   cd ../../claude-pilot-py                                  # if worktree exists
   # OR
   cd /data/workspace/mika-platform/claude-pilot-py
   git checkout -b <same-branch-name>
   git add <files> && git commit -m "..."
   git push -u origin <branch>
   gh pr create --repo senara-solutions/claude-pilot-py ...
   ```

2. **Cross-reference the two PRs** in their bodies: `Companion PR: senara-solutions/<other-repo>#<number>`

3. **Update the mika ticket body** with both PR references so the dispatch_no_grooming_marker doesn't re-fire on future re-dispatch.

## Avoidance

When grooming a cross-repo ticket, the plan should specify:

1. **Which repo is primary** (where the supervisor task lives)
2. **Which files in which repo** (explicit paths, no ambiguity)
3. **Commit order** (typically: secondary first, then primary that references secondary PR)
4. **Explicit "do BOTH commits + BOTH pushes + BOTH PRs" instruction** for the implementing pilot

The architect-pass on the grooming plan should reject cross-repo plans that lack this structure (file followup).

## Followup ticket candidates

1. **dev-pilot system prompt hardening for cross-repo**: add a section: "If the plan touches files outside the primary worktree's repo, ALSO cd to the sibling repo's worktree (creating it if absent), commit + push + open PR there, then come back and complete the primary." With explicit dual-PR template.
2. **dispatch_no_pr_after_pilot guard**: post-condition check after callback that PR exists on the primary branch. Fails the supervisor task fast (instead of relying on text-heuristic in callback handler).
3. **Cross-repo grooming-marker extension**: detect cross-repo intent in the plan (e.g., file paths starting with `claude-pilot-py/`) and require paired-branch callouts in body.

## Related

- mika#943 — wedged (output redirect Rust TIER3) — deferred for operator decision
- mika#944 — wedged (ANSI-C quoting bypass) — deferred for operator decision
- mika#946 — shipped successfully (paired-PR structure clear from grooming)
- claude-pilot-py#16 — companion PR to mika#946's mika-side fix
- `feedback_secondary_pr_plan_doc.md` — sibling pattern on cross-repo plan docs
- `feedback_cross_repo_awareness.md` — sibling pattern on fixing shared issues across repos

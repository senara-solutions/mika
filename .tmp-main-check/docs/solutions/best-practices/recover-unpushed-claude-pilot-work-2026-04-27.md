---
title: "Recover unpushed claude-pilot work — git log before declaring no commits"
date: 2026-04-27
category: best-practices
module: self-dev
problem_type: best_practice
component: development_workflow
severity: high
applies_when:
  - claude-pilot terminates with error_max_turns (SDK turn limit reached)
  - mika-dev post-callback handling classifies a pipeline result as failure
  - Any verdict path that claims "no commits produced" without verifying local branch state
resolved_by: "mika#1268 — unconditional post-flight push finalizer in dispatch-lib.sh"
tags:
  - error-max-turns
  - recover-unpushed-work
  - grounding-check
  - verify-before-claiming
  - pipeline-failure-classification
  - operator-routing
related_components:
  - claude-pilot
  - self-dev
  - self-dev-webhook-qa
---

# Recover unpushed claude-pilot work — git log before declaring no commits

## The failure shape

claude-pilot ran a full pipeline session (plan, work, review, fix commits) but hit the `maxTurns` guardrail (default 200) during closing steps (`/ce:compound`, push, PR creation). The implementation is complete on the local branch but never pushed to origin.

mika-dev's post-callback handler checked GitHub for evidence (`gh pr list --head <branch>`) and concluded "no commits produced." This was true at origin (never pushed) but false at the local repo layer — the branch had 9 commits intact in `.git/objects/`.

**Founding incident:** mika#659 (2026-04-26). claude-pilot ran 201 turns / $22.98 / 40 minutes, completed implementation + `/ce:review` + review-fix commits on local branch `feat/659/dashboard-time-range-filter` (tip `e5bb119d`), hit max_turns at `/ce:compound`. mika-dev proposed redispatch — which would have burned another session redoing completed work.

## The grounding rule

**Always query the local branch ref before declaring "no commits produced."**

```bash
git -C /data/workspace/mika-platform/<repo> log --oneline origin/main..<branch>
```

Returns 0 lines if no local commits exist, >= 1 if they do. This is the ground-truth check that must run BEFORE concluding the pipeline failed.

This is the same "verify local state before claiming downstream state" pattern from `feedback_mika_dev_llm_fabricates_tool_errors.md` and `feedback_verify_before_claiming.md`.

## Failure-recognition decision tree

This decision tree is the institutional knowledge for `error_max_turns` handling. It survives prompt rewrites because it is documented here as a compound doc, not only embedded in the skill prompt.

1. **Does `tasks.result` contain `error_max_turns`?** Yes -> run grounding check (step 2). No -> check secondary trigger (stale in-progress heuristic) or fall through to existing failure handling.
2. **Does `git log origin/main..<branch>` return >= 1 commit?** Yes -> verdict is `recover_unpushed_work`. Apply recovery handler (step 3). No -> genuine no-progress failure. Existing pipeline-retry-or-escalation path applies.
3. **Recovery handler (atomicity: metadata THEN notification):**
   - Write `unpushed_recovery_pending: true` to `tasks.metadata` (status stays `in_progress`)
   - Send structured notification to operator with branch, tip SHA, commit count, and suggested recovery command
   - Do NOT increment `pipeline_retry_count`
   - Do NOT redispatch `run_claude_pilot`

## The recovery procedure

When the operator receives a `recover_unpushed_work` notification:

1. **Locate the worktree** (or create one if it was cleaned up):
   ```bash
   WT=$(git -C /data/workspace/mika-platform/<repo> worktree list --porcelain \
     | awk -v b=<branch> '/^worktree /{p=$2} /^branch refs\/heads\/'b'$/{print p}')
   [ -z "$WT" ] && {
     WT=/data/workspace/mika-platform/.claude/worktrees/<sanitized-branch>/<repo>
     git -C /data/workspace/mika-platform/<repo> worktree add "$WT" <branch>
   }
   ```
2. **Rebase onto origin/main** and resolve any conflicts:
   ```bash
   cd "$WT" && git rebase origin/main
   ```
3. **Push and open PR**, bypassing claude-pilot:
   ```bash
   git push origin <branch>
   gh pr create --repo senara-solutions/<repo>
   ```

## Out-of-scope guardrails

Each claude-pilot guardrail has different failure semantics. The `recover_unpushed_work` verdict applies ONLY to `error_max_turns`. These are explicitly out of scope:

- **`error_stall_threshold`** — LLM produced no output. Likely infinite loop, not "did work then ran out of budget." Recovery does not apply.
- **`error_idle_timeout`** — Process abandoned mid-run. Different failure semantics; manual investigation needed.
- **`error_empty_response_threshold`** — Successive empty responses. Recovery is provider-side, not work-recovery.

Uniform handling of all guardrails would produce incorrect guidance. Each should be treated separately as recovery needs surface.

## Citations

- **Founding incident:** mika#659 (2026-04-26) — claude-pilot log at `/var/log/claude-pilot/1027bc40-7bbf-4d5d-8e87-8f771022ff87.log`
- **Structural fix:** mika#838 — adds `recover_unpushed_work` verdict class to self-dev pipeline-result classification
- **Architectural precedent:** mika#825 — `block[ac]` verdict class (operator-routed, no auto-retry)
- **Related grounding patterns:** `feedback_mika_dev_llm_fabricates_tool_errors.md`, `feedback_verify_before_claiming.md`

## Resolution

This failure class is resolved by the unconditional post-flight push finalizer added in mika#1268. `dispatch-lib.sh` now calls `_post_flight_push()` after `_run_claude_pilot()` returns and before callback delivery, regardless of pilot exit code. The helper:

1. Fetches fresh remote state for the branch (no-ops on first-push where the ref doesn't exist yet).
2. If `origin/$BRANCH` exists and HEAD is not ahead, skips (already in sync — handles idempotency with the Class D push at line 250).
3. Otherwise pushes with `-u` for upstream tracking.
4. Appends a status line to RESULT (`Post-flight push: pushed to origin/$BRANCH` or `Post-flight push: FAILED — commits remain local-only on $BRANCH`).

The manual recovery procedure above remains valid for edge cases where the push itself fails (e.g., diverged remote, network failure), but the common case of "valid commits stranded locally due to non-zero exit code" no longer requires manual intervention.

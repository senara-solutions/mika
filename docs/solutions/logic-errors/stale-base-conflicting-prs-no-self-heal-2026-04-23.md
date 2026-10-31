---
title: Stale-base branches cause CONFLICTING PRs with no self-heal path
date: 2026-04-23
category: logic-errors
module: claude-pilot-handler
problem_type: logic_error
component: tooling
symptoms:
  - PR stuck in mergeable=CONFLICTING state for hours with no autonomous recovery
  - Sequential tickets dispatched after upstream merge inherit stale base
  - resolve-pr-conflicts skill exists but is never invoked by self-dev
root_cause: missing_workflow_step
resolution_type: code_fix
severity: high
tags:
  - claude-pilot
  - self-dev
  - rebase
  - merge-conflicts
  - worktree
  - stale-base
  - resolve-pr-conflicts
---

# Stale-base branches cause CONFLICTING PRs with no self-heal path

## Problem

PRs created by the autonomous dev loop sit in `mergeable=CONFLICTING` state indefinitely because two structural gaps prevent self-healing: (1) the claude-pilot handler checks out pre-committed branches without rebasing onto the latest `origin/main`, and (2) the `resolve-pr-conflicts` skill is loaded nowhere and triggered by nothing in self-dev.

## Symptoms

- PR #746 sat in `mergeable=CONFLICTING, mergeState=DIRTY` for 8+ hours
- Multiple pre-committed branches (`feat/338/...`, `feat/339/...`, `feat/740-742/...`) all branched from stale commit `6a0315be` while main had advanced to `65abe6f2`
- `self-dev/skill.toml` dependencies array did not include `resolve-pr-conflicts`
- `self-dev/system_prompt.md` had zero matches for `resolve_pr_conflicts`, `mergeable`, or `CONFLICTING`

## What Didn't Work

- The milestone grooming pattern pre-commits plan docs on feature branches before dispatch. When those branches are later checked out by the handler, they carry the stale base from grooming time — not from dispatch time.
- The handler's worktree fallthrough path (`git worktree add <WORKTREE> <branch>`) was designed for branch reuse but did not account for the branch being behind `origin/main`.

## Solution

Three targeted fixes in one PR:

**Fix 1 — Rebase-or-abort guard in `skills/bundled/claude-pilot/handlers/run.sh`:**

After worktree creation/reuse (both paths converge), check if the branch is behind `origin/main` and auto-rebase. On conflict, capture the file list BEFORE `rebase --abort` (abort resets the index) and exit with a structured `STATUS=REBASE_CONFLICT` discriminator:

```sh
BEHIND=$(git -C "$WORKTREE_DIR" rev-list --count HEAD..origin/main 2>/dev/null || echo 0)
if [ "$BEHIND" -gt 0 ]; then
    if git -C "$WORKTREE_DIR" rebase origin/main 2>/dev/null; then
        echo "Rebased ${BRANCH} onto origin/main (${BEHIND} commits caught up)." >&2
    else
        CONFLICTS=$(git -C "$WORKTREE_DIR" diff --name-only --diff-filter=U 2>/dev/null | tr '\n' ' ')
        git -C "$WORKTREE_DIR" rebase --abort 2>/dev/null || true
        RESULT="STATUS=REBASE_CONFLICT
Branch ${BRANCH} is ${BEHIND} commits behind origin/main.
Conflicted files: ${CONFLICTS:-<unable to capture>}
Resolve manually before re-dispatching ${REPO}#${ISSUE_NUM}."
        exit 1
    fi
fi
```

Key ordering: `diff --diff-filter=U` runs BEFORE `rebase --abort` because abort resets the index and the conflict markers are gone after.

**Fix 2 — Dependency declaration in `skills/bundled/self-dev/skill.toml`:**

Added `"resolve-pr-conflicts"` to the `dependencies` array. The BFS dependency resolver loads it whenever self-dev activates, making `resolve_pr_conflicts` available in mika-dev's tool inventory.

**Fix 3 — Routing sentence in `skills/bundled/self-dev/system_prompt.md`:**

Added one sentence in the "On success" callback path: check `mergeable` state via `gh pr view --json mergeable` before treating PR as ready; if `CONFLICTING`, invoke `resolve_pr_conflicts` per that skill's documented routing table. No routing-table duplication — the skill's own prompt owns the contract.

## Why This Works

**Root cause 1 (stale base):** The handler's worktree fallthrough path checked out existing branches at their current tip, which could be arbitrarily behind `origin/main`. The rebase guard catches this gap after both worktree paths converge — `origin/main` is already fresh from the fetch at line 233. For clean rebases (no conflicts), claude-pilot starts from a current base. For real conflicts, the structured `STATUS=REBASE_CONFLICT` exits early with diagnostic details instead of silently producing a CONFLICTING PR.

**Root cause 2 (capability not loaded):** The `resolve-pr-conflicts` skill existed with a working `resolve_pr_conflicts` tool, but self-dev didn't declare the dependency (BFS resolver never loaded it) and the prompt never checked mergeable state. Adding the dependency makes the tool structurally available; adding the routing sentence makes the agent check for the condition. This follows the `feedback_prompt_enforcement_fragile.md` principle: structural dependency > prompt enforcement.

## Prevention

- When adding new skills that should be invoked by an orchestrator skill, always add both: (1) the dependency in `skill.toml` so the tool is in the inventory, and (2) the trigger condition in the orchestrator's system prompt. A skill that exists but isn't loaded is invisible.
- When worktree reuse or branch checkout paths exist in handlers, always consider whether the branch may be behind the base. Fetch + rebase-or-abort is the standard pattern.
- The EXIT trap in `run.sh` checks `[ -z "$RESULT" ]` — populate `RESULT` before `exit 1` to deliver a structured callback instead of a generic "HANDLER CRASH" message.

## Follow-up: mid-session duplicate-commit guard (#784)

The #747 rebase-or-abort guard runs once at session START, but mid-session `git pull` or `git merge main` can reintroduce duplicate-hash copies of upstream commits onto the branch. This was observed in PR #782: after the startup guard ran cleanly, the claude-pilot session ran `git pull origin main` mid-session, creating commit `8693b3fd` — a duplicate of main's `14279524` (same author date, message, diff, different hash). GitHub's 3-way merge saw both commits touching the same lines → `mergeable=CONFLICTING`.

**Fix:** A pre-push `_check_duplicate_commits()` guard in `dispatch-lib.sh` uses `git log --cherry-mark --right-only origin/main...HEAD` to detect patch-equivalent commits before push. If duplicates are found, it attempts an automatic rebase onto `origin/main` (rebase naturally drops patch-equivalent commits). On rebase failure, push is skipped with a structured error message.

This closes the gap between the startup guard (#747) and the push boundary — duplicates introduced mid-session by any mechanism (pull, merge, cherry-pick) are caught before they reach GitHub.

See: `docs/solutions/logic-errors/mid-session-duplicate-commit-pre-push-guard-2026-05-26.md`

## Suite : la porte de rebase bute sur la branche d'un vol déjà mergé (mika#2976)

La porte de rebase du Fix 1 suppose que la branche reprise porte un travail en cours. Ce n'est plus vrai quand un ticket a plusieurs vols. Le nom de branche est dérivé du ticket (`scripts/derive-branch-name`), donc le groom du vol 2 dérive le nom du vol 1. Si la PR du vol 1 a été mergée en squash, la branche reprise ne porte plus que des commits dont le contenu est déjà dans `main`. La porte compte `rev-list --count HEAD..origin/main`, voit un retard, rebase, et rejoue ces commits contre leur propre squash : conflit, `STATUS=REBASE_CONFLICT`, groom sans plan ni PR. Trois occurrences en huit jours (mika#1727 le 2026-10-23, mika#2968 deux fois le 2026-10-30), sur origin comme en local seulement.

Le défaut est de compter des **commits** là où un squash demande de compter des **patchs**. Le correctif ne touche pas à la porte de rebase. Il pose avant elle une question qu'elle ne sait pas poser : *cette branche porte-t-elle un travail en cours ?* (`_branch_reuse_gate` dans `skills/bundled/_shared/dispatch-lib.sh`).

- **P1, les PR, en négatif.** « Aucune PR ouverte sur ce head », jamais « il existe une PR mergée » : deux PR peuvent partager un `headRefName`. Et **aucune PR du tout vaut `live`** : c'est une branche de groom poussée avec son plan, pas encore implémentée. La déclarer périmée détruirait ce plan.
- **P2, la containment par patch-id.** Le même idiome que le garde #784 : `git log --cherry-mark --right-only origin/main...<tip>`, sur la ref locale **et** sur la ref `origin/…` quand elles diffèrent, car la ref de suivi locale peut survivre à la branche distante. Zéro `>` : la branche se réinitialise sur `origin/main` (`git branch -f`, après retrait du worktree par le geste unique `_stash_and_remove_worktree`). Au moins un `>` : halte sur un statut neuf, `STATUS=BRANCH_HAS_UNMERGED_WORK`, qui nomme les commits et ne touche à rien.
- **Une non-mesure retombe sur la reprise d'aujourd'hui.** C'est l'inverse du faucheur de worktrees, et c'est voulu : ici l'action est destructive (réinitialiser), donc le repli sûr est l'échec bruyant déjà outillé, le `REBASE_CONFLICT`. La ligne `branch_reuse_gate:` le dit par `action=reuse_unmeasured`.

**Règle qui se généralise.** Une porte qui reprend un état existant doit d'abord savoir si cet état est vivant. Le compter (en retard de *n* commits) ne le dit pas : un état atterri par squash a l'air en retard, et le « rattraper » rejoue ce qui a déjà atterri.

**Le nettoyage asynchrone ne remplace pas la question synchrone.** `worktree_reaper` (mika#2420) finit par retirer worktree et branche locale d'une PR terminale. Mais il passe toutes les dix minutes, après 900 s de grâce, et un dispatch arrivé avant lui reprend la branche périmée. Dans le troisième cas, le premier groom a buté et le second est reparti sain, seulement parce qu'un tick du faucheur était passé entre les deux, 2 h 32 après le merge. Quand le résultat dépend de qui passe en premier, la porte doit poser la question au moment où elle décide.

## Related Issues

- [#747](https://github.com/senara-solutions/mika/issues/747) — this fix
- [#746](https://github.com/senara-solutions/mika/pull/746) — motivating PR (stuck CONFLICTING)
- [#744](https://github.com/senara-solutions/mika/issues/744) — dashboard observability gap that let the stuck state stay hidden
- `feedback_prompt_enforcement_fragile.md` — structural > prompt-level enforcement principle

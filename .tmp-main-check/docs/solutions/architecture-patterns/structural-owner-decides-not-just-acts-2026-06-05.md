---
module: mika-skills
tags: [dispatch-lib, dev-groom, autonomous-loop, prompt-vs-structural, git-state, mika-1407, mika-1271, contract-design]
problem_type: contract-design
category: architecture-patterns
date: 2026-06-05
---

# The structural owner of an action must also own the decision to perform it

## Context

The `dev-groom` autonomous-loop pilot runs the `/mika-groom-plan-only` command (in `mika-platform/.claude/commands/`) to generate a plan, then exits; `dispatch-lib.sh::_push_branch` (in the `mika` repo) pushes the branch afterward. The [pilot-vs-substrate contract split](pilot-vs-substrate-contract-split-2026-05-25.md) already established *who DOES the git work*: the pilot owns content (the plan), dispatch-lib owns workflow (push, architect convergence, body callout). `_push_branch`'s own comment declares it "the sole git-push site for dev-groom dispatches."

But the command prompt still told the pilot to `git push -u origin <branch>` itself and, on push rejection, to exit with `Plan committed locally — remote divergence detected; abort to dispatch-lib for reconciliation`. So the *action* was owned structurally, while a *duplicate of the action plus a decision about it* lived in the prompt. On milestone-30's first Stage-1 dispatch (groom of mika#1255), the pilot fired that abort in ~30s on a branch where `HEAD == origin/<branch>` — **nothing to push** — and the groom went to `blocked`.

## Guidance

**When a deterministic structural layer already owns an action, do not also make a prompt-level layer judge whether or how to perform it.** Two harms compound:

1. **Duplication.** `_push_branch` was going to push regardless (it runs after the session, on every dispatch). The pilot's push was pure redundancy — and redundant action under two owners is how recovery layers stack recursively (the failure mode the [contract split](pilot-vs-substrate-contract-split-2026-05-25.md) was created to stop).
2. **Fragile judgment over volatile state.** To push "only if needed," the prompt forced an LLM to distinguish three git states from inside the session:
   - `HEAD == @{u}` (`origin/$BRANCH`) → nothing to push — a no-op, **not** a divergence
   - `HEAD` ahead of `@{u}` → push (fast-forward, or `--force-with-lease` if rebased)
   - branch base behind `origin/main` → a **rebase** concern, orthogonal to push
   A stale local `main` ref (sitting at the branch's merge-base while `origin/main` advanced) made the model read state 3's symptom and fire state 2's action, then abort. Git refs are exactly the kind of volatile state an LLM re-derives wrongly under prompt pressure.

The cure is to **remove the predicate, not refine it** (a smarter prompt is still an LLM judging three git states). Let the structural owner make the decision in deterministic code, keyed on the remote-tracking branch:

- **mika-platform:** the command now says generate plan, commit, **exit** — no push, no abort string. The pilot makes no push-state judgment at all.
- **mika:** `_push_branch` documents the three states explicitly and is locked by a behavioral regression test (`test_noop_when_head_equals_remote_stale_main`) proving the no-op when `HEAD == origin/<branch>` even with a stale local `main`.

This sharpens the contract split: it is not only "who DOES the git work" but **"who DECIDES it."** A decision keyed on volatile state belongs in code the action's owner controls, never in prose a downstream LLM re-derives.

## Why This Matters

This is the milestone-30 (Loop Trustworthiness) thesis in miniature: *a loop that dispatches tickets which never ship makes "backlog → 0" lie.* The pilot's pre-push diagnostic was a load-bearing predicate on the critical path of every groom; because it was wrong, every dispatch was one stale ref away from a false abort. Removing the predicate (rather than hardening the prose) is what makes the path trustworthy — it eliminates the class of failure instead of patching one instance. Reinforces the standing doctrine that prompt-level enforcement is rationalizable and fragile; structural enforcement is control (see `feedback_prompt_enforcement_fragile`, `feedback_structural_enforcement_layer_for_tool_requirements` in core memory).

## When to Apply

- Any time a prompt instructs an agent to perform an action that a deterministic layer (dispatch-lib, a hook, a post-flight step) already performs — the prompt copy is redundant and should be deleted, not kept "for safety."
- Any time a prompt asks an LLM to branch on volatile runtime state (git refs, queue depth, lock status, remote state) to decide whether to act. Move the branch into the code that owns the action.
- **Stale-local-`main` is a recurring false-"divergence" root cause.** Whenever a diagnostic compares branch state, compare against the remote-tracking branch (`@{u}` / `origin/$BRANCH`), never local `main`. Siblings: mika#1255 (first victim), mika#1364 (push lacked `--force-with-lease`), mika#1383 (chronic stall investigation) — all variants of stale local refs misdiagnosed as divergence. See also [mid-session duplicate-commit pre-push guard](../logic-errors/mid-session-duplicate-commit-pre-push-guard-2026-05-26.md).

## Examples

Before (mika-platform `/mika-groom-plan-only`, Phase 2) — the pilot judges and acts:

```
7. Push the branch:
     git push -u origin <branch>
   ... If `git push` fails because the remote is ahead, ... exit with
   `Plan committed locally — remote divergence detected; abort to
   dispatch-lib for reconciliation.`
```

After — the pilot neither pushes nor judges:

```
7. Do not push. Exit after committing. dispatch-lib's _push_branch runs after
   this session and is the sole git-push site for dev-groom dispatches. It
   pushes only when origin/$BRANCH..HEAD shows local-ahead commits ... The
   pilot performs no git push of any kind.
```

The decision now lives only here, in `_push_branch` (mika `skills/bundled/_shared/dispatch-lib.sh`), keyed on the remote-tracking branch:

```bash
ahead=$(git -C "$WORKTREE_DIR" rev-list "origin/$BRANCH..HEAD" --count 2>/dev/null || echo 0)
[ "${ahead:-0}" -eq 0 ] && return 0   # HEAD == origin/$BRANCH: nothing to push — NOT a divergence
```

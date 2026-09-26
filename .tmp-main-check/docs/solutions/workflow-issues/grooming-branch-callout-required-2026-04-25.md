---
title: Grooming-branch callout in issue body is required for /mika to resume from a committed plan
tags:
  - mika-platform
  - dispatch
  - workflow
  - claude-pilot
problem_type: workflow_issue
severity: medium
created: 2026-04-25
---

# Grooming-branch callout in issue body is required for /mika to resume from a committed plan

## Symptom

You groom a plan, commit it to a branch (e.g., `chore/48/audit-commands-kg-schema-v28-refresh`), push, then dispatch `mika ask --agent mika-dev "implement <repo> issue#<n>"`. mika-dev launches claude-pilot. claude-pilot's worktree comes up on a **different branch** (e.g., `feat/48/chore-update-audit-commands-for-kg-schem`) — derived from the issue title, NOT your grooming branch. Your committed plan file is absent from the worktree. /ce:plan re-plans from scratch instead of resuming.

Net effect: claude-pilot wastes turns re-deriving a plan you already groomed, may produce different decisions than the groomed plan, and on `error_max_turns` you've burned 200 turns of work that doesn't compose with what you committed.

## How /mika picks the branch

`/mika` (the meta-repo executor command, see `mika-platform/.claude/commands/mika.md`) has a deterministic priority order for branch derivation:

1. **Explicit `branch:<name>` prefix** in the dispatch arguments — caller supplied the branch verbatim.
2. **Issue body callout** — `gh issue view` returns the body; /mika searches for a line matching:
   ```
   > - **Branch:** `<branch-name>`
   ```
   …and uses that branch name verbatim.
3. **Deterministic recipe** from the issue title — `<type>/<issue#>/<sanitized-slug>`.

If (1) and (2) both miss, /mika falls through to (3). The recipe is correct in isolation, but if you groomed on a *different* branch name, the recipe-derived branch is fresh from main and your committed plan isn't there.

A comment on the issue with the branch reference does NOT count — the rule is **issue body**, not comments.

## How to avoid this

**Edit the issue body to include the callout BEFORE dispatching.**

```markdown
> - **Branch:** `chore/48/audit-commands-kg-schema-v28-refresh`
> - **Plan:** `docs/plans/2026-04-24-001-chore-audit-commands-kg-schema-plan.md` (committed on branch @ `<sha>`)
> - **Schema target:** v28 (or whatever applies)
> - **Grooming history:** original → reviews → status

<rest of original issue body>
```

The Plan / Schema target / Grooming history lines aren't required by /mika, but they help the implementer understand what they're picking up. Only the `> - **Branch:** ` line is load-bearing for branch derivation.

If you only have a comment with the branch reference (filed via grooming summary), **edit the body too** — `gh issue edit <n> --body-file -` works.

## How to recover when you've already dispatched on the wrong branch

1. **Stop the running claude-pilot** (`kill -TERM <pid>` or `pkill -TERM -f "claude-pilot.*<task-id>"`).
2. **Remove the stale worktree + branch:**
   ```bash
   git -C <repo> worktree remove --force .claude/worktrees/<wrong-name>/<repo>
   git -C <repo> branch -D <wrong-name>
   ```
3. **Edit the issue body** to add the canonical `> - **Branch:** ` callout.
4. **Tell mika-dev to retry** — her self-dev skill maps the keyword `retry` to `update_task_status` + re-launch `run_claude_pilot` with the same `task_id` (lines ~205–215 of `mika/skills/bundled/self-dev/system_prompt.md`):
   ```
   mika ask --agent mika-dev "retry <repo>#<n>. <one-line reason for retry>. <what was fixed>."
   ```

The retry creates a fresh callback child task; /mika now reads the body callout and creates a worktree on the right branch with the committed plan present. Phase 0.1 of /ce:plan detects the existing plan and resumes at /ce:work.

## Recovery cost

Cheap. Two retries today (mika#798 and mika-platform#48) each cost:
- ~$0.10 in mika-dev turn tokens (retry orchestration)
- Some fraction of a claude-pilot subscription session (the killed first attempt)
- 2–3 minutes wall clock to clean up + edit body + retry

Compared to the alternative (let the wrong-branch run finish, discover the divergence at PR review time, manually merge two parallel branches), this is the cheap path.

## When this can happen

Any dispatch path that goes through /mika's body-callout rule:
- `mika ask --agent mika-dev "implement <repo> issue#<n>"` (Generic Workflow)
- mika-dev's autonomous dispatch from webhooks (which is the dispatch-guard problem in `ambient-webhook-mistaken-for-dispatch-2026-04-25.md`, but the branch-callout rule applies there too)
- Direct `claude-pilot --command /mika -- <prompt>` invocations

Any time you groom-then-dispatch and your grooming branch name diverges from /mika's deterministic-recipe output.

## Why this is the right design

Don't be tempted to "fix" /mika to scan for grooming branches by issue number prefix or to read comments. The current rule is correct:
- The issue body is the canonical contract surface — comments are conversation, body is spec.
- Forcing a body edit makes the contract explicit and reviewable. Dispatch fail-fast over silent divergence.
- Comments are append-only artifacts; bodies have edit history. The body callout has audit value the comment doesn't.

The friction is in the **operator workflow**: groom-then-edit-body should be one habit. Encode it in your grooming script if you have one, or in the `mika-issue` / grooming-comment skill prompts.

## Generalization

This is one instance of a broader pattern: **commands that read issue state to make decisions need a canonical, parsable contract surface in the issue body**. Comments-as-spec is fragile because comment shapes drift across humans and bots. mika-arch v1 (mika-platform#51) follows the same convention — its plan committed on a grooming branch is callouted in the parent issue body precisely so /mika picks it up on dispatch.

## Related

- `mika-platform/.claude/commands/mika.md` — the meta-repo /mika command with the branch-derivation priority order.
- `mika/docs/solutions/workflow-issues/comment-event-fires-autonomous-dispatch-2026-04-25.md` — sibling workflow lesson from the same day documenting how comments on open issues fire autonomous claude-pilot dispatch. Today's autonomous dispatch on mika#814 honored the canonical branch callout established in this doc — the worktree came up on the right branch without re-derivation, validating the pattern under autonomous conditions.
- `feedback_secondary_pr_plan_doc.md` (memory) — related operational rule about always committing plan docs before pushing secondary cross-repo PRs.
- senara-solutions/mika-platform#51 — mika-arch v1 parent issue. Follows this convention; provided the test case for verifying the body callout approach lands cleanly.

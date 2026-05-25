---
name: mika-groom-plan-only
description: Content-only grooming for the autonomous dev-groom flow — generate plan, commit, push, exit. dispatch-lib owns architect convergence + body-callout writing.
argument-hint: "<ticket-ref> (e.g. mika issue#814, mika-platform issue#51)"
---

You are in a Claude Code session inside a feature worktree on a branch. Your job is **content-only**: generate the implementation plan for the ticket, commit it on the branch, push, and exit. **Do NOT invoke the architect. Do NOT write the body callout. Do NOT post a comment.** The autonomous dev-groom flow's outer layer (`mika/skills/bundled/_shared/dispatch-lib.sh::_iterate_groom_loop`) owns architect convergence + canonical body-callout writing under mika#1271.

This command is the autonomous-loop counterpart to `/mika-groom-ticket`. The operator-facing `/mika-groom-ticket` still runs the full Phase 1–6 pipeline (architect first/second pass + body callout + comment) and is unchanged. The split exists so the pilot doesn't redundantly invoke the architect that dispatch-lib will invoke immediately after exit.

## Input

`$ARGUMENTS` is a typed ticket reference:

- `mika issue#<n>` — issue on `senara-solutions/mika`
- `mika-platform issue#<n>` — issue on `senara-solutions/mika-platform`
- `mika-cloud issue#<n>` — issue on `senara-solutions/mika-cloud`
- `mika-skills issue#<n>` — issue on `senara-solutions/mika-skills`

Per `feedback_task_reference_format.md` — never use bare `repo#N`; the typed form is canonical.

## Exit contract

This command MUST produce all of the following before exiting:

1. A plan file committed on the grooming branch
2. The branch pushed to origin

That's it. **No body-callout write. No comment. No architect invocation.** The dispatch-lib iterate loop produces the architect-verified callout after this command exits.

If either of artifacts 1–2 is missing when the command would otherwise exit, complete them first.

## Execution

### Phase 1 — Read the ticket and pick the branch

1. Parse `<ticket-ref>` into `<repo>` and `<issue-number>`.
2. `gh issue view <issue-number> --repo senara-solutions/<repo>` — capture title, body, labels.
3. Branch slug derivation — invoke `scripts/derive-branch-name` per `/mika.md` § Branch-name derivation. (Worktree already exists; this is for context-checking the branch ref matches the expected slug.)

### Phase 2 — Set up the worktree state and draft the plan

The worktree was created by `dispatch-lib.sh::_set_up_worktree` before this command fired. You should already be inside it.

4. **Check if a plan already exists for this issue:** `find docs/plans -name "*-<issue-number>-*-plan.md" -size +500c 2>/dev/null | sort -r | head -1`. If a plan is found, reuse it as the starting point — this is an idempotent re-groom case (operator re-dispatched on an already-groomed ticket).
5. Run `/ce:plan` against the ticket. Save the plan to `<repo>/docs/plans/<YYYY-MM-DD>-<NNN>-<type>-<slug-tail>-plan.md` where `<NNN>` is the next sequence number for today.
6. **Stage and commit the plan immediately.** Unlike `/mika-groom-ticket`'s deferred-commit discipline (which gates commit on architect first-pass disposition), this command commits unconditionally. The dispatch-lib iterate loop's architect call runs AFTER this command exits and operates on the committed-on-branch plan:
   ```bash
   git add docs/plans/<file>
   git commit -m "docs(plans): groom <ticket-ref> (content-only, architect pending)"
   ```
7. **Push the branch:**
   ```bash
   git push -u origin <branch>
   ```

### Phase 3 — Exit cleanly

8. Output a brief confirmation: `Plan committed and pushed. Architect convergence pending via dispatch-lib iterate loop.`
9. Exit. The dispatch-lib outer layer (`_iterate_groom_loop`) takes over — finds the plan file, invokes `mika-arch-groom-ticket` first-pass, handles READY/ITERATE/ESCALATE, writes the canonical body-callout block via `_write_canonical_callout` on GROOMED.

## What this command does NOT do

- **No architect calls.** dispatch-lib's iterate loop invokes `mika-arch-groom-ticket` and `mika-arch-second-review` directly via `_arch_ask`. The pilot's job is content (the plan); the architect's job is convergence (the verdict).
- **No `gh issue edit`.** The canonical body callout is written by `_write_canonical_callout` in dispatch-lib, with a verified `second-pass (GROOMED)` marker and the architect session-id. The pilot must not race with the canonical writer or write a competing callout shape.
- **No `gh issue comment`.** The summary comment on the ticket belongs to the operator-direct flow (`/mika-groom-ticket`). The autonomous-loop dispatch comments via mika-dev's callback shape after dispatch-lib finishes.
- **No Phase 2.5 reconciliation checkpoint.** The architect's first-pass review catches body-vs-plan divergences as ITERATE or ESCALATE; explicit pre-architect reconciliation is operator-direct scope.
- **No companion-PR detection (step 5a).** The dispatch-lib worktree setup handles branch derivation upstream. Companion-PR cases route through operator-direct `/mika-groom-ticket`.

## Failure modes

- **Plan generation fails (/ce:plan errors):** propagate the error to the pilot session output. The dispatch-lib outer layer will surface `PIPELINE FAILURE: dev-groom produced no valid plan file` (the existing post-flight check at `dispatch-lib.sh::_run_claude_pilot` step ~589).
- **Commit fails (no changes, hook error):** propagate. The post-flight HEAD-unchanged check fires.
- **Push fails (network, permissions):** propagate. The post-flight push helper (`_push_branch`) will catch a stale HEAD if push partially succeeded.

## Related

- `/mika-groom-ticket` — the operator-facing full grooming pipeline (Phase 1–6 + architect + body callout + comment). Unchanged by sub-PR 8; this command is the autonomous-loop sibling.
- `/mika-revise-plan` — the content-only revise pilot invoked by `_launch_revise_pilot` (sub-PR 4). Same content-only pattern: read findings, revise plan, exit.
- `mika/skills/bundled/_shared/dispatch-lib.sh::_iterate_groom_loop` — the outer layer that calls this command, then invokes architect, then writes canonical callout.
- mika#1271 — contract refactor parent ticket.
- mika#1271 sub-PR 7b retirement note: with Class D shim retired and `/mika-groom-ticket`'s architect calls still active for operator-direct use, the pilot path needs its own content-only entry point — this command — so the cost regression (architect-call doubling) closes when the autonomous loop switches to use it.

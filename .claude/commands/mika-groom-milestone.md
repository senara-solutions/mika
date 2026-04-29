---
name: mika-groom-milestone
description: Groom a milestone end-to-end with mika-arch — per-sub-issue plans, two-pass architect review, sequencing record, commit to coordination branch
argument-hint: "<ticket-ref> (e.g. mika milestone#19)"
---

Take a milestone from "open with sub-issues" to "GROOMED sequencing record committed on a coordination branch, per-sub-issue plans committed on their own branches, all referenced in the milestone parent issue body, ready to dispatch." This command is the milestone sibling of `/mika-groom-ticket` — it orchestrates per-sub-issue grooming, then synthesizes a milestone-level architectural review covering cross-cutting concerns and execution ordering.

## Input

`$ARGUMENTS` is a typed milestone reference:

- `mika milestone#<n>` — milestone on `senara-solutions/mika`
- `mika-platform milestone#<n>` — milestone on `senara-solutions/mika-platform`
- `mika-cloud milestone#<n>` — milestone on `senara-solutions/mika-cloud`
- `mika-skills milestone#<n>` — milestone on `senara-solutions/mika-skills`

Per `feedback_task_reference_format.md` — never use bare `repo#N`; the typed form is canonical.

## Execution

### Phase 1 — Parse milestone ref and fetch metadata

1. Parse `<repo>` and `<milestone-number>` from `$ARGUMENTS`. Validate format: the ref MUST match `<repo> milestone#<N>` where `<N>` is a positive integer. If malformed (e.g., `mika milestone-19` or bare `mika#19`), halt with a clear error message showing the expected format.
2. Fetch milestone metadata:
   ```bash
   gh api "/repos/senara-solutions/<repo>/milestones/<N>" --jq '{title: .title, description: .description, state: .state, open_issues: .open_issues}'
   ```
3. Fetch open sub-issues:
   ```bash
   gh issue list --milestone "<N>" --repo "senara-solutions/<repo>" --json number,title,labels,body,state --state open
   ```
4. If no open sub-issues exist, report "Milestone #<N> has no open sub-issues — nothing to groom" and exit cleanly (not an error).
5. Display the milestone title, description, and the list of open sub-issues for operator confirmation before proceeding.

### Phase 2 — Set up coordination branch and worktree

6. Derive the coordination branch slug using the canonical script:
   ```bash
   SCRIPTS_DIR="$(git -C /data/workspace/mika-platform rev-parse --show-toplevel)/scripts"
   BRANCH=$("$SCRIPTS_DIR/derive-branch-name" --explicit "feat/milestone-<N>/coordination")
   ```
7. Derive the worktree path:
   ```bash
   WORKTREE_PATH=$("$SCRIPTS_DIR/derive-worktree-path" --branch "$BRANCH" --repo "<repo>")
   ```
8. Create the coordination branch + worktree, handling three sub-cases:

   **(a) Fresh creation** — worktree does not exist:
   ```bash
   git -C "<repo>" fetch origin main:main
   git -C "<repo>" worktree add "$WORKTREE_PATH" -b "$BRANCH"
   ```

   **(b) Clean reuse** — worktree exists, branch ref matches the expected slug, no uncommitted state. Reuse as-is. Phase 3+ writes additively. If a prior sequencing record exists (partial from an aborted run), treat it as a draft input — Phase 4 amends rather than overwrites; the architect's groom session sees the prior content and reconciles.

   **(c) Dispatcher-cross-file invariant violation** — worktree exists but branch ref does NOT match `feat/milestone-<N>/coordination`, OR the worktree path slug doesn't match `sanitize(branch_ref)` per `docs/solutions/best-practices/dispatcher-cross-file-invariant-2026-04-28.md`. Halt with error. Do NOT auto-rename or auto-recreate. Surface the mismatch detail to the operator; operator decides whether to remove the divergent worktree manually (`git worktree remove --force`) and re-run, or escalate.

**The coordination branch slug is IMMUTABLE for the rest of grooming.** Same invariant as `/mika-groom-ticket` Phase 1 step 4 — once the worktree is created at `<sanitized-slug>`, the branch ref and worktree path slug are bound. Per `docs/solutions/best-practices/dispatcher-cross-file-invariant-2026-04-28.md`, `git branch -m` and `git worktree move` are both prohibited.

### Phase 3 — Per-sub-issue inner loop

9. For each open sub-issue (in ascending issue-number order):

   **3a.** Run the per-ticket groom flow by directly following the phases in `.claude/commands/mika-groom-ticket.md` for that sub-issue (`<repo> issue#<sub-issue-number>`). This means: parse, derive branch, create worktree, draft plan via `/ce:plan`, stage, first-pass architect review via `/mika-ask-arch`, handle disposition (READY/ITERATE/ESCALATE), iterate if needed, second-pass review, finalize.

   **3b.** Capture the per-sub-issue outcome:
   - `disposition`: the final disposition from the per-ticket flow (READY, ITERATE, ESCALATE, or GROOMED after second-pass)
   - `plan_path`: the committed plan file path (e.g., `docs/plans/2026-04-29-003-feat-874-kg-corpus-fix-plan.md`)
   - `branch_slug`: the per-sub-issue branch slug
   - `session_id`: the mika-arch session_id from the per-ticket first-pass

   **3c.** Short-circuit for already-groomed sub-issues (R5 idempotence): if the per-ticket worktree already exists with a committed plan AND the issue body already has the `> - **Branch:**` callout from a prior groom run, skip the per-ticket flow for that sub-issue. Record its prior disposition as READY (it was groomed in a previous run) and move on. This is the linkage R5 inherits from the per-ticket flow.

   **3d.** If any sub-issue returns ESCALATE, record it and continue to the next sub-issue. Do NOT halt the entire loop on a single sub-issue ESCALATE — complete all sub-issues first, then aggregate in Phase 4.

10. After all sub-issues are processed, compute the aggregate disposition per D8:
    - **All sub-issues READY or GROOMED** --> milestone disposition `READY`
    - **At least one ITERATE, none ESCALATE** --> milestone disposition `ITERATE`
    - **At least one ESCALATE** --> milestone disposition `ESCALATE`

    Display the per-sub-issue disposition table to the operator:
    ```
    Sub-issue dispositions:
    - #874: READY (plan: docs/plans/2026-04-29-003-...-plan.md, branch: feat/874/...)
    - #875: READY (plan: docs/plans/2026-04-29-004-...-plan.md, branch: feat/875/...)
    - #876: ITERATE (plan: docs/plans/2026-04-29-005-...-plan.md, branch: feat/876/...)
    - #877: ESCALATE (no plan committed)

    Aggregate milestone disposition: ESCALATE (highest-severity-wins)
    ```

### Phase 4 — Milestone-level first-pass via mika-arch-groom-milestone skill

11. Compose the milestone-level brief. Include:
    - Milestone metadata (title, description, number of sub-issues)
    - For each sub-issue: title, disposition, plan path, branch slug, key decisions from plan
    - Dependency context: any cross-sub-issue relationships identified during per-sub-issue grooms
    - The milestone-level aggregate disposition from step 10
    - Any existing partial sequencing record from the coordination worktree (if reusing per Phase 2 sub-case b)

    Save the brief to `/tmp/groom-brief-<repo>-milestone-<N>-pass1.md`.

12. Send to mika-arch via the milestone skill:
    ```
    /mika-ask-arch @/tmp/groom-brief-<repo>-milestone-<N>-pass1.md
    ```
    Capture the `session_id` from the `session_id: <uuid>` line emitted by `/mika-ask-arch`. The source of truth is JSON metadata (`metadata.session_id`). If no `session_id:` line appears, that is a contract violation; halt with an error.

13. Parse the response. Look for:
    - `Scope: milestone` — confirms the skill processed this as a milestone-shaped input (not per-ticket).
    - `Disposition: READY` — sequencing is sound. Proceed to Phase 5.
    - `Disposition: ITERATE` — apply the architect's specific concerns. Update sub-issue plans or sequencing as directed, then proceed to Phase 5.
    - `Disposition: ESCALATE` — surface to the operator with the architect's reasoning. Do not commit the sequencing record. The operator decides whether to drop the escalated concern, rework, or escalate to Vincent.

    The `Disposition: <KEYWORD>` MUST be the literal final line of the response per literal-final-line discipline (`docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md`).

    Tolerate paraphrased dispositions (e.g., "Proceed" instead of "READY") per the known prompt-adherence issue, but log a warning.

### Phase 5 — External second-pass (carve-out per D5)

14. This phase does NOT route to mika-arch. Per the recursive self-review carve-out (`docs/solutions/best-practices/recursive-self-review-carve-out-2026-04-26.md`, now codified in `docs/architecture/review-guide.md` section 7), when a milestone touches mika-arch's own operational surface or when the reviewing agent is structurally vested in the outcome, the second-pass routes to an external reviewer.

15. Compose the consolidated second-pass brief:
    - The milestone-level first-pass session_id and disposition
    - The per-sub-issue plan paths and dispositions
    - The draft sequencing record (assembled from Phase 4 output)
    - Any changes applied after first-pass ITERATE feedback
    - Remaining uncertainties or disagreements

    Save to `/tmp/groom-brief-<repo>-milestone-<N>-pass2.md`.

16. Emit a clear pause-and-ask to the operator:
    ```
    --- SECOND-PASS EXTERNAL REVIEW REQUIRED ---

    The milestone-level first-pass is complete. Per the recursive self-review carve-out
    (docs/architecture/review-guide.md section 7), the second-pass for milestone grooming
    routes to an external reviewer (Vincent or Claude Chat), not mika-arch.

    The consolidated brief is at: /tmp/groom-brief-<repo>-milestone-<N>-pass2.md

    Please review the brief and respond with one of:
    - Verdict: GROOMED — plan and sequencing are dispatch-ready
    - Verdict: ESCALATE — concerns require further rework

    Paste the verdict (and any feedback) below.
    ```

17. Parse the operator's response for `Verdict: GROOMED` or `Verdict: ESCALATE`. There is **no third pass** — if GROOMED has not been reached after pass 2, escalation is the only path forward.

### Phase 6 — Finalize

18. Draft the sequencing record using the template at `docs/plans/templates/milestone-sequencing-record-template.md`. Populate all sections from Phase 4 output:
    - `## Sub-issues` — each sub-issue with priority, plan path, branch slug
    - `## Dependencies` — cross-sub-issue dependency edges identified by mika-arch
    - `## Recommended GitHub blockedBy edits` — the `gh issue edit` commands to apply
    - `## Order` — recommended execution order (parallel sets where independent)
    - `## Cross-cutting concerns` — entities/files/contracts touched by multiple sub-issues
    - `## Open milestone-level questions` — unresolved questions with resolution paths

    Save to `<worktree-path>/docs/plans/<YYYY-MM-DD>-<NNN>-milestone-<N>-sequencing.md` where `<NNN>` is the next sequence number for today.

19. Apply any final tweaks from the GROOMED verdict (often there are none).

20. Commit the sequencing record on the coordination branch:
    ```bash
    git -C "$WORKTREE_PATH" add docs/plans/<sequencing-file>
    git -C "$WORKTREE_PATH" commit -m "docs(plans): milestone#<N> sequencing record (GROOMED)"
    ```

21. Push all branches:
    ```bash
    # Push the coordination branch
    git -C "$WORKTREE_PATH" push origin "$BRANCH"
    # Per-sub-issue branches were already pushed by their respective /mika-groom-ticket runs
    ```

22. Update the milestone parent issue body to attach the canonical callouts. Read the existing body, prepend (or merge into existing callouts at the top):
    ```
    > - **Coordination branch:** `feat/milestone-<N>/coordination`
    > - **Sequencing record:** `<repo>/docs/plans/<sequencing-file>` (committed on branch @ `<sha>`)
    > - **Sub-issues:** #<a> (groomed), #<b> (groomed), #<c> (iterate), #<d> (escalate)
    > - **Grooming history:** /ce:plan per-sub-issue -> mika-arch first-pass (<disposition>) -> external second-pass (<verdict>)
    ```
    Use `gh issue edit <milestone-parent-issue> --repo senara-solutions/<repo> --body-file <tmpfile>` to apply. If the milestone does not have a tracking issue (milestones are GitHub milestone objects, not issues), attach the callout as a comment on the milestone's first sub-issue with a note: "This comment tracks the milestone#<N> coordination artifacts."

23. Post a comment on the milestone's tracking issue (or first sub-issue) summarizing:
    ```
    Groomed end-to-end via /mika-groom-milestone.

    Architect (mika-arch) first-pass: <disposition> on session `<session_id>`.
    External second-pass: <verdict>.

    Sequencing record: `<repo>/docs/plans/<sequencing-file>` on branch `<coordination-branch>` (@ `<sha>`).

    Sub-issues:
    - #<a>: GROOMED (plan on branch `<slug-a>`)
    - #<b>: GROOMED (plan on branch `<slug-b>`)
    ...

    Ready to dispatch in order per sequencing record.
    ```

### Phase 7 — Optional dispatch

24. Ask the operator whether to dispatch immediately. Do NOT auto-dispatch — the operator chooses when and in what order to dispatch sub-issues. If the operator confirms, dispatch sub-issues in the order specified by the sequencing record:
    ```
    mika ask --agent mika-dev "implement <repo> issue#<first-in-order>"
    ```

## Idempotency and recovery

- If the coordination worktree exists when this command runs, it is reused per Phase 2 sub-cases (a)/(b)/(c).
- Per-sub-issue worktrees follow the same reuse logic from `/mika-groom-ticket` Phase 2 — existing plans are reused as starting points (R5).
- Already-groomed sub-issues (those with committed plans and issue-body callouts from a prior run) are short-circuited in Phase 3 step 9c. Re-running the milestone groom concentrates work on the not-yet-groomed sub-issues.
- If mika-arch returns ESCALATE at Phase 4, the command halts. Per-sub-issue plans are already committed on their branches (they don't roll back). The coordination branch stays without a sequencing record. Operator intervention: rework the escalated concern, then re-run this command.
- If the external second-pass returns ESCALATE at Phase 5, the command halts. Same recovery: operator reworks, re-runs.
- If `git push` fails (network, permissions), retry. Plans and the sequencing record are committed locally and not lost.

## Disposition aggregation (D8)

Per-sub-issue dispositions aggregate to the milestone-level disposition by **highest-severity-wins** ordering:

| Per-sub-issue outcomes | Milestone disposition |
|---|---|
| All READY/GROOMED | `Disposition: READY` |
| At least one ITERATE, none ESCALATE | `Disposition: ITERATE` |
| At least one ESCALATE | `Disposition: ESCALATE` |

The aggregation rule is enforced by this operator command, not by the skill prompt. The skill emits per-sub-issue dispositions individually; this command aggregates and computes the milestone-level disposition.

## Discipline this command embodies

- **Plan-on-branch is the contract.** Each sub-issue gets its own plan-on-branch (via the per-ticket flow). The sequencing record on the coordination branch is the milestone-level contract binding them together.
- **Two passes max.** No third-pass softball. ESCALATE is a real outcome, not a delay tactic.
- **Recursive self-review carve-out.** Second-pass routes to an external reviewer when the milestone touches the architect's own operational surface (per `docs/architecture/review-guide.md` section 7).
- **Citation-or-silence.** mika-arch's output discipline flows through unchanged; this command doesn't second-guess the architect.
- **Coordination branch callout is canonical.** The `> - **Coordination branch:**` callout is the discovery surface for the sequencing record, just as `> - **Branch:**` is for per-ticket plans.
- **Branch slug immutability.** Per `docs/solutions/best-practices/dispatcher-cross-file-invariant-2026-04-28.md`, once a worktree is created the branch slug is bound. Semantic concerns go in plan filenames and frontmatter, never `git branch -m`.
- **Centralized derivation.** All branch and worktree path derivation uses `scripts/derive-branch-name` and `scripts/derive-worktree-path`. Never re-derive. Per `docs/solutions/cross-repo-patterns/centralized-derivation-load-bearing-invariant-2026-04-28.md`.

## Related

- `/mika-groom-ticket` — per-ticket sibling. The per-sub-issue inner loop (Phase 3) reuses this command's full phase structure.
- `/mika-ask-arch` — per-call wrapper for mika-arch. Used in Phase 4 for milestone-level first-pass.
- `docs/plans/templates/milestone-sequencing-record-template.md` — sequencing record schema used in Phase 6.
- `docs/architecture/review-guide.md` section 7 — self-review boundary (codified carve-out rule for when second-pass routes external).
- `docs/solutions/best-practices/recursive-self-review-carve-out-2026-04-26.md` — historical evidence and original carve-out reasoning.

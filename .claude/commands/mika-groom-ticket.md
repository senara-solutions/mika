---
name: mika-groom-ticket
description: Groom a ticket end-to-end with mika-arch — plan, two-pass architect review, commit plan to branch, attach branch to issue body
argument-hint: "<ticket-ref> (e.g. mika issue#814, mika-platform issue#51)"
---

Take a ticket from "open with description" to "GROOMED plan committed on a branch and referenced in the issue body, ready to dispatch." This command embodies the canonical grooming discipline established in `mika/docs/solutions/workflow-issues/grooming-branch-callout-required-2026-04-25.md` — the GROOMED plan must live on the branch, not just in conversation memory or a comment.

## Input

`$ARGUMENTS` is a typed ticket reference:

- `mika issue#<n>` — issue on `senara-solutions/mika`
- `mika-platform issue#<n>` — issue on `senara-solutions/mika-platform`
- `mika-cloud issue#<n>` — issue on `senara-solutions/mika-cloud`
- `mika-skills issue#<n>` — issue on `senara-solutions/mika-skills`

Per `feedback_task_reference_format.md` — never use bare `repo#N`; the typed form is canonical.

## Exit contract

This command MUST produce all of the following before exiting, regardless of invocation mode (interactive or autonomous):

1. A plan file committed on the grooming branch
2. The branch pushed to origin
3. Branch + Plan + Grooming-history callouts written to the issue body via `gh issue edit`
4. A summary comment posted on the issue via `gh issue comment`

If any of these are missing when the command would otherwise exit, complete them first. The `ready` label is explicitly NOT part of this contract (operator-only per mika-platform issue#112).

**Exception — ESCALATE disposition:** When mika-arch returns ESCALATE (Phase 3 step 10 or Phase 4 step 15), or when the reconciliation checkpoint returns ESCALATE-divergence (Phase 2.5), the command halts immediately WITHOUT producing artifacts 1-4. The plan stays uncommitted in the worktree; Vincent decides whether to proceed. The exit contract applies only to successful grooming paths (READY or GROOMED dispositions).

## Execution

### Phase 1 — Read the ticket and pick the branch

1. Parse `<ticket-ref>` into `<repo>` and `<issue-number>`.
2. `gh issue view <issue-number> --repo senara-solutions/<repo>` — capture title, body, labels.
3. Branch slug derivation — invoke the canonical script `scripts/derive-branch-name`. It enforces the priority order (body callout → conventional-commit prefix in title → label override → default `feat`) and the canonical 40-char truncation.
   ```bash
   SCRIPTS_DIR="$(dirname "$(git rev-parse --git-common-dir)")/scripts"
   BRANCH=$("$SCRIPTS_DIR/derive-branch-name" \
     --title "$ISSUE_TITLE" \
     --issue "$ISSUE_NUMBER" \
     --labels "$LABELS" \
     --body-callout "$ISSUE_BODY")
   ```
4. Record the chosen branch name as bound for the remainder of this grooming session. To override, abort and re-invoke `/mika-groom-ticket` with a `branch:<name>` upstream prefix per `/mika.md` § Branch-name derivation priority 1.

**The branch slug is IMMUTABLE for the rest of grooming.** Once Phase 2 step 5 creates the worktree at `<sanitized-slug>`, the branch ref and the worktree path slug are bound. Architect Finding-5-class concerns ("the work is `refactor` not `feat`, rename the branch") MUST NOT trigger `git branch -m`. Capture semantic accuracy in the **plan filename** (`docs/plans/<date>-<NNN>-<actual-type>-<slug-tail>-plan.md`) and the plan's frontmatter `type:` field. The branch slug carries label-derived type, the plan carries semantic type, and they can disagree without breaking anything.

**Why immutable:** `git branch -m` renames the ref but does NOT move the worktree directory; `git worktree move` moves the directory but doesn't rename the ref. They drift independently, and the dispatcher (`mika/skills/bundled/claude-pilot/handlers/run.sh`) computes the expected worktree path freshly from the current branch slug at dispatch time. If the branch ref says `refactor/844/...` but the worktree directory is still at `feat-844-...`, dispatch hits exit 128 (`fatal: branch already checked out at $OLD_PATH`). Hit on mika#844 dispatch 2026-04-28; investigated and root-caused the same morning. The fix at the spec level is to never introduce the drift in the first place.

### Phase 2 — Set up the worktree and draft the plan

**Repo directory resolution.** Resolve the git directory for `$REPO` before worktree operations.
This mirrors `dispatch-lib.sh` lines 176-181 — `mika-platform` IS the meta-repo root, not a
subdirectory of itself:

```bash
if [ "$REPO" = "mika-platform" ]; then
    SUB_REPO_DIR="$(pwd)"  # meta-repo root
else
    SUB_REPO_DIR="$REPO"   # subdirectory of meta-repo
fi
```

All `git -C` commands in steps 5 and 5a use `$SUB_REPO_DIR`, NOT `$REPO`.
`$REPO` is still used for `--repo` flag in `derive-worktree-path` and for GitHub API calls.

5a. **Companion PR detection.** Before creating a fresh branch from main, parse the ticket body for a `> **Companion PR:** #<N>` callout (a parent-PR reference indicating this ticket is a follow-up that depends on the parent PR's changes). If present:

   ```bash
   # Fetch both state and headRefName in a single API call to avoid TOCTOU races.
   COMPANION_JSON=$(gh pr view <N> --repo senara-solutions/"$REPO" --json state,headRefName)
   COMPANION_STATE=$(echo "$COMPANION_JSON" | jq -r .state)
   COMPANION_REF=$(echo "$COMPANION_JSON" | jq -r .headRefName)

   if [ "$COMPANION_STATE" = "OPEN" ]; then
     # Companion is open — branch from its HEAD so the follow-up can compile.
     git -C "$SUB_REPO_DIR" fetch origin "$COMPANION_REF"
     WORKTREE_PATH=$("$SCRIPTS_DIR/derive-worktree-path" --branch "$COMPANION_REF" --repo "$REPO")
     git -C "$SUB_REPO_DIR" worktree add "$WORKTREE_PATH" -b "$COMPANION_REF" "origin/$COMPANION_REF"
     # Override the push target for steps 18-19.
     ACTIVE_BRANCH="$COMPANION_REF"
     COMPANION_DETECTED=true
     # Skip step 5 — worktree is set up on the companion branch.
   elif [ "$COMPANION_STATE" = "MERGED" ]; then
     # Companion already merged to main — fall through to step 5 (branch from main).
     echo "Companion PR #<N> already merged; branching from main."
     COMPANION_DETECTED=false
   else
     # CLOSED (abandoned) — do not branch from a rejected PR.
     echo "Companion PR #<N> is $COMPANION_STATE (not OPEN); branching from main."
     COMPANION_DETECTED=false
   fi
   ```

   **Why this exists:** Follow-up tickets that depend on an open PR's changes can't compile against main. Three incidents on the same surface established this as a spec gap, not an edge case:
   1. mika#844 — slug-immutability invariant added after dispatcher-drift incident.
   2. milestone#19 coordination branch — `derive-branch-name` regex mismatch, deviation form B shipped.
   3. mika#918 (reference incident) — follow-up to open mika#915. Const-assert references `CURRENT_SCHEMA_VERSION` which #915 bumps from 28→29; cannot compile against main. Resolved by grooming on #915's existing branch (`fix/908/agent-tool-calls-redact-secret-shaped`) instead of creating a separate fork.

   **State handling:** The guard explicitly checks for `OPEN` (use companion HEAD), `MERGED` (fall through to main — companion's changes are already there), and any other state including `CLOSED` (fall through to main — do not branch from a rejected PR). A single `gh pr view` call fetches both `state` and `headRefName` atomically, eliminating the TOCTOU window where a PR could merge between two separate API calls.

   **Variable assignments:** When the companion path activates, it sets `ACTIVE_BRANCH="$COMPANION_REF"` (the push target for step 18) and `COMPANION_DETECTED=true` (the guard for step 5). In the standard path, `ACTIVE_BRANCH` defaults to `$BRANCH` (the Phase 1 ticket-derived slug).

   **Slug-immutability invariant (mika#844) holds:** When a companion PR is detected, the slug is the companion PR's existing `headRefName`, not derived from the current ticket. `derive-worktree-path` derives from the branch ref, not the ticket number, so dispatch path encoding stays consistent.

   **Plan filename convention:** The plan filename still uses the *current ticket's* number (`<NNN>` = current issue). The branch ref encodes the parent PR's number. They are allowed to disagree — same pattern the spec already endorses for type drift between branch slug and plan frontmatter.

   If no `> **Companion PR:**` callout is found, fall through to step 5 with `COMPANION_DETECTED=false`.

5. **Skip if `COMPANION_DETECTED=true`** (step 5a already created the worktree on the companion branch). Otherwise, create the branch + worktree in the target repo. The canonical worktree path comes from `scripts/derive-worktree-path`, which enforces the invariant `worktree_path_slug == sanitize(branch_ref)` and emits an absolute path:
   ```bash
   WORKTREE_PATH=$("$SCRIPTS_DIR/derive-worktree-path" --branch "$BRANCH" --repo "$REPO")
   git -C "$SUB_REPO_DIR" fetch origin main:main
   git -C "$SUB_REPO_DIR" worktree add "$WORKTREE_PATH" -b "$BRANCH"
   ```
   If the worktree already exists (someone groomed earlier and didn't push), reuse it.

   > **Regression guard (mika-platform#116):** Both this command and `dispatch-lib.sh` MUST:
   > 1. Derive worktree paths via `scripts/derive-worktree-path --branch <branch> --repo <repo>`
   >    (the `--repo` flag is mandatory — never `--no-repo` — producing a nested `/<repo>` suffix)
   > 2. Use `$SUB_REPO_DIR` (not `$REPO`) for `git -C` commands (`mika-platform` is the meta-repo
   >    root, not a subdirectory of itself)
   >
   > If this command creates a worktree at a different path shape than dispatch-lib.sh would
   > compute, autonomous-loop dispatch will fail with exit 128.

6. Run `/ce:plan` against the ticket. Save the plan to `<repo>/docs/plans/<YYYY-MM-DD>-<NNN>-<type>-<slug-tail>-plan.md` where `<NNN>` is the next sequence number for today (find by `ls <repo>/docs/plans/<date>-*` and incrementing).
7. **Stage the plan only — do NOT commit yet.** The plan stays on disk in the worktree, but the commit is gated on first-pass architect validation:
   ```bash
   git -C <worktree-path> add docs/plans/<file>
   # No commit yet — the plan is unvalidated state.
   ```
   Why staging-not-committing: committing un-validated state creates indistinguishable lineage between "architect approved this" and "operator drafted this." Yesterday's mika#814 dogfood demonstrated the failure mode — the GROOMED-plan-on-comment looked authoritative but never reached the implementer; claude-pilot derived from scratch in a clean worktree and shipped a different shape. Holding the commit until first-pass disposition is parseable means the first commit on the branch *is* an architect-validated commit. If first-pass returns `READY`, commit directly with the validated message. If it returns `ITERATE`, commit after Phase 4's revisions land. If it returns `ESCALATE`, the worktree stays uncommitted and Vincent decides whether to discard or proceed manually — the branch stays clean either way. Plan-on-branch is still the contract; the contract just doesn't bind until the architect signs.

### Phase 2.5 — AC-vs-plan reconciliation checkpoint

Before composing the first-pass architect brief, verify the staged plan reconciles with the issue body. This step exists to catch operator-authored divergences (issue body says X, plan says Y) before any architect spend — replacing a fuzzy first-pass ESCALATE with a concrete body-vs-plan diff the operator can resolve.

**Why this exists.** The "three-in-one-pass spec-divergence ESCALATE cluster" of 2026-05-17 — mika#1188 F2 (issue premise on SDK auto-mode contradicted by codebase), mika#1189 F2 (issue scope claimed 4 endpoints, plan landed at 1; `mika.db` location wrong), mika#1190 F3 (AC1 claimed YAML/JSON, existing eval suites are all Rust); prior mika#1173 F2 — each burned architect cycles producing ESCALATEs that resolved to "the body says X, the plan says Y." Catching these at the checkpoint avoids the architect-pass spend and surfaces a body-vs-plan diff for the operator instead.

**Procedure.** Re-fetch the issue body fresh with `gh issue view <n> --repo senara-solutions/<repo> --json body` — do NOT reuse the body captured at step 2, because operator edits between Phase 1 and Phase 2.5 must be picked up. Then compare three axes between the freshly-fetched body and the staged plan:

- **AC text.** Every `ACn` line in the issue's Acceptance Criteria section vs every AC tie-back in the plan's commitments. A divergence is an AC claim in the body with no plan commitment matching it (or vice versa), or an AC text that disagrees with the plan's deliverable description.
- **Scope.** The issue's `## Out of scope` section, any `v1 ships <X>` / `v1 scope is <X>` statements, and explicit boundary text in the description vs the plan's scope sections and deliverables list. A divergence is a body-out-of-scope item the plan addresses, or a body-in-scope item the plan omits.
- **Sequence.** Phase ordering, `blockedBy` references, `> - **Companion PR:**` callouts, and explicit dependency statements vs the plan's phase/step ordering. A divergence is body-stated `A → B` with plan-stated `B → A`.

A fourth axis is also reportable: **premise** — body factual claims (file paths, table names, API shapes, library availability) contradicted by what the planner read in the codebase. mika#1188 F2 (SDK auto-mode) and mika#1190 F3 (codebase eval convention) are both premise divergences.

**Output.** Write the divergence list to `/tmp/groom-divergence-<repo>-<n>.md`, one entry per divergence, each with the body claim and the plan claim quoted verbatim:

```
- **Divergence (AC1):** body claims `"<verbatim from issue>"` — plan derives `"<verbatim from plan>"`
- **Divergence (scope):** body out-of-scope lists `"<X>"` — plan addresses `<X>` at step `<N>`
- **Divergence (sequence):** body sequence `"<A → B>"` — plan sequence `"<B → A>"`
- **Divergence (premise):** body claim `"<file.rs has fn Y>"` — plan finding `"<file.rs has fn Z; Y does not exist>"`
```

**Halt or proceed.**
- **Zero divergences:** proceed to step 8 (first-pass brief composition).
- **One or more divergences:** halt with `Disposition: ESCALATE-divergence`. Output the divergence list to the operator (paste from `/tmp/groom-divergence-<repo>-<n>.md`). Do NOT commit the plan. Do NOT compose the first-pass architect brief. The worktree stays as-is.

**Operator resolution paths.**
1. Edit the issue body to match the plan, then re-run `/mika-groom-ticket` (the existing plan is reused per the recovery clause).
2. Edit the plan to match the issue body, then re-run.
3. Override (rare): proceed manually — invoke `/mika-ask-arch` directly with a brief that includes both the staged plan AND the divergence list, letting the architect arbitrate. Use only when the divergence is genuinely architect-judgment territory (e.g., a tradeoff between two valid scope shapes), not a body-vs-plan factual mismatch.

**Why this lives before the brief, not after.** The architect's job is evaluating the plan against the codebase, not arbitrating between two operator-authored artifacts. Reconciliation is operator-judgment territory; the architect signs the plan once the operator has resolved the body↔plan diff.

### Phase 3 — First-pass architect review

8. Run `/mika-ask-a-friend` against the current plan + ticket context. The `/mika-ask-a-friend` output is a markdown peer-review brief. Save it to `/tmp/groom-brief-<repo>-<n>-pass1.md`.
9. Send to mika-arch:
   ```
   /mika-ask-arch @/tmp/groom-brief-<repo>-<n>-pass1.md
   ```
   Capture the `session_id` from the `session_id: <uuid>` line emitted by `/mika-ask-arch`. The source of truth is JSON metadata (`metadata.session_id`) — the printed trailer line is a re-emission for backward compatibility. If no `session_id:` line appears, that is a contract violation; halt with an error.
10. Parse the response. Look for:
    - `Disposition: READY` — plan is sound. Commit the staged plan with `git commit -m "docs(plans): groom <ticket-ref> (architect READY first-pass)"` and skip to Phase 5.
    - `Disposition: ITERATE` — apply the architect's specific concerns to the staged plan, then commit with `git commit -m "docs(plans): groom <ticket-ref> initial plan"`. Continue to Phase 4 with the now-committed initial plan as the base for the iteration commit.
    - `Disposition: ESCALATE` — surface to Vincent with the architect's reasoning. Do not commit. The staged plan stays in the worktree; Vincent decides whether to discard or proceed manually.

    Tolerate paraphrased dispositions (e.g., "Proceed" instead of "READY") per the known prompt-adherence issue, but log a warning.
    <!-- TODO: Sunset paraphrase tolerance once mika/skills/bundled/mika-arch-groom-ticket/system_prompt.md requires literal "Disposition: <KEYWORD>" as the final line of the response. See mika/docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md for the empirical drift this guards against. -->

### Phase 4 — Apply iterations and second-pass review

11. Update `<repo>/docs/plans/<file>` to address each architect concern from the first pass. Be specific: each concern → one named change in the plan.
12. Commit the revisions to the branch:
    ```bash
    git -C <worktree-path> add docs/plans/<file>
    git -C <worktree-path> commit -m "docs(plans): address mika-arch first-pass review (<ticket-ref>)"
    ```
13. Compose the second-pass brief: include the prior session_id context, the changes applied, and a list of remaining uncertainties (the parts you didn't change because you weren't sure or didn't agree). Save to `/tmp/groom-brief-<repo>-<n>-pass2.md`.
14. Send to mika-arch with session continuity:
    ```
    /mika-ask-arch --session-id <captured-from-step-9> @/tmp/groom-brief-<repo>-<n>-pass2.md
    ```
15. Parse the second-pass response. Look for:
    - `Verdict: GROOMED` — plan is final; proceed to Phase 5
    - `Verdict: ESCALATE` — surface to Vincent; do not auto-dispatch

    There is **no third pass** (per spec §4.5 / R11). If GROOMED has not been reached after pass 2, escalation is the only path forward.

### Phase 5 — Finalize and attach

16. Apply any final tweaks from the GROOMED verdict (often there are none — second-pass GROOMED means dispatch-ready).
17. Final commit if the plan changed:
    ```bash
    git -C <worktree-path> add docs/plans/<file>
    git -C <worktree-path> commit -m "docs(plans): apply mika-arch second-pass GROOMED feedback (<ticket-ref>)"
    ```
18. Push the branch. Use `$ACTIVE_BRANCH` (set by step 5a if companion detected, otherwise defaults to `$BRANCH` from Phase 1):
    ```bash
    git -C <worktree-path> push origin "$ACTIVE_BRANCH"
    ```
19. Update the issue body to attach the canonical callouts. Read the existing body, prepend (or merge into existing callouts at the top):
    **Standard form** (new branch from main):
    ```
    > - **Branch:** `<slug>`
    > - **Plan:** `<repo>/docs/plans/<file>` (committed on branch @ `<sha-of-final-commit>`)
    > - **Grooming history:** /ce:plan → mika-arch first-pass (<disposition>) → revisions → mika-arch second-pass (GROOMED)
    ```

    **Companion PR form** (follow-up branched from an open PR via step 5a):
    ```
    > - **Branch:** `<companion-ref>` (Companion PR #<N>)
    > - **Plan:** `<repo>/docs/plans/<file>` (committed on branch @ `<sha>`)
    > - **Grooming history:** /ce:plan → mika-arch first-pass (<disposition>) → revisions → mika-arch second-pass (GROOMED)
    ```

    Use `gh issue edit <n> --repo senara-solutions/<repo> --body-file <tmpfile>` to apply.
20. Post a comment on the ticket summarizing. **The closing comment MUST NOT contain the literal Layer 1 routing pattern** (`implement <repo> issue#<n>` / `implement <repo> milestone#<n>` / `implement <repo> project#<n>`) anywhere in its body — that pattern, when delivered as the body of a `[GitHub] New comment` webhook event, can match mika-dev's Layer 1 classifier despite the bracketed-prefix source-check rule (mika#841 prompt-adherence drift, three documented incidents: mika#798 → mika#838 → mika#906). Use the `ready` label as the canonical dispatch instruction instead — that's the actual dispatch path per mika#841 design, and it routes through engine-side guards (`webhook_ready_label_dispatch` mika#847, `webhook_no_unauthorized_dispatch` mika#910).

    ```
    Groomed end-to-end via /mika-groom-ticket.

    Architect (mika-arch) verdict: GROOMED on session `<session_id>`.

    Plan: `<repo>/docs/plans/<file>` on branch `<slug>` (@ `<sha>`).

    To proceed: add the `ready` label on this issue (canonical positive-consent
    per mika#841). Operator-only step.
    ```

### Completion gate (mandatory — do NOT exit before all checks pass)

Before proceeding to Phase 6, verify all four exit-contract artifacts. If any check fails, execute the missing step immediately:

1. **Plan committed?** Run `git -C <worktree-path> log --oneline -1 -- docs/plans/<file>`. Must show a commit containing the plan file. If empty, stage first (`git -C <worktree-path> add docs/plans/<file>`) then commit with the appropriate message from steps 10/12/17.
2. **Branch pushed?** First check if the remote ref exists: `git -C <worktree-path> ls-remote --exit-code origin $ACTIVE_BRANCH`. If exit code is non-zero (no remote ref), the branch has never been pushed — run `git -C <worktree-path> push --set-upstream origin $ACTIVE_BRANCH`. If the remote ref exists, run `git -C <worktree-path> log origin/$ACTIVE_BRANCH..HEAD --oneline` — must be empty (all local commits exist on remote). If non-empty, run `git -C <worktree-path> push origin $ACTIVE_BRANCH`.
3. **Callout on issue body?** Run `gh issue view <n> --repo senara-solutions/<repo> --json body -q .body | grep -cF '**Plan:**'`. Must return ≥1. If 0, write the callout per step 19.
4. **Comment posted?** Run `gh issue view <n> --repo senara-solutions/<repo> --json comments --jq '[.comments[].body | select(contains("Groomed end-to-end via /mika-groom-ticket"))] | length'`. Must return ≥1. If 0, post the grooming summary per step 20.

All four checks must pass before this phase completes.

### Phase 6 — Dispatch readiness

The grooming is complete — all exit-contract artifacts have been verified by the completion gate.

The `ready` label is an **OPERATOR-ONLY** action per mika-platform issue#112. This command does NOT apply it.

**Interactive invocation:** Inform the operator that the plan is groomed and ready for dispatch. The canonical dispatch path is to apply the `ready` label on the issue.

**Autonomous invocation (claude-pilot):** Exit cleanly. The caller (dispatch-lib.sh) handles post-flight validation and callback delivery. Do not attempt to interact with a user or parent prompt.

## Idempotency and recovery

- If the worktree exists when this command runs, the existing plan is reused as the starting point. The architect rounds run again on top of any revisions already present.
- If mika-arch returns ESCALATE at either pass, the command halts. The branch + plan stay committed. Vincent's intervention path: edit the plan directly, then either re-run this command (which restarts both passes) or run `/mika-ask-arch` manually to settle the escalation.
- If Phase 2.5 returns ESCALATE-divergence, the plan stays staged but uncommitted; the divergence list lives at `/tmp/groom-divergence-<repo>-<n>.md`. Vincent's intervention path: edit the issue body, the plan, or both to resolve the divergence, then re-run this command (the existing plan is reused per the first bullet).
- If `git push` fails (network, permissions), retry. The plan is committed locally and not lost.

## Discipline this command embodies

- **The plan-on-branch is the contract.** Conversation memory is volatile; the comment is a discovery surface. The branch + commit history is what /mika's /ce:plan resumes from.
- **Two passes max.** No third-pass softball. ESCALATE is a real outcome, not a delay tactic.
- **Citation-or-silence.** mika-arch's output discipline (per `mika/docs/architecture/review-guide.md` § 6) flows through unchanged; this command doesn't try to second-guess the architect.
- **Branch callout is canonical.** The `> - **Branch:**` callout is parsed by `/mika` for branch derivation; getting it on the body is what prevents the "right branch, no plan" failure mode from yesterday's #814 run.
- **Reconcile before the architect.** Operator-authored divergences (body says X, plan says Y) belong to operator judgment, not architect judgment. Phase 2.5 catches them as a body-vs-plan diff before any architect spend, replacing fuzzy first-pass ESCALATEs with a concrete diff the operator can resolve.

## Related

- `/mika-ask-arch` — the per-call wrapper this command uses for both passes.
- `/mika-ask-a-friend` — produces the peer-review brief shape this command sends to the architect.
- `mika/docs/solutions/workflow-issues/grooming-branch-callout-required-2026-04-25.md` — why the branch callout matters.
- `mika/docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` — known prompt-adherence drift on first-pass disposition keyword.
- `mika/docs/architecture/review-guide.md` — the principles mika-arch reviews against.
- mika#1188 F2, mika#1189 F2, mika#1190 F3 (the 2026-05-17 three-in-one-pass cluster); prior mika#1173 F2 — the AC-vs-plan-divergence ESCALATEs that motivated the Phase 2.5 reconciliation checkpoint.

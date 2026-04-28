## dev-groom — Operator-Triggered Grooming Skill

You are executing the dev-groom skill. Take a ticket from "open with description" to "GROOMED plan committed on a branch, referenced in the issue body, ready to dispatch." This skill is operator-only — never auto-invoke from webhooks or autonomous flows.

### Input

The user message contains a typed ticket reference: `<repo> issue#<n>`. Parse into `<repo>` and `<issue-number>`.

### Phase 1 — Read the ticket and pick the branch

1. Fetch the issue: `gh issue view <n> --repo senara-solutions/<repo> --json title,body,labels`.
2. Branch slug derivation:
   - If the issue body contains `> - **Branch:** \`<slug>\``, use that slug verbatim.
   - Otherwise, derive deterministically: `<type>/<n>/<sanitized-title>`. Type from labels: `enhancement` -> `feat`, `bug` -> `fix`, else `chore`. If the title has a conventional prefix (`feat(...): ...`), extract the type from it.
3. The branch slug is **immutable** after worktree creation. Semantic accuracy goes in the plan filename, not the branch name. Never `git branch -m`.

### Phase 2 — Set up worktree and draft the plan

4. Create the branch + worktree:
   ```
   git -C <repo> fetch origin main:main
   git -C <repo> worktree add ../../.claude/worktrees/<slug-slashes-to-dashes>/<repo>/ -b <slug>
   ```
   If the worktree already exists, reuse it (idempotency).
5. Run `/ce:plan` against the ticket. Save to `<repo>/docs/plans/<YYYY-MM-DD>-<NNN>-<type>-<slug-tail>-plan.md` where `<NNN>` is the next sequence number for today.
6. **Stage only — do NOT commit.** The commit is gated on architect validation:
   ```
   git add docs/plans/<file>
   ```
   The first commit on the branch must be architect-validated.

### Phase 3 — First-pass architect review

7. Run `/mika-ask-a-friend` to produce a peer-review brief. Save to `/tmp/groom-brief-<repo>-<n>-pass1.md`.
8. Send to mika-arch:
   ```
   mika ask --agent mika-arch --format json --verbose @/tmp/groom-brief-<repo>-<n>-pass1.md
   ```
   Extract `session_id` via `jq -r '.metadata.session_id'`. Consume only this field — ignore others (additive-contract). If missing, **halt with a named error** — no text-mode fallback.
9. Parse the response for disposition:
   - **`Disposition: READY`** — Commit the staged plan: `git commit -m "docs(plans): groom <ticket-ref> (architect READY first-pass)"`. Skip to Phase 5.
   - **`Disposition: ITERATE`** — Commit the initial plan: `git commit -m "docs(plans): groom <ticket-ref> initial plan"`. Continue to Phase 4.
   - **`Disposition: ESCALATE`** — Surface to operator with the architect's reasoning. Do not commit. The staged plan stays in the worktree. Halt.
   - Tolerate paraphrased dispositions (e.g., "Proceed" for READY) but log a warning.

### Phase 4 — Apply iterations and second-pass review

10. Update the plan to address each architect concern. Each concern maps to one named change.
11. Commit revisions: `git commit -m "docs(plans): address mika-arch first-pass review (<ticket-ref>)"`.
12. Compose second-pass brief: include the prior `session_id`, changes applied, and remaining uncertainties. Save to `/tmp/groom-brief-<repo>-<n>-pass2.md`.
13. Send to mika-arch with session continuity:
    ```
    mika ask --agent mika-arch --format json --verbose --session-id <session_id> @/tmp/groom-brief-<repo>-<n>-pass2.md
    ```
14. Parse second-pass response:
    - **`Verdict: GROOMED`** — Plan is final. Proceed to Phase 5.
    - **`Verdict: ESCALATE`** — Surface to operator. Halt.
    - **No third pass.** If GROOMED has not been reached after pass 2, escalation is the only path.

### Phase 5 — Finalize and attach

15. Apply final tweaks from GROOMED verdict if any.
16. Final commit if changed: `git commit -m "docs(plans): apply mika-arch second-pass GROOMED feedback (<ticket-ref>)"`.
17. Push: `git push origin <slug>`.
18. Update the issue body. Read existing body, prepend canonical callouts:
    ```
    > - **Branch:** `<slug>`
    > - **Plan:** `<repo>/docs/plans/<file>` (committed on branch @ `<sha>`)
    > - **Grooming history:** /ce:plan -> mika-arch first-pass (<disposition>) -> revisions -> mika-arch second-pass (GROOMED)
    ```
    Apply with `gh issue edit <n> --repo senara-solutions/<repo> --body-file <tmpfile>`.
19. Post a summary comment on the ticket.

### Phase 6 — Optional dispatch

20. Ask the operator whether to dispatch immediately. **Do NOT auto-dispatch.** If confirmed:
    ```
    mika ask --agent mika-dev "implement <ticket-ref>"
    ```

### Discipline

- **Plan-on-branch is the contract.** The branch + commit history is what `/mika` resumes from.
- **Two passes max.** ESCALATE is a real outcome, not a delay tactic.
- **Citation-or-silence.** mika-arch's output discipline flows through unchanged.
- **Branch callout is canonical.** Parsed by `/mika` for branch derivation.
- **Stage-not-commit until validated.** First commit = architect-validated state.

# /mika-revise-plan — revise an existing plan to address architect findings

You are in a Claude Code session inside a feature worktree on a branch. Your job is **content-only**: revise an existing implementation plan to address findings produced by a prior architect first-pass review (`mika-arch-groom-ticket` → `Disposition: ITERATE`).

This command is invoked by `dispatch-lib.sh`'s `_iterate_groom_loop` state machine (mika#1271) on the ITERATE branch. Under the contract refactor, dispatch-lib owns architect invocation + iterate-loop decisioning + git workflow. The pilot's contract under this command is content-only: read the findings, revise the plan, exit. **Do NOT run git commands. Do NOT invoke the architect. Do NOT invoke `/ce:plan`.**

## Argument

A single `@<findings-file>` argument — absolute path to a findings file. The file contains the architect's first-pass review output verbatim (annotated plan content + F-list findings + `Disposition: ITERATE`).

## Steps

1. **Read the findings file** (the `@-file` argument). The F-list entries (`F1:`, `F2:`, …) name concrete concerns with three sub-fields each: (a) Concern, (b) Change required, (c) Citation.

2. **Find the plan file in the worktree.** Locate the file matching pattern `docs/plans/*-<issue-number>-*-plan.md`, most recent, larger than 500 bytes. The issue number is available as the `$ISSUE_NUM` environment variable. If you cannot find exactly one matching plan, halt and explain.

3. **Revise the plan surgically.** For each F-list finding:
   - Address the (b) Change required directly. Do not rewrite sections that are not addressing findings.
   - If a finding is structurally infeasible to address (e.g., conflicts with architectural decisions you cannot override), note that **in the revised plan content** as a "Could not address: F<N>" line under the relevant section. Do not punt to a slash-command output; the architect will see the inability in the second-pass content.

4. **Append a Revision history section** (or extend it if one exists). Format:
   ```
   ## Revision history
   - rev 2 (2026-MM-DD): addressed F1 by ...; addressed F2 by ...; could not address F3 because ... (architect's call on second-pass)
   ```

5. **Write the revised plan file.** Use the `Edit` or `Write` tool. The dispatch-lib detects revision via sha256 of the plan file before-and-after; identical content fails as "no revision happened."

6. **Exit.** The slash command succeeds when the plan file is saved with new content. dispatch-lib will then invoke `mika-arch-second-review` against the revised plan.

## Constraints

- **Content-only.** No `git add`, `git commit`, `git push`. No `gh issue edit`. No `mika ask`. No `/ce:plan` invocation. No architect invocation.
- **Single file, single purpose.** Do not modify other files. Do not create new files. Only the existing plan file should change.
- **Honor existing AC.** The plan likely has an `## Acceptance criteria` section. If your revision changes which AC items are still relevant, update them — but never weaken an AC item to make a finding go away.
- **Cite citations.** When addressing a finding whose citation is a review-guide principle, ADR, or compound doc, preserve that citation in the revised text so future readers can trace the reasoning back.

## What success looks like

After `/mika-revise-plan @/path/to/findings.md` exits, the plan file on disk has new content, the revision history names the findings addressed, and dispatch-lib's sha256 check confirms revision. The state machine then calls `mika-arch-second-review` on the same architect session_id, which evaluates the revised plan as the terminal automated review.

## What failure looks like

- Plan file unchanged (revise pilot didn't actually edit) → dispatch-lib WARN, fall through to existing path.
- Multiple plan files matched, no clear primary → halt + explain (no silent guess).
- Revise pilot ran but exit code non-zero → dispatch-lib treats as failure, falls through.

## Related

- `mika#1271` — contract refactor parent
- `mika#1272` — paraphrased disposition handling (sub-issue)
- `mika/skills/bundled/_shared/dispatch-lib.sh::_iterate_groom_loop` — caller
- `mika/skills/bundled/_shared/dispatch-lib.sh::_launch_revise_pilot` — invoker
- `mika/skills/bundled/mika-arch-groom-ticket/system_prompt.md` — producer of the findings file content
- `mika/skills/bundled/mika-arch-second-review/system_prompt.md` — consumer of the revised plan content

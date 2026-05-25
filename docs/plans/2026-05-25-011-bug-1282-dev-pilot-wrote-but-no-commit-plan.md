# Plan: dev-pilot wrote-but-no-commit recovery (mika#1282)

## Problem

Live-exercise on mika#1267 (2026-05-25) showed the dev-pilot making correct file edits via the `/mika` pipeline but never invoking `git add`/`git commit`/`git push`. Result: correct work in the worktree, HEAD unchanged, no PR. The existing zero-commit detection (dispatch-lib.sh line 408) fires and emits `PIPELINE FAILURE`, but the correct content is lost — the operator must manually rescue it.

This is the **dev-pilot analog** of the dev-groom wrote-but-no-commit class that mika#1271 closed via the content/workflow split pattern (documented in `docs/solutions/architecture-patterns/pilot-vs-substrate-contract-split-2026-05-25.md`).

## Approach: auto-commit-and-push fallback in dispatch-lib

The dev-groom content/workflow split (mika#1271 sub-PR 8) created a content-only slash command (`/mika-groom-plan-only`) and moved workflow to dispatch-lib. The dev-pilot case is structurally different: the pilot's `/mika` pipeline produces code changes across arbitrary files (not a single plan file in a predictable path), runs `/ce:review` to generate TODOs, resolves them, and creates a PR. Splitting this into a content-only command would require duplicating most of `/mika`'s pipeline minus the git/PR steps — high churn, low confidence.

Instead, this plan takes **option 2** from the ticket (detect-and-recover with auto-commit) but scoped narrowly:

1. **Detection:** dispatch-lib already detects zero-commit (line 408). Extend it to also detect **dirty worktree** (unstaged/staged changes exist but HEAD unchanged).
2. **Recovery:** When dirty-worktree-zero-commit is detected for `dev-pilot`, auto-commit the changes with a `wip()` prefix, push, and open a PR marked as draft.
3. **Fail-loud:** The PIPELINE FAILURE marker remains. The auto-commit is a recovery action, not a success — the callback to mika-dev still reports PIPELINE_INCOMPLETE with a note that content was rescued.

### Why auto-commit (not content-only split) for dev-pilot

The pattern doc warns against auto-commit as an anti-pattern. That analysis is correct for dev-groom where the substrate can own the full workflow (architect is a deterministic API call). For dev-pilot, the workflow steps ARE LLM-shape work:

- `/ce:review` requires reading the diff and reasoning about issues.
- `/compound-engineering:resolve_todo_parallel` requires reading TODO findings and deciding fixes.
- `gh pr create --body "..."` requires summarizing the changes.

Moving these to dispatch-lib would mean either (a) a second claude-pilot session for "git and PR only" (cost regression) or (b) deterministic git add + commit + PR with a generic body (what this plan proposes, but honestly labeled as recovery, not as the primary path).

The auto-commit fallback is justified here because:
- It preserves work that would otherwise be lost.
- It's clearly labeled as recovery (wip prefix, draft PR, PIPELINE_INCOMPLETE outcome).
- It doesn't replace the primary contract — the pilot is still expected to commit and PR.
- It's a single layer, not a recursive detection stack.

## Implementation units

### Unit 1: Dirty-worktree detection in `_run_claude_pilot` post-flight

**File:** `skills/bundled/_shared/dispatch-lib.sh`
**Location:** After the zero-commit detection block (line 408-413), inside the `if [ -n "$PRE_RUN_HEAD" ] && [ -n "$REPO" ]` guard.

Add a dirty-worktree check when HEAD is unchanged:

```bash
# After existing zero-commit block (line 413):
# Unit 1 (mika#1282): detect dirty worktree on zero-commit dev-pilot.
# If the pilot wrote files but never committed, auto-rescue the content
# so it isn't lost with the worktree.
if [ "$PRE_RUN_HEAD" = "$POST_RUN_HEAD" ] && [ "$SKILL" = "dev-pilot" ] && [ -n "$WORKTREE_DIR" ]; then
    DIRTY_FILES=$(git -C "$WORKTREE_DIR" status --porcelain 2>/dev/null | head -20)
    if [ -n "$DIRTY_FILES" ]; then
        # Auto-commit rescue: stage all changes, commit with wip prefix
        git -C "$WORKTREE_DIR" add -A 2>&9
        git -C "$WORKTREE_DIR" commit -m "wip(${REPO}#${ISSUE_NUM}): impl staged by post-flight recovery (mika#1282)

Content written by pilot session ${SESSION_ID:-unknown} but git commit was never invoked.
Auto-rescued by dispatch-lib dirty-worktree detection." 2>&9

        # Update POST_RUN_HEAD so _push_branch sees new commits
        POST_RUN_HEAD=$(git -C "$WORKTREE_DIR" rev-parse HEAD 2>/dev/null || true)

        # Amend the PIPELINE FAILURE message (already set above) with rescue note
        RESULT="PIPELINE FAILURE: claude-pilot exited 0 but HEAD unchanged — dirty worktree detected and auto-committed (mika#1282 recovery).
Files rescued:
${DIRTY_FILES}

${RESULT}"

        # Mark for draft PR creation in Unit 2
        RESCUED_DIRTY_WORKTREE=1
    fi
fi
```

**Key decisions:**
- `git add -A` is acceptable here because the worktree is isolated (created by dispatch-lib, not the user's main checkout). No risk of staging secrets or unrelated files.
- `head -20` on status output caps the log for readability.
- `POST_RUN_HEAD` is updated so `_push_branch` sees the rescue commit and pushes it.
- The `PIPELINE FAILURE` prefix is preserved — this is still a failure, just one with rescued content.

### Unit 2: Draft PR creation on rescued worktree

**File:** `skills/bundled/_shared/dispatch-lib.sh`
**Location:** After the existing PR-existence check block (line 484-488), add a new block for draft PR creation on rescued content.

```bash
# Unit 2 (mika#1282): open a draft PR when content was auto-rescued.
# Draft status signals "pilot failed; content needs human review."
if [ "${RESCUED_DIRTY_WORKTREE:-}" = "1" ] && [ -n "$REPO" ] && [ -n "$BRANCH" ] && [ -z "$PR_URL" ]; then
    # Push first (before _push_branch, which is idempotent)
    git -C "$WORKTREE_DIR" push -u origin "$BRANCH" 2>&9 || true

    # Create draft PR
    RESCUED_PR_URL=$(gh pr create \
        --repo "senara-solutions/$REPO" \
        --head "$BRANCH" \
        --base main \
        --draft \
        --title "wip(${REPO}#${ISSUE_NUM}): auto-rescued impl (mika#1282 recovery)" \
        --body "$(cat <<RESCUEBODY
## Auto-rescued implementation

This PR was created by dispatch-lib's dirty-worktree recovery (mika#1282).

The dev-pilot session wrote correct file changes but never ran \`git commit\` or \`gh pr create\`. dispatch-lib detected the dirty worktree, auto-committed, and opened this draft PR to preserve the work.

**This is a draft PR.** The changes need human review before marking ready.

### Recovery metadata
- Pilot session: \`${SESSION_ID:-unknown}\`
- Turns: ${TURNS:-unknown}
- Cost: \$${COST:-unknown}

Closes #${ISSUE_NUM}
RESCUEBODY
)" 2>&9 || true)

    if [ -n "$RESCUED_PR_URL" ]; then
        PR_URL="$RESCUED_PR_URL"
        RESULT="${RESULT}
Draft PR (auto-rescued): ${PR_URL}"
    fi
fi
```

**Key decisions:**
- Draft PR, not ready — signals the pilot failed and this is recovered content.
- The push happens here (before `_push_branch`) because `_push_branch` needs the remote branch to exist for its ahead-check logic. `_push_branch` is idempotent and will no-op if already pushed.
- `|| true` on both push and PR create — recovery is best-effort; if it fails, the PIPELINE FAILURE marker still surfaces the gap.

### Unit 3: Initialize `RESCUED_DIRTY_WORKTREE` flag

**File:** `skills/bundled/_shared/dispatch-lib.sh`
**Location:** Near the top of `_run_claude_pilot()`, alongside existing variable initialization.

```bash
RESCUED_DIRTY_WORKTREE=0
```

### Unit 4: Outcome classification update

**File:** `skills/bundled/_shared/dispatch-lib.sh`
**Location:** The outcome classification block (lines 494-515).

The existing logic already handles this correctly:
- `PIPELINE FAILURE` prefix → `PIPELINE_INCOMPLETE` (line 494-497) — this fires because the rescue preserves the PIPELINE FAILURE prefix.
- The draft PR URL will be in `$PR_URL` so the "PR_OPENED" branch would fire if PIPELINE FAILURE wasn't set. But PIPELINE FAILURE takes precedence (checked first), so outcome is correctly `PIPELINE_INCOMPLETE`.

No changes needed — the existing classification handles rescued content correctly.

### Unit 5: Update CLAUDE.md post-flight signal documentation

**File:** `CLAUDE.md` (root)
**Location:** After the existing `_post_flight_push` documentation in the Architecture Summary or Skills System section.

Add a brief note about the dirty-worktree recovery in the dispatch-lib post-flight checks section:

```
- **Post-flight dirty-worktree recovery (#1282):** When dev-pilot exits with dirty worktree but zero commits,
  dispatch-lib auto-commits with `wip()` prefix, pushes, and opens a draft PR. Content is rescued;
  outcome remains PIPELINE_INCOMPLETE. Operator must review and promote the draft PR.
```

## Files changed

| File | Change |
|------|--------|
| `skills/bundled/_shared/dispatch-lib.sh` | Units 1-4: dirty-worktree detection, auto-commit, draft PR, flag init |
| `CLAUDE.md` | Unit 5: document the new post-flight recovery signal |

## Testing

1. **Simulated dirty worktree:** Create a worktree, make edits without committing, invoke the post-flight block with `PRE_RUN_HEAD = POST_RUN_HEAD` and `SKILL = dev-pilot`. Verify auto-commit, push, and draft PR creation.
2. **Clean worktree (no regression):** Verify that when HEAD changes (normal success), the dirty-worktree block is skipped entirely.
3. **Dev-groom guard:** Verify the `SKILL = dev-pilot` guard prevents the block from firing on dev-groom dispatches (dev-groom has its own iterate-loop recovery via mika#1271).
4. **Live exercise:** Deploy and run a real dev-pilot dispatch. If the pilot commits normally, the recovery is a no-op. If it doesn't, the rescue fires.

## Risks and mitigations

| Risk | Mitigation |
|------|------------|
| Auto-commit includes test artifacts or build output | Worktrees are clean checkouts; only pilot-generated changes exist. `.gitignore` still applies to `git add -A`. |
| Draft PR confuses mika-dev's acceptance testing | Outcome is PIPELINE_INCOMPLETE, not PR_OPENED — mika-dev's acceptance path requires PR_OPENED to proceed. |
| `git add -A` in worktree picks up unexpected files | Worktrees are created from `main` by dispatch-lib; no user files exist. Risk is negligible. |
| Recovery layer stacking (the anti-pattern) | This is a single layer on top of the existing zero-commit detection. It rescues content; it doesn't add a second recovery for a recovery. If this layer itself fails, the PIPELINE FAILURE marker still fires from the zero-commit block. |

## Sequence

Single PR — all units are small and interdependent (Unit 2 depends on Unit 1's flag, Unit 4 validates Unit 1's outcome shape).

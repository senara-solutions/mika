# Plan: dev-pilot wrote-but-no-commit recovery (mika#1282)

## Problem

Live-exercise on mika#1267 (2026-05-25) showed the dev-pilot making correct file edits via the `/mika` pipeline but never invoking `git add`/`git commit`/`git push`. Result: correct work in the worktree, HEAD unchanged, no PR. The existing zero-commit detection (dispatch-lib.sh line 408) fires and emits `PIPELINE FAILURE`, but the correct content is lost — the operator must manually rescue it.

This is the **dev-pilot analog** of the dev-groom wrote-but-no-commit class that mika#1271 closed via the content/workflow split pattern (documented in `docs/solutions/architecture-patterns/pilot-vs-substrate-contract-split-2026-05-25.md`).

## Approach: content/workflow split — dispatch-lib owns git recovery workflow

This plan implements **option 1** from the ticket: the content/workflow split per the mika#1271 architect verdict (session `0583a902`, documented in `docs/solutions/architecture-patterns/pilot-vs-substrate-contract-split-2026-05-25.md`).

**Contracts:**
- **Pilot's primary contract (unchanged):** Write code, run `/ce:review`, resolve TODOs, commit, push, open PR. This is the success path — pilot owns the full pipeline including git workflow on success.
- **Dispatch-lib's recovery contract (new):** When the pilot writes content but fails to execute the git workflow (dirty worktree, zero commits), dispatch-lib owns the git recovery: stage, commit, push, open draft PR. This is structurally identical to how the dev-groom content/workflow split works — dispatch-lib owns the git workflow when the pilot produces content but doesn't drive git to completion.

**Why this is the content/workflow split (not the option 2 anti-pattern):**

The anti-pattern (option 2) would be: pilot owns git as primary, dispatch-lib adds a second recovery layer on top. That creates ambiguous ownership — who is responsible for git?

This plan's structure is different:
1. Pilot's primary contract includes git (the LLM-shape work: review, TODOs, PR body authorship). The pilot is expected to drive git to completion.
2. When the pilot fails to deliver on the git portion of its contract, dispatch-lib takes ownership of the git workflow — not as a "fallback" but as the substrate exercising its structural responsibility for the git layer (per mika#1271 § "dispatch-lib owns git workflow").
3. The recovery produces a template-body draft PR (not LLM-authored), clearly labeled as substrate-owned recovery. This is the same pattern as dev-groom: pilot writes content, dispatch-lib drives git.

The key distinction from the anti-pattern: dispatch-lib is not "catching" a pilot failure with a second attempt at the same thing. It is exercising a structurally different contract (deterministic git workflow with template body) that produces a different artifact (draft PR, `wip()` prefix, `PIPELINE_INCOMPLETE` outcome). The pilot's LLM-shape work (review, TODOs, PR body quality) is acknowledged as lost — only the raw content is preserved.

Citation: mika#1271 architect verdict session `0583a902` (`flip`); `docs/solutions/architecture-patterns/pilot-vs-substrate-contract-split-2026-05-25.md`; `docs/architecture/review-guide.md` § Single Responsibility / Separation of Concerns.

**Implementation shape:**

1. **Detection:** dispatch-lib already detects zero-commit (line 408). Extend it to also detect **dirty worktree** (unstaged/staged changes exist but HEAD unchanged).
2. **Recovery (dispatch-lib owns git workflow):** When dirty-worktree-zero-commit is detected for `dev-pilot`, dispatch-lib stages, commits with `wip()` prefix, and delegates to `_push_branch` (which handles first-push natively — see line 558-564). Then opens a draft PR with a template body.
3. **Fail-loud:** The PIPELINE FAILURE marker remains. The recovery is dispatch-lib exercising its structural git-ownership contract, not a success — the callback to mika-dev still reports PIPELINE_INCOMPLETE with a note that content was rescued.

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
- `git add -A` stages all non-gitignored worktree content unconditionally. For doc-only or single-file fixes this is low-risk. For implementation dispatches that partially edit multiple files before failing, `git add -A` produces a `wip()` commit with potentially incoherent multi-file changes. **Mitigation:** The draft PR status is the explicit safety net — human review is mandatory before promoting the draft. The `wip()` prefix and `PIPELINE_INCOMPLETE` outcome both signal that this content has not passed `/ce:review` and may be partially coherent. Session-scoped file tracking does not currently exist in dispatch-lib, so `git add -A` is the simplest correct behavior for the recovery path (review-guide.md § KISS). Future enhancement: if claude-pilot gains a "files touched" manifest, scope staging to that list.
- `head -20` on status output caps the log for readability.
- `POST_RUN_HEAD` is updated so `_push_branch` sees the rescue commit and pushes it.
- The `PIPELINE FAILURE` prefix is preserved — this is dispatch-lib exercising its recovery contract, not a success.
- **SKILL guard correctness (F5):** `SKILL` is set from the tool call JSON input at `dispatch-lib.sh` line 113 (`jq -r '.skill // empty'`). The caller (self-dev/dev-pilot `handler.sh` or self-dev/dev-groom `handler.sh`) passes `"skill": "dev-pilot"` or `"skill": "dev-groom"` respectively in its JSON payload. The `_iterate_groom_loop` (mika#1271) does not re-enter `_run_claude_pilot` — it runs `_launch_revise_pilot` and `_invoke_architect` which are separate functions. Therefore the `SKILL = dev-pilot` guard exclusively gates this recovery to implementation dispatches, never grooming dispatches. If a future iterate-loop re-entry were added that passes through `_run_claude_pilot`, the guard would still protect because the iterate loop is groom-only (`SKILL = dev-groom`).

### Unit 2: Draft PR creation on rescued worktree

**File:** `skills/bundled/_shared/dispatch-lib.sh`
**Location:** After the existing PR-existence check block (line 484-488), add a new block for draft PR creation on rescued content. This block runs **after** `_push_branch` in the call sequence (see `_run_dispatch` at line 1207: `_push_branch` then `_deliver_callback`). The draft PR creation is inserted between push and callback delivery.

```bash
# Unit 2 (mika#1282): open a draft PR when content was rescued by dispatch-lib's
# git-workflow ownership (content/workflow split per mika#1271 architect verdict).
# Draft status signals "pilot failed to drive git; substrate owns recovery workflow."
if [ "${RESCUED_DIRTY_WORKTREE:-}" = "1" ] && [ -n "$REPO" ] && [ -n "$BRANCH" ] && [ -z "$PR_URL" ]; then
    # No pre-push needed here. _push_branch (lines 544-582) handles first-push
    # natively: when origin/$BRANCH doesn't exist (line 558 rev-parse fails),
    # it falls through to the always-push path (line 564) and pushes with -u
    # (line 571). The rescue commit from Unit 1 updated POST_RUN_HEAD, so
    # _push_branch sees local-ahead commits and pushes them.

    # Create draft PR with template body (dispatch-lib owns git workflow;
    # no LLM-authored summary — that's acknowledged as lost)
    RESCUED_PR_URL=$(gh pr create \
        --repo "senara-solutions/$REPO" \
        --head "$BRANCH" \
        --base main \
        --draft \
        --title "wip(${REPO}#${ISSUE_NUM}): rescued impl (dispatch-lib recovery)" \
        --body "$(cat <<RESCUEBODY
## Rescued implementation (dispatch-lib content/workflow split)

This PR was created by dispatch-lib's git-workflow recovery (mika#1282).

The dev-pilot session wrote file changes but never completed the git workflow
(no \`git commit\` or \`gh pr create\`). Per the mika#1271 content/workflow split
contract, dispatch-lib took ownership of the git layer: staged, committed with
\`wip()\` prefix, pushed, and opened this draft PR to preserve the content.

**This is a draft PR requiring human review.** The content has NOT passed
\`/ce:review\` and may contain partially-coherent multi-file changes.

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
Draft PR (dispatch-lib recovery): ${PR_URL}"
    fi
fi
```

**Key decisions:**
- Draft PR, not ready — signals the pilot failed to drive git and dispatch-lib is exercising its structural git-workflow ownership.
- **No pre-push in this block** (addressing F3, review-guide.md § DRY). `_push_branch` (lines 544-582) handles first-push natively: line 558 checks `rev-parse --verify "origin/$BRANCH"`; when that fails (first-push case), line 564 falls through to always-push with `-u` (line 571). Unit 1 updates `POST_RUN_HEAD` so `_push_branch` sees the rescue commit as local-ahead. The previous plan's pre-push was redundant and would have created a double-push. Citation: `dispatch-lib.sh` lines 558-564 (first-push fallthrough), line 571 (`push -u`).
- `|| true` on PR create — recovery is best-effort; if it fails, the PIPELINE FAILURE marker still surfaces the gap.
- Template body (not LLM-authored) — this is the content/workflow split's explicit tradeoff: PR body quality is lost when the pilot fails to drive git. The draft PR serves as a content rescue, not a finished artifact.

### Unit 3: Initialize `RESCUED_DIRTY_WORKTREE` flag

**File:** `skills/bundled/_shared/dispatch-lib.sh`
**Location:** Near the top of `_run_claude_pilot()`, alongside existing variable initialization.

```bash
RESCUED_DIRTY_WORKTREE=0
```

### Unit 4: Outcome classification update — verified, no changes needed

**File:** `skills/bundled/_shared/dispatch-lib.sh`
**Location:** The outcome classification block (lines 492-501).

**Verification (addressing F4, review-guide.md § citation-or-silence; Phase 0 Pin):**

The classification block at lines 492-501 uses ordered `if`/`elif` precedence:
- **Line 494:** `if echo "$RESULT" | grep -qF "PIPELINE FAILURE:"; then` — checks for the PIPELINE FAILURE prefix FIRST.
- **Line 497:** Appends `Outcome: PIPELINE_INCOMPLETE — manual recovery needed.`
- **Line 498:** `elif [ -n "$PR_URL" ]; then` — checks PR_URL SECOND (as `elif`, only reached if PIPELINE FAILURE was not matched).
- **Line 501:** Appends `Outcome: PR_OPENED — ${PR_URL}`

Because Unit 1 preserves the `PIPELINE FAILURE:` prefix in RESULT (the rescue prepends it), and because line 494's grep fires before line 498's `PR_URL` check, the outcome is correctly classified as `PIPELINE_INCOMPLETE` even when `$PR_URL` is set (from the Unit 2 draft PR). The `elif` structure guarantees mutual exclusion.

No changes needed — the existing classification handles rescued content correctly. Citation: `dispatch-lib.sh` lines 494 (`grep -qF "PIPELINE FAILURE:"`), 498 (`elif [ -n "$PR_URL" ]`).

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
| `git add -A` stages partial/incoherent multi-file changes from failed impl dispatches | Draft PR status is the explicit safety net — human review mandatory before promoting. `wip()` prefix + `PIPELINE_INCOMPLETE` outcome both signal unreviewed content. This is the acknowledged tradeoff of the content/workflow split when the pilot fails mid-pipeline (review-guide.md § KISS — simplest correct recovery). |
| Build artifacts or test output staged | `.gitignore` still applies to `git add -A`. Worktrees are clean checkouts from `main`; only pilot-generated changes exist. |
| Draft PR confuses mika-dev's acceptance testing | Outcome is `PIPELINE_INCOMPLETE`, not `PR_OPENED` — classification precedence verified at line 494 vs 498. mika-dev's acceptance path requires `PR_OPENED` to proceed. |
| `SKILL` guard fires on wrong dispatch type | `SKILL` is set from tool-call JSON input (line 113). Dispatch callers (`dev-pilot/handler.sh`, `dev-groom/handler.sh`) set it explicitly. `_iterate_groom_loop` does not re-enter `_run_claude_pilot`. Guard is sound for all current call sites. |
| Structural resemblance to the option 2 anti-pattern | This is option 1 (content/workflow split): dispatch-lib owns the git workflow on recovery, producing a structurally different artifact (draft PR, template body, `wip()` prefix). It does not retry the pilot's LLM-shape work. The pilot's primary contract is unchanged. Citation: `pilot-vs-substrate-contract-split-2026-05-25.md`; mika#1271 architect verdict. |

## Sequence

Single PR — all units are small and interdependent (Unit 2 depends on Unit 1's flag, Unit 4 validates Unit 1's outcome shape).

## Revision history

- rev 2 (2026-05-26): addressed F1 by reframing from "option 2 auto-commit fallback" to "option 1 content/workflow split" — dispatch-lib owns git workflow on recovery per mika#1271 architect verdict, producing a structurally different artifact (draft PR with template body), not retrying the pilot's LLM-shape work; addressed F2 by explicitly acknowledging `git add -A` stages unconditionally for partial impl dispatches and citing draft PR mandatory human review as the mitigation (review-guide.md § KISS); addressed F3 by removing redundant pre-push from Unit 2 — `_push_branch` handles first-push natively (lines 558-564 fallthrough, line 571 push -u), cited line references (review-guide.md § DRY); addressed F4 by citing outcome classification block line references (line 494 `grep -qF "PIPELINE FAILURE:"` checked before line 498 `elif [ -n "$PR_URL" ]`) confirming precedence (review-guide.md § citation-or-silence); addressed F5 by verifying `SKILL` is set from tool-call JSON at line 113, confirming dispatch callers set it explicitly, and confirming `_iterate_groom_loop` does not re-enter `_run_claude_pilot` (review-guide.md § Orthogonality).

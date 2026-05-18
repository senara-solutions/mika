# Plan: Fix post-flight recovery fabricated SHA (Class D drift)

**Issue:** mika#1204
**Type:** bug
**Branch:** `bug/1204/dispatch-lib-post-flight-recovery`

## Problem

`_verify_and_write_body_callout()` in `skills/bundled/_shared/dispatch-lib.sh` (lines 209–222) stamps the issue body callout with `git rev-parse --short HEAD` unconditionally. When the pilot exits early — plan file staged via `git add` but never committed — HEAD points to the main-branch tip at worktree-creation time, not a commit containing the plan. The callout reads `(committed on branch @ \`f393fb36\`)` but `git show f393fb36:<plan-path>` fails with `fatal: path exists on disk, but not in 'f393fb36'`.

This is **Class D drift** — plan-found-but-uncommitted → fabricated commit SHA.

## Root Cause

Line 211: `head_sha=$(git -C "$worktree_dir" rev-parse --short HEAD 2>/dev/null)` captures HEAD regardless of whether the plan file is tracked in that commit's tree. No verification step exists between plan-file detection (line 201) and SHA stamping (line 222).

## Solution

**Approach (a) from the issue:** commit the plan before stamping the SHA. This preserves the existing dispatch-gate shape (callout matches `"Plan: docs/plans/...committed @ SHA"` pattern) and makes the callout truthful.

### Steps

#### Step 1: Verify plan is committed before SHA stamping (dispatch-lib.sh lines 209–225)

After detecting the plan file (line 202) and before constructing the callout (line 220), add a verification + auto-commit sequence:

1. Check if the plan file is in the current HEAD tree: `git -C "$worktree_dir" cat-file -e HEAD:"$plan_relpath" 2>/dev/null`
2. If exit code is non-zero (plan NOT in HEAD), the plan is on disk but uncommitted. Auto-commit it:
   ```bash
   git -C "$worktree_dir" add "$plan_relpath"
   git -C "$worktree_dir" commit -m "wip(${repo}#${issue_num}): plan staged by post-flight recovery"
   ```
3. Re-capture HEAD SHA after the commit: `head_sha=$(git -C "$worktree_dir" rev-parse --short HEAD 2>/dev/null)`
4. If commit fails (e.g., nothing to commit — shouldn't happen given the cat-file guard, but defense-in-depth), fall through to approach (b): stamp as `(uncommitted on branch \`${branch}\`, see worktree)` instead of fabricating a SHA.

#### Step 2: Push the recovery commit

After the auto-commit, push the branch so the SHA is reachable from origin. The existing push logic in the post-flight path (if any) should cover this, but if the recovery callout is written before the push step, add an explicit push:
```bash
git -C "$worktree_dir" push origin "$branch" 2>/dev/null || true
```

The `|| true` is intentional — push failure is non-fatal (plan is committed locally, not lost). The callout already warns `operator dispatch required`.

### Concrete diff

**File:** `skills/bundled/_shared/dispatch-lib.sh`

**Replace lines 209–211** (current):
```bash
    local plan_relpath="${plan_file#"$worktree_dir/"}"
    local head_sha
    head_sha=$(git -C "$worktree_dir" rev-parse --short HEAD 2>/dev/null)
```

**With:**
```bash
    local plan_relpath="${plan_file#"$worktree_dir/"}"

    # Class D drift fix (mika#1204): verify the plan file is actually in HEAD's
    # tree before stamping a SHA claim. If not (pilot exited before committing),
    # auto-commit the plan so the SHA is truthful.
    if ! git -C "$worktree_dir" cat-file -e "HEAD:${plan_relpath}" 2>/dev/null; then
        echo "post_flight_class_d_recovery: plan file exists on disk but not in HEAD — committing (mika#1204)" >&2
        git -C "$worktree_dir" add -- "$plan_relpath"
        if ! git -C "$worktree_dir" commit -m "wip(${repo}#${issue_num}): plan staged by post-flight recovery"; then
            echo "WARN: post_flight_class_d_commit_failed for $repo#$issue_num — stamping as uncommitted" >&2
            # Fall through to approach (b): stamp as uncommitted
            local head_sha="uncommitted"
            local callout_block
            callout_block=$(cat <<CALLOUT_EOF
> - **Branch:** \`${branch}\`
> - **Plan:** \`${plan_relpath}\` (uncommitted on branch \`${branch}\`, see worktree)
> - **Grooming history:** body callout recovered by post-flight (mika#1123) — architect verdict not verified, operator dispatch required
CALLOUT_EOF
            )
            # Jump to write step (skip normal callout construction)
            local new_body
            new_body=$(printf '%s\n\n%s' "$callout_block" "$current_body")
            local tmpfile
            tmpfile=$(mktemp /tmp/body-callout-recover-XXXXXX.md)
            printf '%s' "$new_body" > "$tmpfile"
            if gh issue edit "$issue_num" --repo "senara-solutions/$repo" \
                --body-file "$tmpfile" 2>/dev/null; then
                echo "body_callout_drift_recovered: wrote UNCOMMITTED callout to $repo#$issue_num (plan on disk, commit failed)" >&2
            else
                echo "WARN: body_callout_drift_recovery_failed for $repo#$issue_num" >&2
            fi
            rm -f "$tmpfile"
            return 0
        fi
        # Push the recovery commit so SHA is reachable from origin
        git -C "$worktree_dir" push origin "$branch" 2>/dev/null || true
    fi

    local head_sha
    head_sha=$(git -C "$worktree_dir" rev-parse --short HEAD 2>/dev/null)
```

**Lines 213–241 remain unchanged** — the normal callout construction and write logic.

## Acceptance Criteria

- **AC1:** When `_verify_and_write_body_callout()` finds a plan file on disk that is NOT in HEAD's tree, it commits the file before stamping the SHA. The resulting callout's SHA is verifiable: `git show <sha>:<plan-path>` succeeds.
- **AC2:** When the auto-commit fails (edge case), the callout uses `(uncommitted on branch ...)` instead of a fabricated SHA.
- **AC3:** When the plan file IS already in HEAD's tree (normal case, no Class D drift), behavior is unchanged — existing tests and organic flow unaffected.
- **AC4:** Recovery commit message follows the `wip(repo#N)` convention, clearly marking it as post-flight recovery rather than organic pipeline output.

## Out of Scope

- Why the pipeline exits early (tracked by mika#804, mika#716)
- Body-callout drift Classes A/B/C (already fixed by mika#1123 + mika#1144)
- Dispatch-gate changes — the recovery callout still does NOT pass the dispatch gate (no `(GROOMED)` verdict); this fix only makes the SHA truthful

## Risk Assessment

- **Low risk.** The change is additive — a new guard before the existing SHA capture. The normal path (plan already committed) hits the `cat-file -e` check, gets exit 0, and skips the new block entirely.
- **Failure mode:** If `git commit` fails unexpectedly, the fallback stamps `(uncommitted ...)` — degraded but truthful, strictly better than the current fabricated SHA.
- **No downstream parser impact:** The `(uncommitted on branch ...)` fallback text does NOT match the `committed on branch @` pattern, so downstream parsers that regex-extract the SHA will not find a match — they'll treat it as a missing SHA, which is the correct interpretation.

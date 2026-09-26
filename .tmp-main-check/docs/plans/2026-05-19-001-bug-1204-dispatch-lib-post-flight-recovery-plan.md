# Plan: Fix post-flight recovery fabricated SHA (Class D drift)

**Issue:** mika#1204
**Type:** bug
**Branch:** `bug/1204/dispatch-lib-post-flight-recovery`
**Base SHA:** `f393fb36` (includes mika#1144 Class C fix at `964770a8`)

## Phase 0 — Pinned source

### Pin A: SHA-stamping block (dispatch-lib.sh:209–225)

```bash
    local plan_relpath="${plan_file#"$worktree_dir/"}"
    local head_sha
    head_sha=$(git -C "$worktree_dir" rev-parse --short HEAD 2>/dev/null)

    # 4. Construct recovery callout.
    #    IMPORTANT: The grooming-history line does NOT contain "second-pass (GROOMED)"
    #    because we cannot verify the architect actually issued that verdict from
    #    branch state alone. This callout surfaces the drift for operator visibility
    #    but does NOT pass the dispatch gate — the operator must verify and dispatch
    #    manually (or re-run dev-groom which will write the organic callout).
    local callout_block
    callout_block=$(cat <<CALLOUT_EOF
> - **Branch:** \`${branch}\`
> - **Plan:** \`${plan_relpath}\` (committed on branch @ \`${head_sha}\`)
> - **Grooming history:** body callout recovered by post-flight (mika#1123) — architect verdict not verified, operator dispatch required
CALLOUT_EOF
    )
```

**Key observations:**
- `head_sha` is captured into a local variable at line 211 — a single capture point, used once at line 222.
- The callout format string at line 222 embeds `${head_sha}` inline.
- No verification that `HEAD` contains `plan_relpath` in its tree.

### Pin B: Callout-write block (dispatch-lib.sh:227–241)

```bash
    # 5. Prepend callout to existing body and write
    local new_body
    new_body=$(printf '%s\n\n%s' "$callout_block" "$current_body")
    local tmpfile
    tmpfile=$(mktemp /tmp/body-callout-recover-XXXXXX.md)
    printf '%s' "$new_body" > "$tmpfile"

    if gh issue edit "$issue_num" --repo "senara-solutions/$repo" \
        --body-file "$tmpfile" 2>/dev/null; then
        echo "body_callout_drift_recovered: wrote missing callout to $repo#$issue_num (verdict NOT fabricated — operator dispatch required)" >&2
    else
        echo "WARN: body_callout_drift_recovery_failed for $repo#$issue_num" >&2
    fi
    rm -f "$tmpfile"
```

**Key observations:**
- Uses `--body-file` (full body replace via temp file), not `--body` or sed/awk.
- `current_body` was captured at line 180–181 via `gh issue view`.

### Pin C: Invocation site (dispatch-lib.sh:539–545)

```bash
        if [ "$SKILL" = "dev-groom" ] && [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ] && [ -n "$REPO" ] && [ -n "$ISSUE_NUM" ]; then
            _verify_and_write_body_callout "$REPO" "$ISSUE_NUM" "$WORKTREE_DIR" "$BRANCH"
        fi
```

**Key observation:** Only invoked for `dev-groom` skill. `WORKTREE_DIR` is always a per-dispatch isolated worktree — concurrent dispatches cannot race on the same worktree directory.

### Pin D: Downstream parsers

1. **`check_grooming_markers()` (executor.rs:800–814):** Checks for `docs/plans/` substring in the issue body. Does NOT parse the SHA or the `committed on branch @` text. The fallback `(uncommitted on branch ...)` format still contains `docs/plans/` so the plan-callout check passes. The verdict check (line 808) is independent.

2. **`_detect_plan_on_branch()` (dispatch-lib.sh:650–684):** Extracts plan path via `grep -oP '> - \*\*Plan:\*\* \`\Kdocs/plans/[^`]+'`. Captures everything between the backtick delimiters. Both `(committed on branch @ <sha>)` and `(uncommitted on branch <branch>, see worktree)` are outside the backtick-delimited path — the regex is unaffected.

3. **No other downstream parsers exist.** Grepping for `committed on branch @` across the codebase shows only documentation, test fixtures, and the source in dispatch-lib.sh itself.

## Cross-ticket sequencing

**mika#1144 (Class C drift) is already merged** into main at commit `964770a8` (PR #1157, merged 2026-05-16). The base SHA for this branch (`f393fb36`) includes #1144's changes. The line numbers in this plan are verified against the post-#1144 function shape. No ordering constraint or rebase concern.

## Problem

`_verify_and_write_body_callout()` in `skills/bundled/_shared/dispatch-lib.sh` (Pin A, line 211) stamps the issue body callout with `git rev-parse --short HEAD` unconditionally. When the pilot exits early — plan file staged via `git add` but never committed — HEAD points to the main-branch tip at worktree-creation time, not a commit containing the plan. The callout reads `(committed on branch @ f393fb36)` but `git show f393fb36:<plan-path>` fails with `fatal: path exists on disk, but not in 'f393fb36'`.

This is **Class D drift** — plan-found-but-uncommitted → fabricated commit SHA.

## Root Cause

Pin A line 211: `head_sha` is captured once into a local variable, unconditionally — no verification that `HEAD` contains `plan_relpath` in its tree. The variable is then interpolated at line 222 into the callout format string.

## Solution

**Approach (a) from the issue:** commit the plan before stamping the SHA. This preserves the existing dispatch-gate shape and makes the callout truthful. **Approach (b) as fallback** on commit failure: stamp differently to be degraded-but-truthful.

### Concrete diff

**File:** `skills/bundled/_shared/dispatch-lib.sh`

**Replace lines 209–211** (Pin A, first three lines):
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
        # Pathspec-limited commit: only the plan file, not the full index.
        # Prevents capturing any other staged files from a partial pilot run.
        if ! git -C "$worktree_dir" commit -m "wip(${repo}#${issue_num}): plan staged by post-flight recovery" -- "$plan_relpath"; then
            echo "WARN: post_flight_class_d_commit_failed for $repo#$issue_num — stamping as uncommitted" >&2
            # Fall through to approach (b): stamp as uncommitted.
            # The (uncommitted ...) format deliberately does NOT match the
            # "committed on branch @" regex — downstream parsers
            # (check_grooming_markers, _detect_plan_on_branch) skip gracefully.
            local callout_block
            callout_block=$(cat <<CALLOUT_EOF
> - **Branch:** \`${branch}\`
> - **Plan:** \`${plan_relpath}\` (uncommitted on branch \`${branch}\`, see worktree)
> - **Grooming history:** body callout recovered by post-flight (mika#1123) — architect verdict not verified, operator dispatch required
CALLOUT_EOF
            )
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
        # Push the recovery commit so SHA is reachable from origin.
        # On push failure, fall through to approach (b) — a locally-valid
        # but remotely-unreachable SHA is the same fabrication class we're fixing.
        local push_err
        push_err=$(mktemp /tmp/push-err-XXXXXX)
        if ! git -C "$worktree_dir" push origin "$branch" 2>"$push_err"; then
            echo "WARN: post_flight_class_d_push_failed for $repo#$issue_num — falling back to uncommitted callout" >&2
            cat "$push_err" >&2
            rm -f "$push_err"
            # Commit succeeded but push failed — SHA is local-only.
            # Fall through to approach (b).
            local callout_block
            callout_block=$(cat <<CALLOUT_EOF
> - **Branch:** \`${branch}\`
> - **Plan:** \`${plan_relpath}\` (committed locally, push failed — see worktree)
> - **Grooming history:** body callout recovered by post-flight (mika#1123) — architect verdict not verified, operator dispatch required
CALLOUT_EOF
            )
            local new_body
            new_body=$(printf '%s\n\n%s' "$callout_block" "$current_body")
            local tmpfile
            tmpfile=$(mktemp /tmp/body-callout-recover-XXXXXX.md)
            printf '%s' "$new_body" > "$tmpfile"
            if gh issue edit "$issue_num" --repo "senara-solutions/$repo" \
                --body-file "$tmpfile" 2>/dev/null; then
                echo "body_callout_drift_recovered: wrote LOCAL-ONLY callout to $repo#$issue_num (committed but push failed)" >&2
            else
                echo "WARN: body_callout_drift_recovery_failed for $repo#$issue_num" >&2
            fi
            rm -f "$tmpfile"
            return 0
        fi
        rm -f "$push_err"
    fi

    local head_sha
    head_sha=$(git -C "$worktree_dir" rev-parse --short HEAD 2>/dev/null)
```

**Lines 213–241 remain unchanged** — the normal callout construction and write logic.

### Changes from initial plan (addressing mika-arch first-pass findings)

**F1 (Phase 0 Pin):** Added verbatim source slices as Phase 0 Pins A–D. Base SHA `f393fb36` confirmed to include mika#1144.

**F2 (Cross-ticket sequencing):** mika#1144 is already merged at `964770a8` (PR #1157). Base SHA includes it. Line numbers verified against post-#1144 function shape. No ordering constraint.

**F3a (push failure):** Replaced `git push || true` with `git push 2>"$push_err"` + conditional fallback. On push failure, the function now falls through to approach (b) with a `(committed locally, push failed — see worktree)` callout instead of silently swallowing the error.

**F3b (concurrent-dispatch race):** Addressed via Pin C — the invocation site guards on `$WORKTREE_DIR` existence, and each dispatch gets its own worktree (per `derive-worktree-path`). Concurrent dispatches operate on different worktree directories. The branch is shared, but the push is non-force (`git push origin "$branch"`) — if a concurrent push has advanced the remote ref, this push fails and the function falls through to the push-failure fallback. No silent corruption.

**F3c (commit scope):** Replaced `git add -- "$plan_relpath" && git commit` with pathspec-limited `git commit -- "$plan_relpath"`. This commits only the plan file regardless of what else may be staged in the index.

**NF3 (wip convention):** The `wip(...)` prefix is recovery-path-only. It is not a conventional-commit type and will not appear in changelogs. Adding it to the commit convention docs is out of scope for this bug fix but noted for follow-up.

**NF4 (downstream parser verification):** Added Pin D documenting all three downstream parsers and confirming they are unaffected by both the primary and fallback callout formats.

## Acceptance Criteria

- **AC1:** When `_verify_and_write_body_callout()` finds a plan file on disk that is NOT in HEAD's tree, it commits the file (pathspec-limited) and pushes before stamping the SHA. The resulting callout's SHA is verifiable: `git show <sha>:<plan-path>` succeeds.
- **AC2:** When the auto-commit fails (edge case), the callout uses `(uncommitted on branch ...)` instead of a fabricated SHA.
- **AC2b:** When the commit succeeds but push fails, the callout uses `(committed locally, push failed ...)` — truthful about the state.
- **AC3:** When the plan file IS already in HEAD's tree (normal case, no Class D drift), behavior is unchanged — the `cat-file -e` check passes and the new block is skipped entirely.
- **AC4:** Recovery commit message follows the `wip(repo#N)` convention, clearly marking it as post-flight recovery rather than organic pipeline output.

## Out of Scope

- Why the pipeline exits early (tracked by mika#804, mika#716)
- Body-callout drift Classes A/B/C (already fixed by mika#1123 + mika#1144)
- Dispatch-gate changes — the recovery callout still does NOT pass the dispatch gate (no `(GROOMED)` verdict); this fix only makes the SHA truthful
- Documenting `wip(...)` as a commit convention (follow-up)

## Risk Assessment

- **Low risk.** The change is additive — a new guard before the existing SHA capture. The normal path (plan already committed) hits the `cat-file -e` check, gets exit 0, and skips the new block entirely.
- **Failure modes degrade gracefully:**
  - Commit fails → fallback stamps `(uncommitted ...)` — degraded but truthful
  - Push fails → fallback stamps `(committed locally, push failed ...)` — degraded but truthful
  - Both are strictly better than the current fabricated SHA
- **No downstream parser impact:** Pin D confirms all three downstream parsers (`check_grooming_markers`, `_detect_plan_on_branch`, human `git show`) handle the fallback formats correctly or degrade gracefully.
- **No concurrent-dispatch risk:** Pin C confirms worktree-per-dispatch isolation; non-force push fails safely on remote-ref divergence.

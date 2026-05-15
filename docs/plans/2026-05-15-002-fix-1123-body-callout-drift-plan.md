---
ticket: mika#1123
type: fix
module: skills/bundled/_shared, skills/executor
tags: [dev-groom, dispatch-lib, body-callout, dispatch-readiness]
problem_type: structural reliability gap
category: workflow-issues
---

# Fix: Body Callout Drift — dev-groom pushes plan but never updates issue body

## Phase 0 Pin

All code references pinned at main `e2fd037edfd3`. mika#1108 (dispatch gate broadening)
is CLOSED/merged — the gate contract below is the settled post-#1108 state.

### Pin A — dispatch-lib.sh post-flight section (lines 363-385)

```bash
        # Post-flight diff check: detect zero-commit "success" in repo#number mode.
        if [ -n "$PRE_RUN_HEAD" ] && [ -n "$REPO" ]; then
            POST_RUN_HEAD=$(git -C "$WORKTREE_DIR" rev-parse HEAD 2>/dev/null || true)
            if [ -n "$POST_RUN_HEAD" ] && [ "$PRE_RUN_HEAD" = "$POST_RUN_HEAD" ]; then
                RESULT="PIPELINE FAILURE: claude-pilot exited 0 but HEAD unchanged ..."
            fi
        fi

        # Post-flight plan validation (mika#1033): detect dev-groom drift where
        # the session exits "success" but produced no valid plan file (or only a
        # stub/empty one). Runs independently of the HEAD-diff check — a session
        # can commit a 0-byte plan (HEAD changed) but still fail this check.
        if [ "$SKILL" = "dev-groom" ] && [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ]; then
            TODAY_PREFIX=$(date +%Y-%m-%d)
            VALID_PLAN=$(find "$WORKTREE_DIR/docs/plans" -name "${TODAY_PREFIX}-*-plan.md" -size +500c 2>/dev/null | head -1)
            if [ -z "$VALID_PLAN" ]; then
                RESULT="PIPELINE FAILURE: dev-groom produced no valid plan file ..."
            fi
        fi
```

The new body-callout verification inserts **after** line 385, as a third post-flight check.

### Pin B — check_grooming_markers() in executor.rs (lines 794-808)

```rust
pub fn check_grooming_markers(issue_body: &str) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if !issue_body.contains("> - **Branch:**") {
        missing.push("branch_callout");
    }
    if !issue_body.contains("docs/plans/") {
        missing.push("plan_callout");
    }
    let has_groomed_marker = issue_body.contains("second-pass (GROOMED)")
        || issue_body.contains("second-pass (READY, paraphrased GROOMED");
    if !has_groomed_marker {
        missing.push("groomed_verdict");
    }
    missing
}
```

Three substring checks: `> - **Branch:**`, `docs/plans/`, and
`second-pass (GROOMED)` OR `second-pass (READY, paraphrased GROOMED`.
The recovery callout must contain all three substrings to pass.

### Pin C — dev-groom system_prompt.md step 18 callout template (lines 83-89)

```markdown
18. Update the issue body. Read existing body, prepend canonical callouts:
    ```
    > - **Branch:** `<slug>`
    > - **Plan:** `<repo>/docs/plans/<file>` (committed on branch @ `<sha>`)
    > - **Grooming history:** /ce:plan -> mika-arch first-pass (<disposition>) -> revisions -> mika-arch second-pass (GROOMED)
    ```
    Apply with `gh issue edit <n> --repo senara-solutions/<repo> --body-file <tmpfile>`.
```

The LLM is instructed to write this block; the recovery function backstops it.

## Problem

The dev-groom skill's Phase 5 step 18 instructs the LLM to write canonical callouts
(`> - **Branch:**`, `> - **Plan:**`, `> - **Grooming history:**`) to the issue body
via `gh issue edit`. This step is **prompt-only** — no structural enforcement. When the
LLM skips or botches it, the plan commits to the branch successfully but the issue body
never gets the callout. The dispatch-readiness gate (`check_grooming_markers` in
`executor.rs`, Pin B) then correctly rejects with `dispatch_no_grooming_marker`, creating
a stuck ticket that requires operator intervention.

Empirical evidence: mika#794 had 2 commits on branch including a valid plan, but body
lacked all three callouts. Auto-groom fallback hit Mode 1a (HEAD unchanged — grooming
already complete). Parent task ended `blocked`.

## Root Cause

The body-callout write is entirely inside the LLM session — dispatch-lib.sh has
post-flight checks for HEAD-diff (Pin A line 363) and plan-file-exists (Pin A line 377),
but **no post-flight check for body callout**. When the LLM exits without writing the
callout, nothing catches it.

## Approach: Structural post-flight body-callout verification (Option A)

Add a post-flight check in `dispatch-lib.sh` that runs after dev-groom completes.
It verifies the issue body contains the canonical callout, and writes it via
`gh issue edit` if missing. This makes the body-callout write a **structural guarantee**,
not an LLM hope.

**Critical design constraint (F2 from mika-arch first-pass):** The recovery function
must NOT fabricate a GROOMED verdict. A >500-byte plan file on the branch is evidence
of a plan existing, not evidence that the architect issued `Verdict: GROOMED`. The
recovery callout uses a **non-gate-passing format** that surfaces the drift for operator
visibility without claiming grooming completed.

Option B (fallback branch-check in the dispatch gate) is explicitly deferred —
body-as-source-of-truth is the right invariant.

## Implementation

### Step 1: Add `_verify_and_write_body_callout()` to dispatch-lib.sh

**File:** `skills/bundled/_shared/dispatch-lib.sh`
**Location:** After Pin A line 385 (existing post-flight plan validation)

```bash
# Post-flight body-callout verification (mika#1123): detect dev-groom drift
# where the plan is committed and pushed but the issue body never received the
# canonical callout block. If missing, write a recovery callout that surfaces
# the drift without fabricating an architect verdict.
if [ "$SKILL" = "dev-groom" ] && [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ] && [ -n "$REPO" ] && [ -n "$ISSUE_NUM" ]; then
    _verify_and_write_body_callout "$REPO" "$ISSUE_NUM" "$WORKTREE_DIR" "$BRANCH"
fi
```

The function itself:

```bash
_verify_and_write_body_callout() {
    local repo="$1" issue_num="$2" worktree_dir="$3" branch="$4"

    # 1. Fetch current issue body
    local current_body
    current_body=$(gh issue view "$issue_num" --repo "senara-solutions/$repo" \
        --json body -q '.body' 2>/dev/null) || return 0

    # 2. Check if all three callout signals are already present
    #    (mirrors check_grooming_markers() in executor.rs — Pin B)
    local has_branch has_plan has_verdict
    has_branch=$(printf '%s' "$current_body" | grep -cF '> - **Branch:**' || true)
    has_plan=$(printf '%s' "$current_body" | grep -cF 'docs/plans/' || true)
    has_verdict=$(printf '%s' "$current_body" | grep -cE 'second-pass \(GROOMED\)|second-pass \(READY, paraphrased GROOMED' || true)

    if [ "$has_branch" -gt 0 ] && [ "$has_plan" -gt 0 ] && [ "$has_verdict" -gt 0 ]; then
        return 0  # All present, nothing to do
    fi

    # 3. Find the plan file on the branch — scoped to issue number
    local plan_file
    plan_file=$(find "$worktree_dir/docs/plans" -name "*-${issue_num}-*-plan.md" -size +500c \
        2>/dev/null | sort -r | head -1)

    # Fall back to any plan file if issue-scoped search finds nothing
    if [ -z "$plan_file" ]; then
        plan_file=$(find "$worktree_dir/docs/plans" -name "*-plan.md" -size +500c \
            2>/dev/null | sort -r | head -1)
    fi

    [ -n "$plan_file" ] || return 0  # No valid plan file — can't write callout

    local plan_relpath="${plan_file#"$worktree_dir/"}"
    local head_sha
    head_sha=$(git -C "$worktree_dir" rev-parse --short HEAD 2>/dev/null)

    # 4. Determine which signals are missing and construct recovery callout.
    #    IMPORTANT: The grooming-history line does NOT contain "second-pass (GROOMED)"
    #    because we cannot verify the architect actually issued that verdict from
    #    branch state alone. This callout surfaces the drift for operator visibility
    #    but does NOT pass the dispatch gate — the operator must verify and dispatch
    #    manually (or re-run dev-groom which will write the organic callout).
    local callout_block=""

    # Always write all three callout lines (idempotent — existing organic callouts
    # will have passed the check in step 2 and we wouldn't be here)
    callout_block=$(cat <<CALLOUT_EOF
> - **Branch:** \`${branch}\`
> - **Plan:** \`${plan_relpath}\` (committed on branch @ \`${head_sha}\`)
> - **Grooming history:** body callout recovered by post-flight (mika#1123) — architect verdict not verified, operator dispatch required
CALLOUT_EOF
    )

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
}
```

**Design decisions:**

- **No verdict fabrication (F2 resolution):** The grooming-history line says
  "body callout recovered by post-flight — architect verdict not verified, operator
  dispatch required". This does NOT contain "second-pass (GROOMED)" and therefore
  does NOT pass `check_grooming_markers()`. The dispatch gate correctly continues
  to reject until the operator verifies the architect verdict and either:
  (a) re-runs dev-groom (which writes the organic callout with the real verdict), or
  (b) manually edits the body to add the groomed marker.

- **Partial recovery is still valuable:** Even though the callout doesn't pass the
  dispatch gate, it provides Branch + Plan signals. The operator sees exactly which
  plan file exists on which branch, reducing investigation time from "dig through
  branches" to "verify architect verdict and add the marker."

- **Plan-file discovery scoped to issue number (NF1 resolution):** Primary search
  uses `*-${issue_num}-*-plan.md` pattern. Falls back to any `*-plan.md` if
  issue-scoped search finds nothing (handles non-standard naming).

- **Idempotent:** Checks all three signals before writing. Re-runs are safe.

- **Fail-open:** `|| return 0` on gh failures — don't block dispatch on API errors.

- **Dual-write documentation (NF2 resolution):** After this fix, body callouts can
  be written by two paths: (1) the LLM in dev-groom step 18 (organic, passes gate),
  and (2) the structural recovery in `_verify_and_write_body_callout()` (partial,
  does NOT pass gate). This is documented in the function's header comment.

### Step 2: Add unit test for `check_grooming_markers` with recovery callout

**File:** `crates/mika-agent/src/skills/executor.rs`

```rust
#[test]
fn test_check_grooming_markers_recovery_callout_does_not_pass() {
    // Recovery callout written by dispatch-lib.sh post-flight (mika#1123)
    // intentionally does NOT pass the gate — it surfaces drift without
    // fabricating an architect verdict.
    let body = r#"> - **Branch:** `fix/794/agent-pr-merge`
> - **Plan:** `docs/plans/2026-05-15-001-fix-plan.md` (committed on branch @ `abc1234`)
> - **Grooming history:** body callout recovered by post-flight (mika#1123) — architect verdict not verified, operator dispatch required

## Symptom
..."#;
    let missing = check_grooming_markers(body);
    // Branch and plan callouts pass, but groomed_verdict is correctly missing
    assert!(!missing.contains(&"branch_callout"));
    assert!(!missing.contains(&"plan_callout"));
    assert!(missing.contains(&"groomed_verdict"),
        "Recovery callout must NOT pass the groomed_verdict check — \
         it doesn't fabricate an architect verdict");
}

#[test]
fn test_check_grooming_markers_organic_callout_passes() {
    // Organic callout written by the LLM in dev-groom step 18
    let body = r#"> - **Branch:** `fix/794/agent-pr-merge`
> - **Plan:** `docs/plans/2026-05-15-001-fix-plan.md` (committed on branch @ `abc1234`)
> - **Grooming history:** /ce:plan -> mika-arch first-pass (ITERATE) -> revisions -> mika-arch second-pass (GROOMED)

## Symptom
..."#;
    let missing = check_grooming_markers(body);
    assert!(missing.is_empty(),
        "Organic callout with all three signals should pass: {:?}", missing);
}
```

### Step 3: Verify gh auth in dispatch-lib.sh context (NF3 resolution)

The dispatch-lib.sh execution environment already uses `gh` for PR URL discovery
(Pin A line 389: `gh pr list --repo ...`). The same `GITHUB_TOKEN`/`gh auth` context
is available. No additional auth setup needed.

## Files Changed

| File | Change |
|------|--------|
| `skills/bundled/_shared/dispatch-lib.sh` | Add `_verify_and_write_body_callout()` function + post-flight call site after line 385 |
| `crates/mika-agent/src/skills/executor.rs` | Add 2 unit tests for recovery vs organic callout gate behavior |

## Acceptance Criteria Mapping

- **AC-1:** Dev-groom's post-push step always writes the canonical Plan callout →
  backstopped structurally by post-flight. Organic callout (from LLM) passes the
  dispatch gate; recovery callout (from post-flight) surfaces the drift for operator.
- **AC-2:** `dispatch-lib.sh` post-flight check verifies body has callout; writes if
  missing + logs `body_callout_drift_recovered` → implemented in
  `_verify_and_write_body_callout`. Recovery callout intentionally does NOT pass the
  dispatch gate (no verdict fabrication).
- **AC-3:** Regression test: recovery callout format → gate correctly rejects
  (groomed_verdict missing). Organic callout format → gate correctly passes. Both
  covered by executor.rs unit tests.
- **AC-4:** No regression for tickets without grooming →
  `_verify_and_write_body_callout` returns early when no plan file found.

## Out of Scope

- **Option B (branch-check fallback in dispatch gate):** Deferred per ticket
  recommendation. Body-as-source-of-truth preserved.
- **Recovery of already-stuck tickets (mika#794 etc.):** Operator-path body refresh.
  This fix prevents future drift, doesn't retroactively fix existing stuck tickets.
  A one-time recovery script could be a follow-up.
- **Full auto-recovery (writing GROOMED verdict in recovery):** Rejected per F2
  finding. The recovery cannot verify the architect verdict from branch state alone.
  Auto-dispatch requires the organic callout path (LLM writes it after receiving
  the arch verdict) or operator manual verification.
- **NF4 (no-op path logging):** Deferred — low priority, can add later if
  observability is needed.

## Cross-ticket dependency

- **mika#1108** (dispatch gate broadening): CLOSED/merged. The `check_grooming_markers()`
  contract pinned in Pin B is the settled post-#1108 state. No sequencing issue.

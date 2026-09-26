---
title: Strip recovery shim's "any plan file" fallback — Class C body-callout drift
ticket: mika#1144
parent_ticket: mika#1123
type: bug
status: completed
created: 2026-05-16
---

# Plan: Strip recovery shim's "any plan file" fallback (mika#1144, Class C drift)

## TL;DR

The post-flight body-callout recovery shim in `skills/bundled/_shared/dispatch-lib.sh`
(added by mika#1123) has a fallback that, when no issue-scoped plan file is found on
the worktree, picks "any `*-plan.md` in `docs/plans/`, lexicographically last" and writes
a body callout pointing at it. Because the worktree is branched from `main`, that
fallback deterministically returns the most-recently-merged plan from some other ticket.
Result: a fabricated callout that looks valid but routes the reader into stale,
unrelated work (Class C body-callout drift).

**Fix:** Remove the fallback entirely. When no issue-scoped plan exists, log a stderr
warning and return without writing a callout. The orchestrator's Class A path (missing
callout → dispatch dev-groom) handles it correctly. A missing callout is more
recoverable than a wrong one.

## Phase 0 — Pin (load-bearing source slices)

**Base commit:** `31c1b0a5` (worktree HEAD = `origin/main` tip at grooming time,
2026-05-16). The plan-staging commit (this plan file's eventual first commit) is
the only delta on the branch; the source files pinned below are unchanged
from `origin/main`.

> **Naming note (mika-arch second-pass NF6):** The issue body of mika#1144
> refers to the function as `recover_body_callout_drift()`. The actual function
> name in `skills/bundled/_shared/dispatch-lib.sh` (verified via Pin P0.1 below
> at line 158) is **`_verify_and_write_body_callout()`**. This plan and the
> implementation target the underscore-prefixed name as it appears in the
> source tree; the issue body's reference is inaccurate but functionally
> unambiguous (the lines 185–196 citation pins the same code regardless of
> what the function is called). No rename is in scope for this PR.

Five sites determine the load-bearing claims of this plan. P0.1, P0.4, and P0.5
are the implementer-facing pins (the exact slices that are deleted, retained, or
inserted-after). P0.2 and P0.3 are dispatch-gate / orchestrator-path context.

### P0.1 — `_verify_and_write_body_callout()` plan-file lookup (delete + replace target)

`skills/bundled/_shared/dispatch-lib.sh:185–196`:

```bash
    # 3. Find the plan file on the branch — scoped to issue number first
    local plan_file
    plan_file=$(find "$worktree_dir/docs/plans" -name "*-${issue_num}-*-plan.md" -size +500c \
        2>/dev/null | sort -r | head -1)

    # Fall back to any plan file if issue-scoped search finds nothing
    if [ -z "$plan_file" ]; then
        plan_file=$(find "$worktree_dir/docs/plans" -name "*-plan.md" -size +500c \
            2>/dev/null | sort -r | head -1)
    fi

    [ -n "$plan_file" ] || return 0  # No valid plan file — can't write callout
```

The fallback at lines 191–194 is the source of Class C drift. When the worktree is
branched from `main` (which carries every previously-merged plan in `docs/plans/`),
the fallback returns the lexicographically-largest filename — typically a recent
unrelated plan.

### P0.2 — `check_grooming_markers()` dispatch gate (Pin B from mika#1123 plan)

`crates/mika-agent/src/skills/executor.rs:794–808` (semantics — three substring
checks: `> - **Branch:**`, `docs/plans/`, and `second-pass (GROOMED)` or
`second-pass (READY, paraphrased GROOMED`). The recovery callout's "Grooming history"
line intentionally does **not** contain a GROOMED marker, so the dispatch gate
correctly rejects even when the recovery callout is wrong.

This means Class C does **not** auto-dispatch implementation. The danger surface is
operator visibility: an operator skimming the issue body sees a green-looking callout
and may manually dispatch against the wrong plan.

### P0.3 — Class A handling on the orchestrator side

When the recovery shim writes no callout at all (Class A), the next autonomous
dispatch hits the grooming-marker gate (mika#1108), the auto-groom path
(mika#996) re-runs dev-groom, and dev-groom either writes the organic callout
(the happy path) or repeats the drift (escalates upward). Either way, the failure
mode stays loud and recoverable — no wrong-path side effect.

### P0.4 — `$plan_file` consumption chain (untouched call site)

`skills/bundled/_shared/dispatch-lib.sh:198–229`:

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

`$plan_file` is consumed only at line 198 (`plan_relpath="${plan_file#...}"`).
There is no earlier consumer between the discovery section (185–196) and this
call site. The fallback's deletion is therefore safe — no downstream code reads
`$plan_file` in any path that depends on the fallback ever firing. The callout
construction (lines 202–214), the body prepend (216–221), and the `gh issue
edit` invocation (223–228) are all unchanged by this PR.

### P0.5 — Test ordinal for the new regression guard

`skills/bundled/_shared/test-dispatch-lib.sh` contains seven tests today
(Test 1 line 61, Test 2 line 81, Test 3 line 116, Test 4 line 130, Test 5 line
148, Test 6 line 182, Test 7 line 228). The Summary block starts at line 262.
The new test is **Test 8** and is inserted immediately before the Summary block
(after line 260 / Test 7's last assertion).

This pin guards against a concurrent test addition shifting the ordinal — if
another PR lands a Test 8 before this one, the implementer must renumber to
Test 9 at insertion time.

## Problem

Three classes of body-callout drift exist (per `feedback_body_callout_drift_two_classes`
memory, extended to three classes 2026-05-16):

| Class | Signal | Disposition |
|-------|--------|-------------|
| A | No callout at all | Re-dispatch dev-groom |
| B | Callout present, verdict line has extra text in parens | `perl`-patch the verdict line |
| C | Callout present with **wrong plan path** (points at an unrelated, previously-merged plan) | Strip callout + re-groom |

Class C is the most dangerous: the callout passes a casual eyeball check (branch
name matches the issue, plan path is real and on the branch — technically — because
it was inherited from `main`), but the linked plan has nothing to do with the issue.

**Canonical Class C instance:** mika#1142 grooming session 2026-05-16. dev-groom
exited with zero commits ahead of main on worktree
`bug-1142-claude-pilot-log-emits-zero-events-when`. Recovery shim ran:

1. `find docs/plans -name '*-1142-*-plan.md'` → empty.
2. Fallback: `find docs/plans -name '*-plan.md' | sort -r | head -1` →
   `docs/plans/2026-05-15-923-fix-shared-dir-install-plan.md` (mika#923's plan,
   already merged to main).
3. Shim wrote a callout to mika#1142's body pointing at #923's plan.

The "architect verdict not verified, operator dispatch required" hedge in the
Grooming history line is the only barrier between the wrong callout and an
operator-driven implementation dispatch against unrelated work.

## Root cause

The fallback at `dispatch-lib.sh:191–194` is structurally wrong. The worktree is
branched from `main`; `find` scans the on-disk tree, not commits on this branch.
Every plan file from every previously-merged PR is visible, and `sort -r | head -1`
returns the lexicographically-largest filename — which, due to the
`YYYY-MM-DD-NNN-*-plan.md` naming convention, is reliably the most recently-merged
unrelated plan.

The fallback was added (per mika#1123 plan § "Plan-file discovery scoped to issue
number (NF1 resolution)") to handle non-standard plan filenames. Empirically, no
real Class C drift incident has been caused by a properly-written plan with a
non-standard name — the only observed cause is dev-groom drifting and writing
**no plan at all**.

## Approach: Strip the fallback (Option A from ticket)

Remove lines 191–194 of `dispatch-lib.sh` (the fallback `if [ -z "$plan_file" ];
then ... fi` block). When the issue-scoped lookup returns empty, log a stderr
warning and `return 0` — the same exit path the function already takes when no
plan file is found at all.

The recommended fix from the ticket's "Fix proposal" section selects Option A
explicitly: "Class C only occurs because we are trying too hard to write a callout
when the real failure is upstream (dev-groom drift). Treat the absent-plan case as
a hard failure of recovery, log to stderr, and let the orchestrator's 'no callout'
check (Class A path) handle it the same as the no-recovery case."

Option B (constrain the fallback via `git log --diff-filter=A --name-only
main..HEAD`) is rejected for the same reason: it adds defensive complexity to
preserve a fallback that has never recovered a real incident, only created them.

### Why not write a sentinel "callout missing, run dev-groom again" placeholder?

The shim has no canonical contract for partial state; adding one is more surface
than removing the fallback. The orchestrator's Class A path already handles
"no callout" cleanly. A sentinel placeholder would require teaching every consumer
(dispatch gate, operator visual inspection, future dev-groom resume logic) to
recognize it. Simpler to leave the body alone.

## Implementation

### Step 1: Remove the fallback in `dispatch-lib.sh`

**File:** `skills/bundled/_shared/dispatch-lib.sh`
**Location:** `_verify_and_write_body_callout()`, lines 185–196.

**Before** (current shape):

```bash
    # 3. Find the plan file on the branch — scoped to issue number first
    local plan_file
    plan_file=$(find "$worktree_dir/docs/plans" -name "*-${issue_num}-*-plan.md" -size +500c \
        2>/dev/null | sort -r | head -1)

    # Fall back to any plan file if issue-scoped search finds nothing
    if [ -z "$plan_file" ]; then
        plan_file=$(find "$worktree_dir/docs/plans" -name "*-plan.md" -size +500c \
            2>/dev/null | sort -r | head -1)
    fi

    [ -n "$plan_file" ] || return 0  # No valid plan file — can't write callout
```

**After:**

```bash
    # 3. Find the plan file on the branch — scoped to issue number only.
    # Class C drift fix (mika#1144): no fallback to "any plan file". A worktree
    # branched from main carries every previously-merged plan in docs/plans/, so
    # an unscoped `find | sort -r | head -1` reliably returns the most recent
    # unrelated plan. A missing callout (Class A) is more recoverable than a
    # wrong one (Class C).
    local plan_file
    plan_file=$(find "$worktree_dir/docs/plans" -name "*-${issue_num}-*-plan.md" -size +500c \
        2>/dev/null | sort -r | head -1)

    if [ -z "$plan_file" ]; then
        echo "body_callout_drift_recovery_skipped: no issue-scoped plan file found for $repo#$issue_num (Class A — orchestrator will re-dispatch dev-groom)" >&2
        return 0
    fi
```

**What changed:**

- Deleted the `if [ -z "$plan_file" ]; then ... fi` fallback block (lines 191–194 in
  the current file).
- Replaced the silent `[ -n "$plan_file" ] || return 0` with an explicit stderr log
  before returning. The log line uses a distinct key (`body_callout_drift_recovery_skipped`)
  that operators and log scanners can grep for separately from the success log
  (`body_callout_drift_recovered`).
- Added a header comment explaining why the fallback was removed (anchors future
  readers to the Class C incident).

### Step 2: Update the function's header comment to document the new contract

**File:** `skills/bundled/_shared/dispatch-lib.sh`
**Location:** `_verify_and_write_body_callout()` header, lines 158–167.

The current header reads:

```bash
_verify_and_write_body_callout() {
    # Post-flight body-callout recovery (mika#1123): detect dev-groom drift where
    # the plan is committed and pushed but the issue body never received the
    # canonical callout block. If missing, write a recovery callout that surfaces
    # the drift without fabricating an architect verdict.
    #
    # Dual-write documentation: body callouts can be written by two paths:
    #   (1) the LLM in dev-groom step 18 (organic, passes dispatch gate)
    #   (2) this structural recovery (partial, does NOT pass dispatch gate)
```

Append a third bullet to the dual-write documentation block to capture the new
no-write case:

```bash
    # Dual-write documentation: body callouts can be written by two paths:
    #   (1) the LLM in dev-groom step 18 (organic, passes dispatch gate)
    #   (2) this structural recovery (partial, does NOT pass dispatch gate)
    # When the issue-scoped plan file is missing (dev-groom drift produced no
    # plan at all), this function logs body_callout_drift_recovery_skipped and
    # exits without writing — the orchestrator's Class A path handles it
    # (re-dispatch dev-groom). See mika#1144 for the Class C drift this guards
    # against.
```

### Step 3: Add a structural test to `test-dispatch-lib.sh`

**File:** `skills/bundled/_shared/test-dispatch-lib.sh`
**Location:** Append after the existing Test 7 block (per Pin P0.5, last line 260),
before the Summary section at line 262.

The test is a **paired regression guard** (per architect first-pass F2): it must
assert both that the unscoped fallback is **absent** AND that the issue-scoped
find is **present**. A refactor that accidentally removed the scoped find too
would otherwise pass a deletion-only test.

**Grep patterns explicitly used in Test 8:**

| Pattern | Purpose | Expected count |
|---------|---------|----------------|
| `find.*-name *"\*-\${issue_num}-\*-plan\.md"` | Scoped find — the retained call | ≥ 1 in `$VERIFY_FUNC` |
| `plan_file=.*find.*-name *"\*-plan\.md"` followed by `grep -v issue_num` | Unscoped find — the deleted fallback (filter out the scoped find by requiring `issue_num` is absent on the same line) | 0 in `$VERIFY_FUNC` |
| `body_callout_drift_recovery_skipped` | New stderr log key | ≥ 1 in `$VERIFY_FUNC` |
| `return 0` and `>&2` inside the `if [ -z "$plan_file" ]` block | Early-return guard with stderr log | ≥ 1 each in `$SKIP_BLOCK` |

Both production-shape assertions (positive scoped find, absent unscoped find) are
scoped to `$VERIFY_FUNC` — the body of `_verify_and_write_body_callout()`
extracted via `sed`. This mirrors the convention in Test 1 (sed-extracts
`CLOSED_BLOCK`), Test 4 (sed-extracts `ISSUE_STATE_REGION`), and Test 6
(sed-extracts `PLAN_FUNC`).

```bash
# --- Test 8: Recovery shim fallback removed (mika#1144) ---

echo ""
echo "Test 8: _verify_and_write_body_callout has no unscoped fallback"
echo "-----------------------------------------------------------------"

VERIFY_FUNC=$(sed -n '/_verify_and_write_body_callout()/,/^}/p' "$DISPATCH_LIB")

# Assertion 1 (POSITIVE — F2 paired-test discipline): the issue-scoped find is
# retained. If a refactor accidentally removes BOTH finds, this assertion fires.
SCOPED_FIND_COUNT=$(printf '%s\n' "$VERIFY_FUNC" \
    | grep -cE 'find.*-name *"\*-\$\{issue_num\}-\*-plan\.md"' || true)
assert_eq "Issue-scoped find call retained" "1" "$SCOPED_FIND_COUNT"

# Assertion 2 (NEGATIVE — the core mika#1144 regression guard): the unscoped
# fallback find is gone. Detection shape: an assignment to plan_file with a
# `find ... -name "*-plan.md"` glob that does NOT contain `${issue_num}`.
# The scoped find above DOES contain `${issue_num}` and is filtered out by the
# `grep -v issue_num` step.
UNSCOPED_FALLBACK_COUNT=$(printf '%s\n' "$VERIFY_FUNC" \
    | grep -E 'plan_file=.*find.*-name *"\*-plan\.md"' \
    | grep -vc 'issue_num' || true)
assert_eq "No unscoped fallback find call (mika#1144 strip)" "0" "$UNSCOPED_FALLBACK_COUNT"

# Assertion 3: the new stderr log key is present (signals the skip path is
# explicit, not silent).
assert_contains "Skipped-recovery stderr log present" \
    'body_callout_drift_recovery_skipped' "$VERIFY_FUNC"

# Assertion 4: the function still has the early-return guard after the
# issue-scoped find — no plan file → return 0, now with a log line on stderr.
SKIP_BLOCK=$(printf '%s\n' "$VERIFY_FUNC" | sed -n '/if \[ -z "\$plan_file" \]/,/fi/p' | head -10)
assert_contains "Skip block returns 0" "return 0" "$SKIP_BLOCK"
assert_contains "Skip block logs to stderr" '>&2' "$SKIP_BLOCK"
```

The four assertions cover both directions of the structural change:

- **Assertions 1 + 2** are paired: assertion 1 catches "scoped find accidentally
  removed in a refactor"; assertion 2 catches "unscoped fallback re-added."
  Together they nail the discovery-section shape.
- **Assertion 3** verifies the skip path's observability (stderr key present).
- **Assertion 4** verifies the skip path's control-flow shape (`return 0` + `>&2`
  inside the `if [ -z "$plan_file" ]` block).

The grep shapes are anchored on production code (`$VERIFY_FUNC` only, not the
full file), so a future comment line that happens to mention `find ... -name
"*-plan.md"` elsewhere in `dispatch-lib.sh` does not produce a false positive.

### Step 4: Update operator memory note

**File:** `~/.claude/projects/-data-workspace-mika-platform/memory/feedback_body_callout_drift_two_classes.md`

This memory is the operator's diagnostic recipe for the three drift classes. The
current file references "two classes" in its slug and only documents Classes A
and B. The Class C entry should be appended (or the file renamed and rewritten
to cover all three).

**Out of scope for this PR.** Memory edits are operator-managed and cross-repo
(memory lives in the orchestrator's home directory, not this repo). The plan
flags the update as a follow-up so it doesn't get lost; the operator will apply
it after merge.

## Files Changed

| File | Change |
|------|--------|
| `skills/bundled/_shared/dispatch-lib.sh` | Remove unscoped fallback in `_verify_and_write_body_callout()`; add explicit stderr log on skip; update header comment |
| `skills/bundled/_shared/test-dispatch-lib.sh` | Add Test 8: structural regression guard for the removed fallback |

No Rust code changes. No new dependencies. No schema changes.

## Acceptance Criteria Mapping

The ticket lists four acceptance criteria. Each maps to a verification step.

- **AC-1 — Class C eliminated:** When dev-groom drifts on issue N and writes no
  `*-N-*-plan.md` file, the recovery shim does NOT write a callout pointing at any
  other plan. **Verified by:** Test 8 structural assertion that the unscoped
  fallback is gone. **Behavioural verification (manual smoke):** create a worktree
  branched from main with no plan file for issue X; run `_verify_and_write_body_callout
  "mika" "X" "<worktree>" "<branch>"`; assert no `gh issue edit` is fired and
  stderr contains `body_callout_drift_recovery_skipped`.

- **AC-2 — Class A behavior preserved:** When truly no plan exists, the issue body
  remains untouched and the orchestrator sees the missing callout. **Verified by:**
  the new skip block returns 0 before reaching the `gh issue edit` call site
  (Step 1 placement above the existing `plan_relpath`/`head_sha`/`callout_block`
  code). Test 8's "Skip block returns 0" assertion checks this structurally.

- **AC-3 — Class B path unaffected:** Verdict-line drift recovery (if any) is
  separate. **Verified by:** this change touches only the plan-file discovery
  section (lines 185–196). The callout construction and `gh issue edit` call
  (lines 198–229 in the current file) are unchanged. Class B is currently handled
  operator-side (perl-patch) per the memory note; no shim path changes.

- **AC-4 — Regression: organic happy path:** When dev-groom DOES write a plan, the
  issue-scoped match still fires and the callout is written correctly. **Verified
  by:** the issue-scoped find call is unchanged (Test 8's "Issue-scoped find call
  present" assertion). The downstream callout construction is unchanged. The only
  behavioural delta is in the empty-result branch.

## Verification

Local verification before push:

```bash
# 1. Run the test suite (must show 1 new test, 0 failures)
bash skills/bundled/_shared/test-dispatch-lib.sh

# 2. Static check: confirm the fallback grep returns zero in the new shape
grep -c 'find.*"\*-plan\.md"' skills/bundled/_shared/dispatch-lib.sh
# Expect: 1 (only the issue-scoped find remains, which contains "${issue_num}")
grep -c 'find.*"\*-${issue_num}-\*-plan\.md"' skills/bundled/_shared/dispatch-lib.sh
# Expect: 1

# 3. Static check: confirm the new stderr log key is present
grep -c 'body_callout_drift_recovery_skipped' skills/bundled/_shared/dispatch-lib.sh
# Expect: 1
```

End-to-end verification requires a real dispatch with a synthetic dev-groom drift —
that is **out of scope for this PR** (would require provisioning a test agent and
dispatching against a fake issue). The structural test + manual smoke covers the
behavioural change with sufficient confidence given the small surface area.

## Risk and rollback

**Blast radius:** Bash-only change in a recovery code path that has never
recovered a real incident (the fallback's only documented effect is generating
Class C drift). Removing it cannot regress anything that wasn't already broken.

**Rollback:** Revert the single commit. The pre-#1144 behavior is restored. Class
C drift returns; Class A handling is unaffected.

**Forward risk:** A future dev-groom drift that writes a plan with a non-standard
filename (e.g., `docs/plans/2026-05-16-misc-fix.md`, no issue number) would now
trigger Class A instead of Class C-with-wrong-path. Class A re-dispatches
dev-groom, which would re-attempt the plan and either write a properly-named one
or fail loudly. This is strictly better than Class C — but if there is hidden
reliance on the fallback for non-standard names, this PR surfaces it as
re-dispatch loops, which is observable.

## Related

- mika#1123 — parent ticket that added the recovery shim; this PR is a fix-on-top
  of the shim's fallback behavior.
- mika#1142 — Class C canary (the recursive grooming session that exposed this bug).
- mika#1108 — the dispatch-readiness gate that correctly rejects even when the
  wrong callout is written; this fix removes the upstream wrong-callout source.
- mika#996 — auto-groom-on-dispatch (the Class A recovery path that this fix
  routes drift into).
- `docs/solutions/workflow-issues/recovery-shim-fallback-wrong-plan-path-class-c-2026-05-16.md`
  — the solutions-doc characterization of Class C drift.

## Out of scope

- Operator memory note update (`feedback_body_callout_drift_two_classes.md` →
  three classes) — managed in orchestrator home, not this repo.
- Class B verdict-line drift handling improvements — separate failure mode,
  separate ticket if/when it becomes worth automating.
- End-to-end dispatch integration test — would require a test-agent fixture
  beyond the scope of this fix.

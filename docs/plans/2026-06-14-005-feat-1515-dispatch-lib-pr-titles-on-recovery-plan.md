# feat(dispatch-lib): PR titles on recovery should carry the conventional-commit subject, not the recovery class

**Ticket:** mika#1515
**Type:** enhancement / infrastructure
**Scope:** `skills/bundled/_shared/dispatch-lib.sh` — recovery PR-title logic only

## Problem

dispatch-lib's two recovery paths (dirty-worktree mika#1282, PR-create recovery mika#1396) set PR titles that describe the recovery mechanism rather than the implementation content:

- dirty-worktree: `wip(mika#NNNN): rescued impl (dispatch-lib recovery)`
- commit-pushed-no-pr: `mika#NNNN: pilot impl (dispatch-lib PR-create recovery, mika#1396)`

The conventional-commit subject already exists — either as the impl commit's subject (commit-pushed-no-pr class) or derivable from the plan/issue (dirty-worktree class). The fix is to extract it and use it as the PR title.

## Design

### Title derivation strategy by recovery class

**Class: `commit-pushed-no-pr`** (lines 2125–2129)

The pilot committed with a well-formed conventional-commit subject. The branch tip has the impl commit. Extract:

```bash
_rescue_title=$(git -C "$WORKTREE_DIR" log -1 --format='%s' HEAD)
```

This is the simplest case — the subject is already there.

**Class: `dirty-worktree`** (lines 2120–2124)

The pilot never committed. dispatch-lib created a `wip(...)` rescue commit. The rescue commit subject is not useful. Derivation fallback chain:

1. **Plan file H1 title + issue labels → conventional-commit format.** The plan file (`docs/plans/*-<ISSUE_NUM>-*-plan.md`) is reliably present in the rescued tree (the pilot ran `/ce:plan` before writing code). Parse the H1 (`# <title>`), map the issue's primary type label (`enhancement` → `feat`, `bug` → `fix`, etc.) and scope from the plan filename or issue title, and construct `<type>(<scope>): <description> (mika#NNNN)`.

2. **Issue title fallback.** If no plan file is found (edge case — pilot crashed before plan), use the issue title with label-derived type prefix: `<type>: <issue-title> (mika#NNNN)`.

### Label-to-type mapping

```bash
_label_to_type() {
    case "$1" in
        *enhancement*|*feature*) echo "feat" ;;
        *bug*)                   echo "fix" ;;
        *infrastructure*)        echo "chore" ;;
        *documentation*)         echo "docs" ;;
        *refactor*)              echo "refactor" ;;
        *test*)                  echo "test" ;;
        *)                       echo "chore" ;;
    esac
}
```

Uses the `LABELS` variable already captured at line 456 (comma-separated label list from the issue JSON).

### Scope extraction

Extract scope from the plan H1 or issue title by matching the conventional-commit pattern if already present (e.g., `feat(dispatch-lib): ...` → scope is `dispatch-lib`). If no scope is present in the title, omit it — `feat: description` is valid conventional-commit.

### Recovery metadata preservation (AC3)

The recovery class, pilot session ID, turns, and cost are already in the PR body's `## Recovery metadata` block (lines 2139–2153). No change needed — the title cleanup does not touch the body template.

## Implementation

### Step 1: Add `_derive_recovery_pr_title` helper function

Add a new shell function in dispatch-lib.sh, placed near the recovery block (before `_deliver_callback`). This function encapsulates the title derivation logic:

```bash
# _derive_recovery_pr_title — Compute a conventional-commit PR title for
# recovery-class PRs. Called by Unit 2 (mika#1282 + mika#1396) recovery block.
#
# For commit-pushed-no-pr: reads the impl commit subject from branch tip.
# For dirty-worktree: reads the plan file H1 or falls back to issue title.
#
# Args:
#   $1 — recovery class ("dirty-worktree" or "commit-pushed-no-pr")
#   $2 — worktree dir
#   $3 — repo name
#   $4 — issue number
#   $5 — labels (comma-separated)
#   $6 — issue title
#
# Outputs: PR title string to stdout
_derive_recovery_pr_title() {
    local recovery_class="$1"
    local wt_dir="$2"
    local repo="$3"
    local issue_num="$4"
    local labels="$5"
    local issue_title="$6"

    if [ "$recovery_class" = "commit-pushed-no-pr" ]; then
        # The impl commit subject is the branch tip's subject
        local impl_subject
        impl_subject=$(git -C "$wt_dir" log -1 --format='%s' HEAD 2>/dev/null)
        if [ -n "$impl_subject" ]; then
            echo "$impl_subject"
            return
        fi
    fi

    # dirty-worktree or fallback: derive from plan H1 + labels
    local type_prefix
    type_prefix=$(_label_to_type "$labels")

    # Look for plan file
    local plan_file
    plan_file=$(find "$wt_dir/docs/plans" -name "*-${issue_num}-*-plan.md" -size +500c 2>/dev/null | sort -r | head -1)

    if [ -n "$plan_file" ]; then
        # Extract H1 title from plan
        local plan_h1
        plan_h1=$(head -5 "$plan_file" | grep -m1 '^# ' | sed 's/^# //')
        if [ -n "$plan_h1" ]; then
            # Check if H1 already has conventional-commit format
            if echo "$plan_h1" | grep -qE '^(feat|fix|chore|docs|refactor|test|perf|ci)\b'; then
                echo "$plan_h1"
                return
            fi
            # Construct from type + plan H1
            echo "${type_prefix}: ${plan_h1} (${repo}#${issue_num})"
            return
        fi
    fi

    # Final fallback: issue title
    # Check if issue title already has conventional-commit format
    if echo "$issue_title" | grep -qE '^(feat|fix|chore|docs|refactor|test|perf|ci)\b'; then
        echo "$issue_title"
        return
    fi
    echo "${type_prefix}: ${issue_title} (${repo}#${issue_num})"
}
```

### Step 2: Add `_label_to_type` helper

Small function (shown above in design) that maps label strings to conventional-commit type prefixes. Placed adjacent to `_derive_recovery_pr_title`.

### Step 3: Replace hardcoded rescue titles

In the recovery block (lines 2116–2153), replace the two `_rescue_title` assignments:

**Before (dirty-worktree, line 2121):**
```bash
_rescue_title="wip(${REPO}#${ISSUE_NUM}): rescued impl (dispatch-lib recovery)"
```

**After:**
```bash
_rescue_title=$(_derive_recovery_pr_title "dirty-worktree" "$WORKTREE_DIR" "$REPO" "$ISSUE_NUM" "$LABELS" "$ISSUE_TITLE")
```

**Before (commit-pushed-no-pr, line 2126):**
```bash
_rescue_title="${REPO}#${ISSUE_NUM}: pilot impl (dispatch-lib PR-create recovery, mika#1396)"
```

**After:**
```bash
_rescue_title=$(_derive_recovery_pr_title "commit-pushed-no-pr" "$WORKTREE_DIR" "$REPO" "$ISSUE_NUM" "$LABELS" "$ISSUE_TITLE")
```

### Step 4: Preserve `$LABELS` and `$ISSUE_TITLE` availability at recovery site

Both variables are already set earlier in the function:
- `ISSUE_TITLE` at line 455
- `LABELS` at line 456

Both are in scope at the recovery block (lines 2116+). No change needed — they persist through the function's lifetime.

### Step 5: Add a body-level recovery-class indicator

Add a one-line `Recovery class: <class>` note to the PR body title line (line 2139) to compensate for the title no longer carrying recovery provenance. The `### Recovery metadata` block already has this (`Recovery class: \`${RECOVERY_CLASS}\``), so this is already covered.

### Step 6: Update the wip() rescue commit message body to note the title derivation

In the rescue commit messages (lines 792 and 826), the commit body already notes `Auto-rescued by dispatch-lib dirty-worktree detection`. No change needed to commit messages — the PR title is set at `gh pr create` time, not in the commit.

## Files changed

| File | Change |
|------|--------|
| `skills/bundled/_shared/dispatch-lib.sh` | Add `_label_to_type` + `_derive_recovery_pr_title` helpers; replace two `_rescue_title` assignments in recovery block |

## Testing

1. **Unit: `_label_to_type` mapping** — verify each label input produces the expected type prefix. Can be tested by sourcing the function in a shell and asserting outputs.
2. **Unit: `_derive_recovery_pr_title` commit-pushed-no-pr** — in a test repo with a conventional-commit message on HEAD, verify the function returns that subject.
3. **Unit: `_derive_recovery_pr_title` dirty-worktree with plan** — create a plan file with H1, verify the function constructs the conventional-commit title from it.
4. **Unit: `_derive_recovery_pr_title` dirty-worktree without plan** — verify the function falls back to issue title with type prefix.
5. **Integration: existing test-dispatch-lib.sh** — check if existing dispatch-lib tests need updates for the new title format. The `mika-rescue-commit-err` anchor used in test extraction is unchanged (commit messages aren't modified, only PR titles).

## Risks

- **`$LABELS` or `$ISSUE_TITLE` empty at recovery site:** Both are set unconditionally from `ISSUE_JSON` early in the handler. If `ISSUE_JSON` is missing (shouldn't happen — handler exits before reaching recovery without it), the fallback chain degrades to `chore: (mika#NNNN)` which is still better than the current titles.
- **Plan H1 is multi-line or contains shell-unsafe characters:** `head -5 | grep -m1 '^# '` limits to the first H1 within the first 5 lines. `sed 's/^# //'` strips the prefix. Shell quoting of the result via `"$(...)"` handles most special characters; the `gh pr create --title` accepts quoted strings.
- **Plan H1 already has conventional-commit format but with wrong type:** The function detects and passes through existing conventional-commit prefixes without re-wrapping. This respects the plan author's intent.

## Non-goals

- Changing the rescue commit message subjects (`wip(...)` prefix) — these are internal-only markers.
- Retroactively renaming existing recovery PRs — operator can batch-rename if desired.
- Changing the `## Recovery metadata` PR body format — already correct per AC3.

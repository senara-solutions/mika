# Plan: fix(dispatch-lib): `gh pr create` uses bare `$REPO` instead of `senara-solutions/$REPO`

**Issue:** mika#1643
**Type:** Bug fix
**Scope:** `skills/bundled/_shared/dispatch-lib.sh` — 4 call sites

## Problem

`dispatch-lib.sh` has 4 `gh` CLI call sites that pass bare `$REPO` (e.g., `mika`) to `--repo` instead of the required `senara-solutions/$REPO` (OWNER/REPO format). The `gh` CLI rejects the bare form with *"expected the '[HOST/]OWNER/REPO' format"*, crashing the auto-PR-create path. Pilot work (commits) lands safely, but dispatch-lib falls back to the wip-rescue draft path, requiring manual operator un-drafting.

## Requirements

1. Fix 4 bare-`$REPO` call sites to use `senara-solutions/$REPO`.
2. Add a structural regression test ensuring no future bare-`$REPO` usage in `gh` call sites.
3. No functional change to the wip-rescue safety net — it remains as-is.

## Implementation

### Step 1 — Fix the 4 bare-`$REPO` call sites

**File:** `skills/bundled/_shared/dispatch-lib.sh`

| Line | Current | Fixed |
|------|---------|-------|
| ~1035 | `gh pr list --repo "$REPO"` | `gh pr list --repo "senara-solutions/$REPO"` |
| ~1058 | `gh pr create --repo "$REPO"` | `gh pr create --repo "senara-solutions/$REPO"` |
| ~1064 | `gh pr list --repo "$REPO"` | `gh pr list --repo "senara-solutions/$REPO"` |
| ~1075 | `gh pr create --repo ${REPO}` (error message) | `gh pr create --repo senara-solutions/${REPO}` (error message) |

Each change is a single-token prefix insertion. No logic change.

### Step 2 — Add structural regression test

**File:** `skills/bundled/_shared/test-dispatch-lib.sh`

Append a new test section that greps `dispatch-lib.sh` for bare `$REPO` usage in `gh` call sites. The assertion:

```bash
# No bare $REPO in gh --repo arguments (mika#1643)
BARE_REPO_HITS=$(grep -n 'gh.*--repo[[:space:]]*"\$REPO"' "$DISPATCH_LIB" | grep -v 'senara-solutions/' || true)
# Also check unquoted ${REPO} in --repo (error message text)
BARE_REPO_HITS_UNQUOTED=$(grep -n 'gh.*--repo[[:space:]]*\${REPO}' "$DISPATCH_LIB" | grep -v 'senara-solutions/' || true)
```

If either returns non-empty, the test fails. This catches future additions that forget the `senara-solutions/` prefix.

## Verification contract

1. **Structural grep (pre-merge):** `grep -c 'gh.*--repo[[:space:]]*"\$REPO"' dispatch-lib.sh | grep -v senara-solutions` returns 0 matches.
2. **Test suite:** `bash skills/bundled/_shared/test-dispatch-lib.sh` passes (exit 0), including the new regression test.
3. **No unintended changes:** `git diff --stat` shows exactly 2 files changed: `dispatch-lib.sh` and `test-dispatch-lib.sh`.

## Definition of Done

- All 4 call sites fixed.
- Regression test added and passing.
- PR opened with `Closes #1643`.

## Acceptance criteria

- AC1 — All four buggy sites in `skills/bundled/_shared/dispatch-lib.sh` use `senara-solutions/$REPO` (not bare `$REPO`) in their `--repo` argument.
- AC2 — Structural assertion test in `skills/bundled/_shared/test-dispatch-lib.sh`: `! grep -n 'gh.*--repo[[:space:]]*"\$REPO"[^/]' skills/bundled/_shared/dispatch-lib.sh` exits 0 (no bare-$REPO usage in gh call sites). Covers regression prevention.
- AC3 — A pilot session that reaches the `gh pr create` auto-create path successfully creates the PR without triggering the wip-rescue path.

## Risk assessment

**Low.** Mechanical 4-line prefix insertion. No logic, no control-flow change, no new dependencies. The wip-rescue safety net remains untouched as fallback.

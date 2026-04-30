#!/bin/bash
# Verify dev-pilot and dev-groom dispatch handlers are structurally symmetric
# — identical except for the entry slash command (R5 of mika#893: factorize
# dev-groom and dev-pilot).
#
# The refactor's load-bearing invariant is: the ONLY meaningful difference
# between dev-pilot and dev-groom is the Claude Code command claude-pilot
# enters with. All other plumbing — slug derivation, worktree creation, env
# scrubbing, callback delivery — is shared via _shared/dispatch-lib.sh.
#
# Approach: normalize both handlers (strip comments + blank lines), replace
# the entry-command argument with a placeholder, diff. Empty diff = symmetric.
# Any other difference is a structural drift that violates the refactor's
# invariant.
#
# Why structural assertion (not a runtime dry-run): the dispatch-lib's
# dry_run path runs AFTER _set_up_worktree, which requires a real GitHub
# issue to exist (gh issue view). A runtime symmetry test would create
# real worktrees + require GitHub network + couple this test to the issue
# tracker. Structural test runs offline, deterministic, fast.
#
# Usage: bash scripts/test-dispatch-symmetry.sh
# Exit codes: 0 = symmetric, 1 = drift detected, 2 = setup error
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DEV_PILOT_HANDLER="$REPO_ROOT/skills/bundled/dev-pilot/handlers/run.sh"
DEV_GROOM_HANDLER="$REPO_ROOT/skills/bundled/dev-groom/handlers/run.sh"
SHARED_LIB="$REPO_ROOT/skills/bundled/_shared/dispatch-lib.sh"

[ -r "$DEV_PILOT_HANDLER" ] || { echo "ERROR: $DEV_PILOT_HANDLER not readable" >&2; exit 2; }
[ -r "$DEV_GROOM_HANDLER" ] || { echo "ERROR: $DEV_GROOM_HANDLER not readable" >&2; exit 2; }
[ -r "$SHARED_LIB" ] || { echo "ERROR: $SHARED_LIB not readable (refactor invariant: both handlers depend on shared lib)" >&2; exit 2; }

# Normalize: strip comment-only lines + blank lines + trailing whitespace,
# then replace the entry-command string with a placeholder so the diff
# ignores the one legitimate divergence between the wrappers.
normalize() {
    sed -E \
        -e 's/[[:space:]]+$//' \
        -e 's|"/mika-groom-ticket"|"<<ENTRY_COMMAND>>"|g' \
        -e 's|"/mika"|"<<ENTRY_COMMAND>>"|g' \
        "$1" \
        | grep -vE '^[[:space:]]*(#|$)'
}

PILOT_NORM=$(normalize "$DEV_PILOT_HANDLER")
GROOM_NORM=$(normalize "$DEV_GROOM_HANDLER")

if [ "$PILOT_NORM" = "$GROOM_NORM" ]; then
    echo "OK: dispatch handlers are structurally symmetric (entry command is sole legitimate divergence)"
    PILOT_CMD=$(grep -oE '"/mika[^"]*"' "$DEV_PILOT_HANDLER" | head -1)
    GROOM_CMD=$(grep -oE '"/mika[^"]*"' "$DEV_GROOM_HANDLER" | head -1)
    echo "  dev-pilot entry command: $PILOT_CMD"
    echo "  dev-groom entry command: $GROOM_CMD"
    exit 0
fi

echo "FAIL: dev-pilot and dev-groom handlers diverge beyond the entry command" >&2
echo "" >&2
echo "Diff (after normalization — comments/blank lines stripped, entry command placeholdered):" >&2
diff <(printf '%s\n' "$PILOT_NORM") <(printf '%s\n' "$GROOM_NORM") >&2 || true
echo "" >&2
echo "The refactor's invariant (mika#893 R3): adding a hypothetical third sibling skill" >&2
echo "should require only a new manifest + system_prompt + an entry-command lookup row." >&2
echo "Any structural divergence between dev-pilot and dev-groom violates that invariant." >&2
exit 1

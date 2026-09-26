#!/bin/bash
# Test suite for _stamp_pr_origin (mika#2026).
#
# The origin of a PR is a fact stamped by its producer at the moment of
# production. This suite pins the producer half: dispatch-lib labels the PR in
# shell, retries once behind an idempotent `gh label create` for repos that have
# no label-sync, and — critically — NEVER aborts a dispatch when the stamp fails.
#
# `gh` is stubbed on PATH; every call is journalled to $GH_LOG so the assertions
# can read the exact argv the function produced.
#
# Source isolation audit: dispatch-lib.sh has no top-level imperative code —
# all `set -e`, `trap`, and env var references are inside function bodies.
# Safe to source directly without a guard variable.
#
# Run: bash skills/bundled/_shared/tests/test_stamp_pr_origin.sh
# Expected: all assertions pass, exit 0.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"

# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

PASS=0
FAIL=0

assert_eq() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    expected: '$expected'"
        echo "    actual:   '$actual'"
    fi
}

assert_contains() {
    local label="$1" haystack="$2" needle="$3"
    if grep -qF -- "$needle" <<<"$haystack"; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    missing: '$needle'"
        echo "    in:      '$haystack'"
    fi
}

assert_not_contains() {
    local label="$1" haystack="$2" needle="$3"
    if grep -qF -- "$needle" <<<"$haystack"; then
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    unexpectedly present: '$needle'"
    else
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    fi
}

STUB_DIR=$(mktemp -d)
export GH_LOG="$STUB_DIR/gh.log"
trap 'rm -rf "$STUB_DIR"' EXIT

# `gh` stub. Behaviour is driven by $GH_MODE:
#   ok            — everything succeeds
#   needs-label   — `pr edit` fails until `label create` has run, then succeeds
#   always-fails  — every call fails (network down, no write scope, …)
cat > "$STUB_DIR/gh" <<'STUB'
#!/bin/bash
printf '%s\n' "$*" >> "$GH_LOG"
# `pr view --json labels` answers the claim check: $GH_EXISTING_LABELS, one per line.
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
    [ "${GH_MODE:-ok}" = "always-fails" ] && exit 1
    printf '%s' "${GH_EXISTING_LABELS:-}"
    exit 0
fi
case "${GH_MODE:-ok}" in
    ok) exit 0 ;;
    needs-label)
        if [ "$1" = "label" ] && [ "$2" = "create" ]; then
            touch "$(dirname "$GH_LOG")/label-created"; exit 0
        fi
        [ -f "$(dirname "$GH_LOG")/label-created" ] && exit 0
        exit 1 ;;
    always-fails) exit 1 ;;
esac
STUB
chmod +x "$STUB_DIR/gh"
export PATH="$STUB_DIR:$PATH"

MIKA_PR_ORIGIN_EPOCH_FILE="$STUB_DIR/pr-origin-epoch"

reset_stub() {
    : > "$GH_LOG"
    rm -f "$STUB_DIR/label-created" "$MIKA_PR_ORIGIN_EPOCH_FILE"
    unset GH_EXISTING_LABELS
}

echo "=== _stamp_pr_origin (mika#2026) ==="

# ── 1. Nominal: one `pr edit`, correct label, no `label create` needed ────────
echo "-- nominal stamp --"
reset_stub
export GH_MODE=ok
rc=0
_stamp_pr_origin "mika" "https://github.com/senara-solutions/mika/pull/2026" || rc=$?
log=$(cat "$GH_LOG")
assert_eq "returns 0" "0" "$rc"
assert_contains "edits the PR with origin:loop" "$log" \
    "pr edit https://github.com/senara-solutions/mika/pull/2026 --repo senara-solutions/mika --add-label origin:loop"
assert_not_contains "no label create needed on the happy path" "$log" "label create"
assert_eq "two gh calls: claim check, then edit" "2" "$(wc -l < "$GH_LOG")"
assert_contains "checks for an existing claim first" "$log" "pr view"

# ── 2. Default origin is `loop`, and an explicit origin is honoured ───────────
echo "-- explicit origin --"
reset_stub
_stamp_pr_origin "mika-cloud" "202" "manual" || true
assert_contains "applies origin:manual when asked" "$(cat "$GH_LOG")" "--add-label origin:manual"
assert_contains "targets the repo it was given" "$(cat "$GH_LOG")" "--repo senara-solutions/mika-cloud"

# ── 3. Label missing on this repo: create, then retry exactly once ────────────
# dispatch-lib also targets mika-cloud / mika-skills / mika-platform, none of
# which run mika's label-sync workflow — the label will not exist there.
echo "-- label absent on target repo --"
reset_stub
export GH_MODE=needs-label
rc=0
_stamp_pr_origin "mika-skills" "42" || rc=$?
log=$(cat "$GH_LOG")
assert_eq "recovers and returns 0" "0" "$rc"
assert_contains "creates the missing label" "$log" "label create origin:loop --repo senara-solutions/mika-skills"
assert_eq "4 calls: view, edit, create, edit" "4" "$(wc -l < "$GH_LOG")"

# ── 4. Total failure: named on stderr, rc 1, and NOTHING else ────────────────
# The caller invokes with `|| true`. A missing marker costs one `unknown` row in
# a report; it must never cost a dispatch.
echo "-- gh unavailable --"
reset_stub
export GH_MODE=always-fails
rc=0
err=$(_stamp_pr_origin "mika" "2026" 2>&1 >/dev/null) || rc=$?
assert_eq "returns 1" "1" "$rc"
assert_contains "names the failure" "$err" "pr_origin.stamp_failed"
assert_contains "says what the consequence is" "$err" "unclassified"
assert_eq "tries view, edit, create, edit — then stops" "4" "$(wc -l < "$GH_LOG")"

# ── 5. Empty arguments are a silent no-op, never an error ────────────────────
echo "-- missing arguments --"
reset_stub
export GH_MODE=always-fails
rc=0; _stamp_pr_origin "" "2026" || rc=$?
assert_eq "empty repo: rc 0" "0" "$rc"
rc=0; _stamp_pr_origin "mika" "" || rc=$?
assert_eq "empty pr_ref: rc 0" "0" "$rc"
assert_eq "no gh call made" "0" "$(wc -l < "$GH_LOG")"

# ── 6. The vocabulary is closed, and matches what the reader understands ─────
# A value the producer stamps but the reader has no bucket for would disappear
# into "not-loop" or "unknown" without a word — the exact silence this ticket
# exists to end.
echo "-- closed vocabulary --"
reset_stub
export GH_MODE=ok
rc=0
err=$(_stamp_pr_origin "mika" "2026" "robot" 2>&1 >/dev/null) || rc=$?
assert_eq "an unknown origin is refused" "1" "$rc"
assert_contains "and named" "$err" "pr_origin.unknown_value"
assert_eq "nothing was stamped" "0" "$(wc -l < "$GH_LOG")"

for v in loop spawn manual; do
    reset_stub
    rc=0; _stamp_pr_origin "mika" "2026" "$v" || rc=$?
    assert_eq "'$v' is accepted" "0" "$rc"
done

# Producer and reader must agree. Parse the reader's own buckets rather than
# restating them here: a list written twice is a list that drifts.
READER="$SCRIPT_DIR/../../../../scripts/pr-origin-report.sh"
reader_line=$(grep -m1 '^VALUES = {' "$READER" || true)
if [ -z "$reader_line" ]; then
    FAIL=$((FAIL + 1))
    echo "  ✗ could not find the reader's VALUES set in $READER — the drift check is blind"
fi
reader_values=$(grep -o '"[a-z]*"' <<<"$reader_line" | tr -d '"' | sort -u | tr '\n' ' ')
producer_values=$(printf '%s\n' "${MIKA_PR_ORIGIN_VALUES[@]}" | sort -u | tr '\n' ' ')
assert_eq "producer vocabulary == reader buckets" "$producer_values" "$reader_values"

# ── 7. An origin someone else asserted is never overwritten ─────────────────
# Two of the three callsites reach a PR dispatch-lib DISCOVERED on the branch, not
# one it created — and the orchestrator derives branch names with the same script
# the loop uses, so a by-hand PR can already be sitting there.
echo "-- an already-claimed PR --"
reset_stub
export GH_MODE=ok
export GH_EXISTING_LABELS="origin:manual"
rc=0
err=$(_stamp_pr_origin "mika" "2026" 2>&1 >/dev/null) || rc=$?
assert_eq "does not treat it as a failure" "0" "$rc"
assert_contains "reports the existing claim" "$err" "pr_origin.already_claimed"
assert_contains "and names it" "$err" "origin:manual"
assert_eq "no edit attempted" "1" "$(wc -l < "$GH_LOG")"

reset_stub
export GH_EXISTING_LABELS="origin:loop"
rc=0; _stamp_pr_origin "mika" "2026" || rc=$?
assert_eq "re-stamping the same origin is a no-op" "0" "$rc"
assert_eq "and costs one call, not two" "1" "$(wc -l < "$GH_LOG")"
unset GH_EXISTING_LABELS

# ── 8. The producer records its own first stamp ─────────────────────────────
# The reader needs a cut-off, and it must be a fact someone recorded — an mtime
# would track the last daemon restart (seed_support_dirs rewrites the installed
# dispatch-lib.sh unconditionally on every start), walking the cut-off forward all
# day and quietly re-opening the blind window after every bounce.
echo "-- first-stamp record --"
reset_stub
export GH_MODE=ok
_stamp_pr_origin "mika" "2026" || true
if [ -s "$MIKA_PR_ORIGIN_EPOCH_FILE" ]; then
    PASS=$((PASS + 1)); echo "  ✓ a successful stamp records the epoch"
else
    FAIL=$((FAIL + 1)); echo "  ✗ no epoch recorded"
fi
first=$(cat "$MIKA_PR_ORIGIN_EPOCH_FILE")
assert_contains "recorded as an ISO-8601 UTC instant" "$first" "T"
assert_contains "and it ends in Z" "$first" "Z"

# Use a sentinel rather than a second live write: two writes inside the same
# second would compare equal even if the record were being refreshed every time,
# and a cut-off that walks forward silently re-opens the blind window.
printf '2020-01-01T00:00:00Z\n' > "$MIKA_PR_ORIGIN_EPOCH_FILE"
_stamp_pr_origin "mika" "2027" || true
assert_eq "a later stamp never moves the cut-off forward" \
    "2020-01-01T00:00:00Z" "$(cat "$MIKA_PR_ORIGIN_EPOCH_FILE")"

# A failed stamp must not record an epoch: an undetermined epoch makes the reader
# classify nothing, which is the safe direction.
reset_stub
export GH_MODE=always-fails
_stamp_pr_origin "mika" "2026" 2>/dev/null || true
if [ -f "$MIKA_PR_ORIGIN_EPOCH_FILE" ]; then
    FAIL=$((FAIL + 1)); echo "  ✗ a failed stamp recorded an epoch"
else
    PASS=$((PASS + 1)); echo "  ✓ a failed stamp records nothing"
fi

# ── 9. Every gh call is bounded ─────────────────────────────────────────────
# One callsite is the crash/cancel exit trap, whose job is to get RESULT back to
# mika-dev. A hanging GitHub API must not delay the news that a dispatch died.
echo "-- bounded network calls --"
fn_body=$(awk '/^_stamp_pr_origin\(\) \{/,/^\}/' "$DISPATCH_LIB")
n_gh=$(grep -c '[^_]gh pr \|[^_]gh label ' <<<"$fn_body" || true)
n_timeout=$(grep -c 'timeout [0-9]* gh ' <<<"$fn_body" || true)
assert_eq "every gh invocation carries a timeout" "$n_gh" "$n_timeout"

# ── 10. Every callsite in dispatch-lib is fail-open ──────────────────────────
# Structural assertion: this is the property that keeps a reporting nicety from
# becoming a dispatch-breaking dependency. Grep every callsite, not just the
# ones this suite happens to remember.
echo "-- callsites are fail-open --"
callsites=$(grep -n '_stamp_pr_origin "' "$DISPATCH_LIB" | grep -v '^[0-9]*: *#' || true)
n_callsites=$(printf '%s\n' "$callsites" | grep -c . || true)
n_failopen=$(printf '%s\n' "$callsites" | grep -c '|| true' || true)
assert_eq "all callsites end in '|| true'" "$n_callsites" "$n_failopen"
assert_eq "the three PR-production sites are wired" "3" "$n_callsites"

echo
echo "PASS: $PASS  FAIL: $FAIL"
[ "$FAIL" -eq 0 ] || exit 1

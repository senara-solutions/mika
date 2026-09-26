#!/usr/bin/env bash
# The fixture bodies below are single-quoted ON PURPOSE: they are literal shell
# text written INTO a probe file, so `$x` must stay unexpanded (file-wide).
# shellcheck disable=SC2016
#
# Anti-vacuity harness for scripts/verify-no-sigpipe-grep.sh (mika#2055).
#
# The guard is only worth shipping if it can FAIL. This suite proves it:
#   - it is clean on the real (swept) tree;
#   - it goes RED on a deliberately-reintroduced bad pattern (the negative
#     control the ticket demands);
#   - the here-string remedy is accepted;
#   - the ticket-cited escape hatch suppresses, and a BARE marker does not.
#
# "Delete the thing the test protects; confirm the test goes red." — the
# reproduction that separates a real guard from a vacuous one
# (docs/solutions/best-practices/structural-guard-fails-open-parser-fixture-harness.md).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GUARD="$REPO_ROOT/scripts/verify-no-sigpipe-grep.sh"

PASS=0
FAIL=0

# Run the guard against $1; assert its exit code equals $2. $3 = case name.
assert_exit() {
    local root="$1" want="$2" name="$3"
    local got=0
    bash "$GUARD" "$root" >/dev/null 2>&1 || got=$?
    if [ "$got" -eq "$want" ]; then
        echo "PASS: $name (exit $got)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (wanted exit $want, got $got)"
        FAIL=$((FAIL + 1))
    fi
}

# A throwaway fixture tree with one shell file whose body is $1.
make_fixture() {
    local body="$1"
    local dir
    dir="$(mktemp -d)"
    {
        echo '#!/usr/bin/env bash'
        echo 'set -euo pipefail'
        echo 'x="some haystack"'
        printf '%s\n' "$body"
    } > "$dir/probe.sh"
    echo "$dir"
}

# 1. The real, swept tree is clean.
assert_exit "$REPO_ROOT" 0 "swept repo tree is clean"

# 2. Negative control (anti-vacuity): the bad pattern goes RED.
d="$(make_fixture 'if echo "$x" | grep -qF "hay"; then :; fi')"
assert_exit "$d" 1 "reintroduced echo | grep -qF is rejected"
rm -rf "$d"

# 2b. printf producer variant is also caught.
d="$(make_fixture "if printf '%s' \"\$x\" | grep -qE 'ha.' ; then :; fi")"
assert_exit "$d" 1 "reintroduced printf | grep -qE is rejected"
rm -rf "$d"

# 3. The here-string remedy is accepted.
d="$(make_fixture 'if grep -qF -- "hay" <<<"$x"; then :; fi')"
assert_exit "$d" 0 "here-string remedy passes"
rm -rf "$d"

# 3b. A non-short-circuiting grep (no -q) is not the target class.
d="$(make_fixture 'echo "$x" | grep -oF "hay" | head -1')"
assert_exit "$d" 0 "grep without -q is not flagged"
rm -rf "$d"

# 4. Escape hatch: a ticket-cited marker suppresses.
d="$(make_fixture 'if echo "$x" | grep -qF "hay"; then :; fi  # sigpipe-safe: #2055')"
assert_exit "$d" 0 "ticket-cited escape marker suppresses"
rm -rf "$d"

# 5. A BARE marker (no #<digits> citation) does NOT suppress.
d="$(make_fixture 'if echo "$x" | grep -qF "hay"; then :; fi  # sigpipe-safe')"
assert_exit "$d" 1 "bare escape marker does not suppress"
rm -rf "$d"

echo ""
echo "no-sigpipe-grep anti-vacuity: $PASS passed, $FAIL failed."
[ "$FAIL" -eq 0 ]

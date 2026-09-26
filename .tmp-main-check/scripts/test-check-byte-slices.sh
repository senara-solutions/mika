#!/usr/bin/env bash
#
# Anti-vacuity harness for scripts/check-byte-slices.sh (mika#2103).
#
# The founding incident is precisely a guard that could not fail on the defect
# it was supposed to catch: check-byte-slices.sh knew the *slice* spelling of
# the byte-boundary bug (mika#764) and not the `String::truncate` spelling, so
# it stayed green through 26 production panics. A lint nobody has watched go
# red is a decoration.
#
# This suite proves the guard bites, one case per pattern, plus the allowlist
# in both directions.
#
# "Delete the thing the test protects; confirm the test goes red."

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GUARD="$REPO_ROOT/scripts/check-byte-slices.sh"

PASS=0
FAIL=0

# Run the guard against scan root $1; assert its exit equals $2. $3 = case name.
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

# A throwaway fixture tree holding one .rs file whose body is $1.
make_fixture() {
    local dir
    dir="$(mktemp -d)"
    printf '%s\n' "$1" > "$dir/probe.rs"
    echo "$dir"
}

# 1. The real, swept tree is clean.
assert_exit "$REPO_ROOT/crates" 0 "swept repo tree is clean"

# 2. NEGATIVE CONTROL — the exact mika#2103 defect. This is the case the guard
#    was blind to for 26 panics; if it ever goes green again, the class is
#    unprotected regardless of what the other patterns do.
d="$(make_fixture '    output.content.truncate(MAX_OUTPUT_LEN);')"
assert_exit "$d" 1 "Pattern C: the mika#2103 String::truncate defect is rejected"
rm -rf "$d"

# 2b. The guard must not depend on the variable being *named* something it
#     recognises — that name-scoping is the mika#2103 root cause, and a guard
#     that only sees `content` would let the next site through under any other
#     name. This case is the whole point of Pattern C being name-blind.
d="$(make_fixture '    whatever_unanticipated_name.truncate(BUDGET);')"
assert_exit "$d" 1 "Pattern C is name-blind (no variable allowlist to slip past)"
rm -rf "$d"

# 3. Pattern D — the mika#2103 logs.rs shape: a computed byte offset.
d="$(make_fixture '    let (a, b) = input.split_at(input.len() - 1);')"
assert_exit "$d" 1 "Pattern D: split_at at a computed byte offset is rejected"
rm -rf "$d"

d="$(make_fixture '    let tail = s.split_off(s.len() - 4);')"
assert_exit "$d" 1 "Pattern D: split_off at a computed byte offset is rejected"
rm -rf "$d"

# 4. Pattern E — String::insert, recognised by its char-literal argument.
d="$(make_fixture "    buf.insert(idx, 'x');")"
assert_exit "$d" 1 "Pattern E: String::insert at a byte offset is rejected"
rm -rf "$d"

# 4b. ...and the map/set `insert` it must NOT be confused with, or the guard
#     drowns in false positives and gets allowlisted into uselessness.
d="$(make_fixture '    open_pr_set.insert(606);')"
assert_exit "$d" 0 "Pattern E does not fire on HashSet::insert"
rm -rf "$d"

# 5. Pattern A / Pattern B — the original mika#764 slice spellings still bite.
d="$(make_fixture '    let head = &body[..body.len().min(200)];')"
assert_exit "$d" 1 "Pattern A: slice via .len().min() is rejected"
rm -rf "$d"

d="$(make_fixture '    let head = &content[..500];')"
assert_exit "$d" 1 "Pattern B: slice at a literal byte offset is rejected"
rm -rf "$d"

# 6. The canonical remedy passes.
d="$(make_fixture '    let head = mika_common::text::safe_truncate(&body, 200);')"
assert_exit "$d" 0 "safe_truncate remedy passes"
rm -rf "$d"

# 7. The allowlist suppresses...
d="$(make_fixture '    events.truncate(limit); // safe-byte-slice: Vec, no char boundary')"
assert_exit "$d" 0 "// safe-byte-slice: annotation suppresses"
rm -rf "$d"

# 7b. ...and OpenOptions::truncate(bool) is excluded by argument shape, with no
#     annotation needed — a bool is never a byte offset.
d="$(make_fixture '    OpenOptions::new().truncate(true).open(path)?;')"
assert_exit "$d" 0 "OpenOptions::truncate(true) is not flagged"
rm -rf "$d"

echo ""
echo "check-byte-slices anti-vacuity: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1

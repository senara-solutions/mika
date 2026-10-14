#!/usr/bin/env bash
#
# Anti-vacuity harness for scripts/check-eval-ignored-flags.sh (mika#2683).
#
# House constraint, written into ci.yml since mika#2103: *a guard nobody has
# watched go red is a decoration*. The founding incident there is exactly a lint
# that could not fail on the defect it existed to catch, and stayed green
# through 26 production panics.
#
# This suite proves the guard bites on the historical defect itself, on both
# orderings of the pair, at each of the three perimeter entries, on the
# allowlist in both directions, and on all three anti-vacuity floors — and,
# just as load-bearing, that it does NOT bite on either flag used alone, which
# is the entire reason the predicate is co-occurrence and not presence.
#
# "Delete the thing the test protects; confirm the test goes red."
#
# SELF-REFERENCE: this file lives in `scripts/`, inside the guard's own
# perimeter, and its business is to write faulty invocations. Three strings are
# therefore composed at runtime and never typed literally — the two flags, and
# the name of cargo's test command itself:
#
#   - the FLAGS, so no live line here carries both and the real-tree scan stays
#     green. Keep this shape rather than allowlisting the file: a file-level
#     exemption would also hide a real regression written here.
#
#   - the COMMAND NAME, so none of the fixture lines below counts as a real
#     invocation site for R2's second floor. Measured before this was done: the
#     guard reported 19 sites on the real tree where 4 exist, and a tree holding
#     nothing but this harness would have cleared the floor. An anti-vacuity
#     floor fed by the instrument that tests it measures nothing.
#
# Same discipline as the guard's own FLAG_BARE/FLAG_INCLUDE, and as the
# runtime-composed needle in crates/mika-agent/src/canonical_tokens.rs.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GUARD="$REPO_ROOT/scripts/check-eval-ignored-flags.sh"

IGN="ignored"
BARE="--$IGN"
INCL="--include-$IGN"

TEST_CMD="test"
CT="cargo $TEST_CMD"

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

# Assert the guard's output on root $1 contains $2. $3 = case name.
#
# The output is captured and matched with `[[ == * ]]`, not piped into
# `grep -q`: under `pipefail` a failing producer poisons the pipeline's status
# even when grep matches, and `echo | grep -q` is itself refused by the sigpipe
# lint (mika#2055).
assert_says() {
    local root="$1" want="$2" name="$3"
    local out
    out="$(bash "$GUARD" "$root" 2>&1 || true)"
    if [[ "$out" == *"$want"* ]]; then
        echo "PASS: $name"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (output did not mention '$want')"
        FAIL=$((FAIL + 1))
    fi
}

# A throwaway tree holding one perimeter file at relative path $1 whose body
# is $2. Echoes the tree's root.
make_fixture() {
    local dir
    dir="$(mktemp -d)"
    mkdir -p "$dir/$(dirname "$1")"
    printf '%s\n' "$2" > "$dir/$1"
    echo "$dir"
}

# Add another perimeter file to an existing fixture tree.
add_file() {
    mkdir -p "$1/$(dirname "$2")"
    printf '%s\n' "$3" > "$1/$2"
}

# ── 1. The real, swept tree is clean. If this case ever goes red, the guard is
#      accusing a legitimate invocation and the fix is in the guard.
assert_exit "$REPO_ROOT" 0 "real repo perimeter is clean"

# ── 2. R1: the regression this guard exists to catch, byte-for-byte the line
#      eval-calibration.yml carried for 17 red runs — `|| true` included, since
#      that is what made the failure unreadable rather than loud.
d="$(make_fixture '.github/workflows/probe.yml' \
    "          $CT -p mika-agent --test eval -- $BARE $INCL 2>&1 || true")"
assert_exit "$d" 1 "R1: the historical eval-calibration line is rejected"
assert_says "$d" "$BARE and $INCL" "R1: the message names both flags in full"
assert_says "$d" "probe.yml:1" "R1: the message names the site"
rm -rf "$d"

# ── 3. R1 does not depend on the order the pair is written in. A guard that
#      only catches one ordering is caught out by the first reformat.
d="$(make_fixture '.github/workflows/probe.yml' \
    "          $CT -p mika-agent --test eval -- $INCL $BARE")"
assert_exit "$d" 1 "R1: the reversed ordering is rejected identically"
rm -rf "$d"

# ── 4. THE CASE THAT DEFINES THE PREDICATE. The bare flag alone is the shape
#      this repository ships at three invocation sites, including the fixed
#      eval-calibration line. A rule on *presence* rather than *co-occurrence*
#      would accuse it, be disarmed, and let the pair back in.
d="$(make_fixture '.github/workflows/probe.yml' \
    "          $CT -p mika-agent --test eval -- $BARE 2>&1")"
assert_exit "$d" 0 "R1 admits the bare flag alone — the compliant shape"
rm -rf "$d"

# ── 5. ...and the include form alone is equally legitimate: the whole suite
#      plus the ignored ones. Accusing it is the naive predicate the guard's
#      header refuses by name.
d="$(make_fixture '.github/workflows/probe.yml' \
    "          $CT -p mika-agent --test eval -- $INCL 2>&1")"
assert_exit "$d" 0 "R1 admits the include form alone (AC: no naive predicate)"
rm -rf "$d"

# ── 6. Both flags in the same FILE but at two different invocation sites is not
#      a violation — neither cargo process receives a contradiction. The subject
#      is the site, never the file.
d="$(make_fixture 'Makefile' \
    "eval-only:
	$CT -p mika-agent --test eval -- $BARE

eval-all:
	$CT -p mika-agent --test eval -- $INCL")"
assert_exit "$d" 0 "R1 is per invocation site, not per file"
rm -rf "$d"

# ── 7. A comment carrying the pair executes nothing. Without this skip the
#      guard denounces its own header and the ticket's own documentation.
d="$(make_fixture 'scripts/probe.sh' \
    "# historical defect: $CT -p mika-agent --test eval -- $BARE $INCL
$CT -p mika-agent --test eval -- $BARE")"
assert_exit "$d" 0 "a commented invocation is a mention, not an instruction"
rm -rf "$d"

# ── 8. The pair outside a cargo-test invocation is out of R1's population —
#      another tool's flags are its own business. The fixture carries a clean
#      invocation too, so the floors of R2 are satisfied and exit 0 means "R1
#      found nothing", not "the scan was vacuous".
d="$(make_fixture 'scripts/probe.sh' \
    "some-other-runner -- $BARE $INCL
$CT -p mika-agent --test eval -- $BARE")"
assert_exit "$d" 0 "R1 only looks at cargo-test invocations"
rm -rf "$d"

# ── 9. The perimeter covers the Makefile...
d="$(make_fixture 'Makefile' \
    "calibrate:
	$CT -p mika-agent --test eval -- $BARE $INCL")"
assert_exit "$d" 1 "the Makefile is inside the perimeter"
rm -rf "$d"

# ── 10. ...and scripts/*.sh, which is where the two guards of this family and
#        generate-openapi.sh all live.
d="$(make_fixture 'scripts/probe.sh' \
    "$CT -p mika-agent write_agent_openapi_yaml -- $BARE $INCL")"
assert_exit "$d" 1 "scripts/*.sh is inside the perimeter"
rm -rf "$d"

# ── 11. ...and .yaml as well as .yml, because the extension a future workflow
#        picks is not a property anyone will remember to check.
d="$(make_fixture '.github/workflows/probe.yaml' \
    "          $CT -p mika-agent --test eval -- $BARE $INCL")"
assert_exit "$d" 1 "both YAML extensions are inside the perimeter"
rm -rf "$d"

# ── 12. The allowlist suppresses a real violation...
d="$(make_fixture '.github/workflows/probe.yml' \
    "          $CT -p mika-agent --test eval -- $BARE $INCL")"
add_file "$d" 'scripts/eval-ignored-flags-allowlist.txt' \
    '.github/workflows/probe.yml:1  # fixture: proves suppression works at all'
assert_exit "$d" 0 "allowlist suppresses a listed violation"
rm -rf "$d"

# ── 13. ...and is compared in BOTH directions: an entry outliving its site
#        fails the build. One-way comparison is how an allowlist becomes the
#        junk drawer in which the next occurrence of the pair passes unnoticed.
d="$(make_fixture '.github/workflows/probe.yml' \
    "          $CT -p mika-agent --test eval -- $BARE")"
add_file "$d" 'scripts/eval-ignored-flags-allowlist.txt' \
    '.github/workflows/probe.yml:1  # stale'
assert_exit "$d" 1 "a stale allowlist entry fails the build"
assert_says "$d" "matches no R1 violation" "the stale-entry message names the reason"
rm -rf "$d"

# ── 14. R2, floor two: files were read, but no invocation site exists in them.
#        R1 can only fire at such a site, so finding none means the scan looked
#        in the wrong place — not that the tree is clean.
d="$(make_fixture 'scripts/probe.sh' \
    "cargo build --release
echo no invocation here")"
assert_exit "$d" 1 "a perimeter with no invocation site is refused"
assert_says "$d" "no cargo-test invocation" "the no-invocation message names the reason"
rm -rf "$d"

# ── 15. R2, floor three: an invocation site exists, but the flag family appears
#        nowhere. A tree where R1 has no possible subject cannot clear it.
d="$(make_fixture 'scripts/probe.sh' \
    "$CT -p mika-agent --lib -- --nocapture")"
assert_exit "$d" 1 "a perimeter where the flag family is absent is refused"
assert_says "$d" "appears anywhere" "the absent-family message names the reason"
rm -rf "$d"

# ── 16. R2, floor one: a scan whose perimeter does not exist at all. A scan
#        with nothing to scan is a vacuous pass, and a vacuous pass is the
#        decoration mika#2103 is about.
d="$(mktemp -d)"
assert_exit "$d" 1 "an empty perimeter is refused, not silently passed"
assert_says "$d" "vacuous pass" "the empty-perimeter message names the reason"
rm -rf "$d"

echo ""
echo "check-eval-ignored-flags anti-vacuity: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1

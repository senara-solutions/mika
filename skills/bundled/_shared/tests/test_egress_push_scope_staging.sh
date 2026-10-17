#!/bin/bash
# mika#2751: the PUSH PERIMETER of a spawn is really declared host-side, under
# the right mode, with the right content, and a failure to declare it degrades
# to the static bound of mika#2736 instead of killing the dispatch.
#
# WHY THIS SUITE EXISTS, next to `test_egress_push_bound.sh`. That suite drives
# the ADDON: it writes perimeter entries by hand and measures the verdict. It is
# therefore completely blind to whether anything in the dispatch ever writes
# one — a `_stage_pilot_egress_push_scope` that returned immediately would leave
# all 254 of its assertions green, and every pilot would push under the static
# bound alone while the suites claimed a tightening. This suite is the other
# half of that correspondence: the producer really produces, and what it
# produces is the shape the reader reads (`repo=senara-solutions/<repo>`).
#
# WHERE THE STAGING LIVES, because it decides the shape of case 3.
# `_stage_pilot_egress_push_scope` is called INSIDE `_run_pilot_sandboxed`,
# right after `_stage_pilot_egress_sentinel` — not in a dispatch preamble. That
# is what makes the perimeter per-SPAWN. Case 3 therefore drives the real entry
# point rather than the producer, so a perimeter hoisted to once-per-dispatch
# fails there and nowhere else.
#
# `_PILOT_EGRESS_PUSH_SCOPE_DIR` is assigned as a plain GLOBAL per case, never
# as a `VAR=x func` command prefix: in case 3 the producer that must see it is
# two frames down inside `_run_pilot_sandboxed`, where a prefix on this suite's
# own call would not reach. Same reasoning, same measured defect, as the
# sentinel staging suite.
#
# WHAT IS NOT HERE, so nobody reads an omission into it. The per-directory
# purge throttle of the registry reader is Python and lives with its reader:
# `test_egress_sentinel_enforcement.sh` CASE 13. The verdicts themselves
# (`push-scope-repo`, `scope-absent`, `scope-unreadable`, the switch matrix) are
# `test_egress_push_bound.sh` CASES 13-19. This file asserts the WRITE and
# nothing else.
#
# Run: bash skills/bundled/_shared/tests/test_egress_push_scope_staging.sh
# Expected: all assertions pass, exit 0. Needs no bwrap and no gh; case 5 skips
# by name when run as root (mode 0500 refuses nobody there).

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"

NONCE_RE='^[0-9a-f]{32}$'

PASS=0
FAIL=0
assert_eq() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    expected: '$expected'"; echo "    actual:   '$actual'"
    fi
}
assert_match() {
    local label="$1" re="$2" value="$3"
    if [[ "$value" =~ $re ]]; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    expected to match: '$re'"; echo "    actual:            '$value'"
    fi
}
assert_contains() {
    local label="$1" needle="$2" haystack="$3"
    if [[ "$haystack" == *"$needle"* ]]; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    expected to contain: '$needle'"; echo "    actual:              '$haystack'"
    fi
}

TMPROOT=$(mktemp -d "${TMPDIR:-/tmp}/mika2751-XXXXXX")
# mika#2049 — the composed handler is the CALLER's, always: bash keeps one
# handler per signal, so a trap installed inside the fake relay would silently
# replace this `rm -rf` and leak the temp tree.
trap 'if declare -F stop_fake_egress_relay >/dev/null 2>&1; then stop_fake_egress_relay; fi; rm -rf "$TMPROOT"' EXIT

CAPTURE="$TMPROOT/bwrap-argv"
export HOME="$TMPROOT/home"
WORKTREE_DIR="$TMPROOT/worktree"
mkdir -p "$HOME/.mika/data/pilot-transcripts" "$WORKTREE_DIR"

# shellcheck source=/dev/null
source "$DISPATCH_LIB"

bwrap() {
    printf '%s\0' "$@" > "$CAPTURE"
    return 0
}
# shellcheck source=skills/bundled/_shared/tests/lib-fake-egress-relay.sh
source "$SCRIPT_DIR/lib-fake-egress-relay.sh"
stub_serving_egress_relay "$TMPROOT"
_ensure_pilot_helper() { return 1; }
_PILOT_SANDBOX_SECRET_ALLOWLIST=()

# Every case gets its own registry, so a leftover entry can never make a count
# assertion read as a fresh declaration.
registry_for() {
    local d="$TMPROOT/scopes-$1"
    mkdir -p "$(dirname "$d")"
    printf '%s' "$d"
}
entries_in() { ls -1 "$1" 2>/dev/null | wc -l | tr -d ' '; }

echo ""
echo "CASE 1 — the dispatch repository is really declared, in the shape the addon reads"
echo "---------------------------------------------------------------------------------"
R1=$(registry_for case1)
_PILOT_EGRESS_PUSH_SCOPE_DIR="$R1"
REPO="mika"
_stage_pilot_egress_push_scope
rc1=$?
assert_eq "the staging returns 0" "0" "$rc1"
assert_match "the entry is named by a 32-hex nonce" "$NONCE_RE" "$_PILOT_EGRESS_PUSH_SCOPE_ENTRY"
assert_eq "exactly one perimeter is declared" "1" "$(entries_in "$R1")"
E1="$R1/$_PILOT_EGRESS_PUSH_SCOPE_ENTRY"
rc=0; [ -f "$E1" ] || rc=1
assert_eq "the memorized nonce IS the file on disk" "0" "$rc"
# The reader (`_push_scope_repos`) splits on lines and takes the `repo=` key.
# Owner-qualified, lowercase, no `.git` suffix: `_receive_pack_repo` strips
# `.git` on its side, so the two forms meet normalized.
assert_eq "the content is the owner-qualified repository, one line" \
    "repo=senara-solutions/mika" "$(cat "$E1")"
rc=0; grep -q '\.git' "$E1" && rc=1
assert_eq "no .git suffix — the reader strips it on its side, not here" "0" "$rc"

echo ""
echo "CASE 2 — one entry PER SPAWN, both valid at the same instant, host-only mode"
echo "----------------------------------------------------------------------------"
# This is why the registry is a DIRECTORY and not a file, exactly as for the
# sentinel registry: `mitmdump` is long-lived and shared, so two concurrent
# pilots must both be inside their own perimeter at the SAME instant. A single
# overwritten file would refuse the push of whichever spawn started first —
# a 403 on legitimate work, which is the one failure mode able to stop the loop.
REPO="mika-platform"
_stage_pilot_egress_push_scope
E2="$R1/$_PILOT_EGRESS_PUSH_SCOPE_ENTRY"
assert_eq "the second spawn declares a second entry, not an overwrite" "2" "$(entries_in "$R1")"
rc=0; [ "$E1" != "$E2" ] || rc=1
assert_eq "the two spawns draw DIFFERENT entry names" "0" "$rc"
assert_eq "the first spawn's perimeter is still on disk" \
    "repo=senara-solutions/mika" "$(cat "$E1")"
assert_eq "the second spawn's perimeter names ITS repository" \
    "repo=senara-solutions/mika-platform" "$(cat "$E2")"
assert_eq "the registry directory is 0700" "700" "$(stat -c '%a' "$R1")"
assert_eq "each entry is 0600" "600" "$(stat -c '%a' "$E1")"

echo ""
echo "CASE 3 — the SPAWN path stages it, and the registry never enters the sandbox"
echo "----------------------------------------------------------------------------"
R3=$(registry_for case3)
_PILOT_EGRESS_PUSH_SCOPE_DIR="$R3"
REPO="mika"
: > "$CAPTURE"   # a stale capture would make the argv assertions vacuous
_run_pilot_sandboxed /bin/true >/dev/null 2>&1
assert_eq "bwrap was really called (the capture is not stale)" "0" "$([ -s "$CAPTURE" ] && echo 0 || echo 1)"
assert_eq "the spawn declared its perimeter" "1" "$(entries_in "$R3")"
E3="$R3/$_PILOT_EGRESS_PUSH_SCOPE_ENTRY"
assert_eq "and it names the dispatch repository" \
    "repo=senara-solutions/mika" "$(cat "$E3")"
# Host-only state. Were the registry bind-mounted or its path exported, an
# in-sandbox process could widen its own perimeter and the tightening would
# attest the forgery. `~/.mika` is not among `_pilot_gitdir_bind_args` — this
# assertion is what keeps that true as the bind list grows (mika#2724).
rc=0; grep -qzF -- "$R3" "$CAPTURE" && rc=1
assert_eq "the registry path appears nowhere in the bwrap argv" "0" "$rc"
rc=0; grep -qzF -- "_PILOT_EGRESS_PUSH_SCOPE_DIR" "$CAPTURE" && rc=1
assert_eq "the override variable is not handed to the sandbox either" "0" "$rc"
# PER SPAWN on the real entry point, not only on the producer: a perimeter
# hoisted to once-per-dispatch passes cases 1-2 and fails exactly here.
: > "$CAPTURE"
_run_pilot_sandboxed /bin/true >/dev/null 2>&1
assert_eq "a second spawn declares a second perimeter" "2" "$(entries_in "$R3")"

echo ""
echo "CASE 4 — no dispatch repository: declare NOTHING, and say so"
echo "-------------------------------------------------------------"
# Free-text mode and milestone mode reach the sandbox with `REPO` empty. The two
# tempting shapes are both wrong: `senara-solutions/` alone is a perimeter
# naming nothing, and an empty-valued `repo=` line is worse — the reader would
# see a DECLARED perimeter and refuse every push, including to the repository
# the spawn legitimately works on. Declaring nothing degrades to the static
# bound, which is the pre-ticket behaviour.
R4=$(registry_for case4)
_PILOT_EGRESS_PUSH_SCOPE_DIR="$R4"
REPO=""
ERR4="$TMPROOT/case4.stderr"
_stage_pilot_egress_push_scope 2>"$ERR4"
rc4=$?
assert_eq "empty REPO: the staging still returns 0 (dispatch continues)" "0" "$rc4"
assert_eq "empty REPO: no entry name was memorized" "" "$_PILOT_EGRESS_PUSH_SCOPE_ENTRY"
rc=0; [ -d "$R4" ] && rc=1
assert_eq "empty REPO: the registry is not even created" "0" "$rc"
err4=$(cat "$ERR4")
assert_contains "empty REPO: stderr names what is missing" "no dispatch repository in scope" "$err4"
assert_contains "empty REPO: stderr names the bound that applies alone" "mika#2736" "$err4"
assert_contains "empty REPO: stderr names the ticket" "mika#2751" "$err4"

echo ""
echo "CASE 5 — fail-OPEN: an undeclarable perimeter degrades, never blocks"
echo "--------------------------------------------------------------------"
# Same posture as `_stage_pilot_gh_token` and `_stage_pilot_egress_sentinel`,
# and the posture the ticket required of the whole tightening: a bookkeeping
# file never kills a dispatch. What the worst case costs is the tightening
# itself — the static bound of mika#2736 keeps biting underneath, which the
# addon side asserts in `test_egress_push_bound.sh` CASE 17.
if [ "$(id -u)" -eq 0 ]; then
    echo "  ⊘ skipped — running as root, mode 0500 refuses nobody"
else
    RO="$TMPROOT/unwritable"
    mkdir -p "$RO"
    chmod 0500 "$RO"
    ERR5="$TMPROOT/case5.stderr"
    _PILOT_EGRESS_PUSH_SCOPE_DIR="$RO/scopes"
    REPO="mika"
    _stage_pilot_egress_push_scope 2>"$ERR5"
    rc5=$?
    assert_eq "unwritable registry: the staging still returns 0 (dispatch continues)" "0" "$rc5"
    assert_eq "unwritable registry: no entry name was memorized" "" "$_PILOT_EGRESS_PUSH_SCOPE_ENTRY"
    err5=$(cat "$ERR5")
    assert_contains "unwritable registry: stderr names the registry it could not write" "$RO/scopes" "$err5"
    assert_contains "unwritable registry: stderr names the ticket" "mika#2751" "$err5"
    assert_contains "unwritable registry: stderr names the bound that applies alone" "mika#2736" "$err5"
    # And the dispatch really goes on — not merely a return code, the spawn.
    : > "$CAPTURE"
    _run_pilot_sandboxed /bin/true >/dev/null 2>&1
    assert_eq "unwritable registry: the spawn still reaches bwrap" "0" "$([ -s "$CAPTURE" ] && echo 0 || echo 1)"
    chmod 0700 "$RO"   # so the EXIT trap can remove the temp tree
fi

echo ""
echo "CASE 6 — MIKA_PILOT_EGRESS_PUSH_SCOPE has NO reader here, by decision"
echo "----------------------------------------------------------------------"
# DO NOT "FIX" THIS INTO A SWITCH TEST. The switch is read in exactly one place
# in the repository — `_push_scope_enforced()` in the addon, where the refusal
# lives. Teaching the launcher to read it too would be the mika#2165 shape: two
# readers of one name, disagreeing on the day it matters, and an operator who
# disarms the tightening on a running daemon while the launcher keeps declaring
# perimeters (or the reverse). A repository name is not a secret, so there is
# nothing gained by withholding the write — and leaving stale perimeters out of
# the registry is worth less than having a single reader.
R6=$(registry_for case6)
_PILOT_EGRESS_PUSH_SCOPE_DIR="$R6"
REPO="mika"
MIKA_PILOT_EGRESS_PUSH_SCOPE=0 _stage_pilot_egress_push_scope
assert_eq "switch off: the launcher declares the perimeter anyway" "1" "$(entries_in "$R6")"
assert_eq "switch off: and declares it unchanged" \
    "repo=senara-solutions/mika" "$(cat "$R6/$_PILOT_EGRESS_PUSH_SCOPE_ENTRY")"
# The structural half of the same statement: the name may appear in the prose
# that explains the single-reader decision, and nowhere else. A `case` on it, a
# `${MIKA_PILOT_EGRESS_PUSH_SCOPE:-1}`, an `export` — any of those is a second
# reader, and this assertion is what refuses it.
assert_eq "switch off: dispatch-lib names the switch in prose only, never in code" "0" \
    "$(grep -F 'MIKA_PILOT_EGRESS_PUSH_SCOPE' "$DISPATCH_LIB" | grep -vc '^[[:space:]]*#')"
# The neighbouring switch stays orthogonal in the direction that matters here:
# turning the sentinel off must not stop the perimeter from being declared.
R6B=$(registry_for case6b)
_PILOT_EGRESS_PUSH_SCOPE_DIR="$R6B"
MIKA_PILOT_EGRESS_SENTINEL=0 _stage_pilot_egress_push_scope
assert_eq "MIKA_PILOT_EGRESS_SENTINEL=0: the perimeter is declared all the same" "1" "$(entries_in "$R6B")"

echo ""
echo "CASE 7 — the default registry path is resolved at CALL time"
echo "------------------------------------------------------------"
# Measured defect of mika#2716, inherited deliberately rather than rediscovered:
# a default computed at SOURCE time freezes the OPERATOR's real registry,
# because the containment suites `export HOME` AFTER sourcing dispatch-lib.
# `make test` then deposits live-looking perimeters in it — and a perimeter is
# exactly the kind of entry that only ever WIDENS the union, so the residue
# would be invisible until the day it mattered.
R7_DEFAULT="$HOME/.mika/pilot-push-scopes"
rc=0; [ -d "$R7_DEFAULT" ] && rc=1
assert_eq "no override: every case so far steered clear of the default" "0" "$rc"
unset _PILOT_EGRESS_PUSH_SCOPE_DIR
REPO="mika"
_stage_pilot_egress_push_scope
assert_match "no override: a perimeter is still declared" "$NONCE_RE" "$_PILOT_EGRESS_PUSH_SCOPE_ENTRY"
rc=0; [ -d "$R7_DEFAULT" ] || rc=1
assert_eq "no override: it lands under the CURRENT \$HOME, not a frozen one" "0" "$rc"
assert_eq "no override: exactly that one declaration is there" "1" "$(entries_in "$R7_DEFAULT")"
assert_eq "no override: and it is this declaration, under this HOME" \
    "repo=senara-solutions/mika" "$(cat "$R7_DEFAULT/$_PILOT_EGRESS_PUSH_SCOPE_ENTRY")"

echo ""
echo "===================================================="
echo "Results: $PASS passed, $FAIL failed"
echo "===================================================="
# Anti-vacuity floor: the suite must really carry its cases. A `source` that
# dies early, a stub that swallows a case, or a producer renamed out from under
# the file would otherwise print "0 passed, 0 failed" and exit 0.
if [ "$PASS" -lt 36 ]; then
    echo "FAIL: only $PASS assertions ran — the suite lost cases (floor: 36)"
    exit 1
fi
[ "$FAIL" -eq 0 ] || exit 1
exit 0

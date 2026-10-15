#!/bin/bash
# mika#2716: the per-spawn EGRESS SENTINEL is drawn, registered host-side, and
# the value the sandbox receives is the one the registry attests.
#
# WHY THIS SUITE EXISTS, next to four containment suites that already read
# GH_TOKEN. Those four assert a NEGATIVE — no credential in the argv, none in
# the sandbox env, none on the filesystem — and mika#2716 widened each of their
# anchors from a literal to `^proxy-managed-no-secret(-[0-9a-f]{32})?$`. That
# widened anchor is satisfied by the BARE PREFIX, deliberately: the bare prefix
# is the fail-open and the `MIKA_PILOT_EGRESS_SENTINEL=0` shape, and a guard
# refusing it would make the operator switch unusable. Which means all four stay
# green on a staging function that draws NOTHING and registers NOTHING. The
# positive half — a nonce was really drawn, it is really in the registry, and it
# is really the value that travelled — has no home among them. It is here.
#
# WHAT THE REGISTRY IS FOR, and why the identity assertion is the load-bearing
# one. Vol 1 (this flight) endows and attests; the addon is untouched, so no new
# 403 is possible. Vol 2 teaches the addon to READ the registry and refuse a
# request whose sentinel is absent or unknown. That reader is only as good as
# this correspondence: if the value in the sandbox and the filename in the
# registry can ever diverge, vol 2's 403 lands on legitimate pilots. Case 4
# measures exactly that correspondence, and measures it on the argv bwrap was
# ACTUALLY called with.
#
# WHY THE ARGV AND NOT A REAL SANDBOX. The containment suites launch a real
# bwrap because the invariant they guard (a value is absent from a live process
# environment) cannot be read off an argv. This suite's invariant is the
# opposite: a value that the host drew is PRESENT in the argv, unmodified. The
# argv is where `--setenv GH_TOKEN` is decided, so a real bwrap would add a
# minute per case and no evidence. `test_sandbox_gh_usable.sh` already pays for
# the real-sandbox half (gh prints a sentinel-shaped token inside a live bwrap).
#
# WHERE THE STAGING LIVES, because it decides the shape of cases 4 and 7.
# `_stage_pilot_egress_sentinel` is called INSIDE `_run_pilot_sandboxed`, right
# after `_stage_pilot_gh_token` — not in a dispatch preamble. That is what makes
# the sentinel per-SPAWN rather than per-dispatch, so case 4 drives the real
# entry point twice and reads two argv captures, instead of staging by hand and
# hoping the spawn reuses what it finds. Only cases 1-3, 5 and 6 call the
# producer directly, and they do so to isolate ITS behaviour (shape, switch,
# fail-open), never to stand in for a spawn.
#
# `_PILOT_EGRESS_SENTINEL_DIR` is therefore assigned as a plain GLOBAL per case,
# never as a `VAR=x func` command prefix: the producer that must see it is two
# frames down inside `_run_pilot_sandboxed`, where a prefix on this suite's own
# call would not reach. Every case sets it before acting, which is also what
# keeps case 7's "the default was never used" precondition honest.
#
# THREAT MODEL, stated so no reader over-reads this suite. The sentinel travels
# in the sandbox environment, so `cat /proc/self/environ` reveals it from
# INSIDE. Vol 1 closes the ACCIDENT of mika#2713 — a test that overwrites
# GH_TOKEN destroys its own sentinel by its own hand — not deliberate in-sandbox
# replay, which stays covered by mika#2056 (the sandbox holds no credential at
# all). A host-side argv reader already holds the real token, so the sentinel
# gives it nothing it did not have.
#
# Run: bash skills/bundled/_shared/tests/test_egress_sentinel_staging.sh
# Expected: all assertions pass, exit 0. Needs no bwrap and no gh; case 6 skips
# by name when run as root (mode 0500 refuses nobody there).

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"

# The ENABLED form, pinned whole — no optional nonce group. The containment
# suites accept a bare prefix because they must tolerate the operator switch;
# this suite is the one that proves the switch was NOT taken, so a bare prefix
# here is a failure, not a tolerated state.
SENTINEL_RE='^proxy-managed-no-secret-[0-9a-f]{32}$'
BARE='proxy-managed-no-secret'

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

TMPROOT=$(mktemp -d "${TMPDIR:-/tmp}/mika2716-XXXXXX")
# mika#2049 — the composed handler is the CALLER's, always. The fake relay
# documents why: bash keeps one handler per signal, so a trap installed inside
# the helper would silently replace this `rm -rf` and leak the temp tree.
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
# assertion read as a fresh draw.
registry_for() {
    local d="$TMPROOT/registry-$1"
    mkdir -p "$(dirname "$d")"
    printf '%s' "$d"
}
entries_in() { ls -1 "$1" 2>/dev/null | wc -l | tr -d ' '; }

echo ""
echo "CASE 1 — the sentinel is drawn PER SPAWN, never reused"
echo "-------------------------------------------------------"
R1=$(registry_for case1)
_PILOT_EGRESS_SENTINEL_DIR="$R1"
_stage_pilot_egress_sentinel
S1A="$_PILOT_EGRESS_SENTINEL"
_stage_pilot_egress_sentinel
S1B="$_PILOT_EGRESS_SENTINEL"
assert_match "the first spawn's sentinel carries a 32-hex nonce" "$SENTINEL_RE" "$S1A"
assert_match "the second spawn's sentinel carries a 32-hex nonce" "$SENTINEL_RE" "$S1B"
rc=0; [ "$S1A" != "$S1B" ] || rc=1
assert_eq "the two spawns draw DIFFERENT sentinels" "0" "$rc"

echo ""
echo "CASE 2 — both spawns stay valid at the SAME instant"
echo "-----------------------------------------------------"
# This is why the registry is a DIRECTORY and not a file. mitmdump is long-lived
# and shared between dispatches: a single overwritten file would make vol 2's
# reader refuse the older of two concurrent pilots, which is a 403 on legitimate
# work — the exact failure mode the ticket demanded be handled before deploy.
assert_eq "the registry holds two entries, not one overwritten" "2" "$(entries_in "$R1")"
rc=0; [ -f "$R1/${S1A#"$BARE"-}" ] || rc=1
assert_eq "the first spawn's nonce is still attested" "0" "$rc"
rc=0; [ -f "$R1/${S1B#"$BARE"-}" ] || rc=1
assert_eq "the second spawn's nonce is attested too" "0" "$rc"

echo ""
echo "CASE 3 — the registry is the HOST's alone"
echo "-------------------------------------------"
assert_eq "the registry directory is 0700" "700" "$(stat -c '%a' "$R1")"
assert_eq "each entry is 0600" "600" "$(stat -c '%a' "$R1/${S1A#"$BARE"-}")"
# The entry is a marker, not a secret store: its NAME is the nonce and its body
# is empty. Vol 2 reads the name.
assert_eq "each entry is empty — the nonce is the filename" "0" "$(stat -c '%s' "$R1/${S1A#"$BARE"-}")"

echo ""
echo "CASE 4 — the value the sandbox receives is the one the registry attests"
echo "------------------------------------------------------------------------"
R4=$(registry_for case4)
_PILOT_EGRESS_SENTINEL_DIR="$R4"

# The GH_TOKEN value in the last captured bwrap argv, or the empty string.
argv_gh_token() {
    local -a a=()
    local x i out=""
    while IFS= read -r -d '' x; do a+=("$x"); done < "$CAPTURE"
    for ((i = 0; i + 2 < ${#a[@]}; i++)); do
        if [ "${a[$i]}" = "--setenv" ] && [ "${a[$((i + 1))]}" = "GH_TOKEN" ]; then
            out="${a[$((i + 2))]}"
        fi
    done
    printf '%s' "$out"
}

# Spawn once. The sentinel is staged BY this call (see WHERE THE STAGING LIVES),
# so `$_PILOT_EGRESS_SENTINEL` read afterwards is what the host believes it
# issued, and the argv is what the sandbox will actually receive.
: > "$CAPTURE"   # a stale capture would make every assertion below vacuous
_run_pilot_sandboxed /bin/true >/dev/null 2>&1
G4A=$(argv_gh_token)
H4A="$_PILOT_EGRESS_SENTINEL"
assert_eq "bwrap was really called (the capture is not stale)" "0" "$([ -s "$CAPTURE" ] && echo 0 || echo 1)"
assert_match "the argv's GH_TOKEN carries a 32-hex sentinel" "$SENTINEL_RE" "$G4A"
assert_eq "it is EXACTLY what the host believes it issued" "$H4A" "$G4A"
rc=0; [ -f "$R4/${G4A#"$BARE"-}" ] || rc=1
assert_eq "the argv value and the registry filename correspond" "0" "$rc"
# The registry is host-only state. Were it bind-mounted, an in-sandbox process
# could forge an entry and vol 2's reader would attest the forgery.
rc=0; grep -qzF -- "$R4" "$CAPTURE" && rc=1
assert_eq "the registry path appears nowhere in the bwrap argv" "0" "$rc"

# Spawn again. PER SPAWN is asserted on the real entry point here, not only on
# the producer: a sentinel hoisted to once-per-dispatch would pass cases 1-3 and
# fail exactly this.
: > "$CAPTURE"
_run_pilot_sandboxed /bin/true >/dev/null 2>&1
G4B=$(argv_gh_token)
assert_match "the second spawn's argv carries a 32-hex sentinel too" "$SENTINEL_RE" "$G4B"
rc=0; [ "$G4A" != "$G4B" ] || rc=1
assert_eq "two spawns put DIFFERENT sentinels in the argv" "0" "$rc"
rc=0; [ -f "$R4/${G4B#"$BARE"-}" ] || rc=1
assert_eq "the second spawn's value is attested too" "0" "$rc"
assert_eq "and the registry attests both spawns, not one" "2" "$(entries_in "$R4")"

echo ""
echo "CASE 5 — MIKA_PILOT_EGRESS_SENTINEL=0 restores the pre-ticket placeholder"
echo "--------------------------------------------------------------------------"
# The operator switch the ticket required: one variable returns the substrate to
# its mika#2572 behaviour, and it must register NOTHING — an entry written while
# the switch is off would make vol 2's TTL purge the only thing that ever
# removes it.
R5=$(registry_for case5)
_PILOT_EGRESS_SENTINEL_DIR="$R5"
MIKA_PILOT_EGRESS_SENTINEL=0 _stage_pilot_egress_sentinel
assert_eq "disabled: the sentinel is the bare static placeholder" "$BARE" "$_PILOT_EGRESS_SENTINEL"
assert_eq "disabled: nothing is registered" "0" "$(entries_in "$R5")"
rc=0; [ -d "$R5" ] && rc=1
assert_eq "disabled: the registry is not even created" "0" "$rc"
# Same vocabulary as MIKA_PILOT_SANDBOX, case-insensitively — an operator who
# writes `off` must not get a silently-enabled sentinel.
for word in FALSE No oFF Disabled; do
    MIKA_PILOT_EGRESS_SENTINEL="$word" _stage_pilot_egress_sentinel
    assert_eq "disabled by '$word' too" "$BARE" "$_PILOT_EGRESS_SENTINEL"
done
# And the spawn path honours it too — the switch is useless if only the producer
# reads it.
: > "$CAPTURE"
MIKA_PILOT_EGRESS_SENTINEL=0 _run_pilot_sandboxed /bin/true >/dev/null 2>&1
assert_eq "disabled: the argv carries the bare placeholder" "$BARE" "$(argv_gh_token)"
assert_eq "disabled: the spawn registered nothing either" "0" "$(entries_in "$R5")"

echo ""
echo "CASE 6 — fail-OPEN: an unregisterable sentinel degrades, never blocks"
echo "----------------------------------------------------------------------"
# Same posture as _stage_pilot_gh_token, and it is the posture the ticket asked
# for: the failure mode that made this approach get discarded in mika#2713 was
# a staging bug silently 403-ing every legitimate call. Degrading to the bare
# prefix means the worst a staging failure can do in vol 1 is return the
# substrate to its pre-ticket behaviour — loudly, on stderr.
if [ "$(id -u)" -eq 0 ]; then
    echo "  ⊘ skipped — running as root, mode 0500 refuses nobody"
else
    RO="$TMPROOT/unwritable"
    mkdir -p "$RO"
    chmod 0500 "$RO"
    ERR="$TMPROOT/case6.stderr"
    _PILOT_EGRESS_SENTINEL_DIR="$RO/registry"
    _stage_pilot_egress_sentinel 2>"$ERR"
    rc6=$?
    assert_eq "unwritable registry: the sentinel is the bare placeholder" "$BARE" "$_PILOT_EGRESS_SENTINEL"
    assert_eq "unwritable registry: the staging still returns 0 (dispatch continues)" "0" "$rc6"
    err6=$(cat "$ERR")
    assert_contains "unwritable registry: stderr names the registry it could not write" "$RO/registry" "$err6"
    assert_contains "unwritable registry: stderr names the ticket" "mika#2716" "$err6"
    assert_contains "unwritable registry: stderr names the fallback" "falling back to the static placeholder" "$err6"
    # And the degraded value must still be usable by gh (mika#2572), which is
    # precisely what the bare prefix is.
    : > "$CAPTURE"
    _run_pilot_sandboxed /bin/true >/dev/null 2>&1
    rc=0; grep -qzF -- "$BARE" "$CAPTURE" || rc=1
    assert_eq "unwritable registry: the sandbox still receives a gh-usable GH_TOKEN" "0" "$rc"
    chmod 0700 "$RO"   # so the EXIT trap can remove the temp tree
fi

echo ""
echo "CASE 7 — the default registry path is resolved at CALL time"
echo "------------------------------------------------------------"
# Measured defect, first run of this ticket. The default used to be computed at
# SOURCE time (`: "${_PILOT_EGRESS_SENTINEL_DIR:=$HOME/.mika/…}"`), but the
# containment suites `export HOME="$TMPROOT/home"` AFTER sourcing dispatch-lib,
# so the frozen value was the OPERATOR's real registry and `make test` deposited
# live-looking entries in it. A test that forgets to override now gets its own
# HOME's registry, which its own temp tree carries away.
R7_DEFAULT="$HOME/.mika/pilot-egress-sentinels"
rc=0; [ -d "$R7_DEFAULT" ] && rc=1
assert_eq "no override: every case so far steered clear of the default" "0" "$rc"
unset _PILOT_EGRESS_SENTINEL_DIR
_stage_pilot_egress_sentinel
assert_match "no override: a sentinel is still drawn" "$SENTINEL_RE" "$_PILOT_EGRESS_SENTINEL"
rc=0; [ -d "$R7_DEFAULT" ] || rc=1
assert_eq "no override: the draw lands under the CURRENT \$HOME, not a frozen one" "0" "$rc"
assert_eq "no override: exactly that one draw is there" "1" "$(entries_in "$R7_DEFAULT")"
rc=0; [ -f "$R7_DEFAULT/${_PILOT_EGRESS_SENTINEL#"$BARE"-}" ] || rc=1
assert_eq "no override: and it is this draw, under this HOME" "0" "$rc"

echo ""
echo "===================================================="
echo "Results: $PASS passed, $FAIL failed"
echo "===================================================="
[ "$FAIL" -eq 0 ] || exit 1
exit 0

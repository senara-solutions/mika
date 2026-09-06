#!/bin/bash
# mika#2165: the pilot's session-log directory is writable from INSIDE the
# sandbox and lands on the host — and the containment around it is still closed.
#
# WHY THIS TEST HAS THE SHAPE IT HAS. The defect it guards ran undetected for a
# MONTH, across at least twenty consecutive sessions, with an adversarial canary
# suite already in place. Nothing detected it because nothing failed: the
# sandbox root is a writable tmpfs, so claude-pilot's mkdir, open("a") and every
# write SUCCEEDED. The file was created, filled, and then discarded with the
# container. An argv-inspecting test has exactly the same blind spot the defect
# had — the argv was fine, the resulting namespace was not — so argv assertions
# are explicitly out of contract here.
#
# So this suite launches a REAL bwrap, through the REAL `_run_pilot_sandboxed`,
# and then asserts from the HOST that the bytes written inside survived. That
# host-side read is the whole test: "the write succeeded" is precisely what was
# true throughout the outage.
#
# Both halves run against the same sandbox in the same run, because either half
# alone is vacuous: a `--ro-bind /var` would pass every must-work check, and a
# bwrap that fails to launch would pass every must-fail one.
#
#   MUST WORK   a write from inside to $PILOT_LOG_DIR/<id>.log is readable from
#               the host afterwards; the directory is listable and creatable-in.
#
#   MUST FAIL   /var/log itself (the parent — AC1 asks for this directory, not
#               /var/log); /var/log/mika, which holds the egress-proxy log, an
#               incident-diagnosis surface (mika#2041) the pilot must not touch;
#               /var/lib and /srv. Each adjacent target is proven to exist
#               host-side first, so its failure inside is isolation and not
#               absence.
#
#   NON-VACUITY The must-work half is re-run with the bind neutralised and must
#               go RED. AC2 requires this explicitly: a green that stays green
#               without the fix is not evidence.
#
# PILOT_LOG_DIR is redirected to a mktemp dir rather than /var/log/claude-pilot,
# so `make test` never writes into the live operational diagnosis surface — the
# same precaution MIKA_PILOT_EGRESS_LOG_DIR already takes for the egress log.
# It is exported BEFORE sourcing, because _PILOT_LOG_DIR resolves at source time.
#
# Companions, none subsuming this one:
#   test_sandbox_git_usable.sh                 — mika#2141 gitdir binds
#   test_sandbox_no_secret_in_argv.sh          — mika#2039 argv channel
#   test-pilot-github-token-not-in-sandbox.sh  — mika#2056 credential absence
#
# Run: bash skills/bundled/_shared/tests/test_sandbox_log_dir_bound.sh
# Expected: all assertions pass, exit 0. Skips cleanly when bwrap is absent.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"

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
assert_contains() {
    local label="$1" needle="$2" haystack="$3"
    if [[ "$haystack" == *"$needle"* ]]; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    expected to contain: '$needle'"; echo "    actual:              '$haystack'"
    fi
}

if ! command -v bwrap >/dev/null 2>&1; then
    echo "⊘ skipped — bwrap not installed on PATH (mika#2165 needs a real sandbox)"
    exit 0
fi

TMPROOT=$(mktemp -d "${TMPDIR:-/tmp}/mika2165-XXXXXX")
trap 'rm -rf "$TMPROOT"' EXIT

# The redirected log dir. Must be exported before the source below.
export PILOT_LOG_DIR="$TMPROOT/pilot-log"
mkdir -p "$PILOT_LOG_DIR"

# A plain directory is the right worktree shape here: _pilot_gitdir_bind_args
# returns early when there is no .git, which is exactly the harness case it
# documents. This suite is about the log bind, not about git.
WORKTREE_DIR="$TMPROOT/worktree"
mkdir -p "$WORKTREE_DIR"

export HOME="$TMPROOT/home"
mkdir -p "$HOME" "$HOME/.mika/data/pilot-transcripts"

# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

# No real egress proxy / mitmproxy for a test. This exercises the Phase 2a
# fallback bwrap construction — the degraded path, which must carry the log
# bind too. Wiring only the Phase 2b branch is the named risk of this ticket.
_ensure_pilot_egress_proxy() { return 1; }
_ensure_pilot_helper() { return 1; }
_PILOT_SANDBOX_SECRET_ALLOWLIST=()

echo ""
echo "PRECONDITION — the resolver honours the redirect"
echo "------------------------------------------------"
assert_eq "_PILOT_LOG_DIR follows PILOT_LOG_DIR" "$PILOT_LOG_DIR" "$_PILOT_LOG_DIR"

echo ""
echo "MUST WORK — the log directory is writable from inside, and survives"
echo "-------------------------------------------------------------------"

PROBE_ID="probe-2165-$$"
PROBE_LOG="$PILOT_LOG_DIR/$PROBE_ID.log"

# This mirrors what claude-pilot's logger.py actually does: mkdir -p the
# parent, then open("a") and write. Writing it the same way is what makes the
# probe representative rather than merely adjacent.
inside_write=$(_run_pilot_sandboxed /bin/sh -c "
    mkdir -p '$PILOT_LOG_DIR' 2>/dev/null
    if printf '[prompt] /ce-work probe mika#2165\n' >> '$PROBE_LOG' 2>/dev/null; then
        echo wrote
    else
        echo write-failed
    fi" 2>/dev/null)
assert_eq "the pilot can append to its session log from inside" "wrote" "$inside_write"

# THE assertion. Everything above was already true during the outage.
host_sees="missing"
[ -s "$PROBE_LOG" ] && host_sees="present"
assert_eq "the bytes written inside are readable HOST-side afterwards" "present" "$host_sees"
assert_contains "the host-side file carries the [prompt] line (AC2)" "[prompt]" "$(cat "$PROBE_LOG" 2>/dev/null)"

# The pilot creates its own file, so it needs write on the DIRECTORY, not just
# on a pre-created path. A bind that only exposed an existing file would pass
# the append above and fail here.
inside_create=$(_run_pilot_sandboxed /bin/sh -c "
    : > '$PILOT_LOG_DIR/created-inside-$$.log' 2>/dev/null && echo created || echo refused" 2>/dev/null)
assert_eq "a NEW file can be created in the directory from inside" "created" "$inside_create"
host_created="missing"
[ -e "$PILOT_LOG_DIR/created-inside-$$.log" ] && host_created="present"
assert_eq "that new file is visible host-side too" "present" "$host_created"

inside_list=$(_run_pilot_sandboxed /bin/sh -c "[ -d '$PILOT_LOG_DIR' ] && echo listable || echo hidden" 2>/dev/null)
assert_eq "the directory itself is visible from inside" "listable" "$inside_list"

echo ""
echo "MUST FAIL — the bind is this directory, not /var/log (AC1, AC4)"
echo "---------------------------------------------------------------"

# Positive controls first: each adjacent target is reachable host-side, so a
# refusal inside is isolation rather than a path that simply does not exist.
for probe_dir in /var/log /var/lib /srv; do
    host_present=$( [ -d "$probe_dir" ] && echo present || echo missing )
    assert_eq "control: $probe_dir exists host-side" "present" "$host_present"
done

# The distinction between "the filesystem refused" and "the path is not there"
# is load-bearing: a tmpfs materialised over /var would let a write SUCCEED into
# nothing, which is the exact failure mode this whole ticket is about. So a
# write that appears to succeed is classified LEAKED, not passed.
probe_write_refused() {
    local target="$1"
    _run_pilot_sandboxed /bin/sh -c "
        if : > '$target/mika2165-probe-$$' 2>/dev/null; then
            echo LEAKED
            rm -f '$target/mika2165-probe-$$' 2>/dev/null
        else
            echo refused
        fi" 2>/dev/null
}

for probe_dir in /var/log /var/lib /srv; do
    assert_eq "writing into $probe_dir is refused" "refused" "$(probe_write_refused "$probe_dir")"
done

# /var/log/mika is the sharpest adjacent target: it holds the egress-proxy log,
# an incident-diagnosis surface (mika#2041). Asserted only when it exists
# host-side — otherwise the check would be vacuous and would read as a pass.
if [ -d /var/log/mika ]; then
    assert_eq "control: /var/log/mika exists host-side" "present" "present"
    assert_eq "the egress-proxy log directory is unreachable from inside" \
        "refused" "$(probe_write_refused /var/log/mika)"
    got_egress_read=$(_run_pilot_sandboxed /bin/sh -c \
        "[ -e /var/log/mika/pilot-egress-proxy.log ] && echo visible || echo hidden" 2>/dev/null)
    assert_eq "the egress-proxy log file is not even visible from inside" "hidden" "$got_egress_read"
else
    echo "  ⊘ /var/log/mika absent host-side — its two assertions would be vacuous, skipped"
fi

echo ""
echo "NON-VACUITY — remove the bind and the must-work half goes red (AC2)"
echo "-------------------------------------------------------------------"
# Same probe, same sandbox construction, bind neutralised. If this still shows
# the bytes host-side, the assertions above are proving something other than
# the bind and this suite is worthless.
_pilot_log_bind_args() { _PILOT_LOG_BIND_ARGS=(); }

NEG_ID="probe-2165-neg-$$"
NEG_LOG="$PILOT_LOG_DIR/$NEG_ID.log"
neg_inside=$(_run_pilot_sandboxed /bin/sh -c "
    mkdir -p '$PILOT_LOG_DIR' 2>/dev/null
    printf '[prompt] unbound\n' >> '$NEG_LOG' 2>/dev/null && echo wrote || echo write-failed" 2>/dev/null)
# The write still succeeds — into the ephemeral tmpfs. That is the defect, and
# reproducing it here is what proves the host-side check is the real assertion.
assert_eq "without the bind, the write still SUCCEEDS inside (the silent failure)" "wrote" "$neg_inside"

neg_host="missing"
[ -s "$NEG_LOG" ] && neg_host="present"
assert_eq "without the bind, NOTHING reaches the host (the suite is not vacuous)" "missing" "$neg_host"

echo ""
echo "===================================================="
echo "Results: $PASS passed, $FAIL failed"
echo "===================================================="
[ "$FAIL" -eq 0 ] || exit 1
exit 0

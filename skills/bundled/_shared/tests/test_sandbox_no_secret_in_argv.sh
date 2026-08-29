#!/bin/bash
# Test suite for the pilot sandbox secret channel (mika#2039).
#
# Invariant under test: no credential-shaped VALUE ever reaches the bwrap
# argv (world-readable via /proc/<pid>/cmdline) or the BASH_XTRACEFD trace
# file (tailed back into the dispatch callback, and only partially covered
# by the NAME=value redaction at _redact_trace).
#
# This is the R8 "value guard" of the plan. Its companion is the R7 "name
# guard" at scripts/verify-no-secret-in-setenv.sh — neither subsumes the
# other: the lint catches a credential-shaped var entering the --setenv
# allowlist, this suite catches a regression of the mechanism itself.
#
# Anti-vacuity: this suite MUST fail on the pre-mika#2039 form of
# dispatch-lib.sh (where GH_TOKEN is passed via --setenv) and pass on the
# corrected form. A guard never seen failing is not a guard.
#
# Source isolation audit: dispatch-lib.sh has no top-level imperative code —
# all `set -e`, `trap`, and env var references are inside function bodies.
# Safe to source directly without a guard variable.
#
# Run: bash skills/bundled/_shared/tests/test_sandbox_no_secret_in_argv.sh
# Expected: all assertions pass, exit 0.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"

# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

PASS=0
FAIL=0

# Synthetic credential. Never a real token — the shape is what the guard
# matches, the value is inert.
FAKE_TOKEN="github_pat_0000000000000000000000000000000000000000"
CRED_PATTERN='github_pat_|ghp_|gho_|ghu_|ghs_|sk-|AKIA'

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

assert_true() {
    local label="$1" cond_rc="$2"
    if [ "$cond_rc" -eq 0 ]; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
    fi
}

# --- Sandbox harness -------------------------------------------------------
#
# `bwrap` is replaced by a shell function that records its argv NUL-separated
# into $CAPTURE. `command -v bwrap` resolves a function, so the availability
# probe in _run_pilot_sandboxed is satisfied without the real binary.
#
# The two daemon launchers are stubbed so NO real process is started: the
# egress proxy and the mitmproxy helper both spawn background daemons and
# write to /var/log. The stub return code selects Phase 2a vs Phase 2b.

TMPROOT=$(mktemp -d "${TMPDIR:-/tmp}/mika2039-XXXXXX")
trap 'rm -rf "$TMPROOT"' EXIT

CAPTURE="$TMPROOT/bwrap-argv"
export HOME="$TMPROOT/home"
WORKTREE_DIR="$TMPROOT/worktree"
mkdir -p "$HOME" "$WORKTREE_DIR"

MOCK_EGRESS_RC=1   # 1 → Phase 2a (fs cut only); 0 → Phase 2b (full)
MOCK_BWRAP_RC=0

bwrap() {
    printf '%s\0' "$@" > "$CAPTURE"
    return "$MOCK_BWRAP_RC"
}
_ensure_pilot_egress_proxy() { return "$MOCK_EGRESS_RC"; }
_ensure_pilot_helper() { return 1; }

# Read the captured argv back as a bash array.
captured_args() {
    local -a out=()
    local a
    while IFS= read -r -d '' a; do out+=("$a"); done < "$CAPTURE"
    printf '%s\n' "${out[@]}"
}

# Does the captured argv contain a credential-shaped value anywhere?
captured_has_credential() {
    grep -qzE "$CRED_PATTERN" "$CAPTURE"
}

# Index (0-based) of the first occurrence of an exact argument, or -1.
arg_index() {
    local needle="$1" i=0 a
    while IFS= read -r a; do
        if [ "$a" = "$needle" ]; then echo "$i"; return 0; fi
        i=$((i + 1))
    done < <(captured_args)
    echo "-1"
}

# Destinations of each `--ro-bind-data <fd> <dest>` triple. Counting raw
# occurrences of the secret directory would over-count: the entrypoint
# prologue string legitimately names that directory too.
secret_destinations() {
    local -a a=()
    local x i
    while IFS= read -r x; do a+=("$x"); done < <(captured_args)
    for ((i = 0; i < ${#a[@]}; i++)); do
        if [ "${a[$i]}" = "--ro-bind-data" ] && [ $((i + 2)) -lt ${#a[@]} ]; then
            printf '%s\n' "${a[$((i + 2))]}"
        fi
    done
}

run_sandboxed() {
    local rc=0
    GH_TOKEN="$FAKE_TOKEN" _run_pilot_sandboxed "$@" >/dev/null 2>&1 || rc=$?
    echo "$rc"
}

# ============================================================================
# Test 1: Phase 2a — no credential value in the bwrap argv
# ============================================================================
echo ""
echo "Test: Phase 2a (fs cut only) — argv carries no credential"
echo "----------------------------------------------------------"

MOCK_EGRESS_RC=1
run_sandboxed /bin/true >/dev/null

rc=1; captured_has_credential && rc=0
assert_eq "2a: argv contains no credential-shaped value" "1" "$rc"

rc=1; [ "$(arg_index '--ro-bind-data')" != "-1" ] && rc=0
assert_true "2a: argv carries --ro-bind-data (secret channel present)" "$rc"

# ============================================================================
# Test 2: Phase 2b — no credential value in the bwrap argv
# ============================================================================
echo ""
echo "Test: Phase 2b (full containment) — argv carries no credential"
echo "---------------------------------------------------------------"

MOCK_EGRESS_RC=0
run_sandboxed /bin/true >/dev/null

rc=1; captured_has_credential && rc=0
assert_eq "2b: argv contains no credential-shaped value" "1" "$rc"

rc=1; [ "$(arg_index '--ro-bind-data')" != "-1" ] && rc=0
assert_true "2b: argv carries --ro-bind-data (secret channel present)" "$rc"

MOCK_EGRESS_RC=1

# ============================================================================
# Test 3: --perms 0600 immediately precedes --ro-bind-data
# ============================================================================
echo ""
echo "Test: secret file is created 0600"
echo "----------------------------------"

run_sandboxed /bin/true >/dev/null
bd_idx=$(arg_index '--ro-bind-data')
perms_ok=1
if [ "$bd_idx" != "-1" ] && [ "$bd_idx" -ge 2 ]; then
    mapfile -t ARGS < <(captured_args)
    if [ "${ARGS[$((bd_idx - 2))]}" = "--perms" ] && [ "${ARGS[$((bd_idx - 1))]}" = "0600" ]; then
        perms_ok=0
    fi
fi
assert_true "--perms 0600 immediately precedes --ro-bind-data" "$perms_ok"

# ============================================================================
# Test 4: secret destination is bound AFTER --tmpfs /run
# ============================================================================
echo ""
echo "Test: secret destination is nested inside the /run tmpfs"
echo "---------------------------------------------------------"

run_tmpfs_idx=-1
i=0
prev=""
while IFS= read -r a; do
    if [ "$prev" = "--tmpfs" ] && [ "$a" = "/run" ]; then run_tmpfs_idx=$i; fi
    prev="$a"
    i=$((i + 1))
done < <(captured_args)

order_ok=1
if [ "$run_tmpfs_idx" != "-1" ] && [ "$bd_idx" != "-1" ] && [ "$bd_idx" -gt "$run_tmpfs_idx" ]; then
    order_ok=0
fi
assert_true "--ro-bind-data comes after --tmpfs /run" "$order_ok"

# ============================================================================
# Test 5: the xtrace channel carries no credential, and survives the call
# ============================================================================
echo ""
echo "Test: BASH_XTRACEFD trace is clean AND still working afterwards"
echo "----------------------------------------------------------------"

TRACE="$TMPROOT/trace.log"
: > "$TRACE"
# Production shape: the token is already in the environment when the traced
# region starts. Assigning it as a per-call prefix instead would make bash
# trace `+ GH_TOKEN=<value>` — the harness leaking, not the function.
export GH_TOKEN="$FAKE_TOKEN"
exec 9>>"$TRACE"
BASH_XTRACEFD=9
set -x
_run_pilot_sandboxed /bin/true >/dev/null 2>&1 || true
echo "sentinel-after-sandbox-call" >/dev/null
set +x
exec 9>&-
unset GH_TOKEN

rc=1; grep -qE "$CRED_PATTERN" "$TRACE" && rc=0
assert_eq "trace file contains no credential-shaped value" "1" "$rc"

# The specific line shape the fix exists to prevent. `_redact_trace` rewrites
# `NAME=value`, so a `+ GH_TOKEN=...` line would be scrubbed on the way to the
# callback — but `++ printf %s <value>` from an untraced-suppressed process
# substitution would not be, and that is what reaches the caller.
rc=1; grep -qE '\+\+ printf' "$TRACE" && rc=0
assert_eq "trace carries no expanded process-substitution printf" "1" "$rc"

rc=1; grep -q 'sentinel-after-sandbox-call' "$TRACE" && rc=0
assert_true "trace channel still writable after the sandbox call (fd 9 intact)" "$rc"

# ============================================================================
# Test 6: exit status of the payload is preserved
# ============================================================================
echo ""
echo "Test: payload exit status is preserved"
echo "---------------------------------------"

MOCK_BWRAP_RC=42
got=$(run_sandboxed /bin/true)
assert_eq "exit status 42 propagates through _run_pilot_sandboxed" "42" "$got"
MOCK_BWRAP_RC=0

# ============================================================================
# Test 7: secret descriptors are closed on return
# ============================================================================
echo ""
echo "Test: secret file descriptors are closed on return"
echo "---------------------------------------------------"

run_sandboxed /bin/true >/dev/null
leaked=""
for fd in 10 11 12; do
    if [ -e "/proc/$$/fd/$fd" ]; then leaked="$leaked $fd"; fi
done
assert_eq "no secret fd left open after return" "" "$leaked"

# ============================================================================
# Test 8: two secrets get two distinct destinations and two distinct fds
# ============================================================================
echo ""
echo "Test: a second secret gets its own descriptor and destination"
echo "--------------------------------------------------------------"

SECOND_TOKEN="ghp_1111111111111111111111111111111111"
_PILOT_SANDBOX_SECRET_ALLOWLIST=(GH_TOKEN MIKA_TEST_SECOND_SECRET)
export MIKA_TEST_SECOND_SECRET="$SECOND_TOKEN"

run_sandboxed /bin/true >/dev/null

bd_count=$(captured_args | grep -c -- '--ro-bind-data' || true)
assert_eq "two --ro-bind-data arguments emitted" "2" "$bd_count"

dest_count=$(secret_destinations | sort -u | wc -l | tr -d ' ')
assert_eq "two distinct secret destinations emitted" "2" "$dest_count"

rc=1; secret_destinations | grep -qx "/run/mika-pilot-secrets/GH_TOKEN" && rc=0
assert_true "first secret keeps its own destination path" "$rc"
rc=1; secret_destinations | grep -qx "/run/mika-pilot-secrets/MIKA_TEST_SECOND_SECRET" && rc=0
assert_true "second secret gets its own destination path" "$rc"

rc=1; captured_has_credential && rc=0
assert_eq "neither secret value reaches the argv" "1" "$rc"

unset MIKA_TEST_SECOND_SECRET
_PILOT_SANDBOX_SECRET_ALLOWLIST=(GH_TOKEN)

# ============================================================================
# Test 9: no secret set → no secret channel, invocation still valid
# ============================================================================
echo ""
echo "Test: unset GH_TOKEN emits no secret channel"
echo "---------------------------------------------"

rc=0
( unset GH_TOKEN; _run_pilot_sandboxed /bin/true ) >/dev/null 2>&1 || rc=$?
assert_eq "unset token: invocation succeeds" "0" "$rc"

bd_idx_unset=$(arg_index '--ro-bind-data')
assert_eq "unset token: no --ro-bind-data emitted" "-1" "$bd_idx_unset"

# ============================================================================
# Test 10: non-secret passthrough is untouched
# ============================================================================
echo ""
echo "Test: non-secret variables still travel via --setenv"
echo "-----------------------------------------------------"

run_sandboxed /bin/true >/dev/null
has_home=1
prev=""
while IFS= read -r a; do
    if [ "$prev" = "--setenv" ] && [ "$a" = "HOME" ]; then has_home=0; fi
    prev="$a"
done < <(captured_args)
assert_true "HOME is still passed via --setenv" "$has_home"

gh_via_setenv=1
prev=""
while IFS= read -r a; do
    if [ "$prev" = "--setenv" ] && [ "$a" = "GH_TOKEN" ]; then gh_via_setenv=0; fi
    prev="$a"
done < <(captured_args)
assert_eq "GH_TOKEN is NOT passed via --setenv" "1" "$gh_via_setenv"

# ============================================================================
# Test 11: sandbox disabled → direct invocation, no bwrap argv at all
# ============================================================================
echo ""
echo "Test: MIKA_PILOT_SANDBOX=0 falls back to direct invocation"
echo "-----------------------------------------------------------"

: > "$CAPTURE"
rc=0
( export MIKA_PILOT_SANDBOX=0; GH_TOKEN="$FAKE_TOKEN" _run_pilot_sandboxed /bin/true ) >/dev/null 2>&1 || rc=$?
assert_eq "disabled: direct invocation succeeds" "0" "$rc"
assert_eq "disabled: no bwrap argv captured" "0" "$(wc -c < "$CAPTURE" | tr -d ' ')"

# ============================================================================
# Test 12: real bwrap — the descriptor is inherited and the token arrives
# ============================================================================
# This is the ONLY scenario that starts a real process. It restores the real
# bwrap binary first; the daemon stubs stay in place so no egress proxy or
# mitmproxy helper is launched. Skipped when bwrap is not installed.
echo ""
echo "Test: real bwrap — token reaches the sandbox environment"
echo "---------------------------------------------------------"

unset -f bwrap
if ! command -v bwrap >/dev/null 2>&1; then
    echo "  ⊘ skipped — bwrap not installed on PATH"
else
    mkdir -p "$HOME/.mika/data/pilot-transcripts"
    got_token=$(GH_TOKEN="$FAKE_TOKEN" _run_pilot_sandboxed \
        /bin/sh -c 'printf %s "${GH_TOKEN:-<absent>}"' 2>/dev/null)
    assert_eq "sandbox sees the token value via the file channel" "$FAKE_TOKEN" "$got_token"

    got_perms=$(GH_TOKEN="$FAKE_TOKEN" _run_pilot_sandboxed \
        /bin/sh -c 'ls -l /run/mika-pilot-secrets/GH_TOKEN | cut -c1-10' 2>/dev/null)
    assert_eq "secret file is mode 0600 inside the sandbox" "-rw-------" "$got_perms"
fi

# ============================================================================
echo ""
echo "===================================================="
echo "Results: $PASS passed, $FAIL failed"
echo "===================================================="

[ "$FAIL" -eq 0 ] || exit 1
exit 0

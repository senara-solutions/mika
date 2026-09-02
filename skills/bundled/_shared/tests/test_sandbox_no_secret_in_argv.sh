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
# dispatch-lib.sh (where a secret is passed via --setenv) and pass on the
# corrected form. A guard never seen failing is not a guard.
#
# mika#2056 note: `GH_TOKEN` is no longer a member of
# `_PILOT_SANDBOX_SECRET_ALLOWLIST` — the sandbox holds no GitHub token at all;
# it is injected host-side by the egress-proxy MITM. So the CHANNEL-mechanism
# tests below exercise the still-generic --ro-bind-data machinery with a
# SYNTHETIC secret (`MIKA_TEST_SECRET`), and dedicated tests assert the
# mika#2056 invariant directly: GH_TOKEN never reaches the argv, the --setenv
# list, or the real sandbox env/filesystem. The sandbox-absent-but-host-present
# anti-vacuity proof lives in the sibling
# test-pilot-github-token-not-in-sandbox.sh.
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

# mika#2056: the production _PILOT_SANDBOX_SECRET_ALLOWLIST is now empty (no
# GitHub token enters the sandbox). To keep exercising the generic mika#2039
# --ro-bind-data channel machinery, inject a SYNTHETIC secret for the
# mechanism tests. GH_TOKEN-specific absence is asserted separately below.
_PILOT_SANDBOX_SECRET_ALLOWLIST=(MIKA_TEST_SECRET)

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
#
# This is the WEAKER of the two value-side checks. A shape denylist can never
# be complete — npm, Atlassian, Slack and opaque hex secrets all slip past it.
# `assert_setenv_channel_closed` below is the primary check: it asserts the
# CHANNEL (which names may be emitted at all) rather than guessing at shapes.
captured_has_credential() {
    grep -qzE "$CRED_PATTERN" "$CAPTURE"
}

# Every name emitted as `--setenv <NAME>` in the captured argv.
setenv_names() {
    local -a a=()
    local x i
    while IFS= read -r x; do a+=("$x"); done < <(captured_args)
    for ((i = 0; i < ${#a[@]} - 1; i++)); do
        if [ "${a[$i]}" = "--setenv" ]; then
            printf '%s\n' "${a[$((i + 1))]}"
        fi
    done
}

# The complete audited set of names allowed to travel by --setenv: the
# non-secret passthrough allowlist, plus the Phase 2b network vars. Anything
# else appearing is a violation regardless of what its value looks like — this
# is the deny-by-default posture the `--clearenv` invariant already uses,
# applied to the value guard so it does not depend on recognising a vendor.
#
# mika#2141 added three git names. Each was audited on the same terms as the
# rest — a name is added here only after confirming its VALUE carries no
# credential material, since every value lands in a world-readable argv:
#   GIT_CONFIG_GLOBAL    a path (/run/mika-pilot-git/config). The file it names
#                        is generated host-side and holds only an https URL
#                        rewrite and the committer identity; it is never copied
#                        from ~/.gitconfig, so a credential helper there cannot
#                        drift into it.
#   GIT_CONFIG_NOSYSTEM  the literal "1".
#   GIT_TERMINAL_PROMPT  the literal "0". Its absence is what turns a failed
#                        credential injection into a silent hang instead of a
#                        fast, traceable error.
AUDITED_SETENV_NAMES="ANTHROPIC_API_KEY ANTHROPIC_BASE_URL ANTHROPIC_LOG_FILE \
CLAUDE_CODE_API_BASE_URL GIT_CONFIG_GLOBAL GIT_CONFIG_NOSYSTEM \
GIT_TERMINAL_PROMPT HOME HOSTNAME HTTPS_PROXY HTTP_PROXY LANG LC_ALL \
LOGNAME MIKA_LOG_PILOT_TRANSCRIPTS MIKA_PILOT_CONTAINED NODE_EXTRA_CA_CERTS \
NO_PROXY PATH SHELL TERM TMPDIR USER"

assert_setenv_channel_closed() {
    local label="$1" unexpected="" n
    while IFS= read -r n; do
        [ -z "$n" ] && continue
        case " $AUDITED_SETENV_NAMES " in
            *" $n "*) ;;
            *) unexpected="$unexpected $n" ;;
        esac
    done < <(setenv_names)
    assert_eq "$label" "" "$unexpected"
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
    MIKA_TEST_SECRET="$FAKE_TOKEN" _run_pilot_sandboxed "$@" >/dev/null 2>&1 || rc=$?
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
assert_setenv_channel_closed "2a: no unaudited name travels by --setenv"

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
assert_setenv_channel_closed "2b: no unaudited name travels by --setenv"

# The Phase 2b entrypoint is a multi-line `sh -c` script that also launches the
# egress shim. The prologue is prepended to it, and nothing else in the suite
# looks at that string — removing it leaves every other assertion green.
# Scan the raw NUL-separated capture: the entrypoint is a multi-line script,
# so a line-oriented view of the argv would only ever see its last line.
rc=1; grep -qzF -- "$_PILOT_SECRET_PROLOGUE" "$CAPTURE" && rc=0
assert_true "2b: entrypoint script carries the secret prologue" "$rc"

# A credential that is NOT one of the shapes CRED_PATTERN knows. It must never
# reach --setenv either; the channel assertion is what catches it.
export NPM_TOKEN="npm_notarealtokenjustashape000000000000"
export ATLASSIAN_API_TOKEN="ATATT3xFfGF0notarealtokenjustashape"
run_sandboxed /bin/true >/dev/null
assert_setenv_channel_closed "2b: an unknown-vendor secret cannot ride --setenv"
unset NPM_TOKEN ATLASSIAN_API_TOKEN

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
export MIKA_TEST_SECRET="$FAKE_TOKEN"
exec 9>>"$TRACE"
BASH_XTRACEFD=9
set -x
_run_pilot_sandboxed /bin/true >/dev/null 2>&1 || true
echo "sentinel-after-sandbox-call" >/dev/null
set +x
exec 9>&-
unset MIKA_TEST_SECRET

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
_PILOT_SANDBOX_SECRET_ALLOWLIST=(MIKA_TEST_SECRET MIKA_TEST_SECOND_SECRET)
export MIKA_TEST_SECOND_SECRET="$SECOND_TOKEN"

run_sandboxed /bin/true >/dev/null

bd_count=$(captured_args | grep -c -- '--ro-bind-data' || true)
assert_eq "two --ro-bind-data arguments emitted" "2" "$bd_count"

dest_count=$(secret_destinations | sort -u | wc -l | tr -d ' ')
assert_eq "two distinct secret destinations emitted" "2" "$dest_count"

rc=1; grep -qx -- "/run/mika-pilot-secrets/MIKA_TEST_SECRET" <<<"$(secret_destinations)" && rc=0
assert_true "first secret keeps its own destination path" "$rc"
rc=1; grep -qx -- "/run/mika-pilot-secrets/MIKA_TEST_SECOND_SECRET" <<<"$(secret_destinations)" && rc=0
assert_true "second secret gets its own destination path" "$rc"

rc=1; captured_has_credential && rc=0
assert_eq "neither secret value reaches the argv" "1" "$rc"

unset MIKA_TEST_SECOND_SECRET
_PILOT_SANDBOX_SECRET_ALLOWLIST=(MIKA_TEST_SECRET)

# ============================================================================
# Test 9: no secret set → no secret channel, invocation still valid
# ============================================================================
echo ""
echo "Test: unset secret emits no secret channel"
echo "---------------------------------------------"

rc=0
( unset MIKA_TEST_SECRET; _run_pilot_sandboxed /bin/true ) >/dev/null 2>&1 || rc=$?
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

secret_via_setenv=1
prev=""
while IFS= read -r a; do
    if [ "$prev" = "--setenv" ] && [ "$a" = "MIKA_TEST_SECRET" ]; then secret_via_setenv=0; fi
    prev="$a"
done < <(captured_args)
assert_eq "the secret is NOT passed via --setenv" "1" "$secret_via_setenv"

# ============================================================================
# Test 10b (mika#2056): GH_TOKEN present in the parent env NEVER reaches the
# sandbox argv, --setenv list, or --ro-bind-data channel, under the PRODUCTION
# (empty) secret allowlist. This is the argv-side of the mika#2056 invariant;
# the real-sandbox env/fs absence is Test 12 + the sibling suite.
# ============================================================================
echo ""
echo "Test: GH_TOKEN in parent env does not enter the sandbox (mika#2056)"
echo "-------------------------------------------------------------------"

GH_LEAK_TOKEN="ghp_2056205620562056205620562056205620562056"
# Save/restore the harness-injected synthetic allowlist around the check.
_SAVED_ALLOWLIST=("${_PILOT_SANDBOX_SECRET_ALLOWLIST[@]}")
_PILOT_SANDBOX_SECRET_ALLOWLIST=()   # production shape: no sandbox-held secret
: > "$CAPTURE"
( export GH_TOKEN="$GH_LEAK_TOKEN"; _run_pilot_sandboxed /bin/true ) >/dev/null 2>&1 || true

rc=1; grep -qzF -- "$GH_LEAK_TOKEN" "$CAPTURE" && rc=0
assert_eq "GH_TOKEN value is absent from the bwrap argv" "1" "$rc"

gh_tok_setenv=1
prev=""
while IFS= read -r a; do
    if [ "$prev" = "--setenv" ] && [ "$a" = "GH_TOKEN" ]; then gh_tok_setenv=0; fi
    prev="$a"
done < <(captured_args)
assert_eq "GH_TOKEN is NOT passed via --setenv" "1" "$gh_tok_setenv"

rc=1; grep -qzx -- "/run/mika-pilot-secrets/GH_TOKEN" "$CAPTURE" && rc=0
assert_eq "no /run/mika-pilot-secrets/GH_TOKEN file channel is created" "1" "$rc"

bd_idx_prod=$(arg_index '--ro-bind-data')
assert_eq "empty allowlist emits no --ro-bind-data secret channel" "-1" "$bd_idx_prod"

_PILOT_SANDBOX_SECRET_ALLOWLIST=("${_SAVED_ALLOWLIST[@]}")

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
# Test 12: real bwrap — the generic secret channel still delivers, AND (the
# mika#2056 core) GH_TOKEN in the parent env NEVER reaches the real sandbox.
# ============================================================================
# This is the ONLY scenario that starts a real process. It restores the real
# bwrap binary first; the daemon stubs stay in place so no egress proxy or
# mitmproxy helper is launched. Skipped when bwrap is not installed.
echo ""
echo "Test: real bwrap — channel delivers a synthetic secret; GH_TOKEN stays out"
echo "--------------------------------------------------------------------------"

unset -f bwrap
if ! command -v bwrap >/dev/null 2>&1; then
    echo "  ⊘ skipped — bwrap not installed on PATH"
else
    mkdir -p "$HOME/.mika/data/pilot-transcripts"
    # (a) The generic mika#2039 channel still works, proven with a SYNTHETIC
    #     secret in the allowlist (harness set _PILOT_SANDBOX_SECRET_ALLOWLIST
    #     = (MIKA_TEST_SECRET)).
    got_token=$(MIKA_TEST_SECRET="$FAKE_TOKEN" _run_pilot_sandboxed \
        /bin/sh -c 'printf %s "${MIKA_TEST_SECRET:-<absent>}"' 2>/dev/null)
    assert_eq "sandbox sees a listed secret via the file channel" "$FAKE_TOKEN" "$got_token"

    got_perms=$(MIKA_TEST_SECRET="$FAKE_TOKEN" _run_pilot_sandboxed \
        /bin/sh -c 'ls -l /run/mika-pilot-secrets/MIKA_TEST_SECRET | cut -c1-10' 2>/dev/null)
    assert_eq "secret file is mode 0600 inside the sandbox" "-rw-------" "$got_perms"

    # (b) mika#2056: with the PRODUCTION (empty) allowlist, GH_TOKEN present in
    #     the parent env is ABSENT from the real sandbox environment, and no
    #     file materialises under /run/mika-pilot-secrets. This is the real
    #     end-to-end proof the token cannot be read from inside the sandbox.
    _SAVED_ALLOWLIST=("${_PILOT_SANDBOX_SECRET_ALLOWLIST[@]}")
    _PILOT_SANDBOX_SECRET_ALLOWLIST=()
    got_gh=$(GH_TOKEN="$FAKE_TOKEN" _run_pilot_sandboxed \
        /bin/sh -c 'printf %s "${GH_TOKEN:-<absent>}"' 2>/dev/null)
    assert_eq "GH_TOKEN is ABSENT from the real sandbox environment" "<absent>" "$got_gh"

    got_gh_file=$(GH_TOKEN="$FAKE_TOKEN" _run_pilot_sandboxed \
        /bin/sh -c '[ -e /run/mika-pilot-secrets/GH_TOKEN ] && echo present || echo missing' 2>/dev/null)
    assert_eq "no /run/mika-pilot-secrets/GH_TOKEN inside the real sandbox" "missing" "$got_gh_file"
    _PILOT_SANDBOX_SECRET_ALLOWLIST=("${_SAVED_ALLOWLIST[@]}")

    # Phase 2b is the branch every real dispatch takes when the egress proxy is
    # up, and its entrypoint is a materially different string — the prologue is
    # concatenated with the shim launch and a printf '%q'-quoted argv. Running
    # it for real is the only way a quoting fault in that composition surfaces
    # before production.
    if [ -x "$_PILOT_EGRESS_PROXY_BIN" ] && [ -S "$_PILOT_EGRESS_SOCK" ]; then
        MOCK_EGRESS_RC=0
        got_2b=$(MIKA_TEST_SECRET="$FAKE_TOKEN" _run_pilot_sandboxed \
            /bin/sh -c 'printf %s "${MIKA_TEST_SECRET:-<absent>}"' 2>/dev/null)
        assert_eq "Phase 2b: sandbox sees a listed secret via the file channel" \
            "$FAKE_TOKEN" "$got_2b"
        MOCK_EGRESS_RC=1
    else
        echo "  ⊘ Phase 2b real-bwrap run skipped — egress proxy not running"
        echo "    (argv-level Phase 2b coverage above still applies)"
    fi
fi

# Restore the mock: anything appended after this point must not reach the real
# binary with $HOME pointed at the temp root.
bwrap() {
    printf '%s\0' "$@" > "$CAPTURE"
    return "$MOCK_BWRAP_RC"
}

# ============================================================================
echo ""
echo "===================================================="
echo "Results: $PASS passed, $FAIL failed"
echo "===================================================="

[ "$FAIL" -eq 0 ] || exit 1
exit 0

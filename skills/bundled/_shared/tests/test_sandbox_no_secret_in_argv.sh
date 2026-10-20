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
# mika#2056 invariant directly: no GitHub token VALUE reaches the argv, the
# --setenv list, or the real sandbox env/filesystem. Since mika#2572 the NAME
# GH_TOKEN does travel, carrying only a non-secret value (gh refuses to start
# without some local value) — so these tests assert on the value, never on the
# name's absence. Since mika#2716 that value is the per-spawn EGRESS SENTINEL
# `proxy-managed-no-secret-<32 hex>`, drawn and registered host-side, so the
# assertions match a FORM ($SENTINEL_FORM below) rather than a literal. The
# sandbox-absent-but-host-present
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

# mika#2716: the one accepted shape for the GH_TOKEN value. The anchor is
# WIDENED, never relaxed — the prefix is pinned, the suffix is pinned to exactly
# 32 lowercase hex digits, and both ends are anchored. A bare prefix also
# passes, because that is the fail-open and MIKA_PILOT_EGRESS_SENTINEL=0 shape
# and refusing it would make this suite red on an operator's deliberate
# rollback. What still fails is every value that matters: a real token, an empty
# value, a prefix-shaped decoy with the wrong suffix length.
SENTINEL_FORM='^proxy-managed-no-secret(-[0-9a-f]{32})?$'

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
# mika#2049 — the fabricated relay's listener dies with the temp tree.
trap 'if declare -F stop_fake_egress_relay >/dev/null 2>&1; then stop_fake_egress_relay; fi; rm -rf "$TMPROOT"' EXIT

CAPTURE="$TMPROOT/bwrap-argv"
export HOME="$TMPROOT/home"
WORKTREE_DIR="$TMPROOT/worktree"
mkdir -p "$HOME" "$WORKTREE_DIR"

# mika#2108 — the SECOND user of the `--ro-bind-data` mechanism, present on
# purpose.
#
# `_pilot_sandbox_readme_args` materialises a README-SANDBOX at the two sandbox
# locations that answer falsely, and it reads these two globals. Without them it
# takes its silence path and emits nothing — which would leave this suite
# measuring a mechanism with a single user, exactly the world the destination
# filter below was written to leave behind. Set here so the nominal argv carries
# FOUR `--ro-bind-data` of which TWO are secret channels: that divergence is the
# measure that tells a repaired predicate from an extinguished one, and Test 8's
# positive control reads both numbers.
#
# The paths are FICTITIOUS and under this suite's own temp root — never the real
# host workspace. A test that names the real `/data/workspace/...` would bind a
# marker into a real directory's namespace shadow and read as though the suite
# needed the host layout to exist.
PLATFORM_DIR="$TMPROOT/zz-host"
SUB_REPO_DIR="$TMPROOT/zz-host/mika"

# mika#2049 — the default FLIPPED, and the meaning of `1` changed with it.
#
# Before: `1` → Phase 2a (fs cut only, network open), the suite's default shape.
# Now:    `1` → the relay is unavailable → `_run_pilot_sandboxed` REFUSES (78)
#              and never reaches bwrap. There is no Phase 2a any more.
#
# So the default is `0` (relay serving, full Phase 2b), which is the only shape
# a dispatch can now take, and `1` is used exactly once below — to assert the
# refusal itself.
MOCK_EGRESS_RC=0
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
# mika#2049 — the fabricated relay supplies the artefacts Phase 2b binds (a real
# unix socket and a real proxy file); bwrap refuses a `--bind` whose source does
# not exist, so without them the real-bwrap section below would fail for a
# reason unrelated to this suite.
# shellcheck source=skills/bundled/_shared/tests/lib-fake-egress-relay.sh
source "$SCRIPT_DIR/lib-fake-egress-relay.sh"
stub_serving_egress_relay "$TMPROOT"

# …but this suite drives availability itself, so the helper's hard `return 0` is
# replaced by one that honours MOCK_EGRESS_RC. The artefacts and the stamp
# no-ops it installed are kept.
_ensure_pilot_egress_proxy() {
    _PILOT_EGRESS_ABORT=""
    [ "$MOCK_EGRESS_RC" -eq 0 ] || _PILOT_EGRESS_ABORT="egress_bind_timeout"
    return "$MOCK_EGRESS_RC"
}
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
#
# mika#2572 added GH_TOKEN, audited on the same terms as ANTHROPIC_API_KEY:
#   GH_TOKEN             since mika#2716, the per-spawn EGRESS SENTINEL
#                        "proxy-managed-no-secret-<32 hex>" — drawn host-side
#                        and registered under ~/.mika/pilot-egress-sentinels/,
#                        never a credential and never derived from one
#                        (scripts/verify-no-secret-in-setenv.sh rule 3 pins the
#                        provenance). The GitHub addon strips and replaces
#                        Authorization host-side; the value exists so gh does
#                        not stop on `gh auth login`, and so a later flight can
#                        tell a pilot's own gh from a test that overwrote this
#                        name. Test 10b pins that EVERY occurrence matches
#                        $SENTINEL_FORM.
AUDITED_SETENV_NAMES="ANTHROPIC_API_KEY ANTHROPIC_BASE_URL ANTHROPIC_LOG_FILE \
CLAUDE_CODE_API_BASE_URL GH_TOKEN GIT_CONFIG_GLOBAL GIT_CONFIG_NOSYSTEM \
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

# mika#2108 (vol 2): THE SECRET CHANNEL IS IDENTIFIED BY ITS DESTINATION, NEVER
# BY THE `--ro-bind-data` LITERAL.
#
# `--ro-bind-data` was a sufficient discriminant for exactly as long as the
# secret channel was its only user. mika#2108 gives the mechanism a second use —
# a `README-SANDBOX` materialised at the two mount-skeleton locations that answer
# falsely — and from that moment the literal names a MECHANISM, not a CHANNEL.
# Six predicates of this suite read it; left as they were, three would be false
# (an exact count of 2 against four occurrences) and three would be FORTUITOUS:
# `--perms 0600 immediately precedes` and `comes after --tmpfs /run` would be
# measured on whichever triple happens to come first in the argv, and the two
# `== -1` absence checks would assert the absence of the mechanism instead of
# the absence of the channel — a green that no longer means what it says.
#
# The discriminant already exists and is already named: `$_PILOT_SECRET_DIR_SANDBOX`.
# It is read from dispatch-lib, not retyped here, so a move of that directory
# moves the reader with it. This is the mika#2309 doctrine: the predicate belongs
# on the site that carries the invariant, never on the value or the literal.
#
# Nothing below is relaxed by this. Each predicate keeps its ticket of origin
# (mika#2039, mika#2572, mika#2716) and the property that ticket pinned; only the
# READER changes. And because a filter is exactly the kind of gesture that turns
# a test green by emptying it, Test 8 carries its POSITIVE CONTROL: the filter
# must FIND the two secret channels in the nominal case, and distinguish them
# from the raw count. Without it, both `== -1` assertions would be green at
# vacuum the day the accessor stopped recognising anything.
secret_channel_indices() {
    local -a a=()
    local x i
    while IFS= read -r x; do a+=("$x"); done < <(captured_args)
    for ((i = 0; i < ${#a[@]}; i++)); do
        if [ "${a[$i]}" = "--ro-bind-data" ] && [ $((i + 2)) -lt ${#a[@]} ]; then
            case "${a[$((i + 2))]}" in
                "$_PILOT_SECRET_DIR_SANDBOX"/*) printf '%s\n' "$i" ;;
            esac
        fi
    done
}

# Index (0-based) of the FIRST secret channel's `--ro-bind-data`, or -1.
#
# It replaces a generic `arg_index <exact argument>` helper, which is DELETED
# rather than left standing: it was used only on this literal, and a generic
# first-exact-match reader sitting next to `--ro-bind-data` is an invitation to
# re-measure the mechanism and call it the channel — the defect this accessor
# exists to end. Reads the whole list rather than piping to `head -1`: an early-closed
# pipe takes SIGPIPE and, under this suite's `pipefail`, 141 would become the
# value of the substitution (scripts/verify-no-sigpipe-grep.sh, mika#2055).
secret_channel_index() {
    local -a idx=()
    local i
    while IFS= read -r i; do idx+=("$i"); done < <(secret_channel_indices)
    if [ "${#idx[@]}" -eq 0 ]; then
        printf '%s\n' "-1"
    else
        printf '%s\n' "${idx[0]}"
    fi
}

# How many secret channels the argv carries. Replaces `grep -c -- '--ro-bind-data'`,
# which since mika#2108 counts the mechanism's every user.
secret_channel_count() {
    local -a idx=()
    local i
    while IFS= read -r i; do idx+=("$i"); done < <(secret_channel_indices)
    printf '%s\n' "${#idx[@]}"
}

# Destinations of each SECRET `--ro-bind-data <fd> <dest>` triple. Counting raw
# occurrences of the secret directory would over-count: the entrypoint
# prologue string legitimately names that directory too. Same filter as
# `secret_channel_indices` above, and for the same reason.
secret_destinations() {
    local -a a=()
    local x i
    while IFS= read -r x; do a+=("$x"); done < <(captured_args)
    for ((i = 0; i < ${#a[@]}; i++)); do
        if [ "${a[$i]}" = "--ro-bind-data" ] && [ $((i + 2)) -lt ${#a[@]} ]; then
            case "${a[$((i + 2))]}" in
                "$_PILOT_SECRET_DIR_SANDBOX"/*) printf '%s\n' "${a[$((i + 2))]}" ;;
            esac
        fi
    done
}

run_sandboxed() {
    local rc=0
    MIKA_TEST_SECRET="$FAKE_TOKEN" _run_pilot_sandboxed "$@" >/dev/null 2>&1 || rc=$?
    echo "$rc"
}

# ============================================================================
# Test 1: egress unavailable — the launch is REFUSED, so there is no argv at all
# ============================================================================
# This scenario used to be "Phase 2a (fs cut only) — argv carries no
# credential". Phase 2a no longer exists (mika#2049 made the posture
# fail-closed), so the shape it asserted on can no longer be built. The
# scenario is CONVERTED rather than deleted, and the converted form is strictly
# stronger: "no bwrap argv at all" implies "no credential in the bwrap argv".
#
# It is also the assertion this suite owes the fail-closed posture — the argv
# channel is the one mika#2039 guards, and the surest way for a secret not to
# reach it is for no process to be launched.
echo ""
echo "Test: egress unavailable — the launch is refused, no argv is built"
echo "-------------------------------------------------------------------"

: > "$CAPTURE"   # a stale capture from an earlier run would make this vacuous
MOCK_EGRESS_RC=1
# Called directly rather than through `run_sandboxed`, which wraps it in a
# command substitution: `$( … )` is a subshell, so `$_PILOT_SANDBOX_REFUSAL`
# would never come back and the reason assertion below could only ever read an
# empty string. The variable is deliberately non-`local` in dispatch-lib for
# exactly this reason — the caller is where the operator-facing text is built.
_PILOT_SANDBOX_REFUSAL=""
rc=0
MIKA_TEST_SECRET="$FAKE_TOKEN" _run_pilot_sandboxed /bin/true >/dev/null 2>&1 || rc=$?

assert_eq "refusal: _run_pilot_sandboxed returns 78 (containment refusal)" "78" "$rc"
assert_eq "refusal: bwrap was never invoked, so no argv exists to leak" \
    "0" "$(wc -c < "$CAPTURE" | tr -d ' ')"
rc=1
case "$_PILOT_SANDBOX_REFUSAL" in *"egress relay is not serving"*) rc=0 ;; esac
assert_true "refusal: the reason names the relay, not the worktree" "$rc"

MOCK_EGRESS_RC=0

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

rc=1; [ "$(secret_channel_index)" != "-1" ] && rc=0
assert_true "2b: argv carries a secret file channel under \$_PILOT_SECRET_DIR_SANDBOX" "$rc"
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

# mika#2049 — was `=1` (back to the Phase 2a default). Phase 2a is gone, and
# leaving `1` here would make every test below this line a refusal with no argv
# to inspect: nine scenarios silently asserting on a stale capture.
MOCK_EGRESS_RC=0

# ============================================================================
# Test 3: --perms 0600 immediately precedes --ro-bind-data
# ============================================================================
echo ""
echo "Test: secret file is created 0600"
echo "----------------------------------"

run_sandboxed /bin/true >/dev/null
bd_idx=$(secret_channel_index)
perms_ok=1
if [ "$bd_idx" != "-1" ] && [ "$bd_idx" -ge 2 ]; then
    mapfile -t ARGS < <(captured_args)
    if [ "${ARGS[$((bd_idx - 2))]}" = "--perms" ] && [ "${ARGS[$((bd_idx - 1))]}" = "0600" ]; then
        perms_ok=0
    fi
fi
assert_true "--perms 0600 immediately precedes the secret channel" "$perms_ok"

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
assert_true "the secret channel comes after --tmpfs /run" "$order_ok"

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
#
# mika#2049 excluded exactly one line here — `++ printf ' %q' /bin/true` — because
# Phase 2b re-quoted the PAYLOAD argv with `quoted_argv=$(printf ' %q' "$@")`.
# mika#2077 removed that re-quoting: the argv travels in positional parameters,
# so the excluded line has no producer left and the exclusion is gone with it.
# NOTHING is excluded here any more — ANY `++ printf` fails. Which makes this
# assertion a second guard on the mika#2057 defect: putting a `$(printf ...)`
# back on the entrypoint path reddens it here as well as in
# tests/test_pilot_phase2b_argv.sh.
_printf_lines=$(grep -E -- '\+\+ printf' "$TRACE" || true)
rc=1
[ -n "$_printf_lines" ] && rc=0
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

bd_count=$(secret_channel_count)
assert_eq "two secret file channels emitted" "2" "$bd_count"

# POSITIVE CONTROL of the mika#2108 destination filter, and it is not optional.
# An accessor that stopped recognising anything would make the assertion above
# fail loudly (`0` against `2`), but it would make the two `== -1` absence
# assertions below — Test 9 and Test 10b — green FOR THE WRONG REASON: they
# would be asserting that a broken reader finds nothing, not that no secret
# channel exists. So the filter must be shown FINDING what it is meant to find,
# and shown to be a sub-selection of the mechanism rather than a rename of it.
# The second number is the whole point of the re-specification: before mika#2108
# the two counts are equal (2 of 2), after it they diverge (2 of 4), and the
# divergence is what distinguishes a repaired predicate from an extinguished one.
bd_raw=$(captured_args | grep -c -- '--ro-bind-data' || true)
positive_ok=1
if [ "$bd_count" -eq 2 ] && [ "$bd_raw" -ge "$bd_count" ]; then positive_ok=0; fi
assert_true "positive control: the filter finds both secret channels, and never more than the argv carries ($bd_count of $bd_raw --ro-bind-data) (mika#2108)" "$positive_ok"

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

bd_idx_unset=$(secret_channel_index)
assert_eq "unset token: no secret file channel emitted" "-1" "$bd_idx_unset"

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
# Test 10b (mika#2056, mika#2572): a GH_TOKEN VALUE present in the parent env
# NEVER reaches the sandbox argv, --setenv list, or --ro-bind-data channel,
# under the PRODUCTION (empty) secret allowlist. The NAME does travel since
# mika#2572, and every occurrence must carry the placeholder. This is the argv-side of the mika#2056 invariant;
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

# Every `--setenv GH_TOKEN <value>` triple carries an egress sentinel, and there
# is at least one (the mika#2572 fix is present on this path). A count of zero
# would mean this block no longer runs Phase 2b and proves nothing.
gh_tok_occ=0; gh_tok_bad=""
_args=()
while IFS= read -r a; do _args+=("$a"); done < <(captured_args)
for ((i = 0; i + 2 < ${#_args[@]}; i++)); do
    if [ "${_args[$i]}" = "--setenv" ] && [ "${_args[$((i + 1))]}" = "GH_TOKEN" ]; then
        gh_tok_occ=$((gh_tok_occ + 1))
        [[ "${_args[$((i + 2))]}" =~ $SENTINEL_FORM ]] || gh_tok_bad="$gh_tok_bad ${_args[$((i + 2))]}"
    fi
done
unset _args
assert_eq "GH_TOKEN travels via --setenv on the contained path (mika#2572)" "1" "$([ "$gh_tok_occ" -ge 1 ] && echo 1 || echo 0)"
assert_eq "every --setenv GH_TOKEN carries an egress sentinel, never the parent token" "" "$gh_tok_bad"

rc=1; grep -qzx -- "/run/mika-pilot-secrets/GH_TOKEN" "$CAPTURE" && rc=0
assert_eq "no /run/mika-pilot-secrets/GH_TOKEN file channel is created" "1" "$rc"

bd_idx_prod=$(secret_channel_index)
assert_eq "empty allowlist emits no secret file channel" "-1" "$bd_idx_prod"

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

    # (b) mika#2056: with the PRODUCTION (empty) allowlist, the GH_TOKEN VALUE
    #     present in the parent env is absent from the real sandbox — the
    #     environment carries only the mika#2572 placeholder, since mika#2716 an
    #     egress sentinel built on it — and no file materialises under
    #     /run/mika-pilot-secrets. This is the real end-to-end proof the token
    #     cannot be read from inside the sandbox.
    _SAVED_ALLOWLIST=("${_PILOT_SANDBOX_SECRET_ALLOWLIST[@]}")
    _PILOT_SANDBOX_SECRET_ALLOWLIST=()
    got_gh=$(GH_TOKEN="$FAKE_TOKEN" _run_pilot_sandboxed \
        /bin/sh -c 'printf %s "${GH_TOKEN:-<absent>}"' 2>/dev/null)
    rc=0; [[ "$got_gh" =~ $SENTINEL_FORM ]] || rc=1
    assert_eq "GH_TOKEN in the real sandbox is an egress sentinel (mika#2572, mika#2716)" "0" "$rc"
    rc=1; [ "$got_gh" = "$FAKE_TOKEN" ] && rc=0
    assert_eq "the parent GH_TOKEN value never reaches the real sandbox" "1" "$rc"

    got_gh_file=$(GH_TOKEN="$FAKE_TOKEN" _run_pilot_sandboxed \
        /bin/sh -c '[ -e /run/mika-pilot-secrets/GH_TOKEN ] && echo present || echo missing' 2>/dev/null)
    assert_eq "no /run/mika-pilot-secrets/GH_TOKEN inside the real sandbox" "missing" "$got_gh_file"
    _PILOT_SANDBOX_SECRET_ALLOWLIST=("${_SAVED_ALLOWLIST[@]}")

    # Phase 2b is the branch every real dispatch takes when the egress proxy is
    # up, and its entrypoint is a materially different script — the prologue is
    # concatenated with the shim launch, its bounded wait and the EXIT trap that
    # reaps it. Since mika#2077 the argv is NOT part of that composition: it
    # enters in positional parameters, and tests/test_pilot_phase2b_argv.sh
    # measures its traversal octet for octet. What stays worth running for real
    # here is the composition itself, under bwrap.
    if [ -x "$_PILOT_EGRESS_PROXY_BIN" ] && [ -S "$_PILOT_EGRESS_SOCK" ]; then
        MOCK_EGRESS_RC=0
        got_2b=$(MIKA_TEST_SECRET="$FAKE_TOKEN" _run_pilot_sandboxed \
            /bin/sh -c 'printf %s "${MIKA_TEST_SECRET:-<absent>}"' 2>/dev/null)
        assert_eq "Phase 2b: sandbox sees a listed secret via the file channel" \
            "$FAKE_TOKEN" "$got_2b"
        # mika#2049 — stays at 0; `1` would refuse, not degrade.
        MOCK_EGRESS_RC=0
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

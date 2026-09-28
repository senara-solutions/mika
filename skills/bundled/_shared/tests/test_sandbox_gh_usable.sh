#!/bin/bash
# mika#2572: the gh CLI is usable inside the pilot sandbox — it leaves for the
# network instead of stopping locally on `gh auth login`.
#
# WHY THIS TEST HAS THE SHAPE IT HAS. mika#2056 removed GH_TOKEN from the
# sandbox, which was right, and put nothing in its place. gh refuses to send a
# single request without SOME local token, so every `gh` call in every pilot
# died on `gh auth login` before reaching the egress proxy that would have
# authenticated it. The argv looked correct and every credential-absence suite
# stayed green for a month; only a gh invocation inside a REAL sandbox shows it.
# So, like test_sandbox_git_usable.sh (mika#2141), this suite launches a real
# bwrap through the real `_run_pilot_sandboxed`. Argv-only assertions are out of
# contract here.
#
# What a harness can and cannot prove. The fabricated relay (lib-fake-egress-
# relay.sh) serves no HTTPS, so a successful authenticated call is out of reach
# — that is the post-deploy probe's job, against the real mitmdump. What the
# harness proves is the discriminant between the two states, measured on
# gh 2.92.0:
#
#   PLACEHOLDER   `gh auth token` prints it; `gh api user` attempts the request
#                 (`Get "https://api.github.com/user": …`) and fails on the
#                 unreachable proxy — exit 1.
#   NO TOKEN      `gh auth token` fails with `no oauth token`; `gh api user`
#                 prints `gh auth login` and exits 4 (gh: authentication
#                 required) without any network attempt.
#
# Both halves run in the same suite, because either alone is vacuous: a gh that
# fails early for an unrelated reason (unwritable config dir, missing binary)
# would pass a check that only looks for the absence of `gh auth login`, and a
# negative control that never ran would pass every positive check.
#
# ISOLATION. Each half runs in its own `bash` process that sources its
# dispatch-lib.sh FIRST and calls `stub_serving_egress_relay` AFTER. Sourcing a
# dispatch-lib.sh after the stub in the same shell would restore the real
# `_PILOT_EGRESS_SOCK` / `_PILOT_EGRESS_PROXY_BIN` / `_ensure_pilot_egress_proxy`
# and bind the live relay socket, which lib-fake-egress-relay.sh forbids.
#
# Companions, neither subsuming this one:
#   test_sandbox_no_secret_in_argv.sh          — no token VALUE in the argv
#   test-pilot-github-token-not-in-sandbox.sh  — no token VALUE in env / fs
#
# Run: bash skills/bundled/_shared/tests/test_sandbox_gh_usable.sh
# Expected: all assertions pass, exit 0. Skips cleanly when bwrap or gh is
# absent, or when gh is not visible inside the sandbox.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"
PLACEHOLDER="proxy-managed-no-secret"

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
assert_not_contains() {
    local label="$1" needle="$2" haystack="$3"
    if [[ "$haystack" != *"$needle"* ]]; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    expected NOT to contain: '$needle'"; echo "    actual:                  '$haystack'"
    fi
}

if ! command -v bwrap >/dev/null 2>&1; then
    echo "⊘ skipped — bwrap not installed on PATH (mika#2572 needs a real sandbox)"
    exit 0
fi
if ! command -v gh >/dev/null 2>&1; then
    echo "⊘ skipped — gh not installed on PATH"
    exit 0
fi

TMPROOT=$(mktemp -d "${TMPDIR:-/tmp}/mika2572-XXXXXX")
trap 'rm -rf "$TMPROOT"' EXIT

# The negative control's library: the live file minus the ONE line under test.
MUTATED_LIB="$TMPROOT/dispatch-lib-no-gh-placeholder.sh"
grep -vF -- "--setenv GH_TOKEN \"$PLACEHOLDER\"" "$DISPATCH_LIB" > "$MUTATED_LIB"
removed=$(( $(wc -l < "$DISPATCH_LIB") - $(wc -l < "$MUTATED_LIB") ))

echo ""
echo "PRECONDITION — the negative control differs by exactly the line under test"
echo "--------------------------------------------------------------------------"
assert_eq "the mutated library drops exactly one --setenv GH_TOKEN line" "1" "$removed"

# Run a payload inside a real sandbox built from library $1, in a fresh bash
# process (see ISOLATION above). Stdout and stderr of the payload are returned;
# dispatch-lib's own diagnostics are dropped.
run_in_sandbox() {
    local lib="$1" payload="$2" run_root
    run_root=$(mktemp -d "$TMPROOT/run-XXXXXX")
    bash -c '
        set -uo pipefail
        lib="$1"; helper="$2"; root="$3"; payload="$4"
        export HOME="$root/home"
        WORKTREE_DIR="$root/worktree"
        mkdir -p "$HOME/.mika/data/pilot-transcripts" "$WORKTREE_DIR"
        # A GitHub token in the PARENT environment, so the positive half also
        # shows that what gh sees is the placeholder and never this value.
        export GH_TOKEN="ghp_2572parent2572parent2572parent2572pa"
        # shellcheck source=/dev/null
        source "$lib"
        # shellcheck source=skills/bundled/_shared/tests/lib-fake-egress-relay.sh
        source "$helper"
        trap stop_fake_egress_relay EXIT
        stub_serving_egress_relay "$root"
        _ensure_pilot_helper() { return 1; }
        _PILOT_SANDBOX_SECRET_ALLOWLIST=()
        _run_pilot_sandboxed /bin/sh -c "$payload" 2>&1 | grep -v "^dispatch-lib: "
    ' _ "$lib" "$SCRIPT_DIR/lib-fake-egress-relay.sh" "$run_root" "$payload"
}

# Probe payload. `timeout` bounds gh in case a future relay accepts and never
# answers; every field is on its own tagged line so assertions can pick them.
PROBE='
echo "alive"
if ! command -v gh >/dev/null 2>&1; then echo "gh=missing"; exit 0; fi
echo "gh=present"
tok=$(gh auth token 2>&1); echo "token_rc=$?"; echo "token_out=$tok"
api=$(timeout 30 gh api user 2>&1); echo "api_rc=$?"; echo "api_out=$api"
'

field() { printf '%s\n' "$2" | sed -n "s/^$1=//p" | head -1; }

echo ""
echo "POSITIVE — with the placeholder, gh leaves for the network"
echo "-----------------------------------------------------------"
OUT_POS=$(run_in_sandbox "$DISPATCH_LIB" "$PROBE")
assert_contains "the sandbox itself runs" "alive" "$OUT_POS"
if [ "$(field gh "$OUT_POS")" != "present" ]; then
    echo "⊘ skipped — gh is not visible inside the sandbox"
    exit 0
fi
assert_eq "gh auth token exits 0" "0" "$(field token_rc "$OUT_POS")"
assert_eq "gh auth token prints the placeholder, never the parent token" "$PLACEHOLDER" "$(field token_out "$OUT_POS")"
assert_not_contains "gh api user does not stop on gh auth login" "gh auth login" "$OUT_POS"
assert_not_contains "gh api user does not report a logged-out host" "not logged into any GitHub hosts" "$OUT_POS"
assert_contains "gh api user attempts the request to api.github.com" 'Get "https://api.github.com/user"' "$OUT_POS"
assert_not_contains "gh api user does not exit with gh auth-required (4)" "api_rc=4" "$OUT_POS"

echo ""
echo "NEGATIVE CONTROL — without the placeholder, gh stops locally"
echo "--------------------------------------------------------------"
OUT_NEG=$(run_in_sandbox "$MUTATED_LIB" "$PROBE")
assert_contains "the sandbox itself runs" "alive" "$OUT_NEG"
assert_eq "gh is visible in the mutated sandbox too" "present" "$(field gh "$OUT_NEG")"
assert_contains "gh auth token finds no token" "no oauth token" "$OUT_NEG"
assert_contains "gh api user stops on gh auth login" "gh auth login" "$OUT_NEG"
assert_eq "gh api user exits with gh auth-required (4)" "4" "$(field api_rc "$OUT_NEG")"
assert_not_contains "gh api user makes no request to api.github.com" 'Get "https://api.github.com/user"' "$OUT_NEG"

echo ""
echo "===================================================="
echo "Results: $PASS passed, $FAIL failed"
echo "===================================================="
[ "$FAIL" -eq 0 ] || exit 1
exit 0

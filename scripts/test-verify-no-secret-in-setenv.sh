#!/bin/bash
# Test suite for scripts/verify-no-secret-in-setenv.sh (mika#2039).
#
# A guard that has never been seen failing is not a guard. This suite pins the
# lint's NEGATIVE behaviour, so a later refactor that makes it exit 0
# unconditionally — a moved source file, a broken array extraction, a
# swallowed grep — is caught instead of shipping as a green check.
#
# Each case runs the lint against a fixture copy of dispatch-lib.sh, mutated
# in exactly one way. The lint accepts the fixture path as its first argument
# for this purpose; CI passes none.
#
# Run: bash scripts/test-verify-no-secret-in-setenv.sh
# Expected: all assertions pass, exit 0.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LINT="$REPO_ROOT/scripts/verify-no-secret-in-setenv.sh"
LIVE="$REPO_ROOT/skills/bundled/_shared/dispatch-lib.sh"

PASS=0
FAIL=0
TMPROOT=$(mktemp -d "${TMPDIR:-/tmp}/mika2039-lint-XXXXXX")
trap 'rm -rf "$TMPROOT"' EXIT

# Run the lint on a fixture; echo "<exit>|<output>".
run_lint() {
    local fixture="$1" out rc=0
    out=$(bash "$LINT" "$fixture" 2>&1) || rc=$?
    printf '%s|%s' "$rc" "$out"
}

assert_exit() {
    local label="$1" expected="$2" result="$3"
    local actual="${result%%|*}"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    expected exit: $expected"
        echo "    actual exit:   $actual"
        echo "    output: ${result#*|}"
    fi
}

assert_mentions() {
    local label="$1" needle="$2" result="$3"
    if printf '%s' "${result#*|}" | grep -q -- "$needle"; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label — output does not mention '$needle'"
        echo "    output: ${result#*|}"
    fi
}

fixture() {
    local name="$1"
    cp "$LIVE" "$TMPROOT/$name"
    printf '%s' "$TMPROOT/$name"
}

# ============================================================================
echo ""
echo "Test: the live, corrected tree is clean"
echo "----------------------------------------"
assert_exit "corrected tree: exit 0" "0" "$(run_lint "$LIVE")"

# ============================================================================
echo ""
echo "Test: anti-vacuity — the pre-fix form on origin/main is rejected"
echo "-----------------------------------------------------------------"
PREFIX_FIXTURE="$TMPROOT/dispatch-lib.prefix.sh"
if git -C "$REPO_ROOT" show origin/main:skills/bundled/_shared/dispatch-lib.sh \
        > "$PREFIX_FIXTURE" 2>/dev/null; then
    R=$(run_lint "$PREFIX_FIXTURE")
    assert_exit "pre-fix form: exit 1" "1" "$R"
    assert_mentions "pre-fix form: names GH_TOKEN" "GH_TOKEN" "$R"
else
    echo "  ⊘ skipped — origin/main not fetched in this checkout"
fi

# ============================================================================
echo ""
echo "Test: PATH stays legitimate (the PAT substring must not match it)"
echo "-------------------------------------------------------------------"
R=$(run_lint "$LIVE")
assert_exit "PATH in the allowlist: still exit 0" "0" "$R"

# ============================================================================
echo ""
echo "Test: a credential-shaped name added to the allowlist is rejected"
echo "-------------------------------------------------------------------"
F=$(fixture "npm-token.sh")
perl -0pi -e 's/(_PILOT_SANDBOX_ENV_ALLOWLIST=\(\n)/$1    NPM_TOKEN\n/' "$F"
R=$(run_lint "$F")
assert_exit "NPM_TOKEN in allowlist: exit 1" "1" "$R"
assert_mentions "NPM_TOKEN in allowlist: names it" "NPM_TOKEN" "$R"

# ============================================================================
echo ""
echo "Test: a name matching NO pattern is still rejected (deny-by-default)"
echo "----------------------------------------------------------------------"
F=$(fixture "sentry-dsn.sh")
perl -0pi -e 's/(_PILOT_SANDBOX_ENV_ALLOWLIST=\(\n)/$1    SENTRY_DSN\n/' "$F"
R=$(run_lint "$F")
assert_exit "SENTRY_DSN in allowlist: exit 1" "1" "$R"
assert_mentions "SENTRY_DSN: caught by the literal-set rule" "SENTRY_DSN" "$R"

# ============================================================================
echo ""
echo "Test: a literal --setenv outside the allowlist is caught by the net"
echo "---------------------------------------------------------------------"
F=$(fixture "net-setenv.sh")
perl -0pi -e 's/(--setenv NO_PROXY "localhost,127\.0\.0\.1")/$1\n            --setenv NPM_TOKEN "\$NPM_TOKEN"/' "$F"
R=$(run_lint "$F")
assert_exit "--setenv NPM_TOKEN in net_setenv_args: exit 1" "1" "$R"
assert_mentions "net_setenv_args leak: names NPM_TOKEN" "NPM_TOKEN" "$R"

# ============================================================================
echo ""
echo "Test: the ANTHROPIC_API_KEY exemption is conditional on the placeholder"
echo "-------------------------------------------------------------------------"
F=$(fixture "real-anthropic-key.sh")
perl -pi -e 's/--setenv ANTHROPIC_API_KEY "proxy-managed-no-secret"/--setenv ANTHROPIC_API_KEY "\$MIKA_ANTHROPIC_API_KEY"/' "$F"
R=$(run_lint "$F")
assert_exit "real key replacing the placeholder: exit 1" "1" "$R"
assert_mentions "real key: names the placeholder it lost" "proxy-managed-no-secret" "$R"

# ============================================================================
echo ""
echo "Test: removing the secret allowlist is rejected"
echo "------------------------------------------------"
F=$(fixture "no-secret-list.sh")
perl -0pi -e 's/_PILOT_SANDBOX_SECRET_ALLOWLIST=\(\n[^)]*\)\n//' "$F"
R=$(run_lint "$F")
assert_exit "missing secret allowlist: exit 1" "1" "$R"
assert_mentions "missing secret allowlist: names it" "_PILOT_SANDBOX_SECRET_ALLOWLIST" "$R"

# ============================================================================
echo ""
echo "Test: a secret present in BOTH lists is rejected"
echo "-------------------------------------------------"
F=$(fixture "both-lists.sh")
perl -0pi -e 's/(_PILOT_SANDBOX_ENV_ALLOWLIST=\(\n)/$1    GH_TOKEN\n/' "$F"
R=$(run_lint "$F")
assert_exit "GH_TOKEN in both lists: exit 1" "1" "$R"

# ============================================================================
echo ""
echo "Test: a moved or missing source file fails loudly, never silently green"
echo "------------------------------------------------------------------------"
R=$(run_lint "$TMPROOT/does-not-exist.sh")
assert_exit "missing target: exit 1" "1" "$R"
assert_mentions "missing target: says what to update" "update TARGET" "$R"

# ============================================================================
echo ""
echo "===================================================="
echo "Results: $PASS passed, $FAIL failed"
echo "===================================================="

[ "$FAIL" -eq 0 ] || exit 1
exit 0

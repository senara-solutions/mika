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
echo "Test: anti-vacuity — the pre-fix shape is rejected"
echo "---------------------------------------------------"
# Synthesized from the live file, NOT read from `origin/main`. That ref is the
# pre-fix file only until this work merges; afterwards it IS the corrected file
# and the assertions below would invert, turning main red on every subsequent
# PR. A fixture built by mutation states the property without depending on
# where history happens to be standing.
F=$(fixture "prefix-gh-token.sh")
perl -0pi -e 's/(_PILOT_SANDBOX_ENV_ALLOWLIST=\(\n)/$1    GH_TOKEN\n/' "$F"
R=$(run_lint "$F")
assert_exit "GH_TOKEN back on the --setenv allowlist: exit 1" "1" "$R"
assert_mentions "pre-fix shape: names GH_TOKEN" "GH_TOKEN" "$R"

# ============================================================================
echo ""
echo "Test: PATH survives the delimited PAT match — and the delimiter is load-bearing"
echo "--------------------------------------------------------------------------------"
# Running the lint on the unmodified file proves nothing here: PATH only ever
# reaches Rule 1's set-equality check, never Rule 2's name pattern. Force it
# through Rule 2 by planting a literal `--setenv PATH`, then show the
# delimiter is what saves it by removing the delimiter and re-running.
F=$(fixture "literal-setenv-path.sh")
perl -0pi -e 's/(--setenv NO_PROXY "localhost,127\.0\.0\.1")/$1\n            --setenv PATH "\$PATH"/' "$F"
R=$(run_lint "$F")
assert_exit "literal --setenv PATH: exit 0" "0" "$R"

BROKEN_LINT="$TMPROOT/verify-undelimited.sh"
sed -E "s/\\(\\^\\|_\\)PAT\\(_\\|\\\$\\)/PAT/" "$LINT" > "$BROKEN_LINT"
if grep -q "PASSWD|PAT'" "$BROKEN_LINT"; then
    rc=0; out=$(bash "$BROKEN_LINT" "$F" 2>&1) || rc=$?
    assert_exit "undelimited PAT would reject PATH: exit 1" "1" "$rc|$out"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ could not build the undelimited-pattern mutant — the delimiter"
    echo "    assertion below would be vacuous, so this is a failure, not a skip."
fi

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
echo "Test: an append to the allowlist cannot hide from the parser"
echo "-------------------------------------------------------------"
# `extract_array` anchors on `^NAME=(` and stops at the first `)`. Without a
# fail-closed rule, `NAME+=(...)` on a later line puts secrets back in the argv
# with every guard green — reproduced during review.
F=$(fixture "append-allowlist.sh")
printf '_PILOT_SANDBOX_ENV_ALLOWLIST+=(NPM_TOKEN ATLASSIAN_API_TOKEN)\n' >> "$F"
R=$(run_lint "$F")
assert_exit "+=( ) append: exit 1" "1" "$R"
assert_mentions "append: says the form cannot be audited" "cannot audit" "$R"

F=$(fixture "second-assignment.sh")
printf '_PILOT_SANDBOX_ENV_ALLOWLIST=(NPM_TOKEN)\n' >> "$F"
R=$(run_lint "$F")
assert_exit "second plain assignment: exit 1" "1" "$R"

# ============================================================================
echo ""
echo "Test: a --setenv whose name is not a bare literal is rejected"
echo "--------------------------------------------------------------"
# Rule 2 is a text scan. `--setenv "$var"` and a backslash line-continuation
# before the name are both invisible to it — reproduced during review — so the
# non-literal forms fail closed instead.
F=$(fixture "dynamic-setenv.sh")
perl -0pi -e 's/(--setenv NO_PROXY "localhost,127\.0\.0\.1")/$1\n            --setenv "\$_extra" "\$\{!_extra\}"/' "$F"
R=$(run_lint "$F")
assert_exit "dynamic --setenv \"\$var\": exit 1" "1" "$R"

F=$(fixture "continuation-setenv.sh")
perl -0pi -e 's/(--setenv NO_PROXY "localhost,127\.0\.0\.1")/$1\n            --setenv \\\n            NPM_TOKEN "\$NPM_TOKEN"/' "$F"
R=$(run_lint "$F")
assert_exit "line-continuation before the name: exit 1" "1" "$R"

# ============================================================================
echo ""
echo "Test: the ANTHROPIC_API_KEY exemption is per-occurrence"
echo "--------------------------------------------------------"
# A whole-file grep for the placeholder still passes when a SECOND occurrence
# carrying a real key is added alongside it — reproduced during review.
F=$(fixture "second-anthropic-key.sh")
perl -0pi -e 's/(--setenv ANTHROPIC_API_KEY "proxy-managed-no-secret")/$1\n            --setenv ANTHROPIC_API_KEY "\$MIKA_PILOT_ANTHROPIC_KEY"/' "$F"
R=$(run_lint "$F")
assert_exit "second ANTHROPIC_API_KEY alongside the placeholder: exit 1" "1" "$R"

# ============================================================================
echo ""
echo "Test: prose mentioning --setenv does not trip the lint"
echo "-------------------------------------------------------"
# This file explains itself in comments. A doc line naming the very pattern it
# forbids must not fail CI.
F=$(fixture "comment-mentions-setenv.sh")
printf '# Never write --setenv GH_TOKEN by hand; use the secret allowlist.\n' >> "$F"
R=$(run_lint "$F")
assert_exit "comment naming --setenv GH_TOKEN: exit 0" "0" "$R"

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

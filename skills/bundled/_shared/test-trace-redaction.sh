#!/bin/bash
# Test fixture: verifies that _scrub_secrets_from_output redacts known patterns.
# Run: bash skills/bundled/_shared/test-trace-redaction.sh
# Exit 0 on success, 1 on failure.
#
# mika#903 — regression test for BASH_XTRACEFD secret leakage.

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Source only the _scrub_secrets_from_output function from dispatch-lib.sh.
# We use DISPATCH_LIB_TEST_MODE to avoid side effects from the rest of the lib.
eval "$(sed -n '/^_scrub_secrets_from_output()/,/^}/p' "$SCRIPT_DIR/dispatch-lib.sh")"

# Simulate trace lines containing secrets (matches what set -x produces)
TEST_INPUT="+ GH_APP_TOKEN=ghs_abc123XYZ_installation_token
+ echo ghs_abc123XYZ_installation_token
+ MIKA_ANTHROPIC_API_KEY=sk-ant-api03-very-secret
+ MIKA_INTERNAL_TOKEN=internal-secret-token
+ GH_TOKEN=ghp_1234567890abcdef
+ ghp_1234567890abcdef
+ github_pat_fine_grained_token_value
+ MIKA_GITHUB_APP_PRIVATE_KEY=base64encodedpemdata
+ normal_command arg1 arg2
+ echo 'safe output with no secrets'"

EXPECTED_PATTERNS=(
    "GH_APP_TOKEN=<REDACTED>"
    "<REDACTED_TOKEN>"
    "<REDACTED_PAT>"
    "MIKA_ANTHROPIC_API_KEY=<REDACTED>"
    "MIKA_INTERNAL_TOKEN=<REDACTED>"
    "MIKA_GITHUB_APP_PRIVATE_KEY=<REDACTED>"
    "GH_TOKEN=<REDACTED>"
    "normal_command arg1 arg2"
    "safe output with no secrets"
)

FORBIDDEN_PATTERNS=(
    "ghs_abc123"
    "github_pat_fine"
    "sk-ant-api03"
    "internal-secret-token"
    "ghp_1234567890"
    "base64encodedpemdata"
)

RESULT=$(echo "$TEST_INPUT" | _scrub_secrets_from_output)

PASS=true
for pattern in "${EXPECTED_PATTERNS[@]}"; do
    if ! echo "$RESULT" | grep -qF "$pattern"; then
        echo "FAIL: expected pattern not found: $pattern"
        PASS=false
    fi
done

for pattern in "${FORBIDDEN_PATTERNS[@]}"; do
    if echo "$RESULT" | grep -qF "$pattern"; then
        echo "FAIL: secret not redacted: $pattern"
        PASS=false
    fi
done

if [ "$PASS" = true ]; then
    echo "PASS: all secret patterns redacted correctly"
    exit 0
else
    echo "FAIL: some patterns were not handled"
    echo "--- Output was ---"
    echo "$RESULT"
    exit 1
fi

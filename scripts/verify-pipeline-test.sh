#!/usr/bin/env bash
# Test fixture for scripts/verify-pipeline.sh (mika#861)
#
# Covers 5-case stress test (A-E) from the issue body + trailer dual-form tests.
# Uses a temporary git repo to simulate different PR shapes and mocks `gh` commands
# via a mock executable placed on PATH ahead of the real gh.
#
# Usage:
#   bash scripts/verify-pipeline-test.sh
#
# Exit codes:
#   0 - all tests passed
#   1 - one or more tests failed

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VERIFY_SCRIPT="$SCRIPT_DIR/verify-pipeline.sh"
PASS=0
FAIL=0
TOTAL=0

# --- Helpers ---

setup_test_repo() {
  TEST_DIR=$(mktemp -d)
  cd "$TEST_DIR"
  git init --initial-branch main -q
  mkdir -p scripts
  # Copy the verify script into the test repo
  cp "$VERIFY_SCRIPT" scripts/verify-pipeline.sh
  chmod +x scripts/verify-pipeline.sh

  # Create initial commit on main
  echo "initial" > README.md
  git add README.md scripts/
  git commit -q -m "initial commit"
}

cleanup_test_repo() {
  cd /
  rm -rf "$TEST_DIR"
}

# Write a mock gh script that responds to repo view and api calls.
# Args: $1 = "documentation" label response or "enhancement" or ""
write_mock_gh() {
  local label_response="${1:-}"
  local mock_gh="$TEST_DIR/gh"
  cat > "$mock_gh" <<'MOCKEOF'
#!/usr/bin/env bash
# Mock gh CLI for verify-pipeline-test.sh
LABEL_RESPONSE="__LABEL_PLACEHOLDER__"

case "$1" in
  repo)
    if [[ "${2:-}" == "view" ]]; then
      echo "test-owner/test-repo"
      exit 0
    fi
    ;;
  api)
    arg2="${2:-}"
    if [[ "$arg2" == repos/test-owner/test-repo/issues/* ]]; then
      if [ -n "$LABEL_RESPONSE" ]; then
        echo "$LABEL_RESPONSE"
      fi
      exit 0
    fi
    ;;
  pr)
    if [[ "${2:-}" == "view" ]]; then
      exit 1
    fi
    ;;
esac
exit 1
MOCKEOF
  # Replace placeholder with actual label value
  sed -i "s|__LABEL_PLACEHOLDER__|$label_response|" "$mock_gh"
  chmod +x "$mock_gh"
}

# Write a mock gh that always fails (simulates no gh / no context)
write_mock_gh_unavailable() {
  local mock_gh="$TEST_DIR/gh"
  cat > "$mock_gh" <<'MOCKEOF'
#!/usr/bin/env bash
exit 1
MOCKEOF
  chmod +x "$mock_gh"
}

run_verify() {
  local pr_body="${1:-}"
  local exit_code=0
  local output
  output=$(PATH="$TEST_DIR:$PATH" GITHUB_PR_BODY="$pr_body" bash scripts/verify-pipeline.sh main 2>&1) || exit_code=$?
  echo "$output"
  return $exit_code
}

assert_pass() {
  local test_name="$1"
  local exit_code="$2"
  local output="$3"
  local expected_pattern="${4:-}"
  TOTAL=$((TOTAL + 1))
  if [ "$exit_code" -ne 0 ]; then
    echo "FAIL: $test_name (expected exit 0, got $exit_code)"
    echo "  output: $output"
    FAIL=$((FAIL + 1))
    return
  fi
  if [ -n "$expected_pattern" ]; then
    if ! echo "$output" | grep -qF "$expected_pattern"; then
      echo "FAIL: $test_name (expected pattern '$expected_pattern' not found in output)"
      echo "  output: $output"
      FAIL=$((FAIL + 1))
      return
    fi
  fi
  echo "PASS: $test_name"
  PASS=$((PASS + 1))
}

assert_fail() {
  local test_name="$1"
  local exit_code="$2"
  local output="$3"
  local expected_pattern="${4:-}"
  TOTAL=$((TOTAL + 1))
  if [ "$exit_code" -eq 0 ]; then
    echo "FAIL: $test_name (expected exit 1, got 0)"
    echo "  output: $output"
    FAIL=$((FAIL + 1))
    return
  fi
  if [ -n "$expected_pattern" ]; then
    if ! echo "$output" | grep -qF "$expected_pattern"; then
      echo "FAIL: $test_name (expected pattern '$expected_pattern' not found in output)"
      echo "  output: $output"
      FAIL=$((FAIL + 1))
      return
    fi
  fi
  echo "PASS: $test_name"
  PASS=$((PASS + 1))
}

# =========================================================================
# Test Cases A-E from mika#861 issue body
# =========================================================================

echo "=== Case A: documentation label + docs-only diff → PASS ==="
setup_test_repo
git checkout -b feat/test -q
mkdir -p docs/solutions
echo "solution" > docs/solutions/test-solution.md
git add docs/solutions/test-solution.md
git commit -q -m "docs: add solution"
write_mock_gh "documentation"
output="" ; exit_code=0
output=$(run_verify "Closes #42") || exit_code=$?
assert_pass "Case A: documentation label + docs-only → PASS" "$exit_code" "$output" "[pipeline-exempt: issue-label] docs-only PR allowed by linked-issue documentation label (#42)"
cleanup_test_repo

echo ""
echo "=== Case B: documentation label + mixed diff → PASS ==="
setup_test_repo
git checkout -b feat/test -q
mkdir -p docs/plans src
echo "plan" > docs/plans/test-plan.md
echo "code" > src/main.rs
git add docs/plans/test-plan.md src/main.rs
git commit -q -m "feat: add plan and code"
write_mock_gh "documentation"
output="" ; exit_code=0
output=$(run_verify "Closes #42") || exit_code=$?
assert_pass "Case B: documentation label + mixed diff → PASS" "$exit_code" "$output" "Pipeline verification passed"
cleanup_test_repo

echo ""
echo "=== Case C: documentation label + source-only diff → FAIL ==="
setup_test_repo
git checkout -b feat/test -q
mkdir -p src
echo "code" > src/main.rs
git add src/main.rs
git commit -q -m "feat: code only"
write_mock_gh "documentation"
output="" ; exit_code=0
output=$(run_verify "Closes #42") || exit_code=$?
assert_fail "Case C: documentation label + source-only → FAIL (asymmetry)" "$exit_code" "$output" "[pipeline-exempt: none] REJECT: code-only PR"
cleanup_test_repo

echo ""
echo "=== Case D: no documentation label + docs-only diff → FAIL ==="
setup_test_repo
git checkout -b feat/test -q
mkdir -p docs/solutions
echo "solution" > docs/solutions/test-solution.md
git add docs/solutions/test-solution.md
git commit -q -m "docs: add solution"
write_mock_gh "enhancement"
output="" ; exit_code=0
output=$(run_verify "Closes #42") || exit_code=$?
assert_fail "Case D: no documentation label + docs-only → FAIL" "$exit_code" "$output" "[pipeline-exempt: none] REJECT: docs-only PR"
cleanup_test_repo

echo ""
echo "=== Case E: no linked issue + docs-only diff → FAIL ==="
setup_test_repo
git checkout -b feat/test -q
mkdir -p docs/solutions
echo "solution" > docs/solutions/test-solution.md
git add docs/solutions/test-solution.md
git commit -q -m "docs: add solution"
write_mock_gh_unavailable
output="" ; exit_code=0
output=$(run_verify "") || exit_code=$?
assert_fail "Case E: no linked issue + docs-only → FAIL" "$exit_code" "$output" "[pipeline-exempt: none] REJECT: docs-only PR"
cleanup_test_repo

# =========================================================================
# Trailer dual-form tests
# =========================================================================

echo ""
echo "=== Trailer: docs-only with reason → PASS (info) ==="
setup_test_repo
git checkout -b feat/test -q
mkdir -p docs/solutions
echo "solution" > docs/solutions/test-solution.md
git add docs/solutions/test-solution.md
git commit -q -m "docs: add solution

Pipeline-Exempt: docs-only — standalone compound shipment"
write_mock_gh "enhancement"
output="" ; exit_code=0
output=$(run_verify "Closes #42") || exit_code=$?
assert_pass "Trailer: docs-only with reason → PASS (info)" "$exit_code" "$output" "[pipeline-exempt: trailer] docs-only PR allowed by Pipeline-Exempt trailer with reason:"
cleanup_test_repo

echo ""
echo "=== Trailer: docs-only bare → PASS (warn) ==="
setup_test_repo
git checkout -b feat/test -q
mkdir -p docs/solutions
echo "solution" > docs/solutions/test-solution.md
git add docs/solutions/test-solution.md
git commit -q -m "docs: add solution

Pipeline-Exempt: docs-only"
write_mock_gh "enhancement"
output="" ; exit_code=0
output=$(run_verify "Closes #42") || exit_code=$?
assert_pass "Trailer: docs-only bare → PASS (warn)" "$exit_code" "$output" "warn: [pipeline-exempt: trailer] bare Pipeline-Exempt: docs-only trailer detected"
cleanup_test_repo

echo ""
echo "=== Trailer: code-only with reason → PASS (info) ==="
setup_test_repo
git checkout -b feat/test -q
mkdir -p src
echo "code" > src/main.rs
git add src/main.rs
git commit -q -m "fix: hotfix

Pipeline-Exempt: code-only — emergency hotfix, docs follow-up filed"
write_mock_gh "enhancement"
output="" ; exit_code=0
output=$(run_verify "Closes #42") || exit_code=$?
assert_pass "Trailer: code-only with reason → PASS (info)" "$exit_code" "$output" "[pipeline-exempt: trailer] code-only PR allowed by Pipeline-Exempt trailer with reason:"
cleanup_test_repo

echo ""
echo "=== Trailer: code-only bare → PASS (warn) ==="
setup_test_repo
git checkout -b feat/test -q
mkdir -p src
echo "code" > src/main.rs
git add src/main.rs
git commit -q -m "fix: hotfix

Pipeline-Exempt: code-only"
write_mock_gh "enhancement"
output="" ; exit_code=0
output=$(run_verify "Closes #42") || exit_code=$?
assert_pass "Trailer: code-only bare → PASS (warn)" "$exit_code" "$output" "warn: [pipeline-exempt: trailer] bare Pipeline-Exempt: code-only trailer detected"
cleanup_test_repo

echo ""
echo "=== Trailer: malformed → FAIL ==="
setup_test_repo
git checkout -b feat/test -q
mkdir -p docs/solutions
echo "solution" > docs/solutions/test-solution.md
git add docs/solutions/test-solution.md
git commit -q -m "docs: add solution

Pipeline-Exempt: docs-only-typo"
write_mock_gh "enhancement"
output="" ; exit_code=0
output=$(run_verify "Closes #42") || exit_code=$?
assert_fail "Trailer: malformed (docs-only-typo) → FAIL" "$exit_code" "$output" "[pipeline-exempt: none] REJECT"
cleanup_test_repo

# =========================================================================
# Label takes priority over trailer (both present)
# =========================================================================

echo ""
echo "=== Priority: label checked before trailer ==="
setup_test_repo
git checkout -b feat/test -q
mkdir -p docs/solutions
echo "solution" > docs/solutions/test-solution.md
git add docs/solutions/test-solution.md
git commit -q -m "docs: add solution

Pipeline-Exempt: docs-only — also has trailer"
write_mock_gh "documentation"
output="" ; exit_code=0
output=$(run_verify "Closes #42") || exit_code=$?
assert_pass "Priority: label wins over trailer" "$exit_code" "$output" "[pipeline-exempt: issue-label]"
# Should NOT contain trailer message since label takes priority
TOTAL=$((TOTAL + 1))
if echo "$output" | grep -qF "[pipeline-exempt: trailer]"; then
  echo "FAIL: Priority test — trailer message should not appear when label exempts"
  FAIL=$((FAIL + 1))
else
  echo "PASS: Priority: label suppresses trailer message"
  PASS=$((PASS + 1))
fi
cleanup_test_repo

# =========================================================================
# Closes keyword variants (Fixes, Resolves)
# =========================================================================

echo ""
echo "=== Fixes keyword parsed ==="
setup_test_repo
git checkout -b feat/test -q
mkdir -p docs/solutions
echo "solution" > docs/solutions/test-solution.md
git add docs/solutions/test-solution.md
git commit -q -m "docs: add solution"
write_mock_gh "documentation"
output="" ; exit_code=0
output=$(run_verify "Fixes #99") || exit_code=$?
assert_pass "Fixes keyword parsed → label exemption (#99)" "$exit_code" "$output" "[pipeline-exempt: issue-label] docs-only PR allowed by linked-issue documentation label (#99)"
cleanup_test_repo

echo ""
echo "=== Resolves keyword parsed ==="
setup_test_repo
git checkout -b feat/test -q
mkdir -p docs/solutions
echo "solution" > docs/solutions/test-solution.md
git add docs/solutions/test-solution.md
git commit -q -m "docs: add solution"
write_mock_gh "documentation"
output="" ; exit_code=0
output=$(run_verify "Resolves #77") || exit_code=$?
assert_pass "Resolves keyword parsed → label exemption (#77)" "$exit_code" "$output" "[pipeline-exempt: issue-label] docs-only PR allowed by linked-issue documentation label (#77)"
cleanup_test_repo

# =========================================================================
# Summary
# =========================================================================

echo ""
echo "========================================="
echo "Results: $PASS passed, $FAIL failed, $TOTAL total"
echo "========================================="

if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
exit 0

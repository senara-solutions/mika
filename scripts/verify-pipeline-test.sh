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

# --- mika#2419: synthetic GitHub Actions event payloads ---
#
# The automated-author exemption reads `.pull_request.user.login` from
# $GITHUB_EVENT_PATH. `run_verify` above deliberately does NOT set that variable
# (it is the fail-closed baseline — see F3), so the author cases need their own
# runner.

# Write an event payload carrying a PR author login. Shape mirrors both what
# GitHub Actions provides on `pull_request` events and what qa-review's Step 2B
# synthesizes.
write_event_with_author() {
  local login="$1"
  printf '{"pull_request":{"number":1,"labels":[],"user":{"login":"%s"}}}' "$login" \
    > "$TEST_DIR/event.json"
}

# Write a TRUNCATED (invalid) JSON event file. F7's subject: under
# `set -euo pipefail`, a naive `jq` call on this would abort the script with
# jq's own exit code, turning a PR that passes today into a non-zero exit that
# qa-review reads without judgment as `block[pipeline]`.
write_malformed_event() {
  printf '{"pull_request":{"user":{"login":' > "$TEST_DIR/event.json"
}

run_verify_with_event() {
  local pr_body="${1:-}"
  local exit_code=0
  local output
  output=$(PATH="$TEST_DIR:$PATH" GITHUB_PR_BODY="$pr_body" \
    GITHUB_EVENT_PATH="$TEST_DIR/event.json" \
    bash scripts/verify-pipeline.sh main 2>&1) || exit_code=$?
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
    if ! grep -qF -- "$expected_pattern" <<<"$output"; then
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
    if ! grep -qF -- "$expected_pattern" <<<"$output"; then
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
cat > docs/plans/test-plan.md <<'PLANEOF'
# Plan: test

## Acceptance criteria

- [ ] AC1. Test criterion
PLANEOF
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
if grep -qF -- "[pipeline-exempt: trailer]" <<<"$output"; then
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
# Acceptance criteria section tests (mika#1600)
# =========================================================================

echo ""
echo "=== AC check: plan with AC section present → PASS ==="
setup_test_repo
git checkout -b feat/test -q
mkdir -p docs/plans src
cat > docs/plans/test-plan.md <<'PLANEOF'
# Plan: test

## Definition of Done

- [ ] Something done

## Acceptance criteria

- [ ] AC1. First criterion
- [ ] AC2. Second criterion
PLANEOF
echo "code" > src/main.rs
git add docs/plans/test-plan.md src/main.rs
git commit -q -m "feat: plan with AC section"
write_mock_gh ""
output="" ; exit_code=0
output=$(run_verify "") || exit_code=$?
assert_pass "AC check: plan with AC section present → PASS" "$exit_code" "$output" "Pipeline verification passed"
cleanup_test_repo

echo ""
echo "=== AC check: plan missing AC section → FAIL ==="
setup_test_repo
git checkout -b feat/test -q
mkdir -p docs/plans src
cat > docs/plans/test-plan.md <<'PLANEOF'
# Plan: test

## Definition of Done

- [ ] Something done
PLANEOF
echo "code" > src/main.rs
git add docs/plans/test-plan.md src/main.rs
git commit -q -m "feat: plan without AC section"
write_mock_gh ""
output="" ; exit_code=0
output=$(run_verify "") || exit_code=$?
assert_fail "AC check: plan missing AC section → FAIL" "$exit_code" "$output" "missing '## Acceptance criteria' section"
cleanup_test_repo

echo ""
echo "=== AC check: plan with empty AC section → FAIL ==="
setup_test_repo
git checkout -b feat/test -q
mkdir -p docs/plans src
cat > docs/plans/test-plan.md <<'PLANEOF'
# Plan: test

## Definition of Done

- [ ] Something done

## Acceptance criteria

## Next section
PLANEOF
echo "code" > src/main.rs
git add docs/plans/test-plan.md src/main.rs
git commit -q -m "feat: plan with empty AC section"
write_mock_gh ""
output="" ; exit_code=0
output=$(run_verify "") || exit_code=$?
assert_fail "AC check: plan with empty AC section → FAIL" "$exit_code" "$output" "empty '## Acceptance criteria' section"
cleanup_test_repo

echo ""
echo "=== AC check: plan with title-case heading → PASS (mika#1639) ==="
setup_test_repo
git checkout -b feat/test -q
mkdir -p docs/plans src
cat > docs/plans/test-plan.md <<'PLANEOF'
# Plan: test

## Definition of Done

- [ ] Something done

## Acceptance Criteria

- [ ] AC1. First criterion
- [ ] AC2. Second criterion
PLANEOF
echo "code" > src/main.rs
git add docs/plans/test-plan.md src/main.rs
git commit -q -m "feat: plan with title-case AC heading"
write_mock_gh ""
output="" ; exit_code=0
output=$(run_verify "") || exit_code=$?
assert_pass "AC check: title-case '## Acceptance Criteria' → PASS" "$exit_code" "$output" "Pipeline verification passed"
cleanup_test_repo

# =========================================================================
# Numbered AC heading (mika#2516) — n=3 of the recurring class documented in
# docs/solutions/workflow-issues/verify-pipeline-ac-heading-case-insensitive-2026-06-30.md
#
# `/ce:plan` numbers its section headings (`## 0.` … `## 11.`), so the AC heading
# arrives as `## 11. Acceptance criteria`. Three PRs in one day (#2509, #2514,
# #2516) needed a one-line hand strip of that number before Pipeline Artifacts
# would go green.
#
# T1 is the load-bearing fixture, and it is the ONLY one that distinguishes a
# complete fix from a half-applied one. The pattern is matched at TWO sites
# seven lines apart (presence `grep`, non-emptiness `sed`). With only the `grep`
# widened, a numbered heading over a FULL section renders
# "empty '## Acceptance criteria' section" — the false negative is not repaired,
# it is moved, and its message becomes misleading. T2 cannot play that role:
# on the same half-applied fix it would PASS, since "empty" is its expected
# message.
# =========================================================================

echo ""
echo "=== T1: numbered AC heading + content → PASS (mika#2516; both matchers moved) ==="
setup_test_repo
git checkout -b feat/test -q
mkdir -p docs/plans src
cat > docs/plans/test-plan.md <<'PLANEOF'
# Plan: test

## 10. Definition of Done

- [ ] Something done

## 11. Acceptance criteria

- [ ] AC1. First criterion
- [ ] AC2. Second criterion
PLANEOF
echo "code" > src/main.rs
git add docs/plans/test-plan.md src/main.rs
git commit -q -m "feat: plan with numbered AC heading"
write_mock_gh ""
output="" ; exit_code=0
output=$(run_verify "") || exit_code=$?
assert_pass "T1: '## 11. Acceptance criteria' + content → PASS" "$exit_code" "$output" "Pipeline verification passed"
cleanup_test_repo

echo ""
echo "=== T2: numbered AC heading + EMPTY section → FAIL (decision stays strict) ==="
setup_test_repo
git checkout -b feat/test -q
mkdir -p docs/plans src
cat > docs/plans/test-plan.md <<'PLANEOF'
# Plan: test

## 10. Definition of Done

- [ ] Something done

## 11. Acceptance criteria

## 12. Next section
PLANEOF
echo "code" > src/main.rs
git add docs/plans/test-plan.md src/main.rs
git commit -q -m "feat: plan with numbered but empty AC section"
write_mock_gh ""
output="" ; exit_code=0
output=$(run_verify "") || exit_code=$?
assert_fail "T2: numbered heading, empty section → FAIL" "$exit_code" "$output" "empty '## Acceptance criteria' section"
cleanup_test_repo

echo ""
echo "=== T3: numbered AND title-case → PASS (both permissivity axes compose) ==="
# The non-numbered title-case case is already pinned seven lines above by the
# mika#1639 fixture; re-adding it here would be a duplicate regression test.
# What nothing covered is the COMPOSITION — a numbered heading that is also
# title-cased, which is the other half of mika#2516 AC5 ("numbered or not").
setup_test_repo
git checkout -b feat/test -q
mkdir -p docs/plans src
cat > docs/plans/test-plan.md <<'PLANEOF'
# Plan: test

## 10. Definition of Done

- [ ] Something done

## 11. Acceptance Criteria

- [ ] AC1. First criterion
PLANEOF
echo "code" > src/main.rs
git add docs/plans/test-plan.md src/main.rs
git commit -q -m "feat: plan with numbered title-case AC heading"
write_mock_gh ""
output="" ; exit_code=0
output=$(run_verify "") || exit_code=$?
assert_pass "T3: '## 11. Acceptance Criteria' → PASS (mika#1639 × mika#2516)" "$exit_code" "$output" "Pipeline verification passed"
cleanup_test_repo

echo ""
echo "=== T4: 'Notes on Acceptance criteria' only → FAIL (tolerance did not swallow the refusal) ==="
setup_test_repo
git checkout -b feat/test -q
mkdir -p docs/plans src
cat > docs/plans/test-plan.md <<'PLANEOF'
# Plan: test

## 10. Definition of Done

- [ ] Something done

## 11. Notes on Acceptance criteria

- The heading text stays anchored right after the optional number prefix.
PLANEOF
echo "code" > src/main.rs
git add docs/plans/test-plan.md src/main.rs
git commit -q -m "feat: plan whose only AC-ish heading is prefixed by other words"
write_mock_gh ""
output="" ; exit_code=0
output=$(run_verify "") || exit_code=$?
assert_fail "T4: '## 11. Notes on Acceptance criteria' → FAIL (missing)" "$exit_code" "$output" "missing '## Acceptance criteria' section"
cleanup_test_repo

# =========================================================================
# Automated-author exemption (mika#2419)
#
# F1 is the measured case: PR mika#2415, a dependabot Cargo.lock-only bump, was
# verdicted `block[pipeline]` by qa-review. `Cargo.lock` is not under `docs/`,
# `.github/` or `.claude/worktrees/`, so it lands in SOURCE_BUCKET and the PR is
# code-only — a class no label exempts, by design (the docs-only/code-only
# asymmetry protects mika-platform#17). The exemption is therefore on the
# AUTHOR, mirroring `ci.yml`'s branch exclusion on the pipeline-artifacts job.
#
# F2/F6 are the negative controls: without them F1 could pass simply because
# the code-only gate was disarmed for everyone. F5 pins the scope: the
# exemption covers the two bucket rejections and NOT the mika#1600 AC check.
# =========================================================================

echo ""
echo "=== F1: dependabot + Cargo.lock-only → PASS (the mika#2415 case) ==="
setup_test_repo
git checkout -b dependabot/cargo/serde-1.0.0 -q
echo "# lockfile" > Cargo.lock
git add Cargo.lock
git commit -q -m "chore(deps): bump serde from 1.0.0 to 1.0.1"
write_mock_gh ""
write_event_with_author "dependabot[bot]"
output="" ; exit_code=0
output=$(run_verify_with_event "") || exit_code=$?
assert_pass "F1: dependabot + Cargo.lock-only → PASS" "$exit_code" "$output" "[pipeline-exempt: automated-author]"
cleanup_test_repo

echo ""
echo "=== F2: human author + Cargo.lock-only → FAIL (negative control) ==="
setup_test_repo
git checkout -b feat/test -q
echo "# lockfile" > Cargo.lock
git add Cargo.lock
git commit -q -m "chore(deps): bump serde by hand"
write_mock_gh ""
write_event_with_author "samidarko"
output="" ; exit_code=0
output=$(run_verify_with_event "") || exit_code=$?
assert_fail "F2: human author + Cargo.lock-only → FAIL" "$exit_code" "$output" "[pipeline-exempt: none] REJECT: code-only PR"
cleanup_test_repo

echo ""
echo "=== F3: no GITHUB_EVENT_PATH + code-only → FAIL (fail-closed) ==="
setup_test_repo
git checkout -b dependabot/cargo/serde-1.0.0 -q
echo "# lockfile" > Cargo.lock
git add Cargo.lock
git commit -q -m "chore(deps): bump serde from 1.0.0 to 1.0.1"
write_mock_gh ""
output="" ; exit_code=0
output=$(run_verify "") || exit_code=$?
assert_fail "F3: no event file → no exemption (a local run never exempts)" "$exit_code" "$output" "[pipeline-exempt: none] REJECT: code-only PR"
cleanup_test_repo

echo ""
echo "=== F4: app/dependabot recognized as well → PASS ==="
setup_test_repo
git checkout -b dependabot/cargo/serde-1.0.0 -q
echo "# lockfile" > Cargo.lock
git add Cargo.lock
git commit -q -m "chore(deps): bump serde from 1.0.0 to 1.0.1"
write_mock_gh ""
write_event_with_author "app/dependabot"
output="" ; exit_code=0
output=$(run_verify_with_event "") || exit_code=$?
assert_pass "F4: app/dependabot → PASS" "$exit_code" "$output" "[pipeline-exempt: automated-author]"
cleanup_test_repo

echo ""
echo "=== F5: automated author + plan without AC section → FAIL (mika#1600 survives) ==="
setup_test_repo
git checkout -b dependabot/cargo/serde-1.0.0 -q
mkdir -p docs/plans
cat > docs/plans/test-plan.md <<'PLANEOF'
# Plan: test

## Definition of Done

- [ ] Something done
PLANEOF
echo "# lockfile" > Cargo.lock
git add docs/plans/test-plan.md Cargo.lock
git commit -q -m "chore(deps): bump with a plan that has no AC section"
write_mock_gh ""
write_event_with_author "dependabot[bot]"
output="" ; exit_code=0
output=$(run_verify_with_event "") || exit_code=$?
assert_fail "F5: exemption covers the bucket checks only, not mika#1600" "$exit_code" "$output" "missing '## Acceptance criteria' section"
cleanup_test_repo

echo ""
echo "=== F6: substring is not a match → FAIL (negative control) ==="
setup_test_repo
git checkout -b feat/test -q
echo "# lockfile" > Cargo.lock
git add Cargo.lock
git commit -q -m "chore(deps): bump serde"
write_mock_gh ""
write_event_with_author "not-dependabot[bot]"
output="" ; exit_code=0
output=$(run_verify_with_event "") || exit_code=$?
assert_fail "F6: 'not-dependabot[bot]' is not 'dependabot[bot]'" "$exit_code" "$output" "[pipeline-exempt: none] REJECT: code-only PR"
cleanup_test_repo

echo ""
echo "=== F7: malformed event JSON → FAIL with the code-only reject, never a jq abort ==="
setup_test_repo
git checkout -b feat/test -q
echo "# lockfile" > Cargo.lock
git add Cargo.lock
git commit -q -m "chore(deps): bump serde"
write_mock_gh ""
write_malformed_event
output="" ; exit_code=0
output=$(run_verify_with_event "") || exit_code=$?
# The assertion is on the REJECT line, not merely on a non-zero exit: under
# `set -euo pipefail` an aborting `jq` also exits non-zero, so "exit != 0" alone
# would pass while the script had in fact died before reaching any check.
assert_fail "F7: malformed event → fail-closed means CONTINUE without exempting" "$exit_code" "$output" "[pipeline-exempt: none] REJECT: code-only PR"
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

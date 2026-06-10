#!/usr/bin/env bash
# Test fixture for the deploy-info target (mika#1210, mika#1475).
#
# Exercises the deploy-info recipe in a disposable git fixture:
#
# Off-main branch guard (mika#1475):
#   (a) off-main without FORCE — ABORT, exit != 0
#   (b) off-main with FORCE    — WARN, exit 0, continues to freshness check
#   (c) on main                — no guard output, proceeds normally
#
# Freshness check (mika#1210):
#   (d) up-to-date    — "origin/main: up to date"
#   (e) behind        — WARNING with count + behind string
#   (f) unreachable   — "could not reach origin" note, exits zero
#
# Also asserts the fixture recipe (scripts/fixtures/deploy-info-Makefile)
# is byte-identical to the production deploy-info recipe in Makefile — any
# drift fails the test immediately, not at deploy time.
#
# Usage: bash scripts/deploy-info-test.sh
# Exit codes:
#   0 — all checks passed
#   1 — one or more checks failed
#   2 — setup error
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PROD_MAKEFILE="$REPO_ROOT/Makefile"
FIXTURE_MAKEFILE="$REPO_ROOT/scripts/fixtures/deploy-info-Makefile"

[ -r "$PROD_MAKEFILE" ] || { echo "ERROR: $PROD_MAKEFILE not readable" >&2; exit 2; }
[ -r "$FIXTURE_MAKEFILE" ] || { echo "ERROR: $FIXTURE_MAKEFILE not readable" >&2; exit 2; }

PASS=0
FAIL=0

# -------------------------------------------------------------------------
# 1. Byte-identical recipe assertion
# -------------------------------------------------------------------------
# Extract the deploy-info recipe block from the production Makefile (the
# target line + all lines up to but excluding the next blank line). Compare
# against the fixture. Any difference means a single source of truth was
# broken — the test fails here, not later at deploy time.

extract_recipe() {
  awk '
    /^deploy-info:/ { in_block = 1 }
    in_block && /^$/ { exit }
    in_block { print }
  ' "$1"
}

PROD_RECIPE=$(extract_recipe "$PROD_MAKEFILE")
FIXTURE_RECIPE=$(extract_recipe "$FIXTURE_MAKEFILE")

if [ "$PROD_RECIPE" = "$FIXTURE_RECIPE" ]; then
  echo "PASS: fixture deploy-info recipe is byte-identical to production"
  PASS=$((PASS + 1))
else
  echo "FAIL: fixture deploy-info recipe drifted from production"
  echo "Diff (prod vs fixture):"
  diff <(printf '%s\n' "$PROD_RECIPE") <(printf '%s\n' "$FIXTURE_RECIPE") || true
  FAIL=$((FAIL + 1))
fi

# -------------------------------------------------------------------------
# Disposable git fixture setup
# -------------------------------------------------------------------------

TEST_DIR=$(mktemp -d)
trap 'rm -rf "$TEST_DIR"' EXIT

BARE="$TEST_DIR/origin.git"
SEED="$TEST_DIR/seed"
WORK="$TEST_DIR/work"

git init -q --bare "$BARE"

# Seed the bare with an initial commit on main so HEAD is non-empty.
git init -q --initial-branch=main "$SEED"
(
  cd "$SEED"
  git -c user.email=t@e -c user.name=t commit --allow-empty -q -m "initial"
  git remote add origin "$BARE"
  git push -q origin main
)

# Clone fresh working repo from bare; will have HEAD == origin/main.
git clone -q --branch main "$BARE" "$WORK"
(
  cd "$WORK"
  git config user.email "t@e"
  git config user.name "t"
)

# Drop the fixture Makefile into the working clone so we can invoke it
# with `make -f scripts/fixtures/deploy-info-Makefile deploy-info`.
cp "$FIXTURE_MAKEFILE" "$WORK/deploy-info.mk"

run_recipe() {
  (
    cd "$WORK"
    make -s -f deploy-info.mk deploy-info 2>&1
  )
}

assert_contains() {
  local name="$1" expected="$2" output="$3"
  if echo "$output" | grep -qF -- "$expected"; then
    echo "PASS: $name"
    PASS=$((PASS + 1))
  else
    echo "FAIL: $name"
    echo "  expected to contain: $expected"
    echo "  output: $output"
    FAIL=$((FAIL + 1))
  fi
}

assert_not_contains() {
  local name="$1" unexpected="$2" output="$3"
  if echo "$output" | grep -qF -- "$unexpected"; then
    echo "FAIL: $name"
    echo "  expected NOT to contain: $unexpected"
    echo "  output: $output"
    FAIL=$((FAIL + 1))
  else
    echo "PASS: $name"
    PASS=$((PASS + 1))
  fi
}

# =========================================================================
# Off-main branch guard (mika#1475)
# =========================================================================

# -------------------------------------------------------------------------
# 2. Off-main without FORCE — ABORT, exit != 0
# -------------------------------------------------------------------------
(
  cd "$WORK"
  git checkout -q -b test/off-main
)
exit_code=0
out=$(run_recipe) || exit_code=$?
if [ "$exit_code" -ne 0 ]; then
  echo "PASS: off-main without FORCE exits non-zero"
  PASS=$((PASS + 1))
else
  echo "FAIL: off-main without FORCE should exit non-zero, got 0"
  FAIL=$((FAIL + 1))
fi
assert_contains "off-main ABORT message present" "ABORT" "$out"
assert_contains "off-main ABORT names the branch" "test/off-main" "$out"
assert_contains "off-main ABORT names the override" "FORCE_DEPLOY_FROM_BRANCH=1" "$out"
assert_not_contains "off-main ABORT does not reach Building from:" "Building from:" "$out"

# -------------------------------------------------------------------------
# 3. Off-main with FORCE — WARN, exit 0, continues
# -------------------------------------------------------------------------
exit_code=0
out=$(cd "$WORK" && FORCE_DEPLOY_FROM_BRANCH=1 make -s -f deploy-info.mk deploy-info 2>&1) || exit_code=$?
if [ "$exit_code" -eq 0 ]; then
  echo "PASS: off-main with FORCE exits zero"
  PASS=$((PASS + 1))
else
  echo "FAIL: off-main with FORCE should exit zero, got $exit_code"
  FAIL=$((FAIL + 1))
fi
assert_contains "off-main FORCE WARN message present" "WARN" "$out"
assert_contains "off-main FORCE names the branch" "test/off-main" "$out"
assert_contains "off-main FORCE continues to Building from:" "Building from:" "$out"

# Return to main for the freshness tests.
(
  cd "$WORK"
  git checkout -q main
)

# =========================================================================
# Freshness check (mika#1210)
# =========================================================================

# -------------------------------------------------------------------------
# 4. Up-to-date (on main)
# -------------------------------------------------------------------------
out=$(run_recipe)
assert_contains "up-to-date prints freshness OK" "origin/main: up to date" "$out"
assert_contains "up-to-date prints Building from: line" "Building from:" "$out"
assert_not_contains "up-to-date does not print ABORT" "ABORT" "$out"

# -------------------------------------------------------------------------
# 5. Behind (on main)
# -------------------------------------------------------------------------
# Push a new commit to origin without pulling locally.
(
  cd "$SEED"
  git -c user.email=t@e -c user.name=t commit --allow-empty -q -m "advance origin 1"
  git -c user.email=t@e -c user.name=t commit --allow-empty -q -m "advance origin 2"
  git push -q origin main
)
out=$(run_recipe)
assert_contains "behind prints WARNING with count + behind string" \
  "WARNING: HEAD is 2 commits behind origin/main." "$out"
assert_contains "behind prints pull suggestion" \
  "Run 'git pull --ff-only' if you intended to deploy origin/main." "$out"

# -------------------------------------------------------------------------
# 6. Unreachable (on main)
# -------------------------------------------------------------------------
# Point origin at a nonexistent path so `git fetch` fails fast.
(
  cd "$WORK"
  git remote set-url origin "$TEST_DIR/nonexistent.git"
)
# Use a guarded run so a non-zero exit is treated as test failure, not
# script abort under set -e. On-main must exit zero even if origin is
# unreachable.
exit_code=0
out=$(run_recipe) || exit_code=$?
if [ "$exit_code" -eq 0 ]; then
  echo "PASS: unreachable exits zero (recipe does not fail deploy)"
  PASS=$((PASS + 1))
else
  echo "FAIL: unreachable exited $exit_code (expected 0)"
  FAIL=$((FAIL + 1))
fi
assert_contains "unreachable prints could-not-reach-origin note" \
  "NOTE: could not reach origin" "$out"

# -------------------------------------------------------------------------
# Summary
# -------------------------------------------------------------------------
TOTAL=$((PASS + FAIL))
echo ""
echo "Results: $PASS passed, $FAIL failed, $TOTAL total"
if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
exit 0

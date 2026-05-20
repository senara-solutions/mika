#!/usr/bin/env bash
# Test fixture for the deploy-info target (mika#1210).
#
# Exercises the three AC2 outcome paths of the deploy-info recipe in
# a disposable git fixture:
#   (a) up-to-date    — fresh clone of an in-test bare repo,
#                       deploy-info says "origin/main: up to date"
#   (b) behind        — add a commit to origin's main without pulling locally,
#                       deploy-info warns with the correct count + behind string
#   (c) unreachable   — point origin URL to a nonexistent path, deploy-info
#                       prints the "could not reach origin" note and exits zero
#
# AC8 also requires asserting the fixture recipe (scripts/fixtures/deploy-info-Makefile)
# is byte-identical to the production deploy-info recipe in Makefile — any drift
# fails the test immediately, not at deploy time.
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

# -------------------------------------------------------------------------
# 2. AC2.1 — up-to-date
# -------------------------------------------------------------------------
out=$(run_recipe)
assert_contains "AC2.1 up-to-date prints freshness OK" "origin/main: up to date" "$out"
assert_contains "AC2.1 prints Building from: line" "Building from:" "$out"

# -------------------------------------------------------------------------
# 3. AC2.2 — behind
# -------------------------------------------------------------------------
# Push a new commit to origin without pulling locally.
(
  cd "$SEED"
  git -c user.email=t@e -c user.name=t commit --allow-empty -q -m "advance origin 1"
  git -c user.email=t@e -c user.name=t commit --allow-empty -q -m "advance origin 2"
  git push -q origin main
)
out=$(run_recipe)
assert_contains "AC2.2 behind prints WARNING with count + behind string" \
  "WARNING: HEAD is 2 commits behind origin/main." "$out"
assert_contains "AC2.2 behind prints pull suggestion" \
  "Run 'git pull --ff-only' if you intended to deploy origin/main." "$out"

# -------------------------------------------------------------------------
# 4. AC2.3 — unreachable
# -------------------------------------------------------------------------
# Point origin at a nonexistent path so `git fetch` fails fast.
(
  cd "$WORK"
  git remote set-url origin "$TEST_DIR/nonexistent.git"
)
# Use a guarded run so a non-zero exit is treated as test failure, not
# script abort under set -e. AC2 mandates exit-zero on all three paths.
exit_code=0
out=$(run_recipe) || exit_code=$?
if [ "$exit_code" -eq 0 ]; then
  echo "PASS: AC2.3 unreachable exits zero (recipe does not fail deploy)"
  PASS=$((PASS + 1))
else
  echo "FAIL: AC2.3 unreachable exited $exit_code (expected 0)"
  FAIL=$((FAIL + 1))
fi
assert_contains "AC2.3 unreachable prints could-not-reach-origin note" \
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

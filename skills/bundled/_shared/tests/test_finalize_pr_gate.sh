#!/bin/bash
# Test suite for dispatch-lib.sh's mika#1941 finalize-pr gate.
#
# Covers:
#   - _ac8_recent_conventional_commit_title — most-recent conv-commit picking,
#     no-match fallback (empty string), skipping non-conforming subjects
#   - _ac6_verbatim_stats_block — header signature stability, sentinel fallback
#     when git/gh unavailable
#   - _ac7_has_formal_multi_agent_review — signature-keyword detection,
#     trusted-reviewer detection, empty-review-set rejection, malformed-payload
#     handling. Uses `gh` function stub for hermetic testing (no network).
#
# Source isolation audit: safe to source dispatch-lib.sh at test start —
# no top-level imperative code (per test_parse_disposition.sh audit note).
#
# Run: bash skills/bundled/_shared/tests/test_finalize_pr_gate.sh
# Expected: all assertions pass, exit 0.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"

# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

PASS=0
FAIL=0

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

assert_contains() {
    local label="$1" needle="$2" haystack="$3"
    if printf '%s' "$haystack" | grep -qF "$needle"; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    needle:   '$needle'"
        echo "    haystack: '$haystack'"
    fi
}

assert_rc() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label (expected rc $expected; got $actual)"
    fi
}

# Set up a temp worktree directory for fixtures
TMPROOT=$(mktemp -d)
trap 'rm -rf "$TMPROOT"' EXIT

# Helper: initialize a git repo with a series of commits from a file
# Args: $1 — dir, $2+ — commit subjects (one per commit)
init_repo_with_commits() {
    local dir="$1"
    shift
    git -C "$dir" init -q
    git -C "$dir" config user.email 'test@example.com'
    git -C "$dir" config user.name 'Test'
    git -C "$dir" commit --allow-empty -q -m 'initial commit'
    local subj
    for subj in "$@"; do
        git -C "$dir" commit --allow-empty -q -m "$subj"
    done
}

# ============================================================================
# AC8 — _ac8_recent_conventional_commit_title
# ============================================================================
echo "=== AC8: _ac8_recent_conventional_commit_title ==="

# Case 1: single conventional commit at HEAD
CASE1="$TMPROOT/case1"
mkdir -p "$CASE1"
init_repo_with_commits "$CASE1" "fix(engine): sweep NULL-PID phantoms"
result=$(_ac8_recent_conventional_commit_title "$CASE1")
assert_eq "single conv-commit picked" 'fix(engine): sweep NULL-PID phantoms' "$result"

# Case 2: mixed commits — most-recent conv-commit wins over older ones
CASE2="$TMPROOT/case2"
mkdir -p "$CASE2"
init_repo_with_commits "$CASE2" \
    "fix(engine): older fix" \
    "docs(plans): add DoD sections" \
    "feat(dispatch-lib): AC6+AC7+AC8 invariant gate (mika#1941)" \
    "chore: bump version"
result=$(_ac8_recent_conventional_commit_title "$CASE2")
assert_eq "most-recent conv-commit picked (chore over feat over docs)" \
    'chore: bump version' "$result"

# Case 3: HEAD is wip() — falls back to most-recent conv-commit below
CASE3="$TMPROOT/case3"
mkdir -p "$CASE3"
init_repo_with_commits "$CASE3" \
    "feat(x): the real fix" \
    "wip(mika#1383): auto-PR-create rescue"
result=$(_ac8_recent_conventional_commit_title "$CASE3")
assert_eq "wip() skipped, feat picked" 'feat(x): the real fix' "$result"

# Case 4: no conv-commits — returns empty
CASE4="$TMPROOT/case4"
mkdir -p "$CASE4"
init_repo_with_commits "$CASE4" "Merge pull request #123 from foo/bar" "random subject"
result=$(_ac8_recent_conventional_commit_title "$CASE4")
assert_eq "no conv-commits returns empty" '' "$result"

# Case 5: non-git dir — returns empty (safe no-op)
CASE5="$TMPROOT/case5-nogit"
mkdir -p "$CASE5"
result=$(_ac8_recent_conventional_commit_title "$CASE5")
assert_eq "non-git dir returns empty" '' "$result"

# Case 6: breaking-change bang (feat!:) accepted
CASE6="$TMPROOT/case6"
mkdir -p "$CASE6"
init_repo_with_commits "$CASE6" "feat(api)!: remove legacy endpoint"
result=$(_ac8_recent_conventional_commit_title "$CASE6")
assert_eq "conv-commit with bang accepted" 'feat(api)!: remove legacy endpoint' "$result"

# ============================================================================
# AC6 — _ac6_verbatim_stats_block
# ============================================================================
echo ""
echo "=== AC6: _ac6_verbatim_stats_block ==="

# Case 1: worktree not a git repo → sentinel string in git block
CASE_AC6_1="$TMPROOT/ac6-1-nogit"
mkdir -p "$CASE_AC6_1"
# Stub gh for this test — we don't need real API calls
gh() {
    echo '{"error":"gh pr view stubbed"}'
    return 1
}
export -f gh
block=$(_ac6_verbatim_stats_block "$CASE_AC6_1" '1234' 'mika')
assert_contains "AC6 header signature present" \
    'AC6 verbatim ground truth (dispatch-lib finalize gate, mika#1941)' "$block"
assert_contains "non-git sentinel present" 'worktree not a git repo' "$block"
assert_contains "gh pr view command echoed" \
    'gh pr view 1234 --repo senara-solutions/mika --json changedFiles,additions,deletions' "$block"

# Case 2: real git repo + gh stub → git-stat output present
CASE_AC6_2="$TMPROOT/ac6-2"
mkdir -p "$CASE_AC6_2"
init_repo_with_commits "$CASE_AC6_2" "feat: initial"
# Simulate origin/main pointing at initial commit so diff --stat has scope
git -C "$CASE_AC6_2" branch -c main 2>/dev/null || true
git -C "$CASE_AC6_2" update-ref refs/remotes/origin/main HEAD
# Now add a file + commit so diff --stat has content
echo "test content" > "$CASE_AC6_2/new_file.txt"
git -C "$CASE_AC6_2" add new_file.txt
git -C "$CASE_AC6_2" commit -q -m "feat: add file"
block=$(_ac6_verbatim_stats_block "$CASE_AC6_2" '5678' 'mika')
assert_contains "git diff --stat command block present" \
    'git diff --stat origin/main..HEAD' "$block"
assert_contains "new_file.txt appears in stat output" 'new_file.txt' "$block"

# Case 3: missing repo/pr_num → sentinel string in json block
CASE_AC6_3="$TMPROOT/ac6-3-empty"
mkdir -p "$CASE_AC6_3"
block=$(_ac6_verbatim_stats_block "$CASE_AC6_3" '' '')
assert_contains "empty-arg sentinel present" 'pr_num or repo missing' "$block"

# Case 4: strip-then-refresh semantics — awk-based strip should leave PR body
# BEFORE the AC6 block intact and drop trailing blank/`---` lines. This is
# what _finalize_pr_gate uses to refresh a stale block after rebase.
BODY_WITH_AC6='## Summary

Some content here.

Test plan:
- [x] tests pass

Closes #1941

---

## AC6 verbatim ground truth (dispatch-lib finalize gate, mika#1941)

STALE stats block that should be stripped'
stripped=$(printf '%s' "$BODY_WITH_AC6" | awk '
    /^## AC6 verbatim ground truth \(dispatch-lib finalize gate, mika#1941\)$/ { exit }
    { print }
' | awk '
    { lines[NR] = $0 }
    END {
        n = NR
        while (n > 0 && (lines[n] ~ /^[[:space:]]*$/ || lines[n] == "---")) n--
        for (i = 1; i <= n; i++) print lines[i]
    }
')
assert_contains "strip: pre-block content preserved" 'Closes #1941' "$stripped"
if printf '%s' "$stripped" | grep -qF 'AC6 verbatim ground truth'; then
    FAIL=$((FAIL + 1))
    echo "  ✗ strip: AC6 block should be removed (still present)"
else
    PASS=$((PASS + 1))
    echo "  ✓ strip: AC6 block removed"
fi
if printf '%s' "$stripped" | grep -qF 'STALE stats block'; then
    FAIL=$((FAIL + 1))
    echo "  ✗ strip: STALE content should be removed"
else
    PASS=$((PASS + 1))
    echo "  ✓ strip: STALE content removed"
fi
# Trailing `---` must also be stripped so the fresh block's re-added `---` doesn't stack
if printf '%s\n' "$stripped" | tail -1 | grep -q '^---$'; then
    FAIL=$((FAIL + 1))
    echo "  ✗ strip: trailing --- separator should be gone"
else
    PASS=$((PASS + 1))
    echo "  ✓ strip: trailing --- separator removed"
fi

unset -f gh

# ============================================================================
# AC7 — _ac7_has_formal_multi_agent_review
# ============================================================================
echo ""
echo "=== AC7: _ac7_has_formal_multi_agent_review ==="

# Global stub for gh — we control the JSON returned per case
GH_STUB_JSON=''
gh() {
    # Only the `gh api /repos/.../pulls/N/reviews` shape is exercised
    printf '%s' "$GH_STUB_JSON"
}
export -f gh

# Helper: run _ac7 and capture exit code without triggering `set -e`
run_ac7() {
    set +e
    _ac7_has_formal_multi_agent_review "$@"
    local rc=$?
    set -e
    printf '%d' "$rc"
}

# Case 1: signature keyword `/ce:review` in body → present (rc 0)
GH_STUB_JSON='[{"body":"Multi-agent /ce:review complete\n\nVERDICT: pass","user":{"login":"someone"}}]'
rc=$(run_ac7 "mika" "1234")
assert_rc "signature keyword /ce:review detected" 0 "$rc"

# Case 2: signature keyword `adversarial` (case-insensitive) → present
GH_STUB_JSON='[{"body":"Ran ADVERSARIAL lens pass","user":{"login":"anon"}}]'
rc=$(run_ac7 "mika" "1234")
assert_rc "signature keyword adversarial (uppercase) detected" 0 "$rc"

# Case 3: trusted reviewer login `mika-platform-qa[bot]` → present
GH_STUB_JSON='[{"body":"Approved","user":{"login":"mika-platform-qa[bot]"}}]'
rc=$(run_ac7 "mika" "1234")
assert_rc "trusted reviewer mika-platform-qa[bot] detected" 0 "$rc"

# Case 4: trusted reviewer login `ce-code-review-bot` → present
GH_STUB_JSON='[{"body":"","user":{"login":"ce-code-review-bot"}}]'
rc=$(run_ac7 "mika" "1234")
assert_rc "trusted reviewer ce-code-review-bot detected" 0 "$rc"

# Case 5: empty review array → absent (rc 1)
GH_STUB_JSON='[]'
rc=$(run_ac7 "mika" "1234")
assert_rc "empty review array returns absent" 1 "$rc"

# Case 6: reviews present but no signature + no trusted reviewer → absent
GH_STUB_JSON='[{"body":"looks good","user":{"login":"random-user"}}]'
rc=$(run_ac7 "mika" "1234")
assert_rc "informal reviews-only returns absent" 1 "$rc"

# Case 7: malformed payload (object, not array) → error (rc 2)
GH_STUB_JSON='{"error":"not found"}'
rc=$(run_ac7 "mika" "1234")
assert_rc "malformed payload returns error" 2 "$rc"

# Case 8: missing args → error (rc 2)
rc=$(run_ac7 "" "")
assert_rc "missing args returns error" 2 "$rc"

# Case 9: `p1/p2/p3` signature keyword detected
GH_STUB_JSON='[{"body":"P1/P2/P3 findings enumerated","user":{"login":"anon"}}]'
rc=$(run_ac7 "mika" "1234")
assert_rc "signature keyword p1/p2/p3 detected" 0 "$rc"

# Case 10: `multi-agent` signature keyword detected
GH_STUB_JSON='[{"body":"Multi-Agent review complete","user":{"login":"anon"}}]'
rc=$(run_ac7 "mika" "1234")
assert_rc "signature keyword multi-agent detected" 0 "$rc"

unset -f gh

# ============================================================================
# Structural invariant: gate composer wired into dispatch-lib
# ============================================================================
echo ""
echo "=== Structural: functions exist and are callable ==="

for fn in _ac8_recent_conventional_commit_title \
          _ac6_verbatim_stats_block \
          _ac7_has_formal_multi_agent_review \
          _finalize_pr_gate; do
    if declare -F "$fn" >/dev/null; then
        PASS=$((PASS + 1))
        echo "  ✓ $fn defined"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $fn NOT defined"
    fi
done

# ============================================================================
# Summary
# ============================================================================
echo ""
echo "======================================"
echo "Passed: $PASS"
echo "Failed: $FAIL"
echo "======================================"

[ "$FAIL" = 0 ]

#!/bin/bash
# Test suite for the rescue net's closing-reference guard (mika#2157).
#
# `Closes #N` is an instruction GitHub executes automatically on merge. Before
# this guard, dispatch-lib's recovery block wrote it unconditionally — without
# ever looking at what it had captured. A grooming worktree carries, at minimum,
# dispatch-lib's own side effect (two lines in `.claude/groom-verdict-trail.log`),
# and that was enough to open an approved PR whose merge would have closed a p1
# it did not fix (mika-cloud#202 → mika-cloud#192).
#
# Every case below builds a REAL temporary git repository with a real
# `origin/main` ref and runs the real predicate over a real `git diff`. Nothing
# is stubbed: a probe reconstructed from the plan would only test the plan.
#
# Run: bash skills/bundled/_shared/tests/test_rescue_closes_guard.sh
# Expected: all assertions pass, exit 0.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"
QA_PROMPT="$SCRIPT_DIR/../../qa-review/system_prompt.md"

# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

PASS=0
FAIL=0

assert_contains() {
    local label="$1" haystack="$2" needle="$3"
    if grep -qF -- "$needle" <<<"$haystack"; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"; echo "    missing: '$needle'"
    fi
}

assert_not_contains() {
    local label="$1" haystack="$2" needle="$3"
    if grep -qF -- "$needle" <<<"$haystack"; then
        FAIL=$((FAIL + 1)); echo "  ✗ $label"; echo "    unexpectedly present: '$needle'"
    else
        PASS=$((PASS + 1)); echo "  ✓ $label"
    fi
}

assert_eq() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    expected: '$expected'"
        echo "    actual:   '$actual'"
    fi
}

TMP_ROOT=$(mktemp -d)
trap 'rm -rf "$TMP_ROOT"' EXIT

# Recovery-PR body composition reads these from the environment, as the
# pre-extraction heredoc did. Pinned so the metadata block stays deterministic.
export SESSION_ID="test-session" TURNS="7" COST="0.42"

# make_repo — a repo on `main` with one base commit. `origin/main` is planted as
# a bare remote-tracking ref (`update-ref`), which is exactly what a fetched
# branch looks like to `git diff origin/main...HEAD` — no network, no daemon.
make_repo() {
    local dir="$TMP_ROOT/$1"
    mkdir -p "$dir"
    git -C "$dir" init -q -b main
    git -C "$dir" config user.email "test@example.com"
    git -C "$dir" config user.name "test"
    git -C "$dir" config commit.gpgsign false
    mkdir -p "$dir/seed"
    echo base > "$dir/seed/base.txt"
    git -C "$dir" add -A
    git -C "$dir" commit -q --no-verify -m "base"
    git -C "$dir" update-ref refs/remotes/origin/main HEAD
    printf '%s' "$dir"
}

# add_commit <dir> <path>... — write and commit each path on top of HEAD.
add_commit() {
    local dir="$1"; shift
    local p
    for p in "$@"; do
        mkdir -p "$dir/$(dirname "$p")"
        printf 'content for %s\n' "$p" >> "$dir/$p"
        git -C "$dir" add -- "$p"
    done
    git -C "$dir" commit -q --no-verify -m "work"
}

# compose <dir> — the production callsite's argument shape, one arg per line.
compose() {
    _compose_rescue_pr_body "$1" "dirty-worktree" "Class fact sentence." "2157"
}

# ── T1 (AC3) — grooming trail only: the measured mika-cloud#202 case ─────────
echo "-- T1: .claude/groom-verdict-trail.log alone (AC3) --"
R1=$(make_repo t1)
add_commit "$R1" ".claude/groom-verdict-trail.log"
B1=$(compose "$R1")
assert_contains     "T1 body carries a non-closing reference" "$B1" "Refs #2157"
assert_not_contains "T1 body does NOT carry Closes"           "$B1" "Closes #2157"
assert_contains     "T1 marker reads incident-only"           "$B1" "<!-- rescue-diff: incident-only -->"
assert_eq "T1 first line announces that no fix is carried (AC2)" \
    "> **This recovery carries no fix.** Every file in the captured diff is a" \
    "$(head -1 <<<"$B1")"

# ── T2 (AC4) — a real source file: the net must stay armed ──────────────────
echo "-- T2: a real source file (AC4) --"
R2=$(make_repo t2)
add_commit "$R2" "crates/mika-agent/src/foo.rs"
B2=$(compose "$R2")
assert_contains     "T2 body closes the issue"          "$B2" "Closes #2157"
assert_not_contains "T2 body does NOT downgrade to Refs" "$B2" "Refs #2157"
assert_contains     "T2 marker reads carries-work"      "$B2" "<!-- rescue-diff: carries-work -->"
assert_not_contains "T2 body carries no no-fix lede"    "$B2" "This recovery carries no fix."

# ── T3 — an incident artefact riding along with real work disarms nothing ──
echo "-- T3: plan + source together --"
R3=$(make_repo t3)
add_commit "$R3" "docs/plans/2026-09-03-001-x-plan.md" "crates/mika-agent/src/foo.rs"
B3=$(compose "$R3")
assert_contains "T3 body closes the issue"     "$B3" "Closes #2157"
assert_contains "T3 marker reads carries-work" "$B3" "<!-- rescue-diff: carries-work -->"

# ── T4 — a dev-pilot that produced only a plan has fixed nothing ───────────
echo "-- T4: a plan file alone --"
R4=$(make_repo t4)
add_commit "$R4" "docs/plans/2026-09-03-001-x-plan.md"
B4=$(compose "$R4")
assert_contains     "T4 body carries a non-closing reference" "$B4" "Refs #2157"
assert_not_contains "T4 body does NOT carry Closes"           "$B4" "Closes #2157"

# ── T5 — an unmeasurable diff falls on the Refs side (fail-closed, D2) ─────
echo "-- T5: no origin/main ref, diff unmeasurable --"
R5="$TMP_ROOT/t5"
mkdir -p "$R5"
git -C "$R5" init -q -b main
git -C "$R5" config user.email "test@example.com"
git -C "$R5" config user.name "test"
git -C "$R5" config commit.gpgsign false
add_commit "$R5" "crates/mika-agent/src/foo.rs"
B5=$(compose "$R5")
assert_contains     "T5 body carries a non-closing reference" "$B5" "Refs #2157"
assert_not_contains "T5 body does NOT carry Closes"           "$B5" "Closes #2157"

# ── T6 — an empty diff cannot satisfy any AC ───────────────────────────────
echo "-- T6: empty diff --"
R6=$(make_repo t6)
B6=$(compose "$R6")
assert_contains     "T6 body carries a non-closing reference" "$B6" "Refs #2157"
assert_not_contains "T6 body does NOT carry Closes"           "$B6" "Closes #2157"

# ── T7 — a non-ASCII path must not escape the classifier ───────────────────
# Under git's default `core.quotePath=true`, `git diff --name-only` returns
# `"docs/plans/\303\251tude-plan.md"` — quoted, octal-escaped, matching no case
# pattern — and the predicate answered "carries work". A fail-OPEN on the exact
# input this repo produces daily: its plans and tickets are written in French.
# This case exists because the first implementation shipped that bug.
echo "-- T7: an accented plan filename (quotePath fail-open) --"
R7=$(make_repo t7)
add_commit "$R7" "docs/plans/2026-09-03-001-étude-plan.md"
B7=$(compose "$R7")
assert_contains     "T7 body carries a non-closing reference" "$B7" "Refs #2157"
assert_not_contains "T7 body does NOT carry Closes"           "$B7" "Closes #2157"

# ── T8 — the rescue's own scaffold exclusions are incident artefacts ────────
# `.claude/claude-pilot.json` is copied into the worktree from $PLATFORM_DIR and
# is excluded from the rescue commit's `git add -A` by name — dispatch-lib calls
# it a scaffold path in its own NOTE line. A commit-pushed-no-pr rescue carrying
# nothing but that file has fixed nothing.
echo "-- T8: a scaffold path alone --"
R8=$(make_repo t8)
add_commit "$R8" ".claude/claude-pilot.json"
B8=$(compose "$R8")
assert_contains     "T8 body carries a non-closing reference" "$B8" "Refs #2157"
assert_not_contains "T8 body does NOT carry Closes"           "$B8" "Closes #2157"

# ── T9 — an empty worktree dir must not measure the dispatch host's checkout ─
# `git -C ""` silently runs against the dispatch process CWD. Without the guard
# the predicate would answer from a live checkout — and if that checkout carries
# real work, it answers "carries work" for a rescue that captured nothing.
echo "-- T9: empty worktree dir --"
if _rescue_diff_carries_work "" 2>/dev/null; then
    FAIL=$((FAIL + 1)); echo "  ✗ T9 empty worktree dir is refused"
else
    PASS=$((PASS + 1)); echo "  ✓ T9 empty worktree dir is refused"
fi

# ── Structural: the inline heredoc is gone and the callsite calls the fn ────
echo "-- structural: the producing callsite --"
create_block=$(awk '/RESCUED_PR_URL=\$\(gh pr create/,/2>&9 \|\| true\)/' "$DISPATCH_LIB")
assert_contains "gh pr create composes its body through the function" \
    "$create_block" '_compose_rescue_pr_body "$WORKTREE_DIR"'
assert_not_contains "no inline RESCUEBODY heredoc survives at the callsite" \
    "$create_block" "RESCUEBODY"

# The only `Closes #` that dispatch-lib can still PRODUCE lives inside the
# carries-work branch. Anything else is a regression to the unconditional shape.
producing=$(grep -n 'Closes #' "$DISPATCH_LIB" | grep -v '^[0-9]*: *#' || true)
assert_eq "exactly one producing 'Closes #' site remains" "1" \
    "$(printf '%s\n' "$producing" | grep -c . || true)"
assert_contains "and it is the carries-work assignment" "$producing" \
    'issue_ref="Closes #${issue_num}"'

# ── Phase 3.7 — prompt contract for the review side (AC5) ──────────────────
# This pins the PRESENCE of the review-side item, not the model's adherence to
# it. `feedback_prompt_enforcement_fragile` is right: a prompt is not a
# structure. What is structural here is AC1 — after this fix, a mistakenly
# approved incident-only PR closes nothing, because the instruction is gone.
# This check is a guard against silent deletion, and nothing more.
echo "-- prompt contract: qa-review Step 1.5 incident item (AC5) --"
qa_prompt_text=$(cat "$QA_PROMPT")
assert_contains "qa-review reads the incident-only marker" \
    "$qa_prompt_text" "rescue-diff: incident-only"
assert_contains "qa-review refuses an incident-only diff with a non-approving hold" \
    "$qa_prompt_text" "emit \`VERDICT: hold[review]\` with the reason that the diff consists entirely of grooming"
# The token matters, and so does the reason it is not the other one: block[ac]
# structurally dispatches an AC-fix pilot run (verdict_handler.rs handle_block_ac,
# bounded at BLOCK_AC_MAX_RETRIES=3). Pin the measurement so a future edit that
# "upgrades" the verdict has to confront it rather than rediscover it.
assert_contains "and records why block[ac] is the wrong token here" \
    "$qa_prompt_text" "BLOCK_AC_MAX_RETRIES"
assert_contains "and names an approval on such a diff a false positive" \
    "$qa_prompt_text" "is a false positive, not an opinion"

echo
echo "PASS: $PASS  FAIL: $FAIL"
[ "$FAIL" -eq 0 ] || exit 1

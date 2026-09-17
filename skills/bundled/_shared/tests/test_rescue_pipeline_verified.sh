#!/bin/bash
# Test suite for the `rescue-pipeline-verified` producer (mika#2354).
#
# Two gates read `<!-- rescue-pipeline-verified: yes -->` — qa-review Step 1.5
# and `wip_rescue` (mika#2286) — and until this ticket NO site in the repo ever
# wrote `yes`. `_compose_rescue_pr_body` wrote the literal `no` unconditionally,
# so the only path to a green light was a human editing the PR body. The drain
# could not be autonomous: not because it broke, but because the marker that
# unblocks it had no producer.
#
# Every case below builds a REAL temporary git repository — with a real
# `origin/main` ref, a real (tiny) cargo crate, and a real copy of
# `scripts/verify-pipeline.sh` — and runs the real measurement over it. Nothing
# is stubbed, for the reason `test_rescue_closes_guard.sh` states about its own
# probes: a measurement reconstructed from the plan would only test the plan.
# Here it matters twice over, because the whole point of mika#2354 is that `yes`
# attests an EXECUTION and not a shape (mika#2286's lesson).
#
# Run: bash skills/bundled/_shared/tests/test_rescue_pipeline_verified.sh
# Expected: all assertions pass, exit 0.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"
REAL_VERIFY_PIPELINE="$REPO_ROOT/scripts/verify-pipeline.sh"

# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

PASS=0
FAIL=0

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

if [ ! -x "$REAL_VERIFY_PIPELINE" ]; then
    echo "FATAL: $REAL_VERIFY_PIPELINE is absent or not executable — term 5 cannot be exercised" >&2
    exit 1
fi
if ! command -v cargo >/dev/null 2>&1; then
    echo "FATAL: cargo is not on PATH — terms 3 and 4 cannot be exercised" >&2
    exit 1
fi

TMP_ROOT=$(mktemp -d)
trap 'rm -rf "$TMP_ROOT"' EXIT

# One shared target dir across every case: the crates below are byte-identical
# apart from the one file each case perturbs, so cargo compiles once and the
# suite stays a few seconds rather than a few minutes.
export CARGO_TARGET_DIR="$TMP_ROOT/target"

# The composer reads these from the environment, as the pre-mika#2157 heredoc
# did. Pinned so the metadata block stays deterministic — the AC4 byte-identity
# fixture at the bottom depends on it.
export SESSION_ID="test-session" TURNS="7" COST="0.42"

# ── Fixtures ────────────────────────────────────────────────────────────────

# A crate whose source is rustfmt-clean and clippy-clean.
CLEAN_LIB='pub fn describe(items: &[i32]) -> usize {
    items.len()
}
'
# Same crate, rustfmt-clean but tripping `clippy::ptr_arg` — a warning, which
# `-D warnings` turns into a failure, exactly as `ci.yml` and `wip_rescue`'s own
# clippy gate do.
CLIPPY_DIRTY_LIB='pub fn describe(items: &Vec<i32>) -> usize {
    items.len()
}
'
# Same crate, clippy-clean but NOT rustfmt-clean (collapsed body, no trailing
# newline discipline).
FMT_DIRTY_LIB='pub fn describe( items : &[i32] )->usize{items.len()}
'

PLAN_DOC='# A plan

## Acceptance criteria

- [ ] AC1 — something measurable.
'

CARGO_TOML='[package]
name = "mika2354-fixture"
version = "0.1.0"
edition = "2021"

[dependencies]
'

# make_repo <name> [lib-source] — a git repo on `main` with `origin/main`
# planted as a bare remote-tracking ref (exactly what a fetched branch looks
# like to `git diff origin/main...HEAD` — no network, no daemon), carrying a
# real cargo crate and a real copy of verify-pipeline.sh.
#
# The base commit holds only the scaffolding, so the *diff against origin/main*
# is what each case adds on top — which is what the measurement reads.
make_repo() {
    local dir="$TMP_ROOT/$1"
    mkdir -p "$dir/scripts"
    git -C "$dir" init -q -b main
    git -C "$dir" config user.email "test@example.com"
    git -C "$dir" config user.name "test"
    git -C "$dir" config commit.gpgsign false
    printf '%s' "$CARGO_TOML" > "$dir/Cargo.toml"
    cp "$REAL_VERIFY_PIPELINE" "$dir/scripts/verify-pipeline.sh"
    chmod +x "$dir/scripts/verify-pipeline.sh"
    git -C "$dir" add -A
    git -C "$dir" commit -q --no-verify -m "base"
    git -C "$dir" update-ref refs/remotes/origin/main HEAD
    printf '%s' "$dir"
}

# add_work <dir> <lib-source> — the shape of a nominal dispatch's diff: one
# source file plus one plan doc, which is what `verify-pipeline.sh` requires
# (docs && source -> pass).
add_work() {
    local dir="$1" lib="$2"
    mkdir -p "$dir/src" "$dir/docs/plans"
    printf '%s' "$lib" > "$dir/src/lib.rs"
    printf '%s' "$PLAN_DOC" > "$dir/docs/plans/2026-09-17-002-fix-2354-x-plan.md"
    git -C "$dir" add -A
    git -C "$dir" commit -q --no-verify -m "work"
}

# measure <dir> — run the real measurement, capturing verdict + output.
# Returns the rc; sets MEASURE_OUT / MEASURE_TERM / MEASURE_EXCERPT.
measure() {
    local rc
    if MEASURE_OUT=$(_measure_pipeline_verified "$1"); then rc=0; else rc=$?; fi
    MEASURE_TERM=$(head -1 <<<"$MEASURE_OUT")
    MEASURE_EXCERPT=$(tail -n +2 <<<"$MEASURE_OUT")
    return "$rc"
}

# ── T1 (AC1) — every term holds: the marker reads `yes` ─────────────────────
echo "-- T1: a complete, clean pipeline (AC1) --"
R1=$(make_repo t1)
add_work "$R1" "$CLEAN_LIB"
if measure "$R1"; then
    PASS=$((PASS + 1)); echo "  ✓ T1 the measurement succeeds"
else
    FAIL=$((FAIL + 1)); echo "  ✗ T1 the measurement succeeds"
    echo "    term:    '$MEASURE_TERM'"
    echo "    excerpt: '$MEASURE_EXCERPT'"
fi
assert_eq "T1 a success prints nothing" "" "$MEASURE_OUT"
B1=$(_compose_rescue_pr_body "$R1" "dirty-worktree" "Class fact." "2354" "yes" "" "")
assert_contains "T1 body carries the verified marker" "$B1" "<!-- rescue-pipeline-verified: yes -->"
assert_not_contains "T1 body carries no unverified marker" "$B1" "rescue-pipeline-verified: no"
assert_not_contains "T1 body names no failing term" "$B1" "rescue-verify-failed"
assert_contains "T1 body says the measurement attests the pipeline, not the work" \
    "$B1" "not that the work is good, which is what the review decides"
assert_not_contains "T1 body drops the now-objectless operator gesture" \
    "$B1" "Operator: verify pipeline completion"

# ── T2 (AC2, term 1) — an incident-only diff has nothing to verify ──────────
echo "-- T2: incident-only diff (AC2, term \`diff\`) --"
R2=$(make_repo t2)
mkdir -p "$R2/docs/plans"
printf '%s' "$PLAN_DOC" > "$R2/docs/plans/2026-09-17-002-fix-2354-x-plan.md"
git -C "$R2" add -A && git -C "$R2" commit -q --no-verify -m "plan only"
measure "$R2" && { FAIL=$((FAIL + 1)); echo "  ✗ T2 refuses an incident-only diff"; }
assert_eq "T2 names the \`diff\` term" "diff" "$MEASURE_TERM"

# ── T3 (AC2, term 2) — content left outside the PR's commit ─────────────────
echo "-- T3: dirty worktree (AC2, term \`worktree-dirty\`) --"
R3=$(make_repo t3)
add_work "$R3" "$CLEAN_LIB"
printf 'left behind\n' > "$R3/src/stranded.rs"
measure "$R3" && { FAIL=$((FAIL + 1)); echo "  ✗ T3 refuses a dirty worktree"; }
assert_eq "T3 names the \`worktree-dirty\` term" "worktree-dirty" "$MEASURE_TERM"
assert_contains "T3 excerpt names the stranded path" "$MEASURE_EXCERPT" "src/stranded.rs"

# ── T3b — a scaffold path left behind is NOT dirtiness ──────────────────────
# Same exclusions as the rescue commit's own `git add -A` (mika#1288, #1419,
# #1552): a path the rescue refuses to stage is not pilot content, so it must
# not make the worktree read dirty. Without this the measurement would answer
# `no` on every single dispatch, since `_set_up_worktree` copies these in.
echo "-- T3b: a scaffold path left behind is not dirtiness --"
R3B=$(make_repo t3b)
add_work "$R3B" "$CLEAN_LIB"
mkdir -p "$R3B/.claude"
printf '{}\n' > "$R3B/.claude/claude-pilot.json"
printf '{}\n' > "$R3B/.claude/settings.local.json"
if measure "$R3B"; then
    PASS=$((PASS + 1)); echo "  ✓ T3b scaffold paths do not make the worktree dirty"
else
    FAIL=$((FAIL + 1)); echo "  ✗ T3b scaffold paths do not make the worktree dirty"
    echo "    term:    '$MEASURE_TERM'"
    echo "    excerpt: '$MEASURE_EXCERPT'"
fi

# ── T4 (AC2, term 3) — unformatted source ──────────────────────────────────
echo "-- T4: unformatted source (AC2, term \`fmt\`) --"
R4=$(make_repo t4)
add_work "$R4" "$FMT_DIRTY_LIB"
measure "$R4" && { FAIL=$((FAIL + 1)); echo "  ✗ T4 refuses unformatted source"; }
assert_eq "T4 names the \`fmt\` term" "fmt" "$MEASURE_TERM"

# ── T5 (AC2, term 4) — a clippy warning, which `-D warnings` makes fatal ────
echo "-- T5: a clippy lint (AC2, term \`clippy\`) --"
R5=$(make_repo t5)
add_work "$R5" "$CLIPPY_DIRTY_LIB"
measure "$R5" && { FAIL=$((FAIL + 1)); echo "  ✗ T5 refuses a clippy lint"; }
assert_eq "T5 names the \`clippy\` term" "clippy" "$MEASURE_TERM"
assert_contains "T5 excerpt carries the diagnostic, not the \`Compiling\` noise" \
    "$MEASURE_EXCERPT" "ptr_arg"

# ── T6 (AC2, term 5) — a pathological split verify-pipeline.sh rejects ──────
echo "-- T6: code-only diff (AC2, term \`verify-pipeline\`) --"
R6=$(make_repo t6)
mkdir -p "$R6/src"
printf '%s' "$CLEAN_LIB" > "$R6/src/lib.rs"
git -C "$R6" add -A && git -C "$R6" commit -q --no-verify -m "source only"
measure "$R6" && { FAIL=$((FAIL + 1)); echo "  ✗ T6 refuses a code-only diff"; }
assert_eq "T6 names the \`verify-pipeline\` term" "verify-pipeline" "$MEASURE_TERM"

# ── T7 (AC3) — an empty worktree dir must not measure the dispatch host ─────
# `git -C ""` silently runs against the dispatch process CWD. Without the guard
# the measurement would read a live checkout — and a clean one would land
# fail-OPEN, the single direction this design forbids.
echo "-- T7: empty worktree dir (AC3) --"
measure "" && { FAIL=$((FAIL + 1)); echo "  ✗ T7 refuses an empty worktree dir"; }
assert_eq "T7 names the \`worktree-unusable\` term" "worktree-unusable" "$MEASURE_TERM"

# ── T8 (AC3) — an absent verify-pipeline.sh is not a free pass ──────────────
echo "-- T8: verify-pipeline.sh absent (AC3) --"
R8=$(make_repo t8)
add_work "$R8" "$CLEAN_LIB"
git -C "$R8" rm -q -- scripts/verify-pipeline.sh
git -C "$R8" commit -q --no-verify -m "drop the verifier"
measure "$R8" && { FAIL=$((FAIL + 1)); echo "  ✗ T8 refuses an absent verifier"; }
assert_eq "T8 names the \`verify-pipeline\` term" "verify-pipeline" "$MEASURE_TERM"
assert_contains "T8 excerpt says the script is absent or not executable" \
    "$MEASURE_EXCERPT" "absent or not executable"

# ── T9 (AC3) — an exhausted budget yields `no`, never a partial `yes` ───────
echo "-- T9: exhausted budget (AC3) --"
R9=$(make_repo t9)
add_work "$R9" "$CLEAN_LIB"
MIKA_RESCUE_VERIFY_BUDGET_SECS=1 measure "$R9" \
    && { FAIL=$((FAIL + 1)); echo "  ✗ T9 refuses on an exhausted budget"; }
assert_eq "T9 names the \`budget\` term" "budget" "$MEASURE_TERM"

# ── T9b — an invalid budget falls back to the default, it does not disarm ───
# `0` is NOT a disarm here: that is `MIKA_RESCUE_VERIFY_ENABLED`'s job, and
# reading a typo'd budget as a disarm would silently restore the producerless
# marker this ticket exists to remove.
echo "-- T9b: invalid budget values fall back to 900s --"
assert_eq "T9b absent → 900" "900" "$(MIKA_RESCUE_VERIFY_BUDGET_SECS= _rescue_verify_budget_secs 2>/dev/null)"
assert_eq "T9b zero → 900" "900" "$(MIKA_RESCUE_VERIFY_BUDGET_SECS=0 _rescue_verify_budget_secs 2>/dev/null)"
assert_eq "T9b negative → 900" "900" "$(MIKA_RESCUE_VERIFY_BUDGET_SECS=-5 _rescue_verify_budget_secs 2>/dev/null)"
assert_eq "T9b unreadable → 900" "900" "$(MIKA_RESCUE_VERIFY_BUDGET_SECS=soon _rescue_verify_budget_secs 2>/dev/null)"
assert_eq "T9b valid → honoured" "42" "$(MIKA_RESCUE_VERIFY_BUDGET_SECS=42 _rescue_verify_budget_secs 2>/dev/null)"
assert_contains "T9b an invalid value is said, not swallowed" \
    "$(MIKA_RESCUE_VERIFY_BUDGET_SECS=soon _rescue_verify_budget_secs 2>&1 >/dev/null)" \
    "rescue_verify_budget_invalid"

# ── T10 (AC2) — every `no` names its term in the composed body ──────────────
echo "-- T10: a \`no\` is actionable — it names the term (AC2) --"
for term in diff worktree-dirty fmt clippy verify-pipeline budget worktree-unusable; do
    BODY=$(_compose_rescue_pr_body "$R1" "dirty-worktree" "Class fact." "2354" "no" "$term" "some output")
    assert_contains "T10 body names the \`$term\` term" "$BODY" "<!-- rescue-verify-failed: ${term} -->"
    assert_contains "T10 body carries the \`$term\` excerpt" "$BODY" "some output"
done

# ── T11 (AC3) — anything that is not the literal `yes` reads as `no` ────────
echo "-- T11: the verdict is fail-closed (AC3) --"
for verdict in "" "no" "YES" "yes " "true" "maybe"; do
    BODY=$(_compose_rescue_pr_body "$R1" "dirty-worktree" "Class fact." "2354" "$verdict" "" "")
    assert_contains "T11 verdict '$verdict' reads as no" "$BODY" "<!-- rescue-pipeline-verified: no -->"
done

# ── T12 (AC4) — the kill-switch restores the pre-mika#2354 body, byte for byte
# A frozen fixture, not a self-comparison: comparing the new code against itself
# would go green on a body both halves got wrong together. This is the literal
# text the composer emitted before this ticket.
echo "-- T12: kill-switch byte-identity (AC4) --"
EXPECTED_PRE_2354_BODY='## Auto-rescued PR (dispatch-lib recovery, class: dirty-worktree)

<!-- rescue-pipeline-verified: no -->
<!-- rescue-diff: carries-work -->

This PR was created by dispatch-lib'"'"'s git-workflow recovery. Class fact.

**Auto-rescued PR.** Operator: verify pipeline completion, then either un-draft this PR or set the marker above to `yes`.

### Recovery metadata
- Recovery class: `dirty-worktree`
- Pilot session: `test-session`
- Turns: 7
- Cost: $0.42

Closes #2354'

# The shape the kill-switch branch of the callsite produces.
KILLED_BODY=$(_compose_rescue_pr_body "$R1" "dirty-worktree" "Class fact." "2354" "no" "" "")
assert_eq "T12 kill-switch body is byte-identical to the pre-mika#2354 one" \
    "$EXPECTED_PRE_2354_BODY" "$KILLED_BODY"

# And a pre-mika#2354 caller (four arguments) keeps working unchanged.
LEGACY_BODY=$(_compose_rescue_pr_body "$R1" "dirty-worktree" "Class fact." "2354")
assert_eq "T12 the four-argument call shape is unchanged" \
    "$EXPECTED_PRE_2354_BODY" "$LEGACY_BODY"

# ── T13 (AC4) — the kill-switch predicate itself ───────────────────────────
echo "-- T13: the kill-switch predicate (AC4) --"
for off in 0 false no off FALSE Off NO; do
    if MIKA_RESCUE_VERIFY_ENABLED="$off" _rescue_verify_enabled; then
        FAIL=$((FAIL + 1)); echo "  ✗ T13 '$off' disarms the measurement"
    else
        PASS=$((PASS + 1)); echo "  ✓ T13 '$off' disarms the measurement"
    fi
done
for on in "" 1 true yes on anything; do
    if MIKA_RESCUE_VERIFY_ENABLED="$on" _rescue_verify_enabled; then
        PASS=$((PASS + 1)); echo "  ✓ T13 '$on' leaves the measurement armed"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ T13 '$on' leaves the measurement armed"
    fi
done
if (unset MIKA_RESCUE_VERIFY_ENABLED; _rescue_verify_enabled); then
    PASS=$((PASS + 1)); echo "  ✓ T13 absent leaves the measurement armed (default)"
else
    FAIL=$((FAIL + 1)); echo "  ✗ T13 absent leaves the measurement armed (default)"
fi

# ── T14 — AC7: `wip_rescue` stays a reader, never a writer ──────────────────
# The producer is dispatch-lib, sole writer of the marker. A consumer that
# wrote its own green light would be issuing itself the authorisation
# mika#2286 exists to withhold.
echo "-- T14: the daemon never writes its own green light (AC7) --"
WIP_RESCUE_RS="$REPO_ROOT/crates/mika-agent/src/wip_rescue.rs"
if [ -f "$WIP_RESCUE_RS" ]; then
    writers=$(grep -n 'rescue-pipeline-verified' "$WIP_RESCUE_RS" \
        | grep -vE '^[0-9]+:[[:space:]]*(//|///|//!)' \
        | grep -vE 'const (MARKER_NO|MARKER_YES|PIPELINE_VERIFIED_KEY)' \
        | grep -vE 'assert|pipeline_verified\(' || true)
    assert_eq "T14 wip_rescue has no non-test, non-comment marker write" "" "$writers"
else
    FAIL=$((FAIL + 1)); echo "  ✗ T14 wip_rescue.rs not found at $WIP_RESCUE_RS"
fi

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed"
echo "========================================"
[ "$FAIL" -eq 0 ]

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
# `origin/main` ref planted as a remote-tracking ref — and runs the real
# measurement over it: the diff predicate, the worktree-status term, the budget
# clock, the excerpt selection and the term naming all traverse actual git state,
# for the reason `test_rescue_closes_guard.sh` states about its own probes: a
# measurement reconstructed from the plan would only test the plan.
#
# The three EXTERNAL tools the measurement shells out to — `cargo fmt`, `cargo
# clippy`, `scripts/verify-pipeline.sh` — are shims with CONTROLLED output, not
# the real programs. Two reasons, both hard:
#   - Hermeticity. The fixture lives under `mktemp -d`, outside the repo's
#     `rust-toolchain.toml`; on the CI runner rustup then has no toolchain to
#     pick and every cargo call dies with `error: rustup could not choose a
#     version of cargo to run` — the suite read 55/8 there while reading 63/0
#     on a host with a default toolchain. And the real `verify-pipeline.sh`
#     calls `gh pr view` (network) when `gh` is on PATH.
#   - Scope. What this suite proves is what `_measure_pipeline_verified` DOES
#     with each tool's outcome — which term it names, which lines it keeps,
#     that success prints nothing, that it never lands fail-open. Whether
#     rustfmt or clippy is right about a given source file is those tools'
#     contract, tested upstream, not the producer's.
# What is NOT reconstructed: the shims log every invocation (cwd + argv), and
# T1 asserts the exact commands the producer ran, from inside the fixture, in
# the house shape (`--check`, `-D warnings`, `origin/main`) — so a producer that
# drifted from those invocations would go red here even though every shim
# answers green.
#
# Run: bash skills/bundled/_shared/tests/test_rescue_pipeline_verified.sh
# Expected: all assertions pass, exit 0.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

# dispatch-lib writes git noise to fd 9 (opened by the trace setup in a real
# dispatch). Open it here so the `2>&9` redirects in `_rescue_dirty_worktree` —
# which the mika#2631 cases drive for real — have a destination. Same line, same
# reason, as `test_dev_groom_dirty_rescue.sh`: without it those cases die on a
# closed descriptor rather than on an assertion.
exec 9>/dev/null

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

TMP_ROOT=$(mktemp -d)
trap 'rm -rf "$TMP_ROOT"' EXIT

# ── Shims ───────────────────────────────────────────────────────────────────
# One `cargo` shim ahead of PATH for the whole suite, and one
# `verify-pipeline.sh` planted in every fixture (the producer runs it by path,
# `./scripts/verify-pipeline.sh origin/main`, never via PATH). Each answers
# green unless its control variable says `fail`, in which case it emits a
# diagnostic in the REAL tool's shape — `Compiling` noise ahead of the clippy
# `error:` line, rustfmt's `Diff in`, the verifier's own `FAIL:` first line —
# so the excerpt assertions below exercise `_rescue_verify_excerpt` against
# the output shapes it was written for. Every call is appended to
# `$STUB_LOG` as `<cwd>\t<argv>` so a test can assert WHAT was run and WHERE.
#
# Control:  RESCUE_STUB_FMT=fail | RESCUE_STUB_CLIPPY=fail | RESCUE_STUB_VERIFY=fail
STUB_BIN="$TMP_ROOT/stub-bin"
STUB_LOG="$TMP_ROOT/stub-calls.log"
mkdir -p "$STUB_BIN"
: > "$STUB_LOG"
export STUB_LOG

cat > "$STUB_BIN/cargo" <<'EOF'
#!/bin/sh
printf '%s\t%s\n' "$PWD" "$*" >> "$STUB_LOG"
case "$1" in
    fmt)
        if [ "${RESCUE_STUB_FMT:-}" = "fail" ]; then
            printf 'Diff in %s/src/lib.rs:1:\n-pub fn describe( items : &[i32] )->usize{items.len()}\n+pub fn describe(items: &[i32]) -> usize {\n+    items.len()\n+}\n' "$PWD"
            exit 1
        fi
        exit 0 ;;
    clippy)
        if [ "${RESCUE_STUB_CLIPPY:-}" = "fail" ]; then
            printf '   Compiling mika2354-fixture v0.1.0 (%s)\n' "$PWD"
            printf 'error: writing `&Vec` instead of `&[_]` involves a new object where a slice will do\n'
            printf ' --> src/lib.rs:1:24\n  |\n1 | pub fn describe(items: &Vec<i32>) -> usize {\n  |                        ^^^^^^^^^ help: change this to: `&[i32]`\n  |\n'
            printf '  = help: for further information visit https://rust-lang.github.io/rust-clippy/master/index.html#ptr_arg\n'
            printf '  = note: `-D clippy::ptr-arg` implied by `-D warnings`\n\n'
            printf 'error: could not compile `mika2354-fixture` (lib) due to 1 previous error\n'
            exit 101
        fi
        printf '   Compiling mika2354-fixture v0.1.0 (%s)\n    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.42s\n' "$PWD"
        exit 0 ;;
    *)
        printf 'error: no such command: `%s`\n' "$1" >&2
        exit 101 ;;
esac
EOF
chmod +x "$STUB_BIN/cargo"

VERIFY_PIPELINE_STUB='#!/usr/bin/env bash
printf '"'"'%s\t%s\n'"'"' "$PWD" "verify-pipeline.sh $*" >> "$STUB_LOG"
if [ "${RESCUE_STUB_VERIFY:-}" = "fail" ]; then
    echo "FAIL: code-only PR — source files changed but no docs/plans/ or docs/solutions/ file in the diff"
    echo "  Base ref: $1"
    echo "  Run /mika to produce the plan artefact, or add a Pipeline-Exempt: code-only trailer."
    exit 1
fi
echo "PASS: docs && source — pipeline artefacts present (base: $1)"
exit 0
'

export PATH="$STUB_BIN:$PATH"

# The composer reads these from the environment, as the pre-mika#2157 heredoc
# did. Pinned so the metadata block stays deterministic — the AC4 byte-identity
# fixture at the bottom depends on it.
export SESSION_ID="test-session" TURNS="7" COST="0.42"

# ── Fixtures ────────────────────────────────────────────────────────────────

# The one source file each case adds. Its CONTENT is inert here — the shims
# decide fmt/clippy outcomes, not rustfmt/clippy — it exists so the diff
# against `origin/main` carries a source-bucket path (term 1, and the shape
# `verify-pipeline.sh` classifies).
LIB_SRC='pub fn describe(items: &[i32]) -> usize {
    items.len()
}
'

PLAN_DOC='# A plan

## Acceptance criteria

- [ ] AC1 — something measurable.
'

# make_repo <name> — a git repo on `main` with `origin/main` planted as a bare
# remote-tracking ref (exactly what a fetched branch looks like to `git diff
# origin/main...HEAD` — no network, no daemon), carrying the verify-pipeline
# shim at the path the producer requires.
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
    printf '%s' "$VERIFY_PIPELINE_STUB" > "$dir/scripts/verify-pipeline.sh"
    chmod +x "$dir/scripts/verify-pipeline.sh"
    git -C "$dir" add -A
    git -C "$dir" commit -q --no-verify -m "base"
    git -C "$dir" update-ref refs/remotes/origin/main HEAD
    printf '%s' "$dir"
}

# add_work <dir> — the shape of a nominal dispatch's diff: one source file plus
# one plan doc, which is what `verify-pipeline.sh` requires (docs && source ->
# pass).
add_work() {
    local dir="$1"
    mkdir -p "$dir/src" "$dir/docs/plans"
    printf '%s' "$LIB_SRC" > "$dir/src/lib.rs"
    printf '%s' "$PLAN_DOC" > "$dir/docs/plans/2026-09-17-002-fix-2354-x-plan.md"
    git -C "$dir" add -A
    git -C "$dir" commit -q --no-verify -m "work"
}

# measure <dir> — run the real measurement, capturing verdict + output.
# Truncates the shim log first so each case reads only its own calls.
# Returns the rc; sets MEASURE_OUT / MEASURE_TERM / MEASURE_EXCERPT.
measure() {
    local rc
    : > "$STUB_LOG"
    if MEASURE_OUT=$(_measure_pipeline_verified "$1"); then rc=0; else rc=$?; fi
    MEASURE_TERM=$(head -1 <<<"$MEASURE_OUT")
    MEASURE_EXCERPT=$(tail -n +2 <<<"$MEASURE_OUT")
    return "$rc"
}

# ── T1 (AC1) — every term holds: the marker reads `yes` ─────────────────────
echo "-- T1: a complete, clean pipeline (AC1) --"
R1=$(make_repo t1)
add_work "$R1"
if measure "$R1"; then
    PASS=$((PASS + 1)); echo "  ✓ T1 the measurement succeeds"
else
    FAIL=$((FAIL + 1)); echo "  ✗ T1 the measurement succeeds"
    echo "    term:    '$MEASURE_TERM'"
    echo "    excerpt: '$MEASURE_EXCERPT'"
fi
assert_eq "T1 a success prints nothing" "" "$MEASURE_OUT"
# The shims answered green; what makes that a measurement rather than a
# reconstruction is that the producer ran the HOUSE invocations, inside the
# worktree, in order — `--check` (never a rewriting `cargo fmt`), `-D warnings`
# (ci.yml and wip_rescue's own gate), `origin/main` (load-bearing, see the
# producer's comment: a stale local `main` would classify the wrong diff).
assert_eq "T1 the producer ran exactly fmt, clippy, verify-pipeline — in that order, from the worktree" \
    "$(printf '%s\tfmt --all --check\n%s\tclippy --workspace --all-targets -- -D warnings\n%s\tverify-pipeline.sh origin/main' "$R1" "$R1" "$R1")" \
    "$(cat "$STUB_LOG")"
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
add_work "$R3"
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
add_work "$R3B"
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
add_work "$R4"
RESCUE_STUB_FMT=fail measure "$R4" && { FAIL=$((FAIL + 1)); echo "  ✗ T4 refuses unformatted source"; }
assert_eq "T4 names the \`fmt\` term" "fmt" "$MEASURE_TERM"
assert_contains "T4 excerpt keeps rustfmt's \`Diff in\` line" "$MEASURE_EXCERPT" "Diff in"
assert_not_contains "T4 a red fmt short-circuits: clippy never runs" "$(cat "$STUB_LOG")" "clippy"

# ── T5 (AC2, term 4) — a clippy warning, which `-D warnings` makes fatal ────
echo "-- T5: a clippy lint (AC2, term \`clippy\`) --"
R5=$(make_repo t5)
add_work "$R5"
RESCUE_STUB_CLIPPY=fail measure "$R5" && { FAIL=$((FAIL + 1)); echo "  ✗ T5 refuses a clippy lint"; }
assert_eq "T5 names the \`clippy\` term" "clippy" "$MEASURE_TERM"
# `_rescue_verify_excerpt` keeps the `error:` line — the one clippy prints for
# `ptr_arg` reads `error: writing \`&Vec\` instead of \`&[_]\` …`. The lint's
# NAME sits on an indented `= help:` line the excerpt deliberately drops, so
# the assertion targets what the excerpt retains, not what the lint is called.
assert_contains "T5 excerpt carries the diagnostic, not the \`Compiling\` noise" \
    "$MEASURE_EXCERPT" "error: writing \`&Vec\`"
assert_not_contains "T5 excerpt drops the \`Compiling\` line" "$MEASURE_EXCERPT" "Compiling"
assert_not_contains "T5 excerpt drops the indented \`= help:\` line" "$MEASURE_EXCERPT" "= help:"
assert_not_contains "T5 a red clippy short-circuits: verify-pipeline never runs" "$(cat "$STUB_LOG")" "verify-pipeline.sh"

# ── T6 (AC2, term 5) — a pathological split verify-pipeline.sh rejects ──────
echo "-- T6: code-only diff (AC2, term \`verify-pipeline\`) --"
R6=$(make_repo t6)
mkdir -p "$R6/src"
printf '%s' "$LIB_SRC" > "$R6/src/lib.rs"
git -C "$R6" add -A && git -C "$R6" commit -q --no-verify -m "source only"
RESCUE_STUB_VERIFY=fail measure "$R6" && { FAIL=$((FAIL + 1)); echo "  ✗ T6 refuses a code-only diff"; }
assert_eq "T6 names the \`verify-pipeline\` term" "verify-pipeline" "$MEASURE_TERM"
assert_contains "T6 excerpt keeps the verifier's \`FAIL:\` line" "$MEASURE_EXCERPT" "FAIL: code-only PR"
assert_contains "T6 the verifier was handed \`origin/main\`, not a local ref" "$(cat "$STUB_LOG")" "verify-pipeline.sh origin/main"

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
add_work "$R8"
git -C "$R8" rm -q -- scripts/verify-pipeline.sh
git -C "$R8" commit -q --no-verify -m "drop the verifier"
measure "$R8" && { FAIL=$((FAIL + 1)); echo "  ✗ T8 refuses an absent verifier"; }
assert_eq "T8 names the \`verify-pipeline\` term" "verify-pipeline" "$MEASURE_TERM"
assert_contains "T8 excerpt says the script is absent or not executable" \
    "$MEASURE_EXCERPT" "absent or not executable"

# ── T9 (AC3) — an exhausted budget yields `no`, never a partial `yes` ───────
# The budget is in whole seconds and 1 is its floor; the suite's own shims
# answer instantly, so a 1s budget would never be exhausted by them. The clock
# is pinned rather than raced: a second `cargo` shim, ahead of the suite's on
# PATH, that sleeps past the budget. Under `timeout` it is killed at 1s (124 →
# `budget`); without `timeout` the next deadline check catches the overrun —
# both branches of `_rescue_verify_run` yield the same term.
echo "-- T9: exhausted budget (AC3) --"
R9=$(make_repo t9)
add_work "$R9"
SLOW_BIN="$TMP_ROOT/slow-bin"
mkdir -p "$SLOW_BIN"
printf '#!/bin/sh\nexec sleep 5\n' > "$SLOW_BIN/cargo"
chmod +x "$SLOW_BIN/cargo"
PATH="$SLOW_BIN:$PATH" MIKA_RESCUE_VERIFY_BUDGET_SECS=1 measure "$R9" \
    && { FAIL=$((FAIL + 1)); echo "  ✗ T9 refuses on an exhausted budget"; }
assert_eq "T9 names the \`budget\` term" "budget" "$MEASURE_TERM"
assert_contains "T9 excerpt names the term the budget ran out on" \
    "$MEASURE_EXCERPT" "budget"

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
for term in compound-traversal diff worktree-dirty fmt clippy verify-pipeline budget worktree-unusable; do
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
    # Stop at the first `#[cfg(test)]`: the unit-test module holds string
    # fixtures such as `<!-- rescue-pipeline-verified: maybe -->` that are
    # inputs to `pipeline_verified`, not writes of the marker. Only the
    # production half of the file is a candidate writer.
    writers=$(awk '/^#\[cfg\(test\)\]/ { exit } /rescue-pipeline-verified/ { print NR ":" $0 }' "$WIP_RESCUE_RS" \
        | grep -vE '^[0-9]+:[[:space:]]*(//|///|//!)' \
        | grep -vE 'const (MARKER_NO|MARKER_YES|PIPELINE_VERIFIED_KEY)' \
        | grep -vE 'assert|pipeline_verified\(' || true)
    assert_eq "T14 wip_rescue has no non-test, non-comment marker write" "" "$writers"
else
    FAIL=$((FAIL + 1)); echo "  ✗ T14 wip_rescue.rs not found at $WIP_RESCUE_RS"
fi

# ═══════════════════════════════════════════════════════════════════════════
# T15 (mika#2563) — a session the SDK killed does not attest a complete pipeline
#
# The founding measurement: PR #2556 carries `<!-- rescue-pipeline-verified:
# yes -->` — "the local pipeline is complete" — while its pilot was killed at
# 151/150 turns. None of the five terms above reads `STATUS`, so a truncated
# session and a session that concluded produced the same green light.
#
# What is NOT tested here, and the omission is the point: "the diff carries no
# `docs/solutions`" is NOT a refusal. Measured over the last 100 `--first-parent`
# merges of `origin/main`, 66 of the 93 carrying source carry no learning — 71 %,
# the regime, not a defect. T15e is the negative control that pins it.
#
# `STATUS` is the global `_run_claude_pilot` sets from the pilot's JSON. The
# suite has run with it unset until here, which classifies `absent` — harmless,
# because the disposition ships disarmed and T1–T14 never arm it.
# ═══════════════════════════════════════════════════════════════════════════

# ── T15 fixtures ────────────────────────────────────────────────────────────

# add_solution <dir> — the intrinsic attestation: a learning in the diff. The
# NESTED path is deliberate — this repo writes `docs/solutions/<category>/x.md`,
# never a file directly under `docs/solutions/`, so a predicate anchored one
# level too shallow would read `absent` on every real learning.
add_solution() {
    local dir="$1"
    mkdir -p "$dir/docs/solutions/best-practices"
    printf '# A learning\n' > "$dir/docs/solutions/best-practices/2026-09-28-x.md"
    git -C "$dir" add -A
    git -C "$dir" commit -q --no-verify -m "docs(solutions): the learning"
}

# traversal <dir> — the classifier alone, without the four terms behind it.
traversal() { _rescue_compound_traversal "$1"; }

# ── T15a (AC1) — the founding defect: a truncated session, no traversal ─────
echo "-- T15a: truncated session, nothing attests the traversal (AC1, mika#2563) --"
R15A=$(make_repo t15a)
add_work "$R15A"
assert_eq "T15a the classifier reads \`absent\`" "absent" "$(STATUS=terminated traversal "$R15A")"

# Armed, the term refuses — and it refuses FIRST, so the 900s budget is not
# spent on two cargo invocations to be told `no` at the end.
STATUS=terminated MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL=1 measure "$R15A" \
    && { FAIL=$((FAIL + 1)); echo "  ✗ T15a refuses a truncated session with no traversal"; }
assert_eq "T15a names the \`compound-traversal\` term" "compound-traversal" "$MEASURE_TERM"
assert_contains "T15a excerpt names the status that was read" "$MEASURE_EXCERPT" "terminated"
assert_contains "T15a excerpt names the intrinsic way out" "$MEASURE_EXCERPT" "docs/solutions"
assert_contains "T15a excerpt names the declarative way out" "$MEASURE_EXCERPT" "Compound: none"
assert_eq "T15a term 0 short-circuits: no cargo, no verifier ran" "" "$(cat "$STUB_LOG")"

B15A=$(_compose_rescue_pr_body "$R15A" "dirty-worktree" "Class fact." "2563" "no" "compound-traversal" "excerpt" "absent")
assert_contains "T15a body names the failing term" "$B15A" "<!-- rescue-verify-failed: compound-traversal -->"
assert_contains "T15a body carries the traversal marker" "$B15A" "<!-- compound-traversal: absent -->"
assert_contains "T15a body stays unverified" "$B15A" "<!-- rescue-pipeline-verified: no -->"

# ── T15b — the intrinsic attestation: a learning is in the diff ─────────────
echo "-- T15b: a docs/solutions file attests the traversal (AC2) --"
R15B=$(make_repo t15b)
add_work "$R15B"
add_solution "$R15B"
assert_eq "T15b the classifier reads \`attested-solution\`" \
    "attested-solution" "$(STATUS=terminated traversal "$R15B")"
if STATUS=terminated MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL=1 measure "$R15B"; then
    PASS=$((PASS + 1)); echo "  ✓ T15b the conjunction continues past term 0"
else
    FAIL=$((FAIL + 1)); echo "  ✗ T15b the conjunction continues past term 0"
    echo "    term:    '$MEASURE_TERM'"
fi
assert_contains "T15b the cargo terms did run" "$(cat "$STUB_LOG")" "clippy"
B15B=$(_compose_rescue_pr_body "$R15B" "dirty-worktree" "Class fact." "2563" "yes" "" "" "attested-solution")
assert_contains "T15b body carries the traversal marker" "$B15B" "<!-- compound-traversal: attested-solution -->"

# ── T15c (AC3) — the declarative attestation: the absence is STATED ─────────
echo "-- T15c: a \`Compound: none — <reason>\` trailer attests the traversal (AC3) --"
R15C=$(make_repo t15c)
add_work "$R15C"
git -C "$R15C" commit -q --allow-empty --no-verify \
    -m "chore: no learning here" \
    -m "Compound: none — mechanical fix, the diff and its test say everything"
assert_eq "T15c the classifier reads \`attested-trailer\`" \
    "attested-trailer" "$(STATUS=terminated traversal "$R15C")"
if STATUS=terminated MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL=1 measure "$R15C"; then
    PASS=$((PASS + 1)); echo "  ✓ T15c the conjunction continues past term 0"
else
    FAIL=$((FAIL + 1)); echo "  ✗ T15c the conjunction continues past term 0"
    echo "    term:    '$MEASURE_TERM'"
fi

# ── T15d (AC3) — the degraded trailer forms are REFUSED ─────────────────────
# The shape follows `Pipeline-Exempt:` in the same ecosystem: anchored, reason
# mandatory. No bare form — this trailer is born today and carries no backward
# compatibility, so accepting `Compound: none` alone would be inventing a debt.
# `--cleanup=verbatim` so git keeps the indentation the second case is about.
echo "-- T15d: a degraded trailer does not attest (AC3) --"
R15D=$(make_repo t15d)
add_work "$R15D"
git -C "$R15D" commit -q --allow-empty --no-verify --cleanup=verbatim \
    -m "chore: bare form" -m "Compound: none"
assert_eq "T15d a trailer with no reason does not attest" "absent" "$(STATUS=terminated traversal "$R15D")"

R15D2=$(make_repo t15d2)
add_work "$R15D2"
git -C "$R15D2" commit -q --allow-empty --no-verify --cleanup=verbatim \
    -m "chore: indented form" -m "  Compound: none — a reason"
assert_eq "T15d an unanchored trailer does not attest" "absent" "$(STATUS=terminated traversal "$R15D2")"

R15D3=$(make_repo t15d3)
add_work "$R15D3"
git -C "$R15D3" commit -q --allow-empty --no-verify --cleanup=verbatim \
    -m "chore: prose mention" -m "We considered a Compound: none — but wrote the learning instead."
assert_eq "T15d a mid-line mention does not attest" "absent" "$(STATUS=terminated traversal "$R15D3")"

# ── T15e — NEGATIVE CONTROL, and the suite is worthless without it ──────────
# A nominal pilot concludes in `STATUS=success`, and 71 % of the loop's merges
# carry no `docs/solutions`. Without this case, "the term bites" would be
# indistinguishable from "the term bites everybody" — and the second reading
# stops the drain, because `verify-pipeline.sh` is term 5 of this very
# conjunction and every rescue would flip to `rescue-pipeline-verified: no`.
#
# mika#2631: `RESCUED_DIRTY_WORKTREE=0` is now posed EXPLICITLY rather than left
# undefined. The case passed before because the variable happened to be unset in
# this process; an implicit control is one a future edit breaks in silence. The
# expectation is unchanged — a concluded session that committed its own work is
# exempt — only the premise is now stated.
echo "-- T15e: NEGATIVE CONTROL — a concluded session is exempt (AC4) --"
R15E=$(make_repo t15e)
add_work "$R15E"
assert_eq "T15e the classifier reads \`not-applicable\`" \
    "not-applicable" "$(STATUS=success RESCUED_DIRTY_WORKTREE=0 traversal "$R15E")"
if STATUS=success RESCUED_DIRTY_WORKTREE=0 MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL=1 measure "$R15E"; then
    PASS=$((PASS + 1)); echo "  ✓ T15e a concluded session with no learning is NOT refused"
else
    FAIL=$((FAIL + 1)); echo "  ✗ T15e a concluded session with no learning is NOT refused"
    echo "    term:    '$MEASURE_TERM'"
    echo "    excerpt: '$MEASURE_EXCERPT'"
fi
B15E=$(_compose_rescue_pr_body "$R15E" "dirty-worktree" "Class fact." "2563" "yes" "" "" "not-applicable")
assert_not_contains "T15e body names no failing term" "$B15E" "rescue-verify-failed"
assert_contains "T15e body still carries the marker" "$B15E" "<!-- compound-traversal: not-applicable -->"

# `no-shipping-tail` (mika#2492) is the loop's NOMINAL route and carries
# `STATUS = success` by construction — `_pilot_had_no_shipping_tail` requires
# it. Biting there would re-bite the 71 %, under another name (D3).
assert_eq "T15e the no-shipping-tail route is exempt by its own STATUS" \
    "not-applicable" \
    "$(SKILL=dev-pilot PILOT_SHIPPING_TAIL=absent STATUS=success RESCUED_DIRTY_WORKTREE=0 traversal "$R15E")"

# ── T15f (D7) — fail-closed: an unreadable conclusion is not a conclusion ───
echo "-- T15f: fail-closed on every non-success status (D7) --"
R15F=$(make_repo t15f)
add_work "$R15F"
for st in "" "failed" "terminated" "cancelled" "Success" "success "; do
    assert_eq "T15f status '$st' does not attest" "absent" "$(STATUS="$st" traversal "$R15F")"
done
if (unset STATUS; [ "$(_rescue_compound_traversal "$R15F")" = "absent" ]); then
    PASS=$((PASS + 1)); echo "  ✓ T15f an absent STATUS does not attest"
else
    FAIL=$((FAIL + 1)); echo "  ✗ T15f an absent STATUS does not attest"
fi
# An unreadable git is never a satisfied term either — and the empty dir must
# not be classified against the dispatch process CWD, which is a live checkout.
assert_eq "T15f an empty worktree dir does not attest" "absent" "$(STATUS=terminated traversal "")"
assert_eq "T15f a non-repo dir does not attest" "absent" "$(STATUS=terminated traversal "$TMP_ROOT")"
R15F2=$(make_repo t15f2)
add_work "$R15F2"
git -C "$R15F2" update-ref -d refs/remotes/origin/main
assert_eq "T15f an unfetched origin/main does not attest" "absent" "$(STATUS=terminated traversal "$R15F2")"

# ── T15g (AC5) — the kill-switch rollback is byte-identical, still ──────────
# mika#2563 extends the mika#2354 invariant rather than punching a hole in it:
# the kill-switch path passes `$8` empty, so the marker is absent exactly where
# the pre-fix body had nothing.
echo "-- T15g: an absent traversal argument leaves the body byte-identical (AC5) --"
EIGHT_ARG_EMPTY=$(_compose_rescue_pr_body "$R1" "dirty-worktree" "Class fact." "2354" "no" "" "" "")
assert_eq "T15g an empty 8th argument is byte-identical to the pre-mika#2354 body" \
    "$EXPECTED_PRE_2354_BODY" "$EIGHT_ARG_EMPTY"
assert_not_contains "T15g no marker is emitted for an empty value" "$EIGHT_ARG_EMPTY" "compound-traversal"

# ── T15h (AC5) — DISARMED IS NOT INERT ─────────────────────────────────────
# The whole difference between a disarmed detector and a mute one (mika#2205):
# the term measures and writes its marker, it simply does not flip `verified`.
# That is what makes the arming condition measurable on the artefact instead of
# intuited — the PR body is the only durable surface here.
echo "-- T15h: disarmed, the term still measures and still writes its marker (AC5) --"
R15H=$(make_repo t15h)
add_work "$R15H"
if STATUS=terminated measure "$R15H"; then
    PASS=$((PASS + 1)); echo "  ✓ T15h disarmed, the term does not flip the verdict"
else
    FAIL=$((FAIL + 1)); echo "  ✗ T15h disarmed, the term does not flip the verdict"
    echo "    term:    '$MEASURE_TERM'"
fi
assert_eq "T15h the classification happened anyway" "absent" "$(STATUS=terminated traversal "$R15H")"
B15H=$(_compose_rescue_pr_body "$R15H" "dirty-worktree" "Class fact." "2563" "yes" "" "" "absent")
assert_contains "T15h the marker is written while disarmed" "$B15H" "<!-- compound-traversal: absent -->"
assert_contains "T15h the verdict stays what the other terms decided" "$B15H" "<!-- rescue-pipeline-verified: yes -->"
assert_not_contains "T15h disarmed, no term is named as failing" "$B15H" "rescue-verify-failed"

# An explicit `0` is the explicit disarm and must be silent about it.
if STATUS=terminated MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL=0 measure "$R15H"; then
    PASS=$((PASS + 1)); echo "  ✓ T15h an explicit 0 disarms"
else
    FAIL=$((FAIL + 1)); echo "  ✗ T15h an explicit 0 disarms"
fi

# ── T15i — the disposition predicate itself ────────────────────────────────
# House three-tier parse. An unrecognised value stays DISARMED, and says so:
# it must not arm by accident on a term that can withhold a PR's green light,
# and it must not disarm in silence either (mika#2293).
echo "-- T15i: the disposition predicate (three tiers) --"
for on in 1 true yes on TRUE Yes ON; do
    if MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL="$on" _rescue_require_compound_traversal; then
        PASS=$((PASS + 1)); echo "  ✓ T15i '$on' arms the disposition"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ T15i '$on' arms the disposition"
    fi
done
for off in "" 0 false no off FALSE Off NO; do
    if MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL="$off" _rescue_require_compound_traversal; then
        FAIL=$((FAIL + 1)); echo "  ✗ T15i '$off' leaves the disposition disarmed"
    else
        PASS=$((PASS + 1)); echo "  ✓ T15i '$off' leaves the disposition disarmed"
    fi
done
if (unset MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL; _rescue_require_compound_traversal); then
    FAIL=$((FAIL + 1)); echo "  ✗ T15i absent leaves the disposition disarmed (default)"
else
    PASS=$((PASS + 1)); echo "  ✓ T15i absent leaves the disposition disarmed (default)"
fi
if MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL=soon _rescue_require_compound_traversal 2>/dev/null; then
    FAIL=$((FAIL + 1)); echo "  ✗ T15i an unrecognised value stays disarmed"
else
    PASS=$((PASS + 1)); echo "  ✓ T15i an unrecognised value stays disarmed"
fi
assert_contains "T15i an unrecognised value is said, not swallowed" \
    "$(MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL=soon _rescue_require_compound_traversal 2>&1 || true)" \
    "rescue_compound_traversal_disposition_invalid"

# ── T15j — the four values are a wire format ───────────────────────────────
# They land in a PR-body marker `<!-- compound-traversal: … -->` that an
# operator greps (mika#2201). One definition site, four values, pinned here so
# a fifth or a rename is a decision rather than a drift.
echo "-- T15j: the four values are a wire format (mika#2201) --"
for value in not-applicable attested-solution attested-trailer absent; do
    BODY=$(_compose_rescue_pr_body "$R1" "dirty-worktree" "Class fact." "2563" "no" "" "" "$value")
    assert_contains "T15j the \`$value\` value reaches the marker verbatim" \
        "$BODY" "<!-- compound-traversal: ${value} -->"
done

# ═══════════════════════════════════════════════════════════════════════════
# T15k … T15q (mika#2631) — a session that CONCLUDED is not a session that
# DELIVERED
#
# The founding measurement: PR #2630, pilot `bb9163e1`, 2026-10-01. The pilot
# launched its reviewers, wrote "waiting on them" and handed the turn back; the
# session closed `[done] Success | 140 turns` (cpp#267). Nothing was finished —
# no review collected, no compound, no commit, no PR. dispatch-lib SAW it
# (`HEAD unchanged — dirty worktree detected and auto-committed`,
# PIPELINE_INCOMPLETE) and still wrote `<!-- compound-traversal: not-applicable -->`,
# so `MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL=1` did not park the draft.
#
# WHY THESE CASES DRIVE THE REAL RESCUE rather than setting a variable. The fact
# the ticket names — "HEAD unchanged + dirty worktree" — has DISAPPEARED from
# measurable state by the time the classifier runs: the rescue commit advanced
# `POST_RUN_HEAD` and emptied the index, so both halves now read their own
# opposite. Only the `RESCUED_DIRTY_WORKTREE` stamp survives. A case that posed
# the stamp by hand would test the plan; T15k proves the stamp is really written
# by its producer and really reaches the classifier.
#
# NUMBERING: the plan wrote these as T15i…T15o, counting from a suite that ended
# at T15h. T15i (the disposition predicate) and T15j (the wire format) were
# already taken, so the seven cases land at T15k…T15q. Content unchanged.
# ═══════════════════════════════════════════════════════════════════════════

# rescue_in_pilots_place <dir> — set the globals `_rescue_dirty_worktree` reads
# for the zero-commit shape it is scoped to, then call it for real. Patterned on
# `test_dev_groom_dirty_rescue.sh::run_rescue`.
#
# `SESSION_ID` is deliberately NOT overwritten: the suite exports it and T15g's
# byte-identity fixture depends on its value. The rescue only interpolates it
# into a commit body.
rescue_in_pilots_place() {
    local repo="$1"
    WORKTREE_DIR="$repo"
    SKILL="dev-pilot"
    REPO="mika"
    ISSUE_NUM="2631"
    BRANCH="fix/2631/dispatch-lib-compound-traversal-not"
    PILOT_EXIT=0
    RESULT="claude-pilot completed (status: success)."
    RESCUED_DIRTY_WORKTREE=""
    PRE_RUN_HEAD=$(git -C "$repo" rev-parse HEAD)
    POST_RUN_HEAD="$PRE_RUN_HEAD"
    _rescue_dirty_worktree || true
}

# ── T15k (AC1, AC3) — the founding defect, end to end, SEEN RED ──────────────
echo "-- T15k: a rescue that committed in the pilot's place is not exempt (AC1/AC3, mika#2631) --"
R15K=$(make_repo t15k)
# The pilot's content, written and never committed. No `docs/solutions`, no
# trailer — the shape of the founding incident.
mkdir -p "$R15K/src" "$R15K/docs/plans"
printf '%s' "$LIB_SRC" > "$R15K/src/lib.rs"
printf '%s' "$PLAN_DOC" > "$R15K/docs/plans/2026-10-02-001-fix-2631-x-plan.md"
PRE_RESCUE_HEAD=$(git -C "$R15K" rev-parse HEAD)
rescue_in_pilots_place "$R15K"

# The three facts of R1, asserted rather than assumed: this is the proof that the
# predicate the ticket names in symptom terms would be UNMEASURABLE right here.
assert_eq "T15k the producer wrote the stamp" "1" "$RESCUED_DIRTY_WORKTREE"
assert_eq "T15k HEAD advanced, so \`PRE != POST\` now reads its own opposite" "1" \
    "$( [ "$PRE_RESCUE_HEAD" != "$(git -C "$R15K" rev-parse HEAD)" ] && echo 1 || echo 0 )"
assert_eq "T15k the worktree is clean, so \`git status\` reads its own opposite" "" \
    "$(git -C "$R15K" status --porcelain)"

# THE ASSERTION OF THE TICKET. Red before the fix: the `STATUS = success`
# short-circuit returns `not-applicable` whatever the rescue just did.
assert_eq "T15k the classifier reads \`absent\`, not \`not-applicable\`" \
    "absent" "$(STATUS=success traversal "$R15K")"

# ── T15k-bis (AC3) — armed, the term refuses, and it refuses FIRST ───────────
echo "-- T15k-bis: armed, the term refuses a rescue-committed success (AC3) --"
STATUS=success MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL=1 measure "$R15K" \
    && { FAIL=$((FAIL + 1)); echo "  ✗ T15k-bis refuses a rescue-committed success"; }
assert_eq "T15k-bis names the \`compound-traversal\` term" "compound-traversal" "$MEASURE_TERM"
assert_contains "T15k-bis excerpt names the status that was read" "$MEASURE_EXCERPT" "success"
# The second fact behind the refusal, and the reason a `success` is measured at
# all. Without it the sentence would name a concluded session and stop there.
assert_contains "T15k-bis excerpt names the auto-commit" "$MEASURE_EXCERPT" \
    "dispatch-lib committed in the pilot's place"
# The sentence must not contradict itself: this session DID conclude. Saying it
# did not would trade a false marker for a false sentence — the very exchange
# this ticket refuses to make at `_pilot_had_no_shipping_tail`.
assert_not_contains "T15k-bis excerpt does not claim the session failed to conclude" \
    "$MEASURE_EXCERPT" "did not conclude"
assert_contains "T15k-bis excerpt names the intrinsic way out" "$MEASURE_EXCERPT" "docs/solutions"
assert_contains "T15k-bis excerpt names the declarative way out" "$MEASURE_EXCERPT" "Compound: none"
assert_eq "T15k-bis term 0 short-circuits: no cargo, no verifier ran" "" "$(cat "$STUB_LOG")"
B15K=$(_compose_rescue_pr_body "$R15K" "dirty-worktree" "Class fact." "2631" "no" "compound-traversal" "excerpt" "absent")
assert_contains "T15k-bis body carries the traversal marker" "$B15K" "<!-- compound-traversal: absent -->"
assert_contains "T15k-bis body stays unverified" "$B15K" "<!-- rescue-pipeline-verified: no -->"

# Paired control: on a session cut short with no rescue commit, the suffix must
# NOT appear — it names a fact, so it may only be said when the fact holds.
STATUS=terminated RESCUED_DIRTY_WORKTREE=0 MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL=1 measure "$R15K" || true
assert_not_contains "T15k-bis no auto-commit claim without the stamp" "$MEASURE_EXCERPT" \
    "committed in the pilot's place"

# ── T15l (AC2) — POSITIVE CONTROL, the half that keeps the remedy from being
#                worse than the defect
# Without it, "the term bites" is indistinguishable from "the term bites
# everybody" — and the second reading stops the drain, `verify-pipeline.sh` being
# term 5 of this very conjunction.
echo "-- T15l: POSITIVE CONTROL — a success that committed its own work stays exempt (AC2) --"
R15L=$(make_repo t15l)
add_work "$R15L"
assert_eq "T15l the classifier reads \`not-applicable\`" \
    "not-applicable" "$(STATUS=success RESCUED_DIRTY_WORKTREE=0 traversal "$R15L")"
if STATUS=success RESCUED_DIRTY_WORKTREE=0 MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL=1 measure "$R15L"; then
    PASS=$((PASS + 1)); echo "  ✓ T15l armed, a self-committed success is NOT refused"
else
    FAIL=$((FAIL + 1)); echo "  ✗ T15l armed, a self-committed success is NOT refused"
    echo "    term:    '$MEASURE_TERM'"
    echo "    excerpt: '$MEASURE_EXCERPT'"
fi

# ── T15l-bis (AC2, D3) — the `no-shipping-tail` route stays exempt ───────────
# The loop's NOMINAL route, and the 71 % of merges carrying no `docs/solutions`.
# Biting here would re-bite the regime under another name.
assert_eq "T15l-bis the no-shipping-tail route is still exempt" "not-applicable" \
    "$(SKILL=dev-pilot PILOT_SHIPPING_TAIL=absent STATUS=success RESCUED_DIRTY_WORKTREE=0 traversal "$R15L")"

# ── T15m — the stamp does not mask a REAL attestation ────────────────────────
# A rescue that committed the pilot's learning HAS executed the decision. This
# fix removes an exemption; it does not manufacture a refusal.
echo "-- T15m: a rescued learning still attests (AC1) --"
R15M=$(make_repo t15m)
add_work "$R15M"
add_solution "$R15M"
assert_eq "T15m a rescued \`docs/solutions\` file attests" "attested-solution" \
    "$(STATUS=success RESCUED_DIRTY_WORKTREE=1 traversal "$R15M")"
if STATUS=success RESCUED_DIRTY_WORKTREE=1 MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL=1 measure "$R15M"; then
    PASS=$((PASS + 1)); echo "  ✓ T15m the conjunction continues past term 0"
else
    FAIL=$((FAIL + 1)); echo "  ✗ T15m the conjunction continues past term 0"
    echo "    term:    '$MEASURE_TERM'"
fi
assert_contains "T15m the cargo terms did run" "$(cat "$STUB_LOG")" "clippy"

R15M2=$(make_repo t15m2)
add_work "$R15M2"
git -C "$R15M2" commit -q --allow-empty --no-verify \
    -m "chore: no learning here" \
    -m "Compound: none — mechanical fix, the diff and its test say everything"
assert_eq "T15m a rescued anchored trailer still attests" "attested-trailer" \
    "$(STATUS=success RESCUED_DIRTY_WORKTREE=1 traversal "$R15M2")"

# ── T15n — the stamp's domain, exactly ──────────────────────────────────────
# Same exactness as `[ "${PILOT_SHIPPING_TAIL:-}" = "absent" ]`, and the
# fail-safe direction of D4: anything unreadable leaves today's behaviour, which
# is the prior state and not a new permission.
echo "-- T15n: only the literal \`1\` makes a concluded session measure (D4) --"
R15N=$(make_repo t15n)
add_work "$R15N"
for stamp in "0" "yes" "true" " 1 " "1 " "01"; do
    assert_eq "T15n stamp '$stamp' leaves the session exempt" "not-applicable" \
        "$(STATUS=success RESCUED_DIRTY_WORKTREE="$stamp" traversal "$R15N")"
done
if (unset RESCUED_DIRTY_WORKTREE; [ "$(STATUS=success _rescue_compound_traversal "$R15N")" = "not-applicable" ]); then
    PASS=$((PASS + 1)); echo "  ✓ T15n an absent stamp leaves the session exempt"
else
    FAIL=$((FAIL + 1)); echo "  ✗ T15n an absent stamp leaves the session exempt"
fi
assert_eq "T15n the literal \`1\` measures" "absent" \
    "$(STATUS=success RESCUED_DIRTY_WORKTREE=1 traversal "$R15N")"
# And the stamp changes nothing on a session that did NOT conclude: that
# population was already measured, by `STATUS` alone.
assert_eq "T15n a truncated session measures whatever the stamp says" "absent" \
    "$(STATUS=terminated RESCUED_DIRTY_WORKTREE=0 traversal "$R15N")"

# ── T15o (AC4) — the census is exhaustive, compared IN BOTH DIRECTIONS ───────
# Source scan. Every site of `dispatch-lib.sh` reading `STATUS` against the
# literal `success` appears in the census table, AND every entry of the table
# names a site that still exists. Cardinality asserted at 3 — the one failure
# shape no fixture sees: a predicate grown too narrow would pass by looking at
# nothing (mika#2496 U3, class mika#2205).
echo "-- T15o: the AC4 census covers every reader of \`STATUS = success\` (AC4) --"
STATUS_SUCCESS_SITES=$(grep -nE '\[ "\$\{?STATUS(:-)?\}?" = "success" \]' "$DISPATCH_LIB" | cut -d: -f1)
STATUS_SUCCESS_COUNT=$(printf '%s\n' "$STATUS_SUCCESS_SITES" | grep -c '[0-9]')
assert_eq "T15o there are exactly 3 readers of \`STATUS = success\`" "3" "$STATUS_SUCCESS_COUNT"

# The census lives on the file, above the new predicate, where it is exercised.
CENSUS_BLOCK=$(sed -n '/AC4 CENSUS — every reader of `STATUS = success`/,/^# ─\{20,\}$/p' "$DISPATCH_LIB")
if [ -n "$CENSUS_BLOCK" ]; then
    PASS=$((PASS + 1)); echo "  ✓ T15o the census block is on the file"
else
    FAIL=$((FAIL + 1)); echo "  ✗ T15o the census block is on the file"
fi
# Direction 1 — every reader is named. The enclosing function of each hit.
for _line in $STATUS_SUCCESS_SITES; do
    _owner=$(awk -v n="$_line" '
        /^[_a-zA-Z][_a-zA-Z0-9]*\(\) \{$/ { fn = $1; sub(/\(\)$/, "", fn) }
        /^    # mika#940 Unit 1: post-flight PR-existence check\.$/ { fn = "mika#940 Unit 1" }
        NR == n { print fn; exit }' "$DISPATCH_LIB")
    # An empty owner would make the assertion below vacuous: every string
    # contains the empty string.
    assert_eq "T15o the reader at line $_line has an enclosing site" "1" \
        "$( [ -n "$_owner" ] && echo 1 || echo 0 )"
    assert_contains "T15o the census names the reader at line $_line (\`$_owner\`)" \
        "$CENSUS_BLOCK" "$_owner"
done
# Direction 2 — every named site still exists. An entry whose site was renamed or
# removed must REDDEN the build rather than silently exempt a future homonym.
# Same shape as `FIRED_AT_LITERAL_WRITERS` (mika#2133) and `DISPATCH_ENV_KNOWN_INERT`
# (mika#2536).
CENSUS_ENTRIES=$(grep -oE '^# \| `?(_[a-z_]+|mika#940 Unit 1)`? ' <<<"$CENSUS_BLOCK" \
    | sed -E 's/^# \| `?//; s/`? $//')
CENSUS_ENTRY_COUNT=$(printf '%s\n' "$CENSUS_ENTRIES" | grep -c '.')
assert_eq "T15o the census carries exactly 3 entries" "3" "$CENSUS_ENTRY_COUNT"
while IFS= read -r _entry; do
    [ -n "$_entry" ] || continue
    case "$_entry" in
        "mika#940 Unit 1")
            assert_contains "T15o the \`$_entry\` site still exists" \
                "$(cat "$DISPATCH_LIB")" "# mika#940 Unit 1: post-flight PR-existence check." ;;
        *)
            assert_contains "T15o the \`$_entry\` site still exists" \
                "$(cat "$DISPATCH_LIB")" "${_entry}() {" ;;
    esac
done <<<"$CENSUS_ENTRIES"

# ── T15p (AC4) — the two ORDERS that protect `_pilot_had_no_shipping_tail` ───
# Structural, not behavioural, and that is the whole point: inverting either
# order would make NO decision wrong in any existing case, and would reopen this
# ticket in silence. What keeps site #2 correct is where its instructions sit.
echo "-- T15p: the orders protecting \`_pilot_had_no_shipping_tail\` hold (AC4) --"
_class_stamp_line=$(grep -n 'RECOVERY_CLASS="dirty-worktree"' "$DISPATCH_LIB" | head -1 | cut -d: -f1)
_class_tail_line=$(grep -n 'RECOVERY_CLASS="no-shipping-tail"' "$DISPATCH_LIB" | head -1 | cut -d: -f1)
assert_eq "T15p in the class computation, the stamp branch precedes the shipping-tail branch" "1" \
    "$( [ -n "$_class_stamp_line" ] && [ -n "$_class_tail_line" ] \
        && [ "$_class_stamp_line" -lt "$_class_tail_line" ] && echo 1 || echo 0 )"
_unit3_failure_line=$(grep -n 'grep -qF -- "PIPELINE FAILURE:" <<<"\$RESULT"' "$DISPATCH_LIB" | head -1 | cut -d: -f1)
_unit3_tail_line=$(grep -n 'elif _pilot_had_no_shipping_tail; then' "$DISPATCH_LIB" | head -1 | cut -d: -f1)
assert_eq "T15p in Unit 3, the \`PIPELINE FAILURE:\` arm precedes the shipping-tail arm" "1" \
    "$( [ -n "$_unit3_failure_line" ] && [ -n "$_unit3_tail_line" ] \
        && [ "$_unit3_failure_line" -lt "$_unit3_tail_line" ] && echo 1 || echo 0 )"

# ── T15q (R3) — `commit-pushed-no-pr` stays OUT of the population ────────────
# AC1's parenthesis ("or any class that auto-commits in the pilot's place") would,
# read literally, sweep in `commit-pushed-no-pr`, which does create a commit — the
# empty `wip(mika#1383)` marker. It must stay out: on that class the pilot
# committed ITS OWN work and only `gh pr create` failed. Letting it in would bite
# part of the nominal traffic under another name.
echo "-- T15q: commit-pushed-no-pr is not an auto-commit in the pilot's place (R3) --"
R15Q=$(make_repo t15q)
add_work "$R15Q"
git -C "$R15Q" commit -q --allow-empty --no-verify \
    -m "wip(mika#1383): auto-PR-create rescue for mika#2631"
assert_eq "T15q the marker commit does not make the class measure" "not-applicable" \
    "$(STATUS=success RESCUED_DIRTY_WORKTREE=0 traversal "$R15Q")"

# ── T15r (AC1) — Phase A commits in the pilot's place too ───────────────────
# Found in review: the mika#1383 Phase A rescue commits content the pilot wrote
# and never committed, on the HEAD-ADVANCED side. The pilot committed part of its
# work, left the rest dirty and handed the turn back — "in the pilot's place"
# exactly as AC1 means it — and the class reads `commit-pushed-no-pr`. Driven
# through the real `_post_flight_recovery` (patron `test_rescue_fmt_clean.sh`
# T-b), so the case proves Phase A really writes the stamp, not that the
# classifier reads a variable.
echo "-- T15r: the trailing-content rescue makes a concluded session measure (AC1) --"
# trailing_rescue <dir> — the HEAD-advanced shape: the pilot commits part of its
# work (`add_work`), leaves a plan file dirty, concludes in `success`; then the
# real `_post_flight_recovery` runs. `gh` is shadowed so the PR signal stays
# offline.
# shellcheck disable=SC2034  # globals read by the sourced dispatch-lib
trailing_rescue() {
    local repo="$1"
    WORKTREE_DIR="$repo"; SKILL="dev-pilot"; REPO="mika"; ISSUE_NUM="2631"
    BRANCH="fix/2631/dispatch-lib-compound-traversal-not"
    PILOT_EXIT=0; STATUS="success"; PR_URL=""; LOG_ID="log-test"
    RESULT="claude-pilot completed (status: success)."
    RESCUED_DIRTY_WORKTREE=0; RESCUED_TRAILING_CONTENT=0
    RESCUE_COMMITS=""; RESCUE_COMMITS_SIGNALLED=""
    PRE_RUN_HEAD=$(git -C "$repo" rev-parse HEAD)
    add_work "$repo"
    POST_RUN_HEAD=$(git -C "$repo" rev-parse HEAD)
    mkdir -p "$repo/docs/plans"
    printf '%s' "$PLAN_DOC" > "$repo/docs/plans/2026-10-02-002-fix-2631-trailing-plan.md"
    gh() { return 1; }
    _post_flight_recovery >/dev/null 2>&1 || true
    unset -f gh
}
R15R=$(make_repo t15r)
trailing_rescue "$R15R"
assert_eq "T15r Phase A committed the trailing content" "1" \
    "$(git -C "$R15R" log --format=%s | grep -c 'trailing content after pilot end_turn')"
assert_eq "T15r Phase A wrote its stamp" "1" "$RESCUED_TRAILING_CONTENT"
assert_eq "T15r the dirty-worktree stamp stayed down (the class is not dirty-worktree)" \
    "0" "$RESCUED_DIRTY_WORKTREE"
assert_eq "T15r the classifier reads \`absent\`, not \`not-applicable\`" \
    "absent" "$(STATUS=success traversal "$R15R")"
# And the stamp's domain is exact, like its sibling's (T15n).
for stamp in "0" "yes" " 1 "; do
    assert_eq "T15r trailing stamp '$stamp' leaves the session exempt" "not-applicable" \
        "$(STATUS=success RESCUED_DIRTY_WORKTREE=0 RESCUED_TRAILING_CONTENT="$stamp" traversal "$R15R")"
done
# Structural: the stamp is reset per dispatch, next to its sibling, so it cannot
# leak from an earlier dispatch in the same process.
assert_eq "T15r the trailing stamp is reset in \`_run_claude_pilot\`" "1" \
    "$(sed -n '/^_run_claude_pilot() {$/,/^}$/p' "$DISPATCH_LIB" | grep -c '^    RESCUED_TRAILING_CONTENT=0$')"
RESCUED_TRAILING_CONTENT=0

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed"
echo "========================================"
[ "$FAIL" -eq 0 ]

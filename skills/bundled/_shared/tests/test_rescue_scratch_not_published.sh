#!/bin/bash
# Test suite: the mika#1282 rescue does not publish a pilot's scratch probe as if
# it were an implementation (mika#2503).
#
# WHY. PR #2502 (impl of #2497) carried 28 files — `.v1probe/Cargo.toml`,
# `.v1probe/src/lib.rs` and the whole of `.v1probe/target/` (rlib, `incremental/`,
# fingerprints) — and zero implementation. QA blocked it 7/7. Same mechanism as
# #2486 the day before. The chain: `.gitignore` anchors `/target/` at the root, so
# `.v1probe/target/` was never an ignored path; the rescue's `git add -A` obeyed
# `.gitignore` exactly and had been told nothing about that path; and the
# "nothing staged" guard (mika#1288/#1419) saw a populated index because
# `.v1probe/**` is not scaffold.
#
# THE ASYMMETRY THIS SUITE EXISTS TO PROTECT, in both directions.
#   * A false POSITIVE — refusing to rescue real content — is an IRREVERSIBLE loss
#     of implementation: uncommitted pilot work exists in exactly one place and
#     `_set_up_worktree` force-removes the worktree on the next dispatch. That is
#     literally the defect mika#1282 exists to prevent.
#   * A false NEGATIVE — rescuing scratch — is a phantom PR that QA blocks and an
#     operator closes. Reversible, and it is the state of the world today.
# So V4/V5 (the nominal path still opens its PR) and V6..V10 (every unreadable
# signal opens the PR) are not padding: without them, a predicate that refused
# EVERYTHING would pass V1, V2 and V3 and would silently disable the loop.
#
# WHAT IS ASSERTED, by stage.
#   Stage 1 (`.gitignore`, `**/target/`)  — V11..V13, measured against the REAL
#     repository, because the rule is per-repository and a temp-repo assertion
#     would attest nothing about what ships.
#   Stage 2 (`_rescue_touches_tracked_tree`) — V1..V10, calling the REAL function
#     against real temp git repos.
#   Wiring (the refusal is where it must be) — V14..V18, static, because the
#     predicate being correct and the predicate being CONSULTED are two facts and
#     only the second one closes the ticket (see V16's comment).
#
# It calls the REAL functions rather than reimplementing them: the older rescue
# suites (test_auto_rescue_* in test-dispatch-lib.sh) reimplement the rescue and
# therefore cannot falsify the shipped code.
#
# Run: bash skills/bundled/_shared/tests/test_rescue_scratch_not_published.sh
# Expected: all assertions pass, exit 0. No network / cargo / claude-pilot needed.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"

# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

# dispatch-lib writes git noise to fd 9 (opened by the trace setup in a real
# dispatch). Open it here so the rescue's `2>&9` redirects have a destination.
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
    local label="$1" needle="$2" hay="$3"
    if grep -qF -- "$needle" <<<"$hay"; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    missing: '$needle'"
        echo "    in:      '$(printf '%s' "$hay" | head -c 400)'"
    fi
}

assert_not_contains() {
    local label="$1" needle="$2" hay="$3"
    if grep -qF -- "$needle" <<<"$hay"; then
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    unexpectedly present: '$needle'"
    else
        PASS=$((PASS + 1)); echo "  ✓ $label"
    fi
}

# `_rescue_touches_tracked_tree` answers by EXIT STATUS: 0 = open the PR,
# 1 = refuse. Render it as a word so a failure prints the verdict rather than a
# bare number.
#
# THE `declare -F` GUARD IS LOAD-BEARING, and it is this suite's own negative
# control. Without it, an ABSENT function makes bash exit 127, `if` takes the
# else branch, and `verdict` returns `refuse` — so V1/V2/V3 would pass on a
# dispatch-lib that has no fix at all. An assertion that passes for the wrong
# reason is worse than a missing one, and "a silently inert detector reads
# exactly like a healthy one" is the class this very ticket is about.
verdict() {
    local wt="${1-}"
    declare -F _rescue_touches_tracked_tree >/dev/null || { echo MISSING-FUNCTION; return; }
    if _rescue_touches_tracked_tree "$wt"; then echo open; else echo refuse; fi
}

# A worktree-shaped temp repo whose first-level tree is `crates/` + `docs/`, with
# `origin/main` set to that commit. `update-ref` gives the predicate a real
# remote-tracking ref with no network.
make_repo() {
    local repo
    repo="$(mktemp -d "${TMPDIR:-/tmp}/mika-2503-test.XXXXXX")"
    git -C "$repo" init -q
    git -C "$repo" config user.email test@example.com
    git -C "$repo" config user.name "Test"
    git -C "$repo" config commit.gpgsign false
    mkdir -p "$repo/crates/mika-agent/src" "$repo/docs/plans"
    printf 'pub fn seed() {}\n' > "$repo/crates/mika-agent/src/lib.rs"
    printf '# docs\n' > "$repo/docs/README.md"
    git -C "$repo" add -A
    git -C "$repo" commit -q -m "seed" --no-verify
    git -C "$repo" update-ref refs/remotes/origin/main HEAD
    printf '%s' "$repo"
}

# Set the globals `_rescue_dirty_worktree` reads, then call it. PRE/POST_RUN_HEAD
# are both the repo's current HEAD — the zero-commit shape the rescue is scoped
# to, and the shape #2502 was in (pilot exited 1 with HEAD unchanged).
#
# Stderr is CAPTURED, not silenced, into LAST_RESCUE_STDERR. Two reasons: the
# #2502 fixture legitimately contains `.v1probe/src/lib.rs`, so the proactive
# `cargo fmt` (mika#1336) runs and fails loudly on a temp repo with no
# Cargo.toml — fail-safe by design, 30 lines of usage text per case — and V19
# asserts on the scaffold guard's message, which is written to that same stream.
LAST_RESCUE_STDERR=""
run_rescue() {
    local repo="$1" skill="${2:-dev-pilot}" _err
    WORKTREE_DIR="$repo"
    SKILL="$skill"
    REPO="mika"
    ISSUE_NUM="2497"
    BRANCH="fix/2497/some-impl"
    SESSION_ID="sess-test"
    PILOT_EXIT=1
    RESULT="claude-pilot completed (status: terminated)."
    RESCUED_DIRTY_WORKTREE=""
    PRE_RUN_HEAD=$(git -C "$repo" rev-parse HEAD)
    POST_RUN_HEAD="$PRE_RUN_HEAD"
    _err="$(mktemp "${TMPDIR:-/tmp}/mika-2503-rescue-err.XXXXXX")"
    _rescue_dirty_worktree 2>"$_err" || true
    LAST_RESCUE_STDERR="$(cat "$_err")"
    rm -f "$_err"
}

# The exact shape of #2502: a scratch probe crate and its build artefacts, and
# NOTHING else. `target/` content is included on purpose — in a repo whose
# `.gitignore` does not carry `**/target/` (every repo dispatch-lib is deployed
# to except this one, until each gets the rule) stage 2 is the only thing
# standing between that content and a PR.
write_v1probe() {
    local repo="$1"
    mkdir -p "$repo/.v1probe/src" "$repo/.v1probe/target/debug/incremental"
    printf '[package]\nname = "v1probe"\n' > "$repo/.v1probe/Cargo.toml"
    printf 'pub fn probe() {}\n' > "$repo/.v1probe/src/lib.rs"
    printf 'rlib-bytes\n' > "$repo/.v1probe/target/debug/libv1probe.rlib"
    printf 'fingerprint\n' > "$repo/.v1probe/target/debug/incremental/x.bin"
}

echo ""
echo "===================================================================="
echo "Stage 2 — _rescue_touches_tracked_tree (the predicate that travels)"
echo "===================================================================="

# ============================================================================
# V1 (AC1): the founding case — a scratch probe alone must NOT become a PR
# ============================================================================
echo ""
echo "V1: #2502's content (.v1probe/** only) → refuse"
echo "-----------------------------------------------"
R="$(make_repo)"
write_v1probe "$R"
run_rescue "$R" "dev-pilot"
assert_eq "founding case: predicate refuses to open a PR" "refuse" "$(verdict "$R")"
assert_contains "founding case: the probe really was committed (so the refusal is what stops the PR, not an empty diff)" \
    ".v1probe/Cargo.toml" "$(git -C "$R" diff --name-only origin/main...HEAD)"
git clean -qfdx "$R" 2>/dev/null || true
rm -rf "$R"

# ============================================================================
# V2 (AC2): nothing is destroyed — the commit exists and the branch is pushable
# ============================================================================
echo ""
echo "V2: refusal destroys nothing — commit made, POST_RUN_HEAD advanced"
echo "-----------------------------------------------------------------"
R="$(make_repo)"
BEFORE_HEAD=$(git -C "$R" rev-parse HEAD)
write_v1probe "$R"
run_rescue "$R" "dev-pilot"
AFTER_HEAD=$(git -C "$R" rev-parse HEAD)
assert_eq "refusal still commits (content is not lost)" "1" \
    "$( [ "$BEFORE_HEAD" != "$AFTER_HEAD" ] && echo 1 || echo 0 )"
assert_eq "POST_RUN_HEAD advanced so _push_branch publishes the branch" "$AFTER_HEAD" "$POST_RUN_HEAD"
assert_eq "worktree is clean afterwards (nothing left behind to lose)" "" \
    "$(git -C "$R" status --porcelain)"
assert_contains "the probe is reachable from the commit" ".v1probe/src/lib.rs" \
    "$(git -C "$R" ls-tree -r --name-only HEAD)"
rm -rf "$R"

# ============================================================================
# V3 (AC1): a probe PLUS build artefacts under a tracked dir is still refused
#           only when no tracked first segment is touched. Here the probe is the
#           whole content, so: refuse. This is the negative control for V4.
# ============================================================================
echo ""
echo "V3: two scratch roots, no tracked segment → refuse"
echo "--------------------------------------------------"
R="$(make_repo)"
write_v1probe "$R"
mkdir -p "$R/.scratch"
printf 'notes\n' > "$R/.scratch/notes.txt"
run_rescue "$R" "dev-pilot"
assert_eq "several scratch roots, none tracked: refuse" "refuse" "$(verdict "$R")"
rm -rf "$R"

# ============================================================================
# V4 (AC4): THE POSITIVE CONTROL. Without this, a predicate that refused
#           everything would pass V1, V2, V3 and V5..V10 — and would disable the
#           autonomous loop while looking like a fix.
# ============================================================================
echo ""
echo "V4: real implementation under crates/ → open"
echo "--------------------------------------------"
R="$(make_repo)"
printf 'pub fn real_impl() {}\n' > "$R/crates/mika-agent/src/feature.rs"
run_rescue "$R" "dev-pilot"
assert_eq "nominal rescue path: predicate opens the PR" "open" "$(verdict "$R")"
assert_eq "nominal rescue path: draft-PR marker still set" "1" "$RESCUED_DIRTY_WORKTREE"
rm -rf "$R"

# ============================================================================
# V5 (AC4): mixed content — implementation AND a scratch probe → open.
#           The question is "does it touch the tree", not "is it pure".
# ============================================================================
echo ""
echo "V5: implementation + scratch probe together → open"
echo "--------------------------------------------------"
R="$(make_repo)"
write_v1probe "$R"
printf 'pub fn real_impl() {}\n' > "$R/crates/mika-agent/src/feature.rs"
run_rescue "$R" "dev-pilot"
assert_eq "mixed content: predicate opens the PR (never lose the impl half)" "open" "$(verdict "$R")"
rm -rf "$R"

# ============================================================================
# V5b (AC4): a brand-new crate under the tracked `crates/` root → open.
#            This is why the predicate is per-FIRST-SEGMENT and not
#            "is this path tracked": a new file is never tracked yet.
# ============================================================================
echo ""
echo "V5b: brand-new crate under a tracked root → open"
echo "------------------------------------------------"
R="$(make_repo)"
mkdir -p "$R/crates/mika-newthing/src"
printf '[package]\n' > "$R/crates/mika-newthing/Cargo.toml"
printf 'pub fn n() {}\n' > "$R/crates/mika-newthing/src/lib.rs"
run_rescue "$R" "dev-pilot"
assert_eq "new crate under tracked root: open" "open" "$(verdict "$R")"
rm -rf "$R"

# ============================================================================
# V5c (AC4): a plan under docs/plans/ → open. dev-groom's nominal output.
# ============================================================================
echo ""
echo "V5c: a groom plan under docs/ → open"
echo "------------------------------------"
R="$(make_repo)"
printf '# plan\n' > "$R/docs/plans/2026-09-23-004-fix-2503-x-plan.md"
run_rescue "$R" "dev-groom"
assert_eq "groom plan: open" "open" "$(verdict "$R")"
rm -rf "$R"

# ============================================================================
# V6..V10 (AC5): FAIL-OPEN on every unreadable signal, one term per assertion.
#   A single global "unreadable → open" test would pass while one branch was
#   silently fail-closed, and a fail-closed branch here blocks the loop's
#   nominal path (`no-shipping-tail`, mika#2492, goes through the same `if`).
#   This polarity is the INVERSE of `_rescue_diff_carries_work`'s twenty lines
#   above; each site carries its own reason so a reviewer cannot "harmonize"
#   the one they happen to be moving.
# ============================================================================
echo ""
echo "V6..V10 (AC5): every unreadable signal must open the PR"
echo "-------------------------------------------------------"

# V6: empty worktree dir. `git -C ""` silently operates on the dispatch process
# CWD, so this must not be measured at all — and the answer must be "open".
assert_eq "V6  empty worktree dir → open" "open" "$(verdict "")"

# V7: not a git repository.
NOT_A_REPO="$(mktemp -d "${TMPDIR:-/tmp}/mika-2503-norepo.XXXXXX")"
assert_eq "V7  not a git repo → open" "open" "$(verdict "$NOT_A_REPO")"
rm -rf "$NOT_A_REPO"

# V8: `origin/main` was never fetched — the diff cannot be computed.
R="$(mktemp -d "${TMPDIR:-/tmp}/mika-2503-noref.XXXXXX")"
git -C "$R" init -q
git -C "$R" config user.email test@example.com
git -C "$R" config user.name "Test"
git -C "$R" config commit.gpgsign false
printf 'seed\n' > "$R/README.md"
git -C "$R" add -A
git -C "$R" commit -q -m seed --no-verify
assert_eq "V8  no fetched origin/main → open" "open" "$(verdict "$R")"
rm -rf "$R"

# V9: diff is empty (branch identical to origin/main). Nothing to judge.
R="$(make_repo)"
assert_eq "V9  empty diff → open" "open" "$(verdict "$R")"
rm -rf "$R"

# V10: `ls-tree` of the reference is EMPTY — origin/main is an empty-tree commit,
# so no first segment can ever match. Fail-closed here would refuse every PR in
# a repo whose remote ref is degenerate.
R="$(mktemp -d "${TMPDIR:-/tmp}/mika-2503-emptytree.XXXXXX")"
git -C "$R" init -q
git -C "$R" config user.email test@example.com
git -C "$R" config user.name "Test"
git -C "$R" config commit.gpgsign false
git -C "$R" commit -q --allow-empty -m "empty root" --no-verify
git -C "$R" update-ref refs/remotes/origin/main HEAD
mkdir -p "$R/crates/x"
printf 'x\n' > "$R/crates/x/a.rs"
git -C "$R" add -A
git -C "$R" commit -q -m "content" --no-verify
assert_eq "V10 reference tree empty → open" "open" "$(verdict "$R")"
rm -rf "$R"

echo ""
echo "===================================================================="
echo "Stage 1 — .gitignore carries **/target/ (measured on THIS repo)"
echo "===================================================================="

# ============================================================================
# V11 (AC3): `**/target/` is ignored at any depth, and the rule that says so is
#            the one named. `check-ignore -v` prints the matching pattern, which
#            is what distinguishes "ignored by our rule" from "ignored by
#            accident under some other line".
# ============================================================================
echo ""
echo "V11: target/ is ignored at depth, by the **/target/ rule"
echo "--------------------------------------------------------"
CI_PROBE=$(git -C "$REPO_ROOT" check-ignore -v --no-index \
    .v1probe/target/debug/libv1probe.rlib 2>/dev/null || true)
assert_contains "V11a .v1probe/target/... is ignored" ".gitignore" "$CI_PROBE"
assert_contains "V11b ...and the matching rule is **/target/" "**/target/" "$CI_PROBE"
CI_CRATE=$(git -C "$REPO_ROOT" check-ignore -v --no-index \
    crates/foo/target/y 2>/dev/null || true)
assert_contains "V11c crates/foo/target/y is ignored" "**/target/" "$CI_CRATE"
CI_ROOT=$(git -C "$REPO_ROOT" check-ignore -v --no-index target/debug/mika 2>/dev/null || true)
assert_contains "V11d the root target/ is STILL ignored (no regression)" "**/target/" "$CI_ROOT"

# ============================================================================
# V12 (AC3): no currently-tracked path becomes ignored BY THIS RULE.
#
#   Deliberately scoped to `target/` rather than asserting the whole
#   tracked-and-ignored set is empty: it is NOT empty today and was not before
#   this change — `.context/compound-engineering/` was added to .gitignore after
#   those files were already tracked (15 paths, measured 2026-09-23). A global
#   assertion would inherit that pre-existing debt and fail for a reason this
#   ticket does not own.
# ============================================================================
echo ""
echo "V12: no tracked path is swallowed by **/target/"
echo "-----------------------------------------------"
assert_eq "V12a no tracked path lives under a target/ dir" "" \
    "$(git -C "$REPO_ROOT" ls-files | grep -E '(^|/)target/' || true)"
assert_eq "V12b no tracked-and-ignored path is a target/ path" "" \
    "$(git -C "$REPO_ROOT" ls-files -c -i --exclude-standard | grep -E '(^|/)target/' || true)"

# ============================================================================
# V13 (the stage-1 mechanism, and the reason it repairs `git add -A` without
#      touching it): `git status --porcelain` — the dirty-worktree probe
#      `_rescue_dirty_worktree` interrogates — does not report ignored files. So
#      a pilot that leaves only build artefacts behind no longer triggers a
#      rescue AT ALL. Asserted on a temp repo carrying this repo's rule, because
#      the claim is about git's behaviour under the rule, not about this tree.
# ============================================================================
echo ""
echo "V13: with the rule, a target/-only dirty tree triggers no rescue"
echo "----------------------------------------------------------------"
R="$(make_repo)"
printf '**/target/\n' > "$R/.gitignore"
git -C "$R" add .gitignore
git -C "$R" commit -q -m "ignore target" --no-verify
git -C "$R" update-ref refs/remotes/origin/main HEAD
BEFORE_HEAD=$(git -C "$R" rev-parse HEAD)
mkdir -p "$R/.v1probe/target/debug" "$R/crates/mika-agent/target/debug"
printf 'rlib\n' > "$R/.v1probe/target/debug/libv1probe.rlib"
printf 'rlib\n' > "$R/crates/mika-agent/target/debug/x.rlib"
assert_eq "V13a git status does not report the ignored artefacts" "" \
    "$(git -C "$R" status --porcelain)"
run_rescue "$R" "dev-pilot"
assert_eq "V13b HEAD did not move — the rescue never fired" "$BEFORE_HEAD" \
    "$(git -C "$R" rev-parse HEAD)"
assert_eq "V13c no draft-PR marker" "" "$RESCUED_DIRTY_WORKTREE"
rm -rf "$R"

echo ""
echo "===================================================================="
echo "Wiring — the refusal is consulted where it must be"
echo "===================================================================="

# The Unit 2 gate. The predicate being correct and the predicate being CONSULTED
# are two different facts, and only the second closes the ticket.
RESCUE_BLOCK=$(sed -n '/^_rescue_dirty_worktree() {/,/^}$/p' "$DISPATCH_LIB")

# ============================================================================
# V14 (AC8): THE CENTRAL TRAP, pinned.
#
#   Not setting RESCUED_DIRTY_WORKTREE=1 is NOT enough. The rescue commit
#   advances POST_RUN_HEAD, so `PRE_RUN_HEAD != POST_RUN_HEAD` becomes true and
#   the `commit-pushed-no-pr` branch of the RECOVERY_CLASS computation opens the
#   PR anyway. A fix placed at the flag site alone would pass every behavioural
#   test above and change NOTHING in production.
#
#   So the refusal must be a conjunctive term on the Unit 2 gate — which covers
#   all three classes by construction — and must NOT live inside
#   _rescue_dirty_worktree.
# ============================================================================
echo ""
echo "V14 (AC8): the refusal gates PR opening for all three classes"
echo "-------------------------------------------------------------"
# The four PR-due conditions are stated exactly once, so a refusal that clears
# the flag cannot be bypassed by a second branch restating them.
assert_eq "V14a the four PR-due conditions are stated exactly once" "1" \
    "$(grep -c '\[ -n "\$RECOVERY_CLASS" \] && \[ -n "\$REPO" \]' "$DISPATCH_LIB")"
# The refusal is consulted AFTER RECOVERY_CLASS is computed and clears the flag.
assert_contains "V14b the refusal consults the predicate and clears the PR-due flag" \
    "_recovery_pr_due=0" \
    "$(grep -A 2 '_recovery_pr_due" = "1" \] && ! _rescue_touches_tracked_tree' "$DISPATCH_LIB")"
# And the PR-opening body is gated on that same flag — so all three classes are
# covered by construction, never by an enumeration that can fall out of step.
assert_eq "V14c PR opening is gated on the PR-due flag" "1" \
    "$(grep -c '^    if \[ "\$_recovery_pr_due" = "1" \]; then$' "$DISPATCH_LIB")"
assert_not_contains "V14d the refusal is NOT buried in _rescue_dirty_worktree" \
    "_rescue_touches_tracked_tree" "$RESCUE_BLOCK"
# The trap itself, named: the `commit-pushed-no-pr` class is reached with the
# flag alone, so the refusal must not be keyed on RESCUED_DIRTY_WORKTREE.
assert_not_contains "V14e the refusal is not keyed on RESCUED_DIRTY_WORKTREE (the mika#2503 trap)" \
    'RESCUED_DIRTY_WORKTREE' \
    "$(grep -B 2 -A 2 '! _rescue_touches_tracked_tree "\$WORKTREE_DIR"' "$DISPATCH_LIB")"

# ============================================================================
# V15: the predicate lives next to its opposite-polarity sibling, and each
#      states its own polarity. A reviewer who "harmonizes" the two re-opens
#      either this ticket or mika#2157.
# ============================================================================
echo ""
echo "V15: both polarities are stated at their own site"
echo "-------------------------------------------------"
PRED_BLOCK=$(sed -n '/^_rescue_touches_tracked_tree() {/,/^}$/p' "$DISPATCH_LIB")
assert_eq "V15a the predicate is defined exactly once" "1" \
    "$(grep -c '^_rescue_touches_tracked_tree() {' "$DISPATCH_LIB")"
assert_contains "V15b its doc-comment names its FAIL-OPEN polarity" "FAIL-OPEN" \
    "$(grep -B 60 '^_rescue_touches_tracked_tree() {' "$DISPATCH_LIB")"
assert_contains "V15c the sibling still states its fail-closed polarity" "Fail-closed" \
    "$(grep -B 60 '^_rescue_diff_carries_work() {' "$DISPATCH_LIB")"

# ============================================================================
# V16: the reference tree is origin/main, never HEAD.
#
#   MEASURED 2026-09-23: after the rescue commit, `git ls-tree --name-only HEAD`
#   CONTAINS `.v1probe` — the rescue itself just put it there. A predicate based
#   on HEAD therefore answers "yes" on the founding case and the whole fix is
#   inert. Same base as the diff (`origin/main...HEAD`), which is also what makes
#   the two halves of the measurement consistent.
# ============================================================================
echo ""
echo "V16: the reference tree is origin/main, not HEAD"
echo "------------------------------------------------"
assert_contains "V16a the predicate reads ls-tree of origin/main" "ls-tree" "$PRED_BLOCK"
assert_not_contains "V16b it never uses ls-tree HEAD (that would be inert)" \
    "ls-tree --name-only HEAD" "$PRED_BLOCK"

# ============================================================================
# V17 (AC6): the refusal is readable without a grep.
#   Unit 2 runs at `dispatch_claude_pilot` level, in the same regime as
#   `_check_pilot_force_push`: its stderr is `spawn_long_running_exec`'s
#   `Stdio::piped()` handle, which the executor reads ONLY in its
#   `if !status.success()` branch. On a dispatch that succeeds the pipe is
#   dropped unread — that is Signal M, and the mika#2050 class corrected THREE
#   times on Signal S. So the operator surface is RESULT (the callback body,
#   which lands in `tasks.result`), never a log line.
# ============================================================================
echo ""
echo "V17 (AC6): the refusal names itself, its branch and the manual gesture"
echo "----------------------------------------------------------------------"
REFUSAL_REGION=$(sed -n '/_recovery_pr_due" = "1" \] && ! _rescue_touches_tracked_tree/,/^    fi$/p' "$DISPATCH_LIB")
assert_contains "V17a the refusal writes to RESULT (the callback body)" "RESULT=" "$REFUSAL_REGION"
assert_contains "V17b it names the branch" 'branch ${BRANCH} is pushed' "$REFUSAL_REGION"
assert_contains "V17c it names the manual gesture" "gh pr create" "$REFUSAL_REGION"
assert_contains "V17d it names its ticket" "mika#2503" "$REFUSAL_REGION"
# mika#2121 totality contract: exactly one PR-status line per delivered callback,
# written through the canonical helper, and the reason a bare snake_case token —
# `dispatcher.rs::RE_NO_PR` is `(?m)^NO_PR:\s+([a-z_]+)`, declared class B in
# scripts/canonical-tokens.tsv. Prose on that line would have the parser record
# the sentence's first lowercase word as the reason.
assert_contains "V17e the status line goes through _set_pr_status_line (mika#2121)" \
    '_set_pr_status_line "NO_PR: rescue_scratch_only"' "$REFUSAL_REGION"
assert_eq "V17f the NO_PR reason is a bare snake_case token (wire format)" "1" \
    "$(printf '%s' "$REFUSAL_REGION" | grep -cE '_set_pr_status_line "NO_PR: [a-z_]+"$')"

# ============================================================================
# V18/V19 (AC7): the scaffold guard's message no longer lies.
#   An operator who reads "dirty worktree contained only scaffold paths
#   (.claude/commands/, .claude/claude-pilot.json)" after a pilot wrote 28 files
#   of build artefacts goes looking in the wrong place. V18 is static (the
#   hardcoded lie is gone); V19 is BEHAVIOURAL, because "does not say the wrong
#   thing" and "says the right thing" are two facts and the static half can only
#   see the first.
# ============================================================================
echo ""
echo "V18 (AC7): the hardcoded scaffold list is gone from the guard"
echo "-------------------------------------------------------------"
assert_not_contains "V18 the guard no longer hardcodes its own path list" \
    "only scaffold paths (.claude/commands/, .claude/claude-pilot.json)" \
    "$RESCUE_BLOCK"

echo ""
echo "V19 (AC7): the guard reports the paths it actually excluded"
echo "-----------------------------------------------------------"
R="$(make_repo)"
BEFORE_HEAD=$(git -C "$R" rev-parse HEAD)
mkdir -p "$R/.claude/commands"
printf 'slash\n' > "$R/.claude/commands/mika.md"
printf '{}\n' > "$R/.claude/settings.local.json"
run_rescue "$R" "dev-pilot"
assert_eq "V19a scaffold-only: HEAD did not move" "$BEFORE_HEAD" "$(git -C "$R" rev-parse HEAD)"
assert_eq "V19b scaffold-only: draft-PR marker explicitly cleared" "0" "$RESCUED_DIRTY_WORKTREE"
assert_contains "V19c the message names a path that was really excluded" \
    ".claude/commands/mika.md" "$LAST_RESCUE_STDERR"
rm -rf "$R"

echo ""
echo "===================================================================="
echo "Passed: $PASS   Failed: $FAIL"
echo "===================================================================="
[ "$FAIL" -eq 0 ] || exit 1

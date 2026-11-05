#!/bin/bash
# Test suite: the end-of-flight rescue does not publish a tree that is already
# `main`'s (mika#3038).
#
# WHY. Four rescue drafts since 2026-09-04 — #2197, #2702, #2899, #3034 — opened
# on a head whose tree was BYTE-IDENTICAL to `origin/main`'s. All four carried
# `rescue-diff: carries-work`, all four opened less than 20 minutes after a merge
# of a PR on their own branch, all four were closed by hand by the operator. On
# #3034 GitHub displayed `+953 −23` while `d97b53bdf^{tree}` was exactly
# `d7de73137^{tree}`, and six seconds after that draft opened the `ready` label
# was stripped off another ticket on its account.
#
# THE ILLUSION COMES FROM THE SQUASH, and it is the whole ticket. A squash merge
# puts a commit on `main` whose CONTENT is the branch's, without any branch
# commit being an ancestor of main. The merge base therefore stays where it was
# before the merge, and the three-dot form shows the already-merged work all over
# again. The tree cannot be fooled: it is the content, not the history.
#
# THE ASYMMETRY THIS SUITE PROTECTS, in both directions.
#   * A false POSITIVE — withholding a draft that carried real work — loses the
#     pilot's implementation from the only automatic surface that would have
#     published it. That is the defect mika#1282 exists to prevent.
#   * A false NEGATIVE — opening a draft on an empty tree — is a phantom PR an
#     operator closes, after it has stripped a `ready` label somewhere.
# So V2, V7, V10 and V11 (the draft still opens, the branch is still pushed) are
# not padding: without them a predicate that answered "equal" to everything would
# pass V1, V3..V6, V8, V9 and V12 while silently disabling the loop.
#
# WHAT IS ASSERTED, by acceptance criterion.
#   AC1 — V1..V5: no draft opens on an equal tree, whatever the class, and the
#         callback carries exactly one `NO_PR:` line with a bare snake_case
#         reason distinct from `rescue_scratch_only`.
#   AC2 — V6..V8: the `rescue-diff` marker measures what the branch ADDS to
#         `origin/main`, proven with a real squash.
#   AC3 — V9..V10: a branch the merge deleted from `origin` is not re-pushed
#         there when it adds nothing.
#   AC4 — V11..V12: an unmeasurable comparison restores the pre-ticket behaviour
#         and says so in exactly one line.
#   S1  — the wiring scan: the predicate being correct and the predicate being
#         CONSULTED are two facts, and only the second one closes the ticket.
#
# It calls the REAL functions against real temp git repos, with a real local
# `origin` (a second, bare repository) rather than a fake `git`: the claim "the
# branch is not re-pushed" is then measured on the remote itself instead of on an
# argv. Only `gh` is faked, because there is no local forge.
#
# Run: bash skills/bundled/_shared/tests/test_rescue_tree_equals_main.sh
# Expected: all assertions pass, exit 0. No network / cargo / claude-pilot needed.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"
DISPATCHER_RS="$REPO_ROOT/crates/mika-agent/src/task_engine/dispatcher.rs"

# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

# dispatch-lib writes git noise to fd 9 (opened by the trace setup in a real
# dispatch). Open it here so the rescue's `2>&9` redirects have a destination.
exec 9>/dev/null

# The pipeline measurement is disarmed: it shells out to cargo and to
# `scripts/verify-pipeline.sh`, neither of which means anything in a temp repo,
# and this ticket's question is upstream of it. The kill-switch is the library's
# own (mika#2354), so the body stays byte-identical to its pre-mika#2354 shape.
export MIKA_RESCUE_VERIFY_ENABLED=0

TMPROOT="$(mktemp -d "${TMPDIR:-/tmp}/mika-3038-suite.XXXXXX")"

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
        echo "    in:      '$(printf '%s' "$hay" | head -c 600)'"
    fi
}

assert_not_contains() {
    local label="$1" needle="$2" hay="$3"
    if grep -qF -- "$needle" <<<"$hay"; then
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    unexpectedly present: '$needle'"
        echo "    in:      '$(printf '%s' "$hay" | head -c 600)'"
    else
        PASS=$((PASS + 1)); echo "  ✓ $label"
    fi
}

# ─────────────────────────────────────────────────────────────────────────────
# Verdict renderers
#
# THE `declare -F` GUARD IS LOAD-BEARING, and it is this suite's own negative
# control. Without it an ABSENT function makes bash exit 127, the `case` falls
# through, and the renderer would answer a plausible word — so the refusal
# assertions would pass against a dispatch-lib carrying no fix at all. An
# assertion that passes for the wrong reason is worse than a missing one, and "a
# silently inert detector reads exactly like a healthy one" is the very class
# this ticket is about.
# ─────────────────────────────────────────────────────────────────────────────

# `_branch_tree_equals_main` answers by exit status: 0 equal, 1 different,
# 2 undetermined. Rendered as a word so a failure prints the verdict.
#
# `_MAIN_REF_FETCHED` is reset here because each probe stands for its own
# dispatch; the once-per-dispatch property itself is asserted by V11, which
# deliberately does NOT reset between its two consultation sites.
tree_verdict() {
    local wt="${1-}" rc=0
    declare -F _branch_tree_equals_main >/dev/null || { echo MISSING-FUNCTION; return; }
    _MAIN_REF_FETCHED=0
    _branch_tree_equals_main "$wt" || rc=$?
    case "$rc" in
        0) echo equal ;;
        1) echo differs ;;
        2) echo undetermined ;;
        *) echo "unexpected-rc-$rc" ;;
    esac
}

# `_rescue_diff_carries_work` answers 0 = carries work, 1 = carries none.
carries_verdict() {
    local wt="${1-}"
    declare -F _rescue_diff_carries_work >/dev/null || { echo MISSING-FUNCTION; return; }
    if _rescue_diff_carries_work "$wt"; then echo work; else echo none; fi
}

# ─────────────────────────────────────────────────────────────────────────────
# Fixtures
# ─────────────────────────────────────────────────────────────────────────────

_gitcfg() {
    git -C "$1" config user.email test@example.com
    git -C "$1" config user.name "Test"
    git -C "$1" config commit.gpgsign false
    git -C "$1" config gc.auto 0
}

# A worktree plus a real `origin`. Returns the base dir; `$base/wt` is the
# worktree and `$base/origin.git` the bare remote. The first-level tree is
# `crates/` + `docs/`, so `_rescue_touches_tracked_tree` (the mika#2503 refusal
# sitting right after ours) always answers "open" and can never be the reason a
# draft is withheld in this suite.
make_pair() {
    local base origin wt
    base="$(mktemp -d "$TMPROOT/pair.XXXXXX")"
    origin="$base/origin.git"
    wt="$base/wt"
    git init -q --bare "$origin"
    git init -q "$wt"
    _gitcfg "$wt"
    mkdir -p "$wt/crates/mika-agent/src" "$wt/docs/plans"
    printf 'pub fn seed() {}\n' > "$wt/crates/mika-agent/src/lib.rs"
    printf '# docs\n' > "$wt/docs/README.md"
    git -C "$wt" add -A
    git -C "$wt" commit -q -m "seed" --no-verify
    git -C "$wt" branch -M main
    git -C "$wt" remote add origin "$origin"
    git -C "$wt" push -q origin main
    printf '%s' "$base"
}

# Branch off main, add a file, commit, publish the branch.
start_branch() {
    local wt="$1" branch="$2" path="$3"
    git -C "$wt" checkout -q -b "$branch"
    mkdir -p "$(dirname "$wt/$path")"
    printf 'pub fn flight_work() {}\n' > "$wt/$path"
    git -C "$wt" add -A
    git -C "$wt" commit -q -m "feat: the pilot's work" --no-verify
    git -C "$wt" push -q origin "$branch"
}

# THE PRODUCTION SHAPE. `main` squash-merges the branch: its TREE becomes the
# branch's while no branch commit is an ancestor of main, and the forge deletes
# the branch from `origin`. `delete_remote_branch=1` reproduces that deletion,
# which is what AC3 is about.
squash_merge_branch() {
    local wt="$1" branch="$2" delete_remote="${3:-0}"
    git -C "$wt" checkout -q main
    git -C "$wt" merge -q --squash "$branch"
    git -C "$wt" commit -q -m "squash merge of $branch (#1)" --no-verify
    git -C "$wt" push -q origin main
    [ "$delete_remote" = "1" ] && git -C "$wt" push -q origin --delete "$branch"
    git -C "$wt" checkout -q "$branch"
    git -C "$wt" fetch -q origin main
    return 0
}

# A fake `gh`: there is no local forge, so the only honest substitute is one that
# records its argv. Every other measurement in this suite is made on real git
# state.
FAKE_BIN="$TMPROOT/bin"
mkdir -p "$FAKE_BIN"
export T3038_GH_LOG="$TMPROOT/gh-argv.log"
: > "$T3038_GH_LOG"
{
    printf '#!/bin/bash\n'
    printf 'printf "%%s\\n" "$*" >> "$T3038_GH_LOG"\n'
    printf 'if [ "$1 $2" = "pr create" ]; then\n'
    printf '  printf "https://github.com/senara-solutions/mika/pull/9999\\n"\n'
    printf 'fi\n'
    printf 'exit 0\n'
} > "$FAKE_BIN/gh"
chmod +x "$FAKE_BIN/gh"
export PATH="$FAKE_BIN:$PATH"

gh_argv() { cat "$T3038_GH_LOG"; }

# ─────────────────────────────────────────────────────────────────────────────
# The rescue gate, made callable
#
# The decision this ticket changes lives inside `dispatch_claude_pilot`, a
# function whose body reaches the network, the forge and cargo. The region from
# `RECOVERY_CLASS` to the end of the PR-opening block is extracted VERBATIM from
# the shipped file and wrapped in a function, so what runs below is the shipped
# code and not a restatement of it — the older rescue suites reimplement the
# rescue and therefore cannot falsify what ships.
#
# `source` of a generated wrapper rather than anything that evaluates a string:
# the region declares `local`s, which is why it needs a function to live in.
# ─────────────────────────────────────────────────────────────────────────────
GATE_LIB="$TMPROOT/t3038-gate.sh"
{
    printf '_t3038_rescue_gate() {\n'
    sed -n '/^    local RECOVERY_CLASS=""$/,/^    _deliver_callback$/p' "$DISPATCH_LIB" | sed '$d'
    printf '}\n'
} > "$GATE_LIB"
# shellcheck source=/dev/null
source "$GATE_LIB"

LAST_GATE_STDERR=""

# The globals a real dispatch holds when it reaches the gate. Resetting
# `_MAIN_REF_FETCHED` and the AC4 once-per-dispatch flag here is what makes each
# case a distinct dispatch.
start_dispatch() {
    WORKTREE_DIR="$1"
    BRANCH="$2"
    SKILL="${3:-dev-pilot}"
    REPO="mika"
    ISSUE_NUM="3038"
    ISSUE_TITLE="le secours ne publie pas un arbre déjà mergé"
    LABELS="ready"
    SESSION_ID="sess-3038"
    TURNS="42"
    COST="1.23"
    STATUS="success"
    PILOT_EXIT=0
    PILOT_SHIPPING_TAIL="absent"
    PR_URL=""
    RESCUED_PR_URL=""
    RESCUED_DIRTY_WORKTREE=""
    RESCUED_TRAILING_CONTENT=""
    PRE_RUN_HEAD=""
    POST_RUN_HEAD=""
    RESULT="claude-pilot completed (status: success)."
    _MAIN_REF_FETCHED=0
    _TREE_COMPARE_UNDETERMINED_NOTED=0
    : > "$T3038_GH_LOG"
}

# The `no-shipping-tail` class: HEAD moved during the flight and the pilot's
# perimeter never included opening a PR. The only class the four measured drafts
# went through; `commit-pushed-no-pr` shares the same entry test.
pose_class_no_shipping_tail() {
    local wt="$1"
    PRE_RUN_HEAD="$(git -C "$wt" rev-parse 'HEAD~1' 2>/dev/null || git -C "$wt" rev-parse HEAD)"
    POST_RUN_HEAD="$(git -C "$wt" rev-parse HEAD)"
    PILOT_SHIPPING_TAIL="absent"
    SKILL="dev-pilot"
    STATUS="success"
}

run_gate() {
    local _err
    _err="$(mktemp "$TMPROOT/gate-err.XXXXXX")"
    declare -F _t3038_rescue_gate >/dev/null || { LAST_GATE_STDERR="MISSING-GATE"; return; }
    _t3038_rescue_gate 2>"$_err" || true
    LAST_GATE_STDERR="$(cat "$_err")"
    rm -f "$_err"
}

run_push() {
    local _err
    _err="$(mktemp "$TMPROOT/push-err.XXXXXX")"
    _push_branch 2>"$_err" || true
    LAST_PUSH_STDERR="$(cat "$_err")"
    rm -f "$_err"
}
LAST_PUSH_STDERR=""

remote_has() {
    local wt="$1" branch="$2"
    if [ -n "$(git -C "$wt" ls-remote origin "refs/heads/$branch" 2>/dev/null)" ]; then
        echo yes
    else
        echo no
    fi
}

# The harness's own negative control: a degenerate extraction (an anchor that
# stopped matching after a refactor) would make every "no draft opened"
# assertion below pass vacuously.
echo ""
echo "===================================================================="
echo "Harness self-check — the extracted gate is the shipped gate"
echo "===================================================================="
GATE_BODY="$(cat "$GATE_LIB")"
assert_eq "H1 the gate wrapper is defined" "1" \
    "$(declare -F _t3038_rescue_gate >/dev/null && echo 1 || echo 0)"
assert_contains "H2 the extracted region really holds the PR-opening call" "gh pr create" "$GATE_BODY"
assert_contains "H3 ...and the PR-due flag it is gated on" '_recovery_pr_due' "$GATE_BODY"
assert_contains "H4 ...and the three recovery classes" "no-shipping-tail" "$GATE_BODY"

echo ""
echo "===================================================================="
echo "AC1 — an equal tree opens no draft, whatever the class"
echo "===================================================================="

# ============================================================================
# V1 (AC1): the founding case. Branch merged by squash, tree equal to main's,
#           class `no-shipping-tail` — the only class the four measured drafts
#           went through. No draft may open.
# ============================================================================
echo ""
echo "V1: squashed branch, tree == origin/main → no draft opens"
echo "---------------------------------------------------------"
B="$(make_pair)"; W="$B/wt"
start_branch "$W" "fix/3038/flight" "crates/mika-agent/src/feature.rs"
squash_merge_branch "$W" "fix/3038/flight"
assert_eq "V1a the fixture really is the production shape: tree equal" "equal" "$(tree_verdict "$W")"
assert_eq "V1b ...and no branch commit is an ancestor of main (the squash)" "1" \
    "$(git -C "$W" merge-base --is-ancestor HEAD origin/main >/dev/null 2>&1 && echo 0 || echo 1)"
start_dispatch "$W" "fix/3038/flight"
pose_class_no_shipping_tail "$W"
run_gate
assert_not_contains "V1c no draft PR is opened" "pr create" "$(gh_argv)"
assert_eq "V1d PR_URL stays empty" "" "$PR_URL"
V1_RESULT="$RESULT"

# ============================================================================
# V2 (AC1): THE NEGATIVE CONTROL, in the same call — the #3016 case. A prior
#           merge exists on the branch AND the branch carries a real diff: the
#           draft must still open. Without this, a predicate answering "equal"
#           to everything would pass every other AC1 assertion.
# ============================================================================
echo ""
echo "V2: prior merge BUT a real diff (the #3016 case) → draft opens"
echo "--------------------------------------------------------------"
B2="$(make_pair)"; W2="$B2/wt"
start_branch "$W2" "fix/3016/flight" "crates/mika-agent/src/feature.rs"
squash_merge_branch "$W2" "fix/3016/flight"
printf 'pub fn still_unmerged() {}\n' > "$W2/crates/mika-agent/src/later.rs"
git -C "$W2" add -A
git -C "$W2" commit -q -m "feat: work the merge did not carry" --no-verify
assert_eq "V2a the witness fixture differs from main" "differs" "$(tree_verdict "$W2")"
start_dispatch "$W2" "fix/3016/flight"
pose_class_no_shipping_tail "$W2"
run_gate
assert_contains "V2b the draft PR is opened" "pr create" "$(gh_argv)"
assert_contains "V2c and the callback carries its PR line" "PR: https://github.com/" "$RESULT"

# ============================================================================
# V3 (AC1): exactly one `NO_PR:` line in the callback. `_set_pr_status_line`
#           makes the totality contract true by construction (mika#2121); the
#           assertion is here because a second site writing its own line is the
#           regression that contract exists to forbid.
# ============================================================================
echo ""
echo "V3: the callback carries exactly one NO_PR line"
echo "-----------------------------------------------"
assert_eq "V3a exactly one line-anchored NO_PR:" "1" \
    "$(printf '%s\n' "$V1_RESULT" | grep -c '^NO_PR: ' || true)"
assert_eq "V3b and no PR: line alongside it" "0" \
    "$(printf '%s\n' "$V1_RESULT" | grep -c '^PR: ' || true)"
assert_contains "V3c the prose names the ticket and the manual gesture" "mika#3038" "$V1_RESULT"
assert_contains "V3d ...and the command that settles it" "diff origin/main HEAD" "$V1_RESULT"

# ============================================================================
# V4 (AC1): the reason is a bare snake_case token the consumer can read, and it
#           is DISTINCT from `rescue_scratch_only`.
#
#   The consumer is strict: `dispatcher.rs::RE_NO_PR` is
#   `(?m)^NO_PR:\s+([a-z_]+)`, declared class B in scripts/canonical-tokens.tsv.
#   Prose on that line would have the parser record the sentence's first
#   lowercase word as the reason — which is why the token is asserted on the
#   SHAPE and not only on its spelling.
# ============================================================================
echo ""
echo "V4: the reason is a readable snake_case token, distinct from mika#2503's"
echo "-----------------------------------------------------------------------"
V1_REASON="$(printf '%s\n' "$V1_RESULT" | sed -n 's/^NO_PR:[[:space:]]\{1,\}\([a-z_]\{1,\}\)$/\1/p')"
assert_eq "V4a the line matches the consumer's regex and yields a bare token" \
    "rescue_adds_nothing_to_main" "$V1_REASON"
assert_eq "V4b the reason is not mika#2503's" "1" \
    "$( [ "$V1_REASON" != "rescue_scratch_only" ] && echo 1 || echo 0 )"
# Inscribed WHERE the existing reasons are, which is two places and not one: the
# nominative list in `parse_no_pr_reason`'s doc-comment — the only enumeration an
# operator reads to know what the loop can answer — and the Rust test table that
# holds that enumeration to the parser. A reason the parser accepts but no list
# names is a reason nobody can look up.
assert_eq "V4c the token is named at least twice in dispatcher.rs (doc list + test table)" "1" \
    "$( [ "$(grep -c 'rescue_adds_nothing_to_main' "$DISPATCHER_RS")" -ge 2 ] && echo 1 || echo 0 )"
assert_contains "V4d ...and the Rust test table is the second of them" \
    "rescue_adds_nothing_to_main" \
    "$(sed -n '/fn test_parse_no_pr_reason_known_reasons/,/^    }$/p' "$DISPATCHER_RS")"

# ============================================================================
# V5 (AC1): all three recovery classes refuse. The refusal is a term on the
#           shared PR-due flag rather than on a class, so this holds by
#           construction — and the assertion is what keeps it that way.
# ============================================================================
echo ""
echo "V5: the three recovery classes all withhold the draft"
echo "-----------------------------------------------------"
for _class in dirty-worktree no-shipping-tail commit-pushed-no-pr; do
    BC="$(make_pair)"; WC="$BC/wt"
    start_branch "$WC" "fix/3038/c-$_class" "crates/mika-agent/src/feature.rs"
    squash_merge_branch "$WC" "fix/3038/c-$_class"
    start_dispatch "$WC" "fix/3038/c-$_class"
    pose_class_no_shipping_tail "$WC"
    case "$_class" in
        dirty-worktree)      RESCUED_DIRTY_WORKTREE=1 ;;
        no-shipping-tail)    : ;;
        commit-pushed-no-pr) PILOT_SHIPPING_TAIL="present" ;;
    esac
    run_gate
    assert_not_contains "V5 class $_class: no draft opened" "pr create" "$(gh_argv)"
    assert_contains "V5 class $_class: the reason is named" \
        "NO_PR: rescue_adds_nothing_to_main" "$RESULT"
done

echo ""
echo "===================================================================="
echo "AC2 — rescue-diff measures what the branch ADDS to origin/main"
echo "===================================================================="

# ============================================================================
# V6 (AC2): the squash. Before this ticket the three-dot form showed the whole
#           already-merged diff, so the marker read `carries-work` and the body
#           carried `Closes #N` — an instruction GitHub executes on merge, on a
#           branch that adds nothing.
# ============================================================================
echo ""
echo "V6: squashed branch → carries no work, incident-only, non-closing ref"
echo "---------------------------------------------------------------------"
assert_eq "V6a the three-dot diff still shows the merged work (the illusion)" "1" \
    "$( [ -n "$(git -C "$W" diff --name-only 'origin/main...HEAD')" ] && echo 1 || echo 0 )"
assert_eq "V6b ...but the branch adds nothing to origin/main" "none" "$(carries_verdict "$W")"
V6_BODY="$(_compose_rescue_pr_body "$W" "no-shipping-tail" "class fact" "3038" "no" "" "" "not-applicable")"
assert_contains "V6c the body marker reads incident-only" "<!-- rescue-diff: incident-only -->" "$V6_BODY"
assert_contains "V6d and the issue reference is deliberately non-closing" "Refs #3038" "$V6_BODY"
assert_not_contains "V6e no Closes on a branch that adds nothing" "Closes #3038" "$V6_BODY"

# ============================================================================
# V7 (AC2): the witness. Merged content PLUS one file the merge did not carry:
#           the marker must read `carries-work` and the body must close.
# ============================================================================
echo ""
echo "V7: partial squash (one new file survives) → carries-work, Closes"
echo "-----------------------------------------------------------------"
assert_eq "V7a the branch adds work" "work" "$(carries_verdict "$W2")"
V7_BODY="$(_compose_rescue_pr_body "$W2" "no-shipping-tail" "class fact" "3016" "no" "" "" "not-applicable")"
assert_contains "V7b the body marker reads carries-work" "<!-- rescue-diff: carries-work -->" "$V7_BODY"
assert_contains "V7c and the issue reference closes" "Closes #3016" "$V7_BODY"

# ============================================================================
# V8 (AC2): the branch is BEHIND main and adds nothing. This is the case a bare
#           two-dot form would have failed OPEN on: `diff origin/main HEAD`
#           reports main's own newer files as a difference. Only the
#           intersection of the two forms answers the question asked.
# ============================================================================
echo ""
echo "V8: branch behind main, adding nothing → carries no work"
echo "--------------------------------------------------------"
B3="$(make_pair)"; W3="$B3/wt"
git -C "$W3" checkout -q -b "fix/3038/behind"
git -C "$W3" push -q origin "fix/3038/behind"
git -C "$W3" checkout -q main
printf 'pub fn landed_on_main_later() {}\n' > "$W3/crates/mika-agent/src/newer.rs"
git -C "$W3" add -A
git -C "$W3" commit -q -m "feat: main moved on" --no-verify
git -C "$W3" push -q origin main
git -C "$W3" checkout -q "fix/3038/behind"
git -C "$W3" fetch -q origin main
assert_eq "V8a the branch is strictly behind main" "0" \
    "$(git -C "$W3" rev-list 'origin/main..HEAD' --count)"
assert_eq "V8b the bare two-dot form would have seen a difference" "1" \
    "$( [ -n "$(git -C "$W3" diff --name-only origin/main HEAD)" ] && echo 1 || echo 0 )"
assert_eq "V8c the branch adds nothing to origin/main" "none" "$(carries_verdict "$W3")"

echo ""
echo "===================================================================="
echo "AC3 — a branch the merge deleted is not re-created for nothing"
echo "===================================================================="

# ============================================================================
# V9 (AC3): `origin` no longer carries the branch (its PR was merged, which
#           deleted it) and the tree adds nothing. Re-pushing would publish
#           nothing — and would hand the rescue a branch to open a draft on.
#           Measured on the remote itself, not on an argv.
# ============================================================================
echo ""
echo "V9: remote lost the branch + nothing to add → not re-pushed"
echo "-----------------------------------------------------------"
B4="$(make_pair)"; W4="$B4/wt"
start_branch "$W4" "fix/3038/merged" "crates/mika-agent/src/feature.rs"
squash_merge_branch "$W4" "fix/3038/merged" 1
assert_eq "V9a the fixture: the remote no longer carries the branch" "no" \
    "$(remote_has "$W4" "fix/3038/merged")"
start_dispatch "$W4" "fix/3038/merged"
run_push
assert_eq "V9b the branch was NOT re-created on origin" "no" \
    "$(remote_has "$W4" "fix/3038/merged")"
assert_contains "V9c the callback says so, in its own words" \
    "Push: SKIPPED — the remote has no branch fix/3038/merged" "$RESULT"
assert_contains "V9d ...and names its ticket" "mika#3038" "$RESULT"

# ============================================================================
# V10 (AC3): THE WITNESS. Same topology — the remote lost the branch — but the
#            branch carries work the merge did not take. It must be pushed.
# ============================================================================
echo ""
echo "V10: remote lost the branch BUT work remains → pushed"
echo "-----------------------------------------------------"
B5="$(make_pair)"; W5="$B5/wt"
start_branch "$W5" "fix/3038/partial" "crates/mika-agent/src/feature.rs"
squash_merge_branch "$W5" "fix/3038/partial" 1
printf 'pub fn still_unmerged() {}\n' > "$W5/crates/mika-agent/src/later.rs"
git -C "$W5" add -A
git -C "$W5" commit -q -m "feat: work the merge did not carry" --no-verify
assert_eq "V10a the fixture: the remote no longer carries the branch" "no" \
    "$(remote_has "$W5" "fix/3038/partial")"
start_dispatch "$W5" "fix/3038/partial"
run_push
assert_eq "V10b the branch IS published" "yes" "$(remote_has "$W5" "fix/3038/partial")"
assert_not_contains "V10c and no skip is claimed" "Push: SKIPPED — the remote has no branch" "$RESULT"

echo ""
echo "===================================================================="
echo "AC4 — an unmeasurable comparison restores the pre-ticket behaviour"
echo "===================================================================="

# ============================================================================
# V11 (AC4): no `origin/main` at all. The predicate answers "undetermined", the
#            push happens, the draft opens — and the callback says it in exactly
#            ONE line, although BOTH consultation sites ran in this dispatch.
# ============================================================================
echo ""
echo "V11: no origin/main → pre-ticket behaviour, and exactly one Tree-probe line"
echo "--------------------------------------------------------------------------"
B6="$(mktemp -d "$TMPROOT/noref.XXXXXX")"
git init -q --bare "$B6/origin.git"
git init -q "$B6/wt"; W6="$B6/wt"
_gitcfg "$W6"
mkdir -p "$W6/crates/mika-agent/src"
printf 'pub fn seed() {}\n' > "$W6/crates/mika-agent/src/lib.rs"
git -C "$W6" add -A
git -C "$W6" commit -q -m seed --no-verify
git -C "$W6" checkout -q -b "fix/3038/noref"
printf 'pub fn work() {}\n' > "$W6/crates/mika-agent/src/feature.rs"
git -C "$W6" add -A
git -C "$W6" commit -q -m "feat: work" --no-verify
git -C "$W6" remote add origin "$B6/origin.git"
assert_eq "V11a the predicate answers undetermined, never equal" "undetermined" "$(tree_verdict "$W6")"
start_dispatch "$W6" "fix/3038/noref"
pose_class_no_shipping_tail "$W6"
run_push
run_gate
assert_eq "V11b the push still happens (pre-ticket behaviour)" "yes" \
    "$(remote_has "$W6" "fix/3038/noref")"
assert_contains "V11c the draft still opens" "pr create" "$(gh_argv)"
assert_eq "V11d the callback carries EXACTLY one Tree-probe line" "1" \
    "$(printf '%s\n' "$RESULT" | grep -c '^Tree-probe: ' || true)"
assert_contains "V11e ...and it names the fallback" "pre-mika#3038 behaviour is in force" "$RESULT"

# ============================================================================
# V12 (AC4): the two unmeasurable inputs that must never read as "equal".
#
#   The empty-dir case is not hypothetical arithmetic: `git -C ""` silently
#   operates on the dispatch process CWD — a live checkout. Measured from INSIDE
#   a repo whose head tree does equal its `origin/main`, so a predicate that
#   fell through to the CWD would answer "equal" and withhold a legitimate
#   draft. The assertion is that it answers "undetermined" anyway.
# ============================================================================
echo ""
echo "V12: empty dir and non-repo answer undetermined, never equal"
echo "------------------------------------------------------------"
B7="$(make_pair)"; W7="$B7/wt"
assert_eq "V12a the CWD trap is armed: this repo's head tree DOES equal origin/main" \
    "equal" "$(tree_verdict "$W7")"
assert_eq "V12b empty worktree dir → undetermined (the CWD is not measured)" \
    "undetermined" "$(cd "$W7" && tree_verdict "")"
NOT_A_REPO="$(mktemp -d "$TMPROOT/norepo.XXXXXX")"
assert_eq "V12c not a git repository → undetermined" "undetermined" "$(tree_verdict "$NOT_A_REPO")"

echo ""
echo "===================================================================="
echo "S1 — the wiring scan: the predicate is CONSULTED where it must be"
echo "===================================================================="

# The scan asserts COUNTS, not presences. A presence assertion survives the
# regression that matters here — a second consultation site added without its
# refusal, or a consultation deleted while its definition stays — and this ticket
# is precisely about a detector that reads healthy while being inert.
echo ""
echo "S1a: occurrence counts"
echo "----------------------"
# The WIRING is counted apart from the prose, deliberately. A single total over
# both would go red when someone adds a cross-reference in a comment — a change
# with no behavioural content — and the lesson a developer takes from that red is
# "bump the number", which is how a count stops being read. So: exactly two CALL
# sites, exactly one definition, and the remaining mentions asserted as prose.
assert_eq "the predicate is CONSULTED at exactly two sites" "2" \
    "$(grep -c '^    _branch_tree_equals_main "\$WORKTREE_DIR" || _tree_rc=\$?$' "$DISPATCH_LIB")"
assert_eq "the predicate is defined exactly once" "1" \
    "$(grep -c '^_branch_tree_equals_main() {' "$DISPATCH_LIB")"
# Five mentions total: the definition, its own header, the two consults, and the
# cross-reference from `_rescue_diff_carries_work`'s doc — which is load-bearing
# prose, not decoration: it is what tells the next editor that the tree predicate
# does not subsume the diff classifier, the question each answers being a
# different one (V7's case). A sixth mention means a third site; look at it.
assert_eq "the predicate is named exactly 5 times in all" "5" \
    "$(grep -c '_branch_tree_equals_main' "$DISPATCH_LIB")"
assert_eq "the AC4 line appears exactly 3 times (definition + 2 calls)" "3" \
    "$(grep -c '_note_tree_compare_undetermined' "$DISPATCH_LIB")"
assert_eq "the AC4 line is defined exactly once" "1" \
    "$(grep -c '^_note_tree_compare_undetermined() {' "$DISPATCH_LIB")"
assert_eq "the NO_PR reason is emitted at exactly one site" "1" \
    "$(grep -c '_set_pr_status_line "NO_PR: rescue_adds_nothing_to_main"' "$DISPATCH_LIB")"

echo ""
echo "S1b: the two consultation sites are the right two"
echo "-------------------------------------------------"
PUSH_BLOCK="$(sed -n '/^_push_branch() {/,/^}$/p' "$DISPATCH_LIB")"
assert_contains "the push consults the predicate" "_branch_tree_equals_main" "$PUSH_BLOCK"
# AC3 keys on the MEASURED absence, never on the decision that falls back to the
# local tracking ref: `remote_has_branch` answers "treat the remote as carrying
# the branch?" and would claim a measurement on an `ls-remote` that never
# answered — the mika#2304 defect mika#2626 exists to close.
assert_contains "the push skip keys on the MEASURED absence" \
    'remote_known_absent" -eq 1 ] && [ "$_tree_rc" -eq 0 ]' "$PUSH_BLOCK"
assert_contains "the push skip names the ticket in the callback" \
    "re-creating the branch would publish nothing (mika#3038)" "$PUSH_BLOCK"

echo ""
echo "S1c: the mika#3038 refusal comes BEFORE the mika#2503 one"
echo "---------------------------------------------------------"
# Order is load-bearing, not cosmetic. The two refusals are mutually exclusive
# through the PR-due flag the first one extinguishes; the inverse order would
# report `rescue_scratch_only` on this ticket's whole population — a false
# diagnosis sending the operator to look for a probe crate that is not there.
# The line numbers are read with `grep -c` FIRST and asserted non-vacuous,
# because `grep -n … | cut` yields an EMPTY string when it matches nothing, and
# `${L:-0}` then turns that absence into the number 0 — which compares as
# "first" and would make the ordering assertion pass against a dispatch-lib
# carrying no refusal at all. Measured here on the red run: both halves of this
# block passed before the refusal existed.
assert_eq "the mika#3038 refusal is present exactly once" "1" \
    "$(grep -c '_set_pr_status_line "NO_PR: rescue_adds_nothing_to_main"' "$DISPATCH_LIB")"
assert_eq "the mika#2503 refusal is present exactly once" "1" \
    "$(grep -c '_set_pr_status_line "NO_PR: rescue_scratch_only"' "$DISPATCH_LIB")"
L3038="$(grep -n '_set_pr_status_line "NO_PR: rescue_adds_nothing_to_main"' "$DISPATCH_LIB" | head -1 | cut -d: -f1)"
L2503="$(grep -n '_set_pr_status_line "NO_PR: rescue_scratch_only"' "$DISPATCH_LIB" | head -1 | cut -d: -f1)"
assert_eq "both line numbers were really read (never an empty string)" "1" \
    "$( [ -n "$L3038" ] && [ -n "$L2503" ] && echo 1 || echo 0 )"
assert_eq "the mika#3038 refusal is first" "1" \
    "$( [ -n "$L3038" ] && [ -n "$L2503" ] && [ "$L3038" -lt "$L2503" ] && echo 1 || echo 0 )"

echo ""
echo "S1d: rescue-diff reads BOTH forms, exactly once each"
echo "----------------------------------------------------"
DIFF_BLOCK="$(sed -n '/^_rescue_diff_carries_work() {/,/^}$/p' "$DISPATCH_LIB")"
assert_eq "exactly one two-dot read (differs from main's tip)" "1" \
    "$(printf '%s\n' "$DIFF_BLOCK" | grep -c 'origin/main HEAD' || true)"
assert_eq "exactly one three-dot read (what the branch introduces)" "1" \
    "$(printf '%s\n' "$DIFF_BLOCK" | grep -c 'origin/main\.\.\.HEAD' || true)"
assert_contains "the two sets are intersected, not substituted" 'grep -qxF -- "$f" <<<"$_vs_main"' "$DIFF_BLOCK"

echo ""
echo "S1e: the fetch is the narrow one"
echo "--------------------------------"
# `fetch --prune origin` NU mutates the common-dir tracking refs shared by every
# worktree of the checkout, so it would prune those of in-flight dispatches. The
# reasoning is written on `_remote_branch_exists`, which rejected it first.
FETCH_BLOCK="$(sed -n '/^_fetch_main_ref_once() {/,/^}$/p' "$DISPATCH_LIB")"
assert_contains "the fetch names its ref" "fetch --quiet origin main" "$FETCH_BLOCK"
assert_not_contains "and never prunes the shared common dir" "--prune" "$FETCH_BLOCK"

echo ""
echo "S1f: the positive control — the three functions really exist"
echo "------------------------------------------------------------"
# Without this, every refusal assertion above could pass against an unpatched
# dispatch-lib: an absent function makes bash exit 127 inside a `$(...)`, the
# renderer's `declare -F` guard answers MISSING-FUNCTION, and only an explicit
# assertion turns that into a failure rather than a word nobody compares.
for _fn in _branch_tree_equals_main _fetch_main_ref_once _note_tree_compare_undetermined; do
    assert_eq "S1f $_fn is defined and sourceable" "1" \
        "$(declare -F "$_fn" >/dev/null && echo 1 || echo 0)"
done
assert_not_contains "S1f no renderer fell back to its missing-function guard" \
    "MISSING-FUNCTION" "$(tree_verdict "$W7")$(carries_verdict "$W7")"

echo ""
echo "S1g: the exception table stays empty"
echo "------------------------------------"
# Fire-Disposition (a), named exception with an EMPTY table. The gesture that
# would make an exception pointless is the gesture that makes it expire: any
# entry must name its datum and its follow-up ticket, and the assertion below is
# what forces that conversation instead of letting the table grow in silence.
T3038_WIRING_ALLOWLIST=()
if [ "${#T3038_WIRING_ALLOWLIST[@]}" -ne 0 ]; then
    echo "S1: table d'exceptions du scan de câblage — doit rester vide ; toute entrée nomme sa donnée et son ticket de suivi (mika#3038)" >&2
    exit 1
fi
assert_eq "S1g the wiring-scan exception table is empty" "0" "${#T3038_WIRING_ALLOWLIST[@]}"

echo ""
echo "===================================================================="
echo "Passed: $PASS   Failed: $FAIL"
echo "===================================================================="
[ "$FAIL" -eq 0 ] || exit 1

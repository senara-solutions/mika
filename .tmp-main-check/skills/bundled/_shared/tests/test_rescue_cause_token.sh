#!/bin/bash
# Test suite for the rescue commit's cause token (mika#2539).
#
# WHY: the mika#1282 / mika#2031 rescue committed salvaged work under ONE generic
# subject, so git history could not tell apart two sessions whose causes were
# opposite. Measured 2026-09-25/26: #2532 was stopped by a terminal classifier
# deny at turn 62 (then went silent), #2536 hit the 150-turn ceiling. Both read
# "staged by post-flight recovery", and the organ to go and repair is different in
# each case. Since the 150-turn ceiling stays and iterate-from-wip is the nominal
# R-class route (Prime + Vincent, 2026-09-26), the cause is what history must
# carry.
#
# WHAT THIS SUITE IS FOR. Three halves, and the third is the one that is easy to
# skip:
#   V1-V12  the token is right for each cause, and agrees with the callback
#   R       the rescue's BEHAVIOUR is unchanged (a message-only ticket)
#   N1-N5   each guard is shown to fail when its own mechanism is removed
#
# N1-N5 are not decoration. Without them, "the suite passes" and "the suite looks
# at nothing" are indistinguishable (class mika#2205). They are run here as
# automated mutation probes rather than as a one-off manual observation: each
# mutates a COPY of dispatch-lib, replays the vector that should break, and
# asserts the observed subject differs from the healthy one. A mutation that
# fails to apply is caught by its own assertion, so a probe cannot go vacuous.
#
# It calls the REAL `_rescue_dirty_worktree` against a real temp git repo. No
# network, no cargo, no claude-pilot.
#
# Run: bash skills/bundled/_shared/tests/test_rescue_cause_token.sh
# Expected: all assertions pass, exit 0.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"

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

# Scratch root for fixture stderr files. PILOT_LOG_DIR is redirected here so no
# probe of this suite ever reads /var/log/claude-pilot.
FIXTURES="$(mktemp -d "${TMPDIR:-/tmp}/mika-2539-fx.XXXXXX")"
PILOT_LOG_DIR="$FIXTURES/logs"
mkdir -p "$PILOT_LOG_DIR"
trap 'rm -rf "$FIXTURES"' EXIT

# A worktree-shaped temp repo with one commit, so HEAD resolves.
make_repo() {
    local repo
    repo="$(mktemp -d "$FIXTURES/repo.XXXXXX")"
    git -C "$repo" init -q
    git -C "$repo" config user.email test@example.com
    git -C "$repo" config user.name "Test"
    git -C "$repo" config commit.gpgsign false
    printf 'seed\n' > "$repo/README.md"
    git -C "$repo" add -A
    git -C "$repo" commit -q -m "seed" --no-verify
    printf '%s' "$repo"
}

# Set every global the rescue and the classifier read, for one session shape.
#
# The memo reset is LOAD-BEARING here: in production it happens at the `SUBTYPE`
# assignment site, and a suite that forgot it would carry the first vector's
# answer into every later one and pass by accident.
#
# $1 repo  $2 skill  $3 subtype  $4 policy-deny excerpt  $5 lethality
#                    $6 guardrail line to plant in stderr ("" = none)
set_session() {
    WORKTREE_DIR="$1"
    SKILL="$2"
    SUBTYPE="$3"
    POLICY_DENY="$4"
    POLICY_DENY_LETHALITY="$5"
    unset _HALT_SUBTYPE_RESOLVED _HALT_GUARDRAIL_LINE

    REPO="mika"
    ISSUE_NUM="2539"
    BRANCH="fix/2539/rescue-commit-message-cause-token"
    SESSION_ID="sess-2539"
    LOG_ID="log-2539"
    PILOT_EXIT=1
    STATUS="terminated"
    TURNS="151"
    DURATION="42"
    COST="1.00"
    API_ERROR_STATUS=""
    TERMINATION_REASON=""
    RESULT="claude-pilot completed (status: terminated)."
    RESCUED_DIRTY_WORKTREE=""
    PRE_RUN_HEAD=$(git -C "$1" rev-parse HEAD)
    POST_RUN_HEAD="$PRE_RUN_HEAD"

    STDERR_FILE="$FIXTURES/stderr-$$.txt"
    PERSISTENT_STDERR="$FIXTURES/persistent-$$.txt"
    : > "$STDERR_FILE"
    : > "$PERSISTENT_STDERR"
    if [ -n "${6:-}" ]; then
        printf '%s\n' "$6" > "$STDERR_FILE"
    fi
}

# One dirty non-.rs file, so the proactive `cargo fmt` stays out of the way and
# this suite needs no Rust toolchain.
dirty_it() { printf 'impl\n' > "$1/NOTES.md"; }

# Run one vector end to end and echo the rescue commit's subject.
subject_for() {
    local repo skill
    repo="$(make_repo)"; dirty_it "$repo"
    skill="${1:-dev-pilot}"; shift || true
    set_session "$repo" "$skill" "${1:-}" "${2:-}" "${3:-undeclared}" "${4:-}"
    _rescue_dirty_worktree >/dev/null 2>&1 || true
    git -C "$repo" log -1 --format=%s
}

# ============================================================================
# V1: the turn ceiling — replays #2536
# ============================================================================
echo ""
echo "V1: error_max_turns → rescue budget_exhausted (replays #2536)"
echo "-------------------------------------------------------------"
V1_SUBJECT=$(subject_for dev-pilot "error_max_turns" "" "undeclared" "")
assert_contains "V1: subject carries the budget_exhausted cause" \
    "rescue budget_exhausted" "$V1_SUBJECT"
assert_contains "V1: ticket link is still there (AC1)" "wip(mika#2539):" "$V1_SUBJECT"
assert_contains "V1: the existing tail is word-for-word intact" \
    "impl staged by post-flight recovery (mika#1282)" "$V1_SUBJECT"

# ============================================================================
# V2: a terminal deny outranks the halt that followed it — replays #2532
# ============================================================================
echo ""
echo "V2: terminal deny + idle_timeout → rescue policy_deny (replays #2532)"
echo "---------------------------------------------------------------------"
V2_SUBJECT=$(subject_for dev-pilot "idle_timeout" \
    "[policy:deny] Bash: grep -r foo / [no-rule] (terminal)" "terminal" "")
assert_contains "V2: the deny is reported as the cause, not the silence it caused" \
    "rescue policy_deny" "$V2_SUBJECT"
assert_not_contains "V2: the consequential halt family is NOT what is reported" \
    "session_silent" "$V2_SUBJECT"

# ============================================================================
# V3-V4: the ticket's single "post-stall" refined into two diagnoses
# ============================================================================
echo ""
echo "V3/V4: stall is two diagnoses, not one"
echo "--------------------------------------"
V3_SUBJECT=$(subject_for dev-pilot "idle_timeout" "" "undeclared" "")
assert_contains "V3: real silence reads session_silent" "rescue session_silent" "$V3_SUBJECT"
V4_SUBJECT=$(subject_for dev-pilot "stall_detected" "" "undeclared" "")
assert_contains "V4: an unproductive model reads model_unproductive" \
    "rescue model_unproductive" "$V4_SUBJECT"
assert_eq "V3/V4: the two stall shapes do NOT collapse to one token" "1" \
    "$( [ "$V3_SUBJECT" != "$V4_SUBJECT" ] && echo 1 || echo 0 )"

# ============================================================================
# V5: no halt at all — the session finished and left dirt behind
# ============================================================================
echo ""
echo "V5: no signal whatsoever → rescue no_halt_signal"
echo "------------------------------------------------"
V5_SUBJECT=$(subject_for dev-pilot "" "" "undeclared" "")
assert_contains "V5: a session that did not halt says so" "rescue no_halt_signal" "$V5_SUBJECT"

# ============================================================================
# V6: an upstream subtype outside the downstream table
# ============================================================================
echo ""
echo "V6: unmapped subtype → rescue halt_unmapped (never 'not identified')"
echo "--------------------------------------------------------------------"
V6_SUBJECT=$(subject_for dev-pilot "some_future_subtype" "" "undeclared" "")
assert_contains "V6: an unmapped halt is named as unmapped" "rescue halt_unmapped" "$V6_SUBJECT"
assert_eq "V6: 'halted for an unknown reason' ≠ 'never halted' (R3)" "1" \
    "$( [ "$V6_SUBJECT" != "$V5_SUBJECT" ] && echo 1 || echo 0 )"

# ============================================================================
# V7-V8: the lethality condition is a negative control, not decoration
# ============================================================================
echo ""
echo "V7/V8: a deny the session SURVIVED is not the cause"
echo "--------------------------------------------------"
V7_SUBJECT=$(subject_for dev-pilot "error_max_turns" \
    "[policy:deny] Bash: ls /etc [no-rule] (non-terminal)" "non-terminal" "")
assert_contains "V7: non-terminal deny + ceiling → the ceiling is the cause" \
    "rescue budget_exhausted" "$V7_SUBJECT"
assert_not_contains "V7: a survived deny is not reported as the cause" \
    "policy_deny" "$V7_SUBJECT"
V8_SUBJECT=$(subject_for dev-pilot "error_max_turns" \
    "[policy:deny] Bash: ls /etc [no-rule]" "undeclared" "")
assert_contains "V8: undeclared lethality asserts nothing either" \
    "rescue budget_exhausted" "$V8_SUBJECT"
assert_not_contains "V8: undeclared deny is not reported as the cause" \
    "policy_deny" "$V8_SUBJECT"

# ============================================================================
# V9: the [guardrail] fallback — the divergence this design exists to prevent
# ============================================================================
echo ""
echo "V9: no JSON subtype, [guardrail] line in stderr → resolved anyway"
echo "-----------------------------------------------------------------"
V9_SUBJECT=$(subject_for dev-pilot "" "" "undeclared" \
    "[guardrail] idle_timeout: no output for 900s")
assert_contains "V9: the scraped subtype reaches the token" "rescue session_silent" "$V9_SUBJECT"
assert_not_contains "V9: it is NOT misread as 'never halted'" "no_halt_signal" "$V9_SUBJECT"

# ============================================================================
# V10: dev-groom carries its own tail, plus the same token
# ============================================================================
echo ""
echo "V10: dev-groom → token present AND the mika#2031 tail intact"
echo "------------------------------------------------------------"
V10_REPO="$(make_repo)"
mkdir -p "$V10_REPO/docs/plans"
printf '# plan\n' > "$V10_REPO/docs/plans/2026-09-26-001-fix-2539-x-plan.md"
set_session "$V10_REPO" "dev-groom" "error_max_turns" "" "undeclared" ""
_rescue_dirty_worktree >/dev/null 2>&1 || true
V10_SUBJECT=$(git -C "$V10_REPO" log -1 --format=%s)
assert_contains "V10: dev-groom subject carries the cause" "rescue budget_exhausted" "$V10_SUBJECT"
assert_contains "V10: dev-groom tail is word-for-word intact" \
    "plan staged by post-flight recovery (mika#2031)" "$V10_SUBJECT"

# ============================================================================
# V11 (static): the cargo-fmt retry path carries the SAME token
#
# Behaviourally unreachable: both commit sites pass --no-verify (mika#1685), so
# no pre-commit hook can reject the first one and route to the retry. Attested
# statically instead, and the property is stronger than a replay would be — both
# sites interpolate ONE variable computed once, so they cannot drift apart.
# ============================================================================
echo ""
echo "V11 (static): both commit sites interpolate the one _rescue_what"
echo "----------------------------------------------------------------"
assert_eq "V11: exactly 3 rescue commit sites, unchanged (mika#1282 x2 + mika#1383)" "3" \
    "$(grep -c 'git -C "\$WORKTREE_DIR" commit -m "wip(' "$DISPATCH_LIB")"
assert_eq "V11: both mika#1282 sites interpolate \${_rescue_what}" "2" \
    "$(grep -c 'commit -m "wip(\${REPO}#\${ISSUE_NUM}): \${_rescue_what}' "$DISPATCH_LIB")"
assert_eq "V11: _rescue_what is assigned the cause token in exactly two branches" "2" \
    "$(grep -c '_rescue_what="rescue \${_rescue_cause} — ' "$DISPATCH_LIB")"
assert_eq "V11: the cause is computed exactly once per rescue" "1" \
    "$(grep -c '_rescue_cause=\$(_rescue_cause_token)' "$DISPATCH_LIB")"

# ============================================================================
# V12 (R-7): the commit and the callback banner say the SAME word
# ============================================================================
echo ""
echo "V12: commit token and 'Halt class:' agree for one session"
echo "---------------------------------------------------------"
for _v12_subtype in error_max_turns idle_timeout stall_detected some_future_subtype; do
    V12_REPO="$(make_repo)"; dirty_it "$V12_REPO"
    set_session "$V12_REPO" "dev-pilot" "$_v12_subtype" "" "undeclared" ""
    _rescue_dirty_worktree >/dev/null 2>&1 || true
    V12_TOKEN=$(git -C "$V12_REPO" log -1 --format=%s | sed -n 's/.*rescue \([a-z_]*\) —.*/\1/p')
    # The banner is composed AFTER the rescue in production; same order here.
    V12_BANNER=$(_classify_terminated_session banner 2>/dev/null)
    V12_FAMILY=$(printf '%s\n' "$V12_BANNER" | sed -n 's/^Halt class: \([a-z_]*\) .*/\1/p')
    if [ "$_v12_subtype" = "some_future_subtype" ]; then
        # The one deliberate divergence: the banner says `unknown` (its own
        # vocabulary, read by the T6 drift line), the commit says
        # `halt_unmapped` — same fact, and the commit's word has to survive in
        # git log without the banner beside it.
        assert_eq "V12 ($_v12_subtype): banner says unknown" "unknown" "$V12_FAMILY"
        assert_eq "V12 ($_v12_subtype): commit says halt_unmapped" "halt_unmapped" "$V12_TOKEN"
    else
        assert_eq "V12 ($_v12_subtype): commit token == banner halt class" \
            "$V12_FAMILY" "$V12_TOKEN"
    fi
done

# ============================================================================
# R (AC4): the rescue's BEHAVIOUR is unchanged — this is a message-only ticket
# ============================================================================
echo ""
echo "R: non-regression of the rescue behaviour itself"
echo "------------------------------------------------"
R_REPO="$(make_repo)"; dirty_it "$R_REPO"
R_BEFORE=$(git -C "$R_REPO" rev-parse HEAD)
set_session "$R_REPO" "dev-pilot" "error_max_turns" "" "undeclared" ""
_rescue_dirty_worktree >/dev/null 2>&1 || true
R_AFTER=$(git -C "$R_REPO" rev-parse HEAD)
assert_eq "R: HEAD advanced (content is in a commit)" "1" \
    "$( [ "$R_BEFORE" != "$R_AFTER" ] && echo 1 || echo 0 )"
assert_eq "R: POST_RUN_HEAD advanced so _push_branch sees the commit" "$R_AFTER" "$POST_RUN_HEAD"
assert_eq "R: RESCUED_DIRTY_WORKTREE set for dev-pilot" "1" "$RESCUED_DIRTY_WORKTREE"
assert_eq "R: worktree is clean afterwards" "" "$(git -C "$R_REPO" status --porcelain)"
assert_contains "R: the rescue note is unchanged (mika#2031 R6)" "Rescued into commit" "$RESULT"
assert_contains "R: the commit body is unchanged" \
    "Auto-rescued by dispatch-lib dirty-worktree detection." \
    "$(git -C "$R_REPO" log -1 --format=%b)"
# `--no-verify` is not re-asserted here: test_rescue_commit_no_verify.sh owns that
# static guard, and a second copy would be a second thing to keep in step.

# dev-groom still opens no draft PR.
R2_REPO="$(make_repo)"
mkdir -p "$R2_REPO/docs/plans"; printf '# p\n' > "$R2_REPO/docs/plans/2026-09-26-001-fix-2539-y-plan.md"
set_session "$R2_REPO" "dev-groom" "error_max_turns" "" "undeclared" ""
_rescue_dirty_worktree >/dev/null 2>&1 || true
assert_eq "R: dev-groom sets no draft-PR marker" "" "$RESCUED_DIRTY_WORKTREE"

# A clean tree still triggers nothing, whatever the cause signals say.
R3_REPO="$(make_repo)"
R3_BEFORE=$(git -C "$R3_REPO" rev-parse HEAD)
R3_RESULT_BEFORE="claude-pilot completed (status: terminated)."
set_session "$R3_REPO" "dev-pilot" "error_max_turns" \
    "[policy:deny] x (terminal)" "terminal" ""
_rescue_dirty_worktree >/dev/null 2>&1 || true
assert_eq "R: clean tree → HEAD did not move" "$R3_BEFORE" "$(git -C "$R3_REPO" rev-parse HEAD)"
assert_eq "R: clean tree → RESULT untouched" "$R3_RESULT_BEFORE" "$RESULT"
assert_eq "R: clean tree → no draft-PR marker" "" "$RESCUED_DIRTY_WORKTREE"

# Scaffold-only dirt is still excluded.
R4_REPO="$(make_repo)"
R4_BEFORE=$(git -C "$R4_REPO" rev-parse HEAD)
mkdir -p "$R4_REPO/.claude/commands"; printf 'x\n' > "$R4_REPO/.claude/commands/mika.md"
set_session "$R4_REPO" "dev-pilot" "error_max_turns" "" "undeclared" ""
_rescue_dirty_worktree >/dev/null 2>&1 || true
assert_eq "R: scaffold-only dirt → HEAD did not move" "$R4_BEFORE" "$(git -C "$R4_REPO" rev-parse HEAD)"

# ============================================================================
# N1-N5: each guard, shown to fail when its own mechanism is removed
# ============================================================================
echo ""
echo "N1-N5: mutation probes — every guard is shown to be load-bearing"
echo "----------------------------------------------------------------"

# Arming and reading are DELIBERATELY two functions, and the split is the whole
# reason these probes mean anything.
#
# `arm_mutant` runs in the current shell so its verdict reaches the counters and
# the operator's screen. A single function returning the subject would have to be
# called in a command substitution, which puts its own `✓`/`✗` line and its
# `FAIL=$((FAIL+1))` inside a subshell: the anti-vacuity assertion would be
# captured into the caller's variable, invisible and without effect, and a sed
# that matched nothing would leave healthy behaviour in place while the probe
# reported a difference it never caused. That is the mika#2205 shape occurring
# inside the very suite that exists to refuse it.
MUTANT_LIB=""
arm_mutant() {
    local label="$1" sed_expr="$2"
    MUTANT_LIB="$FIXTURES/mutant-$(printf '%s' "$label" | tr -c 'A-Za-z0-9' '_').sh"
    sed "$sed_expr" "$DISPATCH_LIB" > "$MUTANT_LIB"
    if cmp -s "$MUTANT_LIB" "$DISPATCH_LIB"; then
        FAIL=$((FAIL + 1))
        echo "  ✗ $label: the mutation matched nothing — this probe would prove nothing"
        MUTANT_LIB=""
    else
        PASS=$((PASS + 1)); echo "  ✓ $label: mutation applied (probe is armed)"
    fi
}

# Replay one vector against the armed mutant and echo the rescue commit subject.
# With no armed mutant it echoes a sentinel that satisfies no assertion below, so
# a disarmed probe reddens twice rather than passing quietly.
mutant_subject() {
    local skill="$1" subtype="$2" deny="$3" lethality="$4" guardrail="$5"
    local repo
    [ -n "$MUTANT_LIB" ] || { printf 'PROBE-DISARMED-MUTATION-DID-NOT-APPLY'; return 0; }
    repo="$(make_repo)"; dirty_it "$repo"
    (
        source "$MUTANT_LIB" >/dev/null 2>&1
        exec 9>/dev/null
        set_session "$repo" "$skill" "$subtype" "$deny" "$lethality" "$guardrail"
        _rescue_dirty_worktree >/dev/null 2>&1 || true
        git -C "$repo" log -1 --format=%s
    )
}

# N1 — the token removed from the subject ⇒ V1 goes red.
arm_mutant "N1" 's/_rescue_what="rescue \${_rescue_cause} — /_rescue_what="/'
N1=$(mutant_subject dev-pilot "error_max_turns" "" "undeclared" "")
assert_not_contains "N1: without the interpolation, V1's assertion fails" \
    "rescue budget_exhausted" "$N1"

# N2 — rank 1 removed, which is what "moved after rank 2" amounts to: rank 2
# then wins and the consequence is reported as the cause ⇒ V2 goes red.
arm_mutant "N2" \
    '/^    if \[ -n "\${POLICY_DENY:-}" \] && \[ "\${POLICY_DENY_LETHALITY:-}" = "terminal" \]; then$/,+3d'
N2=$(mutant_subject dev-pilot "idle_timeout" "[policy:deny] x (terminal)" "terminal" "")
assert_not_contains "N2: without rank 1's precedence, V2's assertion fails" \
    "rescue policy_deny" "$N2"
assert_contains "N2: and the consequence is reported instead (the wrong organ)" \
    "rescue session_silent" "$N2"

# N3 — the lethality condition removed ⇒ V7 goes red.
arm_mutant "N3" \
    's/if \[ -n "\${POLICY_DENY:-}" \] && \[ "\${POLICY_DENY_LETHALITY:-}" = "terminal" \]; then/if [ -n "${POLICY_DENY:-}" ]; then/'
N3=$(mutant_subject dev-pilot "error_max_turns" "[policy:deny] x (non-terminal)" "non-terminal" "")
assert_not_contains "N3: without the lethality test, V7's assertion fails" \
    "rescue budget_exhausted" "$N3"
assert_contains "N3: a survived deny is then wrongly reported as the cause" \
    "rescue policy_deny" "$N3"

# N4 — ranks 3 and 4 merged ⇒ V5 goes red.
arm_mutant "N4" 's/        printf '"'"'%s'"'"' "no_halt_signal"/        printf '"'"'%s'"'"' "halt_unmapped"/'
N4=$(mutant_subject dev-pilot "" "" "undeclared" "")
assert_not_contains "N4: merged ranks 3/4 break V5's assertion" \
    "rescue no_halt_signal" "$N4"

# N5 — the resolver replaced by a raw $SUBTYPE read ⇒ V9 goes red. This is the
# §3.1 divergence made visible: the banner would still say session_silent.
arm_mutant "N5" 's/^    _subtype="\${_HALT_SUBTYPE_RESOLVED:-}"$/    _subtype="${SUBTYPE:-}"/'
N5=$(mutant_subject dev-pilot "" "" "undeclared" "[guardrail] idle_timeout: no output for 900s")
assert_not_contains "N5: a raw SUBTYPE read breaks V9's assertion" \
    "rescue session_silent" "$N5"
assert_contains "N5: and the commit contradicts the banner for that session" \
    "rescue no_halt_signal" "$N5"

# ============================================================================
# Summary
# ============================================================================
echo ""
echo "===================================================================="
echo "Passed: $PASS   Failed: $FAIL"
echo "===================================================================="
[ "$FAIL" -eq 0 ] || exit 1

#!/bin/bash
# Le callback dev-groom honnête (mika#1772) — extrait de test-dispatch-lib.sh par mika#3178, vol 3.
# Parseurs du callback, enveloppe de relance, trail et `_escalate_groom` ; le
# sixième argument de mika#2149 (`.api_error_status`, cpp#54) ; le pilote tué
# compté par son nom et non par un rc anonyme (mika#2029) ; une ligne par
# callback livré (mika#2121) ; le corps à scanner devenu argument (mika#3118).
#
# Plage d'origine : lignes 4990–5987 de skills/bundled/_shared/test-dispatch-lib.sh
# à la base 8de06d66. Corps et queue extraits par `sed -n 'N,Mp'` (mika#3178
# D1(a), permission claude-pilot#325) : mot pour mot par construction.
set -euo pipefail
_SUITE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib-dispatch-lib-suite.sh
source "$_SUITE_DIR/lib-dispatch-lib-suite.sh"
# ===========================================================================
# mika#1772 — an honest dev-groom callback
#
# Founding incident: tasks f4fff3ff-4e57-4005-b4b6-bda13d68872d (2026-08-28
# 18:08Z) and 74504478-184d-4af4-8d1f-cadb1b1fdce9 (19:08Z), both dispatched
# on mika#2013. claude-pilot returned `status: terminated` at Turns 2 after
# `[guardrail] idle_timeout: No meaningful progress for 300s`, having made
# zero write tool calls. dispatch-lib ran the whole content-validation chain
# on that empty session and emitted a callback with THREE false statements
# ahead of the one true one:
#   1. "Plan exists on branch but architect verdict is missing" — no plan
#      existed and the architect was never reached.
#   2. "no /ce:plan invocation detected in session log" — the session log
#      file did not exist, so nothing was detected either way.
#   3. "plan already committed from prior run" — the guard matched ANY
#      *-plan.md in docs/plans/, of which main carries 769.
# These assertions lock each one shut.
# ===========================================================================

echo ""
echo "Test: dev-groom callback honesty (mika#1772)"
echo "---------------------------------------------"

ITERATE_SRC_1772=$(
    # shellcheck disable=SC1090
    source "$DISPATCH_LIB" 2>/dev/null || true
    declare -f _iterate_groom_loop
)
DISPATCH_SRC_1772=$(
    # shellcheck disable=SC1090
    source "$DISPATCH_LIB" 2>/dev/null || true
    declare -f dispatch_claude_pilot
)
RCP_SRC_1772=$(
    # shellcheck disable=SC1090
    source "$DISPATCH_LIB" 2>/dev/null || true
    declare -f _run_claude_pilot
)
PFR_SRC_1772=$(
    # shellcheck disable=SC1090
    source "$DISPATCH_LIB" 2>/dev/null || true
    declare -f _post_flight_recovery
)

# --- U1: the loop names its own failure reason -----------------------------

assert_contains "U1: _iterate_groom_loop initializes GROOM_LOOP_FAILURE_REASON" \
    'GROOM_LOOP_FAILURE_REASON=' "$ITERATE_SRC_1772"

# KTD1: the reason must reach the caller, so the variable is global. A `local`
# declaration would silently restore the "no reason recorded" fallback on
# every failure.
assert_not_contains "U1: GROOM_LOOP_FAILURE_REASON is not declared local" \
    'local GROOM_LOOP_FAILURE_REASON' "$ITERATE_SRC_1772"

# The invariant that every `return 1` carries a reason is asserted per site,
# further down. A totals comparison was tried first and proved complaisant:
# with one reason setter deleted it still reported 21 >= 18 and passed.

# The three _escalate_groom exits are the cases where the architect genuinely
# refused the plan — the class R2 must keep distinguishable from a guard trip.
assert_contains "U1: first-pass ESCALATE exit records an architect-refusal reason" \
    'architect ESCALATE (first-pass)' "$ITERATE_SRC_1772"

# --- U1: the caller stops inventing a diagnosis ----------------------------

assert_not_contains "U1: dispatch_claude_pilot no longer asserts a plan on the branch" \
    'Plan exists on branch' "$DISPATCH_SRC_1772"
assert_not_contains "U1: dispatch_claude_pilot no longer hardcodes the convergence phrase" \
    'architect convergence did not complete' "$DISPATCH_SRC_1772"
assert_contains "U1: dispatch_claude_pilot emits the recorded reason" \
    'GROOM_LOOP_FAILURE_REASON' "$DISPATCH_SRC_1772"
assert_contains "U1: dispatch_claude_pilot has a fallback when no reason was recorded" \
    'no reason recorded' "$DISPATCH_SRC_1772"

# R2 wants the plan-on-branch claim MEASURED, not deleted: on the 2026-07-04
# class (mika#1723) a plan really was on the branch and that fact is the most
# useful line in the message. _committed_plan_on_branch is the file's existing
# authority for the question.
assert_contains "U1: plan-on-branch is measured via _committed_plan_on_branch" \
    '_committed_plan_on_branch' "$DISPATCH_SRC_1772"

# Behavioral: a guard trip records a reason naming the guard, not convergence.
_iterate_reason_probe() {
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        WORKTREE_DIR="" ISSUE_NUM="1267" REPO="mika"
        _iterate_groom_loop >/dev/null 2>&1
        printf '%s' "$GROOM_LOOP_FAILURE_REASON"
    )
}
ITERATE_REASON_1772=$(_iterate_reason_probe) || ITERATE_REASON_1772=""
assert_contains "U1: WORKTREE_DIR guard trip records a guard reason" \
    "WORKTREE_DIR" "$ITERATE_REASON_1772"
assert_not_contains "U1: guard trip does not blame architect convergence" \
    "architect convergence" "$ITERATE_REASON_1772"

# Behavioral: no issue-scoped plan names the plan lookup, not the architect.
_iterate_noplan_probe() {
    local tmp
    tmp=$(mktemp -d)
    mkdir -p "$tmp/docs/plans"
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        WORKTREE_DIR="$tmp" ISSUE_NUM="1267" REPO="mika"
        _iterate_groom_loop >/dev/null 2>&1
        printf '%s' "$GROOM_LOOP_FAILURE_REASON"
    )
    rm -rf "$tmp"
}
ITERATE_NOPLAN_1772=$(_iterate_noplan_probe) || ITERATE_NOPLAN_1772=""
assert_contains "U1: missing issue plan names the plan lookup" \
    "plan" "$ITERATE_NOPLAN_1772"
assert_not_contains "U1: missing issue plan does not blame architect convergence" \
    "architect convergence" "$ITERATE_NOPLAN_1772"

# --- U2: a terminated session is classified before content validation ------

# KTD5: dispatch_claude_pilot and _run_claude_pilot need a real pilot and CLI,
# so the classification lives in its own callable function the harness can run
# with an injected environment — the same shape as _find_issue_plan's probes.
# mika#2149: a sixth argument carries `.api_error_status` (cpp#54), and a
# seventh selects which stream the probe returns — `stdout` (default) or
# `stderr`, because the halt_family.unknown line (R-4) lives on stderr and the
# 2>/dev/null here used to make that stream unobservable.
_classify_probe() {
    local guardrail_line="$1" turns="${2:-2}" subtype="${3:-}" reason="${4:-}" mode="${5:-full}"
    local api_error_status="${6:-}" stream="${7:-stdout}"
    local tmp log_id
    tmp=$(mktemp -d)
    log_id="probe-1772"
    if [ -n "$guardrail_line" ]; then
        printf '%s\n' "$guardrail_line" > "$tmp/${log_id}.stderr"
    fi
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        STATUS="terminated"
        TURNS="$turns"
        DURATION="602410"
        SESSION_ID="0dab1700-5ca9-4145-87b0-6f618a047220"
        LOG_ID="$log_id"
        PILOT_LOG_DIR="$tmp"
        STDERR_FILE=""
        SUBTYPE="$subtype"
        TERMINATION_REASON="$reason"
        API_ERROR_STATUS="$api_error_status"
        if [ "$stream" = "stderr" ]; then
            _classify_terminated_session "$mode" 2>&1 >/dev/null
        else
            _classify_terminated_session "$mode" 2>/dev/null
        fi
    )
    rm -rf "$tmp"
}

CLASSIFY_WITH_GUARDRAIL=$(_classify_probe '[guardrail] idle_timeout: No meaningful progress for 300s') || CLASSIFY_WITH_GUARDRAIL=""
assert_contains "U2: terminated classification names the termination" \
    "terminated" "$CLASSIFY_WITH_GUARDRAIL"
assert_contains "U2: terminated classification names the guardrail" \
    "idle_timeout" "$CLASSIFY_WITH_GUARDRAIL"
assert_contains "U2: terminated classification names the turn count" \
    "Turns: 2" "$CLASSIFY_WITH_GUARDRAIL"
assert_not_contains "U2: terminated classification carries no plan-lookup diagnosis" \
    "_find_issue_plan" "$CLASSIFY_WITH_GUARDRAIL"
assert_not_contains "U2: terminated classification carries no convergence diagnosis" \
    "architect convergence" "$CLASSIFY_WITH_GUARDRAIL"
# mika#2029: this assertion TURNED, and that is the probe doing its job. Until
# 2026-10-15 it required `mika#1901` — the auth lineage the delivered text named
# as the cause. That lineage was refuted by measurement on 2026-08-29 (the very
# sessions it described show `POST /v1/messages -> 200`, six candidate lineages
# eliminated), and the cause was eventually traced upstream of this repo to
# claude-pilot#177 (CLOSED 2026-09-11). A callback that names a dead lineage
# costs more than a silent one: it sends the next reader into the wrong half of
# the system. The pair below pins BOTH directions — the traced cause is named,
# and the refuted one does not come back through an absent-minded restore.
assert_contains "U2: terminated classification names the cause that was traced" \
    "claude-pilot#177" "$CLASSIFY_WITH_GUARDRAIL"
assert_not_contains "U2: and no longer points at the refuted auth lineage" \
    "mika#1901" "$CLASSIFY_WITH_GUARDRAIL"

# R7: one Outcome line. The old path could stack two — _post_flight_recovery
# appended one and the iterate-loop else-branch rewrote another.
CLASSIFY_OUTCOME_COUNT=$(printf '%s\n' "$CLASSIFY_WITH_GUARDRAIL" | grep -c '^Outcome:' || true)
assert_eq "U2: terminated classification carries exactly one Outcome line" \
    "1" "$CLASSIFY_OUTCOME_COUNT"
assert_contains "U2: terminated classification is PIPELINE_INCOMPLETE" \
    "Outcome: PIPELINE_INCOMPLETE" "$CLASSIFY_WITH_GUARDRAIL"

# KTD3: stderr only enriches. A missing file degrades the text, never the
# classification — the 2026-08-28 tasks had no .log file at all, and a
# fail-closed read there would have hidden the whole class.
CLASSIFY_NO_STDERR=$(_classify_probe '') || CLASSIFY_NO_STDERR=""
assert_contains "U2: classification still fires when stderr is absent" \
    "terminated" "$CLASSIFY_NO_STDERR"
assert_contains "U2: classification without stderr is still PIPELINE_INCOMPLETE" \
    "Outcome: PIPELINE_INCOMPLETE" "$CLASSIFY_NO_STDERR"

# Structural: the guard has to sit INSIDE _run_claude_pilot, because
# _post_flight_recovery is called from there (dispatch-lib.sh:1280) and not
# from dispatch_claude_pilot. A guard placed after _run_claude_pilot returns
# can only prefix text that is already false.
RCP_FIRST_1772=$(printf '%s\n' "$RCP_SRC_1772" \
    | grep -nE '_classify_terminated_session|_post_flight_recovery' | head -1)
assert_contains "U2: terminated guard precedes _post_flight_recovery in _run_claude_pilot" \
    "_classify_terminated_session" "$RCP_FIRST_1772"

# The mika#1615 invariant stays: the recovery call remains in _run_claude_pilot,
# it only becomes conditional.
assert_contains "U2: _post_flight_recovery is still called from _run_claude_pilot" \
    "_post_flight_recovery" "$RCP_SRC_1772"

# Structural: the terminated short-circuit precedes convergence.
DISPATCH_FIRST_1772=$(printf '%s\n' "$DISPATCH_SRC_1772" \
    | grep -nE 'PILOT_SESSION_TERMINATED|_iterate_groom_loop' | head -1)
assert_contains "U2: PILOT_SESSION_TERMINATED is tested before the iterate loop" \
    "PILOT_SESSION_TERMINATED" "$DISPATCH_FIRST_1772"

# The pilot push guard must NOT be gated on termination: it compares the remote
# ref before and after and already returns 0 when nothing moved, so gating it
# bought a no-op and blinded the one check that catches a terminated pilot which
# had already pushed.
assert_not_contains "U2: the pilot push guard is not gated on PILOT_SESSION_TERMINATED" \
    'PILOT_SESSION_TERMINATED:-0}" != "1" ] && ! _check_pilot_force_push' "$DISPATCH_SRC_1772"

# R6: skip the push only when the branch carries nothing. _push_branch pushes
# "any local-ahead commits regardless of pilot exit code" (dispatch-lib.sh:1801)
# and the worktree is force-removed on the next dispatch, so a blanket skip
# would destroy the work of a session killed AFTER committing — the late-hang
# shape mika#1901 describes.
assert_contains "U2: the push skip is conditioned on an empty branch, not on termination alone" \
    'PRE_RUN_HEAD' "$DISPATCH_SRC_1772"

# --- U3: the remaining false statements ------------------------------------

# (a) "unknown" means the log could not be read. Claiming a detection was made
# is a different statement from making one and coming up empty.
assert_contains "U3a: an unreadable session log gets its own message" \
    'session log was not readable' "$PFR_SRC_1772"

# (b) the re-dispatch note must key on THIS issue's plan. The old glob matched
# any *-plan.md, and main carries 769 of them, so the note always fired.
assert_not_contains "U3b: the re-dispatch note no longer globs for any plan file" \
    'find "$WORKTREE_DIR/docs/plans" -name "*-plan.md"' \
    "$PFR_SRC_1772"

# (c) the zero-commit message must not assert an exit code it never read.
# Both 2026-08-28 tasks carried PILOT_EXIT=1.
assert_not_contains "U3c: the zero-commit message no longer hardcodes 'exited 0'" \
    'claude-pilot exited 0 but HEAD unchanged' "$PFR_SRC_1772"
assert_contains "U3c: the zero-commit message reads the real exit code" \
    'PILOT_EXIT' "$PFR_SRC_1772"

# Behavioral fixture — the 2026-08-28 class end to end: a worktree whose
# docs/plans/ holds other tickets' plans and none for this issue, HEAD
# unchanged, PILOT_EXIT=1.
_pfr_probe_1772() {
    local issue_num="$1" plant_own_plan="$2" pilot_exit="$3"
    local base wt head
    base=$(mktemp -d)
    wt="$base/worktree"
    git init -q "$wt" 2>/dev/null
    git -C "$wt" config user.email "test@test.com"
    git -C "$wt" config user.name "Test"
    mkdir -p "$wt/docs/plans"
    # Other tickets' plans — the 769-file condition on main, in miniature.
    for other in 1111 2222; do
        {
            echo "# Plan for another ticket"
            echo "**Ticket:** mika issue#${other}"
            for i in $(seq 1 12); do
                echo "Body line $i — padding padding padding padding padding."
            done
        } > "$wt/docs/plans/2026-08-01-001-fix-${other}-other-plan.md"
    done
    if [ "$plant_own_plan" = "yes" ]; then
        {
            echo "# Plan for this ticket"
            echo "**Ticket:** mika issue#${issue_num}"
            for i in $(seq 1 12); do
                echo "Body line $i — padding padding padding padding padding."
            done
        } > "$wt/docs/plans/2026-08-29-001-fix-${issue_num}-own-plan.md"
    fi
    echo "initial" > "$wt/file.txt"
    git -C "$wt" add -A 2>/dev/null
    git -C "$wt" commit -q -m "initial" 2>/dev/null
    head=$(git -C "$wt" rev-parse HEAD)
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        PRE_RUN_HEAD="$head"
        POST_RUN_HEAD="$head"
        WORKTREE_DIR="$wt"
        SKILL="dev-groom"
        REPO="mika"
        BRANCH="test-branch"
        ISSUE_NUM="$issue_num"
        SESSION_ID="s"
        LOG_ID="probe-1772-absent-log"
        PILOT_EXIT="$pilot_exit"
        STATUS="terminated"
        RESULT="claude-pilot completed (status: terminated)."
        RESCUED_DIRTY_WORKTREE=0
        exec 9>&2
        _post_flight_recovery 2>/dev/null
        printf '%s' "$RESULT"
    )
    rm -rf "$base"
}

PFR_NO_OWN_PLAN=$(_pfr_probe_1772 2013 no 1) || PFR_NO_OWN_PLAN=""
# Positive anchor first: without it the three assert_not_contains below would all
# pass on an empty string if the probe ever failed to run.
assert_contains "U3: the probe produced a real callback to assert against" \
    "PIPELINE FAILURE" "$PFR_NO_OWN_PLAN"
assert_not_contains "U3b: no plan for this issue → no re-dispatch note" \
    "HEAD unchanged on dev-groom re-dispatch" "$PFR_NO_OWN_PLAN"
assert_not_contains "U3c: PILOT_EXIT=1 → the message does not claim 'exited 0'" \
    "exited 0" "$PFR_NO_OWN_PLAN"
assert_not_contains "U3a: an absent session log → no claim that /ce:plan was undetected" \
    "no /ce:plan invocation detected in session log" "$PFR_NO_OWN_PLAN"

PFR_OWN_PLAN=$(_pfr_probe_1772 2013 yes 1) || PFR_OWN_PLAN=""
assert_contains "U3b: a plan for this issue → the re-dispatch note fires" \
    "HEAD unchanged on dev-groom re-dispatch" "$PFR_OWN_PLAN"
assert_contains "U3b: the re-dispatch note names the plan it found" \
    "2026-08-29-001-fix-2013-own-plan.md" "$PFR_OWN_PLAN"

# --- mika#1772 review round: the two populations of `terminated` -----------
#
# `status: terminated` is set both by a guardrail abort and by an SDK limit.
# The first usually kills a session that did nothing; the second often kills
# one that did a great deal. Treating them alike would skip the mika#1282
# dirty-worktree rescue for the second and tell the operator "nothing was
# written" about a branch carrying commits — the exact defect class this ticket
# closes, reintroduced by its own fix.
#
# The subtype vocabulary is deliberately NOT listed here (mika#2149): upstream
# it is `GuardrailAbortReason.guardrail` in claude-pilot's types.py plus
# `SDK_TERMINATION_SUBTYPES` in agent.py; downstream it is `_halt_family` in
# dispatch-lib.sh. The drift guard further down reads the former and checks the
# latter — a comment nobody executes went stale by five values in eighteen days.

echo ""
echo "Test: terminated sessions that left work behind (mika#1772 review)"
echo "-------------------------------------------------------------------"

# _pilot_left_no_work is the measurement the split turns on.
_left_no_work_probe() {
    local dirty="$1" moved="$2"
    local base wt pre post
    base=$(mktemp -d); wt="$base/wt"
    git init -q "$wt" 2>/dev/null
    echo initial > "$wt/f"; git -C "$wt" add -A 2>/dev/null
    git -C "$wt" commit -q -m initial --no-verify 2>/dev/null
    pre=$(git -C "$wt" rev-parse HEAD)
    post="$pre"
    if [ "$moved" = "yes" ]; then
        echo more > "$wt/g"; git -C "$wt" add -A 2>/dev/null
        git -C "$wt" commit -q -m second --no-verify 2>/dev/null
        post=$(git -C "$wt" rev-parse HEAD)
    fi
    [ "$dirty" = "yes" ] && echo uncommitted > "$wt/pilot-wrote-this.rs"
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        WORKTREE_DIR="$wt" PRE_RUN_HEAD="$pre" POST_RUN_HEAD="$post"
        if _pilot_left_no_work; then echo "NO_WORK"; else echo "HAS_WORK"; fi
    )
    rm -rf "$base"
}

assert_eq "clean tree, HEAD unmoved -> no work (the 2026-08-28 shape)" \
    "NO_WORK" "$(_left_no_work_probe no no)"
# The mika#1282 rescue exists for exactly this state; skipping it would lose the
# files, and the worktree is force-removed on the next dispatch.
assert_eq "dirty tree, HEAD unmoved -> work present" \
    "HAS_WORK" "$(_left_no_work_probe yes no)"
assert_eq "HEAD moved -> work present (the mika#1901 late-hang shape)" \
    "HAS_WORK" "$(_left_no_work_probe no yes)"
assert_eq "dirty AND moved -> work present" \
    "HAS_WORK" "$(_left_no_work_probe yes yes)"

# Banner mode: says only what was measured, and carries NO Outcome line — the
# recovery chain ran and owns that.
CLASSIFY_BANNER=$(_classify_probe '' 12 'error_max_turns' 'SDK limit reached: error_max_turns' 'banner') \
    || CLASSIFY_BANNER=""
assert_contains "banner names the SDK limit, not a guardrail" \
    "error_max_turns" "$CLASSIFY_BANNER"
assert_contains "banner names the commit range it measured" \
    "Commits:" "$CLASSIFY_BANNER"
assert_not_contains "banner never claims nothing was written" \
    "nothing was written to the branch" "$CLASSIFY_BANNER"
BANNER_OUTCOME_COUNT=$(printf '%s\n' "$CLASSIFY_BANNER" | grep -c '^Outcome:' || true)
assert_eq "banner carries no Outcome line (recovery owns it)" "0" "$BANNER_OUTCOME_COUNT"

# Full mode still earns its absolute claim, because the caller only reaches it
# when _pilot_left_no_work said so.
CLASSIFY_SDK_FULL=$(_classify_probe '' 2 'idle_timeout' 'No meaningful progress for 300s' 'full') \
    || CLASSIFY_SDK_FULL=""
assert_contains "full mode prefers the structured subtype over the stderr scrape" \
    "idle_timeout" "$CLASSIFY_SDK_FULL"
assert_contains "full mode states the measurement behind its claim" \
    "HEAD did not move and the worktree is clean" "$CLASSIFY_SDK_FULL"

# No subtype and no stderr: the message must say the cause is unrecorded rather
# than name one it did not read.
CLASSIFY_NO_CAUSE=$(_classify_probe '' 2 '' '' 'full') || CLASSIFY_NO_CAUSE=""
assert_contains "an unrecorded cause is reported as unrecorded" \
    "cause not recorded" "$CLASSIFY_NO_CAUSE"

# --- mika#2149: every halt motif gets a family and a retry hint downstream ---
#
# The subtype vocabulary is owned upstream by `GuardrailAbortReason.guardrail`
# (claude-pilot/src/claude_pilot/types.py) plus `SDK_TERMINATION_SUBTYPES` and
# the cpp#187 transport halt in agent.py. Downstream, `_halt_family` in
# dispatch-lib.sh is the ONLY place that enumerates it — a `case` with a `*)`
# arm, so a value added upstream lands on stderr as `halt_family.unknown` the
# first time it is seen instead of silently joining the prose. This comment
# deliberately lists no value: the two tables are the source of truth, and T6
# below checks them against each other.

echo ""
echo "Test: halt motifs are classified downstream (mika#2149)"
echo "--------------------------------------------------------"

# T1 — table-driven: one row per known subtype, both lines present.
while IFS='|' read -r subtype family hint; do
    T1_OUT=$(_classify_probe '' 2 "$subtype" 'x' 'full') || T1_OUT=""
    assert_contains "T1: $subtype -> Halt class: $family" \
        "Halt class: $family" "$T1_OUT"
    assert_contains "T1: $subtype -> Retry hint: $hint" \
        "Retry hint: $hint" "$T1_OUT"
done <<'T1_TABLE'
rate_limited|quota_throttled|transient
awaiting_model|model_never_resumed|transient
awaiting_tool|tool_never_returned|investigate
idle_timeout|session_silent|investigate
stream_stalled|model_stalled|transient
stall_detected|model_unproductive|investigate
empty_response|model_unproductive|investigate
watchdog_error|pilot_bug|investigate
prompt_cache_dead|substrate|investigate
error_max_turns|budget_exhausted|deterministic
error_max_budget_usd|budget_exhausted|deterministic
transport_message_too_large|transport|investigate
T1_TABLE

# T2 — a subtype outside the table is classed unknown/investigate AND said on
# stderr; a known one is not (negative control, same call shape).
T2_OUT=$(_classify_probe '' 2 'foo_bar' 'x' 'full') || T2_OUT=""
assert_contains "T2: unknown subtype -> Halt class: unknown" \
    "Halt class: unknown" "$T2_OUT"
assert_contains "T2: unknown subtype -> Retry hint: investigate" \
    "Retry hint: investigate" "$T2_OUT"
T2_ERR=$(_classify_probe '' 2 'foo_bar' 'x' 'full' '' stderr) || T2_ERR=""
assert_contains "T2: unknown subtype is named on stderr" \
    "halt_family.unknown subtype=foo_bar" "$T2_ERR"
T2_ERR_KNOWN=$(_classify_probe '' 2 'idle_timeout' 'x' 'full' '' stderr) || T2_ERR_KNOWN=""
assert_not_contains "T2 (negative control): a known subtype is silent on stderr" \
    "halt_family.unknown" "$T2_ERR_KNOWN"

# T3 — api_error_status (cpp#54) is a qualifier on the Halt: line, present
# only when the JSON carried it.
T3_OUT=$(_classify_probe '' 2 'rate_limited' 'backoff exhausted' 'full' '429') || T3_OUT=""
assert_contains "T3: api_error_status renders as (HTTP n) on the Halt: line" \
    "Halt: rate_limited (HTTP 429)" "$T3_OUT"
T3_NONE=$(_classify_probe '' 2 'rate_limited' 'backoff exhausted' 'full' '') || T3_NONE=""
assert_not_contains "T3 (negative control): no status -> no (HTTP" \
    "(HTTP" "$T3_NONE"
assert_contains "T3 (negative control): the Halt: line is otherwise intact" \
    "Halt: rate_limited — backoff exhausted" "$T3_NONE"

# T4 — fallback path: no subtype on the JSON, the `[guardrail]` line in stderr
# (ANSI-coloured, as ui.py:113 writes it) feeds the same table.
T4_LINE=$(printf '\033[38;5;208m[guardrail]\033[0m \033[1mawaiting_model\033[0m: model-wait ceiling 900s exceeded')
T4_OUT=$(_classify_probe "$T4_LINE" 2 '' '' 'full') || T4_OUT=""
assert_contains "T4: a stderr-only halt is classified like a JSON one" \
    "Halt class: model_never_resumed" "$T4_OUT"
assert_contains "T4: the Halt: line keeps the scraped text" \
    "Halt: [guardrail] awaiting_model:" "$T4_OUT"
T4_NONE=$(_classify_probe 'no guardrail line here' 2 '' '' 'full') || T4_NONE=""
assert_contains "T4 (negative control): no [guardrail] line -> unknown" \
    "Halt class: unknown" "$T4_NONE"
assert_contains "T4 (negative control): 'cause not recorded' is preserved" \
    "cause not recorded" "$T4_NONE"
T4_NONE_ERR=$(_classify_probe 'no guardrail line here' 2 '' '' 'full' '' stderr) || T4_NONE_ERR=""
assert_not_contains "T4 (negative control): an unrecorded cause is not reported as upstream drift" \
    "halt_family.unknown" "$T4_NONE_ERR"

# T5 — banner mode carries both lines too.
T5_OUT=$(_classify_probe '' 12 'awaiting_tool' 'tool-wait ceiling exceeded' 'banner') || T5_OUT=""
assert_contains "T5: banner mode carries Halt class:" \
    "Halt class: tool_never_returned" "$T5_OUT"
assert_contains "T5: banner mode carries Retry hint:" \
    "Retry hint: investigate" "$T5_OUT"

# T6 — the drift guard (C-5): read the upstream Literal, demand a family for
# every value. Runs on the dispatch host, where claude-pilot is always present
# (it is what this file dispatches). Elsewhere it SKIPs and says so.
#
# T6-arm (F1, first architect pass): a SKIP buried in thousands of output lines
# is indistinguishable from a green to the eye that reads the last line. So the
# block emits EXACTLY ONE `DRIFT-GUARD:` marker on stdout, and a companion
# assertion reads that stdout back — a bare green with no marker is a red. The
# block runs in the current shell (so PASS/FAIL/SKIPPED survive) with its stdout
# duplicated into a capture file by process substitution.
_t6_locate_types_py() {
    # An explicit variable is authoritative — including when it points nowhere,
    # which is how T6-arm control (a) forces the SKIP branch.
    if [ -n "${CLAUDE_PILOT_TYPES:-}" ]; then
        [ -r "$CLAUDE_PILOT_TYPES" ] && printf '%s\n' "$CLAUDE_PILOT_TYPES"
        return 0
    fi
    # <meta>/mika and <meta>/.claude/worktrees/<slug>/mika both have
    # claude-pilot/ one or three levels above the repo root.
    local top c
    # From the script's own repo, never the caller's cwd (mika#2149 review, #6):
    # launched from the meta-repo root, a bare rev-parse resolved the wrong
    # toplevel and the guard SKIPped on the very host it exists to protect.
    top=$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel 2>/dev/null) || return 0
    for c in "$top/../claude-pilot/src/claude_pilot/types.py" \
             "$top/../../../../claude-pilot/src/claude_pilot/types.py"; do
        if [ -r "$c" ]; then
            (cd "$(dirname "$c")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$c")")
            return 0
        fi
    done
    return 0
}

_t6_drift_guard() {
    local types_py values n v family
    types_py=$(_t6_locate_types_py)
    if [ -z "$types_py" ]; then
        echo "DRIFT-GUARD: SKIP — types.py unreachable (set CLAUDE_PILOT_TYPES)"
        SKIPPED=$((SKIPPED + 1))
        return 0
    fi
    # The `guardrail: Literal[ ... ]` block, EVERY quoted token — then fail
    # closed on any token outside the shape a subtype can have (mika#2149
    # review, #3): a charset filter in the extraction excluded a digit-bearing
    # name instead of failing on it, so the guard counted it out and stayed
    # green. The shape mirrors the runtime scrape in dispatch-lib.sh.
    values=$(sed -n '/guardrail: Literal\[/,/\]/p' "$types_py" | grep -o '"[^"]*"' | tr -d '"' || true)
    n=$(printf '%s\n' "$values" | grep -c . || true)
    echo "DRIFT-GUARD: armed against $types_py ($n values)"
    if [ "$n" -eq 0 ]; then
        FAIL=$((FAIL + 1))
        echo "  ✗ T6: the Literal block was found empty — the sed anchor no longer matches types.py"
        return 0
    fi
    for v in $values; do
        if ! grep -Eq -- '^[a-z][a-z0-9_]*$' <<<"$v"; then
            FAIL=$((FAIL + 1))
            echo "  ✗ T6: upstream token '$v' is outside the subtype shape [a-z][a-z0-9_]* — widen the runtime scrape and this guard together"
            continue
        fi
        family=$( ( source "$DISPATCH_LIB" 2>/dev/null || true; _halt_family "$v" 2>/dev/null | cut -d'|' -f1 ) )
        # Positive control first: an empty family (function renamed, source
        # aborted in the subshell) must not pass the drift check vacuously.
        assert_eq "T6 drift: upstream value '$v' resolved to a non-empty family" \
            "yes" "$([ -n "$family" ] && echo yes || echo no)"
        assert_not_contains "T6 drift: upstream value '$v' has a downstream family (got: $family)" \
            "unknown" "$family"
    done
}

T6_CAPTURE=$(mktemp)
_t6_drift_guard > >(tee "$T6_CAPTURE")
wait $! 2>/dev/null || true
T6_MARKER_COUNT=$(grep -c '^DRIFT-GUARD: ' "$T6_CAPTURE" || true)
assert_eq "T6-arm: exactly one DRIFT-GUARD marker was emitted on stdout" \
    "1" "$T6_MARKER_COUNT"
rm -f "$T6_CAPTURE"

# T6 fixtures — the guard's own branches, driven in a subshell so its
# counters do not leak into this run's (mika#2149 review, #3 and testing gaps).
# Each fixture is a minimal types.py; the guard's stdout is the assertion
# surface, and its FAIL count is printed last so the fail-closed direction is
# pinned by number, not by prose.
_t6_fixture_probe() {
    local body="$1" fx
    fx=$(mktemp)
    printf '%s\n' "$body" > "$fx"
    (
        PASS=0; FAIL=0; SKIPPED=0
        CLAUDE_PILOT_TYPES="$fx" _t6_drift_guard
        echo "FAIL=$FAIL SKIPPED=$SKIPPED"
    )
    rm -f "$fx"
}
# (a) a digit-bearing name is iterated, not silently dropped: it reaches the
#     table and comes back unknown, and the guard names it.
T6_FX_DIGIT=$(_t6_fixture_probe '    guardrail: Literal[
        "idle_timeout",
        "http_529",
    ]')
assert_contains "T6 fixture: a digit-bearing upstream value is counted" \
    "(2 values)" "$T6_FX_DIGIT"
assert_contains "T6 fixture: a digit-bearing unknown value is named red" \
    "✗ T6 drift: upstream value 'http_529' has a downstream family (got: unknown)" "$T6_FX_DIGIT"
assert_contains "T6 fixture: exactly one red for the one unknown value" \
    "FAIL=1 SKIPPED=0" "$T6_FX_DIGIT"
# (b) a token outside the subtype shape fails closed, by name.
T6_FX_SHAPE=$(_t6_fixture_probe '    guardrail: Literal[
        "idle_timeout",
        "Bad-Token",
    ]')
assert_contains "T6 fixture: an out-of-shape token is refused by name" \
    "upstream token 'Bad-Token' is outside the subtype shape" "$T6_FX_SHAPE"
assert_contains "T6 fixture: the refusal counts as a FAIL" \
    "FAIL=1 SKIPPED=0" "$T6_FX_SHAPE"
# (c) a matched-but-empty Literal block is a red, not a vacuous green.
T6_FX_EMPTY=$(_t6_fixture_probe '    guardrail: Literal[
    ]')
assert_contains "T6 fixture: an empty Literal block is named" \
    "the Literal block was found empty" "$T6_FX_EMPTY"
assert_contains "T6 fixture: an empty Literal block is a FAIL" \
    "FAIL=1 SKIPPED=0" "$T6_FX_EMPTY"
# (d) the SKIP branch, in-file: marker + SKIPPED, zero content assertions.
T6_FX_SKIP=$( ( PASS=0; FAIL=0; SKIPPED=0; CLAUDE_PILOT_TYPES=/nonexistent _t6_drift_guard; echo "FAIL=$FAIL SKIPPED=$SKIPPED PASS=$PASS" ) )
assert_contains "T6 fixture: unreachable types.py emits the SKIP marker" \
    "DRIFT-GUARD: SKIP" "$T6_FX_SKIP"
assert_contains "T6 fixture: unreachable types.py counts SKIPPED and asserts nothing" \
    "FAIL=0 SKIPPED=1 PASS=0" "$T6_FX_SKIP"

# T2-sink — the drift line through the PRODUCTION channel (mika#2149 review,
# #1). The probe mirrors dispatch_claude_pilot: fd 2 is /dev/null (the
# `exec 9>>"$TRACE_FILE" 2>/dev/null` at its top), and the two sinks are the
# files the callback tail and the persisted .stderr are built from. A bare
# `>&2` passes the earlier T2 (which merges fd 2 in the probe) and lands
# nowhere here — that is the defect this probe exists to keep closed.
_classify_sink_probe() {
    local subtype="$1" tmp
    tmp=$(mktemp -d)
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        exec 2>/dev/null
        STATUS="terminated"; TURNS=2; DURATION=1; SESSION_ID=s; LOG_ID="probe-sink"
        PILOT_LOG_DIR="$tmp"
        STDERR_FILE="$tmp/stderr.tmp"; : > "$STDERR_FILE"
        PERSISTENT_STDERR="$tmp/probe-sink.stderr"; : > "$PERSISTENT_STDERR"
        SUBTYPE="$subtype"; TERMINATION_REASON="x"; API_ERROR_STATUS=""
        _classify_terminated_session >/dev/null
        printf 'tail:%s\n' "$(cat "$STDERR_FILE")"
        printf 'persisted:%s\n' "$(cat "$PERSISTENT_STDERR")"
    )
    rm -rf "$tmp"
}
T2_SINK=$(_classify_sink_probe foo_bar) || T2_SINK=""
assert_contains "T2-sink: the drift line reaches the callback-tail source (STDERR_FILE)" \
    "tail:dispatch-lib: halt_family.unknown subtype=foo_bar" "$T2_SINK"
assert_contains "T2-sink: the drift line reaches the persisted .stderr" \
    "persisted:dispatch-lib: halt_family.unknown subtype=foo_bar" "$T2_SINK"
T2_SINK_KNOWN=$(_classify_sink_probe idle_timeout) || T2_SINK_KNOWN=""
assert_not_contains "T2-sink (negative control): a known subtype writes nothing to either sink" \
    "halt_family.unknown" "$T2_SINK_KNOWN"

# ============================================================================
# mika#2029 — un pilote tué sans avoir produit se compte par son NOM
#
# Le trou mesuré : la branche `_terminated_empty` est la SEULE qui saute
# `_post_flight_recovery`, donc la seule où aucun des deux sites de pose de la
# ligne `PR:`/`NO_PR:` ne tourne. Le callback partait sans aucune des deux — 8/8
# des kills depuis le 2026-09-04 — et le reaper (`engine.rs`
# `reap_orphaned_parent_tasks`) retombait sur son défaut
# `callback_delivered_without_pr_url`, motif qui confond « le pilote a été tué
# avant de produire » avec « le callback est malformé ». Deux pannes, un seul
# nom, aucune mesure possible.
#
# La justification est STRUCTURELLE et non statistique (D1) : le contrat total de
# mika#2121 veut exactement une ligne par callback livré. Ce chemin en doit une
# même si la classe s'éteignait demain — et elle n'est pas éteinte : zéro
# `idle_timeout` en octobre sur 99 sessions, mais P(0)≈0,30 sous un taux
# inchangé, donc l'extinction n'est pas établie.
# ============================================================================

echo ""
echo "Test: le kill sans production porte un motif nommé (mika#2029)"
echo "---------------------------------------------------------------"

# V0 (anti-vacuité) — AVANT tout le reste. Une sonde qui regarde une fonction
# disparue ou vide se lit exactement comme un arbre propre (classe mika#2205),
# et tout ce qui suit serait vacuement vert sur une chaîne vide.
T2029_REASON_FN=$(sed -n '/^_no_pr_reason_from_halt_subtype() {$/,/^}$/p' "$DISPATCH_LIB")
assert_eq "V0 (anti-vacuité): _no_pr_reason_from_halt_subtype existe, corps non vide" "yes" \
    "$(if [ "$(printf '%s\n' "$T2029_REASON_FN" | grep -c .)" -gt 3 ]; then printf 'yes'; else printf 'no'; fi)"

# La branche `_terminated_empty` de `_run_claude_pilot`, extraite une fois. Elle
# n'est pas appelable (il faut un vrai pilote et un vrai CLI), donc la sonde
# comportementale ci-dessous REJOUE sa composition — et V3 épingle cette
# composition au texte exact du site de production, pour que la réplication ne
# puisse pas dériver en silence.
#
# L'extraction lit le FICHIER et non `declare -f` : le dé-parseur de bash rend
# `fi;` avec un point-virgule, donc l'ancre de fin de plage ne matcherait pas et
# la plage courrait jusqu'à la fin de la fonction — une sonde qui regarde trois
# fois trop de texte sans le dire.
T2029_BRANCH=$(sed -n '/^    if \[ "$_terminated_empty" = "1" \]; then$/,/^    fi$/p' "$DISPATCH_LIB")
assert_eq "V0 (anti-vacuité): la branche _terminated_empty a bien été extraite" "yes" \
    "$(if [ "$(printf '%s\n' "$T2029_BRANCH" | grep -c .)" -gt 5 ]; then printf 'yes'; else printf 'no'; fi)"
# La vue SANS commentaires, pour les scans de code. Les commentaires de la
# branche citent `_resolve_halt_subtype` et `RESULT=$(_classify_terminated_session)`
# par leur nom — un scan qui les compterait mesurerait sa propre prose.
T2029_BRANCH_CODE=$(printf '%s\n' "$T2029_BRANCH" | grep -vE '^[[:space:]]*#')
assert_eq "V0 (anti-vacuité): la vue sans commentaires garde le code des deux branches" "yes" \
    "$(if [ "$(printf '%s\n' "$T2029_BRANCH_CODE" | grep -c .)" -ge 8 ]; then printf 'yes'; else printf 'no'; fi)"

# Le LECTEUR de la ligne, reproduit : `RE_NO_PR` dans dispatcher.rs est
# `(?m)^NO_PR:\s+([a-z_]+)`. `[a-z_]+` est gourmand et NON ancré à droite,
# exactement comme la regex Rust — donc une troncature ne se voit pas par un
# échec, elle se voit par une capture plus COURTE que le motif posé. C'est
# précisément ce que cette fonction rend observable.
_t2029_re_no_pr_capture() {
    printf '%s\n' "$1" | sed -n 's/^NO_PR:[[:space:]]\{1,\}\([a-z_]\{1,\}\).*/\1/p' | head -1
}

# La composition de la branche, rejouée sur un environnement injecté. Le
# sous-type passe par `_resolve_halt_subtype` (D4) — les deux canaux, `.subtype`
# structuré et scrape `[guardrail]`, sont donc couverts par la même sonde.
_t2029_line_probe() {
    local subtype="$1" guardrail_line="$2" seed="${3:-}"
    local tmp log_id="probe-2029"
    tmp=$(mktemp -d)
    if [ -n "$guardrail_line" ]; then
        printf '%s\n' "$guardrail_line" > "$tmp/${log_id}.stderr"
    fi
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        STATUS="terminated"; TURNS=2; DURATION=1
        SESSION_ID="0dab1700-5ca9-4145-87b0-6f618a047220"
        LOG_ID="$log_id"; PILOT_LOG_DIR="$tmp"
        STDERR_FILE=""; SUBTYPE="$subtype"
        TERMINATION_REASON=""; API_ERROR_STATUS=""
        RESULT="$seed"
        _resolve_halt_subtype
        _set_pr_status_line "NO_PR: $(_no_pr_reason_from_halt_subtype "${_HALT_SUBTYPE_RESOLVED:-}")"
        printf '%s' "$RESULT"
    )
    rm -rf "$tmp"
}

# V1a — les quatre sous-types MESURÉS sur la fenêtre de 30 jours (578 sessions,
# M2) : error_max_turns 30, stall_detected 6, idle_timeout 5, prompt_cache_dead 1.
# Un par ligne, parce que le motif porte le sous-type et non la famille (D2) :
# `budget_exhausted` confondrait error_max_turns avec error_max_budget_usd, et
# `model_unproductive` confondrait stall_detected avec empty_response.
for _t2029_st in error_max_turns stall_detected idle_timeout prompt_cache_dead; do
    T2029_OUT=$(_t2029_line_probe "$_t2029_st" '') || T2029_OUT=""
    assert_contains "V1a: $_t2029_st -> NO_PR: pilot_terminated_$_t2029_st" \
        "NO_PR: pilot_terminated_$_t2029_st" "$T2029_OUT"
    # V1b — le motif posé survit ENTIER à RE_NO_PR. Une capture plus courte
    # serait un motif plausible, agrégeable, et faux.
    assert_eq "V1b: RE_NO_PR consomme pilot_terminated_$_t2029_st en entier" \
        "pilot_terminated_$_t2029_st" \
        "$(_t2029_re_no_pr_capture "$(printf '%s\n' "$T2029_OUT" | grep '^NO_PR: ')")"
done

# V1a-bis — le second canal. Un résultat sans `.subtype` fait scraper la ligne
# `[guardrail]` par `_resolve_halt_subtype` ; le motif doit suivre ce canal-là
# aussi, sinon la population que mika#2149 (C-4) a rattrapée pour la bannière
# repartirait en `unknown` pour le comptage.
T2029_SCRAPED=$(_t2029_line_probe '' '[guardrail] idle_timeout: No meaningful progress for 300s') || T2029_SCRAPED=""
assert_contains "V1a-bis: un halt arrivé par stderr est compté comme un halt structuré" \
    "NO_PR: pilot_terminated_idle_timeout" "$T2029_SCRAPED"

# V1c — la normalisation, et la troncature qu'elle empêche. Les sous-types
# viennent d'un `Literal` Python amont que personne ici ne contrôle : la
# correction est sur la FORME de ce qu'on pose, jamais sur une liste de valeurs.
T2029_UPPER=$(_t2029_line_probe 'Error-Max-Turns' '') || T2029_UPPER=""
assert_contains "V1c: Error-Max-Turns est normalisé, pas tronqué" \
    "NO_PR: pilot_terminated_error_max_turns" "$T2029_UPPER"
assert_eq "V1c: et RE_NO_PR en consomme la totalité" \
    "pilot_terminated_error_max_turns" \
    "$(_t2029_re_no_pr_capture "$(printf '%s\n' "$T2029_UPPER" | grep '^NO_PR: ')")"
# La contre-mesure : SANS normalisation, le même sous-type arrive dans
# `tasks.result` en `callback_no_pr_pilot_terminated_error`. Pas une erreur —
# une mesure fausse. C'est ce que V1c achète.
assert_eq "V1c (contre-mesure): le motif NON normalisé serait tronqué par RE_NO_PR" \
    "pilot_terminated_error" \
    "$(_t2029_re_no_pr_capture 'NO_PR: pilot_terminated_error-max-turns')"

# V1d — aucune résolution est une résolution. La laisser produire
# `pilot_terminated_` tendrait à RE_NO_PR une queue VIDE : le `+` ne matcherait
# pas, la ligne serait ignorée, et le kill retomberait dans le générique que ce
# livrable comble — un trou refermé par son propre correctif.
T2029_NONE=$(_t2029_line_probe '' '') || T2029_NONE=""
assert_contains "V1d: ni sous-type ni ligne [guardrail] -> pilot_terminated_unknown" \
    "NO_PR: pilot_terminated_unknown" "$T2029_NONE"
assert_eq "V1d: la queue n'est jamais vide (RE_NO_PR capture quelque chose)" \
    "pilot_terminated_unknown" \
    "$(_t2029_re_no_pr_capture "$(printf '%s\n' "$T2029_NONE" | grep '^NO_PR: ')")"

# V1e — le contrat TOTAL de mika#2121 : exactement une ligne, et une seule des
# deux formes. La graine porte une ligne `PR:` pour que le strip de
# `_set_pr_status_line` soit mesuré et non supposé.
T2029_SEEDED=$(_t2029_line_probe idle_timeout '' 'Outcome: PIPELINE_INCOMPLETE — x
PR: https://github.com/senara-solutions/mika/pull/1') || T2029_SEEDED=""
assert_eq "V1e: exactement une ligne NO_PR:" "1" \
    "$(printf '%s\n' "$T2029_SEEDED" | grep -c '^NO_PR: ' || true)"
assert_eq "V1e: et zéro ligne PR:" "0" \
    "$(printf '%s\n' "$T2029_SEEDED" | grep -c '^PR: ' || true)"
# Comptée seule, « exactement une ligne NO_PR: » est VERTE sur l'état d'avant le
# correctif : une ligne `NO_PR: ` sans queue est toujours une ligne. C'est la
# panne même que ce livrable ferme, et elle passerait ce comptage. Donc la ligne
# est lue EN ENTIER, au texte.
assert_eq "V1e (anti-vacuité): et cette ligne porte le motif en entier" \
    "NO_PR: pilot_terminated_idle_timeout" \
    "$(printf '%s\n' "$T2029_SEEDED" | grep '^NO_PR: ' || true)"
assert_contains "V1e: le reste du RESULT est intact" \
    "Outcome: PIPELINE_INCOMPLETE" "$T2029_SEEDED"

# V1f (contre-vacuité) — la population qui a PRODUIT ne reçoit pas ce motif.
# Elle passe par `_post_flight_recovery`, qui pose déjà sa propre ligne ; un
# second motif ici la ferait compter deux fois, et le `banner` dirait d'une
# branche porteuse de commits qu'aucune PR n'en est sortie faute de travail.
# Sans cette assertion, V1a–V1e passeraient à l'identique sur un correctif qui
# poserait la ligne pour TOUT le monde — le défaut que mika#1772 a fermé,
# réintroduit par le correctif de mika#2029.
assert_not_contains "V1f (contre-vacuité): le mode banner ne porte aucun pilot_terminated_*" \
    "pilot_terminated_" "$CLASSIFY_BANNER"
T2029_ELSE=$(printf '%s\n' "$T2029_BRANCH" | sed -n '/^    else$/,$p')
assert_eq "V1f (contre-vacuité): la branche else a bien été extraite" "yes" \
    "$(if [ "$(printf '%s\n' "$T2029_ELSE" | grep -c .)" -gt 3 ]; then printf 'yes'; else printf 'no'; fi)"
assert_contains "V1f (contre-vacuité): et c'est bien celle qui appelle la chaîne de reprise" \
    "_post_flight_recovery" "$T2029_ELSE"
assert_not_contains "V1f (contre-vacuité): la branche else ne compose aucun motif" \
    "_no_pr_reason_from_halt_subtype" "$T2029_ELSE"
# Un seul site de production compose ce motif. Le compte est fait hors
# définition et hors commentaire — seuls les appels.
assert_eq "V1f: _no_pr_reason_from_halt_subtype a exactement un appelant de production" "1" \
    "$(grep -n '_no_pr_reason_from_halt_subtype' "$DISPATCH_LIB" \
        | grep -v '_no_pr_reason_from_halt_subtype() {' \
        | grep -vE ':[[:space:]]*#' | grep -c . || true)"

# --- V2 : le diagnostic réfuté ne survit pas à sa réfutation ----------------
#
# La chaîne « rejet d'auth → stall SDK → idle_timeout 300s → mort au tour 1 »
# était écrite à deux endroits : le commentaire au-dessus de
# `_run_pilot_sandboxed` et le texte LIVRÉ à mika-dev en mode `full`. Elle a été
# réfutée le 2026-08-29 (`POST /v1/messages -> 200` sur les sessions mêmes dont
# elle parlait, six lignées éliminées), et la cause réelle vivait côté pilote
# (claude-pilot#177, CLOSED 2026-09-11). Un diagnostic faux dans le texte livré
# coûte plus qu'un silence : il oriente la lecture suivante vers la mauvaise
# moitié du système.
assert_eq "V2a: le diagnostic réfuté a disparu de dispatch-lib.sh" "0" \
    "$(grep -c 'Anthropic 401' "$DISPATCH_LIB" || true)"
# V2b (contre-vacuité, auto-nettoyante) — le détecteur de V2a est un `grep -c`
# dont le succès est « rien trouvé » : il serait vert sur un fichier vide, sur un
# chemin faux, et sur une suppression trop large. Le 401 GitHub de mika#2056
# (`:1082`) est un 401 LÉGITIME et mesuré, sur un autre service ; il est la
# frontière. S'il disparaît, V2a cesse de mesurer ce qu'il prétend et cette
# ligne rougit le jour même.
assert_contains "V2b (contre-vacuité): le 401 GitHub légitime de mika#2056 est intact" \
    "401 on api.github.com" "$(cat "$DISPATCH_LIB")"
# V2c — le texte livré nomme la cause réelle, il ne se contente pas de taire la
# fausse (D7). Un opérateur qui lit ce callback doit pouvoir aller au bon ticket.
T2029_FULL=$(_classify_probe '' 2 'idle_timeout' 'No meaningful progress for 300s' 'full') || T2029_FULL=""
assert_contains "V2c: le mode full nomme la cause réelle (claude-pilot#177)" \
    "claude-pilot#177" "$T2029_FULL"
assert_contains "V2c: et renvoie l'opérateur à la ligne Halt class:" \
    "The 'Halt class:' line above is what names it" "$T2029_FULL"
assert_not_contains "V2c: le texte livré ne porte plus la chaîne réfutée" \
    "SDK-stall chain" "$T2029_FULL"
# V2c-bis — la phrase trouée. Le corps de `_classify_terminated_session` est un
# `printf '%s' "…"` entre guillemets DOUBLES : un accent grave y est une
# substitution de commande, pas une citation. Mesuré en écrivant ce correctif —
# le premier jet livrait « The  line above is what names it », une phrase à trou,
# et la commande inventée échouait sur un fd 2 déjà dirigé vers /dev/null
# (mika#903), donc en silence. Un texte de diagnostic qui perd un mot est de la
# même famille que le diagnostic faux que V2a retire : il oriente la lecture
# suivante vers rien. Le scan est sur la FORME, au site, parce qu'une assertion
# sur une phrase ne couvre que la phrase qu'elle cite.
T2029_CLASSIFY_FN=$(sed -n '/^_classify_terminated_session() {$/,/^}$/p' "$DISPATCH_LIB")
assert_eq "V2c-bis (anti-vacuité): _classify_terminated_session a bien été extraite" "yes" \
    "$(if [ "$(printf '%s\n' "$T2029_CLASSIFY_FN" | grep -c .)" -gt 20 ]; then printf 'yes'; else printf 'no'; fi)"
assert_eq "V2c-bis: aucun accent grave dans le texte livré (ce serait une substitution)" "0" \
    "$(printf '%s\n' "$T2029_CLASSIFY_FN" | grep -vE '^[[:space:]]*#' | grep -c '`' || true)"

# --- V3 : le sous-type vient du résolveur, pas d'un $SUBTYPE brut -----------
#
# Même forme que le scan mika#2539 (S4) et pour la même raison : un troisième
# lecteur de `$SUBTYPE` ferait diverger le motif de la bannière sur la population
# que le scrape `[guardrail]` rattrape — `Halt class: session_silent` dans le
# callback et `callback_no_pr_pilot_terminated_unknown` dans `tasks.result`, deux
# énoncés contradictoires sur une seule session, dont un dans la base.
assert_contains "V3: la branche résout le sous-type par le résolveur unique" \
    "_resolve_halt_subtype" "$T2029_BRANCH_CODE"
assert_not_contains "V3: la branche ne lit aucun \$SUBTYPE brut" \
    'SUBTYPE:-}' "$(printf '%s\n' "$T2029_BRANCH_CODE" | grep -v '_HALT_SUBTYPE_RESOLVED')"
assert_contains "V3: elle pose la ligne par _set_pr_status_line (D5)" \
    "_set_pr_status_line" "$T2029_BRANCH_CODE"
# La composition EXACTE, au texte. C'est ce qui empêche la sonde
# `_t2029_line_probe` ci-dessus — qui rejoue la branche faute de pouvoir
# l'appeler — de mesurer une composition que la production n'utilise plus.
assert_contains "V3: la composition rejouée par les sondes est celle du site de production" \
    '_set_pr_status_line "NO_PR: $(_no_pr_reason_from_halt_subtype "${_HALT_SUBTYPE_RESOLVED:-}")"' \
    "$T2029_BRANCH"
# L'ORDRE compte, et pas qu'un peu : `RESULT=$(_classify_terminated_session)` est
# une substitution de commande, donc un SOUS-SHELL. Le résolveur appelé APRÈS
# aurait à re-scraper stderr ; appelé DEDANS, son mémo mourrait avec le
# sous-shell. Appelé AVANT, le mémo est hérité — bannière et motif décrivent la
# même résolution par construction, et stderr n'est lu qu'une fois.
T2029_ORDER=$(printf '%s\n' "$T2029_BRANCH_CODE" \
    | grep -nE '_resolve_halt_subtype|RESULT=\$\(_classify_terminated_session\)|_set_pr_status_line' \
    | sed 's/:.*//' | tr '\n' ' ' | sed 's/ *$//')
assert_eq "V3: résolution AVANT la substitution, pose APRÈS l'affectation de RESULT" "yes" \
    "$(set -- $T2029_ORDER; if [ "$#" -eq 3 ] && [ "$1" -lt "$2" ] && [ "$2" -lt "$3" ]; then printf 'yes'; else printf 'no'; fi)"

# --- V4 : herméticité de cette section (patron mika#2178 T9 / mika#2493) ----
T2029_SECTION=$(sed -n '/^# mika#2029 — un pilote tué sans avoir produit se compte par son NOM/,/^# The caller must route on the measurement/p' "${BASH_SOURCE[0]}")
assert_eq "Herméticité: l'extraction de la section mika#2029 a trouvé la section" \
    "yes" "$(if [ -n "$T2029_SECTION" ]; then printf 'yes'; else printf 'no'; fi)"
assert_eq "Herméticité: aucune invocation de forge en tête de commande" \
    "0" "$(printf '%s\n' "$T2029_SECTION" | grep -cE '^[[:space:]]*gh[[:space:]]' || true)"
assert_eq "Herméticité: aucune invocation de forge en substitution" \
    "0" "$(printf '%s\n' "$T2029_SECTION" | grep -cE '\$\(gh[[:space:]]' || true)"
assert_eq "Herméticité: aucune écriture hors mktemp (ni /var/log, ni le dépôt)" \
    "0" "$(printf '%s\n' "$T2029_SECTION" | grep -cE '>[[:space:]]*"?(/var/log|\$SCRIPT_DIR|\$DISPATCH_LIB)' || true)"

# The caller must route on the measurement, not on STATUS alone.
assert_contains "the terminated branch is gated on _pilot_left_no_work" \
    "_pilot_left_no_work" "$RCP_SRC_1772"
assert_contains "a terminated session with work still runs _post_flight_recovery" \
    "_post_flight_recovery" "$RCP_SRC_1772"

# The plan line prefers the measurement that can answer on a first grooming.
# _committed_plan_on_branch asks the remote AND needs a body callout, so on the
# run this line exists for it is silent; VALID_PLAN is the worktree answer.
VALID_FIRST=$(printf '%s\n' "$DISPATCH_SRC_1772" \
    | grep -nE 'VALID_PLAN:-|_committed_plan_on_branch' | head -1)
assert_contains "plan line consults the worktree measurement before the remote one" \
    "VALID_PLAN" "$VALID_FIRST"

# Reason coverage, per site rather than by total. The earlier count comparison
# carried four units of slack, so four future un-reasoned exits could land
# without tripping it.
# mika#3118: le corps à scanner devient un ARGUMENT, défaut inchangé. Un seul
# prédicat pour les deux corps — la boucle et la porte qu'elle délègue —, parce
# que deux sondes recopiées divergeraient (mika#2158) et que c'est précisément la
# délégation qu'il faut vérifier plutôt que promettre.
_reason_pairing_check() {
    local body line prev1="" prev2="" bad=0 n=0
    body=$(printf '%s\n' "${1:-$ITERATE_SRC_1772}")
    while IFS= read -r line; do
        case "$line" in
            *"return 1"*)
                n=$((n + 1))
                case "$line$prev1$prev2" in
                    # mika#2296: `_groom_warn_empty_content` is listed by name,
                    # not covered by relaxing the pattern to `*_groom_warn*`.
                    # It delegates to `_groom_warn` and therefore genuinely sets
                    # the reason — but a wildcard prefix would also accept any
                    # future `_groom_warn_*` that does not, which is how a
                    # detector stops detecting.
                    # mika#3118: `_groom_plan_halt_gate` est listée par son NOM,
                    # pour la même raison et avec la même dette : elle pose
                    # `GROOM_LOOP_FAILURE_REASON` sur chacun de ses deux chemins
                    # de refus, et l'assertion qui suit ce scan le VÉRIFIE sur
                    # son corps au lieu de le croire sur parole. Un nom ajouté
                    # ici sans cette contrepartie est un trou dans le scan.
                    *_groom_warn\ *|*_groom_warn_empty_content\ *|*_groom_plan_halt_gate\ *|*GROOM_LOOP_FAILURE_REASON=*) ;;
                    *) bad=$((bad + 1)); echo "    unreasoned exit: $(printf '%s' "$line" | sed 's/^ *//')" >&2 ;;
                esac
                ;;
        esac
        case "$line" in
            "") ;;
            *) prev2="$prev1"; prev1="$line" ;;
        esac
    done <<<"$body"
    echo "${bad}/${n}"
}
REASON_PAIRING=$(_reason_pairing_check 2>/dev/null)
assert_eq "every return 1 in the loop has a reason setter within two lines" \
    "0/${REASON_PAIRING#*/}" "$REASON_PAIRING"

# mika#3118 — la contrepartie du nom ajouté à la liste ci-dessus. Le scan de la
# boucle accepte désormais une sortie dont la raison est posée AILLEURS ; ce
# qu'il accepte doit donc être mesuré là où ça vit, sinon le nom est un trou.
# Le MÊME prédicat, appliqué au corps de la porte.
GATE_SRC_3118=$(declare -f _groom_plan_halt_gate)
assert_contains "mika#3118: le corps de la porte a été extrait (jamais une chaîne vide)" \
    "_find_issue_plan" "$GATE_SRC_3118"
GATE_PAIRING=$(_reason_pairing_check "$GATE_SRC_3118" 2>/dev/null)
assert_eq "mika#3118: chaque return 1 de la porte pose sa raison à moins de deux lignes" \
    "0/${GATE_PAIRING#*/}" "$GATE_PAIRING"
# Et la cardinalité, parce qu'un corps qui aurait perdu un de ses deux refus
# passerait le pairing en ne refusant plus rien (classe mika#2205).
assert_eq "mika#3118: la porte a exactement deux chemins de refus" \
    "2" "${GATE_PAIRING#*/}"

# --- Summary ---

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed, SKIPPED: $SKIPPED"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0

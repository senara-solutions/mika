#!/bin/bash
# Test suite: a halt the pilot DECLARED is read from the session log's TAIL
# (mika#3142).
#
# WHY. When the per-flight line budget of mika#2693 is exceeded, the pilot stops
# on purpose: no commit, no push, no PR, and a last message of the form
# «**HALTE borne : N lignes … pour `M`**». That declaration is invisible to the
# end-of-flight rescue, because `_run_claude_pilot` reads exactly eight fields
# out of claude-pilot's JSON — `status`, `session_id`, `turns`, `subtype`,
# `termination_reason`, `api_error_status`, `cost_usd`, `duration_ms` — and NONE
# of them carries the pilot's last message. The observable signature of a clean
# turn with no shipping tail is then EXACTLY the `no-shipping-tail` class of
# mika#2492, which is the nominal path of the autonomous loop under the lean
# tails of mika#2694: the rescue pushes the branch and opens a draft.
#
# Measured on #3081, #3092, #3100, #3108. On #3081 it went one step further: the
# draft carried `rescue-pipeline-verified: yes`, and the `auto_resume_wip_rescue_drafts`
# daemon (mika#1852) then un-drafted it — halted work went to review with no
# decision ever taken.
#
# WHY A BARE `grep "HALTE borne"` IS A GUARANTEED FALSE POSITIVE, and that is
# the whole suite. The rule is QUOTED in the plan injected into the pilot's
# prompt, so it appears near the HEAD of every session log. Measured on the two
# logs of mika#3119 § Défaut B:
#
#   log            rule quoted at   declared at              after it   size
#   4492a670-…     l. 100           l. 9630 (`379 lignes`)   24 l.      9 654 l.
#   5813f896-…     l. 88            l. 7268 (`455 lignes`)   26 l.      7 294 l.
#
# Hence TWO independent anchors, and the suite proves each one is load-bearing
# ON ITS OWN:
#   * DIGITS right after `HALTE borne :` — all eighteen measured quotations write
#     the literal `N` there, never a number (V3);
#   * a TAIL WINDOW — because a plan *may* quote the rule with a real number in
#     place of `N`, and quotations live 88–100 lines from the head while
#     declarations live 24–26 lines from EOF (V4).
#
# THE FAIL-SAFE IS NOT SYMMETRIC, and that asymmetry is the design. A predicate
# true by mistake would WITHHOLD an artefact downstream — a push and a draft —
# which is the very loss mika#1282 exists to prevent. So every indeterminacy
# answers "no halt": variable unset, file absent, empty, unreadable, `tail`
# failing (V9, V10, V11).
#
# THE LINE-COUNT CONVENTION used throughout, stated once because V6/V7 measure a
# boundary with it: `d` is the number of lines that FOLLOW the declaration. The
# predicate reads `tail -n 120`, so the window holds exactly `d <= 119`. V6 sits
# at d=119 (inside), V7 at d=120 (outside). The measured logs are d=24 and d=26.
#
# WITNESSES AND COUNTER-WITNESSES (Fire-Disposition of mika#3142, option (a)
# with an allowlist empty for want of an exceptable object): V1, V2, V6 and V8
# are the cases where the detector must FIRE; V3, V4, V5, V7, V9, V10 and V11 are
# the cases where it must stay SILENT. Without the first group a predicate
# answering false to everything would pass the second; without the second a
# predicate answering true to everything would pass the first. The pair is the
# attestation.
#
# It calls the REAL function out of the shipped `dispatch-lib.sh` against real
# log files. No git repository, no `gh`, no network, no cargo: the predicate is a
# pure function over a text file, so the hermeticity condition of mika#2793
# (`user.email` / `user.name` on a fixture repo) has no object here.
#
# Run: bash skills/bundled/_shared/tests/test_pilot_halt_declared.sh
# Expected: all assertions pass, exit 0.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"
CI_YML="$REPO_ROOT/.github/workflows/ci.yml"
SUITE_REL="skills/bundled/_shared/tests/test_pilot_halt_declared.sh"

# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

TMPROOT="$(mktemp -d "${TMPDIR:-/tmp}/mika-3142-suite.XXXXXX")"
LOGDIR="$TMPROOT/logs"
mkdir -p "$LOGDIR"

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

# ─────────────────────────────────────────────────────────────────────────────
# The verdict renderer
#
# THE `declare -F` GUARD IS LOAD-BEARING, and it is this suite's own negative
# control. Without it an ABSENT function makes bash answer non-zero, the
# renderer would print `no-halt`, and every counter-witness below would pass
# against a dispatch-lib carrying no predicate at all. An assertion that passes
# for the wrong reason is worse than a missing one — and "a silently inert
# detector reads exactly like a healthy one" is the class this ticket is about.
# S0 turns the sentinel into a failure rather than a word nobody compares.
# ─────────────────────────────────────────────────────────────────────────────
halt_verdict() {
    declare -F _pilot_declared_halt >/dev/null || { echo MISSING-FUNCTION; return; }
    if _pilot_declared_halt; then echo halt; else echo no-halt; fi
}

# ─────────────────────────────────────────────────────────────────────────────
# Fixtures
#
# `_mk_log <out> <total> <at> <text> [pad]` writes a log of <total> lines whose
# line <at> is <text>; every other line is filler, lengthened by <pad> when
# given (V8 needs a window heavier than a pipe buffer).
#
# The text travels through the ENVIRONMENT rather than through `awk -v`: `-v`
# interprets backslash escapes, and the measured declarations carry backticks
# and Markdown emphasis that must land in the file verbatim.
# ─────────────────────────────────────────────────────────────────────────────
_mk_log() {
    local out="$1" total="$2" at="$3" text="$4" pad="${5:-}"
    T3142_TEXT="$text" T3142_PAD="$pad" awk -v total="$total" -v at="$at" '
        BEGIN {
            text = ENVIRON["T3142_TEXT"]
            pad  = ENVIRON["T3142_PAD"]
            for (i = 1; i <= total; i++) {
                if (i == at) print text
                else printf "%d tour du pilote — bruit de transcript%s\n", i, pad
            }
        }
    ' > "$out"
}

# `at` from the number of lines that FOLLOW the declaration (the convention
# stated in the header). d=24 on a 9 654-line log is the first measured log.
_at_after() { echo $(( $1 - $2 )); }

# The two declarations as they were really written, verbatim. Single quotes:
# both carry Markdown emphasis and the first carries backticks.
DECL_379='**HALTE borne : 379 lignes ajoutées pour `310`**'
DECL_455='**HALTE borne : 455 lignes pour 450.**'
# The quotation of the rule as the eighteen plan files write it: the literal `N`
# where the number would be. This is the string a bare grep would fire on.
QUOTE_N='**HALTE borne : N lignes ajoutées pour `420`**'
# The same quotation with a real number — the case the digits anchor does NOT
# close, and which only the window closes (V4).
QUOTE_540='HALTE borne : 540 lignes pour 540'

probe() {
    _SESSION_LOG_PATH="$1"
    halt_verdict
}

echo ""
echo "===================================================================="
echo "S0 — the positive control: the predicate exists and is sourceable"
echo "===================================================================="
# Asserted FIRST and on its own: everything below is read through
# `halt_verdict`, whose missing-function sentinel would otherwise make the
# counter-witnesses pass vacuously.
assert_eq "S0a _pilot_declared_halt is defined" "1" \
    "$(declare -F _pilot_declared_halt >/dev/null && echo 1 || echo 0)"
assert_eq "S0b the tail-window constant is set" "120" "${_PILOT_HALT_TAIL_WINDOW:-unset}"
assert_eq "S0c the declaration pattern is set" "1" \
    "$([ -n "${_PILOT_HALT_DECL_RE:-}" ] && echo 1 || echo 0)"

echo ""
echo "===================================================================="
echo "AC1 — the declaration is recognised in both MEASURED forms (witnesses)"
echo "===================================================================="

# ============================================================================
# V1 (AC1): log `4492a670-…`, verbatim. 9 654 lines, the declaration 24 lines
#           from EOF. The founding case: this exact log went through the rescue
#           and produced a draft.
# ============================================================================
echo ""
echo "V1: 9 654-line log, declaration 24 lines from EOF → halt"
echo "--------------------------------------------------------"
L1="$LOGDIR/v1.log"
_mk_log "$L1" 9654 "$(_at_after 9654 24)" "$DECL_379"
assert_eq "V1a the fixture really carries the measured line" "1" \
    "$(grep -cF -- "$DECL_379" "$L1" || true)"
assert_eq "V1b ...and it is 24 lines from EOF" "$DECL_379" \
    "$(tail -n 25 "$L1" | head -n 1)"
assert_eq "V1c the predicate answers halt" "halt" "$(probe "$L1")"

# ============================================================================
# V2 (AC1): log `5813f896-…`, verbatim. 7 294 lines, declaration 26 lines from
#           EOF, and a DIFFERENT shape: a full stop, no backticks. Two measured
#           forms, so the pattern is held to both and not to one spelling.
# ============================================================================
echo ""
echo "V2: 7 294-line log, the second measured form → halt"
echo "---------------------------------------------------"
L2="$LOGDIR/v2.log"
_mk_log "$L2" 7294 "$(_at_after 7294 26)" "$DECL_455"
assert_eq "V2a the fixture carries the second measured line" "1" \
    "$(grep -cF -- "$DECL_455" "$L2" || true)"
assert_eq "V2b the predicate answers halt" "halt" "$(probe "$L2")"

echo ""
echo "===================================================================="
echo "AC2 — the QUOTATION of the rule fires nothing, by each anchor alone"
echo "===================================================================="

# ============================================================================
# V3 (AC2): the digits anchor, ISOLATED. Two placements of the same quotation:
#           the production one (head of the log, l. 100) and — the one that
#           really isolates the anchor — the same quotation INSIDE the tail
#           window, where the window cannot be what refuses it.
# ============================================================================
echo ""
echo "V3: the rule quoted with the literal N → no halt (digits anchor)"
echo "----------------------------------------------------------------"
L3="$LOGDIR/v3-head.log"
_mk_log "$L3" 9654 100 "$QUOTE_N"
assert_eq "V3a the production placement (l. 100) → no halt" "no-halt" "$(probe "$L3")"
L3B="$LOGDIR/v3-tail.log"
_mk_log "$L3B" 9654 "$(_at_after 9654 24)" "$QUOTE_N"
assert_eq "V3b the fixture puts the quotation INSIDE the window" "$QUOTE_N" \
    "$(tail -n 25 "$L3B" | head -n 1)"
assert_eq "V3c ...and it still answers no halt — the window is not what refused" \
    "no-halt" "$(probe "$L3B")"

# ============================================================================
# V4 (AC2): the window anchor, ISOLATED. The quotation carries a real NUMBER,
#           so the digits anchor matches; only the window refuses. A plan may
#           well write the rule that way, which is why this case is not
#           hypothetical.
# ============================================================================
echo ""
echo "V4: the rule quoted with a real number, at the head → no halt (window)"
echo "----------------------------------------------------------------------"
L4="$LOGDIR/v4.log"
_mk_log "$L4" 9654 100 "$QUOTE_540"
assert_eq "V4a the digits anchor DOES match this line (so only the window can refuse)" "1" \
    "$(grep -cE -- 'HALTE borne[[:space:]]*:[[:space:]]*[0-9]' "$L4" || true)"
assert_eq "V4b the predicate answers no halt" "no-halt" "$(probe "$L4")"

# ============================================================================
# V5 (AC2): a real, digit-carrying DECLARATION, but out of the window. A halt
#           declared at l. 300 of a 9 000-line log is not this flight's last
#           word; reading it as one would withhold a draft on a flight that
#           shipped.
# ============================================================================
echo ""
echo "V5: a digit-carrying declaration out of the window → no halt"
echo "------------------------------------------------------------"
L5="$LOGDIR/v5.log"
_mk_log "$L5" 9000 300 "$DECL_379"
assert_eq "V5a the predicate answers no halt" "no-halt" "$(probe "$L5")"

echo ""
echo "===================================================================="
echo "AC3 — the window is worth exactly what it declares"
echo "===================================================================="

# ============================================================================
# V6/V7 (AC3): the two sides of `_PILOT_HALT_TAIL_WINDOW=120`. `tail -n 120`
#              holds the declaration when at most 119 lines follow it.
#
#              V7 is the one that cannot be dropped: without it a predicate
#              reading the WHOLE file would pass V1–V6 and every refusal above
#              except this one.
# ============================================================================
echo ""
echo "V6: 119 lines after the declaration (inside the window) → halt"
echo "--------------------------------------------------------------"
L6="$LOGDIR/v6.log"
_mk_log "$L6" 9654 "$(_at_after 9654 119)" "$DECL_379"
assert_eq "V6a the declaration is the FIRST line of the window" "$DECL_379" \
    "$(tail -n 120 "$L6" | head -n 1)"
assert_eq "V6b the predicate answers halt" "halt" "$(probe "$L6")"

echo ""
echo "V7: 120 lines after the declaration (outside the window) → no halt"
echo "------------------------------------------------------------------"
L7="$LOGDIR/v7.log"
_mk_log "$L7" 9654 "$(_at_after 9654 120)" "$DECL_379"
assert_eq "V7a the declaration is one line ABOVE the window" "$DECL_379" \
    "$(tail -n 121 "$L7" | head -n 1)"
assert_eq "V7b the predicate answers no halt" "no-halt" "$(probe "$L7")"

echo ""
echo "===================================================================="
echo "AC4 — the predicate does not carry the SIGPIPE regression of mika#2055"
echo "===================================================================="

# ============================================================================
# V8 (AC4): the only case that distinguishes a here-string from a pipe.
#
#   `grep -q` exits at the FIRST match and closes its input. A producer still
#   writing takes SIGPIPE and exits 141; under the `set -o pipefail` this
#   library is sourced into, that 141 BECOMES the pipeline's status — and the
#   predicate would answer "no halt" for a log that carries one. The fault only
#   appears once the producer's output exceeds the pipe buffer (64 KiB), so on a
#   120-line window it depends on line LENGTH: the false negative would be
#   INTERMITTENT, which is the worst shape a detector can have.
#
#   So: a window heavier than 64 KiB whose FIRST line is the declaration (so a
#   `grep -q` would short-circuit immediately), probed 200 times. A single
#   `no-halt` among them fails the case.
# ============================================================================
echo ""
echo "V8: >64 KiB window, declaration first, 200 consecutive probes → halt ×200"
echo "-------------------------------------------------------------------------"
PAD="$(printf 'x%.0s' $(seq 1 700))"
L8="$LOGDIR/v8.log"
_mk_log "$L8" 2000 "$(_at_after 2000 119)" "$DECL_379" "$PAD"
WINDOW_BYTES="$(tail -n 120 "$L8" | wc -c)"
assert_eq "V8a the fixture's window really exceeds a pipe buffer (64 KiB)" "1" \
    "$([ "$WINDOW_BYTES" -gt 65536 ] && echo 1 || echo 0)"
assert_eq "V8b the declaration is the first line of that window" "$DECL_379" \
    "$(tail -n 120 "$L8" | head -n 1)"
V8_NO_HALT=0
for _i in $(seq 1 200); do
    [ "$(probe "$L8")" = "halt" ] || V8_NO_HALT=$((V8_NO_HALT + 1))
done
assert_eq "V8c 200 probes, zero false negative" "0" "$V8_NO_HALT"

echo ""
echo "===================================================================="
echo "AC5 — every indeterminacy answers 'no halt', and kills no shell"
echo "===================================================================="

# ============================================================================
# V9–V11 (AC5): the fail-safe direction. A predicate true by mistake would
#               withhold the push and the draft at flight 2b — the loss
#               mika#1282 exists to prevent — so an unreadable signal is never
#               a satisfied term. Same doctrine as `_pilot_had_no_shipping_tail`
#               and as mika#3038's `_tree_rc=2`.
# ============================================================================
echo ""
echo "V9: the path does not exist → no halt"
echo "-------------------------------------"
assert_eq "V9a a path that was never written" "no-halt" "$(probe "$LOGDIR/absent.log")"

echo ""
echo "V10: a 0-byte log (the pilot_log_guard.missing case, mika#2165) → no halt"
echo "------------------------------------------------------------------------"
L10="$LOGDIR/v10.log"
: > "$L10"
assert_eq "V10a the fixture is really empty" "0" "$(wc -c < "$L10")"
assert_eq "V10b the predicate answers no halt" "no-halt" "$(probe "$L10")"

echo ""
echo "V11: empty, then UNSET, under set -u → no halt and no dead shell"
echo "----------------------------------------------------------------"
# The variable is a global posed by `_run_claude_pilot` (dispatch-lib.sh:6678).
# Should a refactor make it `local`, the predicate must answer "no halt" rather
# than die on `unbound variable` and take its caller with it.
assert_eq "V11a _SESSION_LOG_PATH empty" "no-halt" "$(probe "")"
unset _SESSION_LOG_PATH
assert_eq "V11b _SESSION_LOG_PATH unset under set -u" "no-halt" "$(halt_verdict)"
assert_eq "V11c ...and the calling shell is still alive to say so" "alive" "alive"

echo ""
echo "===================================================================="
echo "S1 — the anti-regression anchor the existing lint cannot carry"
echo "===================================================================="

# `scripts/verify-no-sigpipe-grep.sh` only matches `printf`/`echo` producers
# (its own scope note says so, and the residue is tracked in mika#2983). A
# `tail | grep -q` would therefore pass UNDER that lint while carrying exactly
# the regression V8 measures. This block is where that shape is refused.
HALT_BLOCK="$(sed -n '/^_pilot_declared_halt() {/,/^}$/p' "$DISPATCH_LIB")"
echo ""
assert_eq "S1a the function body was really extracted (never an empty string)" "1" \
    "$([ -n "$HALT_BLOCK" ] && echo 1 || echo 0)"
assert_eq "S1b no pipe into a short-circuiting grep anywhere in the body" "0" \
    "$(grep -cE '\|[[:space:]]*grep[[:space:]]+-[A-Za-z]*q' <<<"$HALT_BLOCK" || true)"
assert_contains "S1c the match reads a here-string instead" '<<<"$_tail"' "$HALT_BLOCK"
assert_contains "S1d the path is passed after -- (a leading - is not an option)" \
    '-- "$_SESSION_LOG_PATH"' "$HALT_BLOCK"
assert_eq "S1e the predicate is defined exactly once" "1" \
    "$(grep -c '^_pilot_declared_halt() {' "$DISPATCH_LIB" || true)"

echo ""
echo "===================================================================="
echo "S2 — the suite is RUN by the CI, not merely present (mika#2578)"
echo "===================================================================="

# A correct predicate and a predicate the pipeline RUNS are two facts, and only
# the second one closes a ticket. Three times a suite was written, reviewed and
# merged while no workflow called it (mika#2578 twice, mika#2752 twice), which
# is why the wiring is asserted from INSIDE the suite as well as by
# `check-suite-wiring`.
#
# Scoped to the job: a step living in some other job would satisfy a bare grep
# over the file while running in a context that builds nothing of what this
# suite needs — and `dispatch-lib-offline-suites` is the job whose contract is
# "no build, no network".
JOB_BLOCK="$(awk '
    /^  dispatch-lib-offline-suites:/ { inside = 1; print; next }
    inside && /^  [A-Za-z]/ { inside = 0 }
    inside { print }
' "$CI_YML")"
echo ""
assert_eq "S2a the job block was really found in ci.yml" "1" \
    "$([ -n "$JOB_BLOCK" ] && echo 1 || echo 0)"
assert_contains "S2b the job runs this suite by its repo-relative path" \
    "bash $SUITE_REL" "$JOB_BLOCK"
assert_eq "S2c ...through a NAMED step, so a red run says what broke" "1" \
    "$(grep -c 'name: A pilot-declared halt is read from the session log' <<<"$JOB_BLOCK" || true)"
# The registry carries the RESIDUE only — suites no workflow reaches. A wired
# suite listed there would make `check-suite-wiring`'s bidirectional comparison
# go red (direction D2), so the right number of entries is zero.
assert_eq "S2d the suite is absent from scripts/suite-wiring.tsv" "0" \
    "$(grep -cF -- "$SUITE_REL" "$REPO_ROOT/scripts/suite-wiring.tsv" || true)"

echo ""
echo "===================================================================="
echo "Passed: $PASS   Failed: $FAIL"
echo "===================================================================="
[ "$FAIL" -eq 0 ] || exit 1

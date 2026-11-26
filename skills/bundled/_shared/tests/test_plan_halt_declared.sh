#!/bin/bash
# Test suite: a halt the PLAN declares is read before the GROOMED callout is
# written (mika#3118).
#
# THE DOCTRINE IS NOT REPEATED HERE. `_plan_declared_halt`'s header in
# `skills/bundled/_shared/dispatch-lib.sh` carries it in full: the two measured
# incidents, the measurement over the 1331 files of `docs/plans/`, the three
# classes of false positive the H2 anchor excludes, why there is no head window
# (the inverse of `_pilot_declared_halt`), why the grammar is NOT prescribed in
# the prompts, why the predicate has three exit codes rather than two, and why
# the match reads a here-string. This file attests it.
#
# What belongs HERE is the shape of the attestation:
#
#   WITNESSES / COUNTER-WITNESSES (Fire-Disposition option (a), allowlist empty
#   for want of an exceptable object). Must answer HALT: P1-P4. Must answer
#   NO-HALT: N1-N5. Must answer UNDETERMINED: U1-U4. Without the first group a
#   predicate answering "no halt" to everything passes the second; without the
#   second a predicate answering "halt" to everything passes the first. The
#   three groups are the attestation.
#
#   THE THREE ANSWERS ARE THREE, and that is the ticket's own discipline
#   (mika#2205): `plan_halt_declared` and `plan_halt_unreadable` are two
#   populations, so a probe that collapsed code 1 and code 2 into "not a halt"
#   would pass a predicate that fused them. `verdict_of` renders all three.
#
# Offline: the REAL function out of the shipped `dispatch-lib.sh`, against real
# plan files. No git repository, no `gh`, no network, no cargo — the predicate is
# a pure function over a text file, so the hermeticity condition of mika#2793
# (`user.email` / `user.name` on a fixture repo) has no object here.
#
# Run: bash skills/bundled/_shared/tests/test_plan_halt_declared.sh
# Expected: all assertions pass, exit 0.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"
CI_YML="$REPO_ROOT/.github/workflows/ci.yml"
MAKEFILE="$REPO_ROOT/Makefile"
SUITE_REL="skills/bundled/_shared/tests/test_plan_halt_declared.sh"

# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

TMPROOT="$(mktemp -d "${TMPDIR:-/tmp}/mika-3118-suite.XXXXXX")"
PLANDIR="$TMPROOT/plans"
mkdir -p "$PLANDIR"

PASS=0
FAIL=0
# mika#2149: a third column, so a guard that could not arm is never read as a
# bare green. Incremented only by a probe that says SKIP out loud.
SKIPPED=0

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
        # A substring expansion rather than a `| head -c`: the haystack here is a
        # whole function body, and a producer whose reader closes early is the
        # mika#2055 shape this very suite refuses downstream.
        echo "    in:      '${hay:0:600}'"
    fi
}

section() { echo ""; echo "=== $* ==="; }
case_of() { echo ""; echo "$*"; }

# The `declare -F` guard is LOAD-BEARING — it is this suite's own negative
# control. Without it an ABSENT function makes bash answer non-zero, the renderer
# prints `no-halt`, and every counter-witness below passes against a dispatch-lib
# carrying no predicate at all. "A silently inert detector reads exactly like a
# healthy one" is the class this ticket is about.
#
# THREE WORDS, NEVER TWO: `undetermined` is what the caller turns into
# `plan_halt_unreadable`, and a renderer that printed `no-halt` for it would let
# a predicate fusing the two populations pass (mika#2205).
verdict_of() {
    declare -F _plan_declared_halt >/dev/null || { echo MISSING-FUNCTION; return; }
    _plan_declared_halt "$@"
    case "$?" in
        0) echo halt ;;
        1) echo no-halt ;;
        2) echo undetermined ;;
        *) echo UNEXPECTED-CODE ;;
    esac
}

# `_mk_plan <name> <at> <text>` writes a 120-line plan whose line <at> is <text>;
# every other line is plan-shaped filler. 120 because the measured false positive
# that matters most — the quotation of rule mika#2693 — sits at l. 88-100 of every
# groomed plan, and a fixture too short to hold it at its real line would write the
# text nowhere. The text travels through the ENVIRONMENT rather than through
# `awk -v`, which interprets backslash escapes: the measured headings carry
# em-dashes and the quotations carry backticks that must land verbatim. Prints the
# path.
#
# `at` out of range says so ON STDERR and produces no file, rather than yielding a
# fixture silently missing its payload: that mistake happened while writing this
# suite (N2 asked for l. 95 of a 60-line body), and a probe whose witness is absent
# answers "no halt" for the most reassuring of reasons. It cannot `exit` the suite
# from inside a command substitution, which is why every witness below also asserts
# its fixture really carries what it claims.
#
# Two `local` statements and not one: bash expands every word of a `local`
# command BEFORE it performs any of its assignments, so `out="$PLANDIR/$_name"`
# on the first line would read `_name` while it is still unset and die on `set -u`.
_mk_plan() {
    local _name="$1" at="$2" text="$3"
    local out="$PLANDIR/$_name"
    if [ "$at" -lt 1 ] || [ "$at" -gt 120 ]; then
        echo "FIXTURE BUG: _mk_plan $_name — line $at is outside the 120-line body" >&2
        exit 2
    fi
    T3118_TEXT="$text" awk -v at="$at" '
        BEGIN {
            text = ENVIRON["T3118_TEXT"]
            for (i = 1; i <= 120; i++) {
                if (i == at) print text
                else printf "ligne %d du plan — prose de grooming ordinaire\n", i
            }
        }
    ' > "$out"
    echo "$out"
}

# The two declarations as they were really written, verbatim. Single quotes: both
# carry an em-dash, and the surrounding prose is the operator's.
DECL_BORNE='## HALTE — la borne du groom est dépassée'
DECL_BORNE_PERIMETRE='## HALTE BORNE — le périmètre ne tient pas, et ce plan ne le recoupe pas'
# The three measured false positives, verbatim from the files the plan names.
FP_PROSE_BOLD='**HALTE 1 —** le périmètre du vol 1 ne couvre pas cette moitié'
FP_RULE_QUOTE='HALTE borne : 312 lignes ajoutées, pour une borne du groom de 300.'
FP_H3='### HALTE — un titre de niveau 3, hors de portée de l ancre H2'

section "S0 — the positive control: the predicate exists and is sourceable"
# Asserted FIRST and on its own: everything below is read through `verdict_of`,
# whose missing-function sentinel would otherwise make the counter-witnesses pass
# vacuously.
assert_eq "S0a _plan_declared_halt is defined" "1" \
    "$(declare -F _plan_declared_halt >/dev/null && echo 1 || echo 0)"
assert_eq "S0b the declaration pattern is set" "1" \
    "$([ -n "${_PLAN_HALT_DECL_RE:-}" ] && echo 1 || echo 0)"
assert_eq "S0c ...and it is anchored at line start on an H2" "1" \
    "$([ "${_PLAN_HALT_DECL_RE:-}" = '^##[[:space:]]*HALTE' ] && echo 1 || echo 0)"

section "AC1/AC2 — both MEASURED forms are matched (witnesses)"

# P1: the plan of 19/11, verbatim and at its real line. This exact plan converged
# on GROOMED, the callout was written, and the auto-fire launched pilot 137111d7:
# 1,34 $ and zero commits.
case_of "P1: the 19/11 heading at l. 11 → halt"
P1="$(_mk_plan p1.md 11 "$DECL_BORNE")"
assert_eq "P1a the fixture really carries the measured heading at l. 11" "$DECL_BORNE" \
    "$(sed -n '11p' "$P1")"
assert_eq "P1b the predicate answers halt" "halt" "$(verdict_of "$P1")"

# P2: the plan of 17/11, a DIFFERENT shape (a second word before the em-dash), so
# the pattern is held to both measured forms and not to one spelling. That one
# cost a pilot killed by hand around 13:04Z.
case_of "P2: the 17/11 heading at l. 9, with its second word → halt"
P2="$(_mk_plan p2.md 9 "$DECL_BORNE_PERIMETRE")"
assert_eq "P2a the fixture carries the second measured heading at l. 9" "$DECL_BORNE_PERIMETRE" \
    "$(sed -n '9p' "$P2")"
assert_eq "P2b the predicate answers halt" "halt" "$(verdict_of "$P2")"

# P3: the spacing the anchor tolerates. A groomer writing `##HALTE` or `##   HALTE`
# has declared the same thing, and `[[:space:]]*` is what says so.
case_of "P3: the spacing variants of the H2 anchor → halt"
assert_eq "P3a no space after the hashes" "halt" \
    "$(verdict_of "$(_mk_plan p3a.md 11 '##HALTE — la borne est dépassée')")"
assert_eq "P3b three spaces after the hashes" "halt" \
    "$(verdict_of "$(_mk_plan p3b.md 11 '##   HALTE — la borne est dépassée')")"

# P4: THE DECISION THAT THERE IS NO HEAD WINDOW, made executable. A heading at
# l. 48 of a 60-line plan is a TRUE positive: `_pilot_declared_halt`'s 120-line
# window exists because the rule is QUOTED near the head of every session log,
# and a plan has no such quotation at a fixed distance. Adding a window here
# could only manufacture false negatives in the expensive direction.
case_of "P4: the heading in the MIDDLE of the file → halt (no head window)"
P4="$(_mk_plan p4.md 70 "$DECL_BORNE")"
assert_eq "P4a the fixture puts the heading at l. 70 of 120" "$DECL_BORNE" \
    "$(sed -n '70p' "$P4")"
assert_eq "P4b the predicate answers halt all the same" "halt" "$(verdict_of "$P4")"

section "AC3 — the three MEASURED false positives fire nothing (counter-witnesses)"

# N1: bold prose, from plans 2420 and 2518 (l. 572-700). `**HALTE 1 —**` through
# `**HALTE 4 —**`: a plan enumerating halts it DISCUSSES, not one it declares.
case_of "N1: '**HALTE 1 —**' in bold prose → no halt"
assert_eq "N1a the measured bold form" "no-halt" \
    "$(verdict_of "$(_mk_plan n1.md 30 "$FP_PROSE_BOLD")")"

# N2: the rule of mika#2693 QUOTED — 45+ occurrences across `docs/plans/`, each
# at l. 88-100, and the single most likely false positive since every groomed
# plan quotes it. A bare `grep "HALTE"` would fire on every one of them.
case_of "N2: the mika#2693 rule quoted with a real number → no halt"
N2="$(_mk_plan n2.md 95 "$FP_RULE_QUOTE")"
assert_eq "N2a the fixture really carries the quotation" "1" \
    "$(grep -cF -- "$FP_RULE_QUOTE" "$N2" || true)"
assert_eq "N2b ...and it is not at line start under two hashes" "0" \
    "$(grep -cE '^##[[:space:]]*HALTE' "$N2" || true)"
assert_eq "N2c the predicate answers no halt" "no-halt" "$(verdict_of "$N2")"

# N3: an H3 heading. This is the RESIDUAL RISK the plan names at its § Risques
# and does NOT resolve — pinned here in its direction rather than left to be
# discovered: the day the grammar is widened to H3, this assertion is the one
# that must be rewritten deliberately.
case_of "N3: an H3 heading → no halt (the named residual risk, pinned)"
assert_eq "N3a a '### HALTE' heading is not matched" "no-halt" \
    "$(verdict_of "$(_mk_plan n3.md 11 "$FP_H3")")"

# N4: the word in ordinary prose, mid-sentence. The form a retrospective plan
# uses when it documents this very mechanism.
case_of "N4: 'HALTE' inside a sentence → no halt"
assert_eq "N4a prose that merely mentions the word" "no-halt" \
    "$(verdict_of "$(_mk_plan n4.md 25 'La porte route vers une HALTE quand le plan en déclare une.')")"

# N5: the whole point of the negative half — an ORDINARY plan, carrying no
# occurrence at all. Without it a predicate answering "halt" to everything would
# still pass N1-N4 if they happened to share a flaw.
case_of "N5: an ordinary plan with no occurrence → no halt"
N5="$(_mk_plan n5.md 1 '# Un plan ordinaire')"
assert_eq "N5a the fixture carries no occurrence of the word" "0" \
    "$(grep -c 'HALTE' "$N5" || true)"
assert_eq "N5b the predicate answers no halt" "no-halt" "$(verdict_of "$N5")"

section "AC4 — every indeterminacy answers UNDETERMINED, never 'halt' nor 'no halt'"

# U1-U4: the fail-safe direction, and it is the INVERSE of mika#3142's. There a
# false positive WITHHOLDS an artefact (the loss mika#1282 exists to prevent), so
# every indeterminacy answers "no halt". Here a false NEGATIVE launches a flight
# on a halted plan — 1,34 $ and zero commits on 19/11 — so indeterminacy leans
# towards the halt. It leans there under its OWN code: the caller writes
# `plan_halt_unreadable`, and never `plan_halt_declared` (mika#2205).
case_of "U1: no argument at all → undetermined"
assert_eq "U1a the predicate called with no argument" "undetermined" "$(verdict_of)"
assert_eq "U1b ...and with an empty one" "undetermined" "$(verdict_of "")"

case_of "U2: a path that was never written → undetermined"
assert_eq "U2a an absent file" "undetermined" "$(verdict_of "$PLANDIR/absent.md")"
assert_eq "U2b a directory rather than a file" "undetermined" "$(verdict_of "$PLANDIR")"

case_of "U3: a file present but not readable → undetermined"
U3="$(_mk_plan u3.md 11 "$DECL_BORNE")"
chmod 000 "$U3" 2>/dev/null || true
if [ "$(id -u)" -eq 0 ] || [ -r "$U3" ]; then
    # Root bypasses the permission bits, so the fixture cannot arm. Said OUT
    # LOUD rather than counted as a pass: a guard that could not arm is not a
    # green one (mika#2149). CI runs this suite unprivileged.
    SKIPPED=$((SKIPPED + 1))
    echo "  ⊘ SKIP U3a chmod 000 does not deny this user (uid $(id -u)) — the fixture cannot arm"
else
    assert_eq "U3a an unreadable file answers undetermined, NOT halt" "undetermined" \
        "$(verdict_of "$U3")"
fi
chmod 644 "$U3" 2>/dev/null || true
assert_eq "U3b ...and the same file, readable again, answers halt" "halt" "$(verdict_of "$U3")"

# U4: a file that reads EMPTY. `_find_issue_plan` filters at >500 bytes, so a
# plan it returned and that reads empty is a READ anomaly, not a negative answer
# — hence code 2 and not code 1. This is the one place the two are told apart on
# content rather than on access.
case_of "U4: a 0-byte file → undetermined, not 'no halt'"
U4="$PLANDIR/u4.md"
: > "$U4"
assert_eq "U4a the fixture is really empty" "0" "$(wc -c < "$U4")"
assert_eq "U4b the predicate answers undetermined" "undetermined" "$(verdict_of "$U4")"

section "AC6 — the predicate is PURE: one argument, no global read"

# A predicate that read `$WORKTREE_DIR` or `$ISSUE_NUM` would answer differently
# inside and outside the groom loop, and the suite would attest a function the
# production path does not run. Asserted twice over: by execution with every
# candidate global unset under `set -u`, and by a scan of the body.
case_of "S1: the same answer with every groom global unset, under set -u"
S1_PLAN="$(_mk_plan s1.md 11 "$DECL_BORNE")"
S1_CLEAN="$(
    unset WORKTREE_DIR ISSUE_NUM REPO BRANCH _SESSION_LOG_PATH GROOM_CALLOUT_PLAN 2>/dev/null
    set -u
    _plan_declared_halt "$S1_PLAN" && echo halt || echo other
)"
assert_eq "S1a halt is still the answer, and no shell died on an unbound variable" \
    "halt" "$S1_CLEAN"
assert_eq "S1b ...and the calling shell is still alive to say so" "alive" "alive"

# The anti-regression anchor the existing lint cannot carry:
# `scripts/verify-no-sigpipe-grep.sh` only matches `printf`/`echo` producers, so a
# `cat | grep -q` passes UNDER that lint while carrying the mika#2055 regression
# — intermittently, since on a plan file exceeding the 64 KiB pipe buffer depends
# on file size. This block is where that shape is refused.
case_of "S2: the body's shape — a here-string, one argument, no global"
HALT_BLOCK="$(sed -n '/^_plan_declared_halt() {/,/^}$/p' "$DISPATCH_LIB")"
assert_eq "S2a the function body was really extracted (never an empty string)" "1" \
    "$([ -n "$HALT_BLOCK" ] && echo 1 || echo 0)"
assert_eq "S2b no pipe into a short-circuiting grep anywhere in the body" "0" \
    "$(grep -cE '\|[[:space:]]*grep[[:space:]]+-[A-Za-z]*q' <<<"$HALT_BLOCK" || true)"
assert_contains "S2c the match reads a here-string instead" '<<<"$_body"' "$HALT_BLOCK"
assert_contains "S2d the path is passed after -- (a leading - is not an option)" \
    'cat -- "$_path"' "$HALT_BLOCK"
assert_eq "S2e the predicate is defined exactly once" "1" \
    "$(grep -c '^_plan_declared_halt() {' "$DISPATCH_LIB" || true)"
assert_eq "S2f the body reads \$1 and nothing else from outside" "1" \
    "$(grep -c 'local _path="\${1:-}" _body' <<<"$HALT_BLOCK" || true)"
# The three codes, each returned from the body. A body that lost `return 2` would
# collapse the two populations mika#2205 exists to keep apart, and every U case
# above would still pass if the renderer mapped the collapse to `no-halt`.
assert_eq "S2g the body returns 2 on indeterminacy, in five places" "5" \
    "$(grep -cE 'return 2$' <<<"$HALT_BLOCK" || true)"
assert_eq "S2h ...and the wire pattern has a single definition site" "1" \
    "$(grep -cE '^_PLAN_HALT_DECL_RE=' "$DISPATCH_LIB" || true)"

section "S3 — the suite is RUN by the CI, not merely present (mika#2578)"

# A correct predicate and a predicate the pipeline RUNS are two facts, and only
# the second closes a ticket: a suite was written, reviewed and merged while no
# workflow called it four times (mika#2578 once, mika#2752 twice). Scoped to the
# JOB, because a step living in another job would satisfy a bare grep over the
# file while running in a context that builds nothing of what this suite needs —
# and `dispatch-lib-offline-suites` is the job whose contract is "no build, no
# network".
JOB_BLOCK="$(awk '
    /^  dispatch-lib-offline-suites:/ { inside = 1; print; next }
    inside && /^  [A-Za-z]/ { inside = 0 }
    inside { print }
' "$CI_YML")"
echo ""
assert_eq "S3a the job block was really found in ci.yml" "1" \
    "$([ -n "$JOB_BLOCK" ] && echo 1 || echo 0)"
assert_contains "S3b the job runs this suite by its repo-relative path" \
    "bash $SUITE_REL" "$JOB_BLOCK"
assert_eq "S3c ...through a NAMED step, so a red run says what broke" "1" \
    "$(grep -c 'name: A plan-declared halt does not converge on GROOMED' <<<"$JOB_BLOCK" || true)"
assert_eq "S3d a Makefile target names the suite too" "1" \
    "$(grep -c "bash $SUITE_REL" "$MAKEFILE" || true)"
# The registry carries the RESIDUE only — suites no workflow reaches. A wired
# suite listed there would make `check-suite-wiring`'s bidirectional comparison go
# red (direction D2), so the right number of entries is zero.
assert_eq "S3e the suite is absent from scripts/suite-wiring.tsv" "0" \
    "$(grep -cF -- "$SUITE_REL" "$REPO_ROOT/scripts/suite-wiring.tsv" || true)"

echo ""
echo "Passed: $PASS   Failed: $FAIL   SKIPPED: $SKIPPED"
[ "$FAIL" -eq 0 ] || exit 1

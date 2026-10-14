#!/usr/bin/env bash
# CI lint: `--ignored` and `--include-ignored` may never appear at the SAME
# cargo-test invocation — libtest refuses them together (mika#2683, volet b).
#
# ─────────────────────────────────────────────────────────────────────────────
# WHAT THIS EXISTS TO PREVENT, MEASURED
#
# `.github/workflows/eval-calibration.yml` ran the eval suite with BOTH
# `--ignored` and `--include-ignored` after the `--` separator, and swallowed
# the result with `|| true`, from its creation (mika#742, 2026-06-22) to
# 2026-10-12. libtest answers
#
#     error: the options --include-ignored and --ignored are mutually exclusive
#
# and exits non-zero; `|| true` hid it, the next step found no artifact and
# failed. 17 runs out of 17 red, and nobody read the cause, because the step
# that was wrong was not the step that failed. Each Monday the resulting red
# `check_suite` on `main` also cost mika-dev an LLM turn — volets (a) and (c)
# of the same ticket.
#
# The defect is not "a bad flag". It is a flag pair a human eye reads as
# cumulative and the tool reads as contradictory. That shape does not announce
# itself at review, so it gets a structural guard.
#
# ─────────────────────────────────────────────────────────────────────────────
# DO NOT "TIGHTEN" THIS TOWARDS THE NAIVE PREDICATE
#
# The subject is the **co-occurrence at one invocation site**. It is NEVER the
# presence of `--include-ignored`, which is perfectly legitimate on its own
# (the whole suite plus the ignored ones), and never the presence of
# `--ignored`, which this repository writes at three invocation sites on
# purpose (`.github/workflows/eval-calibration.yml`, `.github/workflows/ci.yml`,
# `scripts/generate-openapi.sh`).
#
# A rule reading "no `--include-ignored` under `.github/`" would accuse every
# legitimate site the day someone writes one, so it gets disarmed or
# allowlisted — and then the pair comes back unnoticed. The whole value of this
# guard is that it has nothing to say about a correct invocation.
#
# Reasoning and the measured run history:
# docs/plans/2026-10-14-001-fix-2683-sorties-rien-a-faire-du-ci-failure-plan.md
#
# ─────────────────────────────────────────────────────────────────────────────
# THE TWO RULES
#
#   R1 — co-occurrence at one invocation site.
#        Inside the perimeter, a live line that invokes cargo's test command
#        with a `--` separator may not carry both `--ignored` and
#        `--include-ignored`.
#
#   R2 — anti-vacuity floor.
#        A run that read no file, or read files where no cargo-test invocation
#        with a `--` separator exists, or where the `--ignored` family appears
#        nowhere, is a VACUOUS pass and is refused. A guard that found nobody
#        must not read like a guard that found nothing wrong (mika#2103 /
#        mika#2205). The three floors are asserted on counters this scan itself
#        produced — a floor on anything else can be satisfied by a scan that
#        never happened.
#
# PERIMETER: the places that actually RUN cargo — workflow YAML, the Makefile,
# `scripts/*.sh`. Prose in `docs/` and the `CLAUDE.md` files is deliberately
# out: documentation showing both forms, on one line or two, executes nothing.
# That is a declared gap, not an oversight — if a doc line ever has to be
# covered, widen PERIMETER rather than weaken R1.
#
# Comment lines are skipped (`#`, identically in YAML, shell and make). A
# mention is not an instruction, and this very file would otherwise denounce
# its own header.
#
# KNOWN GAP, stated rather than discovered: this is a line-oriented scan, so an
# invocation split across lines with a backslash continuation escapes R1. Zero
# occurrences today. Covering it means joining continuations before matching —
# do that rather than loosening the predicate.
#
# SELF-REFERENCE: `scripts/test-check-eval-ignored-flags.sh` is inside this
# perimeter and must build faulty fixtures. It therefore composes the pair at
# runtime from two halves, so no live line of it carries both literals. Keep
# that shape; do not allowlist the harness instead — a file-level exemption
# would also hide a real regression written in that file.
#
# ALLOWLIST: `scripts/eval-ignored-flags-allowlist.txt`, compared **in both
# directions** (an entry matching no violation fails the build, exactly like an
# unlisted violation). Shipped empty, and expected to stay empty: libtest
# refuses the pair, so there is no such thing as a legitimate co-occurrence and
# an entry here could only ever be a defect someone parked. It exists so that
# the refusal is said out loud rather than merely absent.
#
# Usage: check-eval-ignored-flags.sh [scan-root]
#   The optional scan root lets the negative-behaviour harness
#   (scripts/test-check-eval-ignored-flags.sh) point the guard at a fixture
#   tree, on the mika#2103 / mika#2309 model.
#
# Exit 0 if clean, 1 with actionable errors otherwise.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCAN_ROOT="${1:-$REPO_ROOT}"

# ── The perimeter: what runs cargo. See the header for what is out, and why.
PERIMETER=(
    ".github/workflows"   # every workflow with a test step
    "Makefile"            # the developer-facing targets
    "scripts"             # the *.sh CI and operators invoke
)

ALLOWLIST_FILE="$SCAN_ROOT/scripts/eval-ignored-flags-allowlist.txt"

# The pair, composed from a shared stem so NEITHER literal appears on a live
# line of this file — see SELF-REFERENCE above, same discipline as the harness.
#
# This is not a flourish. Written literally, `FLAG_BARE="--ignored"` would make
# this file satisfy R2's third floor all by itself, and a fixture tree holding
# nothing but a copy of the guard would read as a real scan. The floors have to
# be fed by the tree, not by the instrument.
#
# The messages below interpolate these variables, so the OUTPUT names both
# flags in full even though the source never does.
IGN="ignored"
FLAG_BARE="--$IGN"
FLAG_INCLUDE="--include-$IGN"

VIOLATIONS=0

fail() {
    echo "ERROR: $*"
    VIOLATIONS=$((VIOLATIONS + 1))
}

# ── Collect the perimeter's files, as paths relative to SCAN_ROOT.
#
# A perimeter entry that does not exist is skipped — a fixture tree carries one
# of them, not all three. A run where NOTHING exists is refused by R2.
collect_files() {
    local entry abs
    for entry in "${PERIMETER[@]}"; do
        abs="$SCAN_ROOT/$entry"
        if [[ -d "$abs" ]]; then
            find "$abs" -type f \( -name '*.yml' -o -name '*.yaml' -o -name '*.sh' \) \
                | sed "s|^$SCAN_ROOT/||"
        elif [[ -f "$abs" ]]; then
            echo "$entry"
        fi
    done
}

FILES="$(collect_files)"

# ── R2, first floor: a scan with nothing to scan is a vacuous pass, not a
# clean one. Refused here rather than reported as success.
if [[ -z "${FILES//[[:space:]]/}" ]]; then
    fail "the perimeter matched no file under '$SCAN_ROOT' — nothing was scanned."
    echo "       Perimeter: ${PERIMETER[*]}"
    echo "       A scan with nothing to scan is a vacuous pass, not a clean one."
    echo ""
    echo "FAILED: $VIOLATIONS violation(s) — see above."
    exit 1
fi

FILES_READ=0
INVOCATIONS_SEEN=0
IGNORED_FAMILY_SEEN=0
R1_HITS=()

while IFS= read -r rel; do
    [[ -n "$rel" ]] || continue
    FILES_READ=$((FILES_READ + 1))

    lineno=0
    while IFS= read -r content || [[ -n "$content" ]]; do
        lineno=$((lineno + 1))

        # A comment executes nothing.
        trimmed="${content#"${content%%[![:space:]]*}"}"
        [[ "$trimmed" == \#* ]] && continue

        if [[ "$content" == *"$FLAG_BARE"* || "$content" == *"$FLAG_INCLUDE"* ]]; then
            IGNORED_FAMILY_SEEN=$((IGNORED_FAMILY_SEEN + 1))
        fi

        # An invocation site: cargo's test command with a `--` separator, the
        # only shape that can carry a libtest flag at all.
        [[ "$content" == *"cargo test"* ]] || continue
        [[ "$content" == *" -- "* ]] || continue
        INVOCATIONS_SEEN=$((INVOCATIONS_SEEN + 1))

        # ── Rule R1 ──────────────────────────────────────────────────────────
        #
        # `--include-ignored` does not contain `--ignored` as a substring, so
        # two plain substring tests are exact here and no word-boundary
        # machinery is needed.
        [[ "$content" == *"$FLAG_INCLUDE"* ]] || continue
        [[ "$content" == *"$FLAG_BARE"* ]] || continue

        R1_HITS+=("$rel:$lineno")
    done < "$SCAN_ROOT/$rel"
done <<< "$FILES"

# ── R2, remaining floors. Both are satisfied by a single one-line fixture, so
# the negative-behaviour harness stays small.
if [[ $FILES_READ -eq 0 ]]; then
    fail "no file was read under '$SCAN_ROOT' — the scan is vacuous."
fi

if [[ $INVOCATIONS_SEEN -eq 0 ]]; then
    fail "no cargo-test invocation with a '--' separator was found under '$SCAN_ROOT'."
    echo "       R1 can only fire at such a site. Finding none means the scan"
    echo "       looked in the wrong place, not that the tree is clean."
fi

if [[ $IGNORED_FAMILY_SEEN -eq 0 ]]; then
    fail "neither $FLAG_BARE nor $FLAG_INCLUDE appears anywhere under '$SCAN_ROOT'."
    echo "       This repository writes the first at three invocation sites on"
    echo "       purpose. Total absence means the perimeter stopped matching"
    echo "       reality — a tree where R1 has no subject cannot clear it."
fi

# ── The allowlist, compared in both directions.
ALLOW_ENTRIES=()
if [[ -f "$ALLOWLIST_FILE" ]]; then
    while IFS= read -r line || [[ -n "$line" ]]; do
        line="${line%%#*}"
        line="${line#"${line%%[![:space:]]*}"}"
        line="${line%"${line##*[![:space:]]}"}"
        [[ -n "$line" ]] || continue
        ALLOW_ENTRIES+=("$line")
    done < "$ALLOWLIST_FILE"
fi

ALLOW_USED=()
for hit in ${R1_HITS[@]+"${R1_HITS[@]}"}; do
    matched=""
    for entry in ${ALLOW_ENTRIES[@]+"${ALLOW_ENTRIES[@]}"}; do
        if [[ "$hit" == "$entry" ]]; then
            matched="$entry"
            break
        fi
    done
    if [[ -n "$matched" ]]; then
        ALLOW_USED+=("$matched")
        continue
    fi
    fail "$hit carries both $FLAG_BARE and $FLAG_INCLUDE at one invocation site."
    echo "       libtest refuses them together — the invocation fails before any"
    echo "       test runs, and an '|| true' on that line hides it entirely."
    echo "       Keep exactly one: $FLAG_BARE to run ONLY the ignored tests,"
    echo "       $FLAG_INCLUDE to run the whole suite PLUS them."
done

for entry in ${ALLOW_ENTRIES[@]+"${ALLOW_ENTRIES[@]}"}; do
    used=""
    for u in ${ALLOW_USED[@]+"${ALLOW_USED[@]}"}; do
        [[ "$u" == "$entry" ]] && used="yes" && break
    done
    if [[ -z "$used" ]]; then
        fail "allowlist entry '$entry' matches no R1 violation — remove it."
        echo "       An allowlist nobody prunes becomes the junk drawer in which"
        echo "       the regression this guard exists to catch passes unnoticed."
    fi
done

if [[ $VIOLATIONS -gt 0 ]]; then
    echo ""
    echo "FAILED: $VIOLATIONS violation(s) — see above."
    exit 1
fi

echo "OK: no cargo-test invocation carries both $FLAG_BARE and $FLAG_INCLUDE."
echo "    ($FILES_READ file(s) read, $INVOCATIONS_SEEN invocation site(s) seen," \
     "$IGNORED_FAMILY_SEEN line(s) mentioning the family.)"
exit 0

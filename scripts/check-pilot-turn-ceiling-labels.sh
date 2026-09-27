#!/usr/bin/env bash
# CI lint: every label that raises the pilot turn ceiling must be declared in
# `.github/labels.yml`.
#
# THE PROPERTY: one list must be a SUBSET of another.
#   `PILOT_LABEL_TURN_CEILINGS` (skills/bundled/_shared/dispatch-lib.sh) maps a
#   ticket label to the `--max-turns` ceiling its dispatch receives (mika#2542:
#   `loop-substrate=200`). `.github/labels.yml` is the vocabulary GitHub is
#   allowed to carry — `.github/workflows/labels.yml` runs EndBug/label-sync
#   with `delete-other-labels: true`, so a label absent from that file is
#   DELETED from the repository on the next sync, and from every issue carrying
#   it, without a single `unlabeled` event.
#
# WHY THE FAILURE IS SILENT, which is why a guard and not a note.
#   When a ceiling label disappears, `_pilot_label_turn_ceiling` matches
#   nothing, the dispatch falls back to the fleet default, and the class the
#   label exists for goes back to being truncated at 151 turns. The pilot line
#   reads `source=env` or `source=default` — exactly what a fleet with no
#   substrate ticket in flight would read. No log, no audit event, no red test.
#   That is the class named in
#   docs/solutions/best-practices/un-label-denforcement-non-declare-echoue-en-silence-2026-09-01.md
#   and it was LIVE when mika#2542 was written: `loop-substrate` existed on the
#   repository and was declared nowhere.
#
# WHY ONE DIRECTION ONLY.
#   check-dispatch-seats-declared.sh (mika#2092), which this script is modelled
#   on, is bidirectional because its two lists are the SAME vocabulary. Here one
#   is a subset of the other: labels.yml carries dozens of labels that are not
#   turn ceilings, and reading their absence from the table as drift would fail
#   every clean tree.
#
# WHEN YOU ADD A CEILING, declare the label in the same commit. "On déclare, on
# n'allowliste pas" (mika#2201): there is no exemption list here, and the fix
# for a red run is a `- name:` entry, never an exception.
#
# Usage: check-pilot-turn-ceiling-labels.sh [dispatch-lib.sh] [labels.yml]
#   Both default to the repo's real files. Explicit arguments let the
#   anti-vacuity harness point the guard at fixtures (see
#   scripts/test-check-pilot-turn-ceiling-labels.sh).
#
# Exit 0 clean, 1 on an undeclared ceiling label, 2 if a file cannot be read,
# 3 if either list could not be parsed at all, or the table holds a form the
# parser cannot audit (a guard that finds nothing to check, or cannot see
# everything, must say so rather than pass).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DISPATCH_LIB="${1:-$REPO_ROOT/skills/bundled/_shared/dispatch-lib.sh}"
LABELS_YML="${2:-$REPO_ROOT/.github/labels.yml}"

if [[ ! -f "$DISPATCH_LIB" ]]; then
    echo "ERROR: ceiling table source not readable: $DISPATCH_LIB" >&2
    exit 2
fi
if [[ ! -f "$LABELS_YML" ]]; then
    echo "ERROR: label declaration file not readable: $LABELS_YML" >&2
    exit 2
fi

# ── Side A: the labels the ceiling table knows.
#
# Anchored on the array ASSIGNMENT in column zero (`PILOT_LABEL_TURN_CEILINGS=(`),
# never on the bare name — the file also iterates the table
# (`"${PILOT_LABEL_TURN_CEILINGS[@]}"`) and names it in comments. Entries are
# read until the closing `)` line, one `"label=ceiling"` per line; comments and
# blank lines inside the array are skipped. The file is parsed, never executed:
# sourcing dispatch-lib.sh would run its top-level setup.
#
# The parser is deliberately narrow, so it must REFUSE what it cannot model
# rather than return a partial list (the class of
# docs/solutions/best-practices/structural-guard-fails-open-parser-fixture-harness.md,
# mika#2039). Bash accepts an unquoted, single-quoted or multi-per-line entry,
# and an append (`PILOT_LABEL_TURN_CEILINGS+=(...)`) anywhere in the file; the
# resolver iterates the real array and honours every one of them. A subset
# check that silently skipped them would go green on exactly the undeclared
# label it exists to catch. So:
#   - every non-blank, non-comment line inside the array must be exactly one
#     `"label=<positive integer>"` entry (optionally followed by a comment) —
#     which also refuses `"x="` or `"x=abc"`, values the resolver would pass
#     straight to `--max-turns`;
#   - the file must contain exactly one assignment to the table, counting `+=`
#     and indented / `declare` / `readonly` forms.
#
# awk exits 3 when no declaration is found, 4 when a declaration is found but
# never closed, 5 on an entry line of any other shape, 6 when the table is
# assigned more than once — all reported as exit 3 below.
extract_ceiling_labels() {
    awk '
        # Any assignment to the table, whatever its prefix. Comment lines are
        # excluded; the `[@]` use site does not match (`{` precedes the name).
        $0 !~ /^[ \t]*#/ && $0 ~ /(^|[ \t;])PILOT_LABEL_TURN_CEILINGS(\+)?=/ {
            assignments++
            if (assignments > 1) {
                printf "%d: %s\n", NR, $0 > "/dev/stderr"
                bad_assign = 1
            }
        }
        /^PILOT_LABEL_TURN_CEILINGS=\(/ { collecting = 1; found = 1; next }
        collecting {
            line = $0
            if (line ~ /^[ \t]*\)/) { collecting = 0; closed = 1; next }
            sub(/^[ \t]+/, "", line)
            if (line == "" || line ~ /^#/) { next }
            if (line !~ /^"[^"=]+=[1-9][0-9]*"[ \t]*(#.*)?$/) {
                printf "%d: %s\n", NR, $0 > "/dev/stderr"
                bad_entry = 1
                next
            }
            split(line, parts, "\"")
            v = parts[2]
            sub(/=.*$/, "", v)
            print v
        }
        END {
            if (!found) { exit 3 }
            if (!closed) { exit 4 }
            if (bad_entry) { exit 5 }
            if (bad_assign) { exit 6 }
        }
    ' "$1"
}

# ── Side B: every label name GitHub is allowed to carry.
extract_declared_labels() {
    awk '
        /^[ \t]*-[ \t]*name:[ \t]*/ {
            any_name = 1
            v = $0
            sub(/^[ \t]*-[ \t]*name:[ \t]*/, "", v)

            # Quoted forms first: the closing quote ends the value, so a `#`
            # inside it is content and not a comment.
            if (v ~ /^"/)      { sub(/^"/, "", v); sub(/".*$/, "", v) }
            else if (v ~ /^'"'"'/) { sub(/^'"'"'/, "", v); sub(/'"'"'.*$/, "", v) }
            else               { sub(/[ \t]+#.*$/, "", v); sub(/[ \t]+$/, "", v) }

            if (v != "") { print v }
        }
        END { if (!any_name) { exit 3 } }
    ' "$1"
}

CEILINGS=""
awk_status=0
CEILINGS="$(extract_ceiling_labels "$DISPATCH_LIB")" || awk_status=$?

if [[ $awk_status -eq 3 ]]; then
    echo "ERROR: no \`PILOT_LABEL_TURN_CEILINGS=(\` declaration found in $DISPATCH_LIB" >&2
    echo "This guard has nothing to check, which is not the same as agreement." >&2
    exit 3
elif [[ $awk_status -eq 4 ]]; then
    echo "ERROR: \`PILOT_LABEL_TURN_CEILINGS=(\` was found in $DISPATCH_LIB but never closed." >&2
    echo "Expected one \"label=ceiling\" entry per line, then a line starting with \`)\`." >&2
    exit 3
elif [[ $awk_status -eq 5 ]]; then
    echo "ERROR: \`PILOT_LABEL_TURN_CEILINGS\` in $DISPATCH_LIB holds an entry this guard cannot audit (line(s) above)." >&2
    echo "Expected exactly one double-quoted \"label=<positive integer>\" per line. Bash would" >&2
    echo "accept other shapes and the resolver would honour them, unchecked — so they are refused." >&2
    exit 3
elif [[ $awk_status -eq 6 ]]; then
    echo "ERROR: \`PILOT_LABEL_TURN_CEILINGS\` is assigned more than once in $DISPATCH_LIB (line(s) above)." >&2
    echo "An append (\`+=\`) or a second assignment adds entries this guard never reads." >&2
    echo "Keep every ceiling inside the single \`PILOT_LABEL_TURN_CEILINGS=(\` literal." >&2
    exit 3
elif [[ $awk_status -ne 0 ]]; then
    echo "ERROR: could not parse $DISPATCH_LIB (awk exit $awk_status)" >&2
    exit 2
fi

if [[ -z "$CEILINGS" ]]; then
    echo "ERROR: \`PILOT_LABEL_TURN_CEILINGS\` in $DISPATCH_LIB has no parseable entry." >&2
    echo "Expected the shape: \"loop-substrate=200\" — one per line. An empty table" >&2
    echo "raises no ceiling; if that is intended, delete the table and this guard." >&2
    exit 3
fi

DECLARED=""
awk_status=0
DECLARED="$(extract_declared_labels "$LABELS_YML")" || awk_status=$?

if [[ $awk_status -eq 3 ]]; then
    echo "ERROR: no \`- name:\` entry found in $LABELS_YML" >&2
    echo "That is not a label declaration file; this guard has nothing to compare." >&2
    exit 3
elif [[ $awk_status -ne 0 ]]; then
    echo "ERROR: could not parse $LABELS_YML (awk exit $awk_status)" >&2
    exit 2
fi

VIOLATIONS=0
while IFS= read -r label; do
    [[ -z "$label" ]] && continue
    if ! grep -qxF -- "$label" <<< "$DECLARED"; then
        echo "ERROR: label \`$label\` raises the pilot turn ceiling (PILOT_LABEL_TURN_CEILINGS) but is not declared in $(basename "$LABELS_YML")"
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
done <<< "$CEILINGS"

if [[ $VIOLATIONS -gt 0 ]]; then
    echo ""
    echo "Found $VIOLATIONS undeclared ceiling label(s)."
    echo ""
    echo "Why this is blocking (mika#2542): .github/workflows/labels.yml syncs with"
    echo "\`delete-other-labels: true\`, so an undeclared label is DELETED from the"
    echo "repository — and from every issue carrying it — with no \`unlabeled\` event."
    echo "The ceiling it raises then applies to nobody, and the dispatch line reads"
    echo "exactly like a fleet with no such ticket in flight."
    echo ""
    echo "Fix: add a \`- name: <label>\` entry (colour + a description naming its machine"
    echo "consequence) to $LABELS_YML, or remove the entry from PILOT_LABEL_TURN_CEILINGS."
    exit 1
fi

COUNT="$(printf '%s\n' "$CEILINGS" | grep -c .)"
echo "Pilot turn-ceiling labels are declared ($COUNT ceiling label(s) checked: $(printf '%s\n' "$CEILINGS" | tr '\n' ' '))."
exit 0

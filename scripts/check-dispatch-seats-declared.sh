#!/usr/bin/env bash
# CI lint: every dispatch seat the engine knows must have a declared label, and
# every declared `dispatch:*` label must name a seat the engine knows.
#
# THE PROPERTY: two lists that must be equal.
#   `KNOWN_DISPATCH_SEATS` (crates/mika-agent/src/webhook_dispatch.rs) is the
#   vocabulary of the seat gate. `.github/labels.yml` is the vocabulary GitHub
#   is allowed to carry — `.github/workflows/labels.yml` runs EndBug/label-sync
#   with `delete-other-labels: true`, so a label absent from that file is a
#   label that gets DELETED from the repository on the next sync, and with it
#   from every issue carrying it.
#
# THE FOUNDING INCIDENT is mika#2092, and it has two halves.
#
#   Half one, measured 2026-08-30: `dispatch:ssc` and `dispatch:mpc` existed on
#   the repository but were declared nowhere. At 09:12:51Z a push touching
#   `.github/labels.yml` for an unrelated reason ran label-sync, and both seat
#   labels vanished — from the repo and from every issue holding one. The
#   timeline of mika#2055 records `labeled dispatch:ssc` and NO `unlabeled`
#   event: deleting a label repo-wide removes it everywhere without writing a
#   single event. Nothing, anywhere, said the marker had existed.
#
#   Half two, the corollary: `CURRENT_DISPATCH_SEAT = "loop"` but no
#   `dispatch:loop` label had ever been declared, so `SeatVerdict::
#   OwnedByCurrentSeat` — the ONLY positive-pass branch of the predicate — was
#   unreachable in production. The loop could never claim a ticket; it could
#   only fail to be refused.
#
# WHY THE FAILURE IS SILENT, which is why a guard and not a note.
#   When a seat label disappears, `classify_dispatch_seat` reads `NoSeatLabel`,
#   `refuses()` is false, and the loop resumes being exactly the engine that
#   produced the 2026-08-30 collision. No log, no audit event, no red test. The
#   protection disarms itself and reports nothing. That is the class named in
#   docs/solutions/best-practices/un-label-denforcement-non-declare-echoue-en-silence-2026-09-01.md
#   — third occurrence, after `dispatch:ssc` and `operator-review`/`blocked`.
#
# WHY IT IS BIDIRECTIONAL.
#   code-without-label is the disarming direction (above). label-without-code is
#   the mirror: an operator posts `dispatch:zorglub`, the engine cannot resolve
#   it, `SeatVerdict::Unresolvable` refuses the ticket — fail-closed, so the
#   loop stops on a label somebody was told existed. Both directions are drift
#   between the same two lists; a guard that only knows one of them lets the
#   other through.
#
# WHEN YOU ADD A SEAT, both sides move in the same commit. That is the whole
# contract, and this script is what makes "the same commit" enforceable rather
# than remembered.
#
# NOT A SEAT: `dispatch:zorglub` appears throughout the Rust tests as the
#   unknown-seat fixture. It must NEVER be declared in labels.yml — declaring it
#   would make the fixture reachable in production. This guard would then fail
#   on the code-side direction, which is the correct answer.
#
# Usage: check-dispatch-seats-declared.sh [webhook_dispatch.rs] [labels.yml]
#   Both default to the repo's real files. Explicit arguments let the
#   anti-vacuity harness point the guard at fixtures (see
#   scripts/test-check-dispatch-seats-declared.sh).
#
# Exit 0 clean, 1 on divergence, 2 if a file cannot be read, 3 if either list
# could not be parsed at all (a guard that finds nothing to check must say so
# rather than pass).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SEATS_RS="${1:-$REPO_ROOT/crates/mika-agent/src/webhook_dispatch.rs}"
LABELS_YML="${2:-$REPO_ROOT/.github/labels.yml}"

if [[ ! -f "$SEATS_RS" ]]; then
    echo "ERROR: seat-vocabulary source not readable: $SEATS_RS" >&2
    exit 2
fi
if [[ ! -f "$LABELS_YML" ]]; then
    echo "ERROR: label declaration file not readable: $LABELS_YML" >&2
    exit 2
fi

# ── Side A: the seats the engine knows.
#
# Anchored on the const DECLARATION, not on the bare name — `webhook_dispatch.rs`
# also carries `KNOWN_DISPATCH_SEATS.contains(...)` and `.join(...)`, and a
# parser that matched those would read a use site as a declaration.
#
# The literal is accumulated until its terminating `;` so a rustfmt reflow onto
# several lines (which is what happens the moment a fourth seat is added) does
# not blind the guard — that would be a guard that knows one *spelling* of the
# declaration, the mika#2103 failure.
extract_seats() {
    awk '
        /^[ \t]*(pub[ \t]*(\([^)]*\)[ \t]*)?)?const[ \t]+KNOWN_DISPATCH_SEATS[ \t]*:/ {
            collecting = 1
        }
        collecting {
            line = $0
            sub(/\/\/.*$/, "", line)          # a trailing line comment is not code
            buf = buf " " line
            if (line ~ /;/) { collecting = 0; found = 1 }
        }
        END {
            if (!found) { exit 3 }

            # Drop everything up to the FIRST `=`. Non-negotiable: the type is
            # `&[&str]`, so a naive search for the first bracketed group would
            # return `[&str]` and report zero seats — a guard that passes by
            # finding nothing.
            sub(/^[^=]*=/, "", buf)

            if (match(buf, /\[[^]]*\]/) == 0) { exit 4 }
            lit = substr(buf, RSTART + 1, RLENGTH - 2)

            n = split(lit, parts, "\"")
            for (i = 2; i <= n; i += 2) {
                v = parts[i]
                gsub(/^[ \t]+|[ \t]+$/, "", v)
                if (v != "") { print v }
            }
        }
    ' "$1"
}

# ── Side B: the `dispatch:*` labels GitHub is allowed to carry.
extract_declared_seats() {
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

            if (v ~ /^dispatch:/) {
                sub(/^dispatch:/, "", v)
                if (v != "") { print v }
            }
        }
        END { if (!any_name) { exit 3 } }
    ' "$1"
}

SEATS=""
awk_status=0
SEATS="$(extract_seats "$SEATS_RS")" || awk_status=$?

if [[ $awk_status -eq 3 ]]; then
    echo "ERROR: no \`const KNOWN_DISPATCH_SEATS\` declaration found in $SEATS_RS" >&2
    echo "This guard has nothing to compare, which is not the same as agreement." >&2
    exit 3
elif [[ $awk_status -eq 4 ]]; then
    echo "ERROR: \`KNOWN_DISPATCH_SEATS\` was found in $SEATS_RS but its array literal did not parse." >&2
    echo "Expected the shape: const KNOWN_DISPATCH_SEATS: &[&str] = &[\"loop\", ...];" >&2
    exit 3
elif [[ $awk_status -ne 0 ]]; then
    echo "ERROR: could not parse $SEATS_RS (awk exit $awk_status)" >&2
    exit 2
fi

if [[ -z "$SEATS" ]]; then
    echo "ERROR: \`KNOWN_DISPATCH_SEATS\` in $SEATS_RS is empty." >&2
    echo "An empty seat vocabulary makes every dispatch:* label unresolvable." >&2
    exit 3
fi

DECLARED=""
awk_status=0
DECLARED="$(extract_declared_seats "$LABELS_YML")" || awk_status=$?

if [[ $awk_status -eq 3 ]]; then
    echo "ERROR: no \`- name:\` entry found in $LABELS_YML" >&2
    echo "That is not a label declaration file; this guard has nothing to compare." >&2
    exit 3
elif [[ $awk_status -ne 0 ]]; then
    echo "ERROR: could not parse $LABELS_YML (awk exit $awk_status)" >&2
    exit 2
fi

# ── Compare the two sets, both ways.
VIOLATIONS=0

while IFS= read -r seat; do
    [[ -z "$seat" ]] && continue
    if ! printf '%s\n' "$DECLARED" | grep -qxF "$seat"; then
        echo "ERROR: seat \`$seat\` is in KNOWN_DISPATCH_SEATS but \`dispatch:$seat\` is not declared in $(basename "$LABELS_YML")"
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
done <<< "$SEATS"

while IFS= read -r declared; do
    [[ -z "$declared" ]] && continue
    if ! printf '%s\n' "$SEATS" | grep -qxF "$declared"; then
        echo "ERROR: label \`dispatch:$declared\` is declared in $(basename "$LABELS_YML") but \`$declared\` is not in KNOWN_DISPATCH_SEATS"
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
done <<< "$DECLARED"

if [[ $VIOLATIONS -gt 0 ]]; then
    echo ""
    echo "Found $VIOLATIONS divergence(s) between the seat vocabulary and the label declarations."
    echo ""
    echo "Sources of truth, which must agree in the SAME commit:"
    echo "  code   $SEATS_RS  (const KNOWN_DISPATCH_SEATS)"
    echo "  labels $LABELS_YML                (- name: \"dispatch:<seat>\")"
    echo ""
    echo "Why this is blocking (mika#2092): .github/workflows/labels.yml syncs with"
    echo "\`delete-other-labels: true\`, so an undeclared seat label is DELETED from the"
    echo "repository — and from every issue carrying it — with no \`unlabeled\` event and"
    echo "no log. The seat gate then reads NoSeatLabel everywhere and refuses nothing."
    echo "The protection disarms itself in silence. That is the failure this guard exists"
    echo "to make loud."
    echo ""
    echo "Fix: add the missing \`- name: \"dispatch:<seat>\"\` entry (with a colour and a"
    echo "description), or remove the seat from KNOWN_DISPATCH_SEATS. Do NOT declare"
    echo "\`dispatch:zorglub\` — it is the unknown-seat TEST fixture, and declaring it would"
    echo "make the fixture reachable in production."
    exit 1
fi

SEAT_COUNT="$(printf '%s\n' "$SEATS" | grep -c .)"
echo "Seat vocabulary and label declarations agree ($SEAT_COUNT seat(s): $(printf '%s\n' "$SEATS" | tr '\n' ' '))."
exit 0

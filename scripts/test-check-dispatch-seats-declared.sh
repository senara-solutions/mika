#!/usr/bin/env bash
#
# Anti-vacuity harness for scripts/check-dispatch-seats-declared.sh (mika#2092).
#
# "Delete the thing the test protects; confirm the test goes red."
#
# A guard nobody has watched go red is a decoration — mika#2103 is the incident
# where a lint stayed green through 26 production panics because it knew one
# spelling of the defect. The guard under test compares two lists that must be
# equal, so this suite pins the *property* in both directions and across the
# reformattings of the Rust literal that a fourth seat will produce.
#
# It also pins the two ways a set-comparison guard passes for the wrong reason:
# by parsing zero seats out of the code (an empty set is a subset of anything)
# and by parsing zero labels out of the YAML. Both must exit 3, never 0.
#
# The battery carries accented fixtures on purpose. This repository writes its
# labels, plans and tickets in French — `description: "Routage : ce ticket est
# pris par…"` is the real shape — so an ASCII-only fixture set tests a
# population that does not exist here. Both the accented path and the accented
# content must survive intact, and the failure message is asserted to reproduce
# the seat name verbatim.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GUARD="$REPO_ROOT/scripts/check-dispatch-seats-declared.sh"

PASS=0
FAIL=0

# Run the guard against ($1 = rs, $2 = yml); assert its exit equals $3. $4 = name.
assert_exit() {
    local rs="$1" yml="$2" want="$3" name="$4"
    local got=0
    bash "$GUARD" "$rs" "$yml" >/dev/null 2>&1 || got=$?
    if [ "$got" -eq "$want" ]; then
        echo "PASS: $name (exit $got)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (wanted exit $want, got $got)"
        FAIL=$((FAIL + 1))
    fi
}

# Assert the guard's combined output for ($1, $2) contains the literal $3.
assert_output_contains() {
    local rs="$1" yml="$2" needle="$3" name="$4"
    local out
    out="$(bash "$GUARD" "$rs" "$yml" 2>&1 || true)"
    if [[ "$out" == *"$needle"* ]]; then
        echo "PASS: $name"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (output did not contain: $needle)"
        FAIL=$((FAIL + 1))
    fi
}

# One root for the whole suite, removed by a trap however the suite exits.
#
# Registering each fixture dir in an array instead does NOT work here: the
# make_* helpers are always called in a command substitution, so their bodies
# run in a subshell and any array they append to is discarded on return. (Same
# reasoning as scripts/test-check-image-tags-immutable.sh, and worth repeating
# rather than re-deriving.)
FIXTURE_ROOT="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_ROOT"' EXIT

# A throwaway file whose body is $1, written at relative path $2 under a fresh
# temp dir. Echoes the file path.
make_fixture() {
    local body="$1" rel="$2" dir
    dir="$(mktemp -d -p "$FIXTURE_ROOT")"
    mkdir -p "$dir/$(dirname "$rel")"
    printf '%s\n' "$body" > "$dir/$rel"
    echo "$dir/$rel"
}

# A labels.yml carrying the entries in $@ under `dispatch:`, plus one ordinary
# label so `- name:` is never absent for the wrong reason.
make_labels() {
    local body="- name: bug
  color: \"d73a4a\"
  description: Something isn't working
"
    local seat
    for seat in "$@"; do
        body+="
- name: \"dispatch:$seat\"
  color: \"1d76db\"
  description: \"Routage : ce ticket est pris par $seat. Un seul dispatcher par ticket.\"
"
    done
    make_fixture "$body" "labels.yml"
}

# A webhook_dispatch.rs stub declaring the seats in $@ on ONE line.
make_seats_oneline() {
    local lit="" seat
    for seat in "$@"; do
        [[ -n "$lit" ]] && lit+=", "
        lit+="\"$seat\""
    done
    make_fixture "/// Every seat this engine knows how to resolve (mika#2084).
pub(crate) const KNOWN_DISPATCH_SEATS: &[&str] = &[$lit];

fn seat_names() -> String {
    KNOWN_DISPATCH_SEATS.join(\", \")
}" "webhook_dispatch.rs"
}

# The same, reflowed by rustfmt across several lines — the shape the real file
# takes the moment a fourth seat is added.
make_seats_multiline() {
    local body="pub(crate) const KNOWN_DISPATCH_SEATS: &[&str] = &[
" seat
    for seat in "$@"; do
        body+="    \"$seat\",
"
    done
    body+="];"
    make_fixture "$body" "webhook_dispatch.rs"
}

# ── 1. POSITIVE CONTROL — the repo's own two files agree.
#    If this goes red, the fix for mika#2092 has been reverted.
assert_exit "$REPO_ROOT/crates/mika-agent/src/webhook_dispatch.rs" \
    "$REPO_ROOT/.github/labels.yml" 0 \
    "the repo's real seat vocabulary and labels.yml agree"

# ── 2. NEGATIVE CONTROL — the exact mika#2092 defect, in the shape it shipped:
#    three seats in code, `dispatch:loop` undeclared. If this ever goes green,
#    the class is unprotected regardless of everything below.
RS3="$(make_seats_oneline loop ssc mpc)"
YML_NO_LOOP="$(make_labels ssc mpc)"
assert_exit "$RS3" "$YML_NO_LOOP" 1 \
    "the mika#2092 defect (dispatch:loop undeclared) is rejected"
assert_output_contains "$RS3" "$YML_NO_LOOP" \
    "seat \`loop\` is in KNOWN_DISPATCH_SEATS" \
    "the failure names the undeclared seat, not just a count"

# ── 3. The rule is set EQUALITY, not "code is a subset". A label declared for a
#    seat the engine cannot resolve makes classify_dispatch_seat refuse the
#    ticket (Unresolvable) — fail-closed, i.e. the loop stops. Same drift,
#    other direction, must bite the same.
YML_EXTRA="$(make_labels loop ssc mpc zorglub)"
assert_exit "$RS3" "$YML_EXTRA" 1 \
    "a declared dispatch:* label with no matching seat is rejected"
assert_output_contains "$RS3" "$YML_EXTRA" \
    "label \`dispatch:zorglub\` is declared" \
    "the failure names the orphan label"

# ── 4. ...and the agreeing case is green, in both writings of the literal. A
#    guard that cannot be satisfied gets disabled rather than obeyed.
YML3="$(make_labels loop ssc mpc)"
assert_exit "$RS3" "$YML3" 0 "one-line literal, three seats declared: green"

RS3_MULTI="$(make_seats_multiline loop ssc mpc)"
assert_exit "$RS3_MULTI" "$YML3" 0 \
    "rustfmt-reflowed multi-line literal parses identically (the 4th-seat shape)"

RS4_MULTI="$(make_seats_multiline loop ssc mpc quatrieme)"
assert_exit "$RS4_MULTI" "$YML3" 1 \
    "a 4th seat added to the reflowed literal without its label is rejected"

# ── 5. THE TWO WAYS A SET-COMPARISON GUARD PASSES FOR THE WRONG REASON.
#    An empty parse is a subset of everything, so it compares equal to
#    everything. Silence here is indistinguishable from agreement — which is
#    precisely how a green check comes to mean nothing.
f="$(make_fixture 'fn main() { let seats = vec!["loop"]; }' "webhook_dispatch.rs")"
assert_exit "$f" "$YML3" 3 "no KNOWN_DISPATCH_SEATS declaration exits 3, not 0"

# A USE site is not a DECLARATION. Anchoring on the bare name would read this
# file as declaring the seats it merely joins.
f="$(make_fixture 'fn seat_names() -> String { KNOWN_DISPATCH_SEATS.join(", ") }' \
    "webhook_dispatch.rs")"
assert_exit "$f" "$YML3" 3 "a KNOWN_DISPATCH_SEATS *use* site is not a declaration"

f="$(make_fixture 'pub(crate) const KNOWN_DISPATCH_SEATS: &[&str] = &[];' \
    "webhook_dispatch.rs")"
assert_exit "$f" "$YML3" 3 "an empty seat array exits 3, not 0"

f="$(make_fixture '# just a comment, no entries' "labels.yml")"
assert_exit "$RS3" "$f" 3 "a labels file with no \`- name:\` entry exits 3, not 0"

assert_exit "/nonexistent/webhook_dispatch.rs" "$YML3" 2 "an unreadable .rs exits 2, not 0"
assert_exit "$RS3" "/nonexistent/labels.yml" 2 "an unreadable labels.yml exits 2, not 0"

# ── 6. The type `&[&str]` contains a bracketed group BEFORE the value array.
#    A parser that takes the first `[...]` reads `[&str]`, finds zero seats, and
#    passes. That is failure mode 5 wearing a plausible disguise, so it gets its
#    own case: the guard must report exactly the three real seats.
assert_output_contains "$RS3" "$YML3" "3 seat(s): loop ssc mpc" \
    "the \`&[&str]\` type is not mistaken for the value array"

# ── 7. Not every `dispatch`-ish label is a seat. `dispatched` and
#    `dispatch-ready` start with the same letters and are ordinary labels; the
#    prefix match is on `dispatch:` exactly, mirroring
#    DISPATCH_SEAT_LABEL_PREFIX in the Rust. Treating them as seats would
#    manufacture two orphan labels and fail a clean repository.
f="$(make_fixture '- name: bug
  color: "d73a4a"
  description: Something

- name: dispatched
  color: "cccccc"
  description: "Not a seat."

- name: dispatch-ready
  color: "cccccc"
  description: "Not a seat either."

- name: "dispatch:loop"
  color: "1d76db"
  description: "Routage : boucle autonome."

- name: "dispatch:ssc"
  color: "5319e7"
  description: "Routage : SSC."

- name: "dispatch:mpc"
  color: "0e8a16"
  description: "Routage : MPC."' "labels.yml")"
assert_exit "$RS3" "$f" 0 "\`dispatched\` / \`dispatch-ready\` are not read as seats"

# ── 8. ACCENTED FIXTURES — our actual population.
#
# The path carries accents, a space and an apostrophe; the descriptions carry
# the French text the real file uses. A guard that mangles UTF-8, or that splits
# an unquoted path, cannot pass these.
ACCENTED_OK='- name: bug
  color: "d73a4a"
  description: "Quelque chose ne fonctionne pas"

- name: "dispatch:loop"
  color: "1d76db"
  description: "Routage : ce ticket est pris par la boucle autonome (mika-dev). Un seul dispatcher par ticket."

- name: "dispatch:ssc"
  color: "5319e7"
  description: "Routage : ce ticket est pris par SSC (senara-solutions-claude). Un seul dispatcher par ticket."

- name: "dispatch:mpc"
  color: "0e8a16"
  description: "Routage : ce ticket est pris par MPC (mika-platform-claude). Un seul dispatcher par ticket."'
f="$(make_fixture "$ACCENTED_OK" "étiquettes du dépôt/labels.yml")"
assert_exit "$RS3" "$f" 0 "accented path + accented French descriptions: green"

# The accent alone must not manufacture a violation, and the failure message
# must reproduce the seat verbatim — an error that garbles the name is an error
# nobody can act on.
ACCENTED_BAD='- name: bug
  color: "d73a4a"
  description: "Quelque chose ne fonctionne pas"

- name: "dispatch:ssc"
  color: "5319e7"
  description: "Routage : ce ticket est pris par SSC. Un seul dispatcher par ticket."

- name: "dispatch:mpc"
  color: "0e8a16"
  description: "Routage : ce ticket est pris par MPC. Un seul dispatcher par ticket."'
f="$(make_fixture "$ACCENTED_BAD" "étiquettes du dépôt/labels.yml")"
assert_exit "$RS3" "$f" 1 "accented path, dispatch:loop missing: rejected"
assert_output_contains "$RS3" "$f" "\`dispatch:loop\` is not declared" \
    "the failure message survives the accented fixture path intact"

echo ""
echo "check-dispatch-seats-declared anti-vacuity: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1

#!/usr/bin/env bash
#
# Anti-vacuity harness for scripts/check-pilot-turn-ceiling-labels.sh (mika#2542).
#
# "Delete the thing the test protects; confirm the test goes red."
#
# The guard checks that every key of PILOT_LABEL_TURN_CEILINGS is declared in
# labels.yml. A subset check passes for the wrong reason in exactly one way: by
# parsing ZERO keys out of the table (the empty set is a subset of anything).
# A rename of the array, a change of its entry format, or a table emptied by
# accident would all read as a clean tree. Each of those must exit 3, never 0,
# and this suite watches them do it.
#
# Fixtures are shaped on the REAL table and the REAL labels.yml — including the
# French descriptions and the in-array comments — never on an invented form.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GUARD="$REPO_ROOT/scripts/check-pilot-turn-ceiling-labels.sh"

PASS=0
FAIL=0

# Run the guard against ($1 = dispatch-lib, $2 = yml); assert exit == $3.
assert_exit() {
    local lib="$1" yml="$2" want="$3" name="$4"
    local got=0
    bash "$GUARD" "$lib" "$yml" >/dev/null 2>&1 || got=$?
    if [ "$got" -eq "$want" ]; then
        echo "PASS: $name (exit $got)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (wanted exit $want, got $got)"
        FAIL=$((FAIL + 1))
    fi
}

assert_output_contains() {
    local lib="$1" yml="$2" needle="$3" name="$4"
    local out
    out="$(bash "$GUARD" "$lib" "$yml" 2>&1 || true)"
    if [[ "$out" == *"$needle"* ]]; then
        echo "PASS: $name"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (output did not contain: $needle)"
        FAIL=$((FAIL + 1))
    fi
}

# One root, removed by a trap. The make_* helpers run inside command
# substitutions (subshells), so registering dirs in an array would be lost —
# same reasoning as scripts/test-check-dispatch-seats-declared.sh.
FIXTURE_ROOT="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_ROOT"' EXIT

make_fixture() {
    local body="$1" rel="$2" dir
    dir="$(mktemp -d -p "$FIXTURE_ROOT")"
    mkdir -p "$dir/$(dirname "$rel")"
    printf '%s\n' "$body" > "$dir/$rel"
    echo "$dir/$rel"
}

# A dispatch-lib stub whose table holds the entries in $@, in the real shape:
# a comment above, an in-array comment, a use site below.
make_lib() {
    local body="# mika#2542 — la table label → plafond de tours. UN SEUL SITE.
PILOT_LABEL_TURN_CEILINGS=(
    # une entrée par ligne
" entry
    for entry in "$@"; do
        body+="    \"$entry\"
"
    done
    body+=")

_pilot_label_turn_ceiling() {
    for _entry in \"\${PILOT_LABEL_TURN_CEILINGS[@]}\"; do :; done
}"
    make_fixture "$body" "dispatch-lib.sh"
}

make_labels() {
    local body="- name: bug
  color: \"d73a4a\"
  description: Something isn't working
" label
    for label in "$@"; do
        body+="
- name: $label
  color: \"5319e7\"
  description: \"Substrat de la boucle autonome. Relève le plafond pilote à 200 tours.\"
"
    done
    make_fixture "$body" "labels.yml"
}

# ── 1. POSITIVE CONTROL — the repo's own table and labels.yml agree.
#    If this goes red, R-2 of mika#2542 has been reverted.
assert_exit "$REPO_ROOT/skills/bundled/_shared/dispatch-lib.sh" \
    "$REPO_ROOT/.github/labels.yml" 0 \
    "the repo's real ceiling table is declared in labels.yml"
assert_output_contains "$REPO_ROOT/skills/bundled/_shared/dispatch-lib.sh" \
    "$REPO_ROOT/.github/labels.yml" "1 ceiling label(s) checked: loop-substrate" \
    "the real run announces a NON-ZERO count, naming the key it saw"

# ── 2. NEGATIVE CONTROL — the exact state mika#2542 found: the table reads
#    `loop-substrate`, labels.yml does not declare it. RED.
LIB1="$(make_lib "loop-substrate=200")"
YML_NONE="$(make_labels)"
assert_exit "$LIB1" "$YML_NONE" 1 "an undeclared ceiling label is rejected"
assert_output_contains "$LIB1" "$YML_NONE" "label \`loop-substrate\` raises the pilot turn ceiling" \
    "the failure names the undeclared label"

# ── 3. GREEN — declared. And a SECOND entry added later without its label is
#    red: the guard reads every key, not just the first.
YML1="$(make_labels loop-substrate)"
assert_exit "$LIB1" "$YML1" 0 "table and declaration agree: green"
LIB2="$(make_lib "loop-substrate=200" "gros-chantier=250")"
assert_exit "$LIB2" "$YML1" 1 "a second ceiling added without its label is rejected"

# ── 4. ONE DIRECTION ONLY — labels.yml carrying labels that are not ceilings
#    is the normal state of the file, and must stay green.
YML_MANY="$(make_labels loop-substrate p1-important agent-core)"
assert_exit "$LIB1" "$YML_MANY" 0 "declared labels that are not ceilings are not drift"

# ── 5. EXACT NAME — a declared `loop-substrate-v2` does not declare
#    `loop-substrate`. A substring match would pass this.
YML_V2="$(make_labels loop-substrate-v2)"
assert_exit "$LIB1" "$YML_V2" 1 "a declared look-alike (loop-substrate-v2) does not count"

# ── 6. THE WAYS A SUBSET GUARD PASSES FOR THE WRONG REASON — all exit 3.
LIB_EMPTY="$(make_lib)"
assert_exit "$LIB_EMPTY" "$YML1" 3 "an empty table exits 3, not 0"

f="$(make_fixture 'for _e in "${PILOT_LABEL_TURN_CEILINGS[@]}"; do :; done' "dispatch-lib.sh")"
assert_exit "$f" "$YML1" 3 "a table *use* site is not a declaration"

f="$(make_fixture 'PILOT_TURN_CEILINGS_BY_LABEL=(
    "loop-substrate=200"
)' "dispatch-lib.sh")"
assert_exit "$f" "$YML1" 3 "a renamed table exits 3 — the guard does not go blind in silence"

f="$(make_fixture 'PILOT_LABEL_TURN_CEILINGS=(
    [loop-substrate]=200
)' "dispatch-lib.sh")"
assert_exit "$f" "$YML1" 3 "a changed entry format (associative) exits 3, not 0"

f="$(make_fixture 'PILOT_LABEL_TURN_CEILINGS=(
    "loop-substrate=200"' "dispatch-lib.sh")"
assert_exit "$f" "$YML1" 3 "an unclosed table exits 3, not 0"

# ── 6b. PARTIAL PARSE — valid bash the resolver honours, but that a lenient
#    parser would skip, leaving the extra key unchecked. Each fixture pairs the
#    declared `loop-substrate` with an UNDECLARED `security-hot`: before the
#    fix each of these exited 0. All must exit 3.
f="$(make_fixture 'PILOT_LABEL_TURN_CEILINGS=(
    "loop-substrate=200"
    security-hot=250
)' "dispatch-lib.sh")"
assert_exit "$f" "$YML1" 3 "an unquoted entry exits 3, not 0"

f="$(make_fixture "PILOT_LABEL_TURN_CEILINGS=(
    \"loop-substrate=200\"
    'security-hot=250'
)" "dispatch-lib.sh")"
assert_exit "$f" "$YML1" 3 "a single-quoted entry exits 3, not 0"

f="$(make_fixture 'PILOT_LABEL_TURN_CEILINGS=(
    "loop-substrate=200" "security-hot=250"
)' "dispatch-lib.sh")"
assert_exit "$f" "$YML1" 3 "two entries on one line exit 3, not 0"

f="$(make_fixture 'PILOT_LABEL_TURN_CEILINGS=(
    "loop-substrate=200"
)
PILOT_LABEL_TURN_CEILINGS+=("security-hot=250")' "dispatch-lib.sh")"
assert_exit "$f" "$YML1" 3 "an append (+=) elsewhere in the file exits 3, not 0"

f="$(make_fixture 'PILOT_LABEL_TURN_CEILINGS=(
    "loop-substrate=200"
)
_reset() {
    PILOT_LABEL_TURN_CEILINGS=("security-hot=250")
}' "dispatch-lib.sh")"
assert_exit "$f" "$YML1" 3 "an indented second assignment exits 3, not 0"

f="$(make_fixture 'PILOT_LABEL_TURN_CEILINGS=(
    "loop-substrate="
)' "dispatch-lib.sh")"
assert_exit "$f" "$YML1" 3 "an empty ceiling value exits 3, not 0"

f="$(make_fixture 'PILOT_LABEL_TURN_CEILINGS=(
    "loop-substrate=abc"
)' "dispatch-lib.sh")"
assert_exit "$f" "$YML1" 3 "a non-numeric ceiling value exits 3, not 0"

#    A trailing comment on an entry is still the canonical shape: green.
f="$(make_fixture 'PILOT_LABEL_TURN_CEILINGS=(
    "loop-substrate=200"  # mika#2542
)' "dispatch-lib.sh")"
assert_exit "$f" "$YML1" 0 "a canonical entry with a trailing comment stays green"

f="$(make_fixture '# just a comment, no entries' "labels.yml")"
assert_exit "$LIB1" "$f" 3 "a labels file with no \`- name:\` entry exits 3, not 0"

# ── 7. Unreadable inputs exit 2.
assert_exit "/nonexistent/dispatch-lib.sh" "$YML1" 2 "an unreadable dispatch-lib exits 2"
assert_exit "$LIB1" "/nonexistent/labels.yml" 2 "an unreadable labels.yml exits 2"

# ── 8. ACCENTED PATH — our actual population writes French.
f="$(make_fixture "- name: loop-substrate
  color: \"5319e7\"
  description: \"Substrat de la boucle autonome (pipeline, dispatch, gardes). Relève le plafond pilote à 200 tours.\"" \
    "étiquettes du dépôt/labels.yml")"
assert_exit "$LIB1" "$f" 0 "accented path + accented description: green"

echo ""
echo "check-pilot-turn-ceiling-labels anti-vacuity: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1

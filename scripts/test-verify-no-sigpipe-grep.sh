#!/usr/bin/env bash
# The fixture bodies below are single-quoted ON PURPOSE: they are literal shell
# text written INTO a probe file, so `$x` must stay unexpanded (file-wide).
# shellcheck disable=SC2016
#
# Anti-vacuity harness for scripts/verify-no-sigpipe-grep.sh (mika#2055).
#
# The guard is only worth shipping if it can FAIL. This suite proves it:
#   - it is clean on the real (swept) tree;
#   - it goes RED on a deliberately-reintroduced bad pattern (the negative
#     control the ticket demands);
#   - the here-string remedy is accepted;
#   - the ticket-cited escape hatch suppresses, and a BARE marker does not.
#
# "Delete the thing the test protects; confirm the test goes red." — the
# reproduction that separates a real guard from a vacuous one
# (docs/solutions/best-practices/structural-guard-fails-open-parser-fixture-harness.md).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GUARD="$REPO_ROOT/scripts/verify-no-sigpipe-grep.sh"

PASS=0
FAIL=0

# Run the guard against $1; assert its exit code equals $2. $3 = case name.
assert_exit() {
    local root="$1" want="$2" name="$3"
    local got=0
    bash "$GUARD" "$root" >/dev/null 2>&1 || got=$?
    if [ "$got" -eq "$want" ]; then
        echo "PASS: $name (exit $got)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (wanted exit $want, got $got)"
        FAIL=$((FAIL + 1))
    fi
}

# A throwaway fixture tree with one shell file whose body is $1.
make_fixture() {
    local body="$1"
    local dir
    dir="$(mktemp -d)"
    {
        echo '#!/usr/bin/env bash'
        echo 'set -euo pipefail'
        echo 'x="some haystack"'
        printf '%s\n' "$body"
    } > "$dir/probe.sh"
    echo "$dir"
}

# Same, but the body is EVERY argument, one per line (mika#3130). The T1 class
# only exists across physical lines, so the single-body helper cannot express it:
# a fixture written as one `$body` with embedded `\n` would be indistinguishable
# from the mono-line cases the six historical assertions already cover.
make_fixture_lines() {
    local dir
    dir="$(mktemp -d)"
    {
        echo '#!/usr/bin/env bash'
        echo 'set -euo pipefail'
        echo 'x="some haystack"'
        printf '%s\n' "$@"
    } > "$dir/probe.sh"
    echo "$dir"
}

# 1. The real, swept tree is clean.
assert_exit "$REPO_ROOT" 0 "swept repo tree is clean"

# 2. Negative control (anti-vacuity): the bad pattern goes RED.
d="$(make_fixture 'if echo "$x" | grep -qF "hay"; then :; fi')"
assert_exit "$d" 1 "reintroduced echo | grep -qF is rejected"
rm -rf "$d"

# 2b. printf producer variant is also caught.
d="$(make_fixture "if printf '%s' \"\$x\" | grep -qE 'ha.' ; then :; fi")"
assert_exit "$d" 1 "reintroduced printf | grep -qE is rejected"
rm -rf "$d"

# 3. The here-string remedy is accepted.
d="$(make_fixture 'if grep -qF -- "hay" <<<"$x"; then :; fi')"
assert_exit "$d" 0 "here-string remedy passes"
rm -rf "$d"

# 3b. A non-short-circuiting grep (no -q) is not the target class.
d="$(make_fixture 'echo "$x" | grep -oF "hay" | head -1')"
assert_exit "$d" 0 "grep without -q is not flagged"
rm -rf "$d"

# 4. Escape hatch: a ticket-cited marker suppresses.
d="$(make_fixture 'if echo "$x" | grep -qF "hay"; then :; fi  # sigpipe-safe: #2055')"
assert_exit "$d" 0 "ticket-cited escape marker suppresses"
rm -rf "$d"

# 5. A BARE marker (no #<digits> citation) does NOT suppress.
d="$(make_fixture 'if echo "$x" | grep -qF "hay"; then :; fi  # sigpipe-safe')"
assert_exit "$d" 1 "bare escape marker does not suppress"
rm -rf "$d"

# ───────────────────────────────────────────────────────────────────────────────
# mika#3130 — the two classes the guard did NOT see, and the two false positives
# the widening must not buy.
#
# The six cases above stay exactly as they were: they are the non-regression
# proof of the mono-line behaviour. Case 1 ("swept repo tree is clean") doubles
# as the proof that the five measured sites really were rewritten — with the
# extended detector, a single one left un-rewritten would make it exit 1.
# ───────────────────────────────────────────────────────────────────────────────

# 6. T1 (line continuation), on the SHAPE OF THE TICKET'S OWN SITE: the producer
#    on one line, the `| grep -qF` on the next after a `\`. This is the form that
#    failed `Check` on attempt 1 of run 43079844489 (PR #3075) while living
#    inside this guard's scan field.
d="$(make_fixture_lines \
    'assert_eq "label" "yes" \' \
    '    "$(if printf '"'"'%s'"'"' "$x" \' \
    '        | grep -qF '"'"'hay'"'"'; \' \
    '       then printf '"'"'yes'"'"'; else printf '"'"'no'"'"'; fi)"')"
assert_exit "$d" 1 "T1: producer and 'grep -q' split by a continuation is rejected"
rm -rf "$d"

# 7. The here-string remedy, written across a continuation, is still accepted —
#    the joining must not turn the FIX into a violation.
d="$(make_fixture_lines \
    'NEEDLE='"'"'hay'"'"'' \
    'assert_eq "label" "yes" \' \
    '    "$(if grep -qF -- "$NEEDLE" <<<"$x"; \' \
    '       then printf '"'"'yes'"'"'; else printf '"'"'no'"'"'; fi)"')"
assert_exit "$d" 0 "T1: here-string remedy across a continuation passes"
rm -rf "$d"

# 8. The escape hatch on a T1 group: the marker lives on the LAST physical line,
#    the violation starts on the first — it must still suppress (the exemption is
#    read before the end-of-line comment is removed).
d="$(make_fixture_lines \
    'if printf '"'"'%s'"'"' "$x" \' \
    '    | grep -qF "hay"; then :; fi  # sigpipe-safe: #2055')"
assert_exit "$d" 0 "T1: ticket-cited marker on any line of the group suppresses"
rm -rf "$d"

# 9. …and a BARE marker on that same group does not. The citation stays
#    structurally required across the continuation too.
d="$(make_fixture_lines \
    'if printf '"'"'%s'"'"' "$x" \' \
    '    | grep -qF "hay"; then :; fi  # sigpipe-safe')"
assert_exit "$d" 1 "T1: bare marker on the group does not suppress"
rm -rf "$d"

# 10. T2 (intermediate link) with a literal `#` inside the middle pattern — the
#     exact shape of test-dispatch-lib.sh's mika#2608 assertion. The old
#     `[^|#]*` middle class could cross neither the `|` nor the `#`.
d="$(make_fixture 'if printf '"'"'%s\n'"'"' "$x" | grep -v '"'"'^[[:space:]]*#'"'"' | grep -qE "hay"; then :; fi')"
assert_exit "$d" 1 "T2: intermediate 'grep -v' link with a '#' in its pattern is rejected"
rm -rf "$d"

# 11. T2 with the `|` living inside a parameter expansion (`"${x#*|}"`) — the
#     shape of test-verify-no-secret-in-setenv.sh's assert_mentions.
d="$(make_fixture 'if printf '"'"'%s'"'"' "${x#*|}" | grep -q "hay"; then :; fi')"
assert_exit "$d" 1 "T2: a '|' inside an expansion no longer hides the pipeline"
rm -rf "$d"

# 12. The first false positive the widening must not buy: an END-OF-LINE comment
#     that merely CITES the forbidden shape. (Full-line prose was already
#     filtered before mika#3130; this is the in-line case.)
d="$(make_fixture 'grep -qF -- "hay" <<<"$x" || true  # prose: never printf "$x" | grep -q hay')"
assert_exit "$d" 0 "no FP: an end-of-line comment citing the shape is not a violation"
rm -rf "$d"

# 13. The second: a `;` between the producer and the pipeline. `[^;&]*` must keep
#     the producer and the `grep -q` in the SAME command — here `cat` is the real
#     producer, and it is out of the `printf|echo` vocabulary, so this must pass.
d="$(make_fixture 'echo "a"; cat /dev/null | grep -q "b" || true')"
assert_exit "$d" 0 "no FP: a ';' cuts the producer off from the pipeline"
rm -rf "$d"

echo ""
echo "no-sigpipe-grep anti-vacuity: $PASS passed, $FAIL failed."
[ "$FAIL" -eq 0 ]

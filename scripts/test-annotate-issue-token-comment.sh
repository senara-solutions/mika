#!/usr/bin/env bash
# Test suite for `scripts/annotate-issue-token-comment.sh` and its guard scan
# (mika#2552, V1–V7 plus the scan's own negative controls).
#
# ─────────────────────────────────────────────────────────────────────────────
# WHAT THIS PINS
#
# The workflow used to resolve its PATCH target by rewriting an HTML url, which
# produced `https://api.github.com/repos/O/R/issues/N#issuecomment-ID`. The
# fragment is not transmitted over HTTP, so the effective target was the
# ISSUE-edit route and `-f body=` replaced the issue body. Measured on mika#2544:
# a 3934-byte groomed body reduced to 979 bytes, its grooming callouts gone.
#
# ─────────────────────────────────────────────────────────────────────────────
# V3 — THE NEGATIVE CONTROL, ON A FROZEN PRE-FIX TARGET
#
# Without it, "the guard refuses" is indistinguishable from "the harness checks
# nothing". The pre-fix target is written out LITERALLY below and never
# re-derived from git history: a fixture that silently failed to reconstruct the
# old form would exercise the post-fix form and pass, which is the vacuous shape
# mika#2103 exists to refuse.
#
# It has three halves, and all three are needed:
#   V3a  the sourced predicate refuses the frozen form;
#   V3b  END TO END, a fixture whose `.url` carries the frozen form (i.e. the
#        resolution regressed) yields a refusal, a non-zero exit, and ZERO PATCH;
#   V3c  a MUTANT whose predicate accepts everything DOES send that PATCH —
#        which is what proves V3a/V3b are load-bearing rather than always-true.
#
# ─────────────────────────────────────────────────────────────────────────────
# HOW THE SCRIPT IS RUN WITHOUT A NETWORK
#
# A stub `gh` is installed on PATH (motif: `scripts/test-pr-origin-report.sh`).
# It RECORDS ITS ARGV, which is what makes the effective PATCH target assertable,
# and it serves the comments listing by running the REAL jq filter over fixture
# pages — so the `select(.body | contains(MARKER))` expression and the
# `--paginate` page-by-page semantics are exercised, not simulated.
#
# Run: bash scripts/test-annotate-issue-token-comment.sh
# Expected: all assertions pass, exit 0.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SUBJECT="$REPO_ROOT/scripts/annotate-issue-token-comment.sh"
SCAN="$REPO_ROOT/scripts/check-issue-comment-patch-guard.sh"
WORKFLOW="$REPO_ROOT/.github/workflows/issue-token-annotate.yml"

MARKER='<!-- canonical-token-annotation (mika#2201) -->'

# The exact shape the pre-mika#2552 substitution produced. FROZEN — never
# re-derived. This string is the whole reason the defect existed.
PRE_FIX_TARGET='https://api.github.com/repos/senara-solutions/mika/issues/2544#issuecomment-3456789'
GOOD_TARGET='https://api.github.com/repos/senara-solutions/mika/issues/comments/3456789'

PASS=0
FAIL=0

ok() { PASS=$((PASS + 1)); echo "  ✓ $1"; }
ko() {
    FAIL=$((FAIL + 1))
    echo "  ✗ $1"
    shift
    for line in "$@"; do echo "    $line"; done
}

assert_eq() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then ok "$label"
    else ko "$label" "expected: '$expected'" "actual:   '$actual'"; fi
}

assert_contains() {
    local label="$1" needle="$2" haystack="$3"
    if [[ "$haystack" == *"$needle"* ]]; then ok "$label"
    else ko "$label" "needle:   '$needle'" "haystack: '$haystack'"; fi
}

assert_not_contains() {
    local label="$1" needle="$2" haystack="$3"
    if [[ "$haystack" != *"$needle"* ]]; then ok "$label"
    else ko "$label" "must NOT contain: '$needle'" "haystack:         '$haystack'"; fi
}

# ── Anti-vacuity, FIRST. A harness whose subjects have moved passes by looking
# at nothing, and reads exactly like a clean run (mika#2103, mika#2205).
for f in "$SUBJECT" "$SCAN" "$WORKFLOW"; do
    if [ ! -r "$f" ]; then
        echo "FATAL: subject not readable: $f — this harness would verify nothing." >&2
        exit 1
    fi
done
if ! command -v jq >/dev/null 2>&1; then
    echo "FATAL: jq is required — the stub runs the real jq filter over fixtures." >&2
    echo "       Refusing to skip: a silent skip reads like a passing suite." >&2
    exit 1
fi

TMPROOT=$(mktemp -d /tmp/annotate-token-comment-test-XXXXXX)
trap 'rm -rf "$TMPROOT"' EXIT

# ─────────────────────────────────────────────────────────────────────────────
# The stub, and the case scaffolding.

new_case() {
    local name="$1"
    local w="$TMPROOT/$name"
    mkdir -p "$w/bin" "$w/fixtures"
    : > "$w/argv.log"
    printf '0' > "$w/count"
    printf 'scripts/canonical-tokens.tsv: line 12: ESCALATE : has a space before its colon\n' \
        > "$w/lint-out.txt"
    cat > "$w/bin/gh" <<'STUB'
#!/usr/bin/env bash
# Stub `gh`: records argv, then serves the comments listing or absorbs a write.
n=$(( $(cat "$GH_STUB_DIR/count") + 1 ))
printf '%s' "$n" > "$GH_STUB_DIR/count"
{
    printf -- '--- call %s ---\n' "$n"
    for a in "$@"; do printf '%s\n' "${a//$'\n'/\\n}"; done
} >> "$GH_STUB_DIR/argv.log"

is_patch=0
paginate=0
filter=""
want_filter=0
prev=""
for a in "$@"; do
    if [ "$want_filter" -eq 1 ]; then filter="$a"; want_filter=0; prev="$a"; continue; fi
    case "$a" in
        --paginate) paginate=1 ;;
        --jq)       want_filter=1 ;;
        PATCH)      [ "$prev" = "-X" ] && is_patch=1 ;;
    esac
    prev="$a"
done

if [ "$is_patch" -eq 1 ]; then
    if [ -n "${GH_STUB_PATCH_FAIL:-}" ]; then
        echo "gh: HTTP 404: Not Found (stubbed)" >&2
        exit 1
    fi
    exit 0
fi

if [ "${1:-}" = "api" ] && [ -n "$filter" ]; then
    if [ -n "${GH_STUB_LIST_FAIL:-}" ]; then
        echo "gh: HTTP 502: Bad Gateway (stubbed)" >&2
        exit 1
    fi
    if [ "$paginate" -eq 1 ]; then
        # `gh api --paginate --jq` applies the filter PAGE BY PAGE and
        # concatenates the outputs. Reproduced faithfully: this is what makes
        # `| last` inside the filter wrong and `tail -n1` in shell right.
        for p in "$GH_STUB_DIR"/fixtures/page-*.json; do
            [ -e "$p" ] || continue
            jq -r "$filter" < "$p"
        done
    else
        jq -r "$filter" < "$GH_STUB_DIR/fixtures/page-1.json"
    fi
    exit 0
fi
exit 0
STUB
    chmod +x "$w/bin/gh"
    printf '%s' "$w"
}

# A comments page. $1 = workdir, $2 = page number, $3.. = raw JSON objects.
write_page() {
    local w="$1" page="$2"
    shift 2
    {
        printf '['
        local first=1 obj
        for obj in "$@"; do
            [ "$first" -eq 1 ] || printf ','
            first=0
            printf '%s' "$obj"
        done
        printf ']\n'
    } > "$w/fixtures/page-$page.json"
}

annotation_comment() {  # $1 = url
    printf '{"body":"%s\\n\\nprevious accusation","url":"%s"}' "$MARKER" "$1"
}
unrelated_comment() {   # $1 = url
    printf '{"body":"an operator comment that mentions nothing","url":"%s"}' "$1"
}

# Runs the subject (or a variant). Sets $RUN_STATUS and writes the combined
# output to `$w/out.txt`.
#
# It deliberately does NOT echo the output: a caller writing
# `OUT="$(run_subject …)"` would run the function inside a command substitution,
# i.e. a SUBSHELL, and `$RUN_STATUS` would never reach this shell — every
# exit-status assertion would then read the initial 0 and pass on a silent
# refusal. Measured on the first execution of this file, on the four assertions
# that exist precisely to prove the refusal is NOT silent.
RUN_STATUS=0
run_subject() {
    local w="$1" clean="$2" script="${3:-$SUBJECT}"
    PATH="$w/bin:$PATH" \
    GH_STUB_DIR="$w" \
    bash "$script" senara-solutions/mika 2544 "$clean" "$w/lint-out.txt" > "$w/out.txt" 2>&1
    RUN_STATUS=$?
}

# Whole-line comments, stripped. Every "this form no longer appears" assertion
# below needs this: the doctrine headers of the subject, the scan and the
# workflow all QUOTE the pre-fix invocation in order to explain it, so a scan
# over raw text accuses the documentation that carries the fix. Measured on the
# first execution of this file — the same false positive mika#2050 recorded for
# Signal S, and the term-1 discipline of the mika#2496 predicate.
strip_comments() { grep -vE '^[[:space:]]*#' "$1"; }

count_calls()   { grep -c '^--- call ' "$1/argv.log" 2>/dev/null || echo 0; }
patch_targets() { awk 'p=="PATCH" && q=="-X" { print } { q=p; p=$0 }' "$1/argv.log"; }
issue_comments() { awk '{ if (p=="issue" && $0=="comment") n++; p=$0 } END { print n+0 }' "$1/argv.log"; }
# Any recorded argument that designates an ISSUE route rather than a comment one.
issue_route_args() {
    grep -E '^https://api\.github\.com/repos/[^/]+/[^/]+/issues/' "$1/argv.log" \
        | grep -v '/issues/comments/' || true
}

echo
echo "── V5: the predicate, on the nine measured forms (AC3)"
# Sourced, so the predicate is exercised with no network and no stub. This is
# what the `if [ "${BASH_SOURCE[0]}" = "$0" ]` footer in the subject buys.
# shellcheck source=scripts/annotate-issue-token-comment.sh
. "$SUBJECT"
# The subject legitimately carries `set -euo pipefail` (its own errexit is
# load-bearing — see its header), and sourcing applied it to THIS shell. Left as
# is, the first failing assertion would abort the harness instead of being
# counted, so a partially broken subject would report a short PASSING run.
# Measured on the first execution of this file.
set +e -uo pipefail

assert_predicate() {  # $1 label, $2 expectation (accept|refuse), $3 target
    if issue_comment_patch_target_is_valid "$3"; then
        if [ "$2" = "accept" ]; then ok "$1"; else ko "$1" "accepted: '$3'"; fi
    else
        if [ "$2" = "refuse" ]; then ok "$1"; else ko "$1" "refused: '$3'"; fi
    fi
}

assert_predicate "1. the issue-comment API route is accepted" accept "$GOOD_TARGET"
assert_predicate "2. the FROZEN pre-fix target is refused (the founding defect)" \
    refuse "$PRE_FIX_TARGET"
assert_predicate "3. the bare issue route is refused" \
    refuse 'https://api.github.com/repos/senara-solutions/mika/issues/2544'
assert_predicate "4. an unrewritten html url is refused" \
    refuse 'https://github.com/senara-solutions/mika/issues/2544#issuecomment-3456789'
assert_predicate "5. a PR review-comment route is refused (different object)" \
    refuse 'https://api.github.com/repos/senara-solutions/mika/pulls/comments/3456789'
assert_predicate "6. a foreign host is refused" \
    refuse 'https://api.example.com/repos/senara-solutions/mika/issues/comments/3456789'
assert_predicate "7. a trailing query is refused" \
    refuse 'https://api.github.com/repos/senara-solutions/mika/issues/comments/3456789?body=x'
assert_predicate "8. a path traversal is refused" \
    refuse 'https://api.github.com/repos/senara-solutions/../mika/issues/comments/3456789'
assert_predicate "9. an empty target is refused" refuse ''

echo
echo "── V3a: the frozen pre-fix target does NOT contain the AC3 substring"
# The literal predicate the ticket names ("contains /issues/comments/") is
# sufficient for the measured defect; the anchored form is a robustness choice,
# not a widening. Pinned so the claim stays true rather than remembered.
if [[ "$PRE_FIX_TARGET" == */issues/comments/* ]]; then
    ko "V3a the pre-fix target must lack /issues/comments/" "it contains it — AC3's literal predicate would pass it"
else
    ok "V3a the pre-fix target lacks /issues/comments/ (AC3's literal predicate suffices)"
fi

echo
echo "── V1: a second pass on an annotated ticket PATCHES THE COMMENT (AC1)"
W="$(new_case v1)"
write_page "$W" 1 "$(unrelated_comment "$GOOD_TARGET")" \
                 "$(annotation_comment 'https://api.github.com/repos/senara-solutions/mika/issues/comments/9999')"
run_subject "$W" false
OUT="$(cat "$W/out.txt")"
assert_eq "V1 exits 0" "0" "$RUN_STATUS"
if [ "$(count_calls "$W")" -ge 1 ]; then ok "V1 the stub was called (V6 anti-vacuity)"
else ko "V1 the stub was never called" "every assertion below would be vacuous"; fi
assert_eq "V1 exactly one PATCH is sent" \
    "https://api.github.com/repos/senara-solutions/mika/issues/comments/9999" \
    "$(patch_targets "$W")"
assert_eq "V1 no new comment is stacked" "0" "$(issue_comments "$W")"
assert_eq "V1 NO argv designates an issue route" "" "$(issue_route_args "$W")"
assert_contains "V1 the annotation body carries the marker" "canonical-token-annotation" \
    "$(cat "$W/argv.log")"

echo
echo "── V2: the withdrawal path edits the SAME comment (AC2)"
W="$(new_case v2)"
write_page "$W" 1 "$(annotation_comment 'https://api.github.com/repos/senara-solutions/mika/issues/comments/8888')"
run_subject "$W" true
OUT="$(cat "$W/out.txt")"
assert_eq "V2 exits 0" "0" "$RUN_STATUS"
assert_eq "V2 exactly one PATCH, on the prior comment" \
    "https://api.github.com/repos/senara-solutions/mika/issues/comments/8888" \
    "$(patch_targets "$W")"
assert_eq "V2 no new comment is stacked" "0" "$(issue_comments "$W")"
assert_eq "V2 NO argv designates an issue route" "" "$(issue_route_args "$W")"
assert_contains "V2 the withdrawal names its cause" "_Resolved: the body now carries canonical machine tokens._" \
    "$(cat "$W/argv.log")"

echo
echo "── V2b: a clean body with NO prior annotation touches nothing at all"
W="$(new_case v2b)"
write_page "$W" 1 "$(unrelated_comment "$GOOD_TARGET")"
run_subject "$W" true
OUT="$(cat "$W/out.txt")"
assert_eq "V2b exits 0" "0" "$RUN_STATUS"
assert_eq "V2b zero PATCH" "" "$(patch_targets "$W")"
assert_eq "V2b zero comments posted" "0" "$(issue_comments "$W")"

echo
echo "── V4: the first pass posts ONE comment and ZERO PATCH"
W="$(new_case v4)"
write_page "$W" 1 "$(unrelated_comment "$GOOD_TARGET")"
run_subject "$W" false
OUT="$(cat "$W/out.txt")"
assert_eq "V4 exits 0" "0" "$RUN_STATUS"
assert_eq "V4 exactly one comment posted" "1" "$(issue_comments "$W")"
assert_eq "V4 zero PATCH" "" "$(patch_targets "$W")"
assert_contains "V4 the annotation carries the lint output" "has a space before its colon" \
    "$(cat "$W/argv.log")"

echo
echo "── V7: the prior annotation is on the SECOND page — found, and mono-line"
# `--paginate` is required (the route pages at 30) and `| last` inside the jq
# filter would emit one value PER PAGE. This case fails on either mistake: the
# target would be empty without --paginate, and a two-line absurdity with
# `| last` in the filter.
W="$(new_case v7)"
write_page "$W" 1 "$(unrelated_comment 'https://api.github.com/repos/senara-solutions/mika/issues/comments/1')" \
                 "$(annotation_comment 'https://api.github.com/repos/senara-solutions/mika/issues/comments/2')"
write_page "$W" 2 "$(unrelated_comment 'https://api.github.com/repos/senara-solutions/mika/issues/comments/3')" \
                 "$(annotation_comment 'https://api.github.com/repos/senara-solutions/mika/issues/comments/4')"
run_subject "$W" false
OUT="$(cat "$W/out.txt")"
assert_eq "V7 exits 0" "0" "$RUN_STATUS"
assert_eq "V7 the LAST annotation across pages is the target, on ONE line" \
    "https://api.github.com/repos/senara-solutions/mika/issues/comments/4" \
    "$(patch_targets "$W")"
assert_eq "V7 exactly one PATCH" "1" "$(patch_targets "$W" | grep -c .)"
assert_eq "V7 no comment is stacked" "0" "$(issue_comments "$W")"
assert_contains "V7 the resolution asks for pagination" "--paginate" "$(cat "$W/argv.log")"

echo
echo "── V7b: the resolution reads \`.url\`, never \`html_url\`"
# A fixture whose `html_url` is the html form and whose `url` is the API form.
# Reading the wrong field would produce the pre-fix target and the guard would
# refuse — so this case would go red rather than silently regress.
W="$(new_case v7b)"
write_page "$W" 1 "$(printf '{"body":"%s\\n\\nx","html_url":"https://github.com/senara-solutions/mika/issues/2544#issuecomment-7777","url":"https://api.github.com/repos/senara-solutions/mika/issues/comments/7777"}' "$MARKER")"
run_subject "$W" false
OUT="$(cat "$W/out.txt")"
assert_eq "V7b exits 0" "0" "$RUN_STATUS"
assert_eq "V7b the API url is patched" \
    "https://api.github.com/repos/senara-solutions/mika/issues/comments/7777" \
    "$(patch_targets "$W")"

echo
echo "── V3b: a REGRESSED resolution is refused end to end, and writes nothing"
W="$(new_case v3b)"
write_page "$W" 1 "$(annotation_comment "$PRE_FIX_TARGET")"
run_subject "$W" false
OUT="$(cat "$W/out.txt")"
if [ "$RUN_STATUS" -ne 0 ]; then ok "V3b the run FAILS (the Actions job goes red)"
else ko "V3b the run must fail" "exit status was 0 — the refusal is silent"; fi
assert_contains "V3b the refusal names its grep token" "issue_comment_patch_target_refused" "$OUT"
assert_contains "V3b the refusal cites the target" "$PRE_FIX_TARGET" "$OUT"
assert_contains "V3b the refusal carries the ticket" "mika#2552" "$OUT"
assert_eq "V3b ZERO PATCH was sent" "" "$(patch_targets "$W")"
assert_eq "V3b no comment was stacked either" "0" "$(issue_comments "$W")"

echo
echo "── V3b': the same refusal on the WITHDRAWAL path (AC2 + AC3 together)"
W="$(new_case v3bw)"
write_page "$W" 1 "$(annotation_comment "$PRE_FIX_TARGET")"
run_subject "$W" true
OUT="$(cat "$W/out.txt")"
if [ "$RUN_STATUS" -ne 0 ]; then ok "V3b' the withdrawal path also fails closed"
else ko "V3b' the withdrawal path must fail closed" "exit status was 0"; fi
assert_contains "V3b' names the token" "issue_comment_patch_target_refused" "$OUT"
assert_eq "V3b' ZERO PATCH was sent" "" "$(patch_targets "$W")"

echo
echo "── V3c: NEGATIVE CONTROL — a permissive predicate DOES send that PATCH"
# Without this, V3a/V3b are indistinguishable from assertions that are always
# true (mika#2103). Mutate a copy, run it against the same regressed fixture,
# and assert the PATCH now leaves.
MUTANT="$TMPROOT/mutant-permissive.sh"
sed 's/^issue_comment_patch_target_is_valid() {$/issue_comment_patch_target_is_valid() { return 0/' \
    "$SUBJECT" > "$MUTANT"
if ! grep -q 'issue_comment_patch_target_is_valid() { return 0' "$MUTANT"; then
    ko "V3c the mutant must accept everything" \
       "the sed substitution did not apply — this negative control verifies nothing"
else
    ok "V3c the mutant is the mutated shape (predicate always accepts)"
    W="$(new_case v3c)"
    write_page "$W" 1 "$(annotation_comment "$PRE_FIX_TARGET")"
    run_subject "$W" false "$MUTANT"
    OUT="$(cat "$W/out.txt")"
    assert_eq "V3c the permissive build DOES patch the issue route (so V3b was doing work)" \
        "$PRE_FIX_TARGET" "$(patch_targets "$W")"
    assert_not_contains "V3c and it names no refusal" "issue_comment_patch_target_refused" "$OUT"
fi

echo
echo "── V-R3: a failing PATCH is no longer swallowed"
W="$(new_case vr3)"
write_page "$W" 1 "$(annotation_comment "$GOOD_TARGET")"
export GH_STUB_PATCH_FAIL=1
run_subject "$W" false
unset GH_STUB_PATCH_FAIL
OUT="$(cat "$W/out.txt")"
if [ "$RUN_STATUS" -ne 0 ]; then ok "V-R3 a failing PATCH reddens the job"
else ko "V-R3 a failing PATCH must redden the job" "exit 0 — \`|| true\` is back"; fi

echo
echo "── V-R3b: a failing comments LISTING refuses rather than stacking"
W="$(new_case vr3b)"
write_page "$W" 1 "$(annotation_comment "$GOOD_TARGET")"
export GH_STUB_LIST_FAIL=1
run_subject "$W" false
unset GH_STUB_LIST_FAIL
OUT="$(cat "$W/out.txt")"
if [ "$RUN_STATUS" -ne 0 ]; then ok "V-R3b an unreadable listing fails closed"
else ko "V-R3b an unreadable listing must fail closed" "exit 0"; fi
assert_eq "V-R3b and stacks NO duplicate comment" "0" "$(issue_comments "$W")"

echo
echo "── U2 wiring: the workflow calls the script and carries no substitution"
WF_SRC="$(cat "$WORKFLOW")"
assert_contains "the workflow calls the script" \
    "bash scripts/annotate-issue-token-comment.sh" "$WF_SRC"
assert_contains "the workflow header names the ticket" "mika#2552" "$WF_SRC"

# The two "this form is gone" assertions read the COMMENT-STRIPPED text. Both the
# workflow header and the subject's header quote the pre-fix invocation and the
# swallowing redirect in order to explain them, so a raw scan accuses the
# documentation that carries the fix.
WF_CODE="$(strip_comments "$WORKFLOW")"
assert_not_contains "the workflow code no longer swallows a PATCH" \
    '>/dev/null 2>&1 || true' "$WF_CODE"

# The class DISAPPEARS rather than being corrected: no production file rewrites
# an html url into an API one. `github.com/https:` is the signature of that
# parameter expansion and of nothing else.
#
# The population is the workflows plus the subject — deliberately not this
# harness nor the scan, which name the form on purpose. Same principled exclusion
# as `_shared/` and `system_prompt.md` in check-pilot-push-sites.sh: the files
# that document a rule are not the files it polices.
#
# `grep -q` is fed by a HERE-STRING, never by a pipe: it exits on its first match,
# the producer then dies of SIGPIPE, and `set -o pipefail` turns that into a
# failure of the whole `if` — so a piped form would read the OPPOSITE of what it
# measures, reporting "no substitution found" precisely when one is found. Same
# trap as mika#2055, and the repo carries a guard for it
# (`scripts/verify-no-sigpipe-grep.sh`).
SUBST_HITS=""
for f in "$WORKFLOW" "$SUBJECT" "$REPO_ROOT"/.github/workflows/*.yml; do
    [ -r "$f" ] || continue
    F_CODE="$(strip_comments "$f")"
    if grep -qF 'github.com/https:' <<<"$F_CODE"; then
        SUBST_HITS="$SUBST_HITS$(basename "$f") "
    fi
done
assert_eq "no html→api substitution survives in the workflows or the subject" "" "$SUBST_HITS"
# Good faith: the scan must still SEE something, or it passes by looking at
# nothing (mika#2103).
if grep -q 'annotate-issue-token-comment.sh' <<<"$WF_CODE"; then
    ok "the stripped-workflow scan looks at real code"
else
    ko "the stripped-workflow scan looks at nothing" "the comment strip removed the code too"
fi

echo
echo "── Scan: it passes on the real script, with a non-vacuous population (V-b)"
SCAN_OUT="$(bash "$SCAN" 2>&1)"
SCAN_STATUS=$?
assert_eq "V-b the scan passes" "0" "$SCAN_STATUS"
assert_contains "V-b and says it scanned 2 sites" "2 PATCH sites scanned" "$SCAN_OUT"
assert_contains "V-b with an empty allowlist" "0 allowlisted" "$SCAN_OUT"

echo
echo "── V-c: NEGATIVE CONTROL on the scan — the guard removed, it must go RED"
NOGUARD="$TMPROOT/scan-fixture-no-guard.sh"
# Exact-string replacement via awk index()/substr(): the needle carries `$`, `!`
# and quotes, and escaping those through sed plus two shell quoting layers is how
# a mutation silently fails to apply.
awk -v n='if ! issue_comment_patch_target_is_valid "$prior"; then' \
    -v r='if false; then' '
    { i = index($0, n); if (i > 0) $0 = substr($0, 1, i - 1) r substr($0, i + length(n)); print }
' "$SUBJECT" > "$NOGUARD"
if grep -q 'issue_comment_patch_target_is_valid "\$prior"' "$NOGUARD"; then
    ko "V-c the fixture must carry no guard call" \
       "the awk substitution did not apply — this negative control verifies nothing"
else
    ok "V-c the fixture is the unguarded shape"
    NOGUARD_OUT="$(bash "$SCAN" "$NOGUARD" 2>&1)"
    if [ $? -ne 0 ]; then ok "V-c the scan goes RED on an unguarded PATCH"
    else ko "V-c the scan must go red on an unguarded PATCH" "$NOGUARD_OUT"; fi
    assert_contains "V-c and names the defect" "a PATCH is not guarded" "$NOGUARD_OUT"
fi

echo
echo "── V-d: NEGATIVE CONTROL on the scan — one site only, it must go RED on cardinality"
ONESITE="$TMPROOT/scan-fixture-one-site.sh"
awk '
    /^withdraw_prior_annotation\(\) \{$/ { dropping = 1; next }
    dropping && /^\}$/               { dropping = 0; next }
    dropping                          { next }
    { print }
' "$SUBJECT" > "$ONESITE"
# Counted on the COMMENT-STRIPPED text: the subject's header quotes `gh api -X
# PATCH` twice while explaining the defect, so a raw count reads 3 on a
# single-site fixture and this control would never run.
ONESITE_PATCHES="$(strip_comments "$ONESITE" | grep -c -- '-X PATCH')"
if [ "$ONESITE_PATCHES" -ne 1 ]; then
    ko "V-d the fixture must carry exactly one PATCH site" \
       "found $ONESITE_PATCHES — the awk drop did not apply as intended"
else
    ok "V-d the fixture is the single-site shape"
    ONESITE_OUT="$(bash "$SCAN" "$ONESITE" 2>&1)"
    if [ $? -ne 0 ]; then ok "V-d the scan goes RED on the wrong cardinality"
    else ko "V-d the scan must go red on the wrong cardinality" "$ONESITE_OUT"; fi
    assert_contains "V-d and names the count" "expected 2 PATCH site(s)" "$ONESITE_OUT"
fi

echo
echo "── V-e: the scan refuses a subject it cannot read (its own anti-vacuity)"
MISSING_OUT="$(bash "$SCAN" "$TMPROOT/definitely-not-here.sh" 2>&1)"
if [ $? -ne 0 ]; then ok "V-e the scan refuses an unreadable subject"
else ko "V-e the scan must refuse an unreadable subject" "$MISSING_OUT"; fi
assert_contains "V-e and says why" "vacuous pass" "$MISSING_OUT"

echo
echo "── V-f: the allowlist is shipped EMPTY, and pinned empty"
# Motif: canonical-tokens-exceptions.tsv, shipped empty and pinned empty by a
# sibling test. The day someone writes a line here, they must justify it in a
# ticket rather than in a commit.
ALLOW_DECL="$(grep -E '^ISSUE_COMMENT_PATCH_GUARD_ALLOWLIST=' "$SCAN")"
assert_eq "V-f the allowlist declaration is the empty string" \
    'ISSUE_COMMENT_PATCH_GUARD_ALLOWLIST=""' "$ALLOW_DECL"

echo
echo "─────────────────────────────────────────────"
echo "  passed: $PASS    failed: $FAIL"
if [ "$FAIL" -ne 0 ]; then exit 1; fi
exit 0

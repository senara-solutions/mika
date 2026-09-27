#!/usr/bin/env bash
# Test suite for `_shared/cwd-guard.sh` and its wiring (mika#2536, T2 / V1–V3).
#
# ─────────────────────────────────────────────────────────────────────────────
# WHAT THIS PINS, AND WHY THE ORDER IS AN ASSERTION OF ITS OWN
#
# `build-mika` crashed four times on the QA of PR #2530, every attempt, with the
# same uninformative `tasks.result`: "HANDLER CRASH (exit code 1)". The crash was
# `cd "$CWD"`, whose stderr the long-running path discards.
#
# The four refusals are asserted individually (V1, V2), and then the ORDER is
# asserted separately (V2d): a cwd holding the literal text `$MIKA_PLATFORM_DIR/…`
# is simultaneously variable-bearing, non-absolute AND non-existent. All three
# statements are true; only the first points at the prompt that produced it. A
# suite that checked the four motifs without checking which one wins would pass on
# a guard that reports "does_not_exist" for every prompt defect — which is a
# refusal that sends an operator to create a directory.
#
# ─────────────────────────────────────────────────────────────────────────────
# V3 — THE NEGATIVE CONTROL, ON A FROZEN PRE-FIX FIXTURE
#
# Without it, "the handler refuses by name" is indistinguishable from "the harness
# checks nothing". V3 rebuilds a copy of the real handler with its cwd section
# replaced by the EXACT pre-fix text (frozen below, never re-derived from git), runs
# it, and asserts it delivers the generic "HANDLER CRASH" — i.e. that the named-
# refusal assertion goes RED on the code this ticket replaced.
#
# The substitution is verified to have applied. A fixture that silently failed to
# patch would run the post-fix handler and pass, which is the vacuous form
# mika#2103 exists to refuse.
#
# ─────────────────────────────────────────────────────────────────────────────
# HOW THE HANDLERS ARE RUN WITHOUT A BUILD AND WITHOUT A REAL CALLBACK
#
# Both handlers prepend `$HOME/.local/bin` to PATH and deliver through
# `mika ask --task-id … --task-complete -- "$RESULT"`. So HOME is pointed at a
# temp dir and a stub `mika` is installed at `$HOME/.local/bin/mika`, which the
# handler's own prepend then puts first. The refusal happens BEFORE `cargo build`,
# so nothing is compiled.
#
# Run: bash scripts/test-cwd-guard.sh
# Expected: all assertions pass, exit 0.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
GUARD="$REPO_ROOT/skills/bundled/_shared/cwd-guard.sh"
BUILD_HANDLER="$REPO_ROOT/skills/bundled/build-mika/handlers/run.sh"
DEPLOY_HANDLER="$REPO_ROOT/skills/bundled/deploy-mika/handlers/run.sh"

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
for f in "$GUARD" "$BUILD_HANDLER" "$DEPLOY_HANDLER"; do
    if [ ! -r "$f" ]; then
        echo "FATAL: subject not readable: $f — this harness would verify nothing." >&2
        exit 1
    fi
done
if ! command -v jq >/dev/null 2>&1; then
    echo "FATAL: jq is required to run the handlers end to end (host dependency)." >&2
    echo "       Refusing to skip: a silent skip reads like a passing suite." >&2
    exit 1
fi

TMPROOT=$(mktemp -d /tmp/cwd-guard-test-XXXXXX)
trap 'rm -rf "$TMPROOT"' EXIT

# shellcheck source=skills/bundled/_shared/cwd-guard.sh
. "$GUARD"

echo
echo "── Wire format: the four refusal reasons have one definition site"
assert_eq "CWD_GUARD_REFUSAL_REASONS is exactly the four declared motifs" \
    "unexpanded_variable not_absolute does_not_exist not_a_directory" \
    "$CWD_GUARD_REFUSAL_REASONS"
assert_eq "the ticket is carried in every refusal" "mika#2536" "$CWD_GUARD_TICKET"

echo
echo "── POSIX discipline: the guard carries no bashism"
#
# Both callers are `#!/bin/sh`. On this development host `/bin/sh` is bash and
# `dash` is not installed, so the end-to-end runs below exercise the guard under
# bash-as-sh and CANNOT demonstrate dash-cleanliness. Asserted structurally
# instead of claimed: the five constructs that would break under dash.
#
# COMMENT LINES ARE STRIPPED FIRST, and that is load-bearing rather than tidy:
# the guard's own header NAMES the constructs it forbids ("no `[[ ]]`, no `=~`,
# no `local`, no arrays"), so a scan over the raw file accuses the documentation
# it protects. Measured on the first run of this very assertion — same class as
# the mika#2050 Signal S false positive, and the term-1 discipline of the
# mika#2508 predicate.
GUARD_CODE="$(grep -vE '^[[:space:]]*#' "$GUARD")"
BASHISMS=(
    '\[\['          # [[ … ]]
    '=~'            # regex match
    '(^|[[:space:]])local[[:space:]]'
    '(^|[[:space:]])function[[:space:]]'
    '\+='           # string/array append
)
for pattern in "${BASHISMS[@]}"; do
    # Here-string, never a pipeline: `grep -q` exits on its first match and the
    # producer then dies of SIGPIPE, which `set -o pipefail` turns into a failure
    # of the whole `if` — the assertion would then read the opposite of what it
    # measures (mika#2055, and the guard `scripts/verify-no-sigpipe-grep.sh`).
    if grep -Eq -- "$pattern" <<<"$GUARD_CODE"; then
        ko "cwd-guard.sh must carry no bashism" "found: $pattern"
    else
        ok "cwd-guard.sh carries no \`$pattern\`"
    fi
done
# Good faith: the scan must still SEE something. A stripped file that scanned
# nothing would pass all five and prove nothing (mika#2103).
if [ "$(printf '%s\n' "$GUARD_CODE" | grep -c 'validate_cwd')" -ge 1 ]; then
    ok "the bashism scan looks at real code (validate_cwd survives the strip)"
else
    ko "the bashism scan looks at nothing" "the comment strip removed the code too"
fi

echo
echo "── V1: an unexpanded variable is refused BY NAME"
if validate_cwd '$MIKA_PLATFORM_DIR/.claude/worktrees/fix-2536/mika/'; then
    ko "V1 a \$-bearing cwd must be refused" "validate_cwd returned 0"
else
    ok "V1 a \$-bearing cwd is refused"
fi
assert_contains "V1 names the motif" "unexpanded_variable" "$CWD_REFUSAL"
assert_contains "V1 cites the cwd" 'cwd=$MIKA_PLATFORM_DIR/.claude/worktrees' "$CWD_REFUSAL"
assert_contains "V1 carries the ticket" "mika#2536" "$CWD_REFUSAL"
assert_contains "V1 is prefixed for tasks.result" "REFUSED (cwd-guard," "$CWD_REFUSAL"

echo
echo "── V2: the three remaining refusals are DISTINCT and each cites the cwd"

if validate_cwd "relative/path/mika"; then
    ko "V2a a relative cwd must be refused" "validate_cwd returned 0"
else
    ok "V2a a relative cwd is refused"
fi
assert_contains "V2a names the motif" "not_absolute" "$CWD_REFUSAL"
assert_contains "V2a cites the cwd" "cwd=relative/path/mika" "$CWD_REFUSAL"

MISSING="$TMPROOT/definitely-not-here/mika"
if validate_cwd "$MISSING"; then
    ko "V2b a non-existent cwd must be refused" "validate_cwd returned 0"
else
    ok "V2b a non-existent cwd is refused"
fi
assert_contains "V2b names the motif" "does_not_exist" "$CWD_REFUSAL"
assert_contains "V2b cites the cwd" "cwd=$MISSING" "$CWD_REFUSAL"

REGULAR_FILE="$TMPROOT/a-file"
: > "$REGULAR_FILE"
if validate_cwd "$REGULAR_FILE"; then
    ko "V2c a regular file must be refused" "validate_cwd returned 0"
else
    ok "V2c a regular file is refused"
fi
assert_contains "V2c names the motif" "not_a_directory" "$CWD_REFUSAL"
assert_contains "V2c cites the cwd" "cwd=$REGULAR_FILE" "$CWD_REFUSAL"

echo
echo "── V2d: THE ORDER. Variable-literal wins over its two true-but-useless siblings"
# This candidate is variable-bearing AND non-absolute AND non-existent.
validate_cwd '$MIKA_PLATFORM_DIR/mika' || true
assert_contains "V2d reports unexpanded_variable" "unexpanded_variable" "$CWD_REFUSAL"
assert_not_contains "V2d does NOT report does_not_exist" "does_not_exist" "$CWD_REFUSAL"
assert_not_contains "V2d does NOT report not_absolute" "not_absolute" "$CWD_REFUSAL"

echo
echo "── Nominal: a real directory passes and leaves no refusal behind"
GOOD="$TMPROOT/good-worktree"
mkdir -p "$GOOD"
CWD_REFUSAL="stale-value-from-a-previous-call"
if validate_cwd "$GOOD"; then ok "an existing directory is accepted"
else ko "an existing directory must be accepted" "refusal: $CWD_REFUSAL"; fi
assert_eq "CWD_REFUSAL is cleared on the nominal path" "" "$CWD_REFUSAL"

echo
echo "── errexit: under \`set -e\`, the production call shape survives a refusal"
#
# Both handlers run under `set -e` and deliver their `RESULT` from an EXIT trap.
# A refusal that aborted the shell before the caller's `RESULT="$CWD_REFUSAL"`
# would leave the trap composing the generic "HANDLER CRASH" — the defect
# reproduced by the mechanism meant to close it.
#
# The property asserted is the one that actually holds: in the shape both callers
# use (`if ! validate_cwd …`, where POSIX suspends errexit for the whole call),
# the shell survives AND the refusal is composed.
#
# A BARE `validate_cwd "$CWD"` is deliberately NOT asserted to survive: the
# function signals by exit status, so under errexit it aborts the caller whatever
# the guard does internally. That is the call site's responsibility, stated in
# the guard's own header, and the assertion below is what pins the shape the two
# production sites use.
SET_E_OUT="$(
    set -e
    # shellcheck disable=SC1090
    . "$GUARD"
    if ! validate_cwd '$MIKA_PLATFORM_DIR/mika'; then
        printf 'SURVIVED:%s' "$CWD_REFUSAL"
    fi
)"
assert_contains "the \`if ! …\` shape survives a refusal under set -e" \
    "SURVIVED:" "$SET_E_OUT"
assert_contains "and the refusal is fully composed by then" \
    "unexpanded_variable" "$SET_E_OUT"

echo
echo "── Wiring: both handlers source the guard and call it"
BUILD_SRC="$(cat "$BUILD_HANDLER")"
DEPLOY_SRC="$(cat "$DEPLOY_HANDLER")"
assert_contains "build-mika sources the shared guard" "_shared/cwd-guard.sh" "$BUILD_SRC"
assert_contains "build-mika calls validate_cwd" "validate_cwd" "$BUILD_SRC"
assert_contains "deploy-mika sources the shared guard" "_shared/cwd-guard.sh" "$DEPLOY_SRC"
assert_contains "deploy-mika calls validate_cwd" "validate_cwd" "$DEPLOY_SRC"
# mika#2536 L3: the shared guard ADDS to deploy-mika's allowed-prefix perimeter,
# it never replaces it. Weakening a safety perimeter would be a side effect of an
# observability ticket.
assert_contains "deploy-mika KEEPS its allowed-prefix case" \
    'path not under allowed prefix' "$DEPLOY_SRC"

# ── Run the real handler with a fabricated environment.
#
# Returns the RESULT the stub `mika` received. `$1` is the handler script,
# `$2` the cwd to feed it.
#
# The stub's output channel is deliberately NOT `MIKA_`-prefixed: `deploy-mika`
# unsets every `MIKA_*` variable before delivering its callback, so a prefixed
# name would be scrubbed and the harness would read an empty result — measured,
# and a small live demonstration of the very scrub that makes mika#2536's relay
# necessary.
run_handler() {
    local handler="$1" cwd="$2" home stub out
    home="$TMPROOT/home-$RANDOM"
    mkdir -p "$home/.local/bin"
    stub="$home/.local/bin/mika"
    cat > "$stub" <<'STUB'
#!/bin/sh
# Records the last positional argument — the RESULT the handler delivered.
while [ "$#" -gt 1 ]; do shift; done
printf '%s' "$1" > "$CWD_GUARD_STUB_OUT"
STUB
    chmod +x "$stub"
    out="$home/result.txt"
    : > "$out"
    printf '{"cwd":%s,"__mika_task_id":"t-2536","__mika_agent":"mika-qa"}\n' \
        "$(printf '%s' "$cwd" | jq -Rs .)" \
        | env HOME="$home" CWD_GUARD_STUB_OUT="$out" sh "$handler" >/dev/null 2>&1
    cat "$out"
}

echo
echo "── End to end: the refusal reaches the callback body (build-mika)"
E2E="$(run_handler "$BUILD_HANDLER" '$MIKA_PLATFORM_DIR/.claude/worktrees/x/mika/')"
# Non-emptiness FIRST: `assert_not_contains` passes vacuously on an empty result,
# so a handler that died before delivering anything would read as a pass on that
# line alone.
if [ -n "$E2E" ]; then ok "build-mika delivered a non-empty callback body"
else ko "build-mika delivered nothing" "the two assertions below would be vacuous"; fi
assert_contains "build-mika delivers the named refusal" "unexpanded_variable" "$E2E"
assert_not_contains "build-mika no longer delivers the generic crash" \
    "HANDLER CRASH" "$E2E"

echo
echo "── End to end: the refusal reaches the callback body (deploy-mika)"
E2E_DEPLOY="$(run_handler "$DEPLOY_HANDLER" '$MIKA_PLATFORM_DIR/mika')"
if [ -n "$E2E_DEPLOY" ]; then ok "deploy-mika delivered a non-empty callback body"
else ko "deploy-mika delivered nothing" "the two assertions below would be vacuous"; fi
assert_contains "deploy-mika delivers the named refusal" "unexpanded_variable" "$E2E_DEPLOY"
assert_not_contains "deploy-mika no longer delivers the generic crash" \
    "HANDLER CRASH" "$E2E_DEPLOY"

echo
echo "── V3: NEGATIVE CONTROL — the pre-fix handler delivers the generic crash"
#
# Frozen fixture: the EXACT text `build-mika/handlers/run.sh` carried before
# mika#2536 (lines 58-65 at d4514180). Never re-derived from git — a fixture that
# refetches loses the very form it exists to remember.
PRE_FIX_BLOCK='# Use provided cwd or default to the main mika repo root
if [ -z "$CWD" ]; then
    _DEFAULT="${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}/mika"
    CWD=$(cd "$_DEFAULT" 2>/dev/null && pwd -P) || CWD="$_DEFAULT"
fi

# Run the build
cd "$CWD" || { echo "ERROR: could not cd to $CWD" >&2; exit 1; }'

PRE_FIX_HANDLER="$TMPROOT/prefix-build-mika.sh"
# Replace everything from the guard-source block to the `cd` line with the
# pre-fix text, using awk so no regex metacharacter in the block can bite.
awk -v block="$PRE_FIX_BLOCK" '
    /^# --- cwd composability guard \(mika#2536\) ---$/ { print block; skipping = 1; next }
    skipping && /^cd "\$CWD"/ { skipping = 0; next }
    skipping { next }
    { print }
' "$BUILD_HANDLER" > "$PRE_FIX_HANDLER"

# The substitution MUST have applied: a fixture that silently failed to patch
# would run the post-fix handler and pass.
if grep -q 'validate_cwd' "$PRE_FIX_HANDLER"; then
    ko "V3 fixture must not carry the guard call" \
       "the awk substitution did not apply — this negative control verifies nothing"
elif ! grep -q 'MIKA_PLATFORM_DIR' "$PRE_FIX_HANDLER"; then
    ko "V3 fixture must carry the pre-fix dead branch" \
       "the awk substitution did not apply — this negative control verifies nothing"
else
    ok "V3 fixture is the pre-fix shape (no guard call, dead MIKA_ branch present)"
fi

chmod +x "$PRE_FIX_HANDLER"
V3_OUT="$(run_handler "$PRE_FIX_HANDLER" '$MIKA_PLATFORM_DIR/.claude/worktrees/x/mika/')"
assert_contains "V3 the pre-fix handler delivers the GENERIC crash" "HANDLER CRASH" "$V3_OUT"
assert_not_contains "V3 the pre-fix handler names no motif" "unexpanded_variable" "$V3_OUT"

echo
echo "── N1/N2: NEGATIVE CONTROLS on the guard itself — mutate a copy, see RED"
#
# Shape borrowed verbatim from `test_pr_push_guard.sh` N1–N3: mutate a copy of the
# guard, source it in a SUBSHELL, and assert the matching assertion above goes
# red. Without these, "the guard refuses" is indistinguishable from "the harness
# asserts something that is always true" (mika#2103).
#
# A subshell, because sourcing a mutated copy in this shell would clobber the
# real `validate_cwd` for every assertion that follows.

# N1 — neutralize the `unexpanded_variable` case PATTERN so its arm becomes
# unreachable. The candidate must then fall through to a DIFFERENT name, which is
# what proves test 1 is what decides V2d's outcome rather than test ordering luck.
#
# Exact-STRING replacement, never a regex: the pattern being neutralized is
# `*'$'*)`, four of whose six characters are regex metacharacters, and escaping
# them through awk and two shell quoting layers is how a mutation silently fails
# to apply. `index()` + `substr()` has no metacharacters at all.
MUTANT_N1="$TMPROOT/mutant-n1.sh"
N1_NEEDLE="*'\$'*)"
N1_REPL="*__never_matches_2536__*)"
awk -v n="$N1_NEEDLE" -v r="$N1_REPL" '
    {
        i = index($0, n)
        if (i > 0) $0 = substr($0, 1, i - 1) r substr($0, i + length(n))
        print
    }
' "$GUARD" > "$MUTANT_N1"

if ! grep -qF "$N1_REPL" "$MUTANT_N1"; then
    ko "N1 mutant must have its variable-case pattern neutralized" \
       "the awk substitution did not apply — this negative control verifies nothing"
else
    ok "N1 mutant is the mutated shape (variable-case pattern neutralized)"
    N1_OUT="$(
        # shellcheck disable=SC1090
        . "$MUTANT_N1"
        validate_cwd '$MIKA_PLATFORM_DIR/mika' >/dev/null 2>&1 || true
        printf '%s' "$CWD_REFUSAL"
    )"
    assert_not_contains "N1 the motif is gone once its case is removed" \
        "unexpanded_variable" "$N1_OUT"
    assert_contains "N1 the candidate falls through to a DIFFERENT name" \
        "not_absolute" "$N1_OUT"
fi

# N2 — make `validate_cwd` return 0 unconditionally. Every refusal assertion
# above must then fail, which is what proves they are load-bearing.
MUTANT_N2="$TMPROOT/mutant-n2.sh"
sed 's/^validate_cwd() {$/validate_cwd() { CWD_REFUSAL=""; return 0/' "$GUARD" > "$MUTANT_N2"

if ! grep -q 'validate_cwd() { CWD_REFUSAL=""; return 0' "$MUTANT_N2"; then
    ko "N2 mutant must accept everything" \
       "the sed substitution did not apply — this negative control verifies nothing"
else
    ok "N2 mutant is the mutated shape (validate_cwd always accepts)"
    if (
        # shellcheck disable=SC1090
        . "$MUTANT_N2"
        validate_cwd '$MIKA_PLATFORM_DIR/mika' >/dev/null 2>&1
    ); then
        ok "N2 a permissive guard ACCEPTS the \$-bearing cwd (so V1 was doing work)"
    else
        ko "N2 a permissive guard must accept everything" \
           "the mutation did not take effect — V1 may be asserting something always true"
    fi
fi

echo
echo "─────────────────────────────────────────────"
echo "  passed: $PASS    failed: $FAIL"
if [ "$FAIL" -ne 0 ]; then exit 1; fi
exit 0

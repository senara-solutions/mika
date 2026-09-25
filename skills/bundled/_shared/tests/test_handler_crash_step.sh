#!/bin/bash
# Test suite for the crash-step naming of long-running handlers (mika#2532 R2/R3).
#
# The defect: a handler that crashed before building its RESULT delivered
#
#     HANDLER CRASH (exit code 1). Script failed before building result.
#
# — four times in a row during the QA of PR #2530, on four different attempts,
# naming no cause at all. The step name is what turns that into an actionable
# line; `$_STEP_DETAIL` carries the specifics for the failures we saw coming.
#
# `mika` and `cargo` are stubbed on PATH, and `$HOME` is redirected to a temp
# dir so the handler's own `export PATH="$HOME/.local/bin:$PATH"` cannot
# resolve a real binary ahead of the stubs. Every callback is journalled so the
# assertions read the exact RESULT the handler produced.
#
# Run: bash skills/bundled/_shared/tests/test_handler_crash_step.sh
# Expected: all assertions pass, exit 0.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUNDLED_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

PASS=0
FAIL=0

assert_contains() {
    local label="$1" haystack="$2" needle="$3"
    if grep -qF -- "$needle" <<<"$haystack"; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    missing: '$needle'"
        echo "    in:      '$haystack'"
    fi
}

assert_not_contains() {
    local label="$1" haystack="$2" needle="$3"
    if grep -qF -- "$needle" <<<"$haystack"; then
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    unexpectedly present: '$needle'"
        echo "    in:                   '$haystack'"
    else
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    fi
}

assert_eq() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    expected: '$expected'"
        echo "    actual:   '$actual'"
    fi
}

command -v jq >/dev/null 2>&1 || { echo "SKIP: jq not installed"; exit 0; }

STUB_DIR=$(mktemp -d)
FAKE_HOME=$(mktemp -d)
trap 'rm -rf "$STUB_DIR" "$FAKE_HOME"' EXIT

CALLBACK_LOG="$STUB_DIR/callback.txt"

# `mika` stub — records the LAST argument, which is the RESULT the handler
# hands to `mika ask --task-id … --task-complete -- "$RESULT"`.
cat > "$STUB_DIR/mika" <<'STUB'
#!/bin/bash
printf '%s' "${*: -1}" > "$CALLBACK_LOG"
exit 0
STUB
chmod +x "$STUB_DIR/mika"

# `cargo` stub — the nominal path must not spend four minutes building.
cat > "$STUB_DIR/cargo" <<'STUB'
#!/bin/bash
echo "Finished `release` profile"
exit 0
STUB
chmod +x "$STUB_DIR/cargo"

export CALLBACK_LOG
export PATH="$STUB_DIR:$PATH"

# Run a handler with the given stdin JSON and echo the RESULT it delivered.
run_handler() {
    local handler="$1" input="$2"
    : > "$CALLBACK_LOG"
    HOME="$FAKE_HOME" printf '%s' "$input" | HOME="$FAKE_HOME" sh "$handler" >/dev/null 2>&1
    cat "$CALLBACK_LOG"
}

BUILD_HANDLER="$BUNDLED_DIR/build-mika/handlers/run.sh"

echo "=== crash-step naming (mika#2532) ==="

# ── S1: exact replay of the measured defect ──────────────────────────────────
# A `cwd` that does not exist is the failure of the four 2026-09-25 crashes.
echo "-- S1: a chdir failure names its step and its path --"
MISSING_CWD="/nope/mika-2532-does-not-exist"
result=$(run_handler "$BUILD_HANDLER" \
    "{\"__mika_task_id\": \"t-2532\", \"cwd\": \"$MISSING_CWD\"}")

assert_contains "names the step" "$result" "at step 'chdir'"
assert_contains "names the faulty path" "$result" "$MISSING_CWD"
# S4 — the prefix is a wire format: `self-dev-callback` documents it as a
# discriminant and `dispatch-lib.sh` greps it. The step is ADDED, not
# substituted (mika#2532 D5).
assert_eq "the prefix is preserved, at column zero" \
    "HANDLER CRASH" "$(printf '%s' "$result" | head -c 13)"
assert_contains "carries the exit code" "$result" "(exit code 1)"
# The generic sentence the ticket was filed about must be gone.
assert_not_contains "the causeless sentence is gone" "$result" \
    "Script failed before building result"

# ── S2: negative control — the nominal path says nothing about a crash ───────
# Without this, "the handler names its crashes" is indistinguishable from "the
# handler always claims a crash".
echo "-- S2: the nominal path carries no crash message --"
GOOD_CWD=$(mktemp -d)
result=$(run_handler "$BUILD_HANDLER" \
    "{\"__mika_task_id\": \"t-2532\", \"cwd\": \"$GOOD_CWD\"}")
rm -rf "$GOOD_CWD"

assert_not_contains "no HANDLER CRASH on success" "$result" "HANDLER CRASH"
assert_contains "the build result is delivered instead" "$result" "Build succeeded"

# ── S3: form — the trap precedes every step that a trap could cover ──────────
#
# R3's property, asserted structurally rather than by replay: a step that runs
# before `trap deliver_callback EXIT` fails with NO callback at all. Only the
# two regions that run before `TASK_ID` is known may sit ahead of it — nothing
# can cover those, which is why R1 persists their stderr instead.
PRE_TRAP_ALLOWED="deps parse_input"

# Echoes the offending step names, or nothing when the file is conformant.
steps_before_trap() {
    local file="$1"
    local trap_line
    trap_line=$(grep -n '^trap deliver_callback EXIT' "$file" | head -1 | cut -d: -f1)
    if [ -z "$trap_line" ]; then
        echo "__NO_TRAP__"
        return
    fi
    local n name
    while IFS= read -r entry; do
        n=${entry%%:*}
        [ "$n" -lt "$trap_line" ] || continue
        name=$(printf '%s' "${entry#*:}" | sed -E 's/.*_STEP="([^"]*)".*/\1/')
        case " $PRE_TRAP_ALLOWED " in
            *" $name "*) ;;
            *) echo "$name" ;;
        esac
    done < <(grep -n '^[[:space:]]*_STEP="' "$file")
}

echo "-- S3: the trap is armed before any coverable step --"
handlers_checked=0
for skill in build-mika deploy-mika address-pr-comments resolve-pr-conflicts; do
    file="$BUNDLED_DIR/$skill/handlers/run.sh"
    offenders=$(steps_before_trap "$file")
    assert_eq "$skill: no step runs before the trap" "" "$offenders"
    # Anti-vacuity: a file with no `_STEP` at all would pass the check above
    # while naming nothing — a silently inert assertion reads exactly like a
    # clean one (mika#2205).
    n_steps=$(grep -c '^[[:space:]]*_STEP="' "$file")
    if [ "$n_steps" -ge 3 ]; then
        PASS=$((PASS + 1))
        echo "  ✓ $skill: names its steps ($n_steps sites)"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $skill: only $n_steps _STEP sites — the check above verifies nothing"
    fi
    handlers_checked=$((handlers_checked + 1))
done
assert_eq "all four handlers were checked" "4" "$handlers_checked"

# ── S3-N1: the negative control of the form check, mechanized ────────────────
#
# The check above is worth exactly what it catches. This fixture reproduces the
# shape `deploy-mika` actually had before mika#2532 — a real step resolved
# ahead of the trap — and the check must refuse it.
echo "-- S3-N1: the form check refuses a step placed before the trap --"
FIXTURE="$STUB_DIR/pre-trap-step.sh"
sed 's|^trap deliver_callback EXIT|_STEP="resolve_cwd"\ntrap deliver_callback EXIT|' \
    "$BUILD_HANDLER" > "$FIXTURE"
assert_eq "the fixture is caught" "resolve_cwd" "$(steps_before_trap "$FIXTURE")"

# ── S3-N2: and it refuses a handler with no trap at all ─────────────────────
FIXTURE2="$STUB_DIR/no-trap.sh"
grep -v '^trap deliver_callback EXIT' "$BUILD_HANDLER" > "$FIXTURE2"
assert_eq "a handler with no trap is caught" "__NO_TRAP__" "$(steps_before_trap "$FIXTURE2")"

echo
echo "PASS: $PASS  FAIL: $FAIL"
[ "$FAIL" -eq 0 ] || exit 1

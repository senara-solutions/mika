#!/usr/bin/env bash
# Behavioural harness for skills/bundled/build-mika/handlers/run.sh (mika#2532).
#
# ─────────────────────────────────────────────────────────────────────────────
# WHAT IT PINS
#
# The handler crashed four times on the QA of PR #2530, every attempt, with one
# sentence in `tasks.result`:
#
#     HANDLER CRASH (exit code 1). Script failed before building result.
#
# That sentence was compatible with four different causes and named none of
# them. Since mika#2532 every pre-result failure carries its STAGE and, for the
# cwd family, the offending path VERBATIM together with where it came from.
#
# ─────────────────────────────────────────────────────────────────────────────
# THE NEGATIVE CONTROL IS THE LOAD-BEARING HALF
#
# Case N1 runs the *pre-fix* shape of the handler through the same assertions.
# It must produce the generic message and therefore FAIL the stage assertion.
# Without it, "the RESULT names its stage" would be indistinguishable from "the
# assertion matches anything" — a harness that has never been watched go red is
# a decoration (mika#2103).
#
# ─────────────────────────────────────────────────────────────────────────────
# HOW THE HANDLER IS DRIVEN
#
# `HOME` is redirected to a scratch tree, because the handler prepends
# `$HOME/.local/bin` to PATH — with the real HOME, a real `mika` would win over
# the stub and the harness would deliver its callback into the live loop.
# `mika` and `cargo` are stubs; `jq` is the real one (a host dependency of every
# handler in this repo).
#
# Run: bash scripts/test-build-mika-handler.sh
# Expected: all cases pass, exit 0.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HANDLER="$REPO_ROOT/skills/bundled/build-mika/handlers/run.sh"

PASS=0
FAIL=0

TMPROOT="$(mktemp -d /tmp/build-mika-handler-test-XXXXXX)"
trap 'rm -rf "$TMPROOT"' EXIT

ok() { PASS=$((PASS + 1)); echo "  ✓ $1"; }
ko() {
    FAIL=$((FAIL + 1))
    echo "  ✗ $1"
    shift
    for line in "$@"; do echo "    $line"; done
}

command -v jq >/dev/null 2>&1 || {
    echo "FATAL: jq is required to drive the handler (it is a host dependency)" >&2
    exit 2
}

# ─────────────────────────────────────────────────────────────────────────────
# Fixture: the PRE-FIX handler, reduced to the shape that produced the measured
# crash — trap installed AFTER the cwd block, one generic message, no stage.
# Kept verbatim in spirit rather than refreshed from git: refreshing it would
# erase the very form the guard exists to refuse.
# ─────────────────────────────────────────────────────────────────────────────
PRE_FIX_HANDLER="$TMPROOT/pre-fix-run.sh"
cat >"$PRE_FIX_HANDLER" <<'PREFIX'
#!/bin/sh
set -e
export PATH="$HOME/.local/bin:$PATH"
command -v jq >/dev/null 2>&1 || { echo "Error: jq is required but not installed" >&2; exit 1; }
command -v mika >/dev/null 2>&1 || { echo "Error: mika CLI is required but not in PATH" >&2; exit 1; }
INPUT=$(cat)
TASK_ID=$(printf '%s\n' "$INPUT" | jq -r '.__mika_task_id // empty')
AGENT=$(printf '%s\n' "$INPUT" | jq -r '.__mika_agent // empty')
CWD=$(printf '%s\n' "$INPUT" | jq -r '.cwd // empty')
if [ -z "$TASK_ID" ]; then
    echo "Error: no __mika_task_id in input" >&2
    exit 1
fi
CALLBACK_SENT=0
deliver_callback() {
    _EXIT_CODE=$?
    [ "$CALLBACK_SENT" -eq 1 ] && return
    [ -z "$TASK_ID" ] && return
    if [ -z "$RESULT" ]; then
        RESULT="HANDLER CRASH (exit code ${_EXIT_CODE}). Script failed before building result."
    fi
    RESULT=$(printf '%s' "$RESULT" | head -c 92000)
    set +e
    mika ask --task-id "$TASK_ID" --task-complete -- "$RESULT"
    CALLBACK_SENT=1
    set -e
}
trap deliver_callback EXIT
if [ -z "$CWD" ]; then
    _DEFAULT="${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}/mika"
    CWD=$(cd "$_DEFAULT" 2>/dev/null && pwd -P) || CWD="$_DEFAULT"
fi
cd "$CWD" || { echo "ERROR: could not cd to $CWD" >&2; exit 1; }
RESULT="Build succeeded (cwd: ${CWD})."
PREFIX
chmod +x "$PRE_FIX_HANDLER"

# ─────────────────────────────────────────────────────────────────────────────
# Harness plumbing
# ─────────────────────────────────────────────────────────────────────────────

CASE_N=0
EXTRA_ENV=()

# setup_case [cargo-exit]
#   Builds a scratch HOME with `mika` and `cargo` stubs, and echoes its root.
setup_case() {
    local cargo_exit="${1:-0}"
    CASE_N=$((CASE_N + 1))
    EXTRA_ENV=()
    local root="$TMPROOT/case$CASE_N"
    mkdir -p "$root/bin" "$root/.local/bin"

    # The `mika` stub records the delivered RESULT — the last positional
    # argument, after the `--` separator.
    cat >"$root/bin/mika" <<'MIKA'
#!/bin/sh
_last=""
for _a in "$@"; do _last="$_a"; done
printf '%s' "$_last" > "$MIKA_STUB_RESULT_FILE"
exit 0
MIKA
    chmod +x "$root/bin/mika"

    cat >"$root/bin/cargo" <<CARGO
#!/bin/sh
echo "stub cargo: \$*"
exit $cargo_exit
CARGO
    chmod +x "$root/bin/cargo"

    printf '%s' "$root"
}

# run_handler <handler> <case-root> <input-json>
#   Drives the handler and leaves the delivered RESULT in LAST_RESULT (empty
#   when nothing was delivered), the stderr in LAST_STDERR, the code in LAST_RC.
#   Extra `NAME=value` assignments go in EXTRA_ENV, reset by every setup_case.
run_handler() {
    local handler="$1" root="$2" input="$3"
    local result_file="$root/result.txt" stderr_file="$root/stderr.txt"
    rm -f "$result_file"

    printf '%s' "$input" | env -i \
        HOME="$root" \
        PATH="$root/bin:/usr/bin:/bin" \
        MIKA_STUB_RESULT_FILE="$result_file" \
        "${EXTRA_ENV[@]}" \
        sh "$handler" >/dev/null 2>"$stderr_file"
    LAST_RC=$?
    LAST_RESULT="$(cat "$result_file" 2>/dev/null || true)"
    LAST_STDERR="$(cat "$stderr_file" 2>/dev/null || true)"
}

# json_input <cwd>
json_input() {
    jq -nc --arg cwd "$1" \
        '{__mika_task_id: "t-2532", __mika_agent: "mika-qa", cwd: $cwd}'
}

# Does the RESULT name a stage at all? This is the predicate the negative
# control must falsify.
names_a_stage() { [[ "$1" == *"at stage '"* ]]; }

echo "mika#2532 — build-mika handler: stage-naming and cwd refusals"
echo

# ── V2: a literal '$' is refused UNDER ITS OWN NAME ──────────────────────────
#
# This is the measured F5 hypothesis: the prompt prescribed
# `$MIKA_PLATFORM_DIR/.claude/worktrees/<branch>/mika/` and nothing expands it.
# The refusal must say so — and must NOT fall through to `cwd_not_absolute`,
# which is also true of that string and far less actionable.
root="$(setup_case 0)"
run_handler "$HANDLER" "$root" "$(json_input '$MIKA_PLATFORM_DIR/.claude/worktrees/fix-2517/mika/')"
if names_a_stage "$LAST_RESULT" \
    && [[ "$LAST_RESULT" == *"enter_cwd"* ]] \
    && [[ "$LAST_RESULT" == *"cwd_unexpanded_variable"* ]] \
    && [[ "$LAST_RESULT" == *'$MIKA_PLATFORM_DIR/.claude/worktrees/fix-2517/mika/'* ]]; then
    ok "V2 — a literal \$ is refused as cwd_unexpanded_variable, citing the path verbatim"
else
    ko "V2 — expected cwd_unexpanded_variable naming the verbatim path" "RESULT: $LAST_RESULT"
fi

# ── V1: a nonexistent cwd names the stage AND the path ──────────────────────
root="$(setup_case 0)"
ABSENT="$TMPROOT/definitely-absent-$RANDOM"
run_handler "$HANDLER" "$root" "$(json_input "$ABSENT")"
if names_a_stage "$LAST_RESULT" \
    && [[ "$LAST_RESULT" == *"enter_cwd"* ]] \
    && [[ "$LAST_RESULT" == *"cwd_not_found"* ]] \
    && [[ "$LAST_RESULT" == *"$ABSENT"* ]]; then
    ok "V1 — a nonexistent cwd is refused as cwd_not_found, citing the path"
else
    ko "V1 — expected cwd_not_found naming the path" "RESULT: $LAST_RESULT"
fi

# ── The provenance rides on the refusal ─────────────────────────────────────
if [[ "$LAST_RESULT" == *"the tool's 'cwd' argument"* ]]; then
    ok "the refusal names the PROVENANCE of the path, not only its value"
else
    ko "expected the refusal to name where the path came from" "RESULT: $LAST_RESULT"
fi

# ── A relative cwd ──────────────────────────────────────────────────────────
root="$(setup_case 0)"
run_handler "$HANDLER" "$root" "$(json_input 'relative/worktrees/mika')"
if [[ "$LAST_RESULT" == *"cwd_not_absolute"* ]] && [[ "$LAST_RESULT" == *"relative/worktrees/mika"* ]]; then
    ok "a relative cwd is refused as cwd_not_absolute"
else
    ko "expected cwd_not_absolute" "RESULT: $LAST_RESULT"
fi

# ── A cwd that exists but is a regular file ─────────────────────────────────
root="$(setup_case 0)"
NOT_A_DIR="$root/a-file"
touch "$NOT_A_DIR"
run_handler "$HANDLER" "$root" "$(json_input "$NOT_A_DIR")"
if [[ "$LAST_RESULT" == *"cwd_not_a_directory"* ]] && [[ "$LAST_RESULT" == *"$NOT_A_DIR"* ]]; then
    ok "a file passed as cwd is refused as cwd_not_a_directory"
else
    ko "expected cwd_not_a_directory" "RESULT: $LAST_RESULT"
fi

# ── Good faith: a valid cwd reaches the build and reports its outcome ───────
#
# Without this the four refusals above would be satisfied by a handler that
# refuses everything.
root="$(setup_case 0)"
GOOD="$root/worktree"
mkdir -p "$GOOD"
run_handler "$HANDLER" "$root" "$(json_input "$GOOD")"
if [[ "$LAST_RESULT" == "Build succeeded"* ]] && [[ "$LAST_RESULT" == *"$GOOD"* ]]; then
    ok "good faith — a valid cwd reaches the build and reports success"
else
    ko "expected 'Build succeeded' naming the cwd" "RESULT: $LAST_RESULT"
fi

root="$(setup_case 1)"
GOOD="$root/worktree"
mkdir -p "$GOOD"
run_handler "$HANDLER" "$root" "$(json_input "$GOOD")"
if [[ "$LAST_RESULT" == "Build FAILED"* ]] && ! names_a_stage "$LAST_RESULT"; then
    ok "good faith — a failing build is 'Build FAILED', never a HANDLER CRASH"
else
    ko "a failing build must not read as a pre-result crash" "RESULT: $LAST_RESULT"
fi

# ── The irreducibles are named, not hidden ──────────────────────────────────
#
# No __mika_task_id: no callback is possible at all. The harness asserts the
# SILENCE, so the header's claim stays true rather than aspirational.
root="$(setup_case 0)"
run_handler "$HANDLER" "$root" '{"cwd": "/tmp"}'
if [[ -z "$LAST_RESULT" ]] && [[ "$LAST_STDERR" == *"__mika_task_id"* ]]; then
    ok "no task id — nothing is delivered, and the stderr says why (L1 carries it)"
else
    ko "expected silence on the callback and a named stderr" "RESULT: $LAST_RESULT" "STDERR: $LAST_STDERR"
fi

root="$(setup_case 0)"
rm -f "$root/bin/mika"
run_handler "$HANDLER" "$root" "$(json_input "$root")"
if [[ -z "$LAST_RESULT" ]] && [[ "$LAST_STDERR" == *"mika CLI is required"* ]]; then
    ok "mika absent — nothing is delivered, and the stderr says why (L1 carries it)"
else
    ko "expected silence on the callback and a named stderr" "RESULT: $LAST_RESULT" "STDERR: $LAST_STDERR"
fi

# ── N1: THE NEGATIVE CONTROL — the pre-fix handler must FAIL the assertion ──
root="$(setup_case 0)"
ABSENT="$TMPROOT/definitely-absent-prefix-$RANDOM"
run_handler "$PRE_FIX_HANDLER" "$root" "$(json_input "$ABSENT")"
if [[ -n "$LAST_RESULT" ]] \
    && ! names_a_stage "$LAST_RESULT" \
    && [[ "$LAST_RESULT" == *"Script failed before building result"* ]] \
    && [[ "$LAST_RESULT" != *"$ABSENT"* ]]; then
    ok "N1 negative control — the pre-fix handler yields the generic message and fails the stage assertion"
else
    ko "N1 — the pre-fix fixture no longer reproduces the measured defect" \
        "This harness can no longer be shown to go red; repair the fixture." \
        "RESULT: $LAST_RESULT"
fi

# ── L4a: the relayed PLATFORM_DIR is what composes the default ──────────────
#
# The SOURCE side (no `MIKA_*` read survives the sandbox) is a class, and
# `scripts/check-sandboxed-handler-env.sh` owns it. What only a real run can
# show is the CONSUMER side: that the relayed name is the one actually read.
# Both terms are needed — the relay reaching the child buys nothing if the
# handler still composes its default from the name that never arrives.
root="$(setup_case 0)"
EXTRA_ENV=("PLATFORM_DIR=$root/relayed-platform")
run_handler "$HANDLER" "$root" '{"__mika_task_id": "t-2532", "__mika_agent": "mika-qa"}'
if [[ "$LAST_RESULT" == *"$root/relayed-platform/mika"* ]] \
    && [[ "$LAST_RESULT" == *"the resolved default"* ]]; then
    ok "L4a — the relayed PLATFORM_DIR composes the default, and its provenance is named"
else
    ko "expected the default to be composed from the relayed PLATFORM_DIR" "RESULT: $LAST_RESULT"
fi

# Negative control for the term above: with MIKA_PLATFORM_DIR set and
# PLATFORM_DIR absent, the handler must NOT honour it — that is exactly the
# dead branch mika#2532 removed, and honouring it would mean the fix is a
# rename rather than a relay.
root="$(setup_case 0)"
EXTRA_ENV=("MIKA_PLATFORM_DIR=$root/should-be-ignored")
run_handler "$HANDLER" "$root" '{"__mika_task_id": "t-2532", "__mika_agent": "mika-qa"}'
if [[ "$LAST_RESULT" != *"should-be-ignored"* ]] && [[ "$LAST_RESULT" == *"$root/workspace/mika-platform/mika"* ]]; then
    ok "L4a negative control — MIKA_PLATFORM_DIR is no longer read by the handler"
else
    ko "the handler still honours MIKA_PLATFORM_DIR — a name the sandbox never delivers" "RESULT: $LAST_RESULT"
fi

echo
echo "  passed: $PASS   failed: $FAIL"
[[ "$FAIL" -eq 0 ]] || exit 1

#!/bin/sh
# Build handler for the build-mika skill.
# Input: JSON on stdin with __mika_task_id and __mika_agent (injected by executor for long-running)
# Output: Delivers result via `mika ask --task-id` callback
#
# This is a long-running handler: stdout goes to /dev/null, and stderr is piped
# by the executor, read on a non-zero exit, and — since mika#2532 — reported on
# `$MIKA_SPIRIT_LOG_FILE` under `long_running_exec_stderr` plus an `audit_events`
# row. Before that it was read into memory and thrown away, because the callback
# had already made the task terminal and `update_task_failed` then writes nothing.
#
# ─────────────────────────────────────────────────────────────────────────────
# WHY THE TRAP IS INSTALLED AS EARLY AS IT IS (mika#2532 L3)
#
# The trap is what turns any later failure into a callback that NAMES its stage.
# So it goes up the instant `TASK_ID` is known, and everything fallible follows
# it. Two steps remain irreducibly silent and are named rather than hidden:
#
#   * `jq` absent — there is no way to parse `TASK_ID`, so no callback is
#     possible at all;
#   * `mika` absent — there is no way to DELIVER one.
#
# Both now reach `long_running_exec_stderr`, which is the other half of the fix:
# L3 closes what is closable, L1 covers the rest.

set -e

# Ensure ~/.local/bin is in PATH (mika CLI needed for callback delivery)
export PATH="$HOME/.local/bin:$PATH"

# --- Stage tracking (mika#2532 L2) ---
# `STAGE` names the coarse step; `STAGE_DETAIL` carries what that step knows and
# a generic message cannot. Composed at ONE site, in `deliver_callback` below —
# a second composition site is how the two halves of a diagnostic start
# disagreeing about the same failure.
STAGE="deps"
STAGE_DETAIL=""

# --- Dependency check: jq (irreducibly silent — see header) ---
command -v jq >/dev/null 2>&1 || { echo "Error: jq is required but not installed" >&2; exit 1; }

# --- Read input JSON from stdin ---
STAGE="parse"
INPUT=$(cat)

TASK_ID=$(printf '%s\n' "$INPUT" | jq -r '.__mika_task_id // empty')
AGENT=$(printf '%s\n' "$INPUT" | jq -r '.__mika_agent // empty')
CWD=$(printf '%s\n' "$INPUT" | jq -r '.cwd // empty')

if [ -z "$TASK_ID" ]; then
    echo "Error: no __mika_task_id in input (not running as long-running handler?)" >&2
    exit 1
fi

# --- Crash-recovery EXIT trap ---
# Ensures callback delivery on any exit (crash, set -e, signals).
# Uses CALLBACK_SENT guard to prevent double delivery.
#
# Installed HERE — as soon as `TASK_ID` is known — and not after the cwd block,
# which is where it used to sit. That placement is the whole of mika#2532's AC3:
# every step below reports its own name instead of the one generic sentence
# `HANDLER CRASH (exit code 1). Script failed before building result.`, which was
# compatible with four different causes and told an operator none of them.
CALLBACK_SENT=0

deliver_callback() {
    _EXIT_CODE=$?
    [ "$CALLBACK_SENT" -eq 1 ] && return
    [ -z "$TASK_ID" ] && return
    if [ -z "$RESULT" ]; then
        if [ -n "$STAGE_DETAIL" ]; then
            _DETAIL="$STAGE_DETAIL"
        else
            # No step-specific detail was recorded, so say what the stage means
            # rather than repeating the stage name in prose.
            case "$STAGE" in
                deps)        _DETAIL="A required dependency (jq or the mika CLI) was not found on PATH." ;;
                parse)       _DETAIL="The input JSON could not be read or did not carry __mika_task_id." ;;
                resolve_cwd) _DETAIL="The build directory could not be resolved." ;;
                enter_cwd)   _DETAIL="The build directory could not be entered." ;;
                build)       _DETAIL="The build step exited before a result was composed." ;;
                deliver)     _DETAIL="The callback delivery step failed." ;;
                *)           _DETAIL="No stage detail was recorded." ;;
            esac
        fi
        RESULT="HANDLER CRASH (exit code ${_EXIT_CODE}) at stage '${STAGE}'. ${_DETAIL}"
    fi
    RESULT=$(printf '%s' "$RESULT" | head -c 92000)
    set +e
    if [ -n "$AGENT" ]; then
        mika ask --task-id "$TASK_ID" --task-complete --agent "$AGENT" -- "$RESULT"
    else
        mika ask --task-id "$TASK_ID" --task-complete -- "$RESULT"
    fi
    CALLBACK_SENT=1
    set -e
}
trap deliver_callback EXIT

# --- Dependency check: mika CLI (irreducibly silent — see header) ---
# After the trap on purpose: the stage is right and the message is composed,
# even though `mika ask` is exactly what cannot run. The stderr is what carries
# it, via `long_running_exec_stderr`.
STAGE="deps"
command -v mika >/dev/null 2>&1 || { echo "Error: mika CLI is required but not in PATH" >&2; exit 1; }

# Scrub sensitive env vars so cargo build scripts cannot leak them
unset MIKA_ANTHROPIC_API_KEY MIKA_INTERNAL_TOKEN MIKA_OPENAI_API_KEY MIKA_BRAVE_API_KEY

# --- Resolve the build directory ---
STAGE="resolve_cwd"
if [ -n "$CWD" ]; then
    CWD_SOURCE="the tool's 'cwd' argument"
else
    CWD_SOURCE="the resolved default"
    # `PLATFORM_DIR` is relayed by the executor (mika#2532 L4a). It replaces
    # `${MIKA_PLATFORM_DIR:-…}`, which was a DEAD BRANCH: `sandboxed_pilot_env`
    # rebuilds the child env from a positive allowlist that refuses every
    # `MIKA_*` name, so the left-hand side could never be read and the handler
    # merely LOOKED like it honoured an operator setting.
    _DEFAULT="${PLATFORM_DIR:-$HOME/workspace/mika-platform}/mika"
    CWD=$(cd "$_DEFAULT" 2>/dev/null && pwd -P) || CWD="$_DEFAULT"
fi

# --- Validate the build directory BEFORE entering it (mika#2532 R6) ---
#
# A `cd` that fails is poor information: one exit code for four different
# faults. Each refusal below carries its own word, so the RESULT says which
# gesture repairs it — and every one of them cites the path VERBATIM together
# with where it came from.
#
# The literal-`$` check comes FIRST, and the order is load-bearing. The measured
# hypothesis for the founding crashes is that the model copied the prompt's
# formula (`$MIKA_PLATFORM_DIR/.claude/worktrees/<branch>/mika/`) into the
# argument, unexpanded — a string that is ALSO non-absolute, so a
# leading-slash check would report `cwd_not_absolute`: true, and far less
# actionable than naming the variable that never got expanded.
STAGE="enter_cwd"

case "$CWD" in
    *'$'*)
        STAGE_DETAIL="cwd_unexpanded_variable: the path from ${CWD_SOURCE} still contains a literal '\$' and was never expanded by any shell: '${CWD}'. Pass an already-expanded absolute path; no environment variable is interpolated on this argument."
        exit 1
        ;;
esac

case "$CWD" in
    /*) ;;
    *)
        STAGE_DETAIL="cwd_not_absolute: the path from ${CWD_SOURCE} is not absolute: '${CWD}'."
        exit 1
        ;;
esac

if [ ! -e "$CWD" ]; then
    STAGE_DETAIL="cwd_not_found: the path from ${CWD_SOURCE} does not exist: '${CWD}'."
    exit 1
fi

if [ ! -d "$CWD" ]; then
    STAGE_DETAIL="cwd_not_a_directory: the path from ${CWD_SOURCE} exists but is not a directory: '${CWD}'."
    exit 1
fi

# Residual: the four checks above passed and `cd` still refused (permissions,
# a racing removal). Named rather than folded into one of them — a refusal that
# claims the wrong cause sends the operator to the wrong gesture.
cd "$CWD" || {
    STAGE_DETAIL="cwd_unreachable: the path from ${CWD_SOURCE} exists and is a directory but could not be entered: '${CWD}'."
    echo "ERROR: could not cd to $CWD" >&2
    exit 1
}

# --- Run the build ---
STAGE="build"
set +e
BUILD_OUTPUT=$(cargo build --release --features telemetry 2>&1)
BUILD_EXIT=$?
set -e

# Deliver result via callback
if [ "$BUILD_EXIT" -eq 0 ]; then
    RESULT="Build succeeded (cwd: ${CWD}).

${BUILD_OUTPUT}"
else
    RESULT="Build FAILED (exit code ${BUILD_EXIT}, cwd: ${CWD}).

${BUILD_OUTPUT}"
fi

# Truncate to ~90KB to stay within 100KB limit
RESULT=$(printf '%s' "$RESULT" | head -c 92000)

# Deliver via mika ask --task-id --task-complete
STAGE="deliver"
set +e
if [ -n "$AGENT" ]; then
    mika ask --task-id "$TASK_ID" --task-complete --agent "$AGENT" -- "$RESULT"
else
    mika ask --task-id "$TASK_ID" --task-complete -- "$RESULT"
fi
CALLBACK_EXIT=$?
CALLBACK_SENT=1
set -e

if [ "$CALLBACK_EXIT" -ne 0 ]; then
    echo "ERROR: callback delivery failed (exit $CALLBACK_EXIT) for task $TASK_ID" >&2
fi

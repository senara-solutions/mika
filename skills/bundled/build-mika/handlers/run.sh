#!/bin/sh
# Build handler for the build-mika skill.
# Input: JSON on stdin with __mika_task_id and __mika_agent (injected by executor for long-running)
# Output: Delivers result via `mika ask --task-id` callback
#
# This is a long-running handler: stdout goes to /dev/null, and stderr is read
# by the executor, which since mika#2532 persists it on the task row under
# `$.handler_failure` — readable with `mika tasks get <task-id>` even when the
# EXIT trap below has already delivered a callback and made the row terminal.
# All meaningful output must be delivered via the callback mechanism.
#
# --- Naming the step that failed (mika#2532 R2/R3) ---
# `_STEP` tracks where we are; the EXIT trap reports it when it has to invent a
# crash message. `_STEP_DETAIL` carries the specifics for the failures we saw
# coming. The step name covers the ones we did not — which are, by definition,
# the ones nobody thought to wrap.

set -e

_STEP="deps"
_STEP_DETAIL=""

# Ensure ~/.local/bin is in PATH (mika CLI needed for callback delivery)
export PATH="$HOME/.local/bin:$PATH"

# Dependency checks
command -v jq >/dev/null 2>&1 || { echo "Error: jq is required but not installed" >&2; exit 1; }
command -v mika >/dev/null 2>&1 || { echo "Error: mika CLI is required but not in PATH" >&2; exit 1; }

# Read input JSON from stdin
_STEP="parse_input"
INPUT=$(cat)

# Scrub sensitive env vars so cargo build scripts cannot leak them
unset MIKA_ANTHROPIC_API_KEY MIKA_INTERNAL_TOKEN MIKA_OPENAI_API_KEY MIKA_BRAVE_API_KEY

# Parse callback fields from enriched input
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
CALLBACK_SENT=0

deliver_callback() {
    _EXIT_CODE=$?
    [ "$CALLBACK_SENT" -eq 1 ] && return
    [ -z "$TASK_ID" ] && return
    if [ -z "$RESULT" ]; then
        # The `HANDLER CRASH` prefix is a wire format: `self-dev-callback`
        # documents it as a discriminant and `dispatch-lib.sh` greps it. The
        # step is ADDED to it, never substituted for it (mika#2532 D5).
        RESULT="HANDLER CRASH (exit code ${_EXIT_CODE}) at step '${_STEP}'."
        if [ -n "$_STEP_DETAIL" ]; then
            RESULT="HANDLER CRASH (exit code ${_EXIT_CODE}) at step '${_STEP}': ${_STEP_DETAIL}"
        fi
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

# --- cwd composability guard (mika#2536) ---
# Sourced AFTER the trap on purpose: a failure here must still deliver a
# callback. Fail-closed and NAMED — a missing guard refuses the dispatch with a
# reason instead of letting `cd` produce the generic "HANDLER CRASH" this whole
# ticket exists to remove.
_CWD_GUARD="$(dirname "$0")/../../_shared/cwd-guard.sh"
if [ -r "$_CWD_GUARD" ]; then
    # shellcheck source=../../_shared/cwd-guard.sh
    . "$_CWD_GUARD"
else
    RESULT="REFUSED (cwd-guard, mika#2536): shared guard unreadable at ${_CWD_GUARD}.
\`skills/bundled/_shared/\` is a projection of the BINARY, not of the checkout: rebuild
(\`make deploy\`) then re-seed, and read \`~/.mika/skills/.manifest-writer\`."
    exit 1
fi

# Use provided cwd or default to the main mika repo root.
# `PLATFORM_DIR` is the name the child receives — relayed from the operator's
# `MIKA_PLATFORM_DIR` by `inject_platform_dir_env` (mika#2536). The `MIKA_`-
# prefixed form was a DEAD branch here: `sandboxed_pilot_env` rebuilds the child
# env from a positive allowlist that refuses every `MIKA_*`.
_STEP="resolve_cwd"
if [ -z "$CWD" ]; then
    _DEFAULT="${PLATFORM_DIR:-$HOME/workspace/mika-platform}/mika"
    CWD=$(cd "$_DEFAULT" 2>/dev/null && pwd -P) || CWD="$_DEFAULT"
fi

# Refuse an uncomposable cwd BY NAMING IT, rather than letting `cd` fail with a
# message no surface relays (mika#2536 R6).
if ! validate_cwd "$CWD"; then
    RESULT="$CWD_REFUSAL"
    exit 1
fi

# Run the build.
#
# `validate_cwd` above has already refused every uncomposable cwd BY NAME, so
# this `cd` is now a NET rather than a path — what reaches it is a directory that
# passed the four tests and still cannot be entered (no `+x`, a mount that went
# away). The net keeps the `HANDLER CRASH` prefix rather than composing a
# `RESULT` of its own: that prefix is a wire format `self-dev-callback`
# documents as a discriminant and `dispatch-lib.sh` greps, and the step plus
# `$_STEP_DETAIL` name the cause inside it (mika#2532 D5).
#
# The `echo` on fd 2 is kept, and mika#2536's reason for dropping it — "`cd`
# writes its diagnostic to a stderr the long-running path discards" — no longer
# holds: mika#2532 R1 persists that stderr on the task row under
# `$.handler_failure`, readable with `mika tasks get <task-id>`.
_STEP="chdir"
cd "$CWD" || {
    _STEP_DETAIL="could not cd to $CWD"
    echo "ERROR: could not cd to $CWD" >&2
    exit 1
}
_STEP="build"
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
_STEP="deliver"
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

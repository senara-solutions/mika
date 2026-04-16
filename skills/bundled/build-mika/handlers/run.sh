#!/bin/sh
# Build handler for the build-mika skill.
# Input: JSON on stdin with __mika_task_id and __mika_agent (injected by executor for long-running)
# Output: Delivers result via `mika ask --task-id` callback
#
# This is a long-running handler: stdout goes to /dev/null, stderr is captured on failure.
# All meaningful output must be delivered via the callback mechanism.

set -e

# Ensure ~/.local/bin is in PATH (mika CLI needed for callback delivery)
export PATH="$HOME/.local/bin:$PATH"

# Dependency checks
command -v jq >/dev/null 2>&1 || { echo "Error: jq is required but not installed" >&2; exit 1; }
command -v mika >/dev/null 2>&1 || { echo "Error: mika CLI is required but not in PATH" >&2; exit 1; }

# Read input JSON from stdin
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
        RESULT="HANDLER CRASH (exit code ${_EXIT_CODE}). Script failed before building result."
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

# Use provided cwd or default to the main mika repo root
if [ -z "$CWD" ]; then
    _DEFAULT="${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}/mika"
    CWD=$(cd "$_DEFAULT" 2>/dev/null && pwd -P) || CWD="$_DEFAULT"
fi

# Run the build
cd "$CWD" || { echo "ERROR: could not cd to $CWD" >&2; exit 1; }
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

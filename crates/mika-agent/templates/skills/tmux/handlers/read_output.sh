#!/bin/sh
# Read output from a tmux session pane.
# Input: JSON on stdin with "session", optional "lines" (default 50), "full"
# Output: pane content on stdout

command -v tmux >/dev/null 2>&1 || { echo "Error: tmux is not installed" >&2; exit 1; }
command -v jq >/dev/null 2>&1 || { echo "Error: jq is required but not installed" >&2; exit 1; }

# Prevent nested tmux client issues when spawned from within tmux
unset TMUX TMUX_PANE

INPUT=$(cat)

# Parse JSON fields
SESSION=$(printf '%s\n' "$INPUT" | jq -r '.session // empty')
LINE_COUNT=$(printf '%s\n' "$INPUT" | jq -r '.lines // empty')
FULL=$(printf '%s\n' "$INPUT" | jq -r 'if .full == true then "true" else "" end')

# Validate LINE_COUNT as a positive integer, clamp to max 10000
case "$LINE_COUNT" in
    ''|*[!0-9]*) LINE_COUNT=50 ;;
esac
if [ "$LINE_COUNT" -gt 10000 ]; then LINE_COUNT=10000; fi
if [ "$LINE_COUNT" -lt 1 ]; then LINE_COUNT=1; fi

if [ -z "$SESSION" ]; then
    echo "Error: session name is required" >&2
    exit 1
fi

if ! tmux has-session -t "$SESSION" 2>/dev/null; then
    echo "Error: session '$SESSION' does not exist" >&2
    exit 1
fi

# Warn if pane is dead — output will be stale
PANE_DEAD=$(tmux display-message -t "$SESSION" -p '#{pane_dead}' 2>/dev/null)
if [ "$PANE_DEAD" = "1" ]; then
    echo "[WARNING: pane in session '$SESSION' is dead — output below is stale]"
fi

if [ "$FULL" = "true" ]; then
    # Capture entire scrollback
    tmux capture-pane -t "$SESSION" -p -J -S -
else
    # Capture last N lines
    tmux capture-pane -t "$SESSION" -p -J -S "-$LINE_COUNT"
fi

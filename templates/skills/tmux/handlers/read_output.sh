#!/bin/sh
# Read output from a tmux session pane.
# Input: JSON on stdin with "session", optional "lines" (default 50), "full"
# Output: pane content on stdout

command -v tmux >/dev/null 2>&1 || { echo "Error: tmux is not installed" >&2; exit 1; }

INPUT=$(cat)

if command -v jq >/dev/null 2>&1; then
    SESSION=$(echo "$INPUT" | jq -r '.session // empty')
    LINE_COUNT=$(echo "$INPUT" | jq -r '.lines // 50')
    FULL=$(echo "$INPUT" | jq -r '.full // false')
else
    SESSION=$(echo "$INPUT" | grep -o '"session":"[^"]*"' | head -1 | cut -d'"' -f4)
    LINE_COUNT=$(echo "$INPUT" | grep -o '"lines":[0-9]*' | head -1 | cut -d':' -f2)
    FULL=$(echo "$INPUT" | grep -o '"full":true' | head -1)
    if [ -z "$LINE_COUNT" ]; then LINE_COUNT=50; fi
    if [ -n "$FULL" ]; then FULL="true"; else FULL="false"; fi
fi

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

if [ "$FULL" = "true" ]; then
    # Capture entire scrollback
    tmux capture-pane -t "$SESSION" -p -J -S -
else
    # Capture last N lines
    tmux capture-pane -t "$SESSION" -p -J -S "-$LINE_COUNT"
fi

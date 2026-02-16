#!/bin/sh
# Kill a tmux session by name.
# Input: JSON on stdin with "session"
# Output: confirmation on stdout, errors on stderr

command -v tmux >/dev/null 2>&1 || { echo "Error: tmux is not installed" >&2; exit 1; }

# Prevent nested tmux client issues when spawned from within tmux
unset TMUX TMUX_PANE

INPUT=$(cat)

if command -v jq >/dev/null 2>&1; then
    SESSION=$(printf '%s\n' "$INPUT" | jq -r '.session // empty')
else
    SESSION=$(printf '%s\n' "$INPUT" | grep -o '"session":"[^"]*"' | head -1 | cut -d'"' -f4)
fi

if [ -z "$SESSION" ]; then
    echo "Error: session name is required" >&2
    exit 1
fi

if ! tmux has-session -t "$SESSION" 2>/dev/null; then
    echo "Error: session '$SESSION' does not exist" >&2
    exit 1
fi

tmux kill-session -t "$SESSION" 2>&1
echo "Killed session '$SESSION'"

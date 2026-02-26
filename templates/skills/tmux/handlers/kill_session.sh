#!/bin/sh
# Kill a tmux session by name.
# Input: JSON on stdin with "session"
# Output: confirmation on stdout, errors on stderr

command -v tmux >/dev/null 2>&1 || { echo "Error: tmux is not installed" >&2; exit 1; }

INPUT=$(cat)

if command -v jq >/dev/null 2>&1; then
    SESSION=$(echo "$INPUT" | jq -r '.session // empty')
else
    SESSION=$(echo "$INPUT" | grep -o '"session":"[^"]*"' | head -1 | cut -d'"' -f4)
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

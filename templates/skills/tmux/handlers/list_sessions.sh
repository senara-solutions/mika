#!/bin/sh
# List all tmux sessions.
# Input: JSON on stdin (no required fields)
# Output: session list on stdout

command -v tmux >/dev/null 2>&1 || { echo "Error: tmux is not installed" >&2; exit 1; }

# Consume stdin
cat >/dev/null

if ! tmux list-sessions 2>/dev/null; then
    echo "No active tmux sessions"
fi

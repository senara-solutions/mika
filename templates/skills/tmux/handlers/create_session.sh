#!/bin/sh
# Create a new tmux session.
# Input: JSON on stdin with optional "name", "command", "working_dir"
# Output: session info on stdout, errors on stderr

command -v tmux >/dev/null 2>&1 || { echo "Error: tmux is not installed" >&2; exit 1; }

INPUT=$(cat)

# Parse JSON fields (jq preferred, grep/cut fallback)
if command -v jq >/dev/null 2>&1; then
    NAME=$(echo "$INPUT" | jq -r '.name // empty')
    COMMAND=$(echo "$INPUT" | jq -r '.command // empty')
    WORKDIR=$(echo "$INPUT" | jq -r '.working_dir // empty')
else
    NAME=$(echo "$INPUT" | grep -o '"name":"[^"]*"' | head -1 | cut -d'"' -f4)
    COMMAND=$(echo "$INPUT" | grep -o '"command":"[^"]*"' | head -1 | cut -d'"' -f4)
    WORKDIR=$(echo "$INPUT" | grep -o '"working_dir":"[^"]*"' | head -1 | cut -d'"' -f4)
fi

# Generate name if not provided
if [ -z "$NAME" ]; then
    NAME="mika-$(date +%s)"
fi

# Validate session name: only allow alphanumeric, dash, underscore, dot
if ! echo "$NAME" | grep -qE '^[a-zA-Z0-9._-]+$'; then
    echo "Error: invalid session name '$NAME' (only alphanumeric, dash, underscore, dot allowed)" >&2
    exit 1
fi

# Check if session already exists
if tmux has-session -t "$NAME" 2>/dev/null; then
    echo "Session '$NAME' already exists"
    exit 0
fi

# Create session with properly quoted arguments
if [ -n "$WORKDIR" ] && [ -d "$WORKDIR" ]; then
    tmux new-session -d -s "$NAME" -c "$WORKDIR" 2>&1 || { echo "Error: failed to create session '$NAME'" >&2; exit 1; }
else
    tmux new-session -d -s "$NAME" 2>&1 || { echo "Error: failed to create session '$NAME'" >&2; exit 1; }
fi

# Run command if provided
if [ -n "$COMMAND" ]; then
    sleep 0.1
    tmux send-keys -t "$NAME" -l -- "$COMMAND"
    sleep 0.1
    tmux send-keys -t "$NAME" Enter
fi

echo "Created session '$NAME'"

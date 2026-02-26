#!/bin/sh
# Send text and/or keys to a tmux session pane.
# Input: JSON on stdin with "session", optional "text", "special_key", "no_enter"
# Output: confirmation on stdout, errors on stderr

command -v tmux >/dev/null 2>&1 || { echo "Error: tmux is not installed" >&2; exit 1; }

INPUT=$(cat)

if command -v jq >/dev/null 2>&1; then
    SESSION=$(echo "$INPUT" | jq -r '.session // empty')
    TEXT=$(echo "$INPUT" | jq -r '.text // empty')
    SPECIAL_KEY=$(echo "$INPUT" | jq -r '.special_key // empty')
    NO_ENTER=$(echo "$INPUT" | jq -r '.no_enter // false')
else
    SESSION=$(echo "$INPUT" | grep -o '"session":"[^"]*"' | head -1 | cut -d'"' -f4)
    TEXT=$(echo "$INPUT" | grep -o '"text":"[^"]*"' | head -1 | cut -d'"' -f4)
    SPECIAL_KEY=$(echo "$INPUT" | grep -o '"special_key":"[^"]*"' | head -1 | cut -d'"' -f4)
    NO_ENTER=$(echo "$INPUT" | grep -o '"no_enter":true' | head -1)
    if [ -n "$NO_ENTER" ]; then NO_ENTER="true"; else NO_ENTER="false"; fi
fi

if [ -z "$SESSION" ]; then
    echo "Error: session name is required" >&2
    exit 1
fi

if ! tmux has-session -t "$SESSION" 2>/dev/null; then
    echo "Error: session '$SESSION' does not exist" >&2
    exit 1
fi

# Send text if provided (literal mode with -l)
if [ -n "$TEXT" ]; then
    tmux send-keys -t "$SESSION" -l -- "$TEXT"
    sleep 0.1
fi

# Allowlisted special keys to prevent arbitrary key injection
ALLOWED_KEYS="Enter|Escape|Tab|Space|C-c|C-d|C-z|C-l|Up|Down|Left|Right|BSpace|Home|End|PageUp|PageDown"

# Send special key or Enter
if [ -n "$SPECIAL_KEY" ]; then
    if echo "$SPECIAL_KEY" | grep -qE "^($ALLOWED_KEYS)$"; then
        tmux send-keys -t "$SESSION" "$SPECIAL_KEY"
    else
        echo "Error: special key '$SPECIAL_KEY' is not allowed. Permitted: $ALLOWED_KEYS" >&2
        exit 1
    fi
elif [ "$NO_ENTER" != "true" ] && [ -n "$TEXT" ]; then
    tmux send-keys -t "$SESSION" Enter
fi

echo "Sent to '$SESSION'"

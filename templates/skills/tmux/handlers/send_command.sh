#!/bin/sh
# Send text and/or keys to a tmux session pane.
# Input: JSON on stdin with "session", optional "text", "special_key", "no_enter"
# Output: confirmation on stdout, errors on stderr

command -v tmux >/dev/null 2>&1 || { echo "Error: tmux is not installed" >&2; exit 1; }

# Prevent nested tmux client issues when spawned from within tmux
unset TMUX TMUX_PANE

INPUT=$(cat)

if command -v jq >/dev/null 2>&1; then
    SESSION=$(printf '%s\n' "$INPUT" | jq -r '.session // empty')
    TEXT=$(printf '%s\n' "$INPUT" | jq -r '.text // empty')
    SPECIAL_KEY=$(printf '%s\n' "$INPUT" | jq -r '.special_key // empty')
    NO_ENTER=$(printf '%s\n' "$INPUT" | jq -r '.no_enter // false')
else
    SESSION=$(printf '%s\n' "$INPUT" | grep -o '"session":"[^"]*"' | head -1 | cut -d'"' -f4)
    TEXT=$(printf '%s\n' "$INPUT" | grep -o '"text":"[^"]*"' | head -1 | cut -d'"' -f4)
    SPECIAL_KEY=$(printf '%s\n' "$INPUT" | grep -o '"special_key":"[^"]*"' | head -1 | cut -d'"' -f4)
    NO_ENTER=$(printf '%s\n' "$INPUT" | grep -o '"no_enter":true' | head -1)
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

# Check pane is alive and not in copy-mode
PANE_DEAD=$(tmux display-message -t "$SESSION" -p '#{pane_dead}' 2>/dev/null)
PANE_MODE=$(tmux display-message -t "$SESSION" -p '#{pane_in_mode}' 2>/dev/null)

if [ "$PANE_DEAD" = "1" ]; then
    echo "Error: target pane in session '$SESSION' is dead" >&2
    exit 1
fi

# Auto-exit copy-mode if detected
if [ "$PANE_MODE" = "1" ]; then
    tmux send-keys -t "$SESSION" -X cancel 2>/dev/null
    sleep 0.1
    # Verify copy-mode actually exited
    PANE_MODE=$(tmux display-message -t "$SESSION" -p '#{pane_in_mode}' 2>/dev/null)
    if [ "$PANE_MODE" = "1" ]; then
        echo "Error: pane in session '$SESSION' is stuck in copy-mode" >&2
        exit 1
    fi
fi

# Send text if provided (literal mode with -l)
if [ -n "$TEXT" ]; then
    tmux send-keys -t "$SESSION" -l -- "$TEXT"
    sleep 0.2
fi

# Allowlisted special keys to prevent arbitrary key injection
ALLOWED_KEYS="Enter|Escape|Tab|Space|C-c|C-d|C-z|C-l|Up|Down|Left|Right|BSpace|Home|End|PageUp|PageDown"

# Send special key or Enter
if [ -n "$SPECIAL_KEY" ]; then
    if printf '%s' "$SPECIAL_KEY" | grep -qE "^($ALLOWED_KEYS)$"; then
        tmux send-keys -t "$SESSION" "$SPECIAL_KEY"
    else
        echo "Error: special key '$SPECIAL_KEY' is not allowed. Permitted: $ALLOWED_KEYS" >&2
        exit 1
    fi
elif [ "$NO_ENTER" != "true" ] && [ -n "$TEXT" ]; then
    tmux send-keys -t "$SESSION" Enter
fi

PANE_CMD=$(tmux display-message -t "$SESSION" -p '#{pane_current_command}' 2>/dev/null)
echo "Sent to '$SESSION' [pane_cmd=$PANE_CMD]"

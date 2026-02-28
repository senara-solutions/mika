#!/bin/sh
# Wait for specific text to appear in a tmux pane.
# Input: JSON on stdin with "session", "pattern", optional "timeout" (default 15),
#        "regex" (default false)
# Output: matched line on stdout, or timeout error on stderr

command -v tmux >/dev/null 2>&1 || { echo "Error: tmux is not installed" >&2; exit 1; }

# Prevent nested tmux client issues when spawned from within tmux
unset TMUX TMUX_PANE

INPUT=$(cat)

if command -v jq >/dev/null 2>&1; then
    SESSION=$(printf '%s\n' "$INPUT" | jq -r '.session // empty')
    PATTERN=$(printf '%s\n' "$INPUT" | jq -r '.pattern // empty')
    TIMEOUT=$(printf '%s\n' "$INPUT" | jq -r '.timeout // 15')
    USE_REGEX=$(printf '%s\n' "$INPUT" | jq -r '.regex // false')
else
    SESSION=$(printf '%s\n' "$INPUT" | grep -o '"session":"[^"]*"' | head -1 | cut -d'"' -f4)
    PATTERN=$(printf '%s\n' "$INPUT" | grep -o '"pattern":"[^"]*"' | head -1 | cut -d'"' -f4)
    TIMEOUT=$(printf '%s\n' "$INPUT" | grep -o '"timeout":[0-9]*' | head -1 | cut -d':' -f2)
    USE_REGEX=$(printf '%s\n' "$INPUT" | grep -o '"regex":true' | head -1)
    if [ -z "$TIMEOUT" ]; then TIMEOUT=15; fi
    if [ -n "$USE_REGEX" ]; then USE_REGEX="true"; else USE_REGEX="false"; fi
fi

if [ -z "$SESSION" ]; then
    echo "Error: session name is required" >&2
    exit 1
fi

if [ -z "$PATTERN" ]; then
    echo "Error: pattern is required" >&2
    exit 1
fi

# Validate pattern length to prevent ReDoS
PATTERN_LEN=$(printf '%s' "$PATTERN" | wc -c)
if [ "$PATTERN_LEN" -gt 200 ]; then
    echo "Error: pattern too long (max 200 characters)" >&2
    exit 1
fi

# Validate timeout as positive integer, clamp to max 25 (within skill timeout of 30s)
case "$TIMEOUT" in
    ''|*[!0-9]*) TIMEOUT=15 ;;
esac
if [ "$TIMEOUT" -gt 25 ]; then TIMEOUT=25; fi
if [ "$TIMEOUT" -lt 1 ]; then TIMEOUT=1; fi

if ! tmux has-session -t "$SESSION" 2>/dev/null; then
    echo "Error: session '$SESSION' does not exist" >&2
    exit 1
fi

# Track wall time for accurate timeout
START_TIME=$(date +%s)

while true; do
    CURRENT_TIME=$(date +%s)
    ELAPSED=$((CURRENT_TIME - START_TIME))
    if [ "$ELAPSED" -ge "$TIMEOUT" ]; then
        break
    fi

    # Check pane is still alive — exit early instead of polling stale content
    PANE_DEAD=$(tmux display-message -t "$SESSION" -p '#{pane_dead}' 2>/dev/null)
    if [ "$PANE_DEAD" = "1" ]; then
        echo "Error: pane in session '$SESSION' died while waiting for '$PATTERN'" >&2
        exit 1
    fi

    OUTPUT=$(tmux capture-pane -t "$SESSION" -p -J)

    if [ "$USE_REGEX" = "true" ]; then
        MATCH=$(printf '%s\n' "$OUTPUT" | timeout 2 grep -E "$PATTERN" | tail -1)
    else
        MATCH=$(printf '%s\n' "$OUTPUT" | grep -F "$PATTERN" | tail -1)
    fi

    if [ -n "$MATCH" ]; then
        echo "Found: $MATCH"
        exit 0
    fi

    sleep 1
done

echo "Error: timed out after ${TIMEOUT}s waiting for '$PATTERN' in session '$SESSION'" >&2
exit 1

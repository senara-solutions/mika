#!/bin/sh
# Wait for specific text to appear in a tmux pane.
# Input: JSON on stdin with "session", "pattern", optional "timeout" (default 15),
#        "regex" (default false)
# Output: matched line on stdout, or timeout error on stderr

command -v tmux >/dev/null 2>&1 || { echo "Error: tmux is not installed" >&2; exit 1; }

INPUT=$(cat)

if command -v jq >/dev/null 2>&1; then
    SESSION=$(echo "$INPUT" | jq -r '.session // empty')
    PATTERN=$(echo "$INPUT" | jq -r '.pattern // empty')
    TIMEOUT=$(echo "$INPUT" | jq -r '.timeout // 15')
    USE_REGEX=$(echo "$INPUT" | jq -r '.regex // false')
else
    SESSION=$(echo "$INPUT" | grep -o '"session":"[^"]*"' | head -1 | cut -d'"' -f4)
    PATTERN=$(echo "$INPUT" | grep -o '"pattern":"[^"]*"' | head -1 | cut -d'"' -f4)
    TIMEOUT=$(echo "$INPUT" | grep -o '"timeout":[0-9]*' | head -1 | cut -d':' -f2)
    USE_REGEX=$(echo "$INPUT" | grep -o '"regex":true' | head -1)
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

# Validate timeout as positive integer, clamp to max 60
case "$TIMEOUT" in
    ''|*[!0-9]*) TIMEOUT=15 ;;
esac
if [ "$TIMEOUT" -gt 60 ]; then TIMEOUT=60; fi
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

    OUTPUT=$(tmux capture-pane -t "$SESSION" -p -J)

    if [ "$USE_REGEX" = "true" ]; then
        MATCH=$(echo "$OUTPUT" | timeout 2 grep -E "$PATTERN" | tail -1)
    else
        MATCH=$(echo "$OUTPUT" | grep -F "$PATTERN" | tail -1)
    fi

    if [ -n "$MATCH" ]; then
        echo "Found: $MATCH"
        exit 0
    fi

    sleep 1
done

echo "Error: timed out after ${TIMEOUT}s waiting for '$PATTERN' in session '$SESSION'" >&2
exit 1

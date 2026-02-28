#!/bin/sh
# File reader handler.
# Input: JSON on stdin with "path" field
# Output: file contents on stdout, or __mika_v1 envelope for images

INPUT=$(cat)
PATH_VALUE=$(echo "$INPUT" | jq -r '.path // empty')

if [ -z "$PATH_VALUE" ]; then
    echo "Error: no path provided" >&2
    exit 1
fi

if [ ! -f "$PATH_VALUE" ]; then
    echo "Error: file not found: $PATH_VALUE" >&2
    exit 1
fi

# Detect image files and return as __mika_v1 envelope for visual analysis
MIME=$(file -b --mime-type "$PATH_VALUE" 2>/dev/null)
case "$MIME" in
    image/jpeg|image/png|image/gif|image/webp)
        # Use jq for safe JSON construction (handles all special characters)
        jq -cn --arg path "$PATH_VALUE" --arg mime "$MIME" \
            '{"__mika_v1":{"text":"Image file: \($path) (\($mime))","images":[$path]}}'
        ;;
    *)
        cat "$PATH_VALUE"
        ;;
esac

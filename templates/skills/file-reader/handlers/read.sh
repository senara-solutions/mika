#!/bin/sh
# File reader handler.
# Input: JSON on stdin with "path" field
# Output: file contents on stdout, or __mika_v1 envelope for images

INPUT=$(cat)
PATH_VALUE=$(echo "$INPUT" | grep -o '"path":"[^"]*"' | head -1 | cut -d'"' -f4)

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
        # Escape path for JSON (handle special chars)
        ESCAPED_PATH=$(printf '%s' "$PATH_VALUE" | sed 's/\\/\\\\/g; s/"/\\"/g')
        printf '{"__mika_v1":{"text":"Image file: %s (%s)","images":["%s"]}}' \
            "$ESCAPED_PATH" "$MIME" "$ESCAPED_PATH"
        ;;
    *)
        cat "$PATH_VALUE"
        ;;
esac

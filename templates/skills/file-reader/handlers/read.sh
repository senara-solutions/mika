#!/bin/sh
# File reader handler.
# Input: JSON on stdin with "path" field
# Output: file contents on stdout

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

cat "$PATH_VALUE"

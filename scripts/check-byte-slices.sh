#!/usr/bin/env bash
# CI lint: detect unsafe byte-slice patterns on &str that panic on multi-byte UTF-8.
# See: https://github.com/senara-solutions/mika/issues/764
#
# Lines containing "// safe-byte-slice:" are excluded (opt-in allowlist).
# Exit 0 if clean, exit 1 with actionable errors if violations found.

set -euo pipefail

VIOLATIONS=0
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Pattern A: [..var.len().min(N)] — truncation via min() without char boundary check.
# This is always unsafe on &str because .len() returns bytes and .min(N) can land
# inside a multi-byte character.
while IFS= read -r line; do
    if [[ -n "$line" ]]; then
        echo "ERROR: unsafe byte-slice pattern (Pattern A) at $line"
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
done < <(grep -rn '\.len()\.min(' "$REPO_ROOT/crates/" --include='*.rs' \
    | grep '\[\.\..*\.len()\.min(' \
    | grep -v '// safe-byte-slice:' \
    | grep -v '/target/' \
    || true)

# Pattern B: &str_var[..LITERAL_INT] — direct byte offset with literal integer.
# Only flag when the variable name is a known string type (content, body, etc.)
# to reduce false positives from &[u8] / byte array indexing.
while IFS= read -r line; do
    if [[ -n "$line" ]]; then
        echo "ERROR: unsafe byte-slice pattern (Pattern B) at $line"
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
done < <(grep -rn -E '&(content|body|bad_output|cleaned|output|msg\.content|second_output|chunk_context)\[\.\.([0-9]+)\]' "$REPO_ROOT/crates/" --include='*.rs' \
    | grep -v '// safe-byte-slice:' \
    | grep -v '/target/' \
    || true)

if [[ $VIOLATIONS -gt 0 ]]; then
    echo ""
    echo "Found $VIOLATIONS unsafe byte-slice pattern(s)."
    echo "These patterns panic on multi-byte UTF-8 (em-dashes, arrows, etc.) — see #764."
    echo "Use mika_common::text::safe_truncate() or annotate with \`// safe-byte-slice: <reason>\`."
    exit 1
fi

echo "No unsafe byte-slice patterns found."
exit 0

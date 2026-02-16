---
status: complete
priority: p2
issue_id: "341"
tags: [code-review, security, multimodal-tool-results]
dependencies: []
---

# Shell Script Path Injection in file-reader read.sh

## Problem Statement

In `templates/skills/file-reader/handlers/read.sh`, the `$PATH_VALUE` variable is interpolated into a `printf` format string that constructs a JSON envelope. If the file path contains special characters (double quotes, backslashes, newlines), the JSON output will be malformed or could produce unexpected behavior.

While the current `sed` escaping handles backslashes and double quotes, other JSON-special characters (newlines, tabs, control characters) are not escaped.

## Findings

- **Source:** security-sentinel review agent
- **Severity:** P2 — requires a malicious or unusual filename to exploit, but could cause malformed JSON
- **Location:** `templates/skills/file-reader/handlers/read.sh`
- **Evidence:** `ESCAPED_PATH=$(printf '%s' "$PATH_VALUE" | sed 's/\\/\\\\/g; s/"/\\"/g')` — only escapes `\` and `"`, not newlines, tabs, or other control chars

## Proposed Solutions

### Solution A: Use jq for JSON construction (Recommended)

Use `jq` to construct the JSON string safely, as it handles all escaping automatically.

```sh
jq -n --arg path "$PATH_VALUE" --arg mime "$MIME" \
  '{"__mika_v1":{"text":"Image file: \($path) (\($mime))","images":[$path]}}'
```

- **Pros:** Correct JSON escaping for all characters, no manual escaping needed
- **Cons:** Requires `jq` to be installed (it's common but not universal)
- **Effort:** Small
- **Risk:** Low

### Solution B: Expand sed escaping

Add escaping for newlines, tabs, and other JSON control characters.

- **Pros:** No additional dependency
- **Cons:** Complex sed expressions, easy to miss edge cases
- **Effort:** Small
- **Risk:** Medium — manual escaping is error-prone

## Recommended Action

Solution A — use `jq` for JSON construction

## Technical Details

- **Affected files:** `templates/skills/file-reader/handlers/read.sh`

## Acceptance Criteria

- [ ] File paths with special characters (quotes, newlines, tabs, unicode) produce valid JSON
- [ ] Backward compatible with normal file paths

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-28 | Created from code review | Identified by security-sentinel agent |

## Resources

- PR branch: `feat/multimodal-tool-results`

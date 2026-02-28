---
status: complete
priority: p2
issue_id: "328"
tags: [code-review, security]
dependencies: []
---

# No Validation of file_path from Telegram API

## Problem Statement

The `file_path` from Telegram's `getFile` API response is interpolated directly into a URL in `download_file_bytes()`. A compromised or tampered `getFile` response with path traversal or URL manipulation characters could redirect the download request, leaking the bot token in the URL path.

## Findings

- Flagged by: security-sentinel (Medium)
- Location: `crates/mika-gateway/src/telegram.rs:356-362`
- The bot token is embedded in the URL; any redirect leaks it

## Proposed Solutions

### Option A: Validate file_path against expected pattern
- **Pros:** Simple, effective. Telegram paths are `photos/file_NNN.jpg` or `documents/file_NNN.ext`
- **Cons:** Pattern could change
- **Effort:** Small
- **Risk:** Low

Assert file_path doesn't contain `..`, `@`, `?`, `#`, and doesn't start with `/`.

## Acceptance Criteria

- [ ] file_path validated before URL construction
- [ ] Rejects traversal and URL manipulation characters

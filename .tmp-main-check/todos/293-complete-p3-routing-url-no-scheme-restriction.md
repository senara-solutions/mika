---
status: complete
priority: p3
issue_id: 293
tags: [code-review, security, defense-in-depth]
dependencies: []
---

# No scheme restriction on `routing_url` validation

## Problem Statement

URL validation in both `chat.rs:41` and `server/mod.rs:127` uses `reqwest::Url::parse()` which accepts any well-formed URL including `file://`, `ftp://`, etc. While reqwest will reject non-HTTP schemes at send time, defense-in-depth suggests validating the scheme explicitly.

## Findings

- **Security Sentinel:** `reqwest::Url::parse` accepts any scheme. In practice low risk since reqwest only handles HTTP(S), but explicit validation prevents confusion and provides clearer error messages.

## Proposed Solutions

### Solution A: Add scheme check after parsing

```rust
let parsed = reqwest::Url::parse(url)?;
if !matches!(parsed.scheme(), "http" | "https") {
    // warn and return None (CLI) or bail (server)
}
```

- **Pros:** Explicit, clear error message, defense-in-depth
- **Cons:** Marginal value since reqwest already rejects non-HTTP at send time
- **Effort:** Small
- **Risk:** None

## Technical Details

- **Affected files:** `crates/mika-cli/src/commands/chat.rs`, `crates/mika-agent/src/server/mod.rs`

## Acceptance Criteria

- [ ] Non-HTTP(S) schemes are rejected with a clear error/warning
- [ ] Both CLI and server paths validate scheme

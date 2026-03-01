---
status: complete
priority: p2
issue_id: "360"
tags: [code-review, security, mcp]
dependencies: []
---

# Authorization header bypasses http::HeaderValue validation

## Problem Statement

In `connect_http()`, custom headers are validated through `http::HeaderName::from_bytes()` and `http::HeaderValue::from_str()` which reject invalid characters. However, the Authorization header value is extracted and passed directly to `transport_config.auth_header()` without the same validation. This creates an asymmetry where the most security-sensitive header skips the strictest validation.

## Findings

- **Source**: security-sentinel review
- **File**: `crates/mika-agent/src/mcp/mod.rs:296-301`
- **Evidence**: Authorization value passed to `auth_header()` without `http::HeaderValue::from_str()` check
- **Risk**: If rmcp's `auth_header()` does not internally validate, malformed values with control characters could be sent

## Proposed Solutions

### Option A: Add explicit validation before auth_header (Recommended)

```rust
if let Some(auth) = headers
    .iter()
    .find_map(|(k, v)| k.eq_ignore_ascii_case("authorization").then_some(v))
{
    if http::HeaderValue::from_str(auth).is_ok() {
        transport_config = transport_config.auth_header(auth.clone());
    } else {
        warn!(server = name, "invalid Authorization header value, skipping");
    }
}
```

- Effort: Small
- Risk: Very low
- Pros: Consistent validation for all headers
- Cons: None

## Recommended Action

Option A

## Technical Details

- **Affected files**: `crates/mika-agent/src/mcp/mod.rs`

## Acceptance Criteria

- [ ] Authorization header value validated through `http::HeaderValue::from_str()` before passing to rmcp
- [ ] Invalid Authorization values logged and skipped
- [ ] Existing tests pass

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-01 | Created from code review | |

## Resources

- PR branch: feat/mcp-headers-cli-enable

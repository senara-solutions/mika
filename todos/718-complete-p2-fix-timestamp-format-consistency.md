---
status: complete
priority: p2
issue_id: "718"
tags: [code-review, quality]
---

# Fix timestamp format consistency in OAuth token parsing

## Problem Statement

`save_oauth_tokens()` and `refresh_tokens()` write `%Y-%m-%dT%H:%M:%SZ` format (e.g. `2026-03-21T12:00:00Z`). This format has `Z` suffix but is NOT valid RFC 3339 (which requires `+00:00`). So in `is_token_valid()`, the `parse_from_rfc3339` call always fails and the `or_else` fallback always executes. The first attempt is dead code.

## Findings

- **Source**: code-simplicity-reviewer
- **Location**: `crates/mika-common/src/oauth.rs` lines 409-416
- **Evidence**: `chrono::DateTime::parse_from_rfc3339("2026-03-21T12:00:00Z")` — actually `Z` IS accepted by chrono's `parse_from_rfc3339`. Let me re-verify: chrono docs say "Z" is accepted. This may NOT be a real issue.

## Proposed Solutions

### Option A: Verify chrono RFC 3339 parsing handles `Z` (Recommended)
- Test with the actual format produced by `save_oauth_tokens`
- If `parse_from_rfc3339` succeeds with `Z` suffix, remove the fallback
- **Effort**: Small
- **Risk**: None

### Option B: Use a single known format parser
- Replace `parse_from_rfc3339().or_else(|_| ...)` with just `NaiveDateTime::parse_from_str(..., "%Y-%m-%dT%H:%M:%SZ")`
- **Effort**: Small
- **Risk**: None

## Acceptance Criteria

- [ ] Timestamp parsing uses a single code path (no always-failing fallback)
- [ ] Tests verify the format produced by `save_oauth_tokens` is correctly parsed
- [ ] `cargo test` passes

---
status: pending
priority: p2
issue_id: 646
tags: [code-review, security]
dependencies: []
---

# `investigate_github_token` should use `SecretString` for consistency

## Problem Statement

The `investigate_github_token` field on `Settings` is stored as `Option<String>`
while other credential fields (`internal_token`, `dashboard_token`,
`otlp_auth_header`) use `Option<SecretString>` from the `secrecy` crate. This
is a defense-in-depth gap: if the `Settings` struct is ever serialized or logged
through a non-Debug path, the GitHub token would be exposed in plaintext.

The manual `Debug` impl does redact it, but `SecretString` provides zeroization
on drop and prevents accidental exposure through any code path.

## Findings

- `crates/mika-common/src/config.rs:251` — `pub investigate_github_token: Option<String>`
- `crates/mika-agent/src/server/investigate.rs:435` — `CreateGithubIssueTool { github_token: String }` (private struct, lower priority)
- `crates/mika-agent/src/server/investigate.rs:550` — `InvestigationToolsConfig { investigate_github_token: Option<String> }`
- Pattern set by `internal_token: Option<SecretString>` at config.rs:220

Detected by: security-sentinel agent

## Proposed Solutions

### Option A: Change Settings field to SecretString
- Change `investigate_github_token: Option<String>` to `Option<SecretString>`
- Update `get_effective_value` to use `.expose_secret()`
- Update `investigate.rs` to call `.expose_secret()` when passing to `CreateGithubIssueTool`
- **Pros:** Matches pattern of other credentials, zeroize on drop, prevents accidental leaks
- **Cons:** Minor churn in 3 files
- **Effort:** Small
- **Risk:** Low

### Option B: Leave as-is
- The manual Debug impl already redacts it
- The token is only used in one place (investigation panel)
- **Pros:** No code change
- **Cons:** Inconsistent with other credentials, no zeroize protection
- **Effort:** None
- **Risk:** Low (theoretical exposure only)

## Recommended Action

Option A — small change, high consistency benefit.

## Technical Details

- **Affected files:** `crates/mika-common/src/config.rs`, `crates/mika-agent/src/server/investigate.rs`
- **Components:** Settings struct, InvestigationToolsConfig, CreateGithubIssueTool

## Acceptance Criteria

- [ ] `investigate_github_token` field uses `Option<SecretString>`
- [ ] `get_effective_value` calls `.expose_secret()` for the field
- [ ] `investigate.rs` calls `.expose_secret()` when extracting the token
- [ ] `cargo test` passes
- [ ] `cargo clippy` clean

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-12 | Created from code review | Consistency with other credential fields |

## Resources

- Security sentinel agent review
- Pattern: `internal_token: Option<SecretString>` at config.rs:220

---
status: complete
priority: p2
issue_id: "452"
tags: [code-review, security, telemetry]
dependencies: []
---

# OTLP Auth Header Stored as Plain String Instead of SecretString

## Problem Statement

`otlp_auth_header` in `Settings` is `Option<String>` while the equivalent `internal_token` field uses `secrecy::SecretString` with zeroize-on-drop. The `Debug` impl correctly redacts the value, but a plain `String` remains in memory after the struct is dropped and can be cloned freely.

## Findings

- **Source**: Security sentinel agent
- **Location**: `crates/mika-common/src/config.rs:78`
- **Evidence**: `internal_token` uses `SecretString` (line 42), `otlp_auth_header` uses `String` (line 78)

## Proposed Solutions

### Option A: Upgrade to SecretString (Recommended)
Change `Option<String>` to `Option<SecretString>`. Update `telemetry.rs` to call `.expose_secret()` before passing to `normalize_auth_header`. The `secrecy` crate is already a dependency.
- **Pros**: Defense-in-depth, consistent with existing secret handling
- **Cons**: ~10 lines changed
- **Effort**: Small

## Acceptance Criteria

- [ ] `otlp_auth_header` uses `SecretString`
- [ ] `normalize_auth_header` receives exposed secret
- [ ] Existing tests pass

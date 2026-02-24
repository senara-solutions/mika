---
status: ready
priority: p2
issue_id: "122"
tags: [code-review, security]
dependencies: []
---

# Internal Token Not Redacted in AppState Debug Output

## Problem Statement

`AppState` stores `internal_token` as a plain `String` field. If AppState is ever logged or debug-printed (via derived Debug or tracing), the token appears in plaintext. The project convention (see `Settings` struct in mika-common) is to use manual `Debug` impl that redacts secrets.

## Findings

- **Source:** security-sentinel (IMPORTANT-1), architecture-strategist
- **Location:** `crates/mika-agent/src/server/state.rs` — `internal_token: String`
- **Evidence:** `Settings` has manual Debug redaction for `anthropic_api_key`, but AppState has no such protection for `internal_token`

## Proposed Solutions

### Option 1: Manual Debug impl for AppState that redacts internal_token
- **Pros**: Follows existing project convention, prevents accidental logging
- **Cons**: Must maintain manual Debug impl
- **Effort**: Small
- **Risk**: Low

### Option 2: Wrap in a Secret<String> newtype
- **Pros**: Type-safe redaction, self-documenting
- **Cons**: Adds a type, may be over-engineered for one field
- **Effort**: Small
- **Risk**: Low

## Recommended Action

Option 1 — add manual `Debug` impl matching the pattern in `Settings`.

## Technical Details

- **Affected Files**: `crates/mika-agent/src/server/state.rs`
- **Database Changes**: None

## Acceptance Criteria

- [ ] `internal_token` is redacted in any Debug output of AppState
- [ ] Pattern matches existing `Settings` redaction approach

## Work Log

### 2026-02-24 - Identified during PR #5 review
**By:** security-sentinel, architecture-strategist

## Resources

- PR #5: Phase 2 Container HTTP Server
- Related: `crates/mika-common/src/config.rs` — Settings manual Debug impl

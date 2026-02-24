---
status: pending
priority: p3
issue_id: "134"
tags: [code-review, security]
dependencies: []
---

# gateway_url Not Validated at Startup

## Problem Statement

`MIKA_ROUTING_URL` is checked for presence but not validated as a proper URL at startup. A malformed URL (e.g., missing scheme, trailing whitespace) would only fail at runtime when the first send attempt is made.

## Findings

- **Source:** security-sentinel
- **Location:** `crates/mika-agent/src/server/mod.rs:55-58`

## Proposed Solutions

### Option 1: Parse with url::Url at startup
- **Pros**: Fail fast with clear error
- **Cons**: Adds url crate dependency (or use reqwest's internal validation)
- **Effort**: Small
- **Risk**: None

## Acceptance Criteria

- [ ] gateway_url validated as parseable URL at startup
- [ ] Clear error message on invalid URL

## Work Log

### 2026-02-24 - Identified during PR #5 review

## Resources

- PR #5: Phase 2 Container HTTP Server

---
status: pending
priority: p3
issue_id: "366"
tags: [code-review, security, gateway]
dependencies: []
---

# Validate MIKA_AGENT_BASE_URL scheme at startup

## Problem Statement

When `MIKA_AGENT_BASE_URL` is set, the gateway routes all traffic to that URL with the bearer token. The value is accepted without URL validation. A misconfigured value could theoretically become an SSRF vector (e.g., pointing to cloud metadata service).

Low severity — requires operator misconfiguration to trigger.

## Proposed Solutions

### Option A: Add URL scheme validation at settings load (Recommended)
- Parse URL, verify http/https scheme
- Warn if not localhost
- Effort: Small
- Risk: None

## Technical Details

**Affected files:**
- `crates/mika-gateway/src/settings.rs`

## Acceptance Criteria

- [ ] MIKA_AGENT_BASE_URL is validated as a well-formed http/https URL at startup
- [ ] Non-localhost values emit a warning

---
status: pending
priority: p1
issue_id: "213"
tags: [code-review, security, skills-system]
dependencies: []
---

# SSRF via HTTP Handler — No URL Validation

## Problem Statement
The HTTP handler in `handler.rs` sends POST requests to any URL specified in a skill's `skill.toml` with no validation. A malicious skill could target internal services (cloud metadata at 169.254.169.254, K8s API, internal databases) and exfiltrate data via the tool response.

## Findings
- Location: `crates/mika-agent/src/skills/handler.rs:91-92`
- `reqwest::Client::new()` with no URL allowlist
- No check against internal/private IP ranges
- Creates a new reqwest::Client per call (also a performance issue)
- Headers from skill manifest are passed through without validation
- On K8s, this could reach the metadata API, other pods, or internal services

## Proposed Solutions

### Option 1: Remove HTTP handler entirely (YAGNI)
- **Pros**: Eliminates SSRF surface completely; no HTTP skills exist
- **Cons**: Must re-implement when HTTP skills are actually needed
- **Effort**: Small (delete code)
- **Risk**: None

### Option 2: Add URL validation (allowlist + block private ranges)
- **Pros**: Keeps extensibility with safety
- **Cons**: Complex (must block all RFC1918, link-local, cloud metadata IPs)
- **Effort**: Medium
- **Risk**: Medium (easy to miss an IP range)

## Recommended Action
Option 1 — Remove HTTP handler. Zero HTTP skills exist. Re-add with proper URL validation when needed.

## Technical Details
- **Affected Files**: `crates/mika-agent/src/skills/handler.rs`, `crates/mika-agent/src/skills/manifest.rs`
- **Related Components**: Skills system, agent loop, K8s deployment
- **Database Changes**: No

## Acceptance Criteria
- [ ] HTTP handler removed or URL properly validated
- [ ] Private/internal IP ranges blocked if handler kept
- [ ] Cloud metadata endpoints blocked

## Work Log

### 2026-02-25 - Created from code review
**By:** Claude Code Review
**Actions:** Finding identified by security-sentinel agent

## Resources
- Related: #212 (same YAGNI argument applies)
- OWASP: Server-Side Request Forgery

---
status: pending
priority: p1
issue_id: 702
tags: [code-review, security]
dependencies: []
---

# SSRF Vulnerability in a2a_call Tool — No URL Validation

## Problem Statement

The `a2a_call` tool accepts arbitrary URLs without validation. An attacker could instruct the agent to call internal IPs (127.0.0.1, 10.x, 172.16.x, 192.168.x), cloud metadata endpoints (169.254.169.254), internal Kubernetes services (mika-{customer}:8080), or non-HTTP schemes (file://, gopher://). This is an SSRF vulnerability.

## Findings

- Location: `crates/mika-agent/src/tools/a2a_call.rs` lines 52-107
- No scheme validation — non-HTTP schemes could be passed through
- No hostname resolution or IP range checking
- Cloud metadata endpoint (169.254.169.254) is reachable from within containers
- Internal Kubernetes service names resolve to cluster-internal IPs
- Similar path validation is already done in file tools but not applied here

## Proposed Solutions

Parse the URL and validate:
1. Scheme is `http` or `https` only
2. Resolve hostname and reject private/reserved IP ranges (127.0.0.0/8, 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, ::1, etc.)
3. Block cloud metadata IP 169.254.169.254

This follows the same defensive pattern used for path validation in file tools.

## Acceptance Criteria

- [ ] `a2a_call` rejects non-http(s) schemes
- [ ] `a2a_call` resolves hostnames and rejects private/reserved IP ranges
- [ ] `a2a_call` rejects the cloud metadata endpoint (169.254.169.254)

---
status: pending
priority: p2
issue_id: "196"
tags: [code-review, security, consistency]
dependencies: []
---

# Gateway Pod Missing fsGroup in Security Context

## Problem Statement
The gateway pod security context does not set `fsGroup: 1000`, unlike the customer deployment which does. The /tmp emptyDir may have permission issues for UID 1000 writes.

## Findings
- **Security sentinel**: Low severity (L4)
- Location: `helm/mika-gateway/templates/deployment.yaml` lines 23-28

## Proposed Solutions

### Option 1: Add fsGroup: 1000 (Recommended)
Add `fsGroup: 1000` to the gateway pod securityContext for consistency.

- **Effort**: Small (2 minutes)
- **Risk**: Low

## Technical Details
- **Affected Files**: `helm/mika-gateway/templates/deployment.yaml`

## Acceptance Criteria
- [ ] Gateway pod securityContext includes `fsGroup: 1000`

## Work Log
### 2026-02-24 - Found during code review
**By:** Security sentinel

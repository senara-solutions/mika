---
status: pending
priority: p3
issue_id: "188"
tags: [code-review, security, helm, plan-review]
dependencies: []
---

# CronJob: Missing Container Security + Third-Party Curl Image

## Problem Statement
The heartbeat CronJob is missing container-level security hardening (allowPrivilegeEscalation, capabilities.drop, readOnlyRootFilesystem). It also uses `curlimages/curl:8.5.0` by tag (not digest), creating a supply chain risk since this image receives the internal token.

## Proposed Solutions

### Container security hardening
Add to the CronJob container:
```yaml
securityContext:
  allowPrivilegeEscalation: false
  readOnlyRootFilesystem: true
  capabilities:
    drop: [ALL]
```

### Image alternatives (pick one)
1. Pin curl image by digest: `curlimages/curl@sha256:<digest>`
2. Use the mika-agent image itself (has wget): eliminates third-party dependency
3. Build a minimal heartbeat image in ECR

## Acceptance Criteria
- [ ] CronJob container has full security context
- [ ] Image either pinned by digest or replaced with project-owned image

## Work Log

### 2026-02-24 - Plan Review Finding
**By:** Security sentinel, architecture strategist

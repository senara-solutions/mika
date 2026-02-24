---
status: complete
priority: p2
issue_id: "183"
tags: [code-review, security, helm, plan-review]
dependencies: []
---

# Missing Pod Security Hardening: seccomp, automountServiceAccountToken

## Problem Statement
All pod specs are missing: (1) seccomp profile (RuntimeDefault), (2) `automountServiceAccountToken: false`. The Mika containers have no need for K8s API access, but a compromised container could use the mounted service account token. Without seccomp, containers have access to ~300+ syscalls.

## Findings
- **Security sentinel**: P2 — Missing seccomp profiles on all pods; automountServiceAccountToken not disabled
- **Architecture strategist**: Confirmed — CronJob also missing container-level security context

## Proposed Solutions

### Add to all pod specs:
```yaml
spec:
  automountServiceAccountToken: false
  securityContext:
    runAsUser: 1000
    runAsGroup: 1000
    runAsNonRoot: true
    fsGroup: 1000
    seccompProfile:
      type: RuntimeDefault
```
Also add container-level hardening to CronJob (currently only has pod-level).
- **Effort**: Small (30 min)
- **Risk**: None

## Acceptance Criteria
- [ ] All Deployment and CronJob pods have `automountServiceAccountToken: false`
- [ ] All pods have `seccompProfile: { type: RuntimeDefault }`
- [ ] CronJob container has `allowPrivilegeEscalation: false` and `capabilities.drop: [ALL]`

## Work Log

### 2026-02-24 - Plan Review Finding
**By:** Security sentinel, architecture strategist

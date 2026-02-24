---
status: complete
priority: p3
issue_id: "190"
tags: [code-review, security, helm, plan-review]
dependencies: []
---

# Agent Container: readOnlyRootFilesystem + Resource Limit Assessment

## Problem Statement
The agent container has `readOnlyRootFilesystem: false`. Since the agent writes only to $MIKA_HOME (on PVC), the root filesystem could be set to read-only with an emptyDir for /tmp. The 256Mi memory limit may also be tight during conversation compaction (50+ messages summarized via Claude API).

## Proposed Solutions

### readOnlyRootFilesystem
Test with `readOnlyRootFilesystem: true` + emptyDir at /tmp:
```yaml
securityContext:
  readOnlyRootFilesystem: true
volumeMounts:
  - name: data
    mountPath: /home/mika/.mika
  - name: tmp
    mountPath: /tmp
volumes:
  - name: tmp
    emptyDir:
      sizeLimit: 10Mi
```

### Resource limits
Consider increasing to 384Mi or 512Mi for initial deployment. Also consider removing CPU limit (keep request only) to avoid CFS throttling — agent work is mostly I/O-bound (waiting on Claude API).

## Acceptance Criteria
- [ ] Test readOnlyRootFilesystem with the agent binary
- [ ] Document resource limit rationale and monitoring plan

## Work Log

### 2026-02-24 - Plan Review Finding
**By:** Architecture strategist, code simplicity reviewer

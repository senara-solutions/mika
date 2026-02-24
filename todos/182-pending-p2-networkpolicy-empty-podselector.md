---
status: pending
priority: p2
issue_id: "182"
tags: [code-review, security, helm, plan-review]
dependencies: []
---

# NetworkPolicy Empty podSelector Allows Agent-to-Agent Traffic

## Problem Statement
The customer NetworkPolicy ingress rule for heartbeat CronJob uses `podSelector: {}` which matches ALL pods in `mika-agents` namespace. Combined with the shared MIKA_INTERNAL_TOKEN, a compromised agent container can directly access other customers' /message and /heartbeat endpoints.

## Findings
- **Security sentinel**: P2 — Eliminates network-level isolation between customer containers

## Proposed Solutions

### Option A: Label-based CronJob selector (Recommended)
Add `mika.io/role: heartbeat` label to CronJob pods, then use it in NetworkPolicy:
```yaml
- from:
    - podSelector:
        matchLabels:
          mika.io/role: heartbeat
          mika.io/customer-id: {{ .Values.customer.id | quote }}
```
- **Effort**: Small (15 min)
- **Risk**: None

### Option B: Remove NetworkPolicy entirely (per simplicity review)
The simplicity reviewer argues NetworkPolicy is YAGNI for 20-30 customers on a dedicated cluster, and requires a CNI plugin (Calico/Cilium) that may not be installed.
- **Effort**: Negative (removes complexity)
- **Risk**: Reduces defense-in-depth

## Acceptance Criteria
- [ ] If NetworkPolicy is kept: No empty podSelector in ingress rules
- [ ] If NetworkPolicy is removed: Document in plan as deferred

## Work Log

### 2026-02-24 - Plan Review Finding
**By:** Security sentinel, code simplicity reviewer

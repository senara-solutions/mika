---
status: complete
priority: p2
issue_id: "184"
tags: [code-review, architecture, helm, plan-review]
dependencies: []
---

# Missing Ingress Resource for Gateway (Prerequisite for Telegram Webhooks)

## Problem Statement
The gateway needs to receive Telegram webhook traffic from the public internet. Without an Ingress resource, Telegram cannot deliver webhooks to a ClusterIP Service. The NetworkPolicy already assumes traffic from `ingress-nginx` namespace, confirming an Ingress controller is expected. Currently deferred to "Future Considerations" but it's a hard prerequisite.

## Findings
- **Architecture strategist**: P2 — Gateway is non-functional without external access

## Proposed Solutions

### Option A: Add Ingress template gated by values flag (Recommended)
```yaml
{{- if .Values.ingress.enabled }}
apiVersion: networking.k8s.io/v1
kind: Ingress
spec:
  rules:
    - host: {{ .Values.ingress.host }}
      http:
        paths:
          - path: /webhook/telegram
            pathType: Exact
            backend:
              service:
                name: mika-gateway
                port:
                  number: 8080
{{- end }}
```
Only expose `/webhook/telegram`, not internal endpoints.
- **Effort**: Small (30 min)
- **Risk**: Low

### Option B: Defer to separate infrastructure setup
Document that Ingress is an external prerequisite managed by Terraform/eksctl.
- **Effort**: None (documentation only)

## Acceptance Criteria
- [ ] Gateway chart includes optional Ingress template OR documents it as prerequisite

## Work Log

### 2026-02-24 - Plan Review Finding
**By:** Architecture strategist

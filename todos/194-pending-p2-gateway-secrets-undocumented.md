---
status: pending
priority: p2
issue_id: "194"
tags: [code-review, operational, documentation]
dependencies: []
---

# Gateway Secrets Creation Not Documented

## Problem Statement
The gateway deployment references `mika-gateway-secrets` K8s secret with keys `database-url`, `telegram-bot-token`, `telegram-webhook-secret`, and `internal-token`. But nothing in this PR creates that secret or documents how to create it. The gateway will fail to start without it.

## Findings
- **Architecture strategist**: Medium severity, missing operational artifact
- Location: `helm/mika-gateway/templates/deployment.yaml` lines 50-66 (secretKeyRef references)

## Proposed Solutions

### Option 1: Add setup-gateway.sh script (Recommended)
Create `scripts/setup-gateway.sh` that creates the gateway K8s secret:

```bash
kubectl create secret generic mika-gateway-secrets \
    --namespace mika-system \
    --from-literal=database-url="${DATABASE_URL}" \
    --from-literal=telegram-bot-token="${TELEGRAM_BOT_TOKEN}" \
    --from-literal=telegram-webhook-secret="${TELEGRAM_WEBHOOK_SECRET}" \
    --from-literal=internal-token="${MIKA_INTERNAL_TOKEN}"
```

- **Pros**: Consistent with provision.sh pattern, single command
- **Cons**: Another script to maintain
- **Effort**: Small (15 minutes)
- **Risk**: Low

## Technical Details
- **Affected Files**: New `scripts/setup-gateway.sh`

## Acceptance Criteria
- [ ] Script creates mika-gateway-secrets with all 4 required keys
- [ ] Script validates env vars before creating secret
- [ ] --help and --dry-run supported

## Work Log
### 2026-02-24 - Found during code review
**By:** Architecture strategist

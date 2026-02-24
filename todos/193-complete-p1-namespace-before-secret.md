---
status: complete
priority: p1
issue_id: "193"
tags: [code-review, correctness, operational]
dependencies: []
---

# Namespace Must Exist Before kubectl create secret

## Problem Statement
`provision.sh` Step 1 runs `kubectl create secret` in `mika-agents` namespace, but the namespace may not exist yet. The Helm install at Step 2 uses `--create-namespace` to create it, but that is too late — Step 1 fails first. The rollback trap fires with `STEP_COMPLETED=0`, which rolls back nothing, producing a misleading "Rolling back..." message.

## Findings
- **Agent-native reviewer**: Warning severity
- Location: `scripts/provision.sh` line 133 (`kubectl create secret ... --namespace "${NAMESPACE}"`)
- Helm install at line 143 has `--create-namespace` but runs after secret creation

## Proposed Solutions

### Option 1: Create namespace before Step 1 (Recommended)
Add a namespace check/create before the first kubectl command:

```bash
# Ensure namespace exists
kubectl get namespace "${NAMESPACE}" >/dev/null 2>&1 || kubectl create namespace "${NAMESPACE}"
```

- **Pros**: Idempotent, handles first-ever provision correctly
- **Cons**: None
- **Effort**: Small (5 minutes)
- **Risk**: Low

## Technical Details
- **Affected Files**: `scripts/provision.sh`

## Acceptance Criteria
- [ ] Namespace created if not exists before kubectl create secret
- [ ] First-ever provision (no prior namespace) succeeds
- [ ] Subsequent provisions (namespace exists) succeed

## Work Log
### 2026-02-24 - Found during code review
**By:** Agent-native reviewer

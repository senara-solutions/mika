---
status: complete
priority: p1
issue_id: "178"
tags: [code-review, security, helm, plan-review]
dependencies: []
---

# MIKA_INTERNAL_TOKEN Stored as Plaintext in Helm Values and Deployment Env

## Problem Statement
The `MIKA_INTERNAL_TOKEN` is set directly as `value: {{ .Values.internalToken | quote }}` in the Deployment and CronJob env vars, while `MIKA_ANTHROPIC_API_KEY` correctly uses `secretKeyRef` via ExternalSecret. This means the shared auth token is visible in `kubectl get deployment -o yaml`, `helm get values`, shell history (via `--set`), and `/proc/<pid>/cmdline`.

## Findings
- **Security sentinel**: P1 — Token visible to anyone with Deployment read access in the namespace
- **Architecture strategist**: P1 — Inconsistent secret handling (API key uses ExternalSecret, internal token is plaintext)
- **Agent-native reviewer**: Warning — Token passed via `--set` flag leaks to shell history and process list

## Proposed Solutions

### Option A: Store in same ExternalSecret (Recommended)
Add `internal_token` to the AWS Secrets Manager secret and reference via `secretKeyRef`.

```yaml
# In external-secret.yaml:
- secretKey: internal-token
  remoteRef:
    key: {{ .Values.secrets.awsSecretName }}
    property: internal_token

# In deployment.yaml:
- name: MIKA_INTERNAL_TOKEN
  valueFrom:
    secretKeyRef:
      name: {{ include "mika-customer.name" . }}-secrets
      key: internal-token
```

For CronJob: mount the same secret as env var.
For Helm install: use `--set-file` or values file instead of `--set`.
- **Effort**: Small (1 hour)
- **Risk**: Low

### Option B: Create dedicated shared K8s Secret
Create a cluster-wide secret for the internal token, referenced by all charts.
- **Effort**: Small
- **Risk**: Low

## Technical Details
- **Affected files**: Plan sections for deployment.yaml (customer + gateway), cronjob-heartbeat.yaml, values.yaml, provision.sh
- **Related Components**: All inter-service auth

## Acceptance Criteria
- [ ] MIKA_INTERNAL_TOKEN never appears as a literal `value:` in any rendered manifest
- [ ] Token not passed via `--set` on command line
- [ ] `helm template` output does not contain the raw token value

## Work Log

### 2026-02-24 - Plan Review Finding
**By:** Technical review agents (security-sentinel, architecture-strategist)
**Actions:** Identified plaintext token in Helm values and Deployment specs

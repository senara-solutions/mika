---
status: pending
priority: p2
issue_id: "185"
tags: [code-review, architecture, plan-review]
dependencies: []
---

# Plan Quality Fixes: ESO API Version, Rollback, Namespace, Deploy Strategy

## Problem Statement
Several medium-severity issues need fixing in the plan before implementation:
1. ExternalSecret uses deprecated `v1beta1` API (should be `v1`)
2. Provision.sh rollback doesn't clean up Step 3 (Postgres row)
3. `.Values.namespace` can drift from `--namespace` flag; should use `.Release.Namespace`
4. Gateway Deployment missing explicit strategy and startup probe
5. Postgres egress allows 0.0.0.0/0 on port 5432

## Findings
- **Architecture strategist**: P2 — Multiple plan quality issues that affect reliability

## Proposed Solutions
1. Change `external-secrets.io/v1beta1` to `external-secrets.io/v1` in both ExternalSecret templates
2. Add Postgres row cleanup to rollback: `if [[ $STEP_COMPLETED -ge 3 ]]; then DELETE...; fi`
3. Replace `.Values.namespace` with `.Release.Namespace` in all templates; remove `namespace` from values.yaml
4. Add explicit `strategy.type: RollingUpdate` with `maxUnavailable: 0, maxSurge: 1` to gateway
5. Add `startupProbe` to gateway Deployment (matches customer pattern)
6. Parameterize Postgres CIDR in gateway NetworkPolicy (not 0.0.0.0/0)
- **Effort**: Medium (1-2 hours for all)
- **Risk**: Low

## Acceptance Criteria
- [ ] ExternalSecret API version is v1
- [ ] Rollback covers all completed steps
- [ ] No `.Values.namespace` in templates (use `.Release.Namespace`)
- [ ] Gateway has explicit deploy strategy and startup probe
- [ ] Postgres egress restricted to VPC CIDR (parameterized)

## Work Log

### 2026-02-24 - Plan Review Finding
**By:** Architecture strategist, security sentinel

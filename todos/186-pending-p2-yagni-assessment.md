---
status: pending
priority: p2
issue_id: "186"
tags: [code-review, architecture, simplification, plan-review]
dependencies: []
---

# YAGNI Assessment: ESO, NetworkPolicy, PDB, Per-Customer CronJob

## Problem Statement
The simplicity reviewer identified several production-grade patterns that may be premature for 20-30 white-glove customers on a dedicated EKS cluster. These add cluster dependencies and operational complexity without proportional benefit at this scale.

## Findings
- **Code simplicity reviewer**: P1 — Four YAGNI violations totaling ~250 lines of unnecessary YAML

## Items to Assess

### 1. External Secrets Operator vs Inline K8s Secrets
ESO requires a cluster dependency (operator + ClusterSecretStore + IAM). At 20-30 customers, `kubectl create secret` in provision.sh achieves the same result with zero dependencies.
- **Keep ESO**: Future-proof, handles rotation, audit trail
- **Drop ESO**: Eliminates cluster dependency, simpler provisioning, manual rotation acceptable at this scale

### 2. NetworkPolicy
Requires a CNI plugin (Calico/Cilium). Default AWS VPC CNI does NOT enforce NetworkPolicy. Dedicated cluster means no multi-tenant threat model.
- **Keep**: Defense-in-depth, good practice
- **Drop**: Removes CNI dependency, resources do nothing on default EKS

### 3. Per-Customer CronJob vs Shared Heartbeat Script
30 CronJobs = 2,880 pod creations/day. A single `heartbeat-all.sh` querying Postgres and curling each agent achieves the same with one CronJob.
- **Keep per-customer**: Self-contained per Helm release
- **Switch to shared**: One CronJob for all, simpler cluster

### 4. PodDisruptionBudget
Gateway restarts in seconds. Minutes of downtime during cluster upgrades is acceptable at this scale.
- **Keep**: Good practice for zero-downtime
- **Drop**: Removes cognitive overhead, near-zero benefit

## Decision Required
User must decide which to keep/drop based on operational priorities. The simplicity reviewer recommends dropping all four for minimum viable infrastructure.

## Acceptance Criteria
- [ ] Each item explicitly decided: keep or defer
- [ ] Deferred items documented in plan's Future Considerations

## Work Log

### 2026-02-24 - Plan Review Finding
**By:** Code simplicity reviewer

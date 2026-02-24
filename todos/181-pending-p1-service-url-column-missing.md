---
status: pending
priority: p1
issue_id: "181"
tags: [code-review, correctness, plan-review]
dependencies: []
---

# provision.sh References Non-Existent service_url Column — Provisioning Broken

## Problem Statement
The provision.sh INSERT statement includes `service_url` in both the column list and VALUES, but the Postgres schema at `crates/mika-gateway/migrations/001_customers.sql` has NO `service_url` column. The gateway computes container URLs deterministically via `container_url()` in `routes.rs:163-165`. This means provisioning will fail at Step 3 with a Postgres column-not-found error every time, triggering rollback and permanent deletion of the AWS secret.

## Findings
- **Security sentinel**: P2 (promoted to P1) — Provisioning is broken. Rollback cascade permanently deletes AWS secret.
- **Architecture strategist**: P1 — Schema mismatch. Column was in brainstorm example but never added to migration.
- **Agent-native reviewer**: Critical — Script cannot work as written.

## Proposed Solutions

### Option A: Remove service_url from INSERT (Recommended)
The column doesn't exist and isn't needed — the gateway derives the URL from customer_id. Remove it from the INSERT statement and the corresponding SERVICE_URL variable.
- **Effort**: Trivial (5 min)
- **Risk**: None

### Option B: Add migration for service_url column
Add the column to the schema. Not recommended — it introduces a data consistency risk (stale URL if naming convention changes).
- **Effort**: Small
- **Risk**: Medium — redundant data

## Acceptance Criteria
- [ ] provision.sh INSERT matches actual Postgres schema columns
- [ ] No reference to service_url in any provisioning script

## Work Log

### 2026-02-24 - Plan Review Finding
**By:** Technical review agents (security-sentinel, architecture-strategist, agent-native-reviewer)

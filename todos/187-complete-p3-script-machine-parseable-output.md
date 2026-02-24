---
status: complete
priority: p3
issue_id: "187"
tags: [code-review, agent-native, plan-review]
dependencies: []
---

# Scripts Need Machine-Parseable Output and Granular Exit Codes

## Problem Statement
Both provision.sh and deprovision.sh output human-readable text only. An agent or CI/CD pipeline cannot parse the customer_id, deep link, or pairing token from the output. Exit codes are binary (0/1) with no failure type distinction.

## Findings
- **Agent-native reviewer**: Warning — 7/12 management operations have no script at all; existing scripts lack structured output

## Proposed Solutions

### Add --output json flag
Default: human-readable. `--output json` emits structured JSON for automation:
```json
{"customer_id": "...", "deep_link": "...", "pairing_token": "...", "expires_in": "72h"}
```

### Use distinct exit codes
- 1: invalid arguments
- 2: AWS secret creation failed
- 3: Helm install failed
- 4: Postgres registration failed
- 10: rollback also failed

### Future: Add admin scripts
- `scripts/list-customers.sh` — Query Postgres for all customers
- `scripts/customer-status.sh <id>` — Single customer status
- `scripts/suspend.sh <id>` — Suspend without full deprovision

## Acceptance Criteria
- [ ] Both scripts support `--output json`
- [ ] Exit codes distinguish failure types
- [ ] Non-interactive stdin detection in deprovision.sh

## Work Log

### 2026-02-24 - Plan Review Finding
**By:** Agent-native reviewer

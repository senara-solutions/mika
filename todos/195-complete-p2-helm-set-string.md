---
status: complete
priority: p2
issue_id: "195"
tags: [code-review, correctness]
dependencies: []
---

# Use --set-string for customer.name in Helm Install

## Problem Statement
`provision.sh` passes customer name via `--set "customer.name=${CUSTOMER_NAME}"`. Helm's `--set` has complex YAML escaping rules. Names with apostrophes (e.g., "O'Brien") can break Helm's YAML parsing despite passing the customer name regex validation.

## Findings
- **Security sentinel**: Low severity (L2)
- **Agent-native reviewer**: Observation #11
- Location: `scripts/provision.sh` line 146

## Proposed Solutions

### Option 1: Use --set-string (Recommended)
Replace `--set "customer.name=..."` with `--set-string "customer.name=..."`:

- **Pros**: 1-word change, handles all string escaping
- **Cons**: None
- **Effort**: Small (1 minute)
- **Risk**: Low

## Technical Details
- **Affected Files**: `scripts/provision.sh`

## Acceptance Criteria
- [ ] `--set-string` used for customer.name
- [ ] Customer name "O'Brien" provisions successfully

## Work Log
### 2026-02-24 - Found during code review
**By:** Security sentinel + Agent-native reviewer

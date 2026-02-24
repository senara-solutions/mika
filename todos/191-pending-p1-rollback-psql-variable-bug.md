---
status: pending
priority: p1
issue_id: "191"
tags: [code-review, security, correctness]
dependencies: []
---

# Rollback Postgres DELETE Uses Broken psql Variable Binding

## Problem Statement
The rollback `cleanup()` function in `scripts/provision.sh` uses a single-quoted heredoc (`<<'ROLLBACK_SQL'`) without passing the customer_id via `-v` flag. The `\set customer_id_val :'CUSTOMER_ID'` attempts to reference a psql variable that was never defined, so the DELETE matches zero rows. If provisioning fails at step 3 or 4, the Postgres row is NOT rolled back — leaving an orphaned customer record.

## Findings
- **Security sentinel, Architecture strategist, Agent-native reviewer** all independently identified this bug
- Location: `scripts/provision.sh` lines 113-116
- The main INSERT at lines 163-182 correctly uses `-v customer_id="${CUSTOMER_ID}"` but the rollback does not
- The `|| true` suppresses the error, making the failure silent

## Proposed Solutions

### Option 1: Fix the `-v` flag (Recommended)
Add `-v customer_id="${CUSTOMER_ID}"` to the rollback psql call, matching the pattern used in the main INSERT.

```bash
psql "${DATABASE_URL}" -v ON_ERROR_STOP=1 \
    -v customer_id="${CUSTOMER_ID}" \
    <<'ROLLBACK_SQL' 2>/dev/null || true
DELETE FROM customers WHERE id = :'customer_id'::uuid;
ROLLBACK_SQL
```

- **Pros**: 1-line fix, matches existing pattern, preserves SQL injection safety
- **Cons**: None
- **Effort**: Small (5 minutes)
- **Risk**: Low

## Technical Details
- **Affected Files**: `scripts/provision.sh`
- **Related Components**: Provisioning rollback, Postgres cleanup
- **Database Changes**: None (fixes existing query)

## Acceptance Criteria
- [ ] Rollback DELETE uses `-v customer_id="${CUSTOMER_ID}"` flag
- [ ] Single-quoted heredoc preserved (no shell interpolation in SQL)
- [ ] Manual test: verify rollback deletes the Postgres row when step 4 fails

## Work Log
### 2026-02-24 - Found during code review
**By:** Security sentinel + Architecture strategist + Agent-native reviewer
**Actions:** Identified broken psql variable binding in rollback cleanup

## Resources
- PR: #8
- File: scripts/provision.sh:113-116

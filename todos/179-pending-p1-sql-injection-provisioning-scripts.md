---
status: pending
priority: p1
issue_id: "179"
tags: [code-review, security, plan-review]
dependencies: []
---

# SQL Injection in Provisioning Scripts via Shell Variable Interpolation

## Problem Statement
Both provision.sh and deprovision.sh use shell variable interpolation directly in SQL strings passed to psql, despite the plan's design decisions table claiming "psql with `\set` variables." The provision.sh heredoc uses `\$\$${CUSTOMER_NAME}\$\$` which is breakable if CUSTOMER_NAME contains `$$`. The deprovision.sh uses direct interpolation for UPDATE and DELETE.

## Findings
- **Security sentinel**: P1 — Dollar-quoting is breakable; CUSTOMER_NAME has no input validation
- **Architecture strategist**: P1 — Plan claims parameterized queries but implementation doesn't use them
- **Agent-native reviewer**: Critical — Any agent passing untrusted input creates injection vector
- **Code simplicity**: P1 — Design decision says one thing, implementation does another

## Proposed Solutions

### Option A: Use psql `\set` variables with quoted heredoc (Recommended)
```bash
psql "${DATABASE_URL}" -v ON_ERROR_STOP=1 \
    -v cid="${CUSTOMER_ID}" \
    -v cname="${CUSTOMER_NAME}" \
    -v cplan="${PLAN}" \
    -v ctz="${TIMEZONE}" \
    -v ctoken="${PAIRING_TOKEN}" <<'SQL'
INSERT INTO customers (id, name, plan, timezone, status, pairing_token, pairing_expires_at)
VALUES (:'cid'::uuid, :'cname', :'cplan', :'ctz', 'provisioned', :'ctoken', now() + interval '72 hours')
ON CONFLICT (id) DO NOTHING;
SQL
```
Note: `<<'SQL'` (single-quoted) prevents all shell expansion. `:'var'` is psql's safe parameterized syntax.
- **Effort**: Small (30 min)
- **Risk**: None

Apply same pattern to deprovision.sh UPDATE and DELETE statements.

## Technical Details
- **Affected files**: Plan sections for provision.sh (Step 3) and deprovision.sh (Steps 1, 5)

## Acceptance Criteria
- [ ] No shell variable expansion inside SQL strings (use quoted heredocs)
- [ ] All psql calls use `-v` variables with `:'var'` syntax
- [ ] CUSTOMER_NAME validated: alphanumeric, spaces, hyphens, max 100 chars

## Work Log

### 2026-02-24 - Plan Review Finding
**By:** Technical review agents (security-sentinel, architecture-strategist, agent-native-reviewer)

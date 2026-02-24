---
status: ready
priority: p2
issue_id: "142"
tags: [plan-review, architecture, data-integrity]
dependencies: []
---

# Postgres schema hardening — CHECK constraints, updated_at, redundant index

## Problem Statement
The planned Postgres schema has several gaps: (1) `status` and `plan` columns are bare TEXT with no CHECK constraint — any string can be inserted, (2) no `updated_at` column for audit trail, (3) redundant explicit index on `telegram_chat_id` when UNIQUE already creates one.

**Why it matters:** Without CHECK constraints, a typo in a psql UPDATE (e.g., `'actve'` instead of `'active'`) silently corrupts data. Missing `updated_at` makes debugging state changes impossible.

## Findings
- Source: Architecture Strategist (High + Low), Security Sentinel (M-6, L-4)
- status column accepts any TEXT value — should be constrained to known states
- plan column similarly unconstrained
- UNIQUE on telegram_chat_id already creates an index — explicit CREATE INDEX is redundant
- No updated_at column means no way to track when customer state changed

## Proposed Solutions

### Option 1: Add constraints and audit column (Recommended)
```sql
CREATE TABLE customers (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    plan TEXT NOT NULL DEFAULT 'standard' CHECK (plan IN ('standard', 'premium')),
    status TEXT NOT NULL DEFAULT 'provisioned' CHECK (status IN ('provisioned', 'active', 'suspended')),
    telegram_chat_id BIGINT UNIQUE,
    timezone TEXT NOT NULL DEFAULT 'UTC',
    service_url TEXT,
    paired_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- Remove redundant idx_customers_telegram_chat_id (UNIQUE already creates it)
CREATE INDEX idx_customers_status ON customers(status);
```
Add a trigger or application-level logic to update `updated_at` on every change.
- **Pros**: Data integrity, audit trail, cleaner schema
- **Cons**: CHECK constraints need updating if new statuses/plans added
- **Effort**: Small
- **Risk**: Low

## Technical Details
- **Affected files**: Plan schema definition, db.rs
- **Related Components**: All customer queries

## Acceptance Criteria
- [ ] status column has CHECK constraint for valid values
- [ ] plan column has CHECK constraint for valid values
- [ ] updated_at column present and auto-updated
- [ ] Redundant index removed
- [ ] Invalid status/plan values rejected by database

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent plan review)
**Actions:** Architecture Strategist and Security Sentinel flagged schema gaps

---
module: packages/ui
tags: [dashboard, pagination, design-system, documentation, enforcement]
problem_type: drift-prevention
---

# Pagination audit — canonical-primitive enforcement pattern

## Problem

`@senara-solutions/ui` exports shared components but without documentation making them mandatory, hand-rolled implementations can drift back in. The audit for `<Pagination />` (mika#663) confirmed 100% adoption (15 callsites across 9 pages), but needed a durable enforcement mechanism.

## Solution

Two-layer enforcement via CLAUDE.md documentation:

1. **`packages/ui/CLAUDE.md`** — Canonical Primitives table with "Hand-rolled forbidden" column and explicit escape-hatch criteria (valid: missing capability with extension PR linked; invalid: timeline pressure).
2. **Root `CLAUDE.md`** — One-sentence bold callout in the `packages/ui/` directory entry making the constraint discoverable at the top-level scan.

## Key insight

The enforcement table pattern (component | purpose | forbidden | migration status) serves multiple tickets in milestone #13. Each audit ticket (#654, #655, #657, #658, #663, #665) marks its row as "Audited clean" when the migration PR ships. New components default to "Audit pending" until their migration ticket confirms zero hand-rolled instances.

## Verification

```bash
# Confirm all Pagination imports come from the shared package
grep -rn "import.*Pagination" dashboard/src/pages/*.tsx | grep -v "@senara-solutions/ui"
# → should return nothing (zero local-path imports)
```

---
title: "docs(solutions): compound the const-assert co-edit pattern from #908/#915"
type: docs
status: draft
date: 2026-05-28
issue: mika#917
---

# Compound the Const-Assert Co-Edit Pattern

## Overview

Create `docs/solutions/best-practices/schema-bump-fixture-pin-co-edit-2026-05-01.md` with the five sections specified in the ticket: (1) the const-assert pattern, (2) applicability conditions, (3) non-applicability boundary, (4) rejected alternative, and (5) inventory table. This supersedes the existing partial doc at `compile-time-co-edit-guard-const-assert-2026-05-01.md` which was compounded during #918 but only covers sections 1–2.

## Problem Frame

The const-assert pattern that landed in `tests/eval/kg_fixtures/mod.rs` (post-#915) collapses what would have been a lefthook-hook + conditional-eval-gate pair into zero new tooling. The pattern is reusable but under-documented. The existing solution doc is missing the "when NOT to use" boundary (section 3), the "rejected alternative" rationale (section 4), and the inventory table (section 5). Without these, the next contributor who sees the CLAUDE.md "Schema vN" drift will try to apply the const-assert pattern to prose and waste a cycle discovering it can't work.

## Scope Boundaries

- **In scope:** One markdown file with five sections, correct frontmatter, and the inventory table.
- **Out of scope:** Fixing the unprotected prose invariants (CLAUDE.md schema line, runtime-structure.md migration table). Generalizing into a co-edit framework.
- **Decision:** Supersede vs. update the existing partial doc. The ticket specifies a different filename (`schema-bump-fixture-pin-co-edit-...`) with different frontmatter (`module: kg` vs current `module: mika-agent`; `problem_type: silent-drift` vs current `drift-detection`). **Plan: create the ticket-specified file as the canonical doc. Remove the existing partial doc to avoid duplication.** Both cover the same pattern; keeping both would violate single-source-of-truth.

## Requirements Trace

- R1. File at `docs/solutions/best-practices/schema-bump-fixture-pin-co-edit-2026-05-01.md`
- R2. YAML frontmatter: `module: kg`, `tags: [schema-migration, eval, lefthook, const-assert]`, `problem_type: silent-drift`, `category: best-practices`
- R3. Section 1 — the const-assert shape from `kg_fixtures/mod.rs` with explanation of mechanism (lefthook pre-commit → clippy → const-eval)
- R4. Section 2 — applicability: two Rust constants that must move together AND both readable at const-eval time. Note the string-literal-only constraint on `assert!` messages.
- R5. Section 3 — non-applicability: `CLAUDE.md` schema line (prose, not const-eval-accessible) and `docs/runtime-structure.md` migration table (same shape). Two sentences max. Boundary documentation, not a roadmap.
- R6. Section 4 — rejected alternative ("make cargo test include eval by default") with the three-point rationale from the ticket.
- R7. Section 5 — inventory table with three rows: (a) `db.rs::CURRENT_SCHEMA_VERSION ↔ kg_fixtures::PINNED_SCHEMA_VERSION` — const-assert, landed; (b) `db.rs::CURRENT_SCHEMA_VERSION ↔ CLAUDE.md "Schema vN" line` — none, unprotected; (c) migration list ↔ `docs/runtime-structure.md` migration table — none, unprotected. Unprotected rows are inventory, not calls to action.
- R8. PR description links to #908 and #915.

## Implementation Steps

### Step 1 — Create the canonical solution doc

Write `docs/solutions/best-practices/schema-bump-fixture-pin-co-edit-2026-05-01.md` with all five sections, pulling the const-assert code from `crates/mika-agent/tests/eval/kg_fixtures/mod.rs` (lines 21–33) as the live reference. Use the current `PINNED_SCHEMA_VERSION` value (39, not the original 29 from #915).

Frontmatter per ticket:
```yaml
---
module: kg
tags: [schema-migration, eval, lefthook, const-assert]
problem_type: silent-drift
category: best-practices
date: 2026-05-01
related_issues: [917, 915, 908]
---
```

### Step 2 — Remove the existing partial doc

Delete `docs/solutions/best-practices/compile-time-co-edit-guard-const-assert-2026-05-01.md`. This file was compounded during #918 and covers sections 1–2 only. The new file supersedes it completely. Check for any cross-references to the old filename and update them.

### Step 3 — Verify cross-references

Search the codebase for references to either filename (`compile-time-co-edit-guard-const-assert` or `schema-bump-fixture-pin-co-edit`) and ensure they point to the new canonical path. Known locations to check:
- `CLAUDE.md` (root) — unlikely, but verify
- Other `docs/solutions/` files that might cross-link
- Plan files that reference the pattern

## Verification

- [ ] `docs/solutions/best-practices/schema-bump-fixture-pin-co-edit-2026-05-01.md` exists with all five sections
- [ ] Frontmatter matches ticket spec exactly
- [ ] Old partial doc `compile-time-co-edit-guard-const-assert-2026-05-01.md` removed
- [ ] No dangling cross-references to old filename
- [ ] `cargo build` still succeeds (no code changes, but verify KG doc ingestion doesn't break)

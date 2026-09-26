---
module: kg
tags: [schema-migration, eval, lefthook, const-assert]
problem_type: silent-drift
category: best-practices
date: 2026-05-01
related_issues: [917, 915, 908]
---

# Schema-Bump / Fixture-Pin Co-Edit Guard via const-assert

## 1. The Pattern

When `db.rs::CURRENT_SCHEMA_VERSION` is bumped by a migration, the KG eval fixture pin in `tests/eval/kg_fixtures/mod.rs` must move in lockstep. A compile-time const-assert ties them together so drift is caught before code leaves the developer's machine:

```rust
use mika_agent::db::CURRENT_SCHEMA_VERSION;

const PINNED_SCHEMA_VERSION: i64 = 40;

const _: () = assert!(
    CURRENT_SCHEMA_VERSION == PINNED_SCHEMA_VERSION,
    "KG eval fixtures pin out of sync with db.rs CURRENT_SCHEMA_VERSION. \
     Bump PINNED_SCHEMA_VERSION in tests/eval/kg_fixtures/mod.rs and update \
     seed_* helpers. See docs/plans/740-kg-self-knowledge-eval.md D5.",
);
```

**How it fires:** The lefthook pre-commit hook runs `cargo clippy --all-targets`, which compiles test targets, which evaluates the `const _: () = assert!(...)` at compile time. If the two constants disagree, the build fails with the message above — before the commit is created, before CI, before review.

The existing runtime `assert_schema_version()` helper checks the DB's *actual* schema version at test time. The const-assert checks the *code-level* pin. Both are needed: the const-assert catches code drift; the runtime assert catches DB-state drift.

## 2. When to Apply

Apply this pattern when **all four** conditions hold:

1. Two Rust constants in different modules must stay in sync.
2. The relationship is mechanical (equality, ordering) — not semantic.
3. Both values are `const` expressions accessible at compile time.
4. The drift class causes test failures, not logic errors (runtime behavior is correct, but tests break on stale fixtures).

**String-literal-only constraint:** `assert!` in const context only accepts a string literal message — no `format!`, no `concat!` of non-literal items. Do not "improve" the message by interpolating version numbers.

## 3. When NOT to Apply

The const-assert pattern does **not** work for prose invariants that are not accessible at compile time. Two known instances:

- **`CLAUDE.md` "Schema vN" line** — prose in a markdown file, not a Rust constant. No `const` expression can read it.
- **`docs/runtime-structure.md` migration table** — same shape: a prose table in documentation, invisible to `rustc`.

These remain unprotected. Documenting the boundary prevents the next contributor from trying to force the pattern where it cannot work.

## 4. Rejected Alternative

**"Make `cargo test` include eval by default"** was considered and rejected for three reasons:

1. **Eval tests are slow.** Including them in every `cargo test` run would add minutes to the inner dev loop for coverage that only matters when schema or KG fixtures change.
2. **Eval tests require fixtures and setup.** Some scenarios need seeded databases and specific agent configurations that don't belong in a fast unit-test pass.
3. **The const-assert is strictly cheaper.** It fires at compile time with zero runtime cost, zero test-infrastructure cost, and zero false negatives for the specific drift class it targets.

## 5. Inventory

| Co-edit pair | Guard | Status |
|---|---|---|
| `db.rs::CURRENT_SCHEMA_VERSION` <-> `kg_fixtures::PINNED_SCHEMA_VERSION` | const-assert | Landed (#915) |
| `db.rs::CURRENT_SCHEMA_VERSION` <-> `CLAUDE.md` "Schema vN" line | None | Unprotected |
| Migration list <-> `docs/runtime-structure.md` migration table | None | Unprotected |

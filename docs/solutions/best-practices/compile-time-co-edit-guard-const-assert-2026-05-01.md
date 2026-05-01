---
module: mika-agent
tags: [testing, schema, compile-time, co-edit, eval]
problem_type: drift-detection
category: best-practices
date: 2026-05-01
related_issues: [918, 917, 908]
---

# Compile-Time Co-Edit Guard via const-assert

## Problem

Schema version bumps require coordinated updates across multiple files. The KG eval fixtures pin (`PINNED_SCHEMA_VERSION` in `tests/eval/kg_fixtures/mod.rs`) must stay in sync with `CURRENT_SCHEMA_VERSION` in `db.rs`. When a migration bumps the schema version, the fixture pin silently drifts — only caught when CI runs the full eval suite, which can be 10+ minutes into the pipeline.

The v28-to-v29 migration (#908/#915) bumped the schema but missed the fixture pin, causing 10 `eval::kg_self_knowledge::*` tests to panic at runtime.

## Solution

Add a compile-time const-assert that ties the fixture pin to the source of truth:

```rust
use mika_agent::db::CURRENT_SCHEMA_VERSION;

const PINNED_SCHEMA_VERSION: i64 = 29;

const _: () = assert!(
    CURRENT_SCHEMA_VERSION == PINNED_SCHEMA_VERSION,
    "KG eval fixtures pin out of sync with db.rs CURRENT_SCHEMA_VERSION. \
     Bump PINNED_SCHEMA_VERSION in tests/eval/kg_fixtures/mod.rs and update \
     seed_* helpers. See docs/plans/740-kg-self-knowledge-eval.md D5.",
);
```

## Key Constraints

1. **String literal only.** `assert!` in const context only accepts a string literal — no `format!`, no `concat!` of non-literal items. Do not "improve" the message by interpolating version numbers.

2. **Type alignment.** Both constants must be the same type (`i64`) for the comparison to work in const context. The fixture pin was previously `i32`, which didn't match the source of truth's `i64`.

3. **Lefthook catches it locally.** The pre-commit hook runs `cargo clippy --all-targets` which compiles tests, triggering the const-assert. Developers see the failure before pushing.

4. **Runtime assert stays as defense-in-depth.** The existing `assert_schema_version()` runtime helper checks the DB's actual schema version at test time. The const-assert checks the code-level pin. Both are needed — the const-assert catches code drift, the runtime assert catches DB-state drift.

## When to Use This Pattern

Apply const-assert co-edit guards when:

- Two constants in different modules must stay in sync
- The relationship is mechanical (equality, ordering) not semantic
- Both values are `const` expressions at compile time
- The drift class causes test failures, not logic errors (runtime behavior is correct, but tests break)

Do **not** prematurely generalize to a framework. This is an N=1 mechanism for the only currently-Rust co-edit pair. Other drift classes (CLAUDE.md schema line, runtime-structure.md migration table) are prose-shaped and need different tooling (#917).
